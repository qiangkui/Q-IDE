//! `AddressOf` 跳板：把 Win32 的回调转回解释器里的 VB 过程调用。
//! 回调上下文、错误暂存、`ByRef`/`ByVal` 实参与返回值的读写都在这里；
//! 跳板地址由 [`PersistentCallback`] 保管到脚本结束。

use super::*;

/// 一次 `Declare` 调用里的回调上下文；`AddressOf` 跳板通过它找回解释器和 VB 过程。
pub(super) struct CallbackCtx {
    /// `Runtime<'p, 'o>` 擦掉生命周期后的指针，只在本次调用期间有效。
    pub(super) runtime: *mut (),
    /// 防止旧回调遇到栈地址复用时误入后续的运行时。
    pub(super) generation: u64,
    /// 要调用的 VB 过程（AST 活得比运行时久）。
    pub(super) procedure: *const Procedure,
    /// 回调里发生的错误：跨不过 FFI 边界，先存下来等调用结束再报。
    pub(super) error: RefCell<Option<RuntimeError>>,
    /// String 回调结果须活到调用方复制完返回值。
    pub(super) strings: RefCell<Vec<Box<[u16]>>>,
}

impl CallbackCtx {
    pub(super) fn new(runtime: *mut (), generation: u64, procedure: &Procedure) -> Self {
        Self {
            runtime,
            generation,
            procedure: procedure as *const Procedure,
            error: RefCell::new(None),
            strings: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn fail(&self, error: RuntimeError) {
        let mut slot = self.error.borrow_mut();
        if slot.is_none() {
            *slot = Some(error);
        }
    }
}

pub(super) fn callback_runtime_is_active(context: &CallbackCtx) -> bool {
    ACTIVE_RUNTIME.with(|slot| {
        let active = slot.get();
        runtime_identity_matches(
            active.pointer,
            active.generation,
            context.runtime,
            context.generation,
        )
    })
}

pub(crate) fn runtime_identity_matches(
    active_pointer: *mut (),
    active_generation: u64,
    callback_pointer: *mut (),
    callback_generation: u64,
) -> bool {
    active_pointer == callback_pointer && active_generation == callback_generation
}

/// 一个在整次脚本运行期间都有效的回调：API 可能把指针存下来，之后再调用。
///
/// 脚本结束（包括出错/停止）时窗口可能还没销毁，系统之后仍可能回调：
/// 这两个字段会在 [`Runtime`] 的 `Drop` 里泄漏掉，回调入口再检查运行时是否还活着。
pub(super) struct PersistentCallback {
    pub(super) name: String,
    pub(super) context: Option<Box<CallbackCtx>>,
    pub(super) trampoline: Option<Trampoline>,
}

/// 跳板转发入口：`RCX` 是上下文，`RDX` 指向原始 GP/XMM 参数快照。
type CallbackThunk = unsafe extern "system" fn(
    *mut CallbackCtx,
    *const crate::interpreter::trampoline::CallbackRegisters,
) -> usize;
type CallbackSingleThunk = unsafe extern "system" fn(
    *mut CallbackCtx,
    *const crate::interpreter::trampoline::CallbackRegisters,
) -> f32;
type CallbackDoubleThunk = unsafe extern "system" fn(
    *mut CallbackCtx,
    *const crate::interpreter::trampoline::CallbackRegisters,
) -> f64;
type CallbackStringThunk = unsafe extern "system" fn(
    *mut CallbackCtx,
    *const crate::interpreter::trampoline::CallbackRegisters,
) -> *const u16;

pub(super) fn callback_thunk_address(ret: VbType) -> usize {
    match ret {
        VbType::Single => callback_single_thunk as CallbackSingleThunk as usize,
        VbType::Double | VbType::Date => callback_double_thunk as CallbackDoubleThunk as usize,
        VbType::Str => callback_string_thunk as CallbackStringThunk as usize,
        _ => callback_thunk as CallbackThunk as usize,
    }
}

/// 机器码跳板的转发入口：参数快照按过程形参类型解码。
unsafe extern "system" fn callback_thunk(
    ctx: *mut CallbackCtx,
    registers: *const crate::interpreter::trampoline::CallbackRegisters,
) -> usize {
    if ctx.is_null() {
        return 0;
    }
    // SAFETY: 跳板由 `Trampoline` 生成，传入的就是 `call_declare` 栈上或持久回调表里的上下文。
    let context = unsafe { &*ctx };
    // 运行时已经结束（窗口可能还活着）：不再进入解释器，直接返回 0。
    if !callback_runtime_is_active(context) {
        return 0;
    }
    // 回调里 panic 会穿过 extern "system" 直接终止进程，这里先兜住。
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dispatch_callback(context, registers)
    })) {
        Ok(Ok(value)) => match if let Value::Currency(_) = value {
            value.as_currency()
        } else {
            value.as_int()
        } {
            Ok(value) => value as usize,
            Err(message) => {
                context.fail(RuntimeError::new(
                    format!("回调返回值不可转换为整数：{message}"),
                    0,
                ));
                0
            }
        },
        Ok(Err(error)) => {
            context.fail(error);
            0
        }
        Err(_) => {
            context.fail(RuntimeError::new("回调内部出错（解释器异常）", 0));
            0
        }
    }
}

