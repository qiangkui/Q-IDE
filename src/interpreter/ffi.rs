//! 动态调用 Win32 API：
//!
//! 脚本里的 `Declare Function Foo Lib "user32" (...) As Long` 会在运行时通过
//! `LoadLibraryW` + `GetProcAddress` 解析，再用与参数个数匹配的函数指针发起调用。
//!
//! 说明：Win64 下通过小型调用桩按参数位置装载通用寄存器、XMM 寄存器与栈参数，
//! 支持最多 64 个整数、`Single`、`Double` 标量任意组合。
//!
//! `ByRef` / `ByVal` UDT 按字段类型与自然对齐打包，支持嵌套记录、定长字符串与定长数组；
//! 变长字符串字段不支持。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[cfg(windows)]
use std::ffi::c_void;

use crate::interpreter::ast::{Declare, TypeDef};
use crate::interpreter::runtime::Slot;
use crate::interpreter::value::{Value, VbRecord, VbType, pad_fixed};

/// 本地时间（`GetLocalTime` 的结果）。
pub struct LocalTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
    pub millisecond: i32,
}

#[cfg(windows)]
#[repr(C)]
struct RawSystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

/// 取本地时间；非 Windows 返回 `None`（调用方回退到 UTC）。
#[cfg(windows)]
pub fn local_time() -> Option<LocalTime> {
    let mut raw = RawSystemTime {
        year: 0,
        month: 0,
        day_of_week: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        milliseconds: 0,
    };
    unsafe { platform::GetLocalTime(&mut raw) };
    if raw.year == 0 {
        return None;
    }
    Some(LocalTime {
        year: i32::from(raw.year),
        month: i32::from(raw.month),
        day: i32::from(raw.day),
        hour: i32::from(raw.hour),
        minute: i32::from(raw.minute),
        second: i32::from(raw.second),
        millisecond: i32::from(raw.milliseconds),
    })
}

#[cfg(not(windows))]
pub fn local_time() -> Option<LocalTime> {
    None
}

/// 按引用传字符串时分配的缓冲区大小（UTF-16 码元）。
const OUT_TEXT_CAPACITY: usize = 4096;

/// 一次 Win32 调用的实参。
pub enum Arg {
    ByVal(Value),
    ByRef(Slot),
    /// `AddressOf 过程名`：`context` 指向解释器的回调上下文，调用时现生成跳板。
    Callback(usize),
}

thread_local! {
    static MODULES: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
    static SYMBOLS: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
}

/// 待传入的参数缓冲；必须活到调用结束。
enum Prepared {
    Immediate(usize),
    Text(Vec<u16>),
    OutNumber(Box<u64>),
    OutText(Vec<u16>),
    /// `ByRef p As POINT`：打包好的结构体字节缓冲。
    Struct(Vec<u8>),
    /// `ByVal p As POINT`：按 x64 ABI 传寄存器（≤ 8 字节）或临时副本指针。
    ByValueStruct {
        buffer: Vec<u8>,
        size: usize,
    },
    /// `Double` / `Date` 参数（走 XMM 寄存器）。
    Float(f64),
    /// `Single` 参数（走 XMM 寄存器的低 32 位）。
    Single(f32),
}

impl Prepared {
    /// 整数/指针参数对应的寄存器值。
    fn register_value(&mut self) -> Result<usize, String> {
        Ok(match self {
            Prepared::Immediate(value) => *value,
            Prepared::Text(buffer) => buffer.as_ptr() as usize,
            Prepared::OutNumber(cell) => (&mut **cell as *mut u64) as usize,
            Prepared::OutText(buffer) => buffer.as_mut_ptr() as usize,
            Prepared::Struct(buffer) => buffer.as_mut_ptr() as usize,
            Prepared::ByValueStruct { buffer, size } => {
                if *size <= 8 {
                    // 小结构体按值放进寄存器（低位在前）。
                    let mut bits = [0u8; 8];
                    bits[..*size].copy_from_slice(&buffer[..*size]);
                    u64::from_le_bytes(bits) as usize
                } else {
                    buffer.as_mut_ptr() as usize
                }
            }
            Prepared::Float(_) | Prepared::Single(_) => {
                return Err("浮点参数不能放进通用寄存器".into());
            }
        })
    }
}

/// 混合调用时一个实参的实际形态。
#[derive(Clone, Copy)]
enum MixedArg {
    Int(usize),
    Float(f64),
}

/// 调用的原始返回值。
enum Raw {
    Int(usize),
    F32(f32),
    F64(f64),
}

