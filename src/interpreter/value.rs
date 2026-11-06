//! VB 值模型：`Variant` 语义、类型名与转换规则。

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// VB 的声明类型（`Dim x As Long` 里的 `Long`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VbType {
    Variant,
    Byte,
    Integer,
    Long,
    LongLong,
    LongPtr,
    Single,
    Double,
    Boolean,
    Str,
    Object,
    Any,
}

impl VbType {
    /// 把 VB 源码里的类型名转成 [`VbType`]（大小写不敏感）。
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "variant" => Self::Variant,
            "byte" => Self::Byte,
            "integer" | "int" => Self::Integer,
            "long" => Self::Long,
            "longlong" => Self::LongLong,
            "longptr" => Self::LongPtr,
            "single" => Self::Single,
            "double" => Self::Double,
            "boolean" | "bool" => Self::Boolean,
            "string" | "str" => Self::Str,
            "object" => Self::Object,
            "any" => Self::Any,
            _ => return None,
        })
    }

    /// 标识符类型后缀（`s$`、`x%`）对应的声明类型；`@`（Currency）尚未实现。
    pub fn from_suffix(suffix: char) -> Option<Self> {
        Some(match suffix {
            '$' => Self::Str,
            '%' => Self::Integer,
            '&' => Self::Long,
            '!' => Self::Single,
            '#' => Self::Double,
            _ => return None,
        })
    }

    /// 源码里书写的名字。
    pub fn name(self) -> &'static str {
        match self {
            Self::Variant => "Variant",
            Self::Byte => "Byte",
            Self::Integer => "Integer",
            Self::Long => "Long",
            Self::LongLong => "LongLong",
            Self::LongPtr => "LongPtr",
            Self::Single => "Single",
            Self::Double => "Double",
            Self::Boolean => "Boolean",
            Self::Str => "String",
            Self::Object => "Object",
            Self::Any => "Any",
        }
    }
}

/// VB 数组：多维、带下界与元素声明类型。元素按行主序展开存放。
///
/// 每个元素是独立的 `Rc<RefCell<Value>>`，这样 `Bump a(0)` 之类的 `ByRef`
/// 实参可以直接共享元素存储。
#[derive(Clone, Debug, PartialEq)]
pub struct VbArray {
    /// 每维的 `(下界, 元素数)`；空表示尚未 `ReDim` 的动态数组。
    dims: Vec<(i32, usize)>,
    items: Vec<Rc<RefCell<Value>>>,
    /// 元素声明类型，写入时按它转换。
    ty: VbType,
    /// `ReDim` 创建（`Erase` 时释放）；`Dim a(10)` 是静态数组。
    dynamic: bool,
    /// 元素是 UDT 时：类型名 + 初始化模板（`ReDim`/`Erase` 用它重新造元素）。
    record: Option<(String, Value)>,
}

impl VbArray {
    /// 由一组值构造 0 基、元素为 Variant 的动态数组（`Split` / `Array`）。
    pub fn from_values(items: Vec<Value>) -> Self {
        let count = items.len();
        Self {
            dims: vec![(0, count)],
            items: items.into_iter().map(new_cell).collect(),
            ty: VbType::Variant,
            dynamic: true,
            record: None,
        }
    }

    pub fn new(dims: Vec<(i32, usize)>, ty: VbType, dynamic: bool) -> Self {
        let fill = default_value(ty);
        Self::with_fill(dims, ty, dynamic, fill, None)
    }

    /// `Dim a(2) As Point`：元素是 UDT，每个元素独立初始化。
    pub fn new_records(
        dims: Vec<(i32, usize)>,
        dynamic: bool,
        type_name: impl Into<String>,
        template: Value,
    ) -> Self {
        let type_name = type_name.into();
        Self::with_fill(
            dims,
            VbType::Variant,
            dynamic,
            template.clone(),
            Some((type_name, template)),
        )
    }

    fn with_fill(
        dims: Vec<(i32, usize)>,
        ty: VbType,
        dynamic: bool,
        fill: Value,
        record: Option<(String, Value)>,
    ) -> Self {
        let total = dims.iter().map(|(_, count)| *count).product::<usize>();
        Self {
            dims,
            items: (0..total)
                .map(|_| new_cell(fill.copy_value()))
                .collect(),
            ty,
            dynamic,
            record,
        }
    }

