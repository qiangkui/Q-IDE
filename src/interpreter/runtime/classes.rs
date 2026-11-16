//! `Class ... End Class` 的实例化、字段与方法调用。

use super::*;

impl<'p, 'o> Runtime<'p, 'o> {
    // ---------- 类（`Class ... End Class`）----------

    /// `Dim` 条目的初始值：`New` → 对象，`As 记录` → 新建记录，`As 类` → 空对象，数组 → 分配。
    pub(super) fn declared_value(
        &mut self,
        item: &Declared,
        line: usize,
    ) -> Result<(Value, VbType), RuntimeError> {
        // `Dim arr As Integer() = {1, 2, 3}`：声明时就带上初值。
        if let Some(initializer) = &item.initializer {
            let value = self.eval(initializer, line)?;
            return Ok((value, VbType::Variant));
        }
        if let Some(kind) = &item.new_object {
            let value = match kind {
                NewObject::Collection => Value::collection(),
                NewObject::Dictionary => Value::dictionary(),
                NewObject::Class(name) => self.instantiate(name, line)?,
            };
            return Ok((value, VbType::Object));
        }
        if let Some(bounds) = &item.bounds {
            let dims = self.array_dims(bounds, line)?;
            let dynamic = bounds.is_empty();
            return Ok(match &item.record {
                Some(type_name) => {
                    let template = self.new_record(type_name, line)?;
                    let value = Value::array_records(dims, dynamic, template.type_name(), template);
                    (value, VbType::Variant)
                }
                // 数组的元素类型存在数组里，槽本身持 Variant 保存。
                None => (Value::array_new(dims, item.ty, dynamic), VbType::Variant),
            });
        }
        if let Some(type_name) = &item.record {
            if self.is_class(type_name) {
                // `Dim p As Class1`：`Set p = New Class1` 之前是空对象。
                return Ok((Value::Empty, VbType::Object));
            }
            return Ok((self.new_record(type_name, line)?, VbType::Variant));
        }
        Ok((default_value(item.ty), item.ty))
    }

    pub(super) fn is_class(&self, name: &str) -> bool {
        self.classes.contains_key(&name.to_ascii_lowercase())
    }

