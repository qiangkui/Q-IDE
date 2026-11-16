//! 各条 VB 语句的执行。

use super::*;

impl<'p, 'o> Runtime<'p, 'o> {
    // ---------- 语句 ----------

    pub(super) fn exec_block(&mut self, stmts: &[Stmt]) -> Result<Flow, RuntimeError> {
        let is_root = self.enter_block();
        let outcome = self.exec_block_inner(stmts, is_root);
        self.exit_block();
        outcome
    }

    pub(super) fn exec_block_inner(
        &mut self,
        stmts: &[Stmt],
        is_root: bool,
    ) -> Result<Flow, RuntimeError> {
        let mut index = 0usize;
        let mut gosub_returns = Vec::new();
        while index < stmts.len() {
            // COM 事件在语句边界分派：回调里只排队，避免重入解释器。
            self.pump_com_events()?;
            match self.exec(&stmts[index]) {
                Ok(Flow::Normal) => index += 1,
                Ok(Flow::Goto(label)) => match find_label(stmts, &label) {
                    Some(position) => index = position,
                    None if is_root => {
                        return Err(RuntimeError::new(
                            format!("找不到标签 `{label}`"),
                            stmt_line(&stmts[index]),
                        ));
                    }
                    None => return Ok(Flow::Goto(label)),
                },
                Ok(Flow::Gosub(label)) => match find_label(stmts, &label) {
                    Some(position) if is_root => {
                        gosub_returns.push(index + 1);
                        index = position;
                    }
                    Some(_) => return Ok(Flow::Gosub(label)),
                    None if is_root => {
                        return Err(RuntimeError::new(
                            format!("找不到标签 `{label}`"),
                            stmt_line(&stmts[index]),
                        ));
                    }
                    None => return Ok(Flow::Gosub(label)),
                },
                Ok(Flow::GosubReturn) if is_root => {
                    let Some(return_to) = gosub_returns.pop() else {
                        return Err(RuntimeError::new(
                            "`Return` 没有匹配的 `GoSub`",
                            stmt_line(&stmts[index]),
                        ));
                    };
                    index = return_to;
                }
                Ok(Flow::GosubReturn) => return Ok(Flow::GosubReturn),
                Ok(Flow::ResumeNext(at)) => {
                    if is_root {
                        index = at + 1;
                    } else {
                        return Ok(Flow::ResumeNext(at));
                    }
                }
                Ok(Flow::ResumeAt(at)) => {
                    if is_root {
                        index = at;
                    } else {
                        return Ok(Flow::ResumeAt(at));
                    }
                }
                Ok(flow) => return Ok(flow),
                Err(error) => match self.error_handler() {
                    Some(ErrorMode::ResumeNext) => {
                        self.last_error = Some(error);
                        index += 1;
                    }
                    Some(ErrorMode::Goto(label)) if is_root => {
                        let line = error.line;
                        self.last_error = Some(error);
                        self.set_resume_index(index);
                        match find_label(stmts, &label) {
                            Some(position) => index = position,
                            None => {
                                return Err(RuntimeError::new(
                                    format!("找不到标签 `{label}`"),
                                    line,
                                ));
                            }
                        }
                    }
                    // 嵌套块里遇到 `On Error GoTo`：交给根块统一跳转。
                    Some(ErrorMode::Goto(_)) => return Err(error),
                    // 没有人处理：脚本本来要中止。调试器开着「出错时中断」时先停在出错行，
                    // 让用户看一眼变量再决定继续（继续后照样抛错、脚本结束）。
                    None | Some(ErrorMode::Disabled) => {
                        if is_root {
                            self.debug_break_on_error(&error)?;
                        }
                        return Err(error);
                    }
                },
            }
        }
        Ok(Flow::Normal)
    }