    /// 新元素的初值：UDT 元素用模板复制一份。
    fn fill(&self) -> Value {
        match &self.record {
            Some((_, template)) => template.copy_value(),
            None => default_value(self.ty),
        }
    }

    pub fn dims(&self) -> &[(i32, usize)] {
        &self.dims
    }

    /// 元素快照，用于 `Join` / `For Each` 这类线性遍历。
    pub fn values(&self) -> Vec<Value> {
        self.items.iter().map(|cell| cell.borrow().clone()).collect()
    }

    /// 数组类型名：`Long()`、`Point()` 等。
    pub fn type_name(&self) -> String {
        match &self.record {
            Some((name, _)) => format!("{name}()"),
            None => format!("{}()", self.ty.name()),
        }
    }

    pub fn is_dynamic(&self) -> bool {
        self.dynamic
    }

    /// 把一组实际下标换算成 `items` 里的线性位置。
    pub fn offset(&self, indices: &[i32]) -> Result<usize, String> {
        if self.dims.is_empty() {
            return Err("数组尚未初始化（需要先 ReDim）".into());
        }
        if indices.len() != self.dims.len() {
            return Err(format!(
                "需要 {} 个下标，实际给了 {} 个",
                self.dims.len(),
                indices.len()
            ));
        }
        let mut offset = 0usize;
        for (dim, (index, (lower, count))) in indices.iter().zip(&self.dims).enumerate() {
            let relative = i64::from(*index) - i64::from(*lower);
            if relative < 0 || relative >= *count as i64 {
                return Err(format!(
                    "下标 {} 超出第 {} 维的范围 {}..{}",
                    index,
                    dim + 1,
                    lower,
                    i64::from(*lower) + *count as i64 - 1
                ));
            }
            offset = offset * count + relative as usize;
        }
        Ok(offset)
    }

    /// 元素存储单元，供 `ByRef` 实参共享。
    pub fn element_cell(&self, indices: &[i32]) -> Result<(Rc<RefCell<Value>>, VbType), String> {
        let offset = self.offset(indices)?;
        Ok((self.items[offset].clone(), self.ty))
    }

    pub fn element(&self, indices: &[i32]) -> Result<Rc<RefCell<Value>>, String> {
        Ok(self.element_cell(indices)?.0)
    }

    pub fn get(&self, indices: &[i32]) -> Result<Value, String> {
        Ok(self.element(indices)?.borrow().clone())
    }

    pub fn set(&mut self, indices: &[i32], value: Value) -> Result<(), String> {
        if let Some((type_name, _)) = &self.record
            && !value.is_record()
        {
            return Err(format!("该数组的元素必须是 `{type_name}` 记录"));
        }
        let coerced = value.coerce(self.ty)?;
        *self.element(indices)?.borrow_mut() = coerced;
        Ok(())
    }

    /// `ReDim [Preserve]`：重建维度；`Preserve` 时按线性位置保留原有元素。
    pub fn redim(&mut self, dims: Vec<(i32, usize)>, ty: Option<VbType>, preserve: bool) {
        let explicit_type = ty.is_some();
        let ty = ty.unwrap_or(self.ty);
        // `ReDim a(n) As String` 会把 UDT 数组改成普通数组。
        let template = if explicit_type { None } else { self.record.clone() };
        let fill = match &template {
            Some((_, template)) => template.copy_value(),
            None => default_value(ty),
        };
        let total = dims.iter().map(|(_, count)| *count).product::<usize>();
        let mut items: Vec<Rc<RefCell<Value>>> =
            (0..total).map(|_| new_cell(fill.copy_value())).collect();
        if preserve && self.dims.len() == dims.len() {
            let element_ty = if template.is_some() {
                VbType::Variant
            } else {
                ty
            };
            for (cell, old) in items.iter_mut().zip(&self.items) {
                *cell.borrow_mut() = old
                    .borrow()
                    .coerce(element_ty)
                    .unwrap_or_else(|_| fill.copy_value());
            }
        }
        self.dims = dims;
        self.items = items;
        self.ty = ty;
        self.dynamic = true;
        self.record = template;
    }

    /// `Erase`：动态数组释放，静态数组清零。
    pub fn erase(&mut self) {
        if self.dynamic {
            self.dims.clear();
            self.items.clear();
        } else {
            let fill = self.fill();
            for cell in &self.items {
                *cell.borrow_mut() = fill.copy_value();
            }
        }
    }

