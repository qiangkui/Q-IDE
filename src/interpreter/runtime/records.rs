//! 自定义类型（UDT）与 `With` 块。

use super::*;

impl<'p, 'o> Runtime<'p, 'o> {
    // ---------- 记录 / With ----------

    /// `Dim p As Point`：按类型定义创建记录，字段取默认值。
    pub(super) fn new_record(&self, type_name: &str, line: usize) -> Result<Value, RuntimeError> {
        let def = self
            .types
            .get(type_name)
            .copied()
            .ok_or_else(|| RuntimeError::new(format!("未定义的类型 `{type_name}`"), line))?;
        self.build_record(def, 0, line)
    }

    pub(super) fn build_record(
        &self,
        def: &'p TypeDef,
        depth: usize,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        if depth > 16 {
            return Err(RuntimeError::new(
                format!("类型 `{}` 嵌套过深（可能是循环定义）", def.name),
                line,
            ));
        }
        let mut fields = Vec::with_capacity(def.fields.len());
        for field in &def.fields {
            let value = match (&field.record, field.array_bounds) {
                (Some(name), bounds) => {
                    let nested =
                        self.types.get(name).copied().ok_or_else(|| {
                            RuntimeError::new(format!("未定义的类型 `{name}`"), line)
                        })?;
                    let template = self.build_record(nested, depth + 1, line)?;
                    match bounds {
                        Some(bounds) => {
                            let display = template
                                .as_record()
                                .map(|record| record.borrow().type_name().to_string())
                                .unwrap_or_else(|_| name.clone());
                            Self::array_field(bounds, field.ty, Some((display, template)), line)?
                        }
                        None => template,
                    }
                }
                (None, Some(bounds)) => Self::array_field(bounds, field.ty, None, line)?,
                (None, None) => match field.fixed_len {
                    Some(len) => Value::Str(pad_fixed("", len)),
                    None => default_value(field.ty),
                },
            };
            let record_field = match field.fixed_len {
                Some(len) => RecordField::fixed(field.name.clone(), field.ty, value, len),
                None => RecordField::new(field.name.clone(), field.ty, value),
            };
            fields.push(record_field);
        }
        Ok(Value::record(def.display.clone(), fields))
    }

    /// 记录里的定长数组字段：`(下界, 上界)` → `VbArray`。
    pub(super) fn array_field(
        bounds: (i64, i64),
        ty: VbType,
        record: Option<(String, Value)>,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let count = (bounds.1 - bounds.0 + 1).max(0) as usize;
        if count > MAX_ARRAY_ELEMENTS {
            return Err(RuntimeError::new(
                format!("数组字段超过上限（{MAX_ARRAY_ELEMENTS} 个元素）"),
                line,
            ));
        }
        let lower = i32::try_from(bounds.0)
            .map_err(|_| RuntimeError::new(format!("数组下界 {} 超出范围", bounds.0), line))?;
        let dims = vec![(lower, count)];
        let array = match record {
            Some((name, template)) => VbArray::new_records(dims, false, name, template),
            None => VbArray::new(dims, ty, false),
        };
        Ok(Value::Array(Rc::new(RefCell::new(array))))
    }

    pub(super) fn read_member(
        &mut self,
        base: &Expr,
        name: &str,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        // 内置 `App` 对象：`App.Path` 是脚本所在目录。
        if let Expr::Var(root) = base
            && root == "app"
            && self.find_slot("app").is_none()
        {
            return match name {
                "path" => Ok(Value::Str(self.script_dir.display().to_string())),
                _ => Err(RuntimeError::new(format!("`App` 没有属性 `{name}`"), line)),
            };
        }

        // 内置 `Err` 对象：`Err.Number` / `Err.Description` / `Err.Source`。
        if let Expr::Var(root) = base
            && root == "err"
            && self.find_slot("err").is_none()
        {
            return Ok(match name {
                "number" => Value::Int(
                    self.last_error
                        .as_ref()
                        .map(|error| error.number)
                        .unwrap_or(0),
                ),
                "description" => Value::Str(
                    self.last_error
                        .as_ref()
                        .map(|error| error.message.to_string())
                        .unwrap_or_default(),
                ),
                "source" => Value::Str(
                    self.last_error
                        .as_ref()
                        .and_then(|error| error.source.as_deref())
                        .map(str::to_string)
                        .unwrap_or_else(|| "vb".to_string()),
                ),
                _ => return Err(RuntimeError::new(format!("`Err` 没有属性 `{name}`"), line)),
            });
        }

        let value = self.eval(base, line)?;
        if let Some(collection) = value.collection_rc() {
            return match name {
                "count" => Ok(Value::Int(collection.borrow().count() as i64)),
                _ => Err(RuntimeError::new(
                    format!("`Collection` 没有属性 `{name}`"),
                    line,
                )),
            };
        }
        if let Some(entries) = value.dictionary_rc() {
            let entries = entries.borrow();
            return match name {
                "count" => Ok(Value::Int(entries.count() as i64)),
                "keys" => Ok(Value::array(entries.keys())),
                "items" => Ok(Value::array(entries.values())),
                _ => Err(RuntimeError::new(
                    format!("`Dictionary` 没有属性 `{name}`"),
                    line,
                )),
            };
        }
        if let Value::Class(instance) = &value {
            return self.read_instance_member(&instance.0, name, line);
        }
        if let Value::ComObject(object) = &value {
            return object.get(name).map_err(|failure| {
                RuntimeError::new(failure.message, line).with_number(failure.number)
            });
        }
        if matches!(value, Value::Empty | Value::Nothing) {
            return Err(RuntimeError::new(
                format!("对象变量还没有 `Set`，不能访问 `.{name}`"),
                line,
            )
            .with_number(91));
        }
        let record = value
            .as_record()
            .map_err(|_| RuntimeError::new(format!("值不是记录类型，不能访问 `.{name}`"), line))?;
        let record = record.borrow();
        record.get(name).ok_or_else(|| {
            RuntimeError::new(
                format!("类型 `{}` 没有字段 `{name}`", record.type_name()),
                line,
            )
        })
    }

