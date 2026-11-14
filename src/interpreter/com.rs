//! Windows COM Automation 桥接，支持 VB 的 `CreateObject` / `GetObject`。

use crate::interpreter::value::Value;

#[cfg(windows)]
mod windows {
    use super::Value;
    use std::ffi::c_void;
    use std::ptr::{self, NonNull};

    type HResult = i32;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const IID_NULL: Guid = Guid {
        data1: 0,
        data2: 0,
        data3: 0,
        data4: [0; 8],
    };
    const IID_IDISPATCH: Guid = Guid {
        data1: 0x0002_0400,
        data2: 0,
        data3: 0,
        data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
    };
    const CLSCTX_ALL: u32 = 0x17;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const STGM_READWRITE: u32 = 0x2;
    const DISPATCH_METHOD: u16 = 0x1;
    const DISPATCH_PROPERTYGET: u16 = 0x2;
    const DISPATCH_PROPERTYPUT: u16 = 0x4;
    const DISPATCH_PROPERTYPUTREF: u16 = 0x8;
    const DISPID_PROPERTYPUT: i32 = -3;
    const LOCALE_USER_DEFAULT: u32 = 0x0400;
    const VT_VARIANT: u16 = 12;
    const VT_ARRAY: u16 = 0x2000;
    const VT_BYREF: u16 = 0x4000;

    #[repr(C)]
    struct SafeArray {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct IDispatch {
        vtable: *const IDispatchVTable,
    }

    #[repr(C)]
    struct IUnknown {
        vtable: *const IUnknownVTable,
    }

    #[repr(C)]
    struct IUnknownVTable {
        query_interface:
            unsafe extern "system" fn(*mut IUnknown, *const Guid, *mut *mut c_void) -> HResult,
        add_ref: unsafe extern "system" fn(*mut IUnknown) -> u32,
        release: unsafe extern "system" fn(*mut IUnknown) -> u32,
    }

    #[repr(C)]
    struct IDispatchVTable {
        query_interface:
            unsafe extern "system" fn(*mut IDispatch, *const Guid, *mut *mut c_void) -> HResult,
        add_ref: unsafe extern "system" fn(*mut IDispatch) -> u32,
        release: unsafe extern "system" fn(*mut IDispatch) -> u32,
        get_type_info_count: unsafe extern "system" fn(*mut IDispatch, *mut u32) -> HResult,
        get_type_info:
            unsafe extern "system" fn(*mut IDispatch, u32, u32, *mut *mut c_void) -> HResult,
        get_ids_of_names: unsafe extern "system" fn(
            *mut IDispatch,
            *const Guid,
            *mut *mut u16,
            u32,
            u32,
            *mut i32,
        ) -> HResult,
        invoke: unsafe extern "system" fn(
            *mut IDispatch,
            i32,
            *const Guid,
            u32,
            u16,
            *mut DispParams,
            *mut Variant,
            *mut ExcepInfo,
            *mut u32,
        ) -> HResult,
    }

    #[repr(C)]
    struct DispParams {
        args: *mut Variant,
        named_args: *mut i32,
        arg_count: u32,
        named_arg_count: u32,
    }

    #[repr(C)]
    struct MultiQi {
        iid: *const Guid,
        interface: *mut IUnknown,
        status: HResult,
    }

    #[repr(C)]
    struct ExcepInfo {
        code: u16,
        reserved: u16,
        source: *mut u16,
        description: *mut u16,
        help_file: *mut u16,
        help_context: u32,
        reserved_ptr: *mut c_void,
        deferred_fill: *mut c_void,
        scode: HResult,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct VariantRecord {
        record: *mut c_void,
        info: *mut c_void,
    }

    #[repr(C)]
    union VariantData {
        i8_val: i8,
        u8_val: u8,
        i16_val: i16,
        u16_val: u16,
        i32_val: i32,
        u32_val: u32,
        i64_val: i64,
        u64_val: u64,
        f32_val: f32,
        f64_val: f64,
        ptr_val: *mut c_void,
        bstr_val: *mut u16,
        dispatch_val: *mut IDispatch,
        array_val: *mut SafeArray,
        record: VariantRecord,
    }

    #[repr(C)]
    struct Variant {
        vt: u16,
        reserved: [u16; 3],
        data: VariantData,
    }

    impl Default for Variant {
        fn default() -> Self {
            Self {
                vt: 0,
                reserved: [0; 3],
                data: VariantData { i64_val: 0 },
            }
        }
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, co_init: u32) -> HResult;
        fn CoUninitialize();
        fn CLSIDFromProgID(prog_id: *const u16, clsid: *mut Guid) -> HResult;
        fn CoCreateInstance(
            clsid: *const Guid,
            outer: *mut c_void,
            context: u32,
            iid: *const Guid,
            object: *mut *mut c_void,
        ) -> HResult;
        fn GetActiveObject(
            clsid: *const Guid,
            reserved: *mut c_void,
            object: *mut *mut c_void,
        ) -> HResult;
        fn CoGetObject(
            name: *const u16,
            bind_options: *mut c_void,
            iid: *const Guid,
            object: *mut *mut c_void,
        ) -> HResult;
        fn CoGetInstanceFromFile(
            server_info: *mut c_void,
            class_id: *const Guid,
            outer: *mut c_void,
            context: u32,
            mode: u32,
            filename: *const u16,
            count: u32,
            results: *mut MultiQi,
        ) -> HResult;
    }