    /// 按值复制整个数组（VB 的数组赋值语义）。
    fn copy(&self) -> Self {
        Self {
            dims: self.dims.clone(),
            items: self
                .items
                .iter()
                .map(|cell| new_cell(cell.borrow().copy_value()))
                .collect(),
            ty: self.ty,
            dynamic: self.dynamic,
            record: self.record.clone(),
        }
    }
}

fn new_cell(value: Value) -> Rc<RefCell<Value>> {
    Rc::new(RefCell::new(value))
}

/// 某声明类型的默认初值（数值 0、字符串 ""、布尔 False）。
pub fn default_value(ty: VbType) -> Value {
    Value::Empty.coerce(ty).unwrap_or(Value::Empty)
}

/// `Type ... End Type` 记录里的一个字段。
///
/// 字段值放在独立的存储单元里，这样 `Bump p.X` 这类 `ByRef` 实参可以共享字段。
#[derive(Clone, Debug, PartialEq)]
pub struct RecordField {
    pub name: String,
    pub ty: VbType,
    /// `String * n`：赋值时按空格补齐/截断到固定长度。
    fixed_len: Option<usize>,
    cell: Rc<RefCell<Value>>,
}

impl RecordField {
    pub fn new(name: impl Into<String>, ty: VbType, value: Value) -> Self {
        Self::with_fixed(name, ty, value, None)
    }

    /// `String * n` 字段。
    pub fn fixed(name: impl Into<String>, ty: VbType, value: Value, len: usize) -> Self {
        Self::with_fixed(name, ty, value, Some(len))
    }

    fn with_fixed(
        name: impl Into<String>,
        ty: VbType,
        value: Value,
        fixed_len: Option<usize>,
    ) -> Self {
        Self {
            name: name.into(),
            ty,
            fixed_len,
            cell: Rc::new(RefCell::new(value)),
        }
    }

    pub fn value(&self) -> Value {
        self.cell.borrow().clone()
    }

    pub fn cell(&self) -> Rc<RefCell<Value>> {
        self.cell.clone()
    }

    pub fn set(&self, value: Value) -> Result<(), String> {
        let coerced = match self.fixed_len {
            Some(len) => Value::Str(pad_fixed(&value.to_display(), len)),
            None => value.coerce(self.ty)?,
        };
        *self.cell.borrow_mut() = coerced;
        Ok(())
    }
}

/// 定长字符串：不足补空格，超出截断。
pub fn pad_fixed(text: &str, len: usize) -> String {
    let mut chars: Vec<char> = text.chars().take(len).collect();
    chars.resize(len, ' ');
    chars.into_iter().collect()
}

/// 用户自定义类型（UDT）的运行期值。
#[derive(Clone, Debug, PartialEq)]
pub struct VbRecord {
    type_name: String,
    fields: Vec<RecordField>,
}

impl VbRecord {
    pub fn new(type_name: impl Into<String>, fields: Vec<RecordField>) -> Self {
        Self {
            type_name: type_name.into(),
            fields,
        }
    }

    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    pub fn fields(&self) -> &[RecordField] {
        &self.fields
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.fields
            .iter()
            .find(|field| field.name == name)
            .map(RecordField::value)
    }

    pub fn set(&mut self, name: &str, value: Value) -> Result<(), String> {
        let Some(field) = self.fields.iter().find(|field| field.name == name) else {
            return Err(format!("类型 `{}` 没有字段 `{name}`", self.type_name));
        };
        field.set(value)
    }

    /// 字段存储单元，供 `ByRef` 实参共享。
    pub fn field_cell(&self, name: &str) -> Option<(Rc<RefCell<Value>>, VbType)> {
        self.fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| (field.cell(), field.ty))
    }

    /// 按值复制整条记录（VB 的 UDT 赋值语义）。
    fn copy(&self) -> Self {
        Self {
            type_name: self.type_name.clone(),
            fields: self
                .fields
                .iter()
                .map(|field| {
                    RecordField::with_fixed(
                        field.name.clone(),
                        field.ty,
                        field.value().copy_value(),
                        field.fixed_len,
                    )
                })
                .collect(),
        }
    }
}

