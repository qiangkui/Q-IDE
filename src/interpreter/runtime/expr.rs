//! 表达式的求值（包含类型转换与运算符）。

use super::*;

impl<'p, 'o> Runtime<'p, 'o> {
    // ---------- 表达异 ----------

    pub(super) fn eval(&mut self, expr: &Expr, line: usize) -> Result<Value, RuntimeError> {
        if self.count_stats {
            self.stats.expressions += 1;
        }
        self.tick(line)?;
        match expr {
            Expr::Literal(value) => Ok(value.clone()),
            Expr::TypedLiteral { value, .. } => Ok(value.clone()),
            Expr::ArrayLiteral(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.eval(item, line)?);
                }
                Ok(Value::array(values))
            }
            Expr::NamedArg { .. } => Err(RuntimeError::new(
                "命名参数仅支持 COM Automation 方法调用",
                line,
            )),
            // 省略的实参在需要值时就是 `Empty`（内置函数、COM 调用走这里）；
            // 用户过程的绑定在 `bind_argument` 之前就把它识别成「调用时省略」。
            Expr::OmittedArg => Ok(Value::Empty),
            Expr::Var(name) => {
                if self.count_stats {
                    self.stats.variable_reads += 1;
                }
                if let Some(slot) = self.lookup_slot(name) {
                    return Ok(slot.value());
                }
                // `vbTrue` / `vbCrLf` 这类 VB 标准常量（VB6 里是 VBA 库的全局常量）。
                if let Some(value) = builtins::vb_constant(name) {
                    return Ok(value);
                }
                if self.procedures.contains_key(name)
                    || self.declares.contains_key(name)
                    || builtins::is_builtin(name)
                {
                    return self.call(name, &[], line);
                }
                Err(RuntimeError::new(format!("未定义的变量 `{name}`"), line))
            }
            Expr::Call { name, args } => self.call(name, args, line),
            Expr::Member { base, name } => self.read_member(base, name, line),
            Expr::MemberIndex {
                base,
                name,
                indices,
            } => {
                let root = self.eval(base, line)?;
                if let Value::Class(instance) = &root {
                    return self.call_class_method(&instance.0, name, indices, line);
                }
                if let Value::ComObject(object) = &root {
                    return self.call_com_member(object, name, indices, line);
                }
                if let Some(collection) = root.collection_rc() {
                    let key = match indices.as_slice() {
                        [single] => self.eval(single, line)?,
                        _ => {
                            return Err(RuntimeError::new(
                                "`Collection.Item` 需要一个下标或键",
                                line,
                            ));
                        }
                    };
                    let (value, narrow) = collection
                        .borrow()
                        .item_with_type(&key)
                        .map_err(|message| RuntimeError::new(message, line).with_number(9))?;
                    self.result_narrow = narrow;
                    return Ok(value);
                }
                if let Some(entries) = root.dictionary_rc() {
                    let key = match indices.as_slice() {
                        [single] => self.eval(single, line)?,
                        _ => {
                            return Err(RuntimeError::new("`Dictionary` 的方法需要一个键", line));
                        }
                    };
                    let key = VbDictionary::key_of(&key)
                        .map_err(|message| RuntimeError::new(message, line))?;
                    return match name.as_str() {
                        "exists" => Ok(Value::Bool(entries.borrow().exists(&key))),
                        "item" => entries
                            .borrow()
                            .get(&key)
                            .map_err(|message| RuntimeError::new(message, line).with_number(9)),
                        _ => Err(RuntimeError::new(
                            format!("`Dictionary` 没有方法 `{name}`"),
                            line,
                        )),
                    };
                }
                let indices = self.eval_indices(indices, line)?;
                let field = {
                    let record = root.as_record().map_err(|_| {
                        RuntimeError::new(format!("值不是记录类型，不能访问 `.{name}`"), line)
                    })?;
                    let record = record.borrow();
                    record.get(name).ok_or_else(|| {
                        RuntimeError::new(
                            format!("类型 `{}` 没有字段 `{name}`", record.type_name()),
                            line,
                        )
                    })?
                };
                let array = field
                    .as_array()
                    .map_err(|_| RuntimeError::new(format!("字段 `{name}` 不是数组"), line))?;
                let value = array.borrow().get(&indices).map_err(|message| {
                    RuntimeError::new(format!("字段 `{name}`：{message}"), line).with_number(9)
                })?;
                Ok(value)
            }
            Expr::WithMember { path } => self.read_with_member(path, line),
            Expr::NewObject(kind) => Ok(match kind {
                NewObject::Collection => Value::collection(),
                NewObject::Dictionary => Value::dictionary(),
                NewObject::Class(name) => self.instantiate(name, line)?,
            }),
            Expr::AddressOf(name) => {
                self.resolve_callback(name, line)?;
                Ok(Value::FuncPtr(name.clone()))
            }
            Expr::TypeOf { expr, class_name } => {
                let value = self.eval(expr, line)?;
                let matched = match &value {
                    Value::Class(instance) => instance
                        .0
                        .borrow()
                        .type_name()
                        .eq_ignore_ascii_case(class_name),
                    Value::ComObject(_) => class_name == "object",
                    Value::Empty | Value::Nothing => false,
                    _ => {
                        return Err(
                            RuntimeError::new("`TypeOf` 只能用于对象", line).with_number(424)
                        );
                    }
                };
                Ok(Value::Bool(matched))
            }
            Expr::Unary { op, expr } => {
                let value = self.eval(expr, line)?;
                let before = match &value {
                    Value::Int(number) => Some(*number),
                    _ => None,
                };
                let negated = match op {
                    UnOp::Negate => match value {
                        Value::Float(number) => Value::Float(-number),
                        Value::Date(number) => Value::Date(-number),
                        Value::Currency(number) => {
                            Value::Currency(number.checked_neg().ok_or_else(|| {
                                RuntimeError::new("数值溢出", line).with_number(6)
                            })?)
                        }
                        other => {
                            let number = other
                                .as_int()
                                .map_err(|message| RuntimeError::new(message, line))?;
                            Value::Int(number.wrapping_neg())
                        }
                    },
                    UnOp::Not => match value {
                        Value::Bool(flag) => Value::Bool(!flag),
                        other => {
                            let number = other
                                .as_int()
                                .map_err(|message| RuntimeError::new(message, line))?;
                            Value::Int(!number)
                        }
                    },
                };
                // `-i`（`i As Integer` = -32768）在 VB6 里报错误号 6（实测）。
                if *op == UnOp::Negate
                    && let (Some(number), Value::Int(after)) = (before, &negated)
                    && fits_i16(number)
                    && !fits_i16(*after)
                    && self.static_type(expr) == Some(VbType::Integer)
                {
                    return Err(RuntimeError::new("溢出", line).with_number(6));
                }
                Ok(negated)
            }
            Expr::Binary { op, left, right } => {
                let mut left_value = self.eval(left, line)?;
                let mut right_value = self.eval(right, line)?;
                // 拼字符串时 `Single` 也按 7 位有效数字写（VB6 实测 `"v=" & CSng(1.23456789)` 是 `v=1.234568`）；
                // 只有真的在拼浮点数时才去查静态类型，字符串拼接的热路径不受影响。
                if *op == BinOp::Concat {
                    self.render_single(left, &mut left_value);
                    self.render_single(right, &mut right_value);
                }
                let mut overflow = None;
                let value = apply_binary(
                    *op,
                    left_value,
                    right_value,
                    line,
                    self.option_compare_text,
                    &mut overflow,
                )?;
                // VB6 的 Integer / Long 运算溢出报错误号 6（实测）：`apply_binary` 只在
                // 「操作数都装得进窄类型、结果却装不进去」时给候选，这里再核实一下静态类型。
                if let Some(narrow) = overflow
                    && Self::binary_static_type(
                        *op,
                        self.static_type(left),
                        self.static_type(right),
                    ) == Some(narrow)
                {
                    return Err(RuntimeError::new("溢出", line).with_number(6));
                }
                Ok(value)
            }
        }
    }
}
