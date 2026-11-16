//! 过程/函数调用：实参绑定、`ByRef` 写回与返回值。

use super::*;

impl<'p, 'o> Runtime<'p, 'o> {
    // ---------- 调用 ----------

    pub(super) fn call(
        &mut self,
        name: &str,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        self.call_with_by_value_args(name, args, &[], line)
    }

    /// `CallByName(对象, "成员", 调用方式[, 实参…])`：按名字做后期绑定。
    ///
    /// 语义逐条来自本机 VB6 实测：名字不区分大小写；`VbGet` 取值、`VbLet` 赋值、
    /// `VbSet` 给对象、`VbMethod` 调方法；实参按原表达式传入，所以 `ByRef` 形参照样能改回调用方
    /// （`CallByName(o, "RefArg", VbMethod, x)` 之后 `x` 也会变）。
    pub(super) fn call_by_name(
        &mut self,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let Some((object_expr, rest)) = args.split_first() else {
            return Err(
                RuntimeError::new("`CallByName` 需要对象、成员名和调用方式", line).with_number(449),
            );
        };
        if rest.len() < 2 {
            return Err(
                RuntimeError::new("`CallByName` 需要对象、成员名和调用方式", line).with_number(449),
            );
        }
        let object = self.eval(object_expr, line)?;
        let name = self.eval(&rest[0], line)?.to_display().to_lowercase();
        let call_type = self
            .eval(&rest[1], line)?
            .coerce(VbType::Long)
            .map_err(|message| RuntimeError::from_coerce(message, line))?;
        let call_args = &rest[2..];
        let Value::Int(call_type) = call_type else {
            return Err(RuntimeError::new("错误的调用方式", line).with_number(438));
        };
        match &object {
            Value::Class(instance) => {
                self.call_by_name_on_instance(&instance.0, &name, call_type, call_args, line)
            }
            Value::ComObject(object) => match call_type {
                1 => self.call_com_member(object, &name, call_args, line),
                2 if call_args.is_empty() => self.call_com_member(object, &name, call_args, line),
                4 | 8 => self.com_put_member(object, &name, call_args, line),
                _ => self.call_com_member(object, &name, call_args, line),
            },
            // 声明成对象的变量（`Nothing`）报 5，空 `Variant` 报 424（本机 VB6 实测）。
            Value::Nothing => Err(RuntimeError::new(builtins::error_text(5), line).with_number(5)),
            _ => Err(RuntimeError::new(builtins::error_text(424), line).with_number(424)),
        }
    }

    /// `CallByName` 落在 QIDE 自己的类实例上（`VbGet` / `VbLet` / `VbSet` / `VbMethod`）。
    pub(super) fn call_by_name_on_instance(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        name: &str,
        call_type: i64,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let def = {
            let borrowed = instance.borrow();
            self.classes.get(borrowed.key()).copied()
        };
        let Some(def) = def else {
            let display = instance.borrow().type_name().to_string();
            return Err(RuntimeError::new(format!("未定义的类 `{display}`"), line));
        };
        match call_type {
            // `VbMethod`
            1 => self.call_class_method(instance, name, args, line),
            // `VbGet`
            2 => {
                let method = def.method(name).or_else(|| def.implemented_method(name));
                if let Some(method) = method
                    && !method.is_get
                {
                    // `Function`/`Sub` 不能用 `VbGet` 取（VB6 实测 `Sub` 是 450，带参数的 `Function` 也是 450）。
                    return Err(RuntimeError::new(builtins::error_text(450), line).with_number(450));
                }
                if method.is_some() && !args.is_empty() {
                    return self.call_class_method(instance, name, args, line);
                }
                self.read_instance_member(instance, name, line)
            }
            // `VbLet` / `VbSet`
            4 | 8 => {
                let set_op = call_type == 8;
                let Some(value) = args.first() else {
                    return Err(RuntimeError::new(builtins::error_text(446), line).with_number(446));
                };
                if args.len() > 1 {
                    return Err(RuntimeError::new(builtins::error_text(451), line).with_number(451));
                }
                self.assign_by_name(instance, def, name, value, set_op, line)
            }
            _ => Err(RuntimeError::new(builtins::error_text(438), line).with_number(438)),
        }
    }

    /// `CallByName` 的 `VbLet` / `VbSet` 分支：有 `Property` 就走它，否则写实例变量。
    pub(super) fn assign_by_name(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        def: &'p ClassDef,
        name: &str,
        value: &Expr,
        set_op: bool,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let setter = if set_op {
            def.property_set(name)
                .or_else(|| def.implemented_property_set(name))
        } else {
            def.property_let(name)
                .or_else(|| def.implemented_property_let(name))
        };
        if let Some(setter) = setter {
            let direct = if set_op {
                def.property_set(name).is_some()
            } else {
                def.property_let(name).is_some()
            };
            if direct {
                self.ensure_class_member_access(instance, name, setter.is_private, line)?;
            }
            let locals = Self::instance_locals(instance);
            self.call_user_with_locals(setter, std::slice::from_ref(value), locals, line)?;
            return Ok(Value::Empty);
        }
        if let Some(field) = instance.borrow().field(name) {
            let is_private = def.field_is_private(name);
            self.ensure_class_member_access(instance, name, is_private, line)?;
            // VB6 实测：对象类型的字段用 `VbLet` 赋值报 91，用 `VbSet` 给普通字段报 451。
            if !set_op && field.ty == VbType::Object {
                return Err(RuntimeError::new(builtins::error_text(91), line).with_number(91));
            }
            if set_op && !matches!(field.ty, VbType::Object | VbType::Variant) {
                return Err(RuntimeError::new(builtins::error_text(451), line).with_number(451));
            }
            let value = self
                .eval(value, line)?
                .coerce(field.ty)
                .map_err(|message| RuntimeError::from_coerce(message, line))?;
            let with_events = def
                .fields
                .iter()
                .find(|item| item.name == *name)
                .is_some_and(|item| item.with_events);
            if with_events && let Value::Class(source) = &value {
                Self::register_event_sink(&mut self.events, &source.0, instance);
            }
            *field.cell.borrow_mut() = value;
            return Ok(Value::Empty);
        }
        // 只剩只读成员（`Property Get` / `Function` / `Sub`）：VB6 实测 `VbSet` 给只有 `Get` 的属性是 450。
        if def.method(name).is_some() || def.implemented_method(name).is_some() {
            return Err(RuntimeError::new(builtins::error_text(450), line).with_number(450));
        }
        Err(RuntimeError::new(builtins::error_text(438), line).with_number(438))
    }

    /// COM 对象的 `VbLet` / `VbSet`：`IDispatch::Invoke` 的属性写入。
    pub(super) fn com_put_member(
        &mut self,
        object: &com::ComObject,
        name: &str,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let Some(value) = args.first() else {
            return Err(RuntimeError::new(builtins::error_text(446), line).with_number(446));
        };
        let value = self.eval(value, line)?;
        object.set(name, &value).map_err(|failure| {
            RuntimeError::new(failure.message, line).with_number(failure.number)
        })?;
        Ok(Value::Empty)
    }