/// VB 的 `Collection`：按插入顺序保存元素，可选字符串键（键大小写不敏感）。
#[derive(Clone, Debug, PartialEq)]
pub struct VbCollection {
    items: Vec<(Option<String>, Value)>,
}

impl VbCollection {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    pub fn values(&self) -> Vec<Value> {
        self.items.iter().map(|(_, value)| value.clone()).collect()
    }

    pub fn add(&mut self, value: Value, key: Option<String>) -> Result<(), String> {
        if let Some(key) = &key
            && self.position_of_key(key).is_some()
        {
            return Err(format!("键 `{key}` 已经存在于集合中"));
        }
        self.items.push((key, value));
        Ok(())
    }

    pub fn item(&self, key: &Value) -> Result<Value, String> {
        let position = self.position(key)?;
        Ok(self.items[position].1.clone())
    }

    pub fn remove(&mut self, key: &Value) -> Result<(), String> {
        let position = self.position(key)?;
        self.items.remove(position);
        Ok(())
    }

    /// 按下标（1 基）或字符串键定位。
    fn position(&self, key: &Value) -> Result<usize, String> {
        match key {
            Value::Str(text) => self
                .position_of_key(text)
                .ok_or_else(|| format!("集合里没有键 `{text}`")),
            other => {
                let index = other.as_int()?;
                if index < 1 || index as usize > self.items.len() {
                    return Err(format!(
                        "集合下标 {index} 超出范围（1..{}）",
                        self.items.len()
                    ));
                }
                Ok(index as usize - 1)
            }
        }
    }

    fn position_of_key(&self, key: &str) -> Option<usize> {
        self.items
            .iter()
            .position(|(name, _)| name.as_deref().is_some_and(|name| name.eq_ignore_ascii_case(key)))
    }
}

impl Default for VbCollection {
    fn default() -> Self {
        Self::new()
    }
}

/// `Dictionary`：键值对（键大小写不敏感，保持插入顺序）。
#[derive(Clone, Debug, PartialEq)]
pub struct VbDictionary {
    entries: Vec<(String, Value)>,
}

impl VbDictionary {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// 键名必须是非空字符串。
    pub fn key_of(value: &Value) -> Result<String, String> {
        let key = value.to_display();
        if key.is_empty() {
            return Err("`Dictionary` 的键不能为空".into());
        }
        Ok(key)
    }

    pub fn exists(&self, key: &str) -> bool {
        self.position(key).is_some()
    }

    pub fn get(&self, key: &str) -> Result<Value, String> {
        self.position(key)
            .map(|index| self.entries[index].1.clone())
            .ok_or_else(|| format!("`Dictionary` 里没有键 `{key}`"))
    }

    pub fn set(&mut self, key: String, value: Value) {
        match self.position(&key) {
            Some(index) => self.entries[index].1 = value,
            None => self.entries.push((key, value)),
        }
    }

    /// `Add`：键已存在时报错（与 `d(k) = v` 的覆盖语义不同）。
    pub fn add(&mut self, key: String, value: Value) -> Result<(), String> {
        if self.exists(&key) {
            return Err(format!("`Dictionary` 里已经有键 `{key}`"));
        }
        self.entries.push((key, value));
        Ok(())
    }

    pub fn remove(&mut self, key: &str) -> Result<(), String> {
        let Some(index) = self.position(key) else {
            return Err(format!("`Dictionary` 里没有键 `{key}`"));
        };
        self.entries.remove(index);
        Ok(())
    }

    pub fn keys(&self) -> Vec<Value> {
        self.entries
            .iter()
            .map(|(key, _)| Value::Str(key.clone()))
            .collect()
    }

    pub fn values(&self) -> Vec<Value> {
        self.entries.iter().map(|(_, value)| value.clone()).collect()
    }

    fn position(&self, key: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|(name, _)| name.eq_ignore_ascii_case(key))
    }
}

impl Default for VbDictionary {
    fn default() -> Self {
        Self::new()
    }
}

/// `Class ... End Class` 的实例：类名 + 实例变量。
///
/// 实例是引用语义（`Set`、赋值、传参共享同一份），成员变量的存储用 `Rc<RefCell<Value>>`，
/// 和 `ByRef` 传参、记录字段共用同一套共享单元。
#[derive(Debug)]
pub struct VbInstance {
    class_name: String,
    fields: Vec<InstanceField>,
    /// `Class_Terminate` 是否已经跑过。
    terminated: std::cell::Cell<bool>,
}

