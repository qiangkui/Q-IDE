//! VB 值模型：`Variant` 语义、类型名与转换规则。

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::interpreter::com::ComObject;

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
    Currency,
    Date,
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
            "currency" => Self::Currency,
            "date" => Self::Date,
            "boolean" | "bool" => Self::Boolean,
            "string" | "str" => Self::Str,
            "object" => Self::Object,
            "any" => Self::Any,
            _ => return None,
        })
    }

    /// 标识符类型后缀（`s$`、`x%`、`amount@`）对应的声明类型。
    pub fn from_suffix(suffix: char) -> Option<Self> {
        Some(match suffix {
            '$' => Self::Str,
            '%' => Self::Integer,
            '&' => Self::Long,
            '!' => Self::Single,
            '#' => Self::Double,
            '@' => Self::Currency,
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
            Self::Currency => "Currency",
            Self::Date => "Date",
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
    /// `Variant` 元素的**内容类型**（`arr(0) = CByte(1)` 之后 `VarType(arr(0))` 是 17、
    /// `For Each` 的循环变量也拿到 17）；元素声明类型不是 `Variant` 时为空。
    narrow: Vec<Option<VbType>>,
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
            narrow: vec![None; count],
            dynamic: true,
            record: None,
        }
    }

    /// `Array(CByte(1), 2)`：元素类型来自各实参的静态类型（`For Each` / `VarType` 要用）。
    pub fn from_typed_values(items: Vec<(Value, Option<VbType>)>) -> Self {
        let count = items.len();
        let narrow = items.iter().map(|(_, ty)| *ty).collect();
        Self {
            dims: vec![(0, count)],
            items: items
                .into_iter()
                .map(|(value, _)| new_cell(value))
                .collect(),
            ty: VbType::Variant,
            narrow,
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
            items: (0..total).map(|_| new_cell(fill.copy_value())).collect(),
            ty,
            narrow: if ty == VbType::Variant {
                vec![None; total]
            } else {
                Vec::new()
            },
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

    /// 元素快照 + 每个元素的内容类型（`Variant` 数组的 `For Each` 用）。
    pub fn values_with_types(&self) -> Vec<(Value, Option<VbType>)> {
        self.items
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                let narrow = self.narrow.get(index).copied().flatten();
                (cell.borrow().clone(), narrow)
            })
            .collect()
    }

    /// 某个下标的元素内容类型（`VarType(arr(0))` 用；非 `Variant` 数组返回 `None`）。
    pub fn narrow_at(&self, indices: &[i32]) -> Option<VbType> {
        if self.narrow.is_empty() {
            return None;
        }
        let offset = self.offset(indices).ok()?;
        self.narrow.get(offset).copied().flatten()
    }

    /// 元素存储单元（共享 `Rc`，不复制值），供只需要读一遍的调用方遍历。
    pub fn cells(&self) -> &[Rc<RefCell<Value>>] {
        &self.items
    }

    /// 数组类型名：`Long()`、`Point()` 等。
    pub fn type_name(&self) -> String {
        match &self.record {
            Some((name, _)) => format!("{name}()"),
            None => format!("{}()", self.ty.name()),
        }
    }

    /// 元素声明类型（`VarType` 拼数组类型码用）。
    pub fn element_type(&self) -> VbType {
        self.ty
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

    pub fn get(&self, indices: &[i32]) -> Result<Value, String> {
        let offset = self.offset(indices)?;
        Ok(self.items[offset].borrow().clone())
    }

    pub fn set(
        &mut self,
        indices: &[i32],
        value: Value,
        narrow: Option<VbType>,
    ) -> Result<(), String> {
        if let Some((type_name, _)) = &self.record
            && !value.is_record()
        {
            return Err(format!("该数组的元素必须是 `{type_name}` 记录"));
        }
        let offset = self.offset(indices)?;
        if !self.narrow.is_empty()
            && let Some(slot) = self.narrow.get_mut(offset)
        {
            *slot = narrow;
        }
        if fits_directly(self.ty, &value) {
            *self.items[offset].borrow_mut() = value;
            return Ok(());
        }
        let coerced = value.coerce(self.ty)?;
        *self.items[offset].borrow_mut() = coerced;
        Ok(())
    }

    /// `ReDim [Preserve]`：重建维度；`Preserve` 时按行保留最后一维里已有的元素。
    pub fn redim(&mut self, dims: Vec<(i32, usize)>, ty: Option<VbType>, preserve: bool) {
        let explicit_type = ty.is_some();
        let ty = ty.unwrap_or(self.ty);
        // `ReDim a(n) As String` 会把 UDT 数组改成普通数组。
        let template = if explicit_type {
            None
        } else {
            self.record.clone()
        };
        let fill = match &template {
            Some((_, template)) => template.copy_value(),
            None => default_value(ty),
        };
        let total = dims.iter().map(|(_, count)| *count).product::<usize>();
        let items: Vec<Rc<RefCell<Value>>> =
            (0..total).map(|_| new_cell(fill.copy_value())).collect();
        let mut narrow = if ty == VbType::Variant {
            vec![None; total]
        } else {
            Vec::new()
        };
        if preserve && self.dims.len() == dims.len() && !self.dims.is_empty() {
            let element_ty = if template.is_some() {
                VbType::Variant
            } else {
                ty
            };
            // 行主序：只有最后一维（行内下标）变化，其余维度由调用方保证不变，
            // 因此按「行」逐段搬运，前几维的数据不会串行。
            let old_last = self.dims[self.dims.len() - 1].1;
            let new_last = dims[dims.len() - 1].1;
            let keep = old_last.min(new_last);
            // `old_last` 理论上不会是 0（`array_dims` 保证每一维至少一个元素）。
            let rows = self.items.len().checked_div(old_last).unwrap_or(0);
            for row in 0..rows {
                for column in 0..keep {
                    let old = self.items[row * old_last + column].borrow();
                    *items[row * new_last + column].borrow_mut() =
                        old.coerce(element_ty).unwrap_or_else(|_| fill.copy_value());
                    if let (Some(target), Some(source)) = (
                        narrow.get_mut(row * new_last + column),
                        self.narrow.get(row * old_last + column),
                    ) {
                        *target = *source;
                    }
                }
            }
        }
        self.dims = dims;
        self.items = items;
        self.narrow = narrow;
        self.ty = ty;
        self.dynamic = true;
        self.record = template;
    }

    /// `Erase`：动态数组释放，静态数组清零。
    pub fn erase(&mut self) {
        if self.dynamic {
            self.dims.clear();
            self.items.clear();
            self.narrow.clear();
        } else {
            let fill = self.fill();
            for cell in &self.items {
                *cell.borrow_mut() = fill.copy_value();
            }
            self.narrow.fill(None);
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
            narrow: self.narrow.clone(),
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
    // 对象变量的初值是 `Nothing` 而不是 `Empty`：本机 VB6 实测 `Dim o As Object` 之后
    // `VarType(o)` 是 9、`TypeName(o)` 是 `Nothing`、`IsObject(o)` 为真、`IsEmpty(o)` 为假。
    if matches!(ty, VbType::Object) {
        return Value::Nothing;
    }
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
    /// `As Variant` 字段里当前值的内容类型（`u.f = i`（Integer）之后 `VarType(u.f)` 是 2）。
    ///
    /// 与 `Slot::narrow` 一样要共享：字段可以 `ByRef` 传出去。
    narrow: Rc<std::cell::Cell<Option<VbType>>>,
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
            narrow: Rc::new(std::cell::Cell::new(None)),
        }
    }

    pub fn value(&self) -> Value {
        self.cell.borrow().clone()
    }

    pub fn cell(&self) -> Rc<RefCell<Value>> {
        self.cell.clone()
    }

    pub fn narrow(&self) -> Rc<std::cell::Cell<Option<VbType>>> {
        self.narrow.clone()
    }

    /// `Variant` 字段里当前值的内容类型。
    pub fn narrow_type(&self) -> Option<VbType> {
        self.narrow.get()
    }

    fn set_narrow(&self, ty: Option<VbType>) {
        self.narrow.set(ty);
    }

    /// 写入并记下内容类型（只有 `Variant` 字段用得上，其它字段照常清空标注）。
    pub fn set_with_narrow(&self, value: Value, narrow: Option<VbType>) -> Result<(), String> {
        let coerced = match self.fixed_len {
            Some(len) => Value::Str(pad_fixed(&value.to_display(), len)),
            None => value.coerce(self.ty)?,
        };
        *self.cell.borrow_mut() = coerced;
        self.set_narrow(if self.ty == VbType::Variant {
            narrow
        } else {
            None
        });
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
        self.set_with_narrow(name, value, None)
    }

    /// 写入字段并记下 `Variant` 字段的内容类型。
    pub fn set_with_narrow(
        &mut self,
        name: &str,
        value: Value,
        narrow: Option<VbType>,
    ) -> Result<(), String> {
        let Some(field) = self.fields.iter().find(|field| field.name == name) else {
            return Err(format!("类型 `{}` 没有字段 `{name}`", self.type_name));
        };
        field.set_with_narrow(value, narrow)
    }

    /// 字段的声明类型与内容类型（`VarType(u.f)` 用）。
    pub fn field_types(&self, name: &str) -> Option<(VbType, Option<VbType>)> {
        self.fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| (field.ty, field.narrow_type()))
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
    /// (键, 值, `Variant` 内容类型)；`For Each` 与 `VarType(c(1))` 要用最后一项。
    items: Vec<(Option<String>, Value, Option<VbType>)>,
}

impl VbCollection {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// 元素快照 + 每个元素的内容类型（`For Each` 用）。
    pub fn values_with_types(&self) -> Vec<(Value, Option<VbType>)> {
        self.items
            .iter()
            .map(|(_, value, narrow)| (value.clone(), *narrow))
            .collect()
    }

    pub fn add(
        &mut self,
        value: Value,
        key: Option<String>,
        narrow: Option<VbType>,
    ) -> Result<(), String> {
        if let Some(key) = &key
            && self.position_of_key(key).is_some()
        {
            return Err(format!("键 `{key}` 已经存在于集合中"));
        }
        self.items.push((key, value, narrow));
        Ok(())
    }

    /// 元素的值与内容类型（`VarType(c(1))` 用）。
    pub fn item_with_type(&self, key: &Value) -> Result<(Value, Option<VbType>), String> {
        let position = self.position(key)?;
        let (_, value, narrow) = &self.items[position];
        Ok((value.clone(), *narrow))
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
        self.items.iter().position(|(name, _, _)| {
            name.as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(key))
        })
    }
}