    /// `VarPtr(变量)` / `ObjPtr(对象)`：返回一个 `Long` 形式的「地址」。
    ///
    /// QIDE 的值是解释器内部结构，没有稳定的原生地址；这里给的是**进程内唯一且稳定**的标识：
    /// `VarPtr` 取变量共享单元的地址（同一变量不变、不同变量不同），
    /// `ObjPtr` 取对象实例（`Rc`）的地址（同一对象相等、不同对象不等）。
    /// 本机 VB6 实测：两者 `VarType` 都是 3（`Long`），`ObjPtr(Nothing)` 是 0。
    pub(super) fn pointer_of(
        &mut self,
        name: &str,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let Some(expr) = args.first() else {
            return Err(
                RuntimeError::new(format!("`{name}` 缺少第 1 个参数"), line).with_number(449)
            );
        };
        if name == "varptr" {
            let Some(slot) = self.lvalue_slot(expr, false, line)? else {
                return Err(
                    RuntimeError::new("`VarPtr` 需要变量、数组元素或记录字段", line)
                        .with_number(449),
                );
            };
            return Ok(Value::Int(std::rc::Rc::as_ptr(&slot.cell) as usize as i64));
        }
        let value = self.eval(expr, line)?;
        let pointer = match &value {
            Value::Empty | Value::Nothing => 0,
            Value::Class(instance) => std::rc::Rc::as_ptr(&instance.0) as usize as i64,
            Value::Collection(items) => std::rc::Rc::as_ptr(items) as usize as i64,
            Value::Dictionary(entries) => std::rc::Rc::as_ptr(entries) as usize as i64,
            Value::Picture(picture) => std::rc::Rc::as_ptr(picture) as usize as i64,
            Value::Record(record) => std::rc::Rc::as_ptr(record) as usize as i64,
            Value::ComObject(object) => object.identity() as i64,
            _ => {
                return Err(RuntimeError::new("`ObjPtr` 需要一个对象", line).with_number(424));
            }
        };
        Ok(Value::Int(pointer))
    }

    pub(super) fn call_com_member(
        &mut self,
        object: &com::ComObject,
        name: &str,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let values = self.com_call_arguments(args, line)?;
        let call_args = Self::com_call_args(&values);
        let outcome = object.call_with_argument_modes(name, &call_args);
        Self::finish_com_call(values, outcome, line)
    }

    /// `obj(实参…)`：走 COM 的**默认成员**（`DISPID_VALUE`），`d("k")` 等同于 `d.Item("k")`。
    pub(super) fn call_com_default(
        &mut self,
        object: &com::ComObject,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let values = self.com_call_arguments(args, line)?;
        let call_args = Self::com_call_args(&values);
        let outcome = object.call_default(&call_args);
        Self::finish_com_call(values, outcome, line)
    }

    /// 用**已经求好值**的实参调用 COM 成员（`rs.Fields("id")` 这种链式里中间层用得上）。
    pub(super) fn call_com_with_values(
        &mut self,
        object: &com::ComObject,
        name: &str,
        values: Vec<Value>,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let arguments: Vec<ComArgument> = values
            .into_iter()
            .map(|value| (None, value, false, None))
            .collect();
        let call_args = Self::com_call_args(&arguments);
        let outcome = object.call_with_argument_modes(name, &call_args);
        Self::finish_com_call(arguments, outcome, line)
    }

    /// 逐个求值 COM 调用的实参，并记下可写回的变量槽（`ByRef` 形参写回用）。
    pub(super) fn com_call_arguments(
        &mut self,
        args: &[Expr],
        line: usize,
    ) -> Result<Vec<ComArgument>, RuntimeError> {
        let mut values = Vec::with_capacity(args.len());
        for expr in args {
            let (argument_name, argument_expr) = match expr {
                Expr::NamedArg { name, value } => (Some(name.clone()), value.as_ref()),
                _ => (None, expr),
            };
            let value = self.eval(argument_expr, line)?;
            let slot = match argument_expr {
                Expr::Var(_) | Expr::Call { .. } => self.lvalue_slot(argument_expr, false, line)?,
                _ => None,
            };
            let by_ref = slot
                .as_ref()
                .is_some_and(|slot| !slot.is_readonly() && !value.is_array());
            values.push((argument_name, value, by_ref, slot));
        }
        Ok(values)
    }

    pub(super) fn com_call_args(values: &[ComArgument]) -> Vec<(Option<String>, Value, bool)> {
        values
            .iter()
            .map(|(name, value, by_ref, _)| (name.clone(), value.clone(), *by_ref))
            .collect()
    }

    pub(super) fn finish_com_call(
        values: Vec<ComArgument>,
        outcome: Result<(Value, Vec<(usize, Value)>), com::ComFailure>,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let (result, updated) = outcome.map_err(|failure| {
            RuntimeError::new(failure.message, line).with_number(failure.number)
        })?;
        for (index, value) in updated {
            if let Some(slot) = values[index].3.as_ref() {
                slot.set(value)
                    .map_err(|message| RuntimeError::from_coerce(message, line))?;
            }
        }
        Ok(result)
    }