    pub(super) fn exec(&mut self, stmt: &Stmt) -> Result<Flow, RuntimeError> {
        let line = stmt_line(stmt);
        if self.count_stats {
            self.stats.statements += 1;
        }
        if self.trace {
            (self.out)(&format!(
                "{} 第 {line} 行",
                crate::interpreter::TRACE_PREFIX
            ));
        }
        self.tick(line)?;
        self.debug_check(line)?;

        match stmt {
            Stmt::Dim {
                items,
                static_,
                line,
            } => {
                // `Static Sub` / `Static Function` 里的局部变量全是静态的。
                let static_ = *static_ || self.current_procedure_is_static();
                for item in items {
                    let (value, slot_ty) = self.declared_value(item, *line)?;
                    if static_ {
                        if !self.bind_static_local(&item.name, value, slot_ty, *line, None)? {
                            continue;
                        }
                    } else if !self.declare_slot(&item.name, value.clone(), slot_ty, *line)? {
                        // 同一条 `Dim` 又跑了一遍（循环体里）：变量保留，初值重新算。
                        if item.initializer.is_some() {
                            self.set_var(&item.name, value, *line, None)?;
                        }
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Redim {
                name,
                bounds,
                ty,
                preserve,
                line,
            } => {
                let dims = self.array_dims(bounds, *line)?;
                let slot = self
                    .find_slot(name)
                    .ok_or_else(|| RuntimeError::new(format!("未定义的数组 `{name}`"), *line))?;
                if slot.is_readonly() {
                    return Err(RuntimeError::new(
                        format!("不能给常量 `{name}` 赋值"),
                        *line,
                    ));
                }
                let value = slot.value();
                let array = value
                    .as_array()
                    .map_err(|message| RuntimeError::new(format!("`{name}` {message}"), *line))?;
                let mut array = array.borrow_mut();
                if !array.is_dynamic() {
                    return Err(RuntimeError::new(
                        format!("`{name}` 是静态数组，声明时已固定大小，不能用 ReDim"),
                        *line,
                    ));
                }
                if *preserve && !array.dims().is_empty() {
                    let old = array.dims();
                    if old.len() != dims.len() {
                        return Err(RuntimeError::new(
                            format!("`ReDim Preserve` 不能改变 `{name}` 的维数"),
                            *line,
                        ));
                    }
                    // 与 VB 一致：`Preserve` 只能调整最后一维的大小。
                    if old[..old.len() - 1] != dims[..dims.len() - 1] {
                        return Err(RuntimeError::new(
                            format!("`ReDim Preserve` 只能改变 `{name}` 最后一维的大小"),
                            *line,
                        )
                        .with_number(9));
                    }
                }
                array.redim(dims, *ty, *preserve);
                Ok(Flow::Normal)
            }
            Stmt::Erase { names, line } => {
                for name in names {
                    let slot = self.find_slot(name).ok_or_else(|| {
                        RuntimeError::new(format!("未定义的数组 `{name}`"), *line)
                    })?;
                    if slot.is_readonly() {
                        return Err(RuntimeError::new(
                            format!("不能给常量 `{name}` 赋值"),
                            *line,
                        ));
                    }
                    let value = slot.value();
                    let array = value.as_array().map_err(|message| {
                        RuntimeError::new(format!("`{name}` {message}"), *line)
                    })?;
                    array.borrow_mut().erase();
                }
                Ok(Flow::Normal)
            }
            Stmt::Const {
                name,
                ty,
                value,
                line,
            } => {
                let value = self.eval(value, *line)?;
                let coerced = value
                    .coerce(*ty)
                    .map_err(|message| RuntimeError::from_coerce(message, *line))?;
                self.scope_mut().insert(
                    std::borrow::Cow::Owned(name.clone()),
                    Slot::constant(coerced, *ty),
                );
                Ok(Flow::Normal)
            }
            Stmt::Assign {
                target,
                value,
                suffix,
                line,
            } => {
                // `Property Let` 很少见：没有任何 Let 过程时跳过这次变量查找。
                if !self.lets.is_empty()
                    && self.find_slot(target).is_none()
                    && let Some(procedure) = self.lets.get(target).copied()
                {
                    return self.call_property_let(procedure, &[], value, *line);
                }
                let evaluated = self.eval(value, *line)?;
                // 只有带类型后缀（`x$ = ...`）才可能在这里隐式声明。
                if !self.option_explicit
                    && suffix.is_some()
                    && self.find_slot(target).is_none()
                    && let Some(ty) = suffix.and_then(VbType::from_suffix)
                {
                    self.declare(target, ty, *line)?;
                }
                // `Variant` 变量要记下右侧表达式的静态类型（见 `Slot::narrow_type`）。
                self.set_var(target, evaluated, *line, Some(value))?;
                Ok(Flow::Normal)
            }
            Stmt::AssignIndex {
                name,
                indices,
                value: rhs,
                line,
            } => {
                if !self.lets.is_empty()
                    && self.find_slot(name).is_none()
                    && let Some(procedure) = self.lets.get(name).copied()
                {
                    return self.call_property_let(procedure, indices, rhs, *line);
                }
                let value = self.eval(rhs, *line)?;
                let slot = self
                    .find_slot(name)
                    .ok_or_else(|| RuntimeError::new(format!("未定义的变量 `{name}`"), *line))?;
                if slot.is_readonly() {
                    return Err(RuntimeError::new(
                        format!("不能给常量 `{name}` 赋值"),
                        *line,
                    ));
                }
                let current = slot.value();
                // 数组是最常见的情形，先判它：`as_dictionary()` 在「不是字典」时会构造一条错误字符串，
                // 放在前面等于每次数组赋值都白分配一次。
                if let Value::Array(array) = &current {
                    let indices = self.eval_indices(indices, *line)?;
                    if self.count_stats {
                        self.stats.array_accesses += 1;
                    }
                    // `Variant` 数组要记下元素的内容类型（`arr(0) = CByte(9)` 之后 `VarType(arr(0))` 是 17）。
                    let narrow = if array.borrow().element_type() == VbType::Variant {
                        self.narrow_of(Some(rhs))
                    } else {
                        None
                    };
                    array
                        .borrow_mut()
                        .set(&indices, value, narrow)
                        .map_err(|message| {
                            if message.contains("溢出") {
                                RuntimeError::from_coerce(
                                    format!("数组 `{name}`：{message}"),
                                    *line,
                                )
                            } else {
                                RuntimeError::new(format!("数组 `{name}`：{message}"), *line)
                                    .with_number(9)
                            }
                        })?;
                    return Ok(Flow::Normal);
                }
                if let Some(entries) = current.dictionary_rc() {
                    // `d("键") = 值`：字典按键写入（不存在则新增）。
                    let key = match indices.as_slice() {
                        [single] => self.eval(single, *line)?,
                        _ => {
                            return Err(RuntimeError::new(
                                "`Dictionary` 的下标只能是一个键",
                                *line,
                            ));
                        }
                    };
                    let key = VbDictionary::key_of(&key)
                        .map_err(|message| RuntimeError::new(message, *line))?;
                    let narrow = self.narrow_of(Some(rhs));
                    entries.borrow_mut().set(key, value, narrow);
                    return Ok(Flow::Normal);
                }
                if let Value::ComObject(object) = &current {
                    // `d("键") = 值`：COM 对象的默认成员（`DISPID_VALUE`）写入（VB6 实测可用）。
                    let mut index_values = Vec::with_capacity(indices.len());
                    for expr in indices {
                        index_values.push(self.eval(expr, *line)?);
                    }
                    object
                        .set_default(&index_values, &value)
                        .map_err(|failure| {
                            RuntimeError::new(failure.message, *line).with_number(failure.number)
                        })?;
                    return Ok(Flow::Normal);
                }
                let _ = self.eval_indices(indices, *line)?;
                // 其余类型（数字/字符串/空）按原样报「不是数组」，与改动前一致。
                let message = current.as_array().err().unwrap_or_default();
                Err(RuntimeError::new(format!("`{name}` {message}"), *line))
            }
            Stmt::AssignMember {
                base,
                path,
                value,
                set_op,
                line,
            } => {
                let root = self.eval(base, *line)?;
                if matches!(root, Value::ComObject(_)) {
                    // `obj.Sub.Prop = v`：中间层按属性读出来，赋值落在最后一层。
                    let mut target = root.clone();
                    for member in &path[..path.len().saturating_sub(1)] {
                        target = self.read_member(&Expr::Literal(target), member, *line)?;
                    }
                    let last = path.last().cloned().unwrap_or_default();
                    let value = self.eval(value, *line)?;
                    let Value::ComObject(object) = &target else {
                        return Err(RuntimeError::new(
                            format!("`{last}` 所在的值不是 COM 对象"),
                            *line,
                        )
                        .with_number(424));
                    };
                    object.set(&last, &value).map_err(|failure| {
                        RuntimeError::new(failure.message, *line).with_number(failure.number)
                    })?;
                    return Ok(Flow::Normal);
                }
                if let Value::Class(instance) = &root {
                    self.assign_instance_member(&instance.0, path, value, *set_op, *line)?;
                    return Ok(Flow::Normal);
                }
                if !root.is_record() {
                    return Err(RuntimeError::new("成员赋值的目标不是记录类型", *line));
                }
                // `Variant` 字段要记下右侧表达式的静态类型（`u.f = i` 之后 `VarType(u.f)` 是 2）。
                let narrow = self.narrow_of(Some(value));
                let value = self.eval(value, *line)?;
                self.assign_member(root, path, value, narrow, *line)?;
                Ok(Flow::Normal)
            }
            Stmt::AssignMemberIndex {
                base,
                path,
                indices,
                member_path,
                value,
                set_op,
                line,
            } => {
                let root = self.eval(base, *line)?;
                if matches!(root, Value::ComObject(_)) {
                    // 只有一级（`d.Item("k") = v`）时走索引属性写入；
                    // 后面还有成员（`rs.Fields("id").Value = v`）就把这一级当调用读出对象再往下写。
                    if member_path.is_empty() {
                        let mut target = root.clone();
                        for member in &path[..path.len().saturating_sub(1)] {
                            target = self.read_member(&Expr::Literal(target), member, *line)?;
                        }
                        let Some(member) = path.last().cloned() else {
                            return Err(RuntimeError::new("缺少成员名", *line));
                        };
                        let Value::ComObject(com) = &target else {
                            return Err(RuntimeError::new("COM 索引属性的目标不是对象", *line));
                        };
                        let mut index_values = Vec::with_capacity(indices.len());
                        for expr in indices {
                            index_values.push(self.eval(expr, *line)?);
                        }
                        let value = self.eval(value, *line)?;
                        com.set_indexed(&member, &index_values, &value)
                            .map_err(|failure| {
                                RuntimeError::new(failure.message, *line)
                                    .with_number(failure.number)
                            })?;
                        return Ok(Flow::Normal);
                    }
                    let container = self.read_member_chain(root, path, indices, *line)?;
                    self.assign_through_members(container, member_path, value, *set_op, *line)?;
                    return Ok(Flow::Normal);
                }
                if let Value::Class(instance) = &root {
                    if member_path.is_empty() {
                        self.assign_instance_index(
                            &instance.0,
                            path.first().map(String::as_str).unwrap_or_default(),
                            indices,
                            value,
                            *set_op,
                            *line,
                        )?;
                    } else {
                        // `obj.Method(实参).Prop = v` / `obj.Prop(1).Sub = v`：先调用再赋值。
                        let container = self.read_member_chain(root, path, indices, *line)?;
                        self.assign_through_members(container, member_path, value, *set_op, *line)?;
                    }
                    return Ok(Flow::Normal);
                }
                let indices = self.eval_indices(indices, *line)?;
                let value = self.eval(value, *line)?;
                let (parent, last) = Self::record_parent(root, path, *line)?;
                let field = {
                    let parent = parent.borrow();
                    parent.get(&last).ok_or_else(|| {
                        RuntimeError::new(
                            format!("类型 `{}` 没有字段 `{last}`", parent.type_name()),
                            *line,
                        )
                    })?
                };
                let array = field
                    .as_array()
                    .map_err(|_| RuntimeError::new(format!("字段 `{last}` 不是数组"), *line))?;
                if member_path.is_empty() {
                    array
                        .borrow_mut()
                        .set(&indices, value, None)
                        .map_err(|message| {
                            if message.contains("溢出") {
                                RuntimeError::from_coerce(
                                    format!("字段 `{last}`：{message}"),
                                    *line,
                                )
                            } else {
                                RuntimeError::new(format!("字段 `{last}`：{message}"), *line)
                                    .with_number(9)
                            }
                        })?;
                } else {
                    let element = array.borrow().get(&indices).map_err(|message| {
                        RuntimeError::new(format!("字段 `{last}`：{message}"), *line).with_number(9)
                    })?;
                    if !element.is_record() {
                        return Err(RuntimeError::new(
                            format!("字段 `{last}` 的元素不是记录类型"),
                            *line,
                        ));
                    }
                    self.assign_member(element, member_path, value, None, *line)?;
                }
                Ok(Flow::Normal)
            }
            Stmt::AssignWithMember {
                path,
                indices,
                member_path,
                value,
                set_op,
                line,
            } => {
                let Some(root) = self.with_stack.last().cloned() else {
                    return Err(RuntimeError::new(
                        "`.成员` 赋值只能出现在 `With` 块内",
                        *line,
                    ));
                };
                if indices.is_empty() && member_path.is_empty() {
                    match &root {
                        Value::ComObject(object) if path.len() == 1 => {
                            let value = self.eval(value, *line)?;
                            object.set(&path[0], &value).map_err(|failure| {
                                RuntimeError::new(failure.message, *line)
                                    .with_number(failure.number)
                            })?;
                        }
                        Value::ComObject(_) => {
                            // `With rs : .Fields.Value = v`：中间层按属性读出来再设最后一层。
                            let Some((last, parents)) = path.split_last() else {
                                return Err(RuntimeError::new("缺少成员名", *line));
                            };
                            let container = self.read_member_chain(root, parents, &[], *line)?;
                            self.assign_through_members(
                                container,
                                std::slice::from_ref(last),
                                value,
                                *set_op,
                                *line,
                            )?;
                        }
                        Value::Class(instance) => {
                            self.assign_instance_member(&instance.0, path, value, *set_op, *line)?;
                        }
                        _ => {
                            let value = self.eval(value, *line)?;
                            self.assign_member(root, path, value, None, *line)?;
                        }
                    }
                    return Ok(Flow::Normal);
                }
                // `.Prop(1) = v` / `.Fields("id").Value = v`：先走完前几层成员。
                let Some((member, parents)) = path.split_last() else {
                    return Err(RuntimeError::new("缺少成员名", *line));
                };
                let mut container = root;
                for step in parents {
                    container = self.read_member(&Expr::Literal(container), step, *line)?;
                }
                if member_path.is_empty() {
                    match &container {
                        Value::ComObject(object) => {
                            let mut values = Vec::with_capacity(indices.len());
                            for expr in indices {
                                values.push(self.eval(expr, *line)?);
                            }
                            let value = self.eval(value, *line)?;
                            object
                                .set_indexed(member, &values, &value)
                                .map_err(|failure| {
                                    RuntimeError::new(failure.message, *line)
                                        .with_number(failure.number)
                                })?;
                        }
                        Value::Class(instance) => {
                            self.assign_instance_index(
                                &instance.0,
                                member,
                                indices,
                                value,
                                *set_op,
                                *line,
                            )?;
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                format!("`.{member}(...)` 赋值的对象不是类或 COM 对象"),
                                *line,
                            )
                            .with_number(438));
                        }
                    }
                } else {
                    let nested = self.read_member_step(container, member, indices, *line)?;
                    self.assign_through_members(nested, member_path, value, *set_op, *line)?;
                }
                Ok(Flow::Normal)
            }
            Stmt::WithCall { path, args, line } => {
                let Some(mut receiver) = self.with_stack.last().cloned() else {
                    return Err(RuntimeError::new(
                        "`.成员` 调用只能出现在 `With` 块内",
                        *line,
                    ));
                };
                let Some((method, parents)) = path.split_last() else {
                    return Err(RuntimeError::new("缺少成员名", *line));
                };
                for parent in parents {
                    receiver = self.read_member(&Expr::Literal(receiver), parent, *line)?;
                }
                match receiver {
                    Value::Class(instance) => {
                        self.call_class_method(&instance.0, method, args, *line)?;
                    }
                    Value::ComObject(object) => {
                        self.call_com_member(&object, method, args, *line)?;
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            format!("`.{method}` 的接收对象不支持方法调用"),
                            *line,
                        ));
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::With {
                subject,
                body,
                line,
            } => {
                let value = self.eval(subject, *line)?;
                if !value.is_record() && !value.is_object() {
                    return Err(RuntimeError::new("`With` 需要记录或对象值", *line));
                }
                self.with_stack.push(value);
                let outcome = self.exec_block(body);
                self.with_stack.pop();
                outcome
            }
            Stmt::Call {
                name,
                args,
                by_value_args,
                line,
            } => {
                self.call_with_by_value_args(name, args, by_value_args, *line)?;
                Ok(Flow::Normal)
            }
            Stmt::Set {
                target,
                value,
                line,
            } => {
                let value = self.eval(value, *line)?;
                if !value.is_collection()
                    && !value.is_dictionary()
                    && !matches!(
                        value,
                        Value::Class(_)
                            | Value::ComObject(_)
                            | Value::Picture(_)
                            | Value::Empty
                            | Value::Nothing
                    )
                {
                    return Err(RuntimeError::new(
                        "`Set` 目前支持 `Collection`、`Dictionary`、`New 类名`、COM 对象、`LoadPicture` 与 `Nothing`",
                        *line,
                    ));
                }
                if self.find_slot(target).is_none() {
                    // `target` 借自语句本身，活不过 `'p`，这里复制一份名字。
                    self.declare(target, VbType::Object, *line)?;
                }
                self.connect_events_for_name(target, &value, *line)?;
                self.set_var(target, value, *line, None)?;
                Ok(Flow::Normal)
            }
            Stmt::CallMethod {
                object,
                path,
                args,
                line,
            } => {
                let method = path.last().cloned().unwrap_or_default();
                let slot = self.find_slot(object);
                let Some(slot) = slot else {
                    // `Bump .Y`（过程调用 + `With` 相对实参）与 `obj.Member` 的记号完全一样，
                    // 这里按「`object` 不是变量」回退成普通调用。
                    let call_args = if args.is_empty() {
                        vec![Expr::WithMember { path: path.clone() }]
                    } else {
                        args.clone()
                    };
                    self.call(object, &call_args, *line)?;
                    return Ok(Flow::Normal);
                };
                // `rs.Fields.Append "x", 3`：先逐层读出中间成员，再在最后一层上调用。
                let mut target = slot.value();
                for member in &path[..path.len().saturating_sub(1)] {
                    target = self.read_member(&Expr::Literal(target), member, *line)?;
                }
                if let Value::Class(instance) = &target {
                    match self.call_class_method(&instance.0, &method, args, *line) {
                        Ok(_) => return Ok(Flow::Normal),
                        Err(error) => {
                            // `h.Value`（实例字段）单独成句：VB6 会报「缺少子程序、函数或属性」，
                            // QIDE 宽容地读一次、丢掉结果。没有这个方法时按字段读一次。
                            if !args.is_empty()
                                || self
                                    .read_member(&Expr::Literal(target.clone()), &method, *line)
                                    .is_err()
                            {
                                return Err(error);
                            }
                            return Ok(Flow::Normal);
                        }
                    }
                }
                if let Value::ComObject(object) = &target {
                    self.call_com_member(object, &method, args, *line)?;
                    return Ok(Flow::Normal);
                }
                let mut values = Vec::with_capacity(args.len());
                for expr in args {
                    values.push(self.eval(expr, *line)?);
                }
                if let Some(collection) = target.collection_rc() {
                    match method.as_str() {
                        "add" => {
                            let Some(item) = values.first() else {
                                return Err(RuntimeError::new("`Add` 缺少要添加的元素", *line));
                            };
                            if values.len() > 2 {
                                return Err(RuntimeError::new(
                                    "`Add` 只支持「元素, 键」两个参数",
                                    *line,
                                ));
                            }
                            let key = values.get(1).map(|value| value.to_display());
                            let narrow = args.first().and_then(|expr| self.narrow_of(Some(expr)));
                            collection
                                .borrow_mut()
                                .add(item.clone(), key, narrow)
                                .map_err(|message| RuntimeError::new(message, *line))?;
                        }
                        "remove" => {
                            let Some(key) = values.first() else {
                                return Err(RuntimeError::new("`Remove` 缺少下标或键", *line));
                            };
                            collection.borrow_mut().remove(key).map_err(|message| {
                                RuntimeError::new(message, *line).with_number(9)
                            })?;
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                format!("`Collection` 没有方法 `{method}`"),
                                *line,
                            ));
                        }
                    }
                    return Ok(Flow::Normal);
                }
                if let Some(entries) = target.dictionary_rc() {
                    match method.as_str() {
                        "add" => {
                            if values.len() != 2 {
                                return Err(RuntimeError::new(
                                    "`Dictionary.Add` 需要「键, 值」两个参数",
                                    *line,
                                ));
                            }
                            let key = VbDictionary::key_of(&values[0])
                                .map_err(|message| RuntimeError::new(message, *line))?;
                            let narrow = args.get(1).and_then(|expr| self.narrow_of(Some(expr)));
                            entries
                                .borrow_mut()
                                .add(key, values[1].clone(), narrow)
                                .map_err(|message| RuntimeError::new(message, *line))?;
                        }
                        "remove" => {
                            let Some(key) = values.first() else {
                                return Err(RuntimeError::new("`Remove` 缺少键", *line));
                            };
                            let key = VbDictionary::key_of(key)
                                .map_err(|message| RuntimeError::new(message, *line))?;
                            entries.borrow_mut().remove(&key).map_err(|message| {
                                RuntimeError::new(message, *line).with_number(9)
                            })?;
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                format!("`Dictionary` 没有方法 `{method}`"),
                                *line,
                            ));
                        }
                    }
                    return Ok(Flow::Normal);
                }
                // `p.X` 这种「裸字段读取」单独成句：VB6 会报「缺少子程序、函数或属性」，
                // QIDE 宽容地按「读出来再丢掉」处理。只在没有实参时回退
                // （`x.Add 1` 仍报「不是 Collection」）。
                if args.is_empty() {
                    let mut value = slot.value();
                    let mut read = true;
                    for member in path {
                        match self.read_member(&Expr::Literal(value), member, *line) {
                            Ok(next) => value = next,
                            Err(_) => {
                                read = false;
                                break;
                            }
                        }
                    }
                    if read {
                        return Ok(Flow::Normal);
                    }
                }
                Err(RuntimeError::new(
                    format!("`{object}` 不是 `Collection` 或 `Dictionary`，不能调用 `{method}`"),
                    *line,
                ))
            }
            Stmt::Print {
                values,
                seps,
                suppress_newline,
                line,
            } => {
                let _ = suppress_newline; // `Debug.Print` 每次输出一整行
                let text = self.render_print(values, seps, 0, *line)?;
                (self.out)(&text);
                Ok(Flow::Normal)
            }
            Stmt::If {
                branches,
                otherwise,
                line,
            } => {
                for (cond, body) in branches {
                    if self.eval_condition(cond, *line)? {
                        return self.exec_block(body);
                    }
                }
                match otherwise {
                    Some(body) => self.exec_block(body),
                    None => Ok(Flow::Normal),
                }
            }
            Stmt::For {
                var,
                start,
                end,
                step,
                body,
                line,
            } => self.exec_for(var, start, end, step.as_ref(), body, *line),
            Stmt::ForEach {
                var,
                collection,
                body,
                line,
            } => self.exec_for_each(var, collection, body, *line),
            Stmt::DoLoop {
                pre,
                post,
                body,
                line,
            } => self.exec_do(pre.as_ref(), post.as_ref(), body, *line),
            Stmt::While { cond, body, line } => {
                loop {
                    self.tick(*line)?;
                    if !self.eval_condition(cond, *line)? {
                        break;
                    }
                    match self.exec_block(body)? {
                        Flow::Normal => {}
                        Flow::ExitFor | Flow::ExitDo => break,
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Select {
                subject,
                arms,
                otherwise,
                line,
            } => self.exec_select(subject, arms, otherwise.as_ref(), *line),
            Stmt::Exit { kind, .. } => Ok(match kind {
                ExitKind::For => Flow::ExitFor,
                ExitKind::Do => Flow::ExitDo,
                ExitKind::Sub | ExitKind::Function => Flow::ExitProc,
            }),
            Stmt::Return { value, line } => {
                let value = match value {
                    Some(expr) => self.eval(expr, *line)?,
                    None => Value::Empty,
                };
                if let Some(slot) = self.frames.last().and_then(|frame| frame.result.clone()) {
                    slot.set(value)
                        .map_err(|message| RuntimeError::from_coerce(message, *line))?;
                }
                Ok(Flow::ExitProc)
            }
            Stmt::GosubReturn { .. } => Ok(Flow::GosubReturn),
            Stmt::Halt { .. } => Ok(Flow::Halt),
            // 单独成句的表达式：求值后丢掉结果（比 VB6 宽，VB6 `/make` 实测这类写法编译不过）。
            Stmt::Eval { expr, line } => {
                self.eval(expr, *line)?;
                Ok(Flow::Normal)
            }
            Stmt::Label(..) => Ok(Flow::Normal),
            Stmt::Goto(label, _) => Ok(Flow::Goto(label.clone())),
            Stmt::Gosub(label, _) => Ok(Flow::Gosub(label.clone())),
            Stmt::OnError { mode, .. } => {
                self.set_error_handler(Some(mode.clone()));
                // VB6 里执行任何 `On Error` 语句都会把 `Err` 清空（本机实测：
                // `On Error Resume Next` / `GoTo 0` / `GoTo -1` 都清）。
                self.last_error = None;
                Ok(Flow::Normal)
            }
            Stmt::Resume { target, line } => match target {
                ResumeTarget::Label(label) => Ok(Flow::Goto(label.clone())),
                ResumeTarget::Next => match self.resume_index() {
                    Some(index) => Ok(Flow::ResumeNext(index)),
                    None => Err(RuntimeError::new("`Resume Next` 之前没有发生错误", *line)),
                },
                ResumeTarget::Current => match self.resume_index() {
                    Some(index) => Ok(Flow::ResumeAt(index)),
                    None => Err(RuntimeError::new("`Resume` 之前没有发生错误", *line)),
                },
            },
            Stmt::ErrClear { .. } => {
                self.last_error = None;
                Ok(Flow::Normal)
            }
            Stmt::OpenFile {
                path,
                mode,
                number,
                record_len,
                line,
            } => {
                let path = self.eval(path, *line)?.to_display();
                let resolved = self.resolve_path(&path);
                let number = match number {
                    Some(number) => self.file_number(number, *line)?,
                    None => self.free_file_number(),
                };
                if self.files.contains_key(&number) {
                    return Err(
                        RuntimeError::new(format!("文件号 #{number} 已经被占用"), *line)
                            .with_number(55),
                    );
                }
                let record_len = match record_len {
                    Some(expr) => {
                        if *mode != FileMode::Random {
                            return Err(RuntimeError::new("`Len =` 只能用于 `For Random`", *line));
                        }
                        let value = self.eval(expr, *line)?.as_int().map_err(|message| {
                            RuntimeError::new(format!("记录长度必须是整数：{message}"), *line)
                        })?;
                        if !(1..=32767).contains(&value) {
                            return Err(RuntimeError::new(
                                format!("记录长度 {value} 超出范围（1..32767）"),
                                *line,
                            ));
                        }
                        Some(value as usize)
                    }
                    None if *mode == FileMode::Random => Some(DEFAULT_RECORD_LEN),
                    None => None,
                };
                let handle = match mode {
                    FileMode::Input => {
                        let file = File::open(&resolved).map_err(|error| {
                            RuntimeError::new(format!("无法打开 `{path}`：{error}"), *line)
                                .with_number(open_error_number(&error, &resolved))
                        })?;
                        FileHandle::Reader(BufReader::new(file))
                    }
                    FileMode::Output => {
                        let file = File::create(&resolved).map_err(|error| {
                            RuntimeError::new(format!("无法创建 `{path}`：{error}"), *line)
                                .with_number(open_error_number(&error, &resolved))
                        })?;
                        FileHandle::Writer(FileWriter::new(file, 2))
                    }
                    FileMode::Append => {
                        let file = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&resolved)
                            .map_err(|error| {
                                RuntimeError::new(format!("无法追加 `{path}`：{error}"), *line)
                                    .with_number(open_error_number(&error, &resolved))
                            })?;
                        FileHandle::Writer(FileWriter::new(file, 8))
                    }
                    FileMode::Binary | FileMode::Random => {
                        let file = OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create(true)
                            .truncate(false)
                            .open(&resolved)
                            .map_err(|error| {
                                RuntimeError::new(format!("无法打开 `{path}`：{error}"), *line)
                                    .with_number(open_error_number(&error, &resolved))
                            })?;
                        FileHandle::Binary(BinaryFile::new(file, record_len))
                    }
                };
                self.files.insert(number, handle);
                Ok(Flow::Normal)
            }
            Stmt::CloseFile { numbers, line } => {
                if numbers.is_empty() {
                    for (_, handle) in self.files.drain() {
                        flush_file(handle);
                    }
                    return Ok(Flow::Normal);
                }
                for expr in numbers {
                    let number = self.file_number(expr, *line)?;
                    let Some(handle) = self.files.remove(&number) else {
                        return Err(
                            RuntimeError::new(format!("文件号 #{number} 没有打开"), *line)
                                .with_number(52),
                        );
                    };
                    flush_file(handle);
                }
                Ok(Flow::Normal)
            }
            Stmt::ResetFiles { .. } => {
                for (_, handle) in self.files.drain() {
                    flush_file(handle);
                }
                Ok(Flow::Normal)
            }
            Stmt::Nop { .. } => Ok(Flow::Normal),
            Stmt::StringAlign {
                target,
                value,
                left,
                line,
            } => {
                let Some(slot) = self.find_slot(target) else {
                    return Err(RuntimeError::new(format!("未定义的变量 `{target}`"), *line));
                };
                // VB6 实测：结果长度取变量**当前**长度——不足补空格、超出截掉右侧，
                // 当前是空串就还是空串（不会因为要放 `"ab"` 而变长）。
                let current = slot.value();
                let mut chars: Vec<char> = match (&current, slot.ty()) {
                    (Value::Str(text), _) => text.chars().collect(),
                    (Value::Empty, VbType::Str | VbType::Variant) => Vec::new(),
                    _ => {
                        return Err(RuntimeError::new(
                            format!(
                                "`LSet` / `RSet` 只支持字符串变量（VB6 里对 `{target}` 这种声明是编译错误）"
                            ),
                            *line,
                        ));
                    }
                };
                let width = chars.len();
                let text = self.eval(value, *line)?.to_display();
                let source: Vec<char> = text.chars().collect();
                let keep = source.len().min(width);
                if *left {
                    chars[..keep].copy_from_slice(&source[..keep]);
                    for ch in chars.iter_mut().skip(keep) {
                        *ch = ' ';
                    }
                } else {
                    let start = width - keep;
                    chars[start..].copy_from_slice(&source[..keep]);
                    for ch in chars.iter_mut().take(start) {
                        *ch = ' ';
                    }
                }
                slot.set(Value::Str(chars.into_iter().collect()))
                    .map_err(|message| RuntimeError::new(message, *line))?;
                Ok(Flow::Normal)
            }
            Stmt::PrintFile {
                number,
                values,
                seps,
                suppress_newline,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                // `Print #n, ...;` 之后同一行还会接着写，`,` 分区要从当前列算起。
                let start_column = match self.files.get(&number) {
                    Some(FileHandle::Writer(writer)) => writer.column,
                    _ => 0,
                };
                let text = self.render_print(values, seps, start_column, *line)?;
                let Some(FileHandle::Writer(writer)) = self.files.get_mut(&number) else {
                    return Err(writer_error(&self.files, number, *line));
                };
                writer
                    .write_line(&text, *suppress_newline)
                    .map_err(|error| {
                        RuntimeError::new(format!("写入 #{number} 失败：{error}"), *line)
                            .with_number(io_error_number(&error))
                    })?;
                Ok(Flow::Normal)
            }
            Stmt::WidthFile {
                number,
                width,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let width = self.eval(width, *line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("`Width` 的宽度必须是整数：{message}"), *line)
                })?;
                let Some(FileHandle::Writer(writer)) = self.files.get_mut(&number) else {
                    return Err(writer_error(&self.files, number, *line));
                };
                writer.width = width.max(0) as usize;
                Ok(Flow::Normal)
            }
            Stmt::WriteFile {
                number,
                values,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let mut fields = Vec::with_capacity(values.len());
                for expr in values {
                    let value = self.eval(expr, *line)?;
                    fields.push(
                        write_field(&value).map_err(|message| RuntimeError::new(message, *line))?,
                    );
                }
                let text = format!("{}\r\n", fields.join(","));
                let Some(FileHandle::Writer(writer)) = self.files.get_mut(&number) else {
                    return Err(writer_error(&self.files, number, *line));
                };
                writer.write_raw(&text).map_err(|error| {
                    RuntimeError::new(format!("写入 #{number} 失败：{error}"), *line)
                        .with_number(io_error_number(&error))
                })?;
                Ok(Flow::Normal)
            }
            Stmt::MidAssign {
                target,
                start,
                length,
                value,
                line,
            } => {
                let Some(slot) = self.find_slot(target) else {
                    return Err(RuntimeError::new(format!("未定义的变量 `{target}`"), *line));
                };
                let start = self.eval(start, *line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("`Mid` 的起点必须是整数：{message}"), *line)
                })?;
                let length = match length {
                    Some(expr) => Some(self.eval(expr, *line)?.as_int().map_err(|message| {
                        RuntimeError::new(format!("`Mid` 的长度必须是整数：{message}"), *line)
                    })?),
                    None => None,
                };
                if start < 1 {
                    return Err(RuntimeError::new(
                        format!("`Mid` 的起点 {start} 无效（从 1 开始）"),
                        *line,
                    )
                    .with_number(5));
                }
                let text = self.eval(value, *line)?.to_display();
                let mut chars: Vec<char> = slot.value().to_display().chars().collect();
                let begin = (start - 1) as usize;
                // VB 的 `Mid` 语句只替换已有字符，不会把字符串变长；起点越界时什么都不做。
                if begin >= chars.len() {
                    return Ok(Flow::Normal);
                }
                let available = chars.len() - begin;
                let count = match length {
                    Some(count) => (count.max(0) as usize).min(available),
                    None => available,
                };
                for (offset, ch) in text.chars().take(count).enumerate() {
                    chars[begin + offset] = ch;
                }
                slot.set(Value::Str(chars.into_iter().collect()))
                    .map_err(|message| RuntimeError::new(message, *line))?;
                Ok(Flow::Normal)
            }
            Stmt::RaiseEvent { name, args, line } => {
                let Some(instance) = self.current_instance() else {
                    return Err(RuntimeError::new("`RaiseEvent` 只能用在类的方法里", *line));
                };
                // 实参在「发事件的这一侧」求值，再按值传给订阅者的处理器。
                let mut values = Vec::with_capacity(args.len());
                for expr in args {
                    values.push(self.eval(expr, *line)?);
                }
                let class_name = instance.borrow().key().to_string();
                let handler_name = format!("{class_name}_{name}");
                let key = Rc::as_ptr(&instance) as usize;
                let subscribers = self.events.remove(&key).unwrap_or_default();
                let mut alive = Vec::with_capacity(subscribers.len());
                for subscriber in subscribers {
                    match subscriber {
                        EventSink::Instance(weak) => {
                            let Some(sink) = weak.upgrade() else {
                                continue;
                            };
                            let sink_class = sink.borrow().key().to_string();
                            let handler = self
                                .classes
                                .get(&sink_class)
                                .and_then(|def| def.method(&handler_name));
                            if let Some(handler) = handler {
                                let locals = Self::instance_locals(&sink);
                                self.call_procedure_values(handler, values.clone(), locals)?;
                            }
                            alive.push(EventSink::Instance(weak));
                        }
                        EventSink::Module { var } => {
                            // VB6 的模块级写法是 `<变量名>_<事件>`；QIDE 也接受类字段那套
                            // `<类名>_<事件>`（见 `docs/脚本语言.md`）。
                            let handler = [format!("{var}_{name}"), handler_name.clone()]
                                .into_iter()
                                .find_map(|candidate| self.procedures.get(&candidate).copied());
                            if let Some(handler) = handler {
                                self.call_procedure_values(
                                    handler,
                                    values.clone(),
                                    FastMap::default(),
                                )?;
                            }
                            alive.push(EventSink::Module { var });
                        }
                    }
                }
                self.events.insert(key, alive);
                Ok(Flow::Normal)
            }
            Stmt::LockFile {
                number,
                unlock,
                start,
                end,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let start = self.eval_position(start, *line)?;
                let end = self.eval_position(end, *line)?;
                let Some(handle) = self.files.get(&number) else {
                    return Err(
                        RuntimeError::new(format!("文件号 #{number} 没有打开"), *line)
                            .with_number(52),
                    );
                };
                let (file, record_len) = match handle {
                    FileHandle::Reader(reader) => (reader.get_ref(), None),
                    FileHandle::Writer(writer) => (writer.file(), None),
                    FileHandle::Binary(binary) => {
                        (&binary.file, binary.random.then_some(binary.record_len))
                    }
                };
                let (offset, length) = Self::lock_span(start, end, record_len, number, *line)?;
                match filelock::lock_range(file, offset, length, *unlock) {
                    Ok(()) => Ok(Flow::Normal),
                    Err(filelock::LockFailure::Busy) => Err(RuntimeError::new(
                        format!("文件号 #{number} 上这段范围已被锁定"),
                        *line,
                    )
                    .with_number(70)),
                    Err(filelock::LockFailure::Io(error)) => Err(RuntimeError::new(
                        format!(
                            "{} #{number} 失败：{error}",
                            if *unlock { "解锁" } else { "锁定" }
                        ),
                        *line,
                    )
                    .with_number(io_error_number(&error))),
                }
            }
            Stmt::LineInput {
                number,
                target,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let text = {
                    let Some(FileHandle::Reader(reader)) = self.files.get_mut(&number) else {
                        return Err(RuntimeError::new(
                            format!("文件号 #{number} 不是以 Input 打开"),
                            *line,
                        ));
                    };
                    let mut buffer = String::new();
                    reader.read_line(&mut buffer).map_err(|error| {
                        RuntimeError::new(format!("读取 #{number} 失败：{error}"), *line)
                    })?;
                    buffer
                };
                let text = text.trim_end_matches(['\r', '\n']).to_string();
                self.set_var(target, Value::Str(text), *line, None)?;
                Ok(Flow::Normal)
            }
            Stmt::InputFile {
                number,
                targets,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let text = {
                    let Some(FileHandle::Reader(reader)) = self.files.get_mut(&number) else {
                        return Err(RuntimeError::new(
                            format!("文件号 #{number} 不是以 Input 打开"),
                            *line,
                        ));
                    };
                    let mut buffer = String::new();
                    reader.read_line(&mut buffer).map_err(|error| {
                        RuntimeError::new(format!("读取 #{number} 失败：{error}"), *line)
                    })?;
                    buffer
                };
                let fields = split_input_fields(&text);
                for (index, target) in targets.iter().enumerate() {
                    let value = fields
                        .get(index)
                        .map(|field| parse_input_value(field))
                        .unwrap_or(Value::Empty);
                    self.set_var(target, value, *line, None)?;
                }
                Ok(Flow::Normal)
            }
            Stmt::GetFile {
                number,
                position,
                target,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let position = self.eval_position(position, *line)?;
                let slot = self.lvalue_slot(target, false, *line)?.ok_or_else(|| {
                    RuntimeError::new("`Get` 的目标必须是变量、数组元素或记录字段", *line)
                })?;
                let current = slot.value();
                if let Some(record) = current.record_rc() {
                    // UDT：按结构体布局整块读写。
                    let type_name = Self::record_type_name(&record);
                    let size = ffi::record_size(&type_name, &self.types)
                        .map_err(|message| RuntimeError::new(format!("`Get` {message}"), *line))?;
                    let bytes = self.binary_read(number, position, size, *line)?;
                    ffi::decode_record(&type_name, &bytes, &record, &self.types).map_err(
                        |message| {
                            RuntimeError::new(format!("读取 #{number} 失败：{message}"), *line)
                        },
                    )?;
                    return Ok(Flow::Normal);
                }
                // 变长 `String`：VB6 实测没有长度前缀，`Get` 按变量当前长度（字节）读，
                // 当前是空串就一个字节也不读（文件位置不动）。
                if slot.ty() == VbType::Str {
                    let len = match current {
                        Value::Str(text) => text.len(),
                        _ => 0,
                    };
                    if len == 0 {
                        return Ok(Flow::Normal);
                    }
                    let bytes = self.binary_read(number, position, len, *line)?;
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    slot.set(Value::Str(text))
                        .map_err(|message| RuntimeError::new(message, *line))?;
                    return Ok(Flow::Normal);
                }
                // `Variant` 变量：VB6 实测读的是「2 字节 VARTYPE + 内容」的带标记布局，
                // 读回来之后 `VarType` 也变成标记里的类型（`Get` 一个 Integer 之后是 2）。
                if slot.ty() == VbType::Variant {
                    let mut tag = [0u8; 2];
                    let header = self.binary_read(number, position, 2, *line)?;
                    tag.copy_from_slice(&header[..2]);
                    let code = i16::from_le_bytes(tag);
                    let (value, narrow) = self.read_variant(code, number, *line)?;
                    slot.set(value)
                        .map_err(|message| RuntimeError::new(message, *line))?;
                    slot.set_narrow(narrow);
                    return Ok(Flow::Normal);
                }
                let size = ffi::binary_width(slot.ty())
                    .map_err(|message| RuntimeError::new(format!("`Get` {message}"), *line))?;
                let bytes = self.binary_read(number, position, size, *line)?;
                let value = ffi::decode_scalar(slot.ty(), &bytes).map_err(|message| {
                    RuntimeError::new(format!("读取 #{number} 失败：{message}"), *line)
                })?;
                slot.set(value)
                    .map_err(|message| RuntimeError::new(message, *line))?;
                Ok(Flow::Normal)
            }
            Stmt::PutFile {
                number,
                position,
                value,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let position = self.eval_position(position, *line)?;
                let bytes = self.encode_put(value, *line)?;
                self.binary_write(number, position, &bytes, *line)?;
                Ok(Flow::Normal)
            }
            Stmt::SeekFile {
                number,
                position,
                line,
            } => {
                let number = self.file_number(number, *line)?;
                let position = self.eval(position, *line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("文件位置必须是整数：{message}"), *line)
                })?;
                self.binary_file_mut(number, *line)?
                    .move_to(position, *line)?;
                Ok(Flow::Normal)
            }
            Stmt::ErrRaise {
                number,
                source,
                description,
                line,
            } => {
                let number = self.eval(number, *line)?.as_int().map_err(|message| {
                    RuntimeError::new(format!("错误号必须是整数：{message}"), *line)
                })?;
                let source = match source {
                    Some(expr) => Some(std::sync::Arc::from(
                        self.eval(expr, *line)?.to_display().as_str(),
                    )),
                    None => None,
                };
                let description = match description {
                    Some(expr) => self.eval(expr, *line)?.to_display(),
                    None => builtins::error_text(number),
                };
                Err(RuntimeError::new(description, *line)
                    .with_number(number)
                    .with_source(source))
            }
        }
    }

    pub(super) fn exec_for(
        &mut self,
        var: &str,
        start: &Expr,
        end: &Expr,
        step: Option<&Expr>,
        body: &[Stmt],
        line: usize,
    ) -> Result<Flow, RuntimeError> {
        let start_value = self.eval(start, line)?;
        let end_value = self.eval(end, line)?;
        let step_expression = match step {
            Some(expr) => Some(self.eval(expr, line)?),
            None => None,
        };
        let step_value = match &step_expression {
            Some(value) => value
                .as_float()
                .map_err(|message| RuntimeError::new(message, line))?,
            None => 1.0,
        };
        if step_value == 0.0 {
            return Err(RuntimeError::new("`For` 的 `Step` 不能为 0", line));
        }

        // 起止与步长都是整数时走整数循环：没有累加误差，边界判断与 VB 完全一致。
        if let (Some(mut current), Some(limit), Some(step)) = (
            exact_int(&start_value),
            exact_int(&end_value),
            step_expression.as_ref().map_or(Some(1), exact_int),
        ) {
            // 计数器单元只在进入循环时解析一次（VB 的 `For` 计数也是按帧偏移直取）。
            let mut counter_slot = self.find_slot(var);
            let counter_type = counter_slot
                .as_ref()
                .map_or(VbType::Variant, |slot| slot.ty());
            loop {
                self.tick(line)?;
                if self.count_stats {
                    self.stats.statements += 1;
                }
                let in_range = if step > 0 {
                    current <= limit
                } else {
                    current >= limit
                };
                if !in_range {
                    break;
                }
                match &counter_slot {
                    Some(slot) => self.store_in(slot, var, Value::Int(current), line, None)?,
                    None => {
                        self.set_var(var, Value::Int(current), line, None)?;
                        counter_slot = self.find_slot(var);
                    }
                }
                match self.exec_block(body)? {
                    Flow::Normal | Flow::ExitDo => {}
                    Flow::ExitFor => return Ok(Flow::Normal),
                    other => return Ok(other),
                }
                // 计数器是 `Variant` / `LongLong` 时，i64 溢出就结束循环，避免回绕成死循环；
                // 声明成 `Integer` / `Long` / `Byte` 时按该类型的范围检查并报错误号 6（见下）。
                let Some(next) = current.checked_add(step) else {
                    break;
                };
                // 声明成整数类型时按该类型的范围检查：VB6 在 `Next` 里溢出报错误号 6。
                if let Some((min, max)) = integer_bounds(counter_type)
                    && (next < min || next > max)
                {
                    return Err(RuntimeError::new("溢出", line).with_number(6));
                }
                current = next;
            }
            // VB 的 `For` 走完时计数器停在「第一个不满足条件的值」（`For i = 1 To 4` 之后 `i` 是 5）。
            if let Some(slot) = &counter_slot {
                self.store_in(slot, var, Value::Int(current), line, None)?;
            }
            return Ok(Flow::Normal);
        }

        let mut current = start_value
            .as_float()
            .map_err(|message| RuntimeError::new(message, line))?;
        let limit = end_value
            .as_float()
            .map_err(|message| RuntimeError::new(message, line))?;
        // 累积误差与步长同阶：按步长的相对量给容差，而不是固定 1e-9。
        let tolerance = step_value.abs() * 1e-9;
        let mut counter_slot = self.find_slot(var);

        loop {
            self.tick(line)?;
            if self.count_stats {
                self.stats.statements += 1;
            }
            let in_range = if step_value > 0.0 {
                current <= limit + tolerance
            } else {
                current >= limit - tolerance
            };
            if !in_range {
                break;
            }
            match &counter_slot {
                Some(slot) => self.store_in(slot, var, number_value(current), line, None)?,
                None => {
                    self.set_var(var, number_value(current), line, None)?;
                    counter_slot = self.find_slot(var);
                }
            }
            match self.exec_block(body)? {
                Flow::Normal | Flow::ExitDo => {}
                Flow::ExitFor => break,
                other => return Ok(other),
            }
            current += step_value;
        }
        // 与整数循环一致：正常结束后计数器停在越界后的那个值。
        if let Some(slot) = &counter_slot {
            self.store_in(slot, var, number_value(current), line, None)?;
        }
        Ok(Flow::Normal)
    }

    pub(super) fn exec_for_each(
        &mut self,
        var: &str,
        collection: &Expr,
        body: &[Stmt],
        line: usize,
    ) -> Result<Flow, RuntimeError> {
        let value = self.eval(collection, line)?;
        if let Some(items) = value.collection_rc() {
            let snapshot = items.borrow().values_with_types();
            for (item, narrow) in snapshot {
                self.tick(line)?;
                self.set_iteration_var(var, item, narrow, line)?;
                match self.exec_block(body)? {
                    Flow::Normal | Flow::ExitDo => {}
                    Flow::ExitFor => break,
                    other => return Ok(other),
                }
            }
            return Ok(Flow::Normal);
        }
        if let Some(entries) = value.dictionary_rc() {
            // `For Each` 遍历 Scripting.Dictionary 时取「键」，按插入顺序（本机 VB6 实测）；
            // 要遍历值请写 `For Each v In d.Items`。
            let snapshot = entries.borrow().keys();
            for item in snapshot {
                self.tick(line)?;
                self.set_var(var, item, line, None)?;
                match self.exec_block(body)? {
                    Flow::Normal | Flow::ExitDo => {}
                    Flow::ExitFor => break,
                    other => return Ok(other),
                }
            }
            return Ok(Flow::Normal);
        }
        if let Value::ComObject(object) = &value {
            // COM 集合：走 `_NewEnum`（`IEnumVARIANT`）拿一个**活枚举器**，每轮 `Next` 取一个，
            // 与 VB6 一致——循环体里增删集合元素的效果由服务器自己的枚举器决定，
            // 不是「进循环前先取一份快照」。
            let mut enumerator = match object.items() {
                Ok(enumerator) => enumerator,
                // 本机 VB6 实测（`WScript.Shell` 这种没有 `_NewEnum` 的对象，`On Error Resume Next` 下）：
                // 循环体照样跑**一次**，此时循环变量保持 `Empty`、`Err.Number` 已经是 438，
                // 然后 `Next` 报 92（「For 循环未初始化」）——循环体里写了 `Exit For` 也一样。
                Err(failure)
                    if failure.number == 438
                        && matches!(self.error_handler(), Some(ErrorMode::ResumeNext)) =>
                {
                    self.last_error =
                        Some(RuntimeError::new(failure.message, line).with_number(failure.number));
                    self.tick(line)?;
                    self.set_iteration_var(var, Value::Empty, None, line)?;
                    match self.exec_block(body)? {
                        Flow::Normal | Flow::ExitDo | Flow::ExitFor => {}
                        other => return Ok(other),
                    }
                    return Err(RuntimeError::new(builtins::error_text(92), line).with_number(92));
                }
                Err(failure) => {
                    return Err(
                        RuntimeError::new(failure.message, line).with_number(failure.number)
                    );
                }
            };
            loop {
                let Some(item) = enumerator.next().map_err(|failure| {
                    RuntimeError::new(failure.message, line).with_number(failure.number)
                })?
                else {
                    break;
                };
                self.tick(line)?;
                self.set_iteration_var(var, item, None, line)?;
                match self.exec_block(body)? {
                    Flow::Normal | Flow::ExitDo => {}
                    Flow::ExitFor => break,
                    other => return Ok(other),
                }
            }
            return Ok(Flow::Normal);
        }
        let array = value.as_array().map_err(|message| {
            RuntimeError::new(
                format!("`For Each` 需要数组、Collection、Dictionary 或 COM 集合：{message}"),
                line,
            )
        })?;
        // 先取快照，避免循环体里改写数组时长时间持有借用。
        let (items, element) = {
            let array = array.borrow();
            if array.dims().is_empty() {
                return Err(RuntimeError::new("`For Each` 的数组尚未初始化", line));
            }
            if array.dims().len() > 1 {
                return Err(RuntimeError::new("`For Each` 只支持一维数组", line));
            }
            // VB6 实测：循环变量取**元素声明类型**（Integer 数组 → 2、Byte 数组 → 17、String → 8）；
            // `Variant` 数组取每个元素记下的内容类型（`Array(CByte(1), 2)` 的第一个是 17）。
            let element = array.element_type();
            let element = (!matches!(element, VbType::Variant | VbType::Any)).then_some(element);
            (array.values_with_types(), element)
        };

        // 循环变量的槽只查一次；`Variant` 变量每轮把元素类型记进去（见 `Slot::narrow_type`）。
        if self.find_slot(var).is_none() {
            self.set_var(var, Value::Empty, line, None)?;
        }
        let loop_slot = self.find_slot(var);
        for (item, narrowed) in items {
            self.tick(line)?;
            match &loop_slot {
                Some(slot) => {
                    self.store_in(slot, var, item, line, None)?;
                    if slot.ty() == VbType::Variant {
                        slot.set_narrow(element.or(narrowed));
                    }
                }
                None => self.set_var(var, item, line, None)?,
            }
            match self.exec_block(body)? {
                Flow::Normal | Flow::ExitDo => {}
                Flow::ExitFor => break,
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    /// `For Each` 的循环变量：把元素（连同内容类型）写进去。
    pub(super) fn set_iteration_var(
        &mut self,
        var: &str,
        item: Value,
        narrow: Option<VbType>,
        line: usize,
    ) -> Result<(), RuntimeError> {
        self.set_var(var, item, line, None)?;
        if let Some(slot) = self.find_slot(var)
            && slot.ty() == VbType::Variant
        {
            slot.set_narrow(narrow);
        }
        Ok(())
    }

    /// `WithEvents x As Object` 的变量 `Set` 到 COM 对象时挂接它的事件。
    ///
    /// VB6 的处理过程名是 `变量_事件`，所以这里只登记脚本里真的写了处理过程的事件名。
    pub(super) fn connect_com_events(
        &mut self,
        object: &com::ComObject,
        variable: &str,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let prefix = format!("{variable}_");
        let mut handlers = Vec::new();
        for name in self.procedures.keys() {
            if let Some(event) = name.strip_prefix(&prefix)
                && !event.is_empty()
            {
                handlers.push(event.to_string());
            }
        }
        if handlers.is_empty() {
            // 没写任何处理过程：不需要挂接，也不算错误。
            return Ok(());
        }
        match object.connect_events(variable, &handlers) {
            Ok(events) => {
                self.com_events.push(ComEventOwner {
                    instance: None,
                    field: variable.to_string(),
                    events,
                });
                Ok(())
            }
            Err(failure) => {
                Err(RuntimeError::new(failure.message, line).with_number(failure.number))
            }
        }
    }

    /// `Set 名字 = 对象`：按「模块级 `WithEvents` 变量」或「当前类的 `WithEvents` 字段」挂接事件。
    ///
    /// 脚本类对象走 `events` 表（处理过程在模块或同类里），COM 对象走 `com_events` 队列。
    pub(super) fn connect_events_for_name(
        &mut self,
        name: &str,
        value: &Value,
        line: usize,
    ) -> Result<(), RuntimeError> {
        if let Value::Class(source) = value {
            if self.current_instance().is_none() && self.with_events_vars.contains(name) {
                Self::register_module_event_sink(&mut self.events, &source.0, name);
            }
            return Ok(());
        }
        let Value::ComObject(object) = value else {
            return Ok(());
        };
        // 类字段：方法体的局部变量表里就是实例字段，所以要查当前类的声明。
        if let Some(instance) = self.current_instance() {
            let class_name = instance.borrow().key().to_string();
            if let Some(def) = self.classes.get(&class_name).copied() {
                let with_events = def
                    .fields
                    .iter()
                    .any(|field| field.name == name && field.with_events);
                if with_events {
                    return self.connect_field_events(object, def, name, &instance, line);
                }
            }
        }
        if self.with_events_vars.contains(name) {
            return self.connect_com_events(object, name, line);
        }
        Ok(())
    }

    /// 类字段上的 `WithEvents`：处理过程是同类的 `字段_事件`。
    pub(super) fn connect_field_events(
        &mut self,
        object: &com::ComObject,
        def: &'p ClassDef,
        name: &str,
        instance: &Rc<RefCell<VbInstance>>,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let prefix = format!("{name}_");
        let handlers: Vec<String> = def
            .methods
            .iter()
            .filter_map(|method| method.name.strip_prefix(prefix.as_str()))
            .filter(|event| !event.is_empty())
            .map(str::to_string)
            .collect();
        if handlers.is_empty() {
            return Ok(());
        }
        match object.connect_events(name, &handlers) {
            Ok(events) => {
                self.com_events.push(ComEventOwner {
                    instance: Some(Rc::downgrade(instance)),
                    field: name.to_string(),
                    events,
                });
                Ok(())
            }
            Err(failure) => {
                Err(RuntimeError::new(failure.message, line).with_number(failure.number))
            }
        }
    }

    /// 分派已排队的 COM 事件。
    ///
    /// 两个调用点：**语句边界**（COM 回调里不能重入解释器，只能先排队）与 **`DoEvents`**
    /// （VB6 在消息循环里同步执行事件，所以 `DoEvents` 返回时事件已经处理完了）。
    /// 处理过程里再进来的调用先跳过，新排队的留到下一次边界，避免递归分派同一条链。
    pub(super) fn pump_com_events(&mut self) -> Result<(), RuntimeError> {
        if self.com_events.is_empty() || self.pumping_events {
            return Ok(());
        }
        self.pumping_events = true;
        let outcome = self.dispatch_queued_com_events();
        self.pumping_events = false;
        outcome
    }

    /// 取走队列里的事件并逐个调用处理过程（`pump_com_events` 的实体）。
    pub(super) fn dispatch_queued_com_events(&mut self) -> Result<(), RuntimeError> {
        if self.com_events.is_empty() {
            return Ok(());
        }
        let mut pending = Vec::new();
        for owner in &self.com_events {
            for event in owner.events.drain() {
                pending.push((owner.instance.clone(), owner.field.clone(), event));
            }
        }
        for (instance, field, event) in pending {
            let handler_name = format!("{field}_{}", event.event);
            let mut values = Vec::with_capacity(event.args.len());
            for arg in event.args {
                values.push(match arg {
                    com::EventArg::Empty => Value::Empty,
                    com::EventArg::Null => Value::Null,
                    com::EventArg::Int(number) => Value::Int(number),
                    com::EventArg::Float(number) => Value::Float(number),
                    com::EventArg::Bool(flag) => Value::Bool(flag),
                    com::EventArg::Str(text) => Value::Str(text),
                    com::EventArg::Object(pointer) => {
                        match com::ComObject::from_raw_owned(pointer) {
                            Some(object) => Value::ComObject(object),
                            None => Value::Empty,
                        }
                    }
                });
            }
            match instance {
                // 类字段上的 `WithEvents`：在类里找 `字段_事件` 方法，用实例的局部变量调用。
                Some(weak_owner) => {
                    let Some(owner) = weak_owner.upgrade() else {
                        continue;
                    };
                    let class_name = owner.borrow().key().to_string();
                    let Some(handler) = self
                        .classes
                        .get(&class_name)
                        .and_then(|def| def.method(&handler_name))
                    else {
                        continue;
                    };
                    let locals = Self::instance_locals(&owner);
                    actions_pad_values(&mut values, handler.params.len());
                    self.call_procedure_values(handler, values, locals)?;
                }
                None => {
                    let Some(procedure) = self.procedures.get(&handler_name).copied() else {
                        continue;
                    };
                    actions_pad_values(&mut values, procedure.params.len());
                    self.call_procedure_values(procedure, values, FastMap::default())?;
                }
            }
        }
        Ok(())
    }
}