/// 调用一个已声明的外部函数。
pub fn call(
    decl: &Declare,
    args: &[Arg],
    types: &HashMap<String, &TypeDef>,
) -> Result<Value, String> {
    let address = resolve(&decl.lib, &decl.symbol)?;
    if args.len() > decl.params.len() {
        return Err(format!(
            "`{}` 最多接受 {} 个参数，实际传入 {} 个",
            decl.symbol,
            decl.params.len(),
            args.len()
        ));
    }
    if decl.params.len() > crate::interpreter::trampoline::MAX_ABI_ARGS {
        return Err(format!(
            "`{}` 的参数超过 {} 个，暂不支持",
            decl.symbol,
            crate::interpreter::trampoline::MAX_ABI_ARGS
        ));
    }

    let mut prepared = Vec::with_capacity(decl.params.len());
    // 结构体参数调用结束后要按布局回读。
    let mut structs: Vec<Option<(Slot, StructLayout)>> =
        (0..decl.params.len()).map(|_| None).collect();
    for (index, param) in decl.params.iter().enumerate() {
        let prepared_arg = if let Some(Arg::Callback(address)) = args.get(index) {
            // 跳板由调用方（运行时）生成并保管，这里只当作指针传下去。
            Prepared::Immediate(*address)
        } else if let Some(record_name) = &param.record {
            let display = types
                .get(record_name)
                .map(|def| def.display.clone())
                .unwrap_or_else(|| record_name.clone());
            let layout = struct_layout(record_name, types, 0)?;
            let mut buffer = vec![0u8; layout.size.max(1)];
            match args.get(index) {
                Some(Arg::ByRef(slot)) if param.by_ref => {
                    let value = slot.value();
                    let record = value
                        .as_record()
                        .map_err(|_| format!("参数 `{}` 需要 `{display}` 记录", param.name))?;
                    write_record(&mut buffer, &layout, &record.borrow())?;
                    structs[index] = Some((slot.clone(), layout));
                    Prepared::Struct(buffer)
                }
                Some(Arg::ByVal(value)) if !param.by_ref => {
                    let record = value
                        .as_record()
                        .map_err(|_| format!("参数 `{}` 需要 `{display}` 记录", param.name))?;
                    write_record(&mut buffer, &layout, &record.borrow())?;
                    Prepared::ByValueStruct {
                        buffer,
                        size: layout.size,
                    }
                }
                _ => {
                    return Err(format!(
                        "`{}` 的结构体参数 `{}` 需要传入记录值",
                        decl.symbol, param.name
                    ));
                }
            }
        } else {
            match args.get(index) {
                Some(Arg::ByRef(slot)) if param.by_ref => match param.ty {
                    VbType::Str => Prepared::OutText(vec![0u16; OUT_TEXT_CAPACITY]),
                    VbType::Double | VbType::Date => {
                        Prepared::OutNumber(Box::new(slot.value().as_float()?.to_bits()))
                    }
                    VbType::Single => Prepared::OutNumber(Box::new(
                        (slot.value().as_float()? as f32).to_bits() as u64,
                    )),
                    VbType::Currency => {
                        Prepared::OutNumber(Box::new(slot.value().as_currency()? as u64))
                    }
                    _ => Prepared::OutNumber(Box::new(slot.value().as_int()? as u64)),
                },
                Some(Arg::ByRef(slot)) => prepare_by_value(&slot.value(), param.ty)?,
                Some(Arg::ByVal(value)) => prepare_by_value(value, param.ty)?,
                Some(Arg::Callback(_)) => {
                    return Err(format!(
                        "`{}` 的参数 `{}` 不接受 `AddressOf`",
                        decl.symbol, param.name
                    ));
                }
                None => prepare_by_value(&Value::Empty, param.ty)?,
            }
        };
        prepared.push(prepared_arg);
    }

    // 小规模调用仍用静态签名；扩展组合由动态调用桩处理。
    let float_count = prepared
        .iter()
        .filter(|item| matches!(item, Prepared::Float(_)))
        .count();
    let single_count = prepared
        .iter()
        .filter(|item| matches!(item, Prepared::Single(_)))
        .count();
    let use_dynamic = prepared.len() > 4 || (single_count > 0 && single_count != prepared.len());
    let raw = if use_dynamic {
        let mut call_args = Vec::with_capacity(prepared.len());
        for item in prepared.iter_mut() {
            call_args.push(match item {
                Prepared::Float(value) => crate::interpreter::trampoline::InvokeArg::Double(*value),
                Prepared::Single(value) => {
                    crate::interpreter::trampoline::InvokeArg::Single(*value)
                }
                other => {
                    crate::interpreter::trampoline::InvokeArg::Integer(other.register_value()?)
                }
            });
        }
        match crate::interpreter::trampoline::invoke(address, &call_args, decl.ret)? {
            crate::interpreter::trampoline::InvokeResult::Integer(value) => Raw::Int(value),
            crate::interpreter::trampoline::InvokeResult::Single(value) => Raw::F32(value),
            crate::interpreter::trampoline::InvokeResult::Double(value) => Raw::F64(value),
        }
    } else if float_count == 0 && single_count == 0 {
        let mut raw_args = Vec::with_capacity(prepared.len());
        for item in prepared.iter_mut() {
            raw_args.push(item.register_value()?);
        }
        match decl.ret {
            VbType::Single => Raw::F32(unsafe { invoke_f32_return(address, &raw_args) }),
            VbType::Double | VbType::Date => {
                Raw::F64(unsafe { invoke_f64_return(address, &raw_args) })
            }
            _ => Raw::Int(unsafe { invoke(address, &raw_args) }),
        }
    } else if single_count == prepared.len() {
        let values: Vec<f32> = prepared
            .iter()
            .map(|item| match item {
                Prepared::Single(value) => Ok(*value),
                _ => Err("`Single` 参数列表里混入了其它类型".to_string()),
            })
            .collect::<Result<_, _>>()?;
        match decl.ret {
            VbType::Single => Raw::F32(unsafe { invoke_single_args_f32_return(address, &values) }),
            VbType::Double | VbType::Date => {
                Raw::F64(unsafe { invoke_single_args_f64_return(address, &values) })
            }
            _ => Raw::Int(unsafe { invoke_single_args_int_return(address, &values) }),
        }
    } else if float_count == prepared.len() {
        let float_args: Vec<f64> = prepared
            .iter()
            .map(|item| match item {
                Prepared::Float(value) => Ok(*value),
                _ => Err("`Double` 参数列表里混入了其它类型".to_string()),
            })
            .collect::<Result<_, _>>()?;
        match decl.ret {
            VbType::Single => {
                Raw::F32(unsafe { invoke_float_args_f32_return(address, &float_args) })
            }
            VbType::Double | VbType::Date => {
                Raw::F64(unsafe { invoke_float_args_f64_return(address, &float_args) })
            }
            _ => Raw::Int(unsafe { invoke_float_args_int_return(address, &float_args) }),
        }
    } else {
        // 整数与 `Double` 混合：按位掩码挑专用签名。
        let mut mixed = Vec::with_capacity(prepared.len());
        for item in prepared.iter_mut() {
            mixed.push(match item {
                Prepared::Float(value) => MixedArg::Float(*value),
                other => MixedArg::Int(other.register_value()?),
            });
        }
        dispatch_mixed(address, &mixed, decl.ret)?
    };

    // 结构体：属性值按布局回写到原记录。
    for (index, entry) in structs.into_iter().enumerate() {
        let Some((slot, layout)) = entry else {
            continue;
        };
        let Prepared::Struct(buffer) = &prepared[index] else {
            continue;
        };
        let value = slot.value();
        let record = value
            .as_record()
            .map_err(|_| format!("结构体参数 `{}` 已不是记录", decl.params[index].name))?;
        read_into_record(buffer, &layout, &record)?;
    }

    for (index, param) in decl.params.iter().enumerate() {
        let Some(Arg::ByRef(slot)) = args.get(index) else {
            continue;
        };
        if !param.by_ref {
            continue;
        }
        match &prepared[index] {
            Prepared::OutNumber(cell) => {
                let raw = **cell;
                let value = match param.ty {
                    VbType::Boolean => Value::Bool(raw as u32 != 0),
                    VbType::Byte => Value::Int(raw as u8 as i64),
                    VbType::Integer => Value::Int(raw as u16 as i16 as i64),
                    VbType::Long => Value::Int((raw as u32 as i32) as i64),
                    VbType::Double => Value::Float(f64::from_bits(raw)),
                    VbType::Date => Value::Date(f64::from_bits(raw)),
                    VbType::Single => Value::Float(f32::from_bits(raw as u32) as f64),
                    VbType::Currency => Value::Currency(raw as i64),
                    _ => Value::Int(raw as i64),
                };
                slot.set(value)?;
            }
            Prepared::OutText(buffer) => {
                slot.set(Value::Str(read_utf16(buffer)))?;
            }
            _ => {}
        }
    }

    if !decl.is_function {
        return Ok(Value::Empty);
    }
    match raw {
        Raw::F32(value) => Ok(Value::Float(value as f64)),
        Raw::F64(value) => Ok(if decl.ret == VbType::Date {
            Value::Date(value)
        } else {
            Value::Float(value)
        }),
        Raw::Int(bits) => convert_return(decl.ret, bits),
    }
}