unsafe extern "system" fn callback_single_thunk(
    ctx: *mut CallbackCtx,
    registers: *const crate::interpreter::trampoline::CallbackRegisters,
) -> f32 {
    let Some(context) = (unsafe { ctx.as_ref() }) else {
        return 0.0;
    };
    if !callback_runtime_is_active(context) {
        return 0.0;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dispatch_callback(context, registers)
    })) {
        Ok(Ok(value)) => match value.as_float() {
            Ok(value) => value as f32,
            Err(message) => {
                context.fail(RuntimeError::new(
                    format!("回调返回值不可转换为 Single：{message}"),
                    0,
                ));
                0.0
            }
        },
        Ok(Err(error)) => {
            context.fail(error);
            0.0
        }
        Err(_) => {
            context.fail(RuntimeError::new("回调内部出错（解释器异常）", 0));
            0.0
        }
    }
}

unsafe extern "system" fn callback_double_thunk(
    ctx: *mut CallbackCtx,
    registers: *const crate::interpreter::trampoline::CallbackRegisters,
) -> f64 {
    let Some(context) = (unsafe { ctx.as_ref() }) else {
        return 0.0;
    };
    if !callback_runtime_is_active(context) {
        return 0.0;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dispatch_callback(context, registers)
    })) {
        Ok(Ok(value)) => match value.as_float() {
            Ok(value) => value,
            Err(message) => {
                context.fail(RuntimeError::new(
                    format!("回调返回值不可转换为 Double：{message}"),
                    0,
                ));
                0.0
            }
        },
        Ok(Err(error)) => {
            context.fail(error);
            0.0
        }
        Err(_) => {
            context.fail(RuntimeError::new("回调内部出错（解释器异常）", 0));
            0.0
        }
    }
}

unsafe extern "system" fn callback_string_thunk(
    ctx: *mut CallbackCtx,
    registers: *const crate::interpreter::trampoline::CallbackRegisters,
) -> *const u16 {
    let Some(context) = (unsafe { ctx.as_ref() }) else {
        return std::ptr::null();
    };
    if !callback_runtime_is_active(context) {
        return std::ptr::null();
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dispatch_callback(context, registers)
    })) {
        Ok(Ok(value)) => {
            let mut text: Vec<u16> = value.to_display().encode_utf16().collect();
            text.push(0);
            let text = text.into_boxed_slice();
            let pointer = text.as_ptr();
            context.strings.borrow_mut().push(text);
            pointer
        }
        Ok(Err(error)) => {
            context.fail(error);
            std::ptr::null()
        }
        Err(_) => {
            context.fail(RuntimeError::new("回调内部出错（解释器异常）", 0));
            std::ptr::null()
        }
    }
}