    /// 表达式的 VB6 静态类型（只用于 `VarType` / `TypeName` / `Hex` / `Oct` 这类要看类型的地方）。
    ///
    /// 值本身只带动态类型（整数一律是 `i64`），所以这里按语法推断：字面量宽度、变量/参数的声明类型、
    /// 一元与二元运算的类型提升、转换函数与用户过程的返回类型。推断不出来时返回 `None`，
    /// 调用方退回按值判断（大部分内置函数、对象成员、数组元素都属于这一类）。规则逐条来自本机 VB6 实测。
    pub(super) fn static_type(&self, expr: &Expr) -> Option<VbType> {
        match expr {
            Expr::TypedLiteral { ty, .. } => Some(*ty),
            Expr::Literal(value) => match value {
                // 整数字面量：装得进 Integer 就是 Integer，否则 Long（实测 `VarType(32767)`=2、`VarType(32768)`=3）。
                Value::Int(number) => Some(if i16::try_from(*number).is_ok() {
                    VbType::Integer
                } else {
                    VbType::Long
                }),
                Value::Float(_) => Some(VbType::Double),
                Value::Currency(_) => Some(VbType::Currency),
                Value::Date(_) => Some(VbType::Date),
                Value::Bool(_) => Some(VbType::Boolean),
                Value::Str(_) => Some(VbType::Str),
                _ => None,
            },
            Expr::Var(name) => {
                if let Some(slot) = self.lookup_slot(name) {
                    // 数组按 `vbArray + 元素类型码` 报，交给按值判断。
                    if slot.value().is_array() {
                        return None;
                    }
                    if slot.ty() == VbType::Variant {
                        // `Variant` 变量按赋值时记下的内容类型回答（`v = i` 之后是 Integer）。
                        return slot.narrow_type();
                    }
                    return Some(slot.ty());
                }
                // `VarType(vbTrue)` 是 3（Long）、`VarType(vbCrLf)` 是 8。
                builtins::vb_constant_type(name)
            }
            // `VarType(u.f)`：记录里 `As Variant` 的字段按写入时记下的内容类型回答。
            // `VarType(obj.Prop)`：类成员的返回类型来自 `Property Get` / `Function` 的声明，
            // `VarType(obj.Field)`：`Variant` 字段按写入时记下的内容类型回答。
            Expr::Member { base, name } => {
                let root = self.static_base_value(base)?;
                if let Value::Class(instance) = &root {
                    let borrowed = instance.0.borrow();
                    if let Some(def) = self.classes.get(borrowed.key()).copied()
                        && let Some(method) =
                            def.method(name).or_else(|| def.implemented_method(name))
                    {
                        return (method.ret != VbType::Variant).then_some(method.ret);
                    }
                    let (field_ty, narrow) = borrowed.field_narrow(name)?;
                    return if field_ty == VbType::Variant {
                        narrow
                    } else {
                        Some(field_ty)
                    };
                }
                let record = root.as_record().ok()?;
                let borrowed = record.borrow();
                let (ty, narrow) = borrowed.field_types(name)?;
                if ty == VbType::Variant {
                    narrow
                } else {
                    Some(ty)
                }
            }
            // `With u: VarType(.f)`：与 `u.f` 同样处理。
            Expr::WithMember { path } => {
                let root = self.with_stack.last().cloned()?;
                let mut value = root;
                for (index, name) in path.iter().enumerate() {
                    let record = value.as_record().ok()?;
                    let borrowed = record.borrow();
                    if index + 1 == path.len() {
                        let (ty, narrow) = borrowed.field_types(name)?;
                        return if ty == VbType::Variant {
                            narrow
                        } else {
                            Some(ty)
                        };
                    }
                    value = borrowed.get(name)?;
                }
                None
            }
            // `Not` 保持操作数类型（实测 `Not CByte(1)`=17、`Not 1`=2）；
            // `-32768` 实测是 Long，因为 `32768` 本身已是 Long。
            Expr::Unary { expr, .. } => self.static_type(expr),
            Expr::Binary { op, left, right } => {
                Self::binary_static_type(*op, self.static_type(left), self.static_type(right))
            }
            Expr::Call { name, args } => {
                if let Some(ty) = conversion_type(name) {
                    return Some(ty);
                }
                // `VarType(arr(0))`：`Variant` 数组要取元素记下的内容类型
                // （`picked(0) = CByte(9)` 之后是 17），字面量下标才能静态查。
                if let Some(ty) = self.array_element_static_type(name, args) {
                    return Some(ty);
                }
                match name.as_str() {
                    // `VarType(CVar(1))` 实测是 2：`CVar` 不改类型。
                    "cvar" | "cvdate" | "abs" | "int" | "fix" => {
                        args.first().and_then(|arg| self.static_type(arg))
                    }
                    "hex" | "oct" | "str" | "chr" => Some(VbType::Str),
                    "sgn" => Some(VbType::Integer),
                    _ => self.procedures.get(name).map(|procedure| procedure.ret),
                }
            }
            _ => None,
        }
    }

    /// `static_type` 用的「静态基值」：只认变量与字面量下标的数组元素（`u`、`recs(0)`）。
    pub(super) fn static_base_value(&self, expr: &Expr) -> Option<Value> {
        match expr {
            Expr::Var(name) => Some(self.lookup_slot(name)?.value()),
            Expr::Call { name, args } => {
                let array = self.lookup_slot(name)?.value();
                if !array.is_array() {
                    return None;
                }
                let mut indices = Vec::with_capacity(args.len());
                for arg in args {
                    match arg {
                        Expr::Literal(Value::Int(number)) => {
                            indices.push(i32::try_from(*number).ok()?);
                        }
                        _ => return None,
                    }
                }
                let array = array.as_array().ok()?;
                array.borrow().get(&indices).ok()
            }
            _ => None,
        }
    }

    /// `arr(0)` 这类**字面量下标**的元素内容类型（只有 `Variant` 数组记了内容类型）。
    pub(super) fn array_element_static_type(&self, name: &str, args: &[Expr]) -> Option<VbType> {
        let slot = self.find_slot(name)?;
        let value = slot.value();
        if value.is_array() {
            let array = value.as_array().ok()?;
            let array = array.borrow();
            if array.element_type() != VbType::Variant {
                return None;
            }
            let mut indices = Vec::with_capacity(args.len());
            for arg in args {
                match arg {
                    Expr::Literal(Value::Int(number)) => {
                        indices.push(i32::try_from(*number).ok()?);
                    }
                    _ => return None,
                }
            }
            return array.narrow_at(&indices);
        }
        // `VarType(d("k"))`：字典也记了每个键的内容类型，字面量键才静态查。
        if value.is_dictionary()
            && let [Expr::Literal(Value::Str(key))] = args
        {
            let entries = value.as_dictionary().ok()?;
            return entries.borrow().narrow(key);
        }
        None
    }

    /// 二元运算结果类型（比较运算给 Boolean，`/` 与 `^` 给 Double，字符串拼接给 String）。
    pub(super) fn binary_static_type(
        op: BinOp,
        left: Option<VbType>,
        right: Option<VbType>,
    ) -> Option<VbType> {
        let (left, right) = (left?, right?);
        // `Variant`（含装 `Null` 的变量）在运算里会传播 `Null`，类型推断不出来，交给按值判断
        // （例如 `TypeName(Null = Null)` 是 "Null"）。
        if matches!(left, VbType::Variant | VbType::Any | VbType::Object)
            || matches!(right, VbType::Variant | VbType::Any | VbType::Object)
        {
            return None;
        }
        if matches!(
            op,
            BinOp::Eq
                | BinOp::NotEq
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::LtEq
                | BinOp::GtEq
                | BinOp::Like
                | BinOp::Is
        ) {
            return Some(VbType::Boolean);
        }
        // `+` 与 `&` 遇到字符串都拼接（实测 `VarType("a" & "b")` 与 `VarType("a" + "b")` 都是 String）。
        if op == BinOp::Concat
            || (op == BinOp::Add && (left == VbType::Str || right == VbType::Str))
        {
            return Some(VbType::Str);
        }
        match op {
            // 浮点除法与乘方一律 Double（实测 `VarType(1 / 2)`=5、`VarType(2 ^ 3)`=5）。
            BinOp::Divide | BinOp::Power => Some(VbType::Double),
            _ => promote_numeric(left, right),
        }
    }