fn prepare_by_value(value: &Value, ty: VbType) -> Result<Prepared, String> {
    let prepared = match ty {
        VbType::Str => {
            let mut buffer: Vec<u16> = value.to_display().encode_utf16().collect();
            buffer.push(0);
            Prepared::Text(buffer)
        }
        VbType::Single => Prepared::Single(value.as_float()? as f32),
        VbType::Double | VbType::Date => Prepared::Float(value.as_float()?),
        VbType::Currency => Prepared::Immediate(value.as_currency()? as usize),
        _ => Prepared::Immediate(value.as_int()? as usize),
    };
    Ok(prepared)
}

fn convert_return(ret: VbType, raw: usize) -> Result<Value, String> {
    Ok(match ret {
        VbType::Variant => Value::Int((raw as u32 as i32) as i64),
        VbType::Boolean => Value::Bool(raw as u32 != 0),
        VbType::Byte => Value::Int(raw as u8 as i64),
        VbType::Integer => Value::Int(raw as u16 as i16 as i64),
        VbType::Long => Value::Int((raw as u32 as i32) as i64),
        VbType::LongLong | VbType::LongPtr | VbType::Object | VbType::Any => Value::Int(raw as i64),
        VbType::Currency => Value::Currency(raw as i64),
        VbType::Str => Value::Str(unsafe { read_utf16_ptr(raw as *const u16) }),
        VbType::Single => Value::Float(f32::from_bits(raw as u32) as f64),
        VbType::Double => Value::Float(f64::from_bits(raw as u64)),
        VbType::Date => Value::Date(f64::from_bits(raw as u64)),
    })
}

// ---------------------------------------------------------------- 结构体

/// 单个结构体字段的数组元素上限。
const MAX_STRUCT_ARRAY: usize = 65_536;

/// UDT 在 Win32 调用中的内存布局。
struct StructLayout {
    size: usize,
    align: usize,
    fields: Vec<(usize, FieldKind)>,
}

enum FieldKind {
    Scalar(VbType),
    /// `String * n`：占 `2n` 字节的 UTF-16（本解释器只支持 W 版 API）。
    FixedStr(usize),
    /// 定长标量数组字段。
    Scalars {
        ty: VbType,
        lower: i32,
        count: usize,
    },
    Records {
        layout: Box<StructLayout>,
        lower: i32,
        count: usize,
    },
    Nested(Box<StructLayout>),
}

fn align_up(value: usize, align: usize) -> usize {
    if align <= 1 {
        value
    } else {
        value.div_ceil(align) * align
    }
}

fn scalar_size(ty: VbType) -> usize {
    match ty {
        VbType::Byte => 1,
        VbType::Integer => 2,
        VbType::Boolean | VbType::Long | VbType::Single => 4,
        _ => 8,
    }
}

/// `Get`/`Put` 写入的字节数；不支持的类型返回原因。
///
/// 与结构体保持一致：`Boolean` 也按 4 字节存（VB6 是 2 字节）。
pub fn binary_width(ty: VbType) -> Result<usize, String> {
    match ty {
        VbType::Byte
        | VbType::Integer
        | VbType::Boolean
        | VbType::Long
        | VbType::Single
        | VbType::Double
        | VbType::Date
        | VbType::Currency
        | VbType::LongLong
        | VbType::LongPtr => Ok(scalar_size(ty)),
        VbType::Str => Err("不支持 `String`（请改用 `Print #` / `Line Input #`）".into()),
        VbType::Variant | VbType::Object | VbType::Any => Err(format!(
            "不支持类型 {}（请声明为 Byte/Integer/Long/Single/Double/Date/Currency/Boolean）",
            ty.name()
        )),
    }
}

/// 把标量编码成 `Put #` 写入的字节。
pub fn encode_scalar(ty: VbType, value: &Value) -> Result<Vec<u8>, String> {
    let mut buffer = vec![0u8; binary_width(ty)?];
    write_scalar(&mut buffer, 0, ty, value)?;
    Ok(buffer)
}

/// 把 `Get #` 读到的字节解码成值。
pub fn decode_scalar(ty: VbType, bytes: &[u8]) -> Result<Value, String> {
    read_scalar(bytes, 0, ty)
}

/// UDT 在二进制文件里的字节数（`Len(x)`）。
pub fn record_size(name: &str, types: &HashMap<String, &TypeDef>) -> Result<usize, String> {
    Ok(struct_layout(name, types, 0)?.size)
}

/// 把 UDT 编码成 `Put #` 写入的字节。
pub fn encode_record(
    name: &str,
    record: &VbRecord,
    types: &HashMap<String, &TypeDef>,
) -> Result<Vec<u8>, String> {
    let layout = struct_layout(name, types, 0)?;
    let mut buffer = vec![0u8; layout.size];
    write_record(&mut buffer, &layout, record)?;
    Ok(buffer)
}

/// 把 `Get #` 读到的字节写回 UDT（就地修改）。
pub fn decode_record(
    name: &str,
    bytes: &[u8],
    record: &Rc<RefCell<VbRecord>>,
    types: &HashMap<String, &TypeDef>,
) -> Result<(), String> {
    let layout = struct_layout(name, types, 0)?;
    read_into_record(bytes, &layout, record)
}

fn scalar_layout(ty: VbType) -> Result<(usize, usize), String> {
    let size = match ty {
        VbType::Byte => 1,
        VbType::Integer => 2,
        VbType::Boolean | VbType::Long | VbType::Single => 4,
        VbType::Double | VbType::Date | VbType::Currency | VbType::LongLong | VbType::LongPtr => 8,
        VbType::Str => return Err("结构体字段暂不支持 `String`".into()),
        VbType::Variant | VbType::Object | VbType::Any => {
            return Err(format!("结构体字段不支持类型 {}", ty.name()));
        }
    };
    Ok((size, size))
}