    /// `With` 块里的 `.X` / `.A.B` 读取。
    pub(super) fn read_with_member(
        &mut self,
        path: &[String],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let Some(root) = self.with_stack.last().cloned() else {
            return Err(RuntimeError::new("`.成员` 只能出现在 `With` 块内", line));
        };
        let Some((last, parents)) = path.split_last() else {
            return Err(RuntimeError::new("缺少成员名", line));
        };
        let mut current = root;
        for parent in parents {
            current = self.read_member(&Expr::Literal(current), parent, line)?;
        }
        self.read_member(&Expr::Literal(current), last, line)
    }

    /// 沿成员路径定位到「最后一层记录 + 字段名」。
    pub(super) fn record_parent(
        root: Value,
        path: &[String],
        line: usize,
    ) -> Result<(Rc<RefCell<VbRecord>>, String), RuntimeError> {
        let (last, parents) = path
            .split_last()
            .ok_or_else(|| RuntimeError::new("缺少成员名", line))?;
        let mut current = root;
        for step in parents {
            let record = current
                .as_record()
                .map_err(|_| RuntimeError::new(format!("`{step}` 所在的值不是记录类型"), line))?;
            current = {
                let record = record.borrow();
                record.get(step).ok_or_else(|| {
                    RuntimeError::new(
                        format!("类型 `{}` 没有字段 `{step}`", record.type_name()),
                        line,
                    )
                })?
            };
        }
        let record = current
            .as_record()
            .map_err(|_| RuntimeError::new(format!("不能访问 `.{last}`：不是记录类型"), line))?;
        Ok((record, last.clone()))
    }

    /// 沿成员路径定位到最后一层记录，再写入字段。
    pub(super) fn assign_member(
        &mut self,
        root: Value,
        path: &[String],
        value: Value,
        narrow: Option<VbType>,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let (parent, last) = Self::record_parent(root, path, line)?;
        parent
            .borrow_mut()
            .set_with_narrow(&last, value, narrow)
            .map_err(|message| RuntimeError::from_coerce(message, line))
    }