    pub(super) fn call_with_by_value_args(
        &mut self,
        name: &str,
        args: &[Expr],
        by_value_args: &[bool],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        if self.count_stats {
            self.stats.calls += 1;
        }
        // `IsMissing` 判定的是"实参有没有省略"，只能从参数槽读，所以在分派前单独处理。
        if name == "ismissing" {
            let missing = match args.first() {
                Some(Expr::Var(var)) => self.find_slot(var).is_some_and(|slot| slot.is_missing()),
                _ => false,
            };
            return Ok(Value::Bool(missing));
        }
        // `VarType` / `TypeName` 要看表达式的**静态类型**（VB6 实测：`Dim n As Integer` 是 2 / "Integer"，
        // `Byte` 是 17、`Single` 是 4，`1 + 1` 也是 2）。值本身不带窄类型信息，所以按语法推断。
        // 推断不出来（大部分内置函数、对象成员、数组）时退回按值判断。
        if matches!(name, "vartype" | "typename")
            && let Some(expr) = args.first()
        {
            if let Some(ty) = self.static_type(expr)
                && !matches!(ty, VbType::Variant | VbType::Any | VbType::Object)
            {
                return Ok(if name == "vartype" {
                    Value::Int(builtins::vartype_of(ty))
                } else {
                    Value::Str(ty.name().to_string())
                });
            }
            // 隐式变量（`DefInt A-Z` 之后）没有声明语句，静态推断看不到它，改看槽上的声明类型；
            // 槽还没建出来时（第一次读取就在这里）按 `DefType` 推。
            if let Expr::Var(variable) = expr {
                let ty = self
                    .find_slot(variable)
                    .map(|slot| slot.ty())
                    .unwrap_or_else(|| self.implicit_type(variable));
                if !matches!(ty, VbType::Variant | VbType::Any | VbType::Object) {
                    return Ok(if name == "vartype" {
                        Value::Int(builtins::vartype_of(ty))
                    } else {
                        Value::Str(ty.name().to_string())
                    });
                }
            }
            // 调用/成员表达式的内容类型只有求值后才知道（返回 `Variant` 的函数、`IIf`/`Choose`/`Switch`、
            // `Collection.Item`），求值时会把类型记进 `result_narrow`
            // （本机 VB6 实测：`IIf(True, i, s)` 是 2、`IIf(False, i, s)` 是 4、`VarType(c.Item(1))` 是 2）。
            if matches!(
                expr,
                Expr::Call { .. }
                    | Expr::Member { .. }
                    | Expr::MemberIndex { .. }
                    | Expr::WithMember { .. }
            ) {
                self.result_narrow = None;
                let value = self.eval(expr, line)?;
                if let Some(ty) = self.result_narrow.take()
                    && !matches!(ty, VbType::Variant | VbType::Any | VbType::Object)
                {
                    return Ok(if name == "vartype" {
                        Value::Int(builtins::vartype_of(ty))
                    } else {
                        Value::Str(ty.name().to_string())
                    });
                }
                return Ok(if name == "vartype" {
                    Value::Int(builtins::vartype_code_of(&value))
                } else {
                    Value::Str(value.type_name())
                });
            }
        }
        // `CStr` / `Str` 拿不到表达式的类型，`Single` 要先按 7 位有效数字渲染再交给内置函数
        // （VB6 实测：`CStr(CSng(1.23456789))` 是 `1.234568`、`Str(CSng(0.5))` 是 `" .5"`）。
        if matches!(name, "cstr" | "str")
            && let Some(expr) = args.first()
            && self.static_type(expr) == Some(VbType::Single)
        {
            let text = match self.eval(expr, line)? {
                Value::Float(number) => crate::interpreter::value::format_single(number),
                Value::Null => return Ok(Value::Null),
                other => other.to_display(),
            };
            return Ok(Value::Str(if name == "str" {
                builtins::str_number_text(text, true)
            } else {
                text
            }));
        }
        if matches!(name, "createobject" | "getobject") {
            if let Some(message) = &self.com_error {
                return Err(RuntimeError::new(message.clone(), line).with_number(429));
            }
            let mut values = Vec::with_capacity(args.len());
            for expr in args {
                values.push(self.eval(expr, line)?);
            }
            if name == "createobject" {
                if values.len() != 1 {
                    return Err(
                        RuntimeError::new("`CreateObject` 需要一个 ProgID 参数", line)
                            .with_number(450),
                    );
                }
                let prog_id = values[0].to_display();
                return com::create_object(&prog_id)
                    .map(Value::ComObject)
                    .map_err(|message| RuntimeError::new(message, line).with_number(429));
            }
            let active_prog_id = match values.as_slice() {
                [path] => {
                    let pathname = path.to_display();
                    if pathname.is_empty() {
                        // VB6 实测：`GetObject("")` 报 5（路径为空），不是「缺少 ProgID」。
                        return Err(RuntimeError::new("`GetObject` 的路径不能是空字符串", line)
                            .with_number(5));
                    }
                    // 像路径/moniker 的解析成绝对路径（相对路径按脚本目录）；
                    // 不像路径的裸名字原样交给 moniker 解析，这样失败号与 VB6 一致
                    // （`GetObject("hosts")` 是 moniker 语法错误 -2147221020）。
                    let resolved = if is_moniker_path(&pathname, &self.cwd) {
                        self.resolve_moniker_path(&pathname)
                    } else {
                        pathname.clone()
                    };
                    return com::get_object(&pathname, &resolved, None)
                        .map(Value::ComObject)
                        .map_err(|failure| {
                            RuntimeError::new(failure.message, line).with_number(failure.number)
                        });
                }
                [Value::Empty, class] => Some(class.to_display()),
                [path, class] if path.to_display().is_empty() => {
                    let prog_id = class.to_display();
                    if prog_id.is_empty() {
                        return Err(RuntimeError::new(
                            "`GetObject` 的路径和 ProgID 不能同时为空",
                            line,
                        )
                        .with_number(5));
                    }
                    return com::create_object(&prog_id)
                        .map(Value::ComObject)
                        .map_err(|message| RuntimeError::new(message, line).with_number(429));
                }
                [path, class] => {
                    let pathname = path.to_display();
                    let resolved = self.resolve_moniker_path(&pathname);
                    return com::get_object(&pathname, &resolved, Some(&class.to_display()))
                        .map(Value::ComObject)
                        .map_err(|failure| {
                            RuntimeError::new(failure.message, line).with_number(failure.number)
                        });
                }
                [path, class, ..] => {
                    let _ = (path, class);
                    return Err(RuntimeError::new(
                        "`GetObject` 最多接受路径和 ProgID 两个参数",
                        line,
                    )
                    .with_number(450));
                }
                _ => {
                    return Err(RuntimeError::new(
                        "`GetObject` 需要路径，可写 `GetObject(\"路径\")` 或 `GetObject(, \"ProgID\")`",
                        line,
                    )
                    .with_number(450));
                }
            };
            let Some(prog_id) = active_prog_id else {
                return Err(
                    RuntimeError::new("`GetObject` 缺少 ProgID 参数", line).with_number(450)
                );
            };
            return com::get_active_object(&prog_id)
                .map(Value::ComObject)
                .map_err(|message| RuntimeError::new(message, line).with_number(429));
        }
        if let Some(slot) = self.find_slot(name) {
            let value = slot.value();
            if value.is_array() {
                return self.index_array(name, &value, args, line);
            }
            if let Some(entries) = value.dictionary_rc() {
                // `d("键")`：按键读值。
                let key = match args {
                    [single] => self.eval(single, line)?,
                    _ => {
                        return Err(RuntimeError::new("`Dictionary` 的下标只能是一个键", line));
                    }
                };
                let key = VbDictionary::key_of(&key)
                    .map_err(|message| RuntimeError::new(message, line))?;
                return entries
                    .borrow()
                    .get(&key)
                    .map_err(|message| RuntimeError::new(message, line).with_number(9));
            }
            if let Some(collection) = value.collection_rc() {
                // `c(1)` / `c("键")`：`Collection` 的默认成员就是 `Item`（VB6 实测可用）。
                let key = match args {
                    [single] => self.eval(single, line)?,
                    _ => {
                        return Err(RuntimeError::new(
                            "`Collection` 的下标只能是一个下标或键",
                            line,
                        ));
                    }
                };
                let (item, narrow) = collection
                    .borrow()
                    .item_with_type(&key)
                    .map_err(|message| RuntimeError::new(message, line).with_number(9))?;
                self.result_narrow = narrow;
                return Ok(item);
            }
            if let Value::ComObject(object) = &value {
                // `d("k")`：COM 对象的默认成员（`DISPID_VALUE`），等同 `d.Item("k")`（VB6 实测）。
                return self.call_com_default(object, args, line);
            }
        }
        if matches!(
            name,
            "eof" | "lof" | "freefile" | "seek" | "loc" | "fileattr"
        ) {
            return self.file_builtin(name, args, line);
        }
        if name == "input" {
            return self.input_function(args, line);
        }
        // 类方法里可以裸调用同类的成员（VB 的隐式 `Me.`）。
        if let Some(def) = self.current_class()
            && let Some(method) = def.method(name)
        {
            let locals = self.current_instance_locals();
            return self.call_user_with_locals_and_by_value_args(
                method,
                args,
                locals,
                by_value_args,
                line,
            );
        }
        if let Some(procedure) = self.procedures.get(name).copied() {
            return self.call_user_with_by_value_args(procedure, args, by_value_args, line);
        }
        if let Some(declare) = self.declares.get(name).copied() {
            return self.call_declare(declare, args, by_value_args, line);
        }
        // `CallByName` 要拿到原始实参表达式（`ByRef` 形参得能写回调用方的槽），所以放在内置函数分派之前。
        if name == "callbyname" {
            return self.call_by_name(args, line);
        }
        // `VarPtr` / `ObjPtr` 也要原始表达式：前者取变量的存储单元，后者取对象实例的身份。
        if name == "varptr" || name == "objptr" {
            return self.pointer_of(name, args, line);
        }
        // `DoEvents` 在 VB6 里会把排队的 COM 事件**当场**执行完（事件在消息循环里同步分派），
        // 所以要在这里拦一道，不能走纯函数的内置函数表。
        if name == "doevents" {
            self.pump_com_events()?;
            ffi::sleep_ms(0);
            return Ok(Value::Int(0));
        }
        if builtins::is_builtin(name) {
            // `Hex` / `Oct` 的输出宽度跟着实参的静态类型走（`Hex(-1)` 是 16 位、`Hex(-1&)` 是 32 位）。
            let arg_type = if matches!(name, "hex" | "oct") {
                args.first().and_then(|expr| self.static_type(expr))
            } else {
                None
            };
            // `Array(CByte(1), 2)` 要把每个实参的类型一起记进数组（`For Each` / `VarType` 用）；
            // `IIf`/`Choose`/`Switch` 返回的是其中一个实参，要用它的类型回答 `VarType`。
            let arg_types: Vec<Option<VbType>> = if name == "array" {
                args.iter().map(|expr| self.narrow_of(Some(expr))).collect()
            } else if matches!(name, "iif" | "choose" | "switch") {
                args.iter().map(|expr| self.narrow_of(Some(expr))).collect()
            } else {
                Vec::new()
            };
            let mut values = Vec::with_capacity(args.len());
            for expr in args {
                values.push(self.eval(expr, line)?);
            }
            let types = &self.types;
            let size_of = |name: &str| ffi::record_size(name, types).ok();
            let mut ctx = builtins::Ctx {
                rng: &mut self.rng,
                started: self.started,
                interrupt: self.interrupt,
                err_number: self
                    .last_error
                    .as_ref()
                    .map(|error| error.number)
                    .unwrap_or(0),
                record_size: &size_of,
                dir: &mut self.dir,
                compare_text: self.option_compare_text,
                allow_ui: self.allow_ui,
                failure: None,
                command: &self.command_line,
                cwd: &mut self.cwd,
                strings: &mut self.strings,
                arg_type,
                arg_types: &arg_types,
                result_narrow: None,
            };
            let outcome = builtins::call(name, values, &mut ctx);
            let failure = ctx.failure;
            self.result_narrow = ctx.result_narrow;
            return outcome.map_err(|message| {
                let error = RuntimeError::from_coerce(message, line);
                match failure {
                    Some(number) => error.with_number(number),
                    None => error,
                }
            });
        }
        Err(RuntimeError::new(
            format!("未定义的函数或过程 `{name}`"),
            line,
        ))
    }