fn struct_layout(
    name: &str,
    types: &HashMap<String, &TypeDef>,
    depth: usize,
) -> Result<StructLayout, String> {
    if depth > 8 {
        return Err(format!("结构体 `{name}` 嵌套过深"));
    }
    let def = types
        .get(name)
        .ok_or_else(|| format!("未定义的类型 `{name}`"))?;
    let mut fields = Vec::with_capacity(def.fields.len());
    let mut offset = 0usize;
    let mut align = 1usize;
    for field in &def.fields {
        let (kind, size, field_align) = if let Some((lower, upper)) = field.array_bounds {
            let count = (upper - lower + 1).max(0) as usize;
            if count > MAX_STRUCT_ARRAY {
                return Err(format!(
                    "结构体字段 `{}` 的数组过大（上限 {MAX_STRUCT_ARRAY} 个元素）",
                    field.name
                ));
            }
            let lower = i32::try_from(lower)
                .map_err(|_| format!("结构体字段 `{}` 的下界超出范围", field.name))?;
            match &field.record {
                Some(record_name) => {
                    let nested = struct_layout(record_name, types, depth + 1)?;
                    let size = nested
                        .size
                        .checked_mul(count)
                        .ok_or_else(|| format!("结构体字段 `{}` 的数组过大", field.name))?;
                    let elem_align = nested.align;
                    (
                        FieldKind::Records {
                            layout: Box::new(nested),
                            lower,
                            count,
                        },
                        size,
                        elem_align,
                    )
                }
                None => {
                    let (elem_size, elem_align) = scalar_layout(field.ty)?;
                    (
                        FieldKind::Scalars {
                            ty: field.ty,
                            lower,
                            count,
                        },
                        elem_size * count,
                        elem_align,
                    )
                }
            }
        } else if let Some(len) = field.fixed_len {
            (FieldKind::FixedStr(len), len * 2, 2)
        } else {
            match &field.record {
                Some(nested_name) => {
                    let nested = struct_layout(nested_name, types, depth + 1)?;
                    let (size, field_align) = (nested.size, nested.align);
                    (FieldKind::Nested(Box::new(nested)), size, field_align)
                }
                None => {
                    let (size, field_align) = scalar_layout(field.ty)?;
                    (FieldKind::Scalar(field.ty), size, field_align)
                }
            }
        };
        offset = align_up(offset, field_align);
        fields.push((offset, kind));
        offset += size;
        align = align.max(field_align);
    }
    Ok(StructLayout {
        size: align_up(offset.max(1), align),
        align,
        fields,
    })
}

fn write_record(buffer: &mut [u8], layout: &StructLayout, record: &VbRecord) -> Result<(), String> {
    for (index, (offset, kind)) in layout.fields.iter().enumerate() {
        let field = record
            .fields()
            .get(index)
            .ok_or_else(|| "结构体字段数量不匹配".to_string())?;
        match kind {
            FieldKind::Scalar(ty) => write_scalar(buffer, *offset, *ty, &field.value())?,
            FieldKind::FixedStr(len) => write_fixed_string(buffer, *offset, *len, &field.value())?,
            FieldKind::Scalars { ty, lower, count } => {
                let value = field.value();
                let array = value
                    .as_array()
                    .map_err(|_| format!("字段 `{}` 需要数组值", field.name))?;
                let array = array.borrow();
                for index in 0..*count {
                    let key = lower.saturating_add(index as i32);
                    let element = array.get(&[key])?;
                    write_scalar(buffer, offset + index * scalar_size(*ty), *ty, &element)?;
                }
            }
            FieldKind::Records {
                layout: nested,
                lower,
                count,
            } => {
                let value = field.value();
                let array = value
                    .as_array()
                    .map_err(|_| format!("字段 `{}` 需要记录数组", field.name))?;
                let array = array.borrow();
                for index in 0..*count {
                    let key = lower.saturating_add(index as i32);
                    let element = array
                        .get(&[key])?
                        .as_record()
                        .map_err(|_| format!("字段 `{}` 需要记录数组", field.name))?;
                    write_record(
                        &mut buffer[offset + index * nested.size..],
                        nested,
                        &element.borrow(),
                    )?;
                }
            }
            FieldKind::Nested(nested) => {
                let value = field.value();
                let nested_record = value
                    .as_record()
                    .map_err(|_| format!("字段 `{}` 需要记录值", field.name))?;
                write_record(buffer, nested, &nested_record.borrow())?;
            }
        }
    }
    Ok(())
}

fn write_scalar(buffer: &mut [u8], offset: usize, ty: VbType, value: &Value) -> Result<(), String> {
    let size = scalar_size(ty);
    let target = buffer
        .get_mut(offset..offset + size)
        .ok_or_else(|| "结构体缓冲区不足".to_string())?;
    match ty {
        VbType::Byte => target[0] = value.as_int()? as u8,
        VbType::Integer => target.copy_from_slice(&(value.as_int()? as i16).to_le_bytes()),
        VbType::Boolean => {
            let raw: i32 = if value.to_bool()? { -1 } else { 0 };
            target.copy_from_slice(&raw.to_le_bytes());
        }
        VbType::Long => target.copy_from_slice(&(value.as_int()? as i32).to_le_bytes()),
        VbType::Single => target.copy_from_slice(&(value.as_float()? as f32).to_le_bytes()),
        VbType::Double => target.copy_from_slice(&value.as_float()?.to_le_bytes()),
        VbType::Date => target.copy_from_slice(&value.as_float()?.to_le_bytes()),
        VbType::Currency => target.copy_from_slice(&value.as_currency()?.to_le_bytes()),
        VbType::LongLong | VbType::LongPtr => {
            target.copy_from_slice(&value.as_int()?.to_le_bytes())
        }
        _ => return Err(format!("结构体字段不支持类型 {}", ty.name())),
    }
    Ok(())
}

fn read_into_record(
    buffer: &[u8],
    layout: &StructLayout,
    record: &Rc<RefCell<VbRecord>>,
) -> Result<(), String> {
    for (index, (offset, kind)) in layout.fields.iter().enumerate() {
        let name = record
            .borrow()
            .fields()
            .get(index)
            .map(|field| field.name.clone())
            .ok_or_else(|| "结构体字段数量不匹配".to_string())?;
        match kind {
            FieldKind::Scalar(ty) => {
                let value = read_scalar(buffer, *offset, *ty)?;
                record.borrow_mut().set(&name, value)?;
            }
            FieldKind::FixedStr(len) => {
                let value = read_fixed_string(buffer, *offset, *len)?;
                record.borrow_mut().set(&name, value)?;
            }
            FieldKind::Scalars { ty, lower, count } => {
                let field_value = record
                    .borrow()
                    .get(&name)
                    .ok_or_else(|| format!("结构体缺少字段 `{name}`"))?;
                let array = field_value
                    .as_array()
                    .map_err(|_| format!("字段 `{name}` 需要数组值"))?;
                for index in 0..*count {
                    let key = lower.saturating_add(index as i32);
                    let value = read_scalar(buffer, offset + index * scalar_size(*ty), *ty)?;
                    array.borrow_mut().set(&[key], value)?;
                }
            }
            FieldKind::Records {
                layout: nested,
                lower,
                count,
            } => {
                let field_value = record
                    .borrow()
                    .get(&name)
                    .ok_or_else(|| format!("结构体缺少字段 `{name}`"))?;
                let array = field_value
                    .as_array()
                    .map_err(|_| format!("字段 `{name}` 需要记录数组"))?;
                for index in 0..*count {
                    let key = lower.saturating_add(index as i32);
                    let value = array
                        .borrow()
                        .get(&[key])?
                        .as_record()
                        .map_err(|_| format!("字段 `{name}` 需要记录数组"))?;
                    read_into_record(&buffer[offset + index * nested.size..], nested, &value)?;
                }
            }
            FieldKind::Nested(nested) => {
                let nested_record = record
                    .borrow()
                    .get(&name)
                    .ok_or_else(|| format!("结构体缺少字段 `{name}`"))?
                    .as_record()
                    .map_err(|_| format!("字段 `{name}` 需要记录值"))?;
                read_into_record(buffer, nested, &nested_record)?;
            }
        }
    }
    Ok(())
}