impl Default for VbCollection {
    fn default() -> Self {
        Self::new()
    }
}

/// `Dictionary`：键值对（键大小写不敏感，保持插入顺序）。
///
/// 另存一张「小写键 → 下标」的索引表，`Exists` / `d(键)` 这类查找是 O(1)，
/// 大量插入时不会退化成 O(n²)。
#[derive(Clone, Debug, PartialEq)]
pub struct VbDictionary {
    /// (?, ?, `Variant` ????)?`VarType(d(?))` ???????
    entries: Vec<(String, Value, Option<VbType>)>,
    index: HashMap<String, usize>,
}

impl VbDictionary {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: HashMap::new(),
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
        self.index.contains_key(&key.to_ascii_lowercase())
    }

    pub fn get(&self, key: &str) -> Result<Value, String> {
        self.position(key)
            .map(|index| self.entries[index].1.clone())
            .ok_or_else(|| format!("`Dictionary` 里没有键 `{key}`"))
    }

    /// ?????????`VarType(d("k"))` ???
    pub fn narrow(&self, key: &str) -> Option<VbType> {
        self.position(key).and_then(|index| self.entries[index].2)
    }

    pub fn set(&mut self, key: String, value: Value, narrow: Option<VbType>) {
        match self.index.get(&key.to_ascii_lowercase()).copied() {
            Some(index) => {
                self.entries[index].1 = value;
                self.entries[index].2 = narrow;
            }
            None => {
                self.index
                    .insert(key.to_ascii_lowercase(), self.entries.len());
                self.entries.push((key, value, narrow));
            }
        }
    }

    /// `Add`：键已存在时报错（与 `d(k) = v` 的覆盖语义不同）。
    pub fn add(&mut self, key: String, value: Value, narrow: Option<VbType>) -> Result<(), String> {
        if self.exists(&key) {
            return Err(format!("`Dictionary` 里已经有键 `{key}`"));
        }
        self.index
            .insert(key.to_ascii_lowercase(), self.entries.len());
        self.entries.push((key, value, narrow));
        Ok(())
    }

    pub fn remove(&mut self, key: &str) -> Result<(), String> {
        let Some(index) = self.position(key) else {
            return Err(format!("`Dictionary` 里没有键 `{key}`"));
        };
        self.entries.remove(index);
        // 被删元素之后的下标整体前移一位；重建索引表的代价是 O(n)，与 `Vec::remove` 同阶。
        self.index.clear();
        for (position, (name, _, _)) in self.entries.iter().enumerate() {
            self.index.insert(name.to_ascii_lowercase(), position);
        }
        Ok(())
    }

    pub fn keys(&self) -> Vec<Value> {
        self.entries
            .iter()
            .map(|(key, _, _)| Value::Str(key.clone()))
            .collect()
    }

    pub fn values(&self) -> Vec<Value> {
        self.entries
            .iter()
            .map(|(_, value, _)| value.clone())
            .collect()
    }

    fn position(&self, key: &str) -> Option<usize> {
        self.index.get(&key.to_ascii_lowercase()).copied()
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
    /// 小写类名：类定义表按小写名索引，缓存下来省掉每次成员访问的字符串转换。
    key: String,
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
    /// `As Variant` 字段里当前值的内容类型（与 `Slot` / `RecordField` 同一套语义）。
    pub narrow: Rc<std::cell::Cell<Option<VbType>>>,
}