/// 一条实例变量。
#[derive(Clone, Debug)]
pub struct InstanceField {
    pub name: String,
    pub cell: Rc<RefCell<Value>>,
    pub ty: VbType,
}

impl VbInstance {
    pub fn new(class_name: impl Into<String>) -> Self {
        Self {
            class_name: class_name.into(),
            fields: Vec::new(),
            terminated: std::cell::Cell::new(false),
        }
    }

    /// 标记 `Class_Terminate` 已经执行过；返回 true 表示这是第一次。
    pub fn begin_terminate(&self) -> bool {
        !self.terminated.replace(true)
    }

    pub fn type_name(&self) -> &str {
        &self.class_name
    }

    /// 追加一条实例变量；重名时覆盖旧值。
    pub fn set_field(&mut self, name: impl Into<String>, ty: VbType, value: Value) {
        let name = name.into();
        if let Some(existing) = self.fields.iter().find(|field| field.name == name) {
            *existing.cell.borrow_mut() = value;
            return;
        }
        self.fields.push(InstanceField {
            name,
            cell: Rc::new(RefCell::new(value)),
            ty,
        });
    }

    pub fn field(&self, name: &str) -> Option<InstanceField> {
        self.fields
            .iter()
            .find(|field| field.name == name)
            .cloned()
    }

    pub fn fields(&self) -> &[InstanceField] {
        &self.fields
    }
}

impl Drop for VbInstance {
    fn drop(&mut self) {
        // 最后一个引用消失时跑 `Class_Terminate`（只有脚本执行期间才能回调解释器）。
        crate::interpreter::runtime::terminate_instance(self);
    }
}

/// `Value::Class` 的载荷：按引用比较（同一个实例才相等）。
#[derive(Clone, Debug)]
pub struct ClassRef(pub Rc<RefCell<VbInstance>>);

impl PartialEq for ClassRef {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// 运行期值：VB 的 `Variant` 只分「整数 / 浮点 / 字符串 / 布尔 / 空」几类。
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// 尚未赋值（VB 的 `Empty`）。
    Empty,
    /// 数据库空值（`Null`），参与运算即报错。
    Null,
    Bool(bool),
    /// 所有整数类型统一用 `i64` 承载，赋值时按声明类型截断。
    Int(i64),
    Float(f64),
    Str(String),
    /// VB 数组（`Dim a(10)`、`ReDim`、`Split`、`Array` 都会产生它）。
    Array(Rc<RefCell<VbArray>>),
    /// `Type ... End Type` 定义的记录；`Rc` 让 `ByRef` 传参共享同一份字段。
    Record(Rc<RefCell<VbRecord>>),
    /// `Collection` 对象；对象是引用语义，赋值/`Set` 共享同一份。
    Collection(Rc<RefCell<VbCollection>>),
    /// `Dictionary` 对象（键值对）。
    Dictionary(Rc<RefCell<VbDictionary>>),
    /// `AddressOf 过程名`：回调入口，只能传给 `Declare` 声明的指针参数。
    FuncPtr(String),
    /// `Class ... End Class` 的实例（引用语义）。
    Class(ClassRef),
}

impl Value {
    /// 用一组值构造 0 基动态数组（`Split` / `Array`）。
    pub fn array(items: Vec<Value>) -> Self {
        Self::Array(Rc::new(RefCell::new(VbArray::from_values(items))))
    }

    /// 按维度与元素类型构造数组。
    pub fn array_new(dims: Vec<(i32, usize)>, ty: VbType, dynamic: bool) -> Self {
        Self::Array(Rc::new(RefCell::new(VbArray::new(dims, ty, dynamic))))
    }

    /// 构造 UDT 元素数组；每个元素都是模板的一份独立副本。
    pub fn array_records(
        dims: Vec<(i32, usize)>,
        dynamic: bool,
        type_name: impl Into<String>,
        template: Value,
    ) -> Self {
        Self::Array(Rc::new(RefCell::new(VbArray::new_records(
            dims, dynamic, type_name, template,
        ))))
    }

    pub fn record(type_name: impl Into<String>, fields: Vec<RecordField>) -> Self {
        Self::Record(Rc::new(RefCell::new(VbRecord::new(type_name, fields))))
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }

    pub fn is_record(&self) -> bool {
        matches!(self, Self::Record(_))
    }

    pub fn is_collection(&self) -> bool {
        matches!(self, Self::Collection(_))
    }

    /// 新建一个空的 `Collection`。
    pub fn collection() -> Self {
        Self::Collection(Rc::new(RefCell::new(VbCollection::new())))
    }

    /// 新建一个空的 `Dictionary`。
    pub fn dictionary() -> Self {
        Self::Dictionary(Rc::new(RefCell::new(VbDictionary::new())))
    }

    /// 取集合本体；不是集合时给出可直接展示的错误。
    pub fn as_collection(&self) -> Result<Rc<RefCell<VbCollection>>, String> {
        match self {
            Self::Collection(items) => Ok(items.clone()),
            _ => Err("该值不是 Collection".into()),
        }
    }

    /// 取字典本体；不是字典时给出可直接展示的错误。
    pub fn as_dictionary(&self) -> Result<Rc<RefCell<VbDictionary>>, String> {
        match self {
            Self::Dictionary(entries) => Ok(entries.clone()),
            _ => Err("该值不是 Dictionary".into()),
        }
    }

    pub fn is_dictionary(&self) -> bool {
        matches!(self, Self::Dictionary(_))
    }

    /// 是否是对象值：`Collection` / `Dictionary` / `Class` 实例。
    pub fn is_object(&self) -> bool {
        matches!(
            self,
            Self::Collection(_) | Self::Dictionary(_) | Self::Class(_)
        )
    }

    /// 取数组本体；不是数组时给出可直接展示的错误。
    pub fn as_array(&self) -> Result<Rc<RefCell<VbArray>>, String> {
        match self {
            Self::Array(array) => Ok(array.clone()),
            _ => Err("该值不是数组".into()),
        }
    }

    /// 取记录本体；不是记录时给出可直接展示的错误。
    pub fn as_record(&self) -> Result<Rc<RefCell<VbRecord>>, String> {
        match self {
            Self::Record(record) => Ok(record.clone()),
            _ => Err("该值不是记录类型".into()),
        }
    }

    /// 按值复制：记录与数组会深拷贝（VB 的赋值语义），集合是引用语义（共享同一份）。
    pub fn copy_value(&self) -> Value {
        match self {
            Self::Record(record) => {
                Value::Record(Rc::new(RefCell::new(record.borrow().copy())))
            }
            Self::Array(array) => {
                Value::Array(Rc::new(RefCell::new(array.borrow().copy())))
            }
            other => other.clone(),
        }
    }

    pub fn type_name(&self) -> String {
        match self {
            Self::Empty => "Empty".into(),
            Self::Null => "Null".into(),
            Self::Bool(_) => "Boolean".into(),
            Self::Int(_) => "Long".into(),
            Self::Float(_) => "Double".into(),
            Self::Str(_) => "String".into(),
            Self::Array(array) => array.borrow().type_name(),
            Self::Record(record) => record.borrow().type_name().to_string(),
            Self::Collection(_) => "Collection".into(),
            Self::Dictionary(_) => "Dictionary".into(),
            Self::FuncPtr(_) => "LongPtr".into(),
            Self::Class(instance) => instance.0.borrow().type_name().to_string(),
        }
    }

    /// 求值用的真假判定（`If`、`Do While` 等）。
    pub fn to_bool(&self) -> Result<bool, String> {
        Ok(match self {
            Self::Empty => false,
            Self::Null => return Err("Null 不能用作条件".into()),
            Self::Bool(value) => *value,
            Self::Int(value) => *value != 0,
            Self::Float(value) => *value != 0.0,
            Self::Str(value) => !value.is_empty() && !value.eq_ignore_ascii_case("false"),
            Self::Array(_) => return Err("数组不能用作条件".into()),
            Self::Record(_) => return Err("记录不能用作条件".into()),
            Self::Collection(_) => return Err("集合不能用作条件".into()),
            Self::Dictionary(_) => return Err("字典不能用作条件".into()),
            Self::FuncPtr(_) => true,
            Self::Class(_) => return Err("对象不能用作条件".into()),
        })
    }