/// `String * n` 字段：写 `2n` 字节 UTF-16（定长字符串通常已被空格补齐）。
fn write_fixed_string(
    buffer: &mut [u8],
    offset: usize,
    len: usize,
    value: &Value,
) -> Result<(), String> {
    let target = buffer
        .get_mut(offset..offset + len * 2)
        .ok_or_else(|| "结构体缓冲区不足".to_string())?;
    let text = value.to_display();
    let mut units: Vec<u16> = text.encode_utf16().take(len).collect();
    units.resize(len, 0);
    for (index, unit) in units.iter().enumerate() {
        target[index * 2..index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

/// `String * n` 字段：读 `2n` 字节 UTF-16，去掉尾部 NUL 后按空格补足长度。
fn read_fixed_string(buffer: &[u8], offset: usize, len: usize) -> Result<Value, String> {
    let bytes = buffer
        .get(offset..offset + len * 2)
        .ok_or_else(|| "结构体缓冲区不足".to_string())?;
    let mut units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    while units.last() == Some(&0) {
        units.pop();
    }
    let text = String::from_utf16_lossy(&units);
    Ok(Value::Str(pad_fixed(&text, len)))
}

fn read_scalar(buffer: &[u8], offset: usize, ty: VbType) -> Result<Value, String> {
    let size = scalar_size(ty);
    let bytes = buffer
        .get(offset..offset + size)
        .ok_or_else(|| "结构体缓冲区不足".to_string())?;
    let array = |len: usize| -> Result<[u8; 8], String> {
        let mut slot = [0u8; 8];
        slot[..len].copy_from_slice(&bytes[..len]);
        Ok(slot)
    };
    Ok(match ty {
        VbType::Byte => Value::Int(bytes[0] as i64),
        VbType::Integer => {
            Value::Int(i16::from_le_bytes(array(2)?[..2].try_into().unwrap()) as i64)
        }
        VbType::Boolean => Value::Bool(i32::from_le_bytes(array(4)?[..4].try_into().unwrap()) != 0),
        VbType::Long => Value::Int(i32::from_le_bytes(array(4)?[..4].try_into().unwrap()) as i64),
        VbType::Single => {
            Value::Float(f32::from_le_bytes(array(4)?[..4].try_into().unwrap()) as f64)
        }
        VbType::Double => Value::Float(f64::from_le_bytes(array(8)?)),
        VbType::Date => Value::Date(f64::from_le_bytes(array(8)?)),
        VbType::Currency => Value::Currency(i64::from_le_bytes(array(8)?)),
        VbType::LongLong | VbType::LongPtr => Value::Int(i64::from_le_bytes(array(8)?)),
        _ => return Err(format!("结构体字段不支持类型 {}", ty.name())),
    })
}

fn read_utf16(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// 读取以 NUL 结尾的 UTF-16 字符串。只在内核返回合法指针时调用。
unsafe fn read_utf16_ptr(pointer: *const u16) -> String {
    if pointer.is_null() {
        return String::new();
    }
    const LIMIT: usize = 32 * 1024;
    let mut length = 0usize;
    while length < LIMIT && unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(pointer, length) };
    String::from_utf16_lossy(slice)
}

/// 读取原生 `LPCWSTR`：`AddressOf` 回调把 API 传进来的字符串还原成 VB 字符串时用。
pub unsafe fn read_wstring(pointer: *const u16) -> String {
    unsafe { read_utf16_ptr(pointer) }
}

/// 用与参数个数匹配的签名发起调用。
///
/// 安全前提：`address` 指向的函数确实按声明的参数个数接收整数/指针参数。
unsafe fn invoke(address: usize, args: &[usize]) -> usize {
    type F0 = unsafe extern "system" fn() -> usize;
    type F1 = unsafe extern "system" fn(usize) -> usize;
    type F2 = unsafe extern "system" fn(usize, usize) -> usize;
    type F3 = unsafe extern "system" fn(usize, usize, usize) -> usize;
    type F4 = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
    type F5 = unsafe extern "system" fn(usize, usize, usize, usize, usize) -> usize;
    type F6 = unsafe extern "system" fn(usize, usize, usize, usize, usize, usize) -> usize;
    type F7 = unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize) -> usize;
    type F8 =
        unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize;
    type F9 = unsafe extern "system" fn(
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) -> usize;
    type F10 = unsafe extern "system" fn(
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) -> usize;
    type F11 = unsafe extern "system" fn(
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) -> usize;
    type F12 = unsafe extern "system" fn(
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) -> usize;
    type F13 = unsafe extern "system" fn(
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) -> usize;
    type F14 = unsafe extern "system" fn(
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) -> usize;

    match args.len() {
        0 => {
            let function: F0 = unsafe { std::mem::transmute(address) };
            unsafe { function() }
        }
        1 => {
            let function: F1 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0]) }
        }
        2 => {
            let function: F2 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1]) }
        }
        3 => {
            let function: F3 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2]) }
        }
        4 => {
            let function: F4 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2], args[3]) }
        }
        5 => {
            let function: F5 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2], args[3], args[4]) }
        }
        6 => {
            let function: F6 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2], args[3], args[4], args[5]) }
        }
        7 => {
            let function: F7 = unsafe { std::mem::transmute(address) };
            unsafe {
                function(
                    args[0], args[1], args[2], args[3], args[4], args[5], args[6],
                )
            }
        }
        8 => {
            let function: F8 = unsafe { std::mem::transmute(address) };
            unsafe {
                function(
                    args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                )
            }
        }
        9 => {
            let function: F9 = unsafe { std::mem::transmute(address) };
            unsafe {
                function(
                    args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
                )
            }
        }
        10 => {
            let function: F10 = unsafe { std::mem::transmute(address) };
            unsafe {
                function(
                    args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    args[8], args[9],
                )
            }
        }
        11 => {
            let function: F11 = unsafe { std::mem::transmute(address) };
            unsafe {
                function(
                    args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    args[8], args[9], args[10],
                )
            }
        }
        12 => {
            let function: F12 = unsafe { std::mem::transmute(address) };
            unsafe {
                function(
                    args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    args[8], args[9], args[10], args[11],
                )
            }
        }
        13 => {
            let function: F13 = unsafe { std::mem::transmute(address) };
            unsafe {
                function(
                    args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    args[8], args[9], args[10], args[11], args[12],
                )
            }
        }
        14 => {
            let function: F14 = unsafe { std::mem::transmute(address) };
            unsafe {
                function(
                    args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                    args[8], args[9], args[10], args[11], args[12], args[13],
                )
            }
        }
        _ => 0,
    }
}