    /// `a(0)`、`m(1, 2)`：数组元素读取（写操作在 `Stmt::AssignIndex` 里处理）。
    pub(super) fn index_array(
        &mut self,
        name: &str,
        array: &Value,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        if self.count_stats {
            self.stats.array_accesses += 1;
        }
        let indices = self.eval_indices(args, line)?;
        let array = array
            .as_array()
            .map_err(|message| RuntimeError::new(format!("`{name}` {message}"), line))?;
        let value = array.borrow().get(&indices).map_err(|message| {
            RuntimeError::new(format!("数组 `{name}`：{message}"), line).with_number(9)
        })?;
        Ok(value)
    }

    /// `x = 值` / `x(下标) = 值` 分派到同合 `Property Let`。
    pub(super) fn call_property_let(
        &mut self,
        procedure: &'p Procedure,
        prefix: &[Expr],
        value: &Expr,
        line: usize,
    ) -> Result<Flow, RuntimeError> {
        let mut args: Vec<Expr> = prefix.to_vec();
        args.push(value.clone());
        if args.len() != procedure.params.len() {
            return Err(RuntimeError::new(
                format!(
                    "`Property Let {}` 需要 {} 个参数，实际给了 {}",
                    procedure.name,
                    procedure.params.len(),
                    args.len()
                ),
                line,
            ));
        }
        self.call_user(procedure, &args, line)?;
        Ok(Flow::Normal)
    }

    pub(super) fn call_user(
        &mut self,
        procedure: &'p Procedure,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        self.call_user_with_locals(procedure, args, FastMap::default(), line)
    }

    pub(super) fn call_user_with_by_value_args(
        &mut self,
        procedure: &'p Procedure,
        args: &[Expr],
        by_value_args: &[bool],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        self.call_user_with_locals_and_by_value_args(
            procedure,
            args,
            FastMap::default(),
            by_value_args,
            line,
        )
    }

    /// 执行过程：`locals` 是预先注入的局部变量（类方法用它注入实例变量与 `Me`）。
    pub(super) fn call_user_with_locals(
        &mut self,
        procedure: &'p Procedure,
        args: &[Expr],
        locals: FastMap<std::borrow::Cow<'p, str>, Slot>,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        self.call_user_with_locals_and_by_value_args(procedure, args, locals, &[], line)
    }