    /// 转整数：字符串按 VB 规则尝试解析。
    pub fn as_int(&self) -> Result<i64, String> {
        match self {
            Self::Empty => Ok(0),
            Self::Null => Err("Null 不能转换为数值".into()),
            Self::Bool(value) => Ok(i64::from(*value)),
            Self::Int(value) => Ok(*value),
            Self::Float(value) => Ok(round_half_even(*value) as i64),
            Self::Str(text) => parse_number(text)
                .map(|value| round_half_even(value) as i64)
                .ok_or_else(|| format!("无法把字符串 \"{text}\" 转换为数值")),
            Self::Array(_) => Err("数组不能转换为数值".into()),
            Self::Record(_) => Err("记录不能转换为数值".into()),
            Self::Collection(_) => Err("集合不能转换为数值".into()),
            Self::Dictionary(_) => Err("字典不能转换为数值".into()),
            Self::FuncPtr(name) => Err(format!(
                "`AddressOf {name}` 只能传给 `Declare` 的指针参数"
            )),
            Self::Class(_) => Err("对象不能转换为数值".into()),
        }
    }

    /// 转浮点数。
    pub fn as_float(&self) -> Result<f64, String> {
        match self {
            Self::Empty => Ok(0.0),
            Self::Null => Err("Null 不能转换为数值".into()),
            Self::Bool(value) => Ok(if *value { -1.0 } else { 0.0 }),
            Self::Int(value) => Ok(*value as f64),
            Self::Float(value) => Ok(*value),
            Self::Str(text) => parse_number(text)
                .ok_or_else(|| format!("无法把字符串 \"{text}\" 转换为数值")),
            Self::Array(_) => Err("数组不能转换为数值".into()),
            Self::Record(_) => Err("记录不能转换为数值".into()),
            Self::Collection(_) => Err("集合不能转换为数值".into()),
            Self::Dictionary(_) => Err("字典不能转换为数值".into()),
            Self::FuncPtr(name) => Err(format!(
                "`AddressOf {name}` 只能传给 `Declare` 的指针参数"
            )),
            Self::Class(_) => Err("对象不能转换为数值".into()),
        }
    }

    /// 是否可当作数值参与运算（字符串里是数字也算）。
    pub fn is_numeric_like(&self) -> bool {
        match self {
            Self::Int(_) | Self::Float(_) | Self::Bool(_) | Self::Empty => true,
            Self::Str(text) => parse_number(text).is_some(),
            Self::Null
            | Self::Array(_)
            | Self::Record(_)
            | Self::Collection(_)
            | Self::Dictionary(_)
            | Self::FuncPtr(_)
            | Self::Class(_) => false,
        }
    }