/// 整数参数、浮点返回值（`XMM0`）。
unsafe fn invoke_f64_return(address: usize, args: &[usize]) -> f64 {
    type F0 = unsafe extern "system" fn() -> f64;
    type F1 = unsafe extern "system" fn(usize) -> f64;
    type F2 = unsafe extern "system" fn(usize, usize) -> f64;
    type F3 = unsafe extern "system" fn(usize, usize, usize) -> f64;
    type F4 = unsafe extern "system" fn(usize, usize, usize, usize) -> f64;

    match args.len() {
        0 => {
            let function: F0 = unsafe { std::mem::transmute(address) };
            unsafe { function() }
        }
        1 => {
            let function: F1 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0]) }
        }
        2 => {
            let function: F2 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1]) }
        }
        3 => {
            let function: F3 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2]) }
        }
        _ => {
            let function: F4 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2], args[3]) }
        }
    }
}

/// 整数参数、`Single` 返回值。
unsafe fn invoke_f32_return(address: usize, args: &[usize]) -> f32 {
    type F0 = unsafe extern "system" fn() -> f32;
    type F1 = unsafe extern "system" fn(usize) -> f32;
    type F2 = unsafe extern "system" fn(usize, usize) -> f32;
    type F3 = unsafe extern "system" fn(usize, usize, usize) -> f32;
    type F4 = unsafe extern "system" fn(usize, usize, usize, usize) -> f32;

    match args.len() {
        0 => {
            let function: F0 = unsafe { std::mem::transmute(address) };
            unsafe { function() }
        }
        1 => {
            let function: F1 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0]) }
        }
        2 => {
            let function: F2 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1]) }
        }
        3 => {
            let function: F3 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2]) }
        }
        _ => {
            let function: F4 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2], args[3]) }
        }
    }
}

/// 全 `Double` 参数、整数返回值（`RAX`）。
unsafe fn invoke_float_args_int_return(address: usize, args: &[f64]) -> usize {
    type F1 = unsafe extern "system" fn(f64) -> usize;
    type F2 = unsafe extern "system" fn(f64, f64) -> usize;
    type F3 = unsafe extern "system" fn(f64, f64, f64) -> usize;
    type F4 = unsafe extern "system" fn(f64, f64, f64, f64) -> usize;

    match args.len() {
        1 => {
            let function: F1 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0]) }
        }
        2 => {
            let function: F2 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1]) }
        }
        3 => {
            let function: F3 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2]) }
        }
        _ => {
            let function: F4 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2], args[3]) }
        }
    }
}

/// 全 `Double` 参数、`Double` 返回值。
unsafe fn invoke_float_args_f64_return(address: usize, args: &[f64]) -> f64 {
    type F1 = unsafe extern "system" fn(f64) -> f64;
    type F2 = unsafe extern "system" fn(f64, f64) -> f64;
    type F3 = unsafe extern "system" fn(f64, f64, f64) -> f64;
    type F4 = unsafe extern "system" fn(f64, f64, f64, f64) -> f64;

    match args.len() {
        1 => {
            let function: F1 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0]) }
        }
        2 => {
            let function: F2 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1]) }
        }
        3 => {
            let function: F3 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2]) }
        }
        _ => {
            let function: F4 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2], args[3]) }
        }
    }
}

/// 全 `Double` 参数、`Single` 返回值。
unsafe fn invoke_float_args_f32_return(address: usize, args: &[f64]) -> f32 {
    type F1 = unsafe extern "system" fn(f64) -> f32;
    type F2 = unsafe extern "system" fn(f64, f64) -> f32;
    type F3 = unsafe extern "system" fn(f64, f64, f64) -> f32;
    type F4 = unsafe extern "system" fn(f64, f64, f64, f64) -> f32;

    match args.len() {
        1 => {
            let function: F1 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0]) }
        }
        2 => {
            let function: F2 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1]) }
        }
        3 => {
            let function: F3 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2]) }
        }
        _ => {
            let function: F4 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2], args[3]) }
        }
    }
}
/// 全 `Single` 参数、整数返回值。
unsafe fn invoke_single_args_int_return(address: usize, args: &[f32]) -> usize {
    type F0 = unsafe extern "system" fn() -> usize;
    type F1 = unsafe extern "system" fn(f32) -> usize;
    type F2 = unsafe extern "system" fn(f32, f32) -> usize;
    type F3 = unsafe extern "system" fn(f32, f32, f32) -> usize;
    type F4 = unsafe extern "system" fn(f32, f32, f32, f32) -> usize;

    match args.len() {
        0 => {
            let function: F0 = unsafe { std::mem::transmute(address) };
            unsafe { function() }
        }
        1 => {
            let function: F1 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0]) }
        }
        2 => {
            let function: F2 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1]) }
        }
        3 => {
            let function: F3 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2]) }
        }
        _ => {
            let function: F4 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2], args[3]) }
        }
    }
}

/// 全 `Single` 参数、`Double` 返回值。
unsafe fn invoke_single_args_f64_return(address: usize, args: &[f32]) -> f64 {
    type F0 = unsafe extern "system" fn() -> f64;
    type F1 = unsafe extern "system" fn(f32) -> f64;
    type F2 = unsafe extern "system" fn(f32, f32) -> f64;
    type F3 = unsafe extern "system" fn(f32, f32, f32) -> f64;
    type F4 = unsafe extern "system" fn(f32, f32, f32, f32) -> f64;

    match args.len() {
        0 => {
            let function: F0 = unsafe { std::mem::transmute(address) };
            unsafe { function() }
        }
        1 => {
            let function: F1 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0]) }
        }
        2 => {
            let function: F2 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1]) }
        }
        3 => {
            let function: F3 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2]) }
        }
        _ => {
            let function: F4 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2], args[3]) }
        }
    }
}