    pub(super) fn call_user_with_locals_and_by_value_args(
        &mut self,
        procedure: &'p Procedure,
        args: &[Expr],
        mut locals: FastMap<std::borrow::Cow<'p, str>, Slot>,
        by_value_args: &[bool],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        if self.frames.len() >= self.max_depth {
            return Err(RuntimeError::new(
                format!("调用层次过深（上限 {} 层）", self.max_depth),
                line,
            ));
        }
        // `max_depth` 是逻辑上的层数；真正先到顶的往往是物理栈（debug 构建每层几十 KB）。
        // 与其让进程被栈溢出直接终止，不如在这里报 VB 的错误号 28。
        if stack_headroom() < MIN_CALL_STACK {
            return Err(RuntimeError::new("调用层次过深（栈空间不足）", line).with_number(28));
        }
        if args.len() > procedure.params.len()
            && !procedure.params.iter().any(|param| param.param_array)
        {
            // VB6 实测：实参比形参多是 450（`CallByName(o, "Bump", VbMethod, 1, 2)`）。
            return Err(RuntimeError::new(
                format!(
                    "过程 `{}` 只接受 {} 个参数，实际传入 {} 个",
                    procedure.name,
                    procedure.params.len(),
                    args.len()
                ),
                line,
            )
            .with_number(450));
        }
        // 参数与返回值都要塞进局部表：先按数量预留，省掉逐次扩容的哈希搬迁。
        locals.reserve(procedure.params.len() + 1);
        for (index, param) in procedure.params.iter().enumerate() {
            if param.param_array {
                // `ParamArray xs()`：把剩下的实参打包成 0 基 Variant 数组（连每个元素的内容类型一起记）。
                let mut values = Vec::new();
                for expr in args.iter().skip(index) {
                    let value = self.eval(expr, line)?;
                    let narrow = self.narrow_of(Some(expr));
                    values.push((value, narrow));
                }
                locals.insert(
                    param.name.clone().into(),
                    Slot::new(Value::array_with_types(values), VbType::Variant),
                );
                continue;
            }
            let slot = match args.get(index) {
                // `F 1, , 3`：省略的实参按「调用时省略」绑定（`IsMissing` 为真），
                // 必填形参省略时报 449（VB6 的「参数不可选」）。
                Some(Expr::OmittedArg) => {
                    if param.optional {
                        let slot = self.bind_default(param, line)?;
                        // `IsMissing` 只对「省略的、没有默认值的 `Variant` 可选形参」为真；
                        // 带默认值的可选参数按默认值绑定，`IsMissing` 是 False。
                        if param.ty == VbType::Variant && param.default.is_none() {
                            slot.omitted_optional()
                        } else {
                            slot
                        }
                    } else {
                        return Err(RuntimeError::new(
                            format!(
                                "过程 `{}` 的必填参数 `{}` 不能省略",
                                procedure.name, param.name
                            ),
                            line,
                        )
                        .with_number(449));
                    }
                }
                Some(expr) => {
                    let slot = self.bind_argument(
                        expr,
                        param,
                        by_value_args.get(index).copied().unwrap_or(false),
                        line,
                    )?;
                    // `ByVal v As Variant` 也要记下内容类型（VB6 实测 `Narrow(i)` 是 2、`Narrow(CSng(1.5))` 是 4）。
                    // `ByRef` 传进来的就是调用方那个槽，标注已经在它身上了，不能覆盖。
                    if slot.ty() == VbType::Variant
                        && !param.is_array
                        && !(param.by_ref && !by_value_args.get(index).copied().unwrap_or(false))
                    {
                        slot.set_narrow(self.narrow_of(Some(expr)));
                    }
                    slot
                }
                None if param.optional => self.bind_default(param, line)?.omitted_optional(),
                None => {
                    // VB6 实测：缺少必填实参是 449（`CallByName(o, "Total", VbMethod, 3)`）。
                    return Err(RuntimeError::new(
                        format!("过程 `{}` 缺少参数 `{}`", procedure.name, param.name),
                        line,
                    )
                    .with_number(449));
                }
            };
            locals.insert(std::borrow::Cow::Borrowed(param.name.as_str()), slot);
        }

        let result = if procedure.is_function {
            let slot = Slot::new(Value::Empty, procedure.ret);
            locals.insert(
                std::borrow::Cow::Borrowed(procedure.name.as_str()),
                slot.clone(),
            );
            Some(slot)
        } else {
            None
        };

        self.frames.push(Frame {
            locals,
            result: result.clone(),
            procedure: Some(procedure.name.as_str()),
            call_line: line,
            ..Frame::default()
        });
        // `With` 是词法作用域：进入过程时把调用方的 With 栈摘下来，返回时还原。
        let outer_with = std::mem::take(&mut self.with_stack);
        let outcome = self.exec_block(&procedure.body);
        self.with_stack = outer_with;
        self.frames.pop();
        outcome?;

        // 返回 `Variant` 的函数：内容类型只有执行完才知道（函数体里 `F = i` 时记在了结果槽上），
        // 顺手交给 `result_narrow`，`VarType(F())` 才能报 `Integer`。
        if procedure.ret == VbType::Variant
            && let Some(slot) = &result
        {
            self.result_narrow = slot.narrow_type();
        }
        Ok(result.map(|slot| slot.value()).unwrap_or(Value::Empty))
    }

    /// 绑定一个实参：变量、数组元素与记录字段按引用共享存储，其余按值传入。
    pub(super) fn bind_argument(
        &mut self,
        expr: &Expr,
        param: &Param,
        force_by_value: bool,
        line: usize,
    ) -> Result<Slot, RuntimeError> {
        if param.by_ref && !force_by_value {
            if let Some(slot) = self.lvalue_slot(expr, param.is_array, line)? {
                return Ok(slot);
            }
            if let Expr::Var(name) = expr {
                return Err(RuntimeError::new(format!("未定义的变量 `{name}`"), line));
            }
        }
        let value = self.eval(expr, line)?;
        Self::bind_value(value, param, line)
    }

