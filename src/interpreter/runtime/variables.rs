//! 变量读写：作用域查找、声明、赋值与类型转换。

use super::*;

impl<'p, 'o> Runtime<'p, 'o> {
    // ---------- 变量 ----------

    /// 把脚本里的路径变成实际路径：绝对路径原样返回，相对路径按脚本当前目录解析。
    pub(super) fn resolve_path(&self, path: &str) -> PathBuf {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            self.cwd.join(path)
        }
    }

    pub(super) fn resolve_moniker_path(&self, pathname: &str) -> String {
        if pathname.contains(':') || pathname.starts_with("new:") {
            pathname.to_string()
        } else {
            self.resolve_path(pathname).to_string_lossy().into_owned()
        }
    }

    pub(super) fn find_slot(&self, name: &str) -> Option<Slot> {
        if let Some(frame) = self.frames.last()
            && let Some(slot) = frame.locals.get(name)
        {
            return Some(slot.clone());
        }
        if let Some(slot) = self.static_of_current(name) {
            return Some(slot.clone());
        }
        self.globals.get(name).cloned()
    }

    /// 只读地借用槽（读变量走这条，省掉 `Slot` 克隆带来的 `Rc` 计数与释放）。
    pub(super) fn lookup_slot(&self, name: &str) -> Option<&Slot> {
        if let Some(frame) = self.frames.last()
            && let Some(slot) = frame.locals.get(name)
        {
            return Some(slot);
        }
        if let Some(slot) = self.static_of_current(name) {
            return Some(slot);
        }
        self.globals.get(name)
    }

    /// 当前过程的 `Static` 局部（`Static n As Long` 与 `Static Sub` 里的隐式变量）。
    ///
    /// 过程体里没有对应的 `Dim` 时也要找得到——隐式变量只可能在上一次调用里建过。
    fn static_of_current(&self, name: &str) -> Option<&Slot> {
        if self.statics.is_empty() {
            return None;
        }
        let procedure = self.frames.last().and_then(|frame| frame.procedure)?;
        self.statics.get(procedure)?.get(name)
    }

    /// 当前作用域：模块级写全局表，过程内写栈顶帧。
    pub(super) fn scope_mut(&mut self) -> &mut FastMap<std::borrow::Cow<'p, str>, Slot> {
        match self.frames.last_mut() {
            Some(frame) => &mut frame.locals,
            None => &mut self.globals,
        }
    }

    pub(super) fn resume_index(&self) -> Option<usize> {
        match self.frames.last() {
            Some(frame) => frame.resume_index,
            None => self.module_resume,
        }
    }

    pub(super) fn set_error_handler(&mut self, handler: Option<ErrorMode>) {
        match self.frames.last_mut() {
            Some(frame) => frame.on_error = handler,
            None => self.module_error = handler,
        }
    }

    pub(super) fn set_resume_index(&mut self, index: usize) {
        match self.frames.last_mut() {
            Some(frame) => frame.resume_index = Some(index),
            None => self.module_resume = Some(index),
        }
    }

    pub(super) fn error_handler(&self) -> Option<ErrorMode> {
        match self.frames.last() {
            Some(frame) => frame.on_error.clone(),
            None => self.module_error.clone(),
        }
    }

    /// 进入一层语句块；返回它是否是过程 模块的根块。
    pub(super) fn enter_block(&mut self) -> bool {
        match self.frames.last_mut() {
            Some(frame) => {
                let root = frame.depth == 0;
                frame.depth += 1;
                root
            }
            None => {
                let root = self.module_depth == 0;
                self.module_depth += 1;
                root
            }
        }
    }

    pub(super) fn exit_block(&mut self) {
        match self.frames.last_mut() {
            Some(frame) => frame.depth = frame.depth.saturating_sub(1),
            None => self.module_depth = self.module_depth.saturating_sub(1),
        }
    }

    pub(super) fn declare(
        &mut self,
        name: &str,
        ty: VbType,
        line: usize,
    ) -> Result<(), RuntimeError> {
        self.declare_slot(name, default_value(ty), ty, line)
            .map(|_| ())
    }

    /// 在当前作用域里声明变量；返回是否新建了单元。
    ///
    /// 同一条 `Dim` 语句被反复执行（例如写在循环体里）不算重复声明：
    /// VB 的声明是按过程算一次的，变量会保留上一轮的值。
    pub(super) fn declare_slot(
        &mut self,
        name: &str,
        value: Value,
        ty: VbType,
        line: usize,
    ) -> Result<bool, RuntimeError> {
        // `Static Sub` 里的声明也全是静态的，和 VB6 实测一致。
        if self.current_procedure_is_static() {
            return self.bind_static_local(name, value, ty, line, None);
        }
        let scope = self.scope_mut();
        if let Some(existing) = scope.get(name) {
            if existing.declared_at == line {
                return Ok(false);
            }
            return Err(RuntimeError::new(format!("变量 `{name}` 重复声明"), line));
        }
        scope.insert(
            std::borrow::Cow::Owned(name.to_string()),
            Slot::new(value, ty).at_line(line),
        );
        Ok(true)
    }

    /// 把变量绑成当前过程的 `Static` 局部：首次进入时创建，之后复用同一份单元。
    ///
    /// 返回是否新建了绑定（同一条 `Dim` 在循环体里重复执行时不再算重复声明）。
    pub(super) fn bind_static_local(
        &mut self,
        name: &str,
        value: Value,
        ty: VbType,
        line: usize,
        narrow: Option<VbType>,
    ) -> Result<bool, RuntimeError> {
        let slot = self.static_slot(name, value, ty, line)?;
        if narrow.is_some() {
            slot.set_narrow(narrow);
        }
        let frame = self.frames.last_mut().ok_or_else(|| {
            RuntimeError::new(format!("`Static` 只能用在过程内部（`{name}`）"), line)
        })?;
        if let Some(existing) = frame.locals.get(name) {
            if existing.declared_at == line {
                return Ok(false);
            }
            return Err(RuntimeError::new(format!("变量 `{name}` 重复声明"), line));
        }
        frame
            .locals
            .insert(std::borrow::Cow::Owned(name.to_string()), slot);
        Ok(true)
    }

    /// 取过程内的 `Static` 单元：首次调用时创建，之后复用。
    pub(super) fn static_slot(
        &mut self,
        name: &str,
        value: Value,
        ty: VbType,
        line: usize,
    ) -> Result<Slot, RuntimeError> {
        let procedure = self
            .frames
            .last()
            .and_then(|frame| frame.procedure)
            .ok_or_else(|| {
                RuntimeError::new(format!("`Static` 只能用在过程内部（`{name}`）"), line)
            })?;
        if let Some(slot) = self.statics.get(procedure).and_then(|vars| vars.get(name)) {
            return Ok(slot.clone());
        }
        let slot = Slot::new(value, ty);
        self.statics
            .entry(procedure)
            .or_default()
            .insert(name.to_string(), slot.clone());
        Ok(slot)
    }

    /// 隐式变量的声明类型：默认 `Variant`，被 `DefInt A-Z` 之类的范围覆盖时取那个类型。
    pub(super) fn implicit_type(&self, name: &str) -> VbType {
        if self.def_types.is_empty() {
            return VbType::Variant;
        }
        let Some(first) = name.chars().next() else {
            return VbType::Variant;
        };
        let first = first.to_ascii_lowercase();
        // 后写的范围覆盖先写的（`DefInt A-Z` 之后再 `DefStr S`）。
        self.def_types
            .iter()
            .rev()
            .find(|(from, to, _)| first >= *from && first <= *to)
            .map(|(_, _, ty)| *ty)
            .unwrap_or(VbType::Variant)
    }

    /// 当前过程是不是 `Static Sub` / `Static Function`（局部变量跨调用保留）。
    pub(super) fn current_procedure_is_static(&self) -> bool {
        self.frames
            .last()
            .and_then(|frame| frame.procedure)
            .and_then(|name| self.procedures.get(name))
            .is_some_and(|procedure| procedure.static_locals)
    }

    pub(super) fn set_var(
        &mut self,
        name: &str,
        value: Value,
        line: usize,
        from: Option<&Expr>,
    ) -> Result<(), RuntimeError> {
        match self.lookup_slot(name) {
            Some(slot) => self.store_in(slot, name, value, line, from),
            None => {
                if self.option_explicit {
                    return Err(RuntimeError::new(
                        format!("变量 `{name}` 未声明（Option Explicit）"),
                        line,
                    ));
                }
                let ty = self.implicit_type(name);
                let narrow = self.narrow_of(from);
                let value = if ty == VbType::Variant {
                    value.copy_value()
                } else {
                    // `DefInt A-Z` 之后的隐式变量就是 Integer：赋值要按声明类型转换，
                    // 与 VB6 实测一致（`A = 40000` 报 6、`D = "text"` 报 13）。
                    value
                        .coerce(ty)
                        .map_err(|message| RuntimeError::from_coerce(message, line))?
                };
                let slot = if self.current_procedure_is_static() {
                    self.bind_static_local(name, value, ty, line, narrow)?;
                    return Ok(());
                } else {
                    Slot::new(value, ty)
                };
                slot.set_narrow(narrow);
                self.scope_mut()
                    .insert(std::borrow::Cow::Owned(name.to_string()), slot);
                Ok(())
            }
        }
    }

    /// `&` 拼接时把 `Single` 值先按 7 位有效数字渲染成字符串（VB6 实测）；其它值原样不动。
    pub(super) fn render_single(&self, expr: &Expr, value: &mut Value) {
        if let Value::Float(number) = value
            && self.static_type(expr) == Some(VbType::Single)
        {
            *value = Value::Str(crate::interpreter::value::format_single(*number));
        }
    }

    /// `Variant` 槽要记下的内容类型（`Variant` / 对象 / 推断不出来的都记为 `None`）。
    pub(super) fn narrow_of(&self, from: Option<&Expr>) -> Option<VbType> {
        let ty = self.static_type(from?)?;
        (!matches!(ty, VbType::Variant | VbType::Any | VbType::Object)).then_some(ty)
    }

    /// 写入已有的单元（`set_var` 与循环计数器共用）。
    pub(super) fn store_in(
        &self,
        slot: &Slot,
        name: &str,
        value: Value,
        line: usize,
        from: Option<&Expr>,
    ) -> Result<(), RuntimeError> {
        if slot.is_readonly() {
            return Err(RuntimeError::new(format!("不能给常量 `{name}` 赋值"), line));
        }
        if slot.is_array_binding() && !value.is_array() {
            return Err(RuntimeError::new(
                format!("`{name}` 是数组，不能整体赋值为普通值"),
                line,
            ));
        }
        slot.set(value)
            .map_err(|message| RuntimeError::from_coerce(message, line))?;
        // 写进 `Variant` 时记下值的 VB6 类型（实测 `v = i`（Integer）之后 `VarType(v)` 是 2）。
        if slot.ty() == VbType::Variant {
            slot.set_narrow(self.narrow_of(from));
        }
        Ok(())
    }
}