/// 全 `Single` 参数、`Single` 返回值。
unsafe fn invoke_single_args_f32_return(address: usize, args: &[f32]) -> f32 {
    type F0 = unsafe extern "system" fn() -> f32;
    type F1 = unsafe extern "system" fn(f32) -> f32;
    type F2 = unsafe extern "system" fn(f32, f32) -> f32;
    type F3 = unsafe extern "system" fn(f32, f32, f32) -> f32;
    type F4 = unsafe extern "system" fn(f32, f32, f32, f32) -> f32;

    match args.len() {
        0 => {
            let function: F0 = unsafe { std::mem::transmute(address) };
            unsafe { function() }
        }
        1 => {
            let function: F1 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0]) }
        }
        2 => {
            let function: F2 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1]) }
        }
        3 => {
            let function: F3 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2]) }
        }
        _ => {
            let function: F4 = unsafe { std::mem::transmute(address) };
            unsafe { function(args[0], args[1], args[2], args[3]) }
        }
    }
}

/// 混合调用里每个参数对应的 Rust 类型。
macro_rules! mixed_ty {
    (int) => {
        usize
    };
    (float) => {
        f64
    };
}

/// 按位置取出下一个实参。
macro_rules! mixed_next {
    (int, $cursor:ident) => {
        match $cursor.next() {
            Some(MixedArg::Int(value)) => *value,
            _ => return Err("参数类型与调用约定不匹配".to_string()),
        }
    };
    (float, $cursor:ident) => {
        match $cursor.next() {
            Some(MixedArg::Float(value)) => *value,
            _ => return Err("参数类型与调用约定不匹配".to_string()),
        }
    };
}

/// 用给定签名发起一次混合调用。
macro_rules! mixed_call {
    ($ret:ty, $address:expr, $args:expr, $($kind:ident),+) => {{
        let mut cursor = $args.iter();
        type Call = unsafe extern "system" fn($(mixed_ty!($kind)),+) -> $ret;
        let function: Call = unsafe { std::mem::transmute($address) };
        unsafe { function($(mixed_next!($kind, cursor)),+) }
    }};
}

/// 参数里哪些位置是浮点（第 i 位对应第 i 个参数）。
fn kind_mask(args: &[MixedArg]) -> u32 {
    let mut mask = 0u32;
    for (index, item) in args.iter().enumerate() {
        if matches!(item, MixedArg::Float(_)) {
            mask |= 1 << index;
        }
    }
    mask
}

/// 按「参数个数 + 浮点位置」挑签名；最多 4 个参数。
///
/// 位掩码的第 i 位对应第 i 个参数（1 = 浮点）。
macro_rules! mixed_dispatch {
    ($ret:ty, $address:expr, $args:expr) => {{
        match ($args.len(), kind_mask($args)) {
            (1, 0b0) => mixed_call!($ret, $address, $args, int),
            (1, 0b1) => mixed_call!($ret, $address, $args, float),
            (2, 0b00) => mixed_call!($ret, $address, $args, int, int),
            (2, 0b01) => mixed_call!($ret, $address, $args, float, int),
            (2, 0b10) => mixed_call!($ret, $address, $args, int, float),
            (2, 0b11) => mixed_call!($ret, $address, $args, float, float),
            (3, 0b000) => mixed_call!($ret, $address, $args, int, int, int),
            (3, 0b001) => mixed_call!($ret, $address, $args, float, int, int),
            (3, 0b010) => mixed_call!($ret, $address, $args, int, float, int),
            (3, 0b011) => mixed_call!($ret, $address, $args, float, float, int),
            (3, 0b100) => mixed_call!($ret, $address, $args, int, int, float),
            (3, 0b101) => mixed_call!($ret, $address, $args, float, int, float),
            (3, 0b110) => mixed_call!($ret, $address, $args, int, float, float),
            (3, 0b111) => mixed_call!($ret, $address, $args, float, float, float),
            (4, 0b0000) => mixed_call!($ret, $address, $args, int, int, int, int),
            (4, 0b0001) => mixed_call!($ret, $address, $args, float, int, int, int),
            (4, 0b0010) => mixed_call!($ret, $address, $args, int, float, int, int),
            (4, 0b0011) => mixed_call!($ret, $address, $args, float, float, int, int),
            (4, 0b0100) => mixed_call!($ret, $address, $args, int, int, float, int),
            (4, 0b0101) => mixed_call!($ret, $address, $args, float, int, float, int),
            (4, 0b0110) => mixed_call!($ret, $address, $args, int, float, float, int),
            (4, 0b0111) => mixed_call!($ret, $address, $args, float, float, float, int),
            (4, 0b1000) => mixed_call!($ret, $address, $args, int, int, int, float),
            (4, 0b1001) => mixed_call!($ret, $address, $args, float, int, int, float),
            (4, 0b1010) => mixed_call!($ret, $address, $args, int, float, int, float),
            (4, 0b1011) => mixed_call!($ret, $address, $args, float, float, int, float),
            (4, 0b1100) => mixed_call!($ret, $address, $args, int, int, float, float),
            (4, 0b1101) => mixed_call!($ret, $address, $args, float, int, float, float),
            (4, 0b1110) => mixed_call!($ret, $address, $args, int, float, float, float),
            (4, 0b1111) => mixed_call!($ret, $address, $args, float, float, float, float),
            (count, _) => return Err(format!("暂不支持 {count} 个参数的混合整型/浮点调用")),
        }
    }};
}

/// 整数与 `Double` 混合参数：按返回类型挑签名。
fn dispatch_mixed(address: usize, args: &[MixedArg], ret: VbType) -> Result<Raw, String> {
    Ok(match ret {
        VbType::Double | VbType::Date => Raw::F64(mixed_dispatch!(f64, address, args)),
        VbType::Single => Raw::F32(mixed_dispatch!(f32, address, args)),
        _ => Raw::Int(mixed_dispatch!(usize, address, args)),
    })
}

fn wide(text: &str) -> Vec<u16> {
    let mut buffer: Vec<u16> = text.encode_utf16().collect();
    buffer.push(0);
    buffer
}