    #[link(name = "oleaut32")]
    unsafe extern "system" {
        fn SysAllocStringLen(source: *const u16, length: u32) -> *mut u16;
        fn SysStringLen(value: *mut u16) -> u32;
        fn SysFreeString(value: *mut u16);
        fn VariantClear(value: *mut Variant) -> HResult;
        fn SafeArrayCreateVector(element_type: u16, lower_bound: i32, count: u32)
        -> *mut SafeArray;
        fn SafeArrayPutElement(
            array: *mut SafeArray,
            index: *mut i32,
            value: *mut c_void,
        ) -> HResult;
        fn SafeArrayGetDim(array: *mut SafeArray) -> u32;
        fn SafeArrayGetLBound(array: *mut SafeArray, dimension: u32, bound: *mut i32) -> HResult;
        fn SafeArrayGetUBound(array: *mut SafeArray, dimension: u32, bound: *mut i32) -> HResult;
        fn SafeArrayGetElement(
            array: *mut SafeArray,
            index: *mut i32,
            value: *mut c_void,
        ) -> HResult;
        fn SafeArrayDestroy(array: *mut SafeArray) -> HResult;
    }

    pub struct ApartmentGuard {
        uninitialize: bool,
    }

    impl Drop for ApartmentGuard {
        fn drop(&mut self) {
            if self.uninitialize {
                // SAFETY: 此 guard 在初始化 COM 的同一线程释放。
                unsafe { CoUninitialize() };
            }
        }
    }

    pub fn initialize() -> Result<ApartmentGuard, String> {
        // SAFETY: CoInitializeEx 要求保留参数为空指针。
        let result = unsafe { CoInitializeEx(ptr::null_mut(), COINIT_APARTMENTTHREADED) };
        if result >= 0 {
            return Ok(ApartmentGuard { uninitialize: true });
        }
        // RPC_E_CHANGED_MODE 表示线程已属于另一种 COM apartment，可沿用现有 apartment。
        if result as u32 == 0x8001_0106 {
            return Ok(ApartmentGuard {
                uninitialize: false,
            });
        }
        Err(format!(
            "初始化 COM 失败（HRESULT 0x{:08X}）",
            result as u32
        ))
    }

    pub struct ComObject {
        dispatch: NonNull<IDispatch>,
    }

    impl Clone for ComObject {
        fn clone(&self) -> Self {
            // SAFETY: `dispatch` 是此包装器持有的有效 IDispatch 引用。
            unsafe { ((*(*self.dispatch.as_ptr()).vtable).add_ref)(self.dispatch.as_ptr()) };
            Self {
                dispatch: self.dispatch,
            }
        }
    }

    impl std::fmt::Debug for ComObject {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("ComObject")
        }
    }

    impl PartialEq for ComObject {
        fn eq(&self, other: &Self) -> bool {
            self.dispatch == other.dispatch
        }
    }

    impl Drop for ComObject {
        fn drop(&mut self) {
            // SAFETY: 每个包装器恰好持有一个引用。
            unsafe { ((*(*self.dispatch.as_ptr()).vtable).release)(self.dispatch.as_ptr()) };
        }
    }

    impl ComObject {
        unsafe fn from_owned(dispatch: *mut IDispatch) -> Result<Self, String> {
            NonNull::new(dispatch)
                .map(|dispatch| Self { dispatch })
                .ok_or_else(|| "COM 返回了空对象".to_string())
        }

        fn from_borrowed(dispatch: *mut IDispatch) -> Result<Self, String> {
            if dispatch.is_null() {
                return Err("COM 返回了空对象".into());
            }
            // SAFETY: 调用期间 VARIANT 持有该 dispatch；先 AddRef，再由 VariantClear
            // 释放 VARIANT 的引用，包装器因此拥有独立引用。
            unsafe { ((*(*dispatch).vtable).add_ref)(dispatch) };
            // SAFETY: 上面刚增加了一个引用。
            unsafe { Self::from_owned(dispatch) }
        }

        fn dispid(&self, name: &str) -> Result<i32, String> {
            let mut wide: Vec<u16> = name.encode_utf16().collect();
            wide.push(0);
            let mut name_ptr = wide.as_mut_ptr();
            let mut dispid = 0;
            // SAFETY: 调用期间 `wide` 一直有效，且成员名以 NUL 结尾。
            let result = unsafe {
                ((*(*self.dispatch.as_ptr()).vtable).get_ids_of_names)(
                    self.dispatch.as_ptr(),
                    &IID_NULL,
                    &mut name_ptr,
                    1,
                    LOCALE_USER_DEFAULT,
                    &mut dispid,
                )
            };
            if result < 0 {
                return Err(format!(
                    "COM 对象不支持成员 `{name}`（HRESULT 0x{:08X}）",
                    result as u32
                ));
            }
            Ok(dispid)
        }