    /// 可赋值目标的存储单元：变量、数组元素、记录字段（含 UDT 数组字段的元素）。
    ///
    /// `ByRef` 实参和 `Get #` 的目标共用这一套解析，保证 `Bump a(0)` 与
    /// `Get #1, , a(0)` 落在同一份存储上。
    pub(super) fn lvalue_slot(
        &mut self,
        expr: &Expr,
        want_array: bool,
        line: usize,
    ) -> Result<Option<Slot>, RuntimeError> {
        if want_array {
            return Ok(match expr {
                Expr::Var(name) => self.find_slot(name),
                _ => None,
            });
        }
        match expr {
            Expr::Var(name) => Ok(self.find_slot(name)),
            Expr::Call { name, args } => {
                let Some(slot) = self.find_slot(name) else {
                    return Ok(None);
                };
                let value = slot.value();
                if !value.is_array() {
                    return Ok(None);
                }
                let indices = self.eval_indices(args, line)?;
                let array = value
                    .as_array()
                    .map_err(|message| RuntimeError::new(format!("`{name}` {message}"), line))?;
                let (cell, ty) = array.borrow().element_cell(&indices).map_err(|message| {
                    RuntimeError::new(format!("数组 `{name}`：{message}"), line).with_number(9)
                })?;
                Ok(Some(Slot::from_cell(cell, ty)))
            }
            Expr::MemberIndex {
                base,
                name,
                indices,
            } => {
                let root = self.eval(base, line)?;
                if let Value::Class(instance) = &root {
                    // `obj.Prop(1)`：实例数组字段的元素，或带下标的 Property Get 的存储。
                    let field = instance.0.borrow().field(name);
                    if let Some(field) = field {
                        let is_private = {
                            let borrowed = instance.0.borrow();
                            self.classes
                                .get(borrowed.key())
                                .is_some_and(|def| def.field_is_private(name))
                        };
                        self.ensure_class_member_access(&instance.0, name, is_private, line)?;
                        let value = field.cell.borrow().clone();
                        let indices = self.eval_indices(indices, line)?;
                        let array = value.as_array().map_err(|_| {
                            RuntimeError::new(format!("成员 `{name}` 不是数组"), line)
                        })?;
                        let (cell, ty) =
                            array.borrow().element_cell(&indices).map_err(|message| {
                                RuntimeError::new(format!("成员 `{name}`：{message}"), line)
                                    .with_number(9)
                            })?;
                        return Ok(Some(Slot::from_cell(cell, ty)));
                    }
                    return Ok(None);
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
                let (cell, ty) = array.borrow().element_cell(&indices).map_err(|message| {
                    RuntimeError::new(format!("字段 `{name}`：{message}"), line).with_number(9)
                })?;
                Ok(Some(Slot::from_cell(cell, ty)))
            }
            Expr::Member { base, name } => {
                let root = self.eval(base, line)?;
                if let Value::Class(instance) = &root {
                    let field = instance.0.borrow().field(name);
                    if let Some(field) = field {
                        let is_private = {
                            let borrowed = instance.0.borrow();
                            self.classes
                                .get(borrowed.key())
                                .is_some_and(|def| def.field_is_private(name))
                        };
                        self.ensure_class_member_access(&instance.0, name, is_private, line)?;
                        return Ok(Some(Slot::from_cell(field.cell, field.ty)));
                    }
                    return Ok(None);
                }
                let (parent, last) = Self::record_parent(root, std::slice::from_ref(name), line)?;
                Ok(Some(Self::field_slot(&parent, &last, line)?))
            }
            Expr::WithMember { path } => match self.with_stack.last().cloned() {
                Some(root) => {
                    let (parent, last) = Self::record_parent(root, path, line)?;
                    Ok(Some(Self::field_slot(&parent, &last, line)?))
                }
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// 记录字段的存储单元。
    pub(super) fn field_slot(
        parent: &Rc<RefCell<VbRecord>>,
        name: &str,
        line: usize,
    ) -> Result<Slot, RuntimeError> {
        let parent = parent.borrow();
        let field = parent.fields().iter().find(|field| field.name == name);
        let Some(field) = field else {
            return Err(RuntimeError::new(
                format!("类型 `{}` 没有字段 `{name}`", parent.type_name()),
                line,
            ));
        };
        Ok(Slot::from_cell_with_narrow(
            field.cell(),
            field.ty,
            field.narrow(),
        ))
    }

    pub(super) fn bind_default(
        &mut self,
        param: &Param,
        line: usize,
    ) -> Result<Slot, RuntimeError> {
        let value = match &param.default {
            Some(expr) => self.eval(expr, line)?,
            None if param.is_array => Value::array_new(Vec::new(), param.ty, true),
            None => Value::Empty,
        };
        Self::bind_value(value, param, line)
    }

    pub(super) fn bind_value(
        value: Value,
        param: &Param,
        line: usize,
    ) -> Result<Slot, RuntimeError> {
        if param.is_array {
            // `ByVal arr()` 得到一份独立副本；外层 `ByRef` 分支会共享存储。
            value.as_array().map_err(|_| {
                RuntimeError::new(
                    format!("参数 `{}` 需要数组，实际传入了普通值", param.name),
                    line,
                )
            })?;
            Ok(Slot::new(value.copy_value(), VbType::Variant))
        } else {
            let coerced = value
                .coerce(param.ty)
                .map_err(|message| RuntimeError::from_coerce(message, line))?;
            Ok(Slot::new(coerced, param.ty))
        }
    }

    pub(super) fn call_declare(
        &mut self,
        declare: &'p Declare,
        args: &[Expr],
        by_value_args: &[bool],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let mut values = Vec::with_capacity(declare.params.len());
        for (index, param) in declare.params.iter().enumerate() {
            let Some(expr) = args.get(index) else {
                values.push(ffi::Arg::ByVal(Value::Empty));
                continue;
            };
            if param.by_ref
                && !by_value_args.get(index).copied().unwrap_or(false)
                && let Expr::Var(name) = expr
            {
                match self.find_slot(name) {
                    Some(slot) => {
                        values.push(ffi::Arg::ByRef(slot));
                        continue;
                    }
                    None => {
                        return Err(RuntimeError::new(format!("未定义的变量 `{name}`"), line));
                    }
                }
            }
            values.push(ffi::Arg::ByVal(self.eval(expr, line)?));
        }

        // 结构体里存着的 `AddressOf`：API 可能把指针保存下来以后再回调，
        // 所以这里换成整次脚本运行期间都有效的跳板。
        for value in &values {
            match value {
                ffi::Arg::ByVal(value) => self.materialize_record_callbacks(value, line)?,
                ffi::Arg::ByRef(slot) => {
                    self.materialize_record_callbacks(&slot.value(), line)?;
                }
                ffi::Arg::Callback(_) => {}
            }
        }

        // `AddressOf` 实参：现场生成跳板，调用返回后立刻释放。
        let mut contexts: Vec<Box<CallbackCtx>> = Vec::new();
        let mut trampolines: Vec<Trampoline> = Vec::new();
        // 先记下要换成跳板的实参位置，循环里才好修改 `values`。
        let callbacks: Vec<(usize, String)> = values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| match value {
                ffi::Arg::ByVal(Value::FuncPtr(name)) => Some((index, name.clone())),
                _ => None,
            })
            .collect();
        for (index, name) in callbacks {
            let declared = declare.params[index].ty;
            if !matches!(
                declared,
                VbType::Long
                    | VbType::LongLong
                    | VbType::LongPtr
                    | VbType::Variant
                    | VbType::Any
                    | VbType::Object
            ) {
                return Err(RuntimeError::new(
                    format!(
                        "`{}` 的参数 `{}` 是 {}，`AddressOf` 只能传给整数/指针参数",
                        declare.symbol,
                        declare.params[index].name,
                        declared.name()
                    ),
                    line,
                ));
            }
            let procedure = self.resolve_callback(&name, line)?;
            contexts.push(Box::new(CallbackCtx::new(
                self as *mut Self as *mut (),
                self.generation,
                procedure,
            )));
            let context = &**contexts.last().expect("刚压入") as *const CallbackCtx as usize;
            let thunk = callback_thunk_address(procedure.ret);
            let trampoline = Trampoline::new(context, thunk, procedure.params.len())
                .map_err(|message| RuntimeError::new(message, line))?;
            values[index] = ffi::Arg::Callback(trampoline.address());
            trampolines.push(trampoline);
        }

        // 类型表克隆一份（只复制共享引用），这样 `ffi::call` 期间不再借用 `self`，
        // 回调才能安全地重新进入解释器。
        let types = self.types.clone();
        let outcome =
            ffi::call(declare, &values, &types).map_err(|message| RuntimeError::new(message, line));
        // 回调里出的错跨不过 FFI 边界，调用返回后在这里补报。
        for context in &contexts {
            if let Some(error) = context.error.take() {
                return Err(error);
            }
        }
        for callback in &self.persistent_callbacks {
            if let Some(error) = callback
                .context
                .as_ref()
                .and_then(|context| context.error.take())
            {
                return Err(error);
            }
        }
        outcome
    }

    /// `AddressOf 过程名`：找到回调目标并检查形状是否受支持。
    pub(super) fn resolve_callback(
        &self,
        name: &str,
        line: usize,
    ) -> Result<&'p Procedure, RuntimeError> {
        let Some(procedure) = self.procedures.get(name).copied() else {
            return Err(RuntimeError::new(
                format!("`AddressOf` 找不到过程或函数 `{name}`"),
                line,
            ));
        };
        if procedure.params.len() > crate::interpreter::trampoline::MAX_ABI_ARGS {
            return Err(RuntimeError::new(
                format!(
                    "回调 `{}` 有 {} 个参数，`AddressOf` 最多支持 {} 个",
                    procedure.name,
                    procedure.params.len(),
                    crate::interpreter::trampoline::MAX_ABI_ARGS
                ),
                line,
            ));
        }
        for param in &procedure.params {
            if param.is_array || param.param_array {
                return Err(RuntimeError::new(
                    format!(
                        "回调 `{}` 的数组形参 `{}` 暂不支持",
                        procedure.name, param.name
                    ),
                    line,
                ));
            }
            if param.record.is_some() && !param.by_ref {
                return Err(RuntimeError::new(
                    format!(
                        "回调 `{}` 的 UDT 形参 `{}` 必须声明为 ByRef",
                        procedure.name, param.name
                    ),
                    line,
                ));
            }
            if !matches!(
                param.ty,
                VbType::Byte
                    | VbType::Integer
                    | VbType::Long
                    | VbType::Currency
                    | VbType::Boolean
                    | VbType::LongLong
                    | VbType::LongPtr
                    | VbType::Single
                    | VbType::Double
                    | VbType::Date
                    | VbType::Str
                    | VbType::Any
            ) {
                return Err(RuntimeError::new(
                    format!(
                        "回调 `{}` 的形参 `{}` 是 {}，暂不支持",
                        procedure.name,
                        param.name,
                        param.ty.name()
                    ),
                    line,
                ));
            }
        }
        Ok(procedure)
    }

    /// 把 `AddressOf` 换成持久的跳板地址；同一过程只生成一次。
    pub(super) fn persistent_callback_address(
        &mut self,
        name: &str,
        line: usize,
    ) -> Result<usize, RuntimeError> {
        let key = name.to_ascii_lowercase();
        if let Some(trampoline) = self
            .persistent_callbacks
            .iter()
            .find(|item| item.name == key)
            .and_then(|callback| callback.trampoline.as_ref())
        {
            return Ok(trampoline.address());
        }
        let procedure = self.resolve_callback(name, line)?;
        let context = Box::new(CallbackCtx::new(
            self as *mut Self as *mut (),
            self.generation,
            procedure,
        ));
        let pointer = &*context as *const CallbackCtx as usize;
        let trampoline = Trampoline::new(
            pointer,
            callback_thunk_address(procedure.ret),
            procedure.params.len(),
        )
        .map_err(|message| RuntimeError::new(message, line))?;
        let address = trampoline.address();
        self.persistent_callbacks.push(PersistentCallback {
            name: key,
            context: Some(context),
            trampoline: Some(trampoline),
        });
        Ok(address)
    }

    /// 结构体字段里的 `AddressOf` 换成真实地址：这类回调可能在 `Declare` 返回之后才被调用。
    pub(super) fn materialize_record_callbacks(
        &mut self,
        value: &Value,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let Value::Record(record) = value else {
            return Ok(());
        };
        let fields: Vec<(String, Value)> = {
            let record = record.borrow();
            record
                .fields()
                .iter()
                .map(|field| (field.name.clone(), field.value()))
                .collect()
        };
        for (name, field) in fields {
            match field {
                Value::FuncPtr(procedure) => {
                    let address = self.persistent_callback_address(&procedure, line)?;
                    record
                        .borrow_mut()
                        .set(&name, Value::Int(address as i64))
                        .map_err(|message| RuntimeError::from_coerce(message, line))?;
                }
                Value::Record(_) => self.materialize_record_callbacks(&field, line)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// 从原生回调里进入 VB 过程：实参已经按形参类型还原成值。
    pub(super) fn call_callback_with_params(
        &mut self,
        procedure: &'p Procedure,
        values: Vec<Value>,
    ) -> Result<(Value, Vec<Value>), RuntimeError> {
        self.call_procedure_values_with_params(procedure, values, FastMap::default(), true)
    }

    /// 按「已求值的实参」调用过程；`locals` 是预先注入的局部变量（类方法用它注入实例变量）。
    pub(super) fn call_procedure_values(
        &mut self,
        procedure: &'p Procedure,
        values: Vec<Value>,
        locals: FastMap<std::borrow::Cow<'p, str>, Slot>,
    ) -> Result<Value, RuntimeError> {
        self.call_procedure_values_with_params(procedure, values, locals, false)
            .map(|(value, _)| value)
    }

    pub(super) fn call_procedure_values_with_params(
        &mut self,
        procedure: &'p Procedure,
        values: Vec<Value>,
        mut locals: FastMap<std::borrow::Cow<'p, str>, Slot>,
        capture_params: bool,
    ) -> Result<(Value, Vec<Value>), RuntimeError> {
        let line = procedure.line;
        if self.frames.len() >= self.max_depth {
            return Err(RuntimeError::new(
                format!("调用层次过深（上限 {} 层）", self.max_depth),
                line,
            ));
        }
        if stack_headroom() < MIN_CALL_STACK {
            return Err(RuntimeError::new("调用层次过深（栈空间不足）", line).with_number(28));
        }
        for (index, param) in procedure.params.iter().enumerate() {
            let value = match values.get(index) {
                Some(value) => value.copy_value(),
                None => Value::Empty,
            };
            locals.insert(
                std::borrow::Cow::Borrowed(param.name.as_str()),
                Self::bind_value(value, param, line)?,
            );
        }
        let result = if procedure.is_function {
            let slot = Slot::new(Value::Empty, procedure.ret);
            locals.insert(
                std::borrow::Cow::Borrowed(procedure.name.as_str()),
                slot.clone(),
            );
            Some(slot)
        } else {
            None
        };

        self.frames.push(Frame {
            locals,
            result: result.clone(),
            procedure: Some(procedure.name.as_str()),
            call_line: line,
            ..Frame::default()
        });
        let outer_with = std::mem::take(&mut self.with_stack);
        let outcome = self.exec_block(&procedure.body);
        self.with_stack = outer_with;
        let param_values = if capture_params {
            self.frames.last().map_or_else(Vec::new, |frame| {
                procedure
                    .params
                    .iter()
                    .map(|param| {
                        frame
                            .locals
                            .get(param.name.as_str())
                            .map(Slot::value)
                            .unwrap_or(Value::Empty)
                    })
                    .collect()
            })
        } else {
            Vec::new()
        };
        self.frames.pop();
        outcome?;
        Ok((
            result.map(|slot| slot.value()).unwrap_or(Value::Empty),
            param_values,
        ))
    }
}