fn narrow(text: &str) -> Vec<u8> {
    let mut buffer: Vec<u8> = text.bytes().collect();
    buffer.push(0);
    buffer
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn LoadLibraryW(name: *const u16) -> *mut c_void;
        pub fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        pub fn GetTickCount() -> u32;
        pub fn Sleep(milliseconds: u32);
        pub fn GetLocalTime(time: *mut super::RawSystemTime);
        pub fn GetCurrentProcessId() -> u32;
        pub fn CreateProcessW(
            application: *const u16,
            command_line: *mut u16,
            process_attributes: *mut c_void,
            thread_attributes: *mut c_void,
            inherit_handles: i32,
            creation_flags: u32,
            environment: *mut c_void,
            current_directory: *const u16,
            startup_info: *mut super::platform::StartupInfoW,
            process_info: *mut super::platform::ProcessInfo,
        ) -> i32;
        pub fn CloseHandle(handle: *mut c_void) -> i32;
    }

    /// `STARTUPINFOW`（字段顺序与平台结构体一致）。
    #[repr(C)]
    #[derive(Default)]
    pub struct StartupInfoW {
        pub cb: u32,
        pub reserved: *mut u16,
        pub desktop: *mut u16,
        pub title: *mut u16,
        pub x: u32,
        pub y: u32,
        pub x_size: u32,
        pub y_size: u32,
        pub x_count_chars: u32,
        pub y_count_chars: u32,
        pub fill_attribute: u32,
        pub flags: u32,
        pub show_window: u16,
        pub reserved2_size: u16,
        pub reserved2: *mut u8,
        pub std_input: *mut c_void,
        pub std_output: *mut c_void,
        pub std_error: *mut c_void,
    }

    /// `PROCESS_INFORMATION`。
    #[repr(C)]
    #[derive(Default)]
    pub struct ProcessInfo {
        pub process: *mut c_void,
        pub thread: *mut c_void,
        pub process_id: u32,
        pub thread_id: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn EnumWindows(procedure: Option<EnumProc>, data: isize) -> i32;
        pub fn GetWindowThreadProcessId(window: *mut c_void, process: *mut u32) -> u32;
        pub fn GetClassNameW(window: *mut c_void, name: *mut u16, max: i32) -> i32;
        pub fn PostMessageW(window: *mut c_void, message: u32, wparam: usize, lparam: isize)
        -> i32;
    }

    pub type EnumProc = unsafe extern "system" fn(*mut c_void, isize) -> i32;

    /// 关掉本进程里还开着的对话框：脚本线程会一直等对话框，
    /// 「停止脚本」只能靠 `WM_CLOSE` 把它叫醒。
    pub unsafe fn close_dialogs() {
        let process = unsafe { GetCurrentProcessId() };
        unsafe { EnumWindows(Some(close_dialog), process as isize) };
    }

    unsafe extern "system" fn close_dialog(window: *mut c_void, data: isize) -> i32 {
        const WM_CLOSE: u32 = 0x0010;

        let mut process = 0u32;
        unsafe { GetWindowThreadProcessId(window, &mut process) };
        if process as isize != data {
            return 1;
        }
        let mut name = [0u16; 32];
        let length = unsafe { GetClassNameW(window, name.as_mut_ptr(), name.len() as i32) };
        let dialog_class: Vec<u16> = "#32770".encode_utf16().collect();
        if name.get(..length.max(0) as usize) != Some(dialog_class.as_slice()) {
            return 1;
        }
        unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
        1
    }

    /// 启动一个程序；返回新进程的 id（VB 的 task id）。
    pub unsafe fn shell(command: &str, window_style: i32) -> Result<u32, String> {
        const STARTF_USESHOWWINDOW: u32 = 0x0000_0001;

        let mut line = super::wide(command);
        let mut startup = StartupInfoW {
            cb: std::mem::size_of::<StartupInfoW>() as u32,
            show_window: window_style.clamp(0, 10) as u16,
            flags: STARTF_USESHOWWINDOW,
            ..StartupInfoW::default()
        };
        let mut info = ProcessInfo::default();

        let started = unsafe {
            CreateProcessW(
                std::ptr::null(),
                line.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null(),
                &mut startup,
                &mut info,
            )
        };
        if started == 0 {
            let error = std::io::Error::last_os_error();
            return Err(format!("无法启动 `{command}`：{error}"));
        }
        unsafe {
            CloseHandle(info.process);
            CloseHandle(info.thread);
        }
        Ok(info.process_id)
    }
}

/// 解析 `库名!导出名` 的地址（带缓存）。
fn resolve(lib: &str, symbol: &str) -> Result<usize, String> {
    let cache_key = format!("{lib}!{}", symbol.to_ascii_lowercase());
    if let Some(address) = SYMBOLS.with(|cache| cache.borrow().get(&cache_key).copied()) {
        return Ok(address);
    }

    let module = module_handle(lib)?;
    #[cfg(windows)]
    {
        let name = narrow(symbol);
        let address = unsafe { platform::GetProcAddress(module as *mut c_void, name.as_ptr()) };
        if address.is_null() {
            return Err(format!("库 `{lib}` 里找不到导出函数 `{symbol}`"));
        }
        let address = address as usize;
        SYMBOLS.with(|cache| cache.borrow_mut().insert(cache_key, address));
        Ok(address)
    }
    #[cfg(not(windows))]
    {
        let _ = module;
        Err("当前平台不支持动态调用 Win32 API".into())
    }
}

fn module_handle(lib: &str) -> Result<usize, String> {
    let key = lib.to_ascii_lowercase();
    if let Some(handle) = MODULES.with(|cache| cache.borrow().get(&key).copied()) {
        return Ok(handle);
    }
    #[cfg(windows)]
    {
        let file = if lib.to_ascii_lowercase().ends_with(".dll") {
            lib.to_string()
        } else {
            format!("{lib}.dll")
        };
        let name = wide(&file);
        let handle = unsafe { platform::LoadLibraryW(name.as_ptr()) };
        if handle.is_null() {
            return Err(format!("无法加载库 `{file}`"));
        }
        let handle = handle as usize;
        MODULES.with(|cache| cache.borrow_mut().insert(key, handle));
        Ok(handle)
    }
    #[cfg(not(windows))]
    {
        let _ = key;
        Err("当前平台不支持动态调用 Win32 API".into())
    }
}

/// 启动一个程序（`Shell`）：返回新进程 id；失败时给出系统错误。
pub fn shell(command: &str, window_style: i32) -> Result<i64, String> {
    #[cfg(windows)]
    {
        let id = unsafe { platform::shell(command, window_style)? };
        Ok(i64::from(id))
    }
    #[cfg(not(windows))]
    {
        let _ = (command, window_style);
        Err("当前平台不支持 Shell".into())
    }
}

/// 关掉本进程里还开着的对话框。
///
/// 脚本线程会一直等对话框，所以「停止脚本」时要先把它们关掉，否则线程停不下来。
pub fn close_dialogs() {
    #[cfg(windows)]
    unsafe {
        platform::close_dialogs();
    }
}

/// `Sleep` 的实现。
pub fn sleep_ms(milliseconds: u32) {
    #[cfg(windows)]
    unsafe {
        platform::Sleep(milliseconds);
    }
    #[cfg(not(windows))]
    let _ = milliseconds;
}

/// 系统启动以来的毫秒数。
pub fn tick_count() -> u32 {
    #[cfg(windows)]
    {
        unsafe { platform::GetTickCount() }
    }
    #[cfg(not(windows))]
    {
        0
    }
}