/// 把寄存器里的实参还原成 VB 值并执行过程。
unsafe fn dispatch_callback(
    context: &CallbackCtx,
    registers: *const crate::interpreter::trampoline::CallbackRegisters,
) -> Result<Value, RuntimeError> {
    if registers.is_null() {
        return Err(RuntimeError::new("回调缺少参数寄存器", 0));
    }
    let registers = unsafe { &*registers };
    // SAFETY: 上下文里的指针在 `ffi::call` 期间一直有效；`call_declare` 在调用外部函数时
    // 不持有 `self` 的任何引用（类型表是提前克隆的），所以这里重新借出 `&mut` 不会
    // 和数据里的其它引用重叠。
    let runtime = unsafe { &mut *(context.runtime as *mut Runtime<'static, 'static>) };
    let procedure = unsafe { &*context.procedure };
    let mut values = Vec::with_capacity(procedure.params.len());
    let mut record_args = Vec::new();
    for (index, param) in procedure.params.iter().enumerate() {
        let raw = if !param.by_ref
            && matches!(param.ty, VbType::Single | VbType::Double | VbType::Date)
        {
            registers.float[index] as usize
        } else {
            registers.integer[index]
        };
        if let Some(record_name) = &param.record {
            if !param.by_ref {
                return Err(RuntimeError::new(
                    format!(
                        "回调 `{}` 的 UDT 形参 `{}` 必须声明为 ByRef",
                        procedure.name, param.name
                    ),
                    procedure.line,
                ));
            }
            if raw == 0 {
                return Err(RuntimeError::new(
                    format!("回调 `{}` 的记录指针为空", procedure.name),
                    procedure.line,
                ));
            }
            let size = ffi::record_size(record_name, &runtime.types)
                .map_err(|message| RuntimeError::new(message, procedure.line))?;
            let value = runtime.new_record(record_name, procedure.line)?;
            let record = value
                .as_record()
                .map_err(|message| RuntimeError::new(message, procedure.line))?;
            let bytes = unsafe { std::slice::from_raw_parts(raw as *const u8, size) };
            ffi::decode_record(record_name, bytes, &record, &runtime.types)
                .map_err(|message| RuntimeError::new(message, procedure.line))?;
            record_args.push((index, raw as *mut u8, record_name.clone()));
            values.push(value);
            continue;
        }
        match callback_arg(param, raw) {
            Ok(value) => values.push(value),
            Err(message) => {
                return Err(RuntimeError::new(
                    format!("回调 `{}`：{message}", procedure.name),
                    procedure.line,
                ));
            }
        }
    }
    let (result, params) = runtime.call_callback_with_params(procedure, values)?;
    for (index, pointer, type_name) in record_args {
        let value = params
            .get(index)
            .ok_or_else(|| RuntimeError::new("回调记录参数丢失", procedure.line))?;
        let record = value
            .as_record()
            .map_err(|message| RuntimeError::new(message, procedure.line))?;
        let bytes = ffi::encode_record(&type_name, &record.borrow(), &runtime.types)
            .map_err(|message| RuntimeError::new(message, procedure.line))?;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len());
        }
    }
    for (index, param) in procedure.params.iter().enumerate() {
        if !param.by_ref || param.record.is_some() {
            continue;
        }
        let value = params
            .get(index)
            .ok_or_else(|| RuntimeError::new("回调参数丢失", procedure.line))?;
        write_callback_arg(param, registers.integer[index], value, context)
            .map_err(|message| RuntimeError::new(message, procedure.line))?;
    }
    Ok(result)
}