        pub fn get(&self, name: &str) -> Result<Value, String> {
            self.invoke_named(name, &[], &[], DISPATCH_PROPERTYGET)
                .map(|(value, _)| value)
        }

        pub fn call_with_argument_modes(
            &self,
            name: &str,
            args: &[(Option<String>, Value, bool)],
        ) -> Result<(Value, Vec<(usize, Value)>), String> {
            let mut named: Vec<(&str, (usize, Value, bool))> = Vec::new();
            let mut positional = Vec::new();
            for (index, (argument_name, value, by_ref)) in args.iter().enumerate() {
                let argument = (index, value.clone(), *by_ref);
                if let Some(argument_name) = argument_name {
                    if named
                        .iter()
                        .any(|(existing, _)| existing.eq_ignore_ascii_case(argument_name))
                    {
                        return Err(format!("COM 调用重复指定命名参数 `{argument_name}`"));
                    }
                    named.push((argument_name.as_str(), argument));
                } else {
                    positional.push(argument);
                }
            }
            self.invoke_named(
                name,
                &positional,
                &named,
                DISPATCH_METHOD | DISPATCH_PROPERTYGET,
            )
        }

        pub fn set(&self, name: &str, value: &Value) -> Result<(), String> {
            self.set_indexed(name, &[], value)
        }

        pub fn set_indexed(
            &self,
            name: &str,
            indices: &[Value],
            value: &Value,
        ) -> Result<(), String> {
            let mut args = indices.to_vec();
            args.push(value.clone());
            self.invoke_named(
                name,
                &args
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (index, value.clone(), false))
                    .collect::<Vec<_>>(),
                &[],
                if value.is_object() {
                    DISPATCH_PROPERTYPUTREF
                } else {
                    DISPATCH_PROPERTYPUT
                },
            )?;
            Ok(())
        }

        fn invoke_named(
            &self,
            name: &str,
            positional: &[(usize, Value, bool)],
            named: &[(&str, (usize, Value, bool))],
            flags: u16,
        ) -> Result<(Value, Vec<(usize, Value)>), String> {
            let (dispid, named_dispids) = self.dispids(name, named)?;
            let mut variants = named
                .iter()
                .map(|(_, (index, value, by_ref))| ArgumentVariant::new(*index, value, *by_ref))
                .chain(
                    positional
                        .iter()
                        .rev()
                        .map(|(index, value, by_ref)| ArgumentVariant::new(*index, value, *by_ref)),
                )
                .collect::<Result<Vec<_>, _>>()?;
            let mut named_dispids = named_dispids;
            let is_property_put = flags & (DISPATCH_PROPERTYPUT | DISPATCH_PROPERTYPUTREF) != 0;
            if is_property_put {
                named_dispids.clear();
                named_dispids.push(DISPID_PROPERTYPUT);
            }
            let mut params = DispParams {
                args: variants
                    .first_mut()
                    .map_or(ptr::null_mut(), |variant| &mut variant.value.0),
                named_args: if !named_dispids.is_empty() {
                    named_dispids.as_mut_ptr()
                } else {
                    ptr::null_mut()
                },
                arg_count: variants.len() as u32,
                named_arg_count: named_dispids.len() as u32,
            };
            let mut result = VariantGuard(Variant::default());
            let mut exception = ExcepInfo {
                code: 0,
                reserved: 0,
                source: ptr::null_mut(),
                description: ptr::null_mut(),
                help_file: ptr::null_mut(),
                help_context: 0,
                reserved_ptr: ptr::null_mut(),
                deferred_fill: ptr::null_mut(),
                scode: 0,
            };
            let mut argument_error = 0;
            // SAFETY: 调用期间 COM 接口及参数/结果缓冲区都保持有效。
            let status = unsafe {
                ((*(*self.dispatch.as_ptr()).vtable).invoke)(
                    self.dispatch.as_ptr(),
                    dispid,
                    &IID_NULL,
                    LOCALE_USER_DEFAULT,
                    flags,
                    &mut params,
                    &mut result.0,
                    &mut exception,
                    &mut argument_error,
                )
            };
            if status < 0 {
                let message = if !exception.description.is_null() {
                    // SAFETY: EXCEPINFO 持有 Invoke 返回的 BSTR。
                    let length = unsafe { SysStringLen(exception.description) } as usize;
                    let text = unsafe {
                        String::from_utf16_lossy(std::slice::from_raw_parts(
                            exception.description,
                            length,
                        ))
                    };
                    // SAFETY: EXCEPINFO 的 BSTR 字段由被调用方分配。
                    format!("调用 COM 成员 `{name}` 失败：{text}")
                } else {
                    format!(
                        "调用 COM 成员 `{name}` 失败（HRESULT 0x{:08X}，参数索引 {argument_error}）",
                        status as u32
                    )
                };
                // SAFETY: 这些字段是 Invoke 分配的 BSTR；SysFreeString 接受空指针。
                unsafe {
                    SysFreeString(exception.source);
                    SysFreeString(exception.description);
                    SysFreeString(exception.help_file);
                }
                return Err(message);
            }
            let by_ref_values = variants
                .iter()
                .filter_map(|argument| {
                    argument
                        .by_ref
                        .as_ref()
                        .map(|value| Ok((argument.index, value.to_value()?)))
                })
                .collect::<Result<Vec<_>, String>>()?;
            if is_property_put {
                Ok((Value::Empty, by_ref_values))
            } else {
                Ok((result.to_value()?, by_ref_values))
            }
        }