    /// `Print` / `Print #` 的文本渲染：处理 `,` 的 14 列分区与 `Spc`/`Tab` 补位。
    ///
    /// `start_column` 是当前行已经写出的字符数（`Print #n, ...;` 续行时不为 0）。
    pub(super) fn render_print(
        &mut self,
        values: &[Expr],
        seps: &[PrintSep],
        start_column: usize,
        line: usize,
    ) -> Result<String, RuntimeError> {
        let mut text = String::new();
        let mut column = start_column;
        for (index, expr) in values.iter().enumerate() {
            if index > 0 && matches!(seps.get(index - 1), Some(PrintSep::Comma)) {
                // `,` 把输出推进到下一个打印区的开头；正好站在区边界上时跳到再下一个。
                let next = (column / PRINT_ZONE + 1) * PRINT_ZONE;
                text.extend(std::iter::repeat_n(' ', next - column));
                column = next;
            }
            if let Expr::Call { name, args } = expr
                && (name == "spc" || name == "tab")
                && self.find_slot(name).is_none()
            {
                let Some(argument) = args.first() else {
                    return Err(RuntimeError::new(format!("`{name}` 缺少第 1 个参数"), line));
                };
                let target = self.eval(argument, line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("`{name}` 的参数必须是整数：{message}"), line)
                })?;
                let pad = if name == "spc" {
                    target.max(0) as usize
                } else {
                    // `Tab(n)`：移到第 n 列（1 基）。已经越过目标列时 VB6 会先换行再到第 n 列
                    // （实测 `"abc"; Tab(2); "z"` 是 `abc` 换行后 ` z`）。
                    let wanted = (target.max(1) - 1) as usize;
                    if wanted >= column {
                        let pad = wanted - column;
                        text.extend(std::iter::repeat_n(' ', pad));
                        column = wanted;
                        continue;
                    }
                    text.push('\n');
                    text.extend(std::iter::repeat_n(' ', wanted));
                    column = wanted;
                    continue;
                };
                text.extend(std::iter::repeat_n(' ', pad));
                column += pad;
                continue;
            }
            let value = self.eval(expr, line)?;
            // `Single` 按 7 位有效数字打印（VB6 实测）；其余类型仍走 `to_display`。
            let rendered = match &value {
                Value::Float(number) if self.static_type(expr) == Some(VbType::Single) => {
                    crate::interpreter::value::format_single(*number)
                }
                // VB6 实测：`Debug.Print 对象` / `Print #1, 对象` 报 450。
                Value::ComObject(_) => {
                    return Err(RuntimeError::new(
                        "对象没有可用的默认属性，不能转换为字符串",
                        line,
                    )
                    .with_number(450));
                }
                _ => value.to_display(),
            };
            // 边写边记列位置，避免每个值都重新数一遍整段文本（`Print` 参数很多时是 O(n²)）。
            column += rendered.chars().count();
            text.push_str(&rendered);
        }
        Ok(text)
    }

    pub(super) fn exec_do(
        &mut self,
        pre: Option<&(bool, Expr)>,
        post: Option<&(bool, Expr)>,
        body: &[Stmt],
        line: usize,
    ) -> Result<Flow, RuntimeError> {
        loop {
            self.tick(line)?;
            if let Some((positive, cond)) = pre
                && !self.cond_matches(*positive, cond, line)?
            {
                break;
            }
            match self.exec_block(body)? {
                Flow::Normal => {}
                Flow::ExitFor | Flow::ExitDo => break,
                other => return Ok(other),
            }
            if let Some((positive, cond)) = post
                && !self.cond_matches(*positive, cond, line)?
            {
                break;
            }
        }
        Ok(Flow::Normal)
    }

    pub(super) fn exec_select(
        &mut self,
        subject: &Expr,
        arms: &[(Vec<CasePattern>, Vec<Stmt>)],
        otherwise: Option<&Vec<Stmt>>,
        line: usize,
    ) -> Result<Flow, RuntimeError> {
        let subject = self.eval(subject, line)?;
        for (patterns, body) in arms {
            for pattern in patterns {
                let matched = match pattern {
                    CasePattern::Value(expr) => {
                        let value = self.eval(expr, line)?;
                        compare_values(&subject, &value, self.option_compare_text)
                            .map_err(|message| RuntimeError::new(message, line))?
                            == Ordering::Equal
                    }
                    CasePattern::Is(op, expr) => {
                        let value = self.eval(expr, line)?;
                        let ordering = compare_values(&subject, &value, self.option_compare_text)
                            .map_err(|message| RuntimeError::new(message, line))?;
                        match op {
                            BinOp::Eq => ordering == Ordering::Equal,
                            BinOp::NotEq => ordering != Ordering::Equal,
                            BinOp::Lt => ordering == Ordering::Less,
                            BinOp::Gt => ordering == Ordering::Greater,
                            BinOp::LtEq => ordering != Ordering::Greater,
                            BinOp::GtEq => ordering != Ordering::Less,
                            _ => false,
                        }
                    }
                    CasePattern::Range(first, last) => {
                        let low = self.eval(first, line)?;
                        let high = self.eval(last, line)?;
                        compare_values(&subject, &low, self.option_compare_text)
                            .map_err(|message| RuntimeError::new(message, line))?
                            != Ordering::Less
                            && compare_values(&subject, &high, self.option_compare_text)
                                .map_err(|message| RuntimeError::new(message, line))?
                                != Ordering::Greater
                    }
                };
                if matched {
                    return self.exec_block(body);
                }
            }
        }
        match otherwise {
            Some(body) => self.exec_block(body),
            None => Ok(Flow::Normal),
        }
    }

    pub(super) fn eval_condition(
        &mut self,
        cond: &Expr,
        line: usize,
    ) -> Result<bool, RuntimeError> {
        let value = self.eval(cond, line)?;
        // `If Nothing Then` 在 VB6 里是运行时错误 91（不是 `to_bool` 其它失败对应的 13）。
        if matches!(value, Value::Nothing) {
            return Err(RuntimeError::new(builtins::error_text(91), line).with_number(91));
        }
        value
            .to_bool()
            .map_err(|message| RuntimeError::from_coerce(message, line))
    }

    pub(super) fn cond_matches(
        &mut self,
        positive: bool,
        cond: &Expr,
        line: usize,
    ) -> Result<bool, RuntimeError> {
        let value = self.eval_condition(cond, line)?;
        Ok(if positive { value } else { !value })
    }
}
