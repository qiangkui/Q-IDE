//! `Open` / `Print #` / `Input #` 等文件语句的运行时支持。

use super::*;

impl<'p, 'o> Runtime<'p, 'o> {
    // ---------- 文件 ----------
    pub(super) fn file_number(&mut self, expr: &Expr, line: usize) -> Result<i32, RuntimeError> {
        let value = self
            .eval(expr, line)?
            .as_int()
            .map_err(|message| RuntimeError::new(format!("文件号必须是整数：{message}"), line))?;
        i32::try_from(value)
            .map_err(|_| RuntimeError::new(format!("文件号 {value} 超出范围"), line))
    }

    /// 下一个空闲文件号（`FreeFile` 与省略 `As #n` 的 `Open` 共用）。
    pub(super) fn free_file_number(&self) -> i32 {
        let mut number = 1i32;
        while self.files.contains_key(&number) {
            number += 1;
        }
        number
    }

    /// `Get`/`Put` 的位置参数（1 基，和 VB 一致；省略时用游标）。
    pub(super) fn eval_position(
        &mut self,
        position: &Option<Expr>,
        line: usize,
    ) -> Result<Option<i64>, RuntimeError> {
        match position {
            Some(expr) => {
                let value = self.eval(expr, line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("文件位置必须是整数：{message}"), line)
                })?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// `Put #` 的写入值：UDT 按结构体布局，标量按声明类型（变量）或动态类型（表达式）写，
    /// 变长 `String` 直接写字符字节（没有长度前缀，见 VB6 实测）。
    pub(super) fn encode_put(&mut self, expr: &Expr, line: usize) -> Result<Vec<u8>, RuntimeError> {
        let value = self.eval(expr, line)?;
        if let Some(record) = value.record_rc() {
            let type_name = Self::record_type_name(&record);
            return ffi::encode_record(&type_name, &record.borrow(), &self.types)
                .map_err(|message| RuntimeError::new(format!("`Put` {message}"), line));
        }
        let declared = match expr {
            Expr::Var(name) => self.find_slot(name).map(|slot| slot.ty()),
            // `Put #1, , CByte(65)`：转换函数的结果宽度由函数决定，不能按动态类型猜
            // （`CByte` 的值在解释器里是 `Int`，猜成 Integer 会写 2 字节）。
            Expr::Call { name, .. } => conversion_type(name),
            _ => None,
        };
        let ty = match declared {
            Some(VbType::Variant | VbType::Object | VbType::Any) | None => {
                Self::dynamic_type(&value)
            }
            Some(ty) => ty,
        };
        // `Variant` 变量：VB6 实测写的是「2 字节 VARTYPE + 内容」的带标记布局
        // （`v = 5` 写 4 字节、`v = "AB"` 写 6 字节、`v = True` 写 4 字节）。
        if matches!(declared, Some(VbType::Variant)) {
            let narrow = self
                .static_type(expr)
                .filter(|ty| !matches!(ty, VbType::Variant | VbType::Any | VbType::Object));
            return Self::encode_variant(narrow.unwrap_or(ty), &value);
        }
        // 变长 `String`：VB6 实测直接写字符字节、没有长度前缀（VB6 用系统代码页，QIDE 统一 UTF-8）。
        if ty == VbType::Str {
            return Ok(value.to_display().into_bytes());
        }
        ffi::encode_scalar(ty, &value)
            .map_err(|message| RuntimeError::new(format!("`Put` {message}"), line))
    }

    /// `Put #` 写 `Variant` 时的带标记布局：2 字节 VARTYPE + 内容（本机 VB6 逐字节比对过）。
    ///
    /// 不支持的类型（对象、数组…）报错误号 458，与 VB6 一致。
    pub(super) fn encode_variant(ty: VbType, value: &Value) -> Result<Vec<u8>, RuntimeError> {
        let number = builtins::vartype_of(ty);
        let mut bytes = vec![0u8; 2];
        let code = match (&value, ty) {
            (Value::Empty, _) => 0,
            (Value::Null, _) => 1,
            _ => number,
        };
        bytes[..2].copy_from_slice(&(code as i16).to_le_bytes());
        match code {
            0 | 1 => {}
            // `Boolean` 在 VB6 的 Variant 里是 2 字节（`0B 00 FF FF`），不是 Win32 的 4 字节 BOOL。
            11 => {
                let flag: i16 = match value {
                    Value::Bool(true) => -1,
                    _ => 0,
                };
                bytes.extend_from_slice(&flag.to_le_bytes());
            }
            // `String`：2 字节长度前缀 + 字节（QIDE 用 UTF-8，长度按字节数）。
            8 => {
                let text = value.to_display();
                let raw = text.as_bytes();
                let len = u16::try_from(raw.len()).map_err(|_| {
                    RuntimeError::new("`Put` 的字符串太长（超过 65535 字节）", 0).with_number(458)
                })?;
                bytes.extend_from_slice(&len.to_le_bytes());
                bytes.extend_from_slice(raw);
            }
            _ => bytes.extend_from_slice(&ffi::encode_scalar(ty, value).map_err(|message| {
                RuntimeError::new(format!("`Put` {message}"), 0).with_number(458)
            })?),
        }
        Ok(bytes)
    }

    /// 读回 `Put #` 写下的带标记 `Variant`：按 2 字节 VARTYPE 决定后面跟多少字节。
    pub(super) fn read_variant(
        &mut self,
        code: i16,
        number: i32,
        line: usize,
    ) -> Result<(Value, Option<VbType>), RuntimeError> {
        let (ty, narrow) = match code {
            0 => return Ok((Value::Empty, None)),
            1 => return Ok((Value::Null, None)),
            2 => (VbType::Integer, VbType::Integer),
            3 => (VbType::Long, VbType::Long),
            4 => (VbType::Single, VbType::Single),
            5 => (VbType::Double, VbType::Double),
            6 => (VbType::Currency, VbType::Currency),
            7 => (VbType::Date, VbType::Date),
            17 => (VbType::Byte, VbType::Byte),
            11 => {
                let raw = self.binary_read(number, None, 2, line)?;
                let flag = i16::from_le_bytes([raw[0], raw[1]]) != 0;
                return Ok((Value::Bool(flag), Some(VbType::Boolean)));
            }
            8 => {
                let header = self.binary_read(number, None, 2, line)?;
                let len = u16::from_le_bytes([header[0], header[1]]) as usize;
                let raw = self.binary_read(number, None, len, line)?;
                let text = String::from_utf8_lossy(&raw).into_owned();
                return Ok((Value::Str(text), Some(VbType::Str)));
            }
            other => {
                return Err(RuntimeError::new(
                    format!("`Get` 读到不支持的 Variant 类型标记 {other}"),
                    line,
                )
                .with_number(458));
            }
        };
        let size = ffi::binary_width(ty)
            .map_err(|message| RuntimeError::new(format!("`Get` {message}"), line))?;
        let bytes = self.binary_read(number, None, size, line)?;
        let value = ffi::decode_scalar(ty, &bytes).map_err(|message| {
            RuntimeError::new(format!("读取 #{number} 失败：{message}"), line)
        })?;
        Ok((value, Some(narrow)))
    }

    /// `Lock #` 的范围换算：`For Random` 下位置是记录号，其它模式是 1 基字节偏移。
    pub(super) fn lock_span(
        start: Option<i64>,
        end: Option<i64>,
        record_len: Option<usize>,
        number: i32,
        line: usize,
    ) -> Result<(u64, u64), RuntimeError> {
        let unit = record_len.unwrap_or(1) as u64;
        let offset = match start {
            Some(start) => {
                if start < 1 {
                    return Err(RuntimeError::new(
                        format!("`#{number}` 的起始位置 {start} 无效（从 1 开始）"),
                        line,
                    )
                    .with_number(63));
                }
                (start as u64 - 1) * unit
            }
            None => 0,
        };
        let length = match end {
            Some(end) => {
                let first = start.unwrap_or(1);
                if end < first {
                    return Err(RuntimeError::new(
                        format!("`#{number}` 的结束位置 {end} 小于起始位置 {first}"),
                        line,
                    )
                    .with_number(63));
                }
                ((end - first) as u64 + 1) * unit
            }
            // 只给起始位置：一直锁到文件末尾。
            None => u64::MAX,
        };
        Ok((offset, length))
    }

    /// 记录的类型名（小写，和 `types` 表的键一致）。
    pub(super) fn record_type_name(record: &Rc<RefCell<VbRecord>>) -> String {
        record.borrow().type_name().to_ascii_lowercase()
    }

    /// 值的动态类型：小数持 `Double`、布尔按 `Boolean`、字符串持 `String`，其余按 `Long`。
    pub(super) fn dynamic_type(value: &Value) -> VbType {
        match value {
            Value::Float(_) => VbType::Double,
            Value::Currency(_) => VbType::Currency,
            Value::Date(_) => VbType::Date,
            Value::Bool(_) => VbType::Boolean,
            Value::Str(_) => VbType::Str,
            Value::Int(_) => VbType::Long,
            _ => VbType::Variant,
        }
    }

    pub(super) fn binary_file_mut(
        &mut self,
        number: i32,
        line: usize,
    ) -> Result<&mut BinaryFile, RuntimeError> {
        match self.files.get_mut(&number) {
            Some(FileHandle::Binary(file)) => Ok(file),
            Some(_) => Err(RuntimeError::new(
                format!("文件号 #{number} 不是以 Binary 或 Random 打开"),
                line,
            )
            .with_number(54)),
            None => {
                Err(RuntimeError::new(format!("文件号 #{number} 没有打开"), line).with_number(52))
            }
        }
    }

    pub(super) fn binary_read(
        &mut self,
        number: i32,
        position: Option<i64>,
        size: usize,
        line: usize,
    ) -> Result<Vec<u8>, RuntimeError> {
        let file = self.binary_file_mut(number, line)?;
        if let Some(position) = position {
            file.move_to(position, line)?;
        }
        let advance = file.advance(size, line)?;
        file.file.seek(SeekFrom::Start(file.pos)).map_err(|error| {
            RuntimeError::new(format!("定位 #{number} 失败：{error}"), line).with_number(57)
        })?;
        let mut buffer = vec![0u8; size];
        file.file.read_exact(&mut buffer).map_err(|error| {
            RuntimeError::new(format!("读取 #{number} 失败：{error}"), line).with_number(62)
        })?;
        file.pos += advance as u64;
        Ok(buffer)
    }

    pub(super) fn binary_write(
        &mut self,
        number: i32,
        position: Option<i64>,
        bytes: &[u8],
        line: usize,
    ) -> Result<(), RuntimeError> {
        let file = self.binary_file_mut(number, line)?;
        if let Some(position) = position {
            file.move_to(position, line)?;
        }
        let advance = file.advance(bytes.len(), line)?;
        file.file.seek(SeekFrom::Start(file.pos)).map_err(|error| {
            RuntimeError::new(format!("定位 #{number} 失败：{error}"), line).with_number(57)
        })?;
        file.file.write_all(bytes).map_err(|error| {
            RuntimeError::new(format!("写入 #{number} 失败：{error}"), line).with_number(57)
        })?;
        if advance > bytes.len() {
            let padding = vec![0u8; advance - bytes.len()];
            file.file.write_all(&padding).map_err(|error| {
                RuntimeError::new(format!("写入 #{number} 失败：{error}"), line).with_number(57)
            })?;
        }
        file.pos += advance as u64;
        Ok(())
    }

    /// `EOF(n)` / `LOF(n)` / `FreeFile()`。
    pub(super) fn file_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        if name == "freefile" {
            return Ok(Value::Int(i64::from(self.free_file_number())));
        }
        let Some(expr) = args.first() else {
            return Err(RuntimeError::new(format!("`{name}` 缺少第 1 个参数"), line));
        };
        let number = self.file_number(expr, line)?;
        match name {
            "eof" => {
                let Some(handle) = self.files.get_mut(&number) else {
                    return Err(RuntimeError::new(
                        format!("文件号 #{number} 没有打开"),
                        line,
                    ));
                };
                match handle {
                    FileHandle::Reader(reader) => {
                        let buffer = reader.fill_buf().map_err(|error| {
                            RuntimeError::new(format!("读取 #{number} 失败：{error}"), line)
                        })?;
                        Ok(Value::Bool(buffer.is_empty()))
                    }
                    FileHandle::Binary(file) => {
                        let size = file.file.metadata().map(|meta| meta.len()).unwrap_or(0);
                        Ok(Value::Bool(file.pos >= size))
                    }
                    FileHandle::Writer(_) => Ok(Value::Bool(true)),
                }
            }
            "fileattr" => {
                // `FileAttr(n, returntype)`：VB6 里 `returntype` 只能是 1（返回模式码），
                // 其它值（包括 2）报错误号 5——但**先查文件号**：没打开时是 52，
                // 不管 `returntype` 传的是什么（本机实测 `FileAttr(已关闭的文件号, 2)` 是 52）。
                if !self.files.contains_key(&number) {
                    return Err(
                        RuntimeError::new(format!("文件号 #{number} 没有打开"), line)
                            .with_number(52),
                    );
                }
                let Some(expression) = args.get(1) else {
                    return Err(RuntimeError::new("`FileAttr` 缺少第 2 个参数", line));
                };
                let return_type = self.eval(expression, line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("`FileAttr` 的返回值类型：{message}"), line)
                })?;
                if return_type != 1 {
                    return Err(
                        RuntimeError::new("`FileAttr` 的返回值类型只能是 1", line).with_number(5)
                    );
                }
                let Some(handle) = self.files.get(&number) else {
                    return Err(
                        RuntimeError::new(format!("文件号 #{number} 没有打开"), line)
                            .with_number(52),
                    );
                };
                let mode = match handle {
                    FileHandle::Reader(_) => 1,
                    FileHandle::Writer(writer) => writer.mode,
                    // `For Random` 是 4，`For Binary` 是 32。
                    FileHandle::Binary(binary) => {
                        if binary.random {
                            4
                        } else {
                            32
                        }
                    }
                };
                Ok(Value::Int(mode))
            }
            "seek" | "loc" => {
                let Some(handle) = self.files.get_mut(&number) else {
                    return Err(
                        RuntimeError::new(format!("文件号 #{number} 没有打开"), line)
                            .with_number(52),
                    );
                };
                match handle {
                    FileHandle::Binary(file) => {
                        let value = if name == "loc" {
                            file.location()
                        } else {
                            file.position()
                        };
                        Ok(Value::Int(value))
                    }
                    FileHandle::Reader(reader) => {
                        // BufReader 的逻辑位置 = 底层位置 - 尚未消费的缓冲长度。
                        let raw = reader.get_ref().stream_position().unwrap_or(0);
                        let buffered = reader.buffer().len() as u64;
                        Ok(Value::Int((raw.saturating_sub(buffered)) as i64 + 1))
                    }
                    FileHandle::Writer(writer) => {
                        let raw = writer.file().stream_position().unwrap_or(0);
                        Ok(Value::Int(raw as i64 + 1))
                    }
                }
            }
            _ => {
                let Some(handle) = self.files.get(&number) else {
                    return Err(
                        RuntimeError::new(format!("文件号 #{number} 没有打开"), line)
                            .with_number(52),
                    );
                };
                let file = match handle {
                    FileHandle::Reader(reader) => reader.get_ref(),
                    FileHandle::Writer(writer) => writer.file(),
                    FileHandle::Binary(binary) => &binary.file,
                };
                let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
                Ok(Value::Int(size as i64))
            }
        }
    }

    /// `Input(n, #f)`：从 `For Input` 的文本文件里读 `n` 个字符（换行也照读）。
    ///
    /// 本机 VB6 实测：剩下的字符不够 `n` 个时报**错误号 62**（输入超出文件尾），赋值不生效。
    pub(super) fn input_function(
        &mut self,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let Some(count_expr) = args.first() else {
            return Err(RuntimeError::new("`Input` 缺少第 1 个参数", line));
        };
        let count = self
            .eval(count_expr, line)?
            .as_int()
            .map_err(|message| RuntimeError::new(format!("`Input` 的字符数：{message}"), line))?;
        let count = usize::try_from(count.max(0)).unwrap_or(usize::MAX);
        let Some(number_expr) = args.get(1) else {
            return Err(RuntimeError::new("`Input` 缺少第 2 个参数", line));
        };
        let number = self.file_number(number_expr, line)?;
        let Some(handle) = self.files.get_mut(&number) else {
            return Err(
                RuntimeError::new(format!("文件号 #{number} 没有打开"), line).with_number(52),
            );
        };
        let reader = match handle {
            FileHandle::Reader(reader) => reader,
            // 本机 VB6 实测：`For Binary` 的文件上 `Input(n, #f)` 不报错、返回空串；
            // `For Output` / `For Append` 才报 54（文件模式错误）。
            FileHandle::Binary(_) => return Ok(Value::Str(String::new())),
            FileHandle::Writer(_) => {
                return Err(
                    RuntimeError::new(format!("文件号 #{number} 不是以 Input 打开"), line)
                        .with_number(54),
                );
            }
        };
        let mut text = String::new();
        for _ in 0..count {
            match read_char(reader) {
                Ok(Some(ch)) => text.push(ch),
                Ok(None) => {
                    return Err(RuntimeError::new("输入超出文件尾", line).with_number(62));
                }
                Err(error) => {
                    return Err(
                        RuntimeError::new(format!("读取 #{number} 失败：{error}"), line)
                            .with_number(io_error_number(&error)),
                    );
                }
            }
        }
        Ok(Value::Str(text))
    }

    /// 求维度表达式并换算成 `(下界, 元素数)`。
    pub(super) fn array_dims(
        &mut self,
        bounds: &[(Expr, Expr)],
        line: usize,
    ) -> Result<Vec<(i32, usize)>, RuntimeError> {
        let mut dims = Vec::with_capacity(bounds.len());
        let mut total = 1usize;
        for (lower, upper) in bounds {
            let lower_value = self.eval(lower, line)?.as_int().map_err(|message| {
                RuntimeError::new(format!("数组下界必须是整数：{message}"), line)
            })?;
            let upper_value = self.eval(upper, line)?.as_int().map_err(|message| {
                RuntimeError::new(format!("数组上界必须是整数：{message}"), line)
            })?;
            if lower_value > upper_value {
                return Err(RuntimeError::new(
                    format!("数组下界 {lower_value} 不能大于上界 {upper_value}"),
                    line,
                ));
            }
            let lower_value = i32::try_from(lower_value)
                .map_err(|_| RuntimeError::new(format!("数组下界 {lower_value} 超出范围"), line))?;
            // 上界可以接近 `i64::MAX`，用 `i128` 算元素数，避免相减时溢出。
            let count = i128::from(upper_value) - i128::from(lower_value) + 1;
            let count = usize::try_from(count).unwrap_or(usize::MAX);
            total = total.saturating_mul(count);
            if total > MAX_ARRAY_ELEMENTS {
                return Err(RuntimeError::new(
                    format!("数组元素超过上限（{MAX_ARRAY_ELEMENTS} 个）"),
                    line,
                ));
            }
            dims.push((lower_value, count));
        }
        Ok(dims)
    }

    /// 求一组下标表达式，统一转成 `i32`。
    /// 求值一组下标。常见 1–2 维走栈上缓冲，不再每次访问数组都分配 `Vec`。
    pub(super) fn eval_indices(
        &mut self,
        exprs: &[Expr],
        line: usize,
    ) -> Result<IndexBuf, RuntimeError> {
        if exprs.len() <= INLINE_DIMS {
            let mut indices = IndexBuf {
                inline: [0; INLINE_DIMS],
                len: exprs.len(),
                heap: Vec::new(),
            };
            for (position, expr) in exprs.iter().enumerate() {
                indices.inline[position] = self.eval_index(expr, line)?;
            }
            Ok(indices)
        } else {
            let mut heap = Vec::with_capacity(exprs.len());
            for expr in exprs {
                heap.push(self.eval_index(expr, line)?);
            }
            Ok(IndexBuf {
                inline: [0; INLINE_DIMS],
                len: exprs.len(),
                heap,
            })
        }
    }

    pub(super) fn eval_index(&mut self, expr: &Expr, line: usize) -> Result<i32, RuntimeError> {
        let value = self.eval(expr, line)?.as_int().map_err(|message| {
            RuntimeError::new(format!("数组下标必须是整数：{message}"), line).with_number(9)
        })?;
        i32::try_from(value).map_err(|_| {
            RuntimeError::new(format!("数组下标 {value} 超出范围"), line).with_number(9)
        })
    }
}