        fn dispids(
            &self,
            name: &str,
            named: &[(&str, (usize, Value, bool))],
        ) -> Result<(i32, Vec<i32>), String> {
            if named.is_empty() {
                return Ok((self.dispid(name)?, Vec::new()));
            }
            let mut wide_names = Vec::with_capacity(named.len() + 1);
            wide_names.push(name.encode_utf16().chain(Some(0)).collect::<Vec<_>>());
            wide_names.extend(
                named
                    .iter()
                    .map(|(argument, _)| argument.encode_utf16().chain(Some(0)).collect()),
            );
            let mut name_ptrs: Vec<_> = wide_names.iter_mut().map(Vec::as_mut_ptr).collect();
            let mut dispids = vec![0; name_ptrs.len()];
            // SAFETY: Each name is NUL-terminated and remains alive for the call.
            let status = unsafe {
                ((*(*self.dispatch.as_ptr()).vtable).get_ids_of_names)(
                    self.dispatch.as_ptr(),
                    &IID_NULL,
                    name_ptrs.as_mut_ptr(),
                    name_ptrs.len() as u32,
                    LOCALE_USER_DEFAULT,
                    dispids.as_mut_ptr(),
                )
            };
            if status < 0 {
                return Err(format!(
                    "COM 对象不支持成员 `{name}` 或命名参数（HRESULT 0x{:08X}）",
                    status as u32
                ));
            }
            let dispid = dispids.remove(0);
            Ok((dispid, dispids))
        }
    }

    #[repr(transparent)]
    struct VariantGuard(Variant);

    impl Drop for VariantGuard {
        fn drop(&mut self) {
            // SAFETY: VARIANT 已由本代码初始化或由 IDispatch::Invoke 返回。
            unsafe { VariantClear(&mut self.0) };
        }
    }

    impl VariantGuard {
        fn from_value(value: &Value) -> Result<Self, String> {
            let mut variant = Variant::default();
            match value {
                Value::Empty => {}
                Value::Null => variant.vt = 1,
                Value::Bool(value) => {
                    variant.vt = 11;
                    variant.data = VariantData {
                        i16_val: if *value { -1 } else { 0 },
                    };
                }

                Value::Int(value) => {
                    if let Ok(value) = i32::try_from(*value) {
                        variant.vt = 3;
                        variant.data = VariantData { i32_val: value };
                    } else {
                        variant.vt = 20;
                        variant.data = VariantData { i64_val: *value };
                    }
                }
                Value::Float(value) => {
                    variant.vt = 5;
                    variant.data = VariantData { f64_val: *value };
                }
                Value::Date(value) => {
                    variant.vt = 7;
                    variant.data = VariantData { f64_val: *value };
                }
                Value::Currency(value) => {
                    variant.vt = 6;
                    variant.data = VariantData { i64_val: *value };
                }
                Value::Str(value) => {
                    let wide: Vec<u16> = value.encode_utf16().collect();
                    let bstr = unsafe { SysAllocStringLen(wide.as_ptr(), wide.len() as u32) };
                    if bstr.is_null() {
                        return Err("分配 COM 字符串失败".into());
                    }
                    variant.vt = 8;
                    variant.data = VariantData { bstr_val: bstr };
                }
                Value::ComObject(object) => {
                    // SAFETY: 包装器持有有效 IDispatch 引用；VARIANT 额外持有一个引用，
                    // 并由 VariantClear 释放。
                    unsafe {
                        ((*(*object.dispatch.as_ptr()).vtable).add_ref)(object.dispatch.as_ptr())
                    };
                    variant.vt = 9;
                    variant.data = VariantData {
                        dispatch_val: object.dispatch.as_ptr(),
                    };
                }
                Value::Array(array) => return Self::from_array(array),
                _ => return Err(format!("不支持把 {} 作为 COM 参数", value.type_name())),
            }
            Ok(Self(variant))
        }

        fn from_array(
            array: &std::rc::Rc<std::cell::RefCell<crate::interpreter::value::VbArray>>,
        ) -> Result<Self, String> {
            let array = array.borrow();
            if array.dims().len() != 1 {
                return Err("COM Automation 目前仅支持一维 SAFEARRAY".into());
            }
            let (lower_bound, count) = array.dims()[0];
            if count > 1_000_000 {
                return Err("COM SAFEARRAY 超过一百万个元素的安全上限".into());
            }
            let count = u32::try_from(count).map_err(|_| "COM 数组元素数量超出上限")?;
            let safe_array =
                SafeArrayGuard(unsafe { SafeArrayCreateVector(VT_VARIANT, lower_bound, count) });
            if safe_array.0.is_null() {
                return Err("创建 COM SAFEARRAY 失败".into());
            }
            for (offset, item) in array.values().iter().enumerate() {
                let mut index = lower_bound
                    .checked_add(i32::try_from(offset).map_err(|_| "COM 数组下标超出范围")?)
                    .ok_or_else(|| "COM 数组下标溢出".to_string())?;
                let mut variant = VariantGuard::from_value(item)?;
                // SAFETY: The SAFEARRAY is one-dimensional and the element VARIANT stays alive.
                let status = unsafe {
                    SafeArrayPutElement(
                        safe_array.0,
                        &mut index,
                        &mut variant.0 as *mut Variant as *mut c_void,
                    )
                };
                if status < 0 {
                    return Err(format!(
                        "写入 COM SAFEARRAY 失败（HRESULT 0x{:08X}）",
                        status as u32
                    ));
                }
            }
            let safe_array = safe_array.into_raw();
            Ok(Self(Variant {
                vt: VT_ARRAY | VT_VARIANT,
                reserved: [0; 3],
                data: VariantData {
                    array_val: safe_array,
                },
            }))
        }

        fn to_value(&self) -> Result<Value, String> {
            if self.0.vt & VT_ARRAY != 0 {
                return self.array_to_value();
            }
            // SAFETY: `self.0` 是有效且已初始化的 VARIANT。
            unsafe {
                match self.0.vt {
                    0 => Ok(Value::Empty),
                    1 => Ok(Value::Null),
                    2 => Ok(Value::Int(i64::from(self.0.data.i16_val))),
                    3 => Ok(Value::Int(i64::from(self.0.data.i32_val))),
                    4 => Ok(Value::Float(f64::from(self.0.data.f32_val))),
                    5 => Ok(Value::Float(self.0.data.f64_val)),
                    7 => Ok(Value::Date(self.0.data.f64_val)),
                    6 => Ok(Value::Currency(self.0.data.i64_val)),
                    8 => {
                        let bstr = self.0.data.bstr_val;
                        if bstr.is_null() {
                            return Ok(Value::Str(String::new()));
                        }
                        let len = SysStringLen(bstr) as usize;
                        Ok(Value::Str(String::from_utf16_lossy(
                            std::slice::from_raw_parts(bstr, len),
                        )))
                    }
                    11 => Ok(Value::Bool(self.0.data.i16_val != 0)),
                    16 => Ok(Value::Int(i64::from(self.0.data.i8_val))),
                    17 => Ok(Value::Int(i64::from(self.0.data.u8_val))),
                    18 => Ok(Value::Int(i64::from(self.0.data.u16_val))),
                    19 => Ok(Value::Int(i64::from(self.0.data.u32_val))),
                    20 => Ok(Value::Int(self.0.data.i64_val)),
                    21 => i64::try_from(self.0.data.u64_val)
                        .map(Value::Int)
                        .map_err(|_| "COM 返回的 UInt64 超出解释器整数范围".into()),
                    9 => {
                        let object = ComObject::from_borrowed(self.0.data.dispatch_val)?;
                        Ok(Value::ComObject(object))
                    }
                    13 => {
                        let unknown_ptr = self.0.data.ptr_val as *mut IUnknown;
                        if unknown_ptr.is_null() {
                            return Ok(Value::Empty);
                        }
                        let mut dispatch = ptr::null_mut();
                        let status = ((*(*unknown_ptr).vtable).query_interface)(
                            unknown_ptr,
                            &IID_IDISPATCH,
                            &mut dispatch as *mut _ as *mut *mut c_void,
                        );
                        if status < 0 {
                            return Err("COM 返回了不支持 Automation 的对象".into());
                        }
                        Ok(Value::ComObject(ComObject::from_owned(dispatch)?))
                    }
                    vt => Err(format!("不支持 COM 返回的 Variant 类型 VT={vt}")),
                }
            }
        }

        fn array_to_value(&self) -> Result<Value, String> {
            let array = unsafe { self.0.data.array_val };
            if array.is_null() {
                return Ok(Value::Empty);
            }
            let element_type = self.0.vt & 0x0fff;
            // SAFETY: `array` is owned by this VARIANT and remains valid during conversion.
            let dimensions = unsafe { SafeArrayGetDim(array) };
            if dimensions != 1 {
                return Err(format!(
                    "COM 返回了 {dimensions} 维 SAFEARRAY；目前仅支持一维数组"
                ));
            }
            let mut lower_bound = 0;
            let mut upper_bound = -1;
            // SAFETY: Dimension 1 is valid because the array is one-dimensional.
            let lower_status = unsafe { SafeArrayGetLBound(array, 1, &mut lower_bound) };
            let upper_status = unsafe { SafeArrayGetUBound(array, 1, &mut upper_bound) };
            if lower_status < 0 || upper_status < 0 {
                return Err("读取 COM SAFEARRAY 边界失败".into());
            }
            let count = if upper_bound < lower_bound {
                0
            } else {
                usize::try_from(i64::from(upper_bound) - i64::from(lower_bound) + 1)
                    .map_err(|_| "COM SAFEARRAY 元素数量无效")?
            };
            if count > 1_000_000 {
                return Err("COM SAFEARRAY 超过一百万个元素的安全上限".into());
            }
            let result = Value::array_new(
                vec![(lower_bound, count)],
                crate::interpreter::value::VbType::Variant,
                true,
            );
            let result_array = result
                .as_array()
                .map_err(|message| format!("创建脚本数组失败：{message}"))?;
            for offset in 0..count {
                let index = lower_bound
                    .checked_add(i32::try_from(offset).map_err(|_| "COM SAFEARRAY 下标超出范围")?)
                    .ok_or_else(|| "COM SAFEARRAY 下标溢出".to_string())?;
                let value = safe_array_element(array, index, element_type)?;
                result_array
                    .borrow_mut()
                    .set(&[index], value)
                    .map_err(|message| format!("写入脚本数组失败：{message}"))?;
            }
            Ok(result)
        }
    }

    struct SafeArrayGuard(*mut SafeArray);

    impl SafeArrayGuard {
        fn into_raw(self) -> *mut SafeArray {
            let array = self.0;
            std::mem::forget(self);
            array
        }
    }

    impl Drop for SafeArrayGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: Guard owns this unlocked SAFEARRAY.
                unsafe { SafeArrayDestroy(self.0) };
            }
        }
    }

    fn safe_array_element(
        array: *mut SafeArray,
        index: i32,
        element_type: u16,
    ) -> Result<Value, String> {
        let index = &index as *const i32 as *mut i32;
        // SAFETY: The supplied index is within the one-dimensional SAFEARRAY bounds.
        unsafe {
            match element_type {
                VT_VARIANT => {
                    let mut element = Variant::default();
                    let status = SafeArrayGetElement(
                        array,
                        index,
                        &mut element as *mut Variant as *mut c_void,
                    );
                    let element = VariantGuard(element);
                    if status < 0 {
                        Err(format!(
                            "读取 COM SAFEARRAY 元素失败（HRESULT 0x{:08X}）",
                            status as u32
                        ))
                    } else {
                        element.to_value()
                    }
                }
                8 => {
                    let mut bstr: *mut u16 = ptr::null_mut();
                    let status =
                        SafeArrayGetElement(array, index, &mut bstr as *mut _ as *mut c_void);
                    if status < 0 {
                        return Err(format!(
                            "读取 COM SAFEARRAY 元素失败（HRESULT 0x{:08X}）",
                            status as u32
                        ));
                    }
                    let value = if bstr.is_null() {
                        String::new()
                    } else {
                        let length = SysStringLen(bstr) as usize;
                        String::from_utf16_lossy(std::slice::from_raw_parts(bstr, length))
                    };
                    SysFreeString(bstr);
                    Ok(Value::Str(value))
                }
                2 => {
                    let mut value = 0i16;
                    let status =
                        SafeArrayGetElement(array, index, &mut value as *mut _ as *mut c_void);
                    scalar_array_value(status, Value::Int(i64::from(value)))
                }
                3 => {
                    let mut value = 0i32;
                    let status =
                        SafeArrayGetElement(array, index, &mut value as *mut _ as *mut c_void);
                    scalar_array_value(status, Value::Int(i64::from(value)))
                }
                4 => {
                    let mut value = 0f32;
                    let status =
                        SafeArrayGetElement(array, index, &mut value as *mut _ as *mut c_void);
                    scalar_array_value(status, Value::Float(f64::from(value)))
                }
                5 | 7 => {
                    let mut value = 0f64;
                    let status =
                        SafeArrayGetElement(array, index, &mut value as *mut _ as *mut c_void);
                    scalar_array_value(
                        status,
                        if element_type == 7 {
                            Value::Date(value)
                        } else {
                            Value::Float(value)
                        },
                    )
                }
                6 => {
                    let mut value = 0i64;
                    let status =
                        SafeArrayGetElement(array, index, &mut value as *mut _ as *mut c_void);
                    scalar_array_value(status, Value::Currency(value))
                }
                11 => {
                    let mut value = 0i16;
                    let status =
                        SafeArrayGetElement(array, index, &mut value as *mut _ as *mut c_void);
                    scalar_array_value(status, Value::Bool(value != 0))
                }
                16 => {
                    let mut value = 0i8;
                    let status =
                        SafeArrayGetElement(array, index, &mut value as *mut _ as *mut c_void);
                    scalar_array_value(status, Value::Int(i64::from(value)))
                }
                17 => {
                    let mut value = 0u8;
                    let status =
                        SafeArrayGetElement(array, index, &mut value as *mut _ as *mut c_void);
                    scalar_array_value(status, Value::Int(i64::from(value)))
                }
                20 => {
                    let mut value = 0i64;
                    let status =
                        SafeArrayGetElement(array, index, &mut value as *mut _ as *mut c_void);
                    scalar_array_value(status, Value::Int(value))
                }
                _ => Err(format!("不支持 COM SAFEARRAY 元素类型 VT={element_type}")),
            }
        }
    }

    fn scalar_array_value(status: HResult, value: Value) -> Result<Value, String> {
        if status < 0 {
            Err(format!(
                "读取 COM SAFEARRAY 元素失败（HRESULT 0x{:08X}）",
                status as u32
            ))
        } else {
            Ok(value)
        }
    }

    struct ArgumentVariant {
        index: usize,
        value: VariantGuard,
        by_ref: Option<Box<VariantGuard>>,
    }

    impl ArgumentVariant {
        fn new(index: usize, value: &Value, by_ref: bool) -> Result<Self, String> {
            if !by_ref {
                return Ok(Self {
                    index,
                    value: VariantGuard::from_value(value)?,
                    by_ref: None,
                });
            }
            let mut target = Box::new(VariantGuard::from_value(value)?);
            let variant = Variant {
                vt: VT_VARIANT | VT_BYREF,
                reserved: [0; 3],
                data: VariantData {
                    ptr_val: &mut target.0 as *mut Variant as *mut c_void,
                },
            };
            Ok(Self {
                index,
                value: VariantGuard(variant),
                by_ref: Some(target),
            })
        }
    }

    fn class_id(prog_id: &str) -> Result<Guid, String> {
        if prog_id.contains('\0') {
            return Err("ProgID 不能包含 NUL 字符".into());
        }
        let mut wide: Vec<u16> = prog_id.encode_utf16().collect();
        wide.push(0);
        let mut clsid = IID_NULL;
        // SAFETY: ProgID 以 NUL 结尾，输出参数指向有效 GUID。
        let status = unsafe { CLSIDFromProgID(wide.as_ptr(), &mut clsid) };
        if status < 0 {
            return Err(format!(
                "找不到 COM ProgID `{prog_id}`（HRESULT 0x{:08X}）",
                status as u32
            ));
        }
        Ok(clsid)
    }

    pub fn create_object(prog_id: &str) -> Result<ComObject, String> {
        let clsid = class_id(prog_id)?;
        let mut dispatch = ptr::null_mut();
        // SAFETY: CoCreateInstance 为请求的类写入 IDispatch 指针。
        let status = unsafe {
            CoCreateInstance(
                &clsid,
                ptr::null_mut(),
                CLSCTX_ALL,
                &IID_IDISPATCH,
                &mut dispatch,
            )
        };
        if status < 0 {
            return Err(format!(
                "无法创建 COM 对象 `{prog_id}`（HRESULT 0x{:08X}）",
                status as u32
            ));
        }
        // SAFETY: CoCreateInstance 成功时返回一个由调用方持有的接口引用。
        unsafe { ComObject::from_owned(dispatch as *mut IDispatch) }
    }

    pub fn get_active_object(prog_id: &str) -> Result<ComObject, String> {
        let clsid = class_id(prog_id)?;
        let mut unknown = ptr::null_mut();
        // SAFETY: GetActiveObject 成功时返回一个由调用方持有的 IUnknown 引用。
        let status = unsafe { GetActiveObject(&clsid, ptr::null_mut(), &mut unknown) };
        if status < 0 {
            return Err(format!(
                "没有正在运行的 COM 对象 `{prog_id}`（HRESULT 0x{:08X}）",
                status as u32
            ));
        }
        let unknown = NonNull::new(unknown as *mut IUnknown)
            .ok_or_else(|| "GetActiveObject 返回了空对象".to_string())?;
        let mut dispatch = ptr::null_mut();
        // SAFETY: `unknown` 是 GetActiveObject 返回的有效 IUnknown 指针。
        let query_status = unsafe {
            ((*(*unknown.as_ptr()).vtable).query_interface)(
                unknown.as_ptr(),
                &IID_IDISPATCH,
                &mut dispatch,
            )
        };
        // 无论 QI 是否成功，都释放 IUnknown 引用。
        unsafe { ((*(*unknown.as_ptr()).vtable).release)(unknown.as_ptr()) };
        if query_status < 0 {
            return Err("正在运行的 COM 对象不支持 IDispatch".into());
        }
        // SAFETY: QueryInterface 返回一个由调用方持有的 IDispatch 引用。
        unsafe { ComObject::from_owned(dispatch as *mut IDispatch) }
    }

    pub fn get_object(pathname: &str, class: Option<&str>) -> Result<ComObject, String> {
        if let Some(class) = class
            && std::path::Path::new(pathname).is_file()
        {
            return get_object_from_file(pathname, class);
        }

        let mut display_name: Vec<u16> = pathname.encode_utf16().collect();
        display_name.push(0);
        let mut dispatch = ptr::null_mut();
        // SAFETY: CoGetObject 解析 OLE moniker display name；输出按 IID_IDISPATCH 请求。
        let status = unsafe {
            CoGetObject(
                display_name.as_ptr(),
                ptr::null_mut(),
                &IID_IDISPATCH,
                &mut dispatch,
            )
        };
        if status < 0 {
            let target = class.map_or_else(
                || format!("`{pathname}`"),
                |class| format!("`{pathname}`（{class}）"),
            );
            return Err(format!(
                "无法通过 OLE moniker 获取对象 {target}（HRESULT 0x{:08X}）",
                status as u32
            ));
        }
        // SAFETY: CoGetObject 成功时返回由调用方持有的 IDispatch 引用。
        unsafe { ComObject::from_owned(dispatch as *mut IDispatch) }
    }

    fn get_object_from_file(pathname: &str, class: &str) -> Result<ComObject, String> {
        let clsid = class_id(class)?;
        let mut filename: Vec<u16> = pathname.encode_utf16().collect();
        filename.push(0);
        let mut result = MultiQi {
            iid: &IID_IDISPATCH,
            interface: ptr::null_mut(),
            status: 0,
        };
        // SAFETY: CoGetInstanceFromFile reads the nul-terminated path and writes one MULTI_QI.
        let status = unsafe {
            CoGetInstanceFromFile(
                ptr::null_mut(),
                &clsid,
                ptr::null_mut(),
                CLSCTX_ALL,
                STGM_READWRITE,
                filename.as_ptr(),
                1,
                &mut result,
            )
        };
        if status < 0 || result.status < 0 {
            if !result.interface.is_null() {
                // SAFETY: A non-null interface returned in MULTI_QI carries an owned reference.
                unsafe { ((*(*result.interface).vtable).release)(result.interface) };
            }
            return Err(format!(
                "无法从文件 `{pathname}` 获取 `{class}` 对象（HRESULT 0x{:08X}）",
                if status < 0 {
                    status as u32
                } else {
                    result.status as u32
                }
            ));
        }
        // SAFETY: CoGetInstanceFromFile returned the requested IDispatch reference.
        unsafe { ComObject::from_owned(result.interface as *mut IDispatch) }
    }

    pub use ApartmentGuard as Guard;
    pub use ComObject as Object;
}