pub(super) fn write_callback_arg(
    param: &Param,
    raw: usize,
    value: &Value,
    context: &CallbackCtx,
) -> Result<(), String> {
    if raw == 0 {
        return Err(format!("ByRef 参数 `{}` 的指针为空", param.name));
    }
    if param.ty == VbType::Str {
        let mut text: Vec<u16> = value.to_display().encode_utf16().collect();
        text.push(0);
        let text = text.into_boxed_slice();
        let pointer = text.as_ptr();
        context.strings.borrow_mut().push(text);
        unsafe {
            (raw as *mut *const u16).write(pointer);
        }
        return Ok(());
    }
    let value = value.coerce(param.ty)?;
    unsafe {
        match param.ty {
            VbType::Byte => (raw as *mut u8).write(value.as_int()? as u8),
            VbType::Integer => (raw as *mut i16).write(value.as_int()? as i16),
            VbType::Boolean => {
                (raw as *mut i32).write(if value.to_bool()? { -1 } else { 0 });
            }
            VbType::Long => (raw as *mut i32).write(value.as_int()? as i32),
            VbType::Single => (raw as *mut f32).write(value.as_float()? as f32),
            VbType::Double => (raw as *mut f64).write(value.as_float()?),
            VbType::Date => (raw as *mut f64).write(value.as_float()?),
            VbType::Currency => (raw as *mut i64).write(value.as_currency()?),
            _ => (raw as *mut i64).write(value.as_int()?),
        }
    }
    Ok(())
}

/// 单个回调实参：`ByRef` 时寄存器里是原生指针，`ByVal` 时就是值本身。
pub(super) fn callback_arg(param: &Param, raw: usize) -> Result<Value, String> {
    if param.record.is_some() {
        return Err("暂不支持记录类型形参".into());
    }
    if param.is_array || param.param_array {
        return Err("暂不支持数组形参".into());
    }
    if matches!(param.ty, VbType::Str) {
        // `ByVal s As String` 是 `LPCWSTR`，`ByRef s As String` 是 `LPWSTR *`。
        let pointer = if param.by_ref {
            unsafe { *(raw as *const *const u16) }
        } else {
            raw as *const u16
        };
        return Ok(Value::Str(unsafe { ffi::read_wstring(pointer) }));
    }
    Ok(if param.by_ref {
        scalar_from_pointer(raw, param.ty)
    } else {
        scalar_from_register(raw, param.ty)
    })
}

/// `ByRef` 形参：按声明类型从原生指针读值。
pub(super) fn scalar_from_pointer(raw: usize, ty: VbType) -> Value {
    unsafe {
        match ty {
            VbType::Byte => Value::Int(i64::from(*(raw as *const u8))),
            VbType::Integer => Value::Int(i64::from(*(raw as *const i16))),
            VbType::Boolean => Value::Bool(*(raw as *const i32) != 0),
            VbType::Long => Value::Int(i64::from(*(raw as *const i32))),
            VbType::Currency => Value::Currency(*(raw as *const i64)),
            VbType::Single => Value::Float(f64::from(*(raw as *const f32))),
            VbType::Double => Value::Float(*(raw as *const f64)),
            VbType::Date => Value::Date(*(raw as *const f64)),
            _ => Value::Int(*(raw as *const i64)),
        }
    }
}

/// `ByVal` 形参：寄存器里的值按声明类型截断。
pub(super) fn scalar_from_register(raw: usize, ty: VbType) -> Value {
    match ty {
        VbType::Byte => Value::Int(i64::from(raw as u8)),
        VbType::Integer => Value::Int(i64::from(raw as u16 as i16)),
        VbType::Boolean => Value::Bool(raw as u32 != 0),
        VbType::Long => Value::Int(i64::from(raw as u32 as i32)),
        VbType::Currency => Value::Currency(raw as i64),
        VbType::Single => Value::Float(f64::from(f32::from_bits(raw as u32))),
        VbType::Double => Value::Float(f64::from_bits(raw as u64)),
        VbType::Date => Value::Date(f64::from_bits(raw as u64)),
        _ => Value::Int(raw as i64),
    }
}