    /// 当前正在执行的方法所属的类（不在方法里时返回 `None`）。
    pub(super) fn current_class(&self) -> Option<&'p ClassDef> {
        let instance = self.current_instance()?;
        let instance = instance.borrow();
        self.classes.get(instance.key()).copied()
    }

    /// 当前方法里的 `Me` 实例。
    pub(super) fn current_instance(&self) -> Option<Rc<RefCell<VbInstance>>> {
        let frame = self.frames.last()?;
        match frame.locals.get("me").map(Slot::value) {
            Some(Value::Class(instance)) => Some(instance.0),
            _ => None,
        }
    }

    /// 裸调用同类成员时沿用的局部变量（实例变量 + `Me`）。
    pub(super) fn current_instance_locals(&self) -> FastMap<std::borrow::Cow<'p, str>, Slot> {
        match self.current_instance() {
            Some(instance) => Self::instance_locals(&instance),
            None => FastMap::default(),
        }
    }

    /// 执行 `Class_Terminate`：把实例变量注入帧，再跑成员过程（错误只能吞掉）。
    pub(super) fn terminate_instance(&mut self, instance: &VbInstance) {
        let Some(def) = self.classes.get(instance.key()).copied() else {
            return;
        };
        let Some(terminate) = def.method("class_terminate") else {
            return;
        };
        let mut locals = FastMap::with_capacity_and_hasher(
            instance.fields().len(),
            std::hash::BuildHasherDefault::default(),
        );
        for field in instance.fields() {
            locals.insert(
                field.name.clone().into(),
                Slot::from_cell(field.cell.clone(), field.ty),
            );
        }
        let _ = self.call_user_with_locals(terminate, &[], locals, terminate.line);
    }

    /// `New 类名`：按类定义建实例（实例变量取默认值），再调用 `Class_Initialize`。
    pub(super) fn instantiate(
        &mut self,
        class_name: &str,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let key = class_name.to_ascii_lowercase();
        let Some(def) = self.classes.get(&key).copied() else {
            return Err(
                RuntimeError::new(format!("未定义的类 `{class_name}`"), line).with_number(429),
            );
        };
        let instance = Rc::new(RefCell::new(VbInstance::new(def.display.clone())));
        // 字段声明先克隆出来，避免在 `self` 可变借用期间还持有类定义的引用。
        let fields = def.fields.clone();
        for item in &fields {
            let (value, ty) = self.declared_value(item, line)?;
            instance
                .borrow_mut()
                .set_field(item.name.clone(), ty, value);
        }
        if let Some(init) = def.method("class_initialize") {
            let locals = Self::instance_locals(&instance);
            self.call_user_with_locals(init, &[], locals, line)?;
        }
        // `WithEvents` 字段若是对象，立刻订阅它的事件。
        for item in &fields {
            if !item.with_events {
                continue;
            }
            let source = instance
                .borrow()
                .field(&item.name)
                .map(|field| field.cell.borrow().clone());
            if let Some(Value::Class(source)) = source {
                Self::register_event_sink(&mut self.events, &source.0, &instance);
            }
        }
        Ok(Value::Class(ClassRef(instance)))
    }

    /// 把 `sink` 登记为 `source` 的事件订阅者（弱引用，不延长生命周期）。
    pub(super) fn register_event_sink(
        events: &mut HashMap<usize, Vec<Weak<RefCell<VbInstance>>>>,
        source: &Rc<RefCell<VbInstance>>,
        sink: &Rc<RefCell<VbInstance>>,
    ) {
        let key = Rc::as_ptr(source) as usize;
        let entry = events.entry(key).or_default();
        let sink_weak = Rc::downgrade(sink);
        let already =
            entry
                .iter()
                .any(|existing| match (existing.upgrade(), sink_weak.upgrade()) {
                    (Some(a), Some(b)) => Rc::ptr_eq(&a, &b),
                    _ => false,
                });
        if !already {
            entry.push(sink_weak);
        }
    }

    /// 类方法执行时的起始局部变量：实例变量（共享存储）+ `Me`。
    pub(super) fn instance_locals(
        instance: &Rc<RefCell<VbInstance>>,
    ) -> FastMap<std::borrow::Cow<'p, str>, Slot> {
        let borrowed = instance.borrow();
        let fields = borrowed.fields();
        // 预分配容量，避免字段较多时反复扩容。
        let mut locals = FastMap::with_capacity_and_hasher(
            fields.len() + 1,
            std::hash::BuildHasherDefault::default(),
        );
        for field in fields {
            locals.insert(
                field.name.clone().into(),
                Slot::from_cell_with_narrow(field.cell.clone(), field.ty, field.narrow.clone()),
            );
        }
        drop(borrowed);
        locals.insert(
            "me".into(),
            Slot::new(Value::Class(ClassRef(instance.clone())), VbType::Object),
        );
        locals
    }

    /// `obj.Method(args)`：带上实例变量执行成员过程。
    pub(super) fn call_class_method(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        name: &str,
        args: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let def = {
            let instance = instance.borrow();
            self.classes.get(instance.key()).copied()
        };
        let Some(def) = def else {
            let display = instance.borrow().type_name().to_string();
            return Err(RuntimeError::new(format!("未定义的类 `{display}`"), line));
        };
        let method = def.method(name);
        let implemented = method.is_none();
        let Some(method) = method.or_else(|| def.implemented_method(name)) else {
            let display = instance.borrow().type_name().to_string();
            return Err(
                RuntimeError::new(format!("类 `{display}` 没有成员 `{name}`"), line)
                    .with_number(438),
            );
        };
        if !implemented {
            self.ensure_class_member_access(instance, name, method.is_private, line)?;
        }
        let locals = Self::instance_locals(instance);
        self.call_user_with_locals(method, args, locals, line)
    }

    pub(super) fn ensure_class_member_access(
        &self,
        instance: &Rc<RefCell<VbInstance>>,
        name: &str,
        is_private: bool,
        line: usize,
    ) -> Result<(), RuntimeError> {
        if !is_private {
            return Ok(());
        }
        let class_name = instance.borrow().key().to_string();
        if self
            .current_class()
            .is_some_and(|current| current.name == class_name)
        {
            return Ok(());
        }
        Err(RuntimeError::new(
            format!("类成员 `{name}` 是 `Private`，只能在所属类中访问"),
            line,
        )
        .with_number(438))
    }

    /// `obj.成员`：无参成员过程优先当属性读，其次读实例变量。
    pub(super) fn read_instance_member(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        name: &str,
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let def = {
            let instance = instance.borrow();
            self.classes.get(instance.key()).copied()
        };
        if let Some(def) = def
            && let Some(method) = def.method(name).or_else(|| def.implemented_method(name))
        {
            if def.method(name).is_some() {
                self.ensure_class_member_access(instance, name, method.is_private, line)?;
            }
            if method.params.is_empty() {
                let locals = Self::instance_locals(instance);
                return self.call_user_with_locals(method, &[], locals, line);
            }
            let display = instance.borrow().type_name().to_string();
            return Err(RuntimeError::new(
                format!(
                    "类 `{display}` 的成员 `{name}` 需要 {} 个参数（写成 `.{name}(...)`）",
                    method.params.len()
                ),
                line,
            )
            .with_number(438));
        }
        if let Some(field) = instance.borrow().field(name) {
            let is_private = def.is_some_and(|def| def.field_is_private(name));
            self.ensure_class_member_access(instance, name, is_private, line)?;
            // `Variant` 字段的内容类型也一并交给 `result_narrow`（`VarType(obj.Field)` 用）。
            if field.ty == VbType::Variant {
                self.result_narrow = field.narrow.get();
            }
            return Ok(field.cell.borrow().clone());
        }
        let display = instance.borrow().type_name().to_string();
        Err(RuntimeError::new(format!("类 `{display}` 没有成员 `{name}`"), line).with_number(438))
    }

    /// `obj.成员 = 值` / `Set obj.成员 = 对象`：有对应 Property 就走它，否则直接写实例变量。
    pub(super) fn assign_instance_member(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        path: &[String],
        value: &Expr,
        set_op: bool,
        line: usize,
    ) -> Result<(), RuntimeError> {
        if path.len() > 1 {
            // `obj.Inner.X = 值` / `obj.Self2.Value = 值`：先把第一层取出来，
            // 再按取到的容器（类 / COM / 记录）继续往下写。
            let mut current = Value::Class(ClassRef(instance.clone()));
            for step in &path[..path.len() - 1] {
                current = self.read_member(&Expr::Literal(current), step, line)?;
            }
            let last = path.last().expect("path 长度大于 1");
            return self.assign_value_member(current, last, value, set_op, line);
        }
        let name = path
            .first()
            .ok_or_else(|| RuntimeError::new("缺少成员名", line))?;
        let def = {
            let instance = instance.borrow();
            self.classes.get(instance.key()).copied()
        };
        if let Some(def) = def {
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
                return self
                    .call_user_with_locals(setter, std::slice::from_ref(value), locals, line)
                    .map(|_| ());
            }
        }
        if let Some(field) = instance.borrow().field(name) {
            let is_private = def.is_some_and(|def| def.field_is_private(name));
            self.ensure_class_member_access(instance, name, is_private, line)?;
            // `Variant` 字段要记下右侧表达式的静态类型（`c.P = i` 之后 `VarType(c.P)` 是 2）。
            let narrow = self.narrow_of(Some(value));
            let value = self
                .eval(value, line)?
                .coerce(field.ty)
                .map_err(|message| RuntimeError::from_coerce(message, line))?;
            let with_events = def
                .and_then(|def| {
                    def.fields
                        .iter()
                        .find(|item| item.name == *name)
                        .map(|item| item.with_events)
                })
                .unwrap_or(false);
            if with_events && let Value::Class(source) = &value {
                Self::register_event_sink(&mut self.events, &source.0, instance);
            }
            if with_events
                && let Value::ComObject(object) = &value
                && let Some(def) = def
            {
                // 类字段上的 `WithEvents`：处理过程是同类的 `字段_事件`。
                self.connect_field_events(object, def, name, instance, line)?;
            }
            *field.cell.borrow_mut() = value;
            field.narrow.set(if field.ty == VbType::Variant {
                narrow
            } else {
                None
            });
            return Ok(());
        }
        if let Some(def) = def
            && let Some(method) = def.method(name).or_else(|| def.implemented_method(name))
        {
            if def.method(name).is_some() {
                self.ensure_class_member_access(instance, name, method.is_private, line)?;
            }
            let display = instance.borrow().type_name().to_string();
            return Err(RuntimeError::new(
                format!("类 `{display}` 的 `{name}` 是只读成员（缺少 `Property Let`）"),
                line,
            )
            .with_number(438));
        }
        let display = instance.borrow().type_name().to_string();
        Err(RuntimeError::new(format!("类 `{display}` 没有成员 `{name}`"), line).with_number(438))
    }

    /// 成员链的中间一层：`obj.A.B` / `obj.Method(实参).B` 里把这一段读成值。
    ///
    /// 带括号（`indices` 非空，或上游解析出空括号）表示**调用**；不带括号按成员读，
    /// 与表达式里 `obj.Self2()` / `obj.Self3` 的规则一致。
    pub(super) fn read_member_step(
        &mut self,
        current: Value,
        name: &str,
        indices: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        if indices.is_empty() {
            return self.read_member(&Expr::Literal(current), name, line);
        }
        match &current {
            Value::Class(instance) => self.call_class_method(&instance.0, name, indices, line),
            Value::ComObject(object) => {
                let mut values = Vec::with_capacity(indices.len());
                for expr in indices {
                    values.push(self.eval(expr, line)?);
                }
                self.call_com_with_values(object, name, values, line)
            }
            _ => Err(
                RuntimeError::new(format!("值不是对象，不能调用 `{name}(...)`"), line)
                    .with_number(438),
            ),
        }
    }

    /// 把值写到容器（类实例 / COM 对象 / 记录）的 `name` 成员上。
    pub(super) fn assign_value_member(
        &mut self,
        container: Value,
        name: &str,
        value: &Expr,
        set_op: bool,
        line: usize,
    ) -> Result<(), RuntimeError> {
        match &container {
            Value::Class(instance) => {
                self.assign_instance_member(&instance.0, &[name.to_string()], value, set_op, line)
            }
            Value::ComObject(object) => {
                let value = self.eval(value, line)?;
                object.set(name, &value).map_err(|failure| {
                    RuntimeError::new(failure.message, line).with_number(failure.number)
                })
            }
            _ => {
                if !container.is_record() {
                    return Err(RuntimeError::new(
                        format!("不能给 `{name}` 赋值：所在的值不是对象或记录"),
                        line,
                    )
                    .with_number(424));
                }
                let narrow = self.narrow_of(Some(value));
                let value = self.eval(value, line)?;
                self.assign_member(container, &[name.to_string()], value, narrow, line)
            }
        }
    }

    /// 走完成员路径（最后一段带 `indices`，空括号也算调用）拿到链中间的对象。
    pub(super) fn read_member_chain(
        &mut self,
        root: Value,
        path: &[String],
        indices: &[Expr],
        line: usize,
    ) -> Result<Value, RuntimeError> {
        let (last, parents) = path
            .split_last()
            .ok_or_else(|| RuntimeError::new("缺少成员名", line))?;
        let mut current = root;
        for step in parents {
            current = self.read_member(&Expr::Literal(current), step, line)?;
        }
        self.read_member_step(current, last, indices, line)
    }

    /// 沿 `member_path` 读到最后一级，再把值写上去（`member_path` 至少一级）。
    pub(super) fn assign_through_members(
        &mut self,
        container: Value,
        member_path: &[String],
        value: &Expr,
        set_op: bool,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let (target, steps) = member_path
            .split_last()
            .ok_or_else(|| RuntimeError::new("缺少成员名", line))?;
        let mut current = container;
        for step in steps {
            current = self.read_member(&Expr::Literal(current), step, line)?;
        }
        self.assign_value_member(current, target, value, set_op, line)
    }

    /// `obj.Prop(下标) = 值` / `Set obj.Prop(下标) = 对象`：交给带参数的 `Property Let/Set`。
    pub(super) fn assign_instance_index(
        &mut self,
        instance: &Rc<RefCell<VbInstance>>,
        name: &str,
        indices: &[Expr],
        value: &Expr,
        set_op: bool,
        line: usize,
    ) -> Result<(), RuntimeError> {
        let def = {
            let instance = instance.borrow();
            self.classes.get(instance.key()).copied()
        };
        let Some(def) = def else {
            let display = instance.borrow().type_name().to_string();
            return Err(RuntimeError::new(format!("未定义的类 `{display}`"), line));
        };
        let display = || instance.borrow().type_name().to_string();
        let setter = if set_op {
            def.property_set(name)
                .or_else(|| def.implemented_property_set(name))
        } else {
            def.property_let(name)
                .or_else(|| def.implemented_property_let(name))
        };
        let Some(setter) = setter else {
            return Err(RuntimeError::new(
                format!("类 `{}` 没有可写的 `{name}(下标)`", display()),
                line,
            )
            .with_number(438));
        };
        let direct = if set_op {
            def.property_set(name).is_some()
        } else {
            def.property_let(name).is_some()
        };
        if direct {
            self.ensure_class_member_access(instance, name, setter.is_private, line)?;
        }
        let mut args: Vec<Expr> = indices.to_vec();
        args.push(value.clone());
        let locals = Self::instance_locals(instance);
        self.call_user_with_locals(setter, &args, locals, line)?;
        Ok(())
    }
}