#[cfg(not(windows))]
mod windows {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Object;

    impl Object {
        pub fn get(&self, _: &str) -> Result<Value, String> {
            Err("COM 自动化仅支持 Windows".into())
        }

        pub fn call_with_argument_modes(
            &self,
            _: &str,
            _: &[(Option<String>, Value, bool)],
        ) -> Result<(Value, Vec<(usize, Value)>), String> {
            Err("COM 自动化仅支持 Windows".into())
        }

        pub fn set(&self, _: &str, _: &Value) -> Result<(), String> {
            Err("COM 自动化仅支持 Windows".into())
        }
    }

    #[derive(Debug)]
    pub struct Guard;

    pub fn initialize() -> Result<Guard, String> {
        Ok(Guard)
    }

    pub fn create_object(_: &str) -> Result<Object, String> {
        Err("COM 自动化仅支持 Windows".into())
    }

    pub fn get_active_object(_: &str) -> Result<Object, String> {
        Err("COM 自动化仅支持 Windows".into())
    }

    pub fn get_object(_: &str, _: Option<&str>) -> Result<Object, String> {
        Err("COM 自动化仅支持 Windows".into())
    }
}

pub use windows::{Guard as ApartmentGuard, Object as ComObject};

pub fn initialize() -> Result<ApartmentGuard, String> {
    windows::initialize()
}

pub fn create_object(prog_id: &str) -> Result<ComObject, String> {
    #[cfg(windows)]
    {
        windows::create_object(prog_id)
    }
    #[cfg(not(windows))]
    {
        let _ = prog_id;
        Err("COM 自动化仅支持 Windows".into())
    }
}

pub fn get_active_object(prog_id: &str) -> Result<ComObject, String> {
    #[cfg(windows)]
    {
        windows::get_active_object(prog_id)
    }
    #[cfg(not(windows))]
    {
        let _ = prog_id;
        Err("COM 自动化仅支持 Windows".into())
    }
}

pub fn get_object(pathname: &str, class: Option<&str>) -> Result<ComObject, String> {
    #[cfg(windows)]
    {
        windows::get_object(pathname, class)
    }
    #[cfg(not(windows))]
    {
        let _ = (pathname, class);
        Err("COM 自动化仅支持 Windows".into())
    }
}