impl VbInstance {
    pub fn new(class_name: impl Into<String>) -> Self {
        let class_name = class_name.into();
        let key = class_name.to_ascii_lowercase();
        Self {
            class_name,
            key,
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

    /// 小写类名（`classes` 表的键）。
    pub fn key(&self) -> &str {
        &self.key
    }

    /// 追加一条实例变量；重名时覆盖旧值。
    pub fn set_field(&mut self, name: impl Into<String>, ty: VbType, value: Value) {
        let name = name.into();
        if let Some(existing) = self.fields.iter().find(|field| field.name == name) {
            *existing.cell.borrow_mut() = value;
            existing.narrow.set(None);
            return;
        }
        self.fields.push(InstanceField {
            name,
            cell: Rc::new(RefCell::new(value)),
            ty,
            narrow: Rc::new(std::cell::Cell::new(None)),
        });
    }

    pub fn field(&self, name: &str) -> Option<InstanceField> {
        self.fields.iter().find(|field| field.name == name).cloned()
    }

    /// 字段的声明类型与（`Variant` 字段的）内容类型。
    pub fn field_narrow(&self, name: &str) -> Option<(VbType, Option<VbType>)> {
        self.fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| (field.ty, field.narrow.get()))
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

/// 运行期值：保留 VB 标量子类型身份，并用引用值表示数组与对象。
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// 尚未赋值（VB 的 `Empty`）。
    Empty,
    /// 数据库空值（`Null`），参与运算即报错。
    Null,
    /// 空对象引用（`Nothing`）。
    ///
    /// 在 VB6 里它是**独立的值**，不能和 `Empty` 混为一谈（本机实测）：`VarType(Nothing)` 是 9、
    /// `TypeName(Nothing)` 是 `Nothing`、`IsObject(Nothing)` 为真、`IsEmpty(Nothing)` 为假；
    /// 用 `CStr`/`Len`/`&`/算术把它当普通值用是**编译期**错误，运行时按错误号 91 处理。
    /// 声明成对象的变量（`Dim o As Object`）初值就是它。
    Nothing,
    Bool(bool),
    /// 所有整数类型统一用 `i64` 承载，赋值时按声明类型截断。
    Int(i64),
    Float(f64),
    /// VB Currency：有符号 64 位定点数，最低 4 位表示小数（精度 1/10000）。
    Currency(i64),
    /// OLE Automation 日期序列（Double），与普通浮点数保持独立类型身份。
    Date(f64),
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
    /// Windows COM Automation 对象。
    ComObject(ComObject),
    /// `LoadPicture` 的结果。
    ///
    /// QIDE 没有图片控件，也没有图像解码，所以只留来源路径与原文件字节：`TypeName` 报
    /// `Picture`、`VarType` 报 9（对象）、`Is Nothing` 为 `False`（与 VB6 一致）；
    /// `IPicture` 的成员（`Width`/`Height`/`Handle`…）不支持。
    Picture(Rc<PictureValue>),
}

/// `Value::Picture` 的载荷。
#[derive(Debug, PartialEq)]
pub struct PictureValue {
    /// 来源路径（`LoadPicture()` 不带参数时为空串）。
    pub path: String,
    /// 文件内容（空串参数时为空）。
    pub bytes: Vec<u8>,
}

impl Value {
    /// 用一组值构造 0 基动态数组（`Split` / `Array`）。
    pub fn array(items: Vec<Value>) -> Self {
        Self::Array(Rc::new(RefCell::new(VbArray::from_values(items))))
    }

    /// `Array(CByte(1), 2)`：连每个元素的静态类型一起记下来（`For Each` / `VarType` 要用）。
    pub fn array_with_types(items: Vec<(Value, Option<VbType>)>) -> Self {
        Self::Array(Rc::new(RefCell::new(VbArray::from_typed_values(items))))
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

    /// 取集合本体；不是集合时返回 `None`。
    ///
    /// 热路径判类型用这个（或 [`Self::is_collection`]）——以前的 `as_collection()` 失败时会
    /// 现场构造一条错误字符串，放在每次循环都走的分支上等于白分配，已删除。
    pub fn collection_rc(&self) -> Option<Rc<RefCell<VbCollection>>> {
        match self {
            Self::Collection(items) => Some(items.clone()),
            _ => None,
        }
    }

    /// 取字典本体；不是字典时给出可直接展示的错误。
    pub fn as_dictionary(&self) -> Result<Rc<RefCell<VbDictionary>>, String> {
        match self {
            Self::Dictionary(entries) => Ok(entries.clone()),
            _ => Err("该值不是 Dictionary".into()),
        }
    }

    /// 取字典本体；不是字典时返回 `None`（热路径判类型用，理由同 [`Self::collection_rc`]）。
    pub fn dictionary_rc(&self) -> Option<Rc<RefCell<VbDictionary>>> {
        match self {
            Self::Dictionary(entries) => Some(entries.clone()),
            _ => None,
        }
    }

    pub fn is_dictionary(&self) -> bool {
        matches!(self, Self::Dictionary(_))
    }

    /// 是否是对象值：`Collection` / `Dictionary` / 类实例 / COM 对象。
    pub fn is_object(&self) -> bool {
        matches!(
            self,
            Self::Nothing
                | Self::Collection(_)
                | Self::Dictionary(_)
                | Self::Class(_)
                | Self::ComObject(_)
                | Self::Picture(_)
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

    /// 取记录本体；不是记录时返回 `None`（热路径判类型用，理由同 [`Self::collection_rc`]）。
    pub fn record_rc(&self) -> Option<Rc<RefCell<VbRecord>>> {
        match self {
            Self::Record(record) => Some(record.clone()),
            _ => None,
        }
    }

    /// 按值复制：记录与数组会深拷贝；集合、类与 COM 对象保留引用语义。
    pub fn copy_value(&self) -> Value {
        match self {
            Self::Record(record) => Value::Record(Rc::new(RefCell::new(record.borrow().copy()))),
            Self::Array(array) => Value::Array(Rc::new(RefCell::new(array.borrow().copy()))),
            other => other.clone(),
        }
    }

    pub fn type_name(&self) -> String {
        match self {
            Self::Empty => "Empty".into(),
            Self::Null => "Null".into(),
            Self::Nothing => "Nothing".into(),
            Self::Bool(_) => "Boolean".into(),
            Self::Int(_) => "Long".into(),
            Self::Float(_) => "Double".into(),
            Self::Currency(_) => "Currency".into(),
            Self::Date(_) => "Date".into(),
            Self::Str(_) => "String".into(),
            Self::Array(array) => array.borrow().type_name(),
            Self::Record(record) => record.borrow().type_name().to_string(),
            Self::Collection(_) => "Collection".into(),
            Self::Dictionary(_) => "Dictionary".into(),
            Self::FuncPtr(_) => "LongPtr".into(),
            Self::Class(instance) => instance.0.borrow().type_name().to_string(),
            // COM 对象报类型信息里的接口名（VB6 实测 `Dictionary` / `IWshShell3`），取不到时退回 `Object`。
            Self::ComObject(object) => object.type_name().unwrap_or_else(|| "Object".into()),
            Self::Picture(_) => "Picture".into(),
        }
    }

    /// 求值用的真假判定（`If`、`Do While` 等）。
    ///
    /// 字符串只接受 `True`/`False`（大小写不敏感）与数字文本，其余按 VB 报类型不匹配，
    /// 避免 `If "abc" Then` 这类明显写错的代码被当成真。
    pub fn to_bool(&self) -> Result<bool, String> {
        Ok(match self {
            Self::Empty => false,
            Self::Null => return Err("无效使用 Null：Null 不能用作条件".into()),
            // VB6 里 `If Nothing Then` 是运行时错误 91（对象变量未设置）。
            Self::Nothing => return Err("对象变量未设置（Nothing）不能用作条件".into()),
            Self::Bool(value) => *value,
            Self::Int(value) => *value != 0,
            Self::Float(value) => *value != 0.0,
            Self::Currency(value) => *value != 0,
            Self::Date(value) => *value != 0.0,
            Self::Str(text) => {
                if text.eq_ignore_ascii_case("true") {
                    true
                } else if text.eq_ignore_ascii_case("false") {
                    false
                } else {
                    match parse_number(text) {
                        Some(number) => number != 0.0,
                        None => {
                            return Err(format!("类型不匹配：字符串 \"{text}\" 不能转换为布尔值"));
                        }
                    }
                }
            }
            Self::Array(_) => return Err("类型不匹配：数组不能用作条件".into()),
            Self::Record(_) => return Err("类型不匹配：记录不能用作条件".into()),
            Self::Collection(_) => return Err("类型不匹配：集合不能用作条件".into()),
            Self::Dictionary(_) => return Err("类型不匹配：字典不能用作条件".into()),
            Self::FuncPtr(_) => true,
            Self::Class(_) => return Err("类型不匹配：对象不能用作条件".into()),
            Self::ComObject(_) => return Err("类型不匹配：对象不能用作条件".into()),
            Self::Picture(_) => return Err("类型不匹配：对象不能用作条件".into()),
        })
    }

    /// 转整数：字符串按 VB 规则尝试解析。
    pub fn as_int(&self) -> Result<i64, String> {
        match self {
            Self::Empty => Ok(0),
            Self::Null => Err("Null 不能转换为数值".into()),
            Self::Nothing => Err("Nothing 不能转换为数值".into()),
            // VB 的 `True` 转成整数是 -1，与 [`Self::as_float`] 保持一致。
            Self::Bool(value) => Ok(if *value { -1 } else { 0 }),
            Self::Int(value) => Ok(*value),
            Self::Float(value) => Ok(round_half_even(*value) as i64),
            Self::Currency(value) => Ok(round_half_even(*value as f64 / 10_000.0) as i64),
            Self::Date(value) => Ok(round_half_even(*value) as i64),
            Self::Str(text) => parse_number(text)
                .map(|value| round_half_even(value) as i64)
                .ok_or_else(|| format!("类型不匹配：无法把字符串 \"{text}\" 转换为数值")),
            Self::Array(_) => Err("数组不能转换为数值".into()),
            Self::Record(_) => Err("记录不能转换为数值".into()),
            Self::Collection(_) => Err("集合不能转换为数值".into()),
            Self::Dictionary(_) => Err("字典不能转换为数值".into()),
            Self::FuncPtr(name) => Err(format!("`AddressOf {name}` 只能传给 `Declare` 的指针参数")),
            Self::Class(_) => Err("对象不能转换为数值".into()),
            Self::ComObject(_) => Err("对象不能转换为数值".into()),
            Self::Picture(_) => Err("对象不能转换为数值".into()),
        }
    }

    /// 转浮点数。
    pub fn as_float(&self) -> Result<f64, String> {
        match self {
            Self::Empty => Ok(0.0),
            Self::Null => Err("Null 不能转换为数值".into()),
            Self::Nothing => Err("Nothing 不能转换为数值".into()),
            Self::Bool(value) => Ok(if *value { -1.0 } else { 0.0 }),
            Self::Int(value) => Ok(*value as f64),
            Self::Float(value) => Ok(*value),
            Self::Currency(value) => Ok(*value as f64 / 10_000.0),
            Self::Date(value) => Ok(*value),
            Self::Str(text) => parse_number(text)
                .ok_or_else(|| format!("类型不匹配：无法把字符串 \"{text}\" 转换为数值")),
            Self::Array(_) => Err("数组不能转换为数值".into()),
            Self::Record(_) => Err("记录不能转换为数值".into()),
            Self::Collection(_) => Err("集合不能转换为数值".into()),
            Self::Dictionary(_) => Err("字典不能转换为数值".into()),
            Self::FuncPtr(name) => Err(format!("`AddressOf {name}` 只能传给 `Declare` 的指针参数")),
            Self::Class(_) => Err("对象不能转换为数值".into()),
            Self::ComObject(_) => Err("对象不能转换为数值".into()),
            Self::Picture(_) => Err("对象不能转换为数值".into()),
        }
    }

    pub fn as_currency(&self) -> Result<i64, String> {
        match self {
            Self::Currency(value) => Ok(*value),
            _ => currency_from_float(self.as_float()?),
        }
    }

    /// 是否可当作数值参与运算（字符串里是数字也算）。
    pub fn is_numeric_like(&self) -> bool {
        match self {
            Self::Int(_) | Self::Float(_) | Self::Currency(_) | Self::Bool(_) | Self::Empty => true,
            // `IsNumeric("&H10")` 是 True：进制写法整串都要能解析（`IsNumeric("12a")` 是 False）。
            Self::Str(text) => {
                let trimmed = text.trim();
                parse_number(trimmed).is_some()
                    || parse_radix_text(trimmed).is_some_and(|(_, used)| used == trimmed.len())
            }
            Self::Null
            | Self::Nothing
            | Self::Date(_)
            | Self::Array(_)
            | Self::Record(_)
            | Self::Collection(_)
            | Self::Dictionary(_)
            | Self::FuncPtr(_)
            | Self::Class(_)
            | Self::ComObject(_)
            | Self::Picture(_) => false,
        }
    }

    /// VB 里 `CStr` / `Print` 的显示形式。
    pub fn to_display(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Null => "Null".into(),
            // VB6 把「把 `Nothing` 当普通值用」判成**编译期**错误（`CStr(Nothing)`、`Len(Nothing)`、
            // `Nothing & ""` 都编不过），没有运行时行为可对齐；QIDE 按空串处理（与 `Empty` 一致）。
            Self::Nothing => String::new(),
            Self::Bool(value) => {
                if *value {
                    "True".into()
                } else {
                    "False".into()
                }
            }
            Self::Int(value) => value.to_string(),
            Self::Float(value) => format_float(*value),
            Self::Currency(value) => format_currency(*value),
            Self::Date(value) => crate::interpreter::builtins::format_date_display(*value),
            Self::Str(text) => text.clone(),
            Self::Array(array) => {
                let array = array.borrow();
                let joined = array
                    .cells()
                    .iter()
                    .map(|cell| cell.borrow().to_display())
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
            Self::ComObject(_) => "COM Object".into(),
            Self::Picture(picture) => {
                if picture.path.is_empty() {
                    "Picture".into()
                } else {
                    format!("Picture({})", picture.path)
                }
            }
        }
    }

    /// 调试面板里的简短预览：数组/记录只给规模，长字符串截断。
    ///
    /// 直接对数组调用 [`Self::to_display`] 会把每个元素都格式化一遍，
    /// 大数组会让暂停时的界面卡住，所以调试面板改用它。
    pub fn debug_preview(&self, max_chars: usize) -> String {
        let text = match self {
            Self::Array(array) => format!("Array({})", array.borrow().cells().len()),
            Self::Record(record) => {
                let record = record.borrow();
                format!("{}（{} 个字段）", record.type_name(), record.fields().len())
            }
            other => other.to_display(),
        };
        if text.chars().count() <= max_chars {
            return text;
        }
        let mut preview = text
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
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
                    | VbType::Any
                    | VbType::LongPtr
                    | VbType::LongLong
                    | VbType::Long
            )
        {
            return Ok(self.clone());
        }
        Ok(match ty {
            VbType::Variant | VbType::Object | VbType::Any => self.copy_value(),
            VbType::Date => match self {
                Self::Date(value) => Value::Date(*value),
                Self::Str(text) => crate::interpreter::builtins::parse_date_value(text)
                    .map(Value::Date)
                    .ok_or_else(|| format!("类型不匹配：无法把字符串 \"{text}\" 转换为日期"))?,
                _ => Value::Date(self.as_float()?),
            },
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
            VbType::Currency => Value::Currency(self.as_currency()?),
            VbType::Boolean => Value::Bool(self.to_bool()?),
            VbType::Str => {
                // VB6 实测：`s = 对象` 报 450（对象没有可用的默认属性）。
                if matches!(self, Self::ComObject(_)) {
                    return Err("对象没有可用的默认属性，不能转换为字符串".into());
                }
                Value::Str(self.to_display())
            }
        })
    }
}

/// 值与声明类型已经一致、不需要转换时直接入库。
///
/// VB 脚本里绝大多数赋值都是同类型直存（`Long = Long`、`String = String`），
/// 这条快路径省掉 `coerce` 的类型分派；不满足就退回 `coerce`（含范围检查与报错）。
pub fn fits_directly(ty: VbType, value: &Value) -> bool {
    match (value, ty) {
        // 数组与记录在 Variant 里按值语义深拷贝，交给 `coerce`。
        (Value::Array(_) | Value::Record(_), _) => false,
        (_, VbType::Variant | VbType::Object | VbType::Any) => true,
        (Value::Int(number), VbType::Long) => i32::try_from(*number).is_ok(),
        (Value::Int(number), VbType::Integer) => i16::try_from(*number).is_ok(),
        (Value::Int(number), VbType::Byte) => u8::try_from(*number).is_ok(),
        (Value::Int(_), VbType::LongLong | VbType::LongPtr) => true,
        (Value::Float(_), VbType::Double) => true,
        (Value::Bool(_), VbType::Boolean) => true,
        (Value::Str(_), VbType::Str) => true,
        (Value::Date(_), VbType::Date) => true,
        (Value::Currency(_), VbType::Currency) => true,
        _ => false,
    }
}

pub fn currency_from_float(value: f64) -> Result<i64, String> {
    let scaled = value * 10_000.0;
    if !scaled.is_finite() || scaled < i64::MIN as f64 || scaled >= -(i64::MIN as f64) {
        return Err("溢出".into());
    }
    Ok(round_half_even(scaled) as i64)
}

fn format_currency(value: i64) -> String {
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    let whole = magnitude / 10_000;
    let fraction = magnitude % 10_000;
    if fraction == 0 {
        format!("{}{whole}", if negative { "-" } else { "" })
    } else {
        let fraction = format!("{fraction:04}").trim_end_matches('0').to_string();
        format!("{}{whole}.{fraction}", if negative { "-" } else { "" })
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

/// 数字转字符串：整数不带小数点，浮点按 VB6 的 15 位有效数字显示。
///
/// VB6 的 `Debug.Print` / `CStr` 只显示 15 位有效数字，因此 `0.1 + 0.2` 显示 `0.3`
/// 而不是最短往返表示 `0.30000000000000004`；定点写法的**小数位超过 15 位**或**整数位超过 15 位**
/// 时改用科学计数法（本机 VB6 实测：`CStr(1.5E-14)` 是 `0.000000000000015`、
/// `CStr(1.23456789012345E-11)` 是 `1.23456789012345E-11`、`CStr(1E+15)` 是 `1E+15`、
/// `CStr(999900000000000)` 仍是定点）。
pub fn format_float(value: f64) -> String {
    const DIGITS: usize = 15;
    if !value.is_finite() {
        return format!("{value}");
    }
    if value == 0.0 {
        return "0".into();
    }
    let exponent = value.abs().log10().floor();
    let decimals = (DIGITS as f64 - 1.0 - exponent).max(0.0) as usize;
    let fixed = trim_float_text(format!("{value:.decimals$}"));
    let digits_used = fixed
        .trim_start_matches('-')
        .strip_prefix("0.")
        .unwrap_or_else(|| fixed.trim_start_matches('-'))
        .chars()
        .filter(char::is_ascii_digit)
        .count();
    if digits_used <= DIGITS {
        return fixed;
    }
    format_scientific_digits(value, DIGITS)
}

/// `Single` 的打印：VB6 实测用 **7 位有效数字**，定点/科学计数法按「定点写法要占多少位」切换
/// （`CSng(1.23456789)`→`1.234568`、`CSng(0.1)`→`0.1`、`CSng(12345678)`→`1.234568E+07`、
/// `CSng(0.00001)`→`0.00001`、`CSng(0.000123456789)`→`1.234568E-04`、`CSng(1E+20)`→`1E+20`）。
pub fn format_single(value: f64) -> String {
    const DIGITS: usize = 7;
    if !value.is_finite() {
        return format!("{value}");
    }
    if value == 0.0 {
        return "0".into();
    }
    let exponent = value.abs().log10().floor();
    let decimals = (DIGITS as f64 - 1.0 - exponent).max(0.0) as usize;
    let fixed = trim_float_text(format!("{value:.decimals$}"));
    // 定点写法要占多少位（`0.` 前缀不算）：`0.3333333` 是 7 位、`0.0001234568` 是 10 位。
    let digits_used = fixed
        .trim_start_matches('-')
        .strip_prefix("0.")
        .unwrap_or_else(|| fixed.trim_start_matches('-'))
        .chars()
        .filter(char::is_ascii_digit)
        .count();
    if digits_used <= DIGITS {
        return fixed;
    }
    format_scientific_digits(value, DIGITS)
}

/// 科学计数法（`1.5E+20`、`1E-05`），尾数保留 `digits` 位有效数字。
fn format_scientific_digits(value: f64, digits: usize) -> String {
    let mut exponent = value.abs().log10().floor() as i32;
    let mut mantissa = value / 10f64.powi(exponent);
    if !mantissa.is_finite() {
        return format!("{value}");
    }
    let mut text = format!("{mantissa:.*}", digits - 1);
    // 舍入可能把尾数推到 10，需要进位到下一档指数。
    if text.trim_start_matches('-').starts_with("10") {
        exponent += 1;
        mantissa /= 10.0;
        text = format!("{mantissa:.*}", digits - 1);
    }
    format!(
        "{}E{}{:02}",
        trim_float_text(text),
        if exponent < 0 { '-' } else { '+' },
        exponent.abs()
    )
}

/// 去掉定点小数里多余的 0（`2.000` → `2`、`0.30` → `0.3`）。
fn trim_float_text(mut text: String) -> String {
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
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

/// 解析 `&H`（十六进制）/ `&O`（八进制）形式的数字文本，返回 `(值, 消耗的字节数)`。
///
/// `Val`、`IsNumeric` 用；取补码规则与进制字面量一致：装得进 16 位就当 Integer，
/// 否则按 Long（`&HFFFF` 是 -1、`&HFFFF&` 是 65535、`&H80000000` 是 -2147483648）。
pub fn parse_radix_text(text: &str) -> Option<(i64, usize)> {
    let body = text.strip_prefix('&')?;
    let (radix, digits_at) = match body.as_bytes().first() {
        Some(b'h' | b'H') => (16u32, 1usize),
        Some(b'o' | b'O') => (8, 1),
        _ => (8, 0),
    };
    let mut value = 0i64;
    let mut digits = 0usize;
    let mut consumed = 1 + digits_at;
    for ch in body[digits_at..].chars() {
        let Some(digit) = ch.to_digit(radix) else {
            break;
        };
        value = value
            .saturating_mul(i64::from(radix))
            .saturating_add(i64::from(digit));
        digits += 1;
        consumed += ch.len_utf8();
    }
    if digits == 0 {
        return None;
    }
    // `&HFFFF&` 末尾的 `&` 表示按 Long 处理（值不变）。
    let long_suffix = body[consumed - 1..].starts_with('&');
    if long_suffix {
        consumed += 1;
    }
    let digit_limit = if radix == 16 { 4 } else { 6 };
    let value = if long_suffix || digits > digit_limit {
        match u32::try_from(value) {
            Ok(narrow) => i64::from(narrow as i32),
            Err(_) => value,
        }
    } else {
        i64::from(value as u16 as i16)
    };
    Some((value, consumed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_and_float_coerce() {
        assert_eq!(
            Value::Float(3.9).coerce(VbType::Long).unwrap(),
            Value::Int(4)
        );
        assert_eq!(Value::Int(300).coerce(VbType::Byte).unwrap_err(), "溢出");
        assert_eq!(Value::Int(-1).coerce(VbType::Byte).unwrap_err(), "溢出");
        assert_eq!(
            Value::Int(70000).coerce(VbType::Integer).unwrap_err(),
            "溢出"
        );
        assert_eq!(
            Value::Int(2_200_000_000).coerce(VbType::Long).unwrap_err(),
            "溢出"
        );
        assert_eq!(
            Value::Int(255).coerce(VbType::Byte).unwrap(),
            Value::Int(255)
        );
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
    fn currency_coercion_is_fixed_point_and_uses_bankers_rounding() {
        assert_eq!(
            Value::Float(1.23455).coerce(VbType::Currency).unwrap(),
            Value::Currency(12_346)
        );
        assert_eq!(
            Value::Float(1.23445).coerce(VbType::Currency).unwrap(),
            Value::Currency(12_344)
        );
        assert_eq!(
            Value::Float(-1.23455).coerce(VbType::Currency).unwrap(),
            Value::Currency(-12_346)
        );
        assert_eq!(Value::Currency(12_340).to_display(), "1.234");
        assert_eq!(Value::Currency(12_345).to_display(), "1.2345");
        assert_eq!(
            Value::Float(1.0e15).coerce(VbType::Currency).unwrap_err(),
            "溢出"
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