    /// VB 里 `CStr` / `Print` 的显示形式。
    pub fn to_display(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Null => "Null".into(),
            Self::Bool(value) => {
                if *value {
                    "True".into()
                } else {
                    "False".into()
                }
            }
            Self::Int(value) => value.to_string(),
            Self::Float(value) => format_float(*value),
            Self::Str(text) => text.clone(),
            Self::Array(array) => {
                let array = array.borrow();
                let joined = array
                    .values()
                    .iter()
                    .map(Value::to_display)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Array({joined})")
            }
            Self::Record(record) => {
                let record = record.borrow();
                let fields = record
                    .fields()
                    .iter()
                    .map(|field| format!("{}={}", field.name, field.value().to_display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({fields})", record.type_name())
            }
            Self::Collection(items) => format!("Collection({})", items.borrow().count()),
            Self::Dictionary(entries) => format!("Dictionary({})", entries.borrow().count()),
            Self::FuncPtr(name) => format!("AddressOf {name}"),
            Self::Class(instance) => instance.0.borrow().type_name().to_string(),
        }
    }

    /// 调试面板里的简短预览：数组/记录只给规模，长字符串截断。
    ///
    /// 直接对数组调用 [`Self::to_display`] 会把每个元素都格式化一遍，
    /// 大数组会让暂停时的界面卡住，所以调试面板改用它。
    pub fn debug_preview(&self, max_chars: usize) -> String {
        let text = match self {
            Self::Array(array) => format!("Array({})", array.borrow().values().len()),
            Self::Record(record) => {
                let record = record.borrow();
                format!("{}（{} 个字段）", record.type_name(), record.fields().len())
            }
            other => other.to_display(),
        };
        if text.chars().count() <= max_chars {
            return text;
        }
        let mut preview = text.chars().take(max_chars.saturating_sub(1)).collect::<String>();
        preview.push('…');
        preview
    }

    /// 按目标类型做一次赋值转换（VB 会在赋值处隐式转换）。
    pub fn coerce(&self, ty: VbType) -> Result<Value, String> {
        if !matches!(ty, VbType::Variant | VbType::Object | VbType::Any) {
            if self.is_array() {
                return Err(format!("数组不能转换为 {}", ty.name()));
            }
            if self.is_record() {
                return Err(format!("记录不能转换为 {}", ty.name()));
            }
        }
        // `p = AddressOf Foo`：回调入口按指针语义原样保留，等待传给 `Declare`。
        if matches!(self, Self::FuncPtr(_))
            && matches!(
                ty,
                VbType::Variant
                    | VbType::Object
                    | VbType::Any                    | VbType::LongPtr
                    | VbType::LongLong
                    | VbType::Long
            )
        {
            return Ok(self.clone());
        }
        Ok(match ty {
            VbType::Variant | VbType::Object | VbType::Any => self.copy_value(),
            VbType::Byte => Value::Int(i64::from(
                u8::try_from(self.as_int()?).map_err(|_| "溢出".to_string())?,
            )),
            VbType::Integer => Value::Int(i64::from(
                i16::try_from(self.as_int()?).map_err(|_| "溢出".to_string())?,
            )),
            VbType::Long => Value::Int(i64::from(
                i32::try_from(self.as_int()?).map_err(|_| "溢出".to_string())?,
            )),
            VbType::LongLong | VbType::LongPtr => Value::Int(self.as_int()?),
            VbType::Single => {
                let value = self.as_float()?;
                let narrowed = value as f32;
                if narrowed.is_infinite() && value.is_finite() {
                    return Err("溢出".to_string());
                }
                Value::Float(f64::from(narrowed))
            }
            VbType::Double => Value::Float(self.as_float()?),
            VbType::Boolean => Value::Bool(self.to_bool()?),
            VbType::Str => Value::Str(self.to_display()),
        })
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_display())
    }
}

/// VB 的 `Round` / `CInt` 使用银行家舍入。
pub fn round_half_even(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let floor = value.floor();
    let fraction = value - floor;
    if (fraction - 0.5).abs() < f64::EPSILON {
        if (floor as i64) % 2 == 0 {
            floor
        } else {
            floor + 1.0
        }
    } else {
        value.round()
    }
}

/// 数字转字符串：整数不带小数点，浮点去掉多余的 0。
pub fn format_float(value: f64) -> String {
    if !value.is_finite() {
        return format!("{value}");
    }
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let text = format!("{value}");
    text
}

/// 解析数字字面量（`123`、`-1.5e3`、`&HFF` 已在词法阶段处理）。
pub fn parse_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_and_float_coerce() {
        assert_eq!(Value::Float(3.9).coerce(VbType::Long).unwrap(), Value::Int(4));
        assert_eq!(Value::Int(300).coerce(VbType::Byte).unwrap_err(), "溢出");
        assert_eq!(Value::Int(-1).coerce(VbType::Byte).unwrap_err(), "溢出");
        assert_eq!(Value::Int(70000).coerce(VbType::Integer).unwrap_err(), "溢出");
        assert_eq!(
            Value::Int(2_200_000_000).coerce(VbType::Long).unwrap_err(),
            "溢出"
        );
        assert_eq!(Value::Int(255).coerce(VbType::Byte).unwrap(), Value::Int(255));
        assert_eq!(
            Value::Float(1.0e39).coerce(VbType::Single).unwrap_err(),
            "溢出"
        );
        assert_eq!(
            Value::Str("12".into()).coerce(VbType::Integer).unwrap(),
            Value::Int(12)
        );
    }

    #[test]
    fn banker_rounding_matches_vb() {
        assert_eq!(round_half_even(0.5), 0.0);
        assert_eq!(round_half_even(1.5), 2.0);
        assert_eq!(round_half_even(2.5), 2.0);
        assert_eq!(round_half_even(-1.5), -2.0);
    }

    #[test]
    fn display_matches_vb() {
        assert_eq!(Value::Bool(true).to_display(), "True");
        assert_eq!(Value::Float(2.0).to_display(), "2");
        assert_eq!(Value::Empty.to_display(), "");
    }
}
