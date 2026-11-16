//! Windows COM Automation 桥接，支持 VB 的 `CreateObject` / `GetObject`。

use crate::interpreter::value::Value;

/// COM 调用失败：文案 + VB6 的 `Err.Number`。
///
/// VB6 的编号规则（本机实测）：`DISP_E_EXCEPTION` 取 `EXCEPINFO.scode` 的低 16 位
/// （`d.Add` 重复键 457、`Dictionary` 相关错误 32811 等），成员不存在 438，
/// 参数数量不符 450，类型不匹配 13，其余按「Automation 错误」440。
#[derive(Debug)]
pub struct ComFailure {
    pub message: String,
    pub number: i64,
}

impl ComFailure {
    fn new(message: String, number: i64) -> Self {
        Self { message, number }
    }
}

#[cfg(windows)]
mod windows {
    use super::{ComFailure, Value};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::ptr::{self, NonNull};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
    use std::sync::{Arc, Mutex};

    type HResult = i32;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq)]
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
    const IID_IENUMVARIANT: Guid = Guid {
        data1: 0x0002_0404,
        data2: 0,
        data3: 0,
        data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
    };
    /// `IProvideClassInfo`：问对象要**组件类**（coclass）的类型信息，`TypeName` 用它拿类名。
    const IID_IPROVIDECLASSINFO: Guid = Guid {
        data1: 0xB196_B283,
        data2: 0xBAB4,
        data3: 0x101A,
        data4: [0xB6, 0x9C, 0x00, 0xAA, 0x00, 0x34, 0x1D, 0x07],
    };
    /// `IConnectionPointContainer`：`WithEvents` 用它挂接 COM 事件。
    const IID_ICONNECTIONPOINTCONTAINER: Guid = Guid {
        data1: 0xB196_B284,
        data2: 0xBAB4,
        data3: 0x101A,
        data4: [0xB6, 0x9C, 0x00, 0xAA, 0x00, 0x34, 0x1D, 0x07],
    };
    const IID_IUNKNOWN: Guid = Guid {
        data1: 0,
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
    /// `DISPID_NEWENUM`：取集合的枚举器（`For Each` 走这里）。
    const DISPID_NEWENUM: i32 = -4;
    /// `DISP_E_MEMBERNOTFOUND`：成员不存在（或调用方式不对）。
    const DISP_E_MEMBERNOTFOUND: u32 = 0x8002_0003;
    /// `DISP_E_EXCEPTION`：服务器抛了异常，真正的错误号在 `EXCEPINFO.scode` 里。
    const DISP_E_EXCEPTION: u32 = 0x8002_0009;
    /// `DISP_E_BADPARAMCOUNT`：参数个数不对（VB6 报 450）。
    const DISP_E_PARAMCOUNT: u32 = 0x8002_000E;
    /// `DISP_E_TYPEMISMATCH`：参数类型不匹配（VB6 报 13）。
    const DISP_E_TYPEMISMATCH: u32 = 0x8002_0005;
    /// `E_INVALIDARG`：VB6 在 `EXCEPINFO.scode` 里拿到它时报 5（本机实测 `WScript.Shell.Run ""`）。
    const E_INVALIDARG: u32 = 0x8007_0057;
    /// `REGDB_E_CLASSNOTAVAILABLE`：`new:` moniker 找不到类（VB6 报 429）。
    const REGDB_E_CLASSNOTAVAILABLE: u32 = 0x8004_0154;
    /// `CO_E_CLASSSTRING`：`new:` moniker 里的类名不是合法 ProgID（VB6 也报 429）。
    const CO_E_CLASSSTRING: u32 = 0x8004_01F3;
    /// `vbObjectError` 所在的高位：服务器自己抛的 VB 错误（`0x800Axxxx`）取低 16 位就是 VB 错误号。
    const VB_OBJECT_ERROR: u32 = 0x800A_0000;
    const LOCALE_USER_DEFAULT: u32 = 0x0400;
    const HKEY_CLASSES_ROOT: isize = -2147483648_isize;
    const KEY_READ: u32 = 0x2_0019;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegOpenKeyExW(
            key: isize,
            sub_key: *const u16,
            options: u32,
            access: u32,
            result: *mut isize,
        ) -> i32;
        fn RegQueryValueExW(
            key: isize,
            value_name: *const u16,
            reserved: *const u32,
            kind: *mut u32,
            data: *mut u8,
            length: *mut u32,
        ) -> i32;
        fn RegEnumKeyExW(
            key: isize,
            index: u32,
            name: *mut u16,
            name_length: *mut u32,
            reserved: *const u32,
            class: *mut u16,
            class_length: *mut u32,
            last_write: *mut u64,
        ) -> i32;
        fn RegCloseKey(key: isize) -> i32;
    }
    const VT_VARIANT: u16 = 12;
    const VT_ARRAY: u16 = 0x2000;
    const VT_BYREF: u16 = 0x4000;

    #[repr(C)]
    struct SafeArray {
        _private: [u8; 0],
    }

    /// `SAFEARRAYBOUND`：`rgsabound[0]` 是**最右边**那一维（SAFEARRAY 按列优先存放）。
    #[repr(C)]
    struct SafeArrayBound {
        element_count: u32,
        lower_bound: i32,
    }

    #[repr(C)]
    struct IDispatch {
        vtable: *const IDispatchVTable,
    }

    /// `ITypeInfo`：只用到 `GetDocumentation`（取类型名给 `TypeName`），其余方法占位不调用。
    #[repr(C)]
    struct TypeInfo {
        vtable: *const TypeInfoVTable,
    }

    #[repr(C)]
    struct TypeInfoVTable {
        query_interface:
            unsafe extern "system" fn(*mut TypeInfo, *const Guid, *mut *mut c_void) -> HResult,
        add_ref: unsafe extern "system" fn(*mut TypeInfo) -> u32,
        release: unsafe extern "system" fn(*mut TypeInfo) -> u32,
        get_type_attr: unsafe extern "system" fn() -> HResult,
        get_type_comp: unsafe extern "system" fn() -> HResult,
        get_func_desc: unsafe extern "system" fn() -> HResult,
        get_var_desc: unsafe extern "system" fn() -> HResult,
        get_names: unsafe extern "system" fn() -> HResult,
        get_ref_type_of_impl_type: unsafe extern "system" fn() -> HResult,
        get_impl_type_flags: unsafe extern "system" fn() -> HResult,
        get_ids_of_names:
            unsafe extern "system" fn(*mut TypeInfo, *mut *mut u16, u32, *mut i32) -> HResult,
        invoke: unsafe extern "system" fn() -> HResult,
        get_documentation: unsafe extern "system" fn(
            *mut TypeInfo,
            i32,
            *mut *mut u16,
            *mut *mut u16,
            *mut u32,
            *mut *mut u16,
        ) -> HResult,
        get_dll_entry: unsafe extern "system" fn() -> HResult,
        get_ref_type_info: unsafe extern "system" fn() -> HResult,
        address_of_member: unsafe extern "system" fn() -> HResult,
        create_instance: unsafe extern "system" fn() -> HResult,
        get_mops: unsafe extern "system" fn() -> HResult,
        get_containing_type_lib:
            unsafe extern "system" fn(*mut TypeInfo, *mut *mut TypeLib, *mut u32) -> HResult,
    }

    /// `ITypeLib`：只用到 `GetTypeInfoOfGuid`（按源接口 IID 取类型信息）与 `Release`。
    #[repr(C)]
    struct TypeLib {
        vtable: *const TypeLibVTable,
    }

    #[repr(C)]
    struct TypeLibVTable {
        query_interface:
            unsafe extern "system" fn(*mut TypeLib, *const Guid, *mut *mut c_void) -> HResult,
        add_ref: unsafe extern "system" fn(*mut TypeLib) -> u32,
        release: unsafe extern "system" fn(*mut TypeLib) -> u32,
        get_type_info_count: unsafe extern "system" fn(*mut TypeLib, *mut u32) -> HResult,
        get_type_info: unsafe extern "system" fn() -> HResult,
        get_type_info_type: unsafe extern "system" fn() -> HResult,
        get_type_info_of_guid:
            unsafe extern "system" fn(*mut TypeLib, *const Guid, *mut *mut c_void) -> HResult,
    }

    struct TypeLibGuard {
        pointer: NonNull<TypeLib>,
    }

    impl Drop for TypeLibGuard {
        fn drop(&mut self) {
            // SAFETY: 本包装器持有一个 `ITypeLib` 引用。
            unsafe { ((*(*self.pointer.as_ptr()).vtable).release)(self.pointer.as_ptr()) };
        }
    }

    struct TypeInfoGuard {
        pointer: NonNull<TypeInfo>,
    }

    impl Drop for TypeInfoGuard {
        fn drop(&mut self) {
            // SAFETY: 本包装器持有 `GetTypeInfo` 返回的一个引用。
            unsafe { ((*(*self.pointer.as_ptr()).vtable).release)(self.pointer.as_ptr()) };
        }
    }

    struct ProvideClassInfoGuard {
        pointer: NonNull<ProvideClassInfo>,
    }

    impl Drop for ProvideClassInfoGuard {
        fn drop(&mut self) {
            // SAFETY: 本包装器持有一个 `IProvideClassInfo` 引用。
            unsafe { ((*(*self.pointer.as_ptr()).vtable).release)(self.pointer.as_ptr()) };
        }
    }

    /// 取 `ITypeInfo`（`MEMBERID_NIL`）自身的名字，同时接走 `ITypeInfo` 的引用。
    fn documentation_name(info: NonNull<TypeInfo>) -> Option<String> {
        let info = TypeInfoGuard { pointer: info };
        let mut name = ptr::null_mut();
        let mut doc_string = ptr::null_mut();
        let mut help_context = 0;
        let mut help_file = ptr::null_mut();
        // SAFETY: `info` 有效；`MEMBERID_NIL`（-1）取类型自身的名字。
        let status = unsafe {
            ((*(*info.pointer.as_ptr()).vtable).get_documentation)(
                info.pointer.as_ptr(),
                -1,
                &mut name,
                &mut doc_string,
                &mut help_context,
                &mut help_file,
            )
        };
        // SAFETY: 这些 BSTR 都由 `GetDocumentation` 分配；`SysFreeString` 接受空指针。
        unsafe {
            SysFreeString(doc_string);
            SysFreeString(help_file);
        }
        if status < 0 || name.is_null() {
            return None;
        }
        // SAFETY: `name` 是 `GetDocumentation` 分配的 BSTR。
        let length = unsafe { SysStringLen(name) } as usize;
        let text = unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(name, length)) };
        // SAFETY: 上面已按 BSTR 长度取出内容，这里释放所有权。
        unsafe { SysFreeString(name) };
        (!text.is_empty()).then_some(text)
    }

    /// `IProvideClassInfo`：只有 `GetClassInfo` 一个方法。
    #[repr(C)]
    struct ProvideClassInfo {
        vtable: *const ProvideClassInfoVTable,
    }

    #[repr(C)]
    struct ProvideClassInfoVTable {
        query_interface: unsafe extern "system" fn(
            *mut ProvideClassInfo,
            *const Guid,
            *mut *mut c_void,
        ) -> HResult,
        add_ref: unsafe extern "system" fn(*mut ProvideClassInfo) -> u32,
        release: unsafe extern "system" fn(*mut ProvideClassInfo) -> u32,
        get_class_info:
            unsafe extern "system" fn(*mut ProvideClassInfo, *mut *mut c_void) -> HResult,
    }

    /// `IConnectionPointContainer`：枚举 / 查找连接点。
    #[repr(C)]
    struct ConnectionPointContainer {
        vtable: *const ConnectionPointContainerVTable,
    }

    #[repr(C)]
    struct ConnectionPointContainerVTable {
        query_interface: unsafe extern "system" fn(
            *mut ConnectionPointContainer,
            *const Guid,
            *mut *mut c_void,
        ) -> HResult,
        add_ref: unsafe extern "system" fn(*mut ConnectionPointContainer) -> u32,
        release: unsafe extern "system" fn(*mut ConnectionPointContainer) -> u32,
        enum_connection_points: unsafe extern "system" fn(
            *mut ConnectionPointContainer,
            *mut *mut EnumConnectionPoints,
        ) -> HResult,
        find_connection_point: unsafe extern "system" fn() -> HResult,
    }

    /// `IEnumConnectionPoints`：只用到 `Next`。
    #[repr(C)]
    struct EnumConnectionPoints {
        vtable: *const EnumConnectionPointsVTable,
    }

    #[repr(C)]
    struct EnumConnectionPointsVTable {
        query_interface: unsafe extern "system" fn(
            *mut EnumConnectionPoints,
            *const Guid,
            *mut *mut c_void,
        ) -> HResult,
        add_ref: unsafe extern "system" fn(*mut EnumConnectionPoints) -> u32,
        release: unsafe extern "system" fn(*mut EnumConnectionPoints) -> u32,
        next: unsafe extern "system" fn(
            *mut EnumConnectionPoints,
            u32,
            *mut *mut ConnectionPoint,
            *mut u32,
        ) -> HResult,
        skip: unsafe extern "system" fn() -> HResult,
        reset: unsafe extern "system" fn() -> HResult,
        clone: unsafe extern "system" fn() -> HResult,
    }

    /// `IConnectionPoint`：`Advise` 挂接收器，`Unadvise` 断开。
    #[repr(C)]
    struct ConnectionPoint {
        vtable: *const ConnectionPointVTable,
    }

    #[repr(C)]
    struct ConnectionPointVTable {
        query_interface: unsafe extern "system" fn(
            *mut ConnectionPoint,
            *const Guid,
            *mut *mut c_void,
        ) -> HResult,
        add_ref: unsafe extern "system" fn(*mut ConnectionPoint) -> u32,
        release: unsafe extern "system" fn(*mut ConnectionPoint) -> u32,
        get_connection_interface:
            unsafe extern "system" fn(*mut ConnectionPoint, *mut Guid) -> HResult,
        get_connection_point_container: unsafe extern "system" fn() -> HResult,
        advise: unsafe extern "system" fn(*mut ConnectionPoint, *mut c_void, *mut u32) -> HResult,
        unadvise: unsafe extern "system" fn(*mut ConnectionPoint, u32) -> HResult,
        enum_connections: unsafe extern "system" fn() -> HResult,
    }

    /// `IEnumVARIANT`：`For Each` 遍历 COM 集合用的枚举器。
    #[repr(C)]
    struct EnumVariant {
        vtable: *const EnumVariantVTable,
    }

    #[repr(C)]
    struct EnumVariantVTable {
        query_interface:
            unsafe extern "system" fn(*mut EnumVariant, *const Guid, *mut *mut c_void) -> HResult,
        add_ref: unsafe extern "system" fn(*mut EnumVariant) -> u32,
        release: unsafe extern "system" fn(*mut EnumVariant) -> u32,
        next: unsafe extern "system" fn(*mut EnumVariant, u32, *mut Variant, *mut u32) -> HResult,
        skip: unsafe extern "system" fn(*mut EnumVariant, u32) -> HResult,
        reset: unsafe extern "system" fn(*mut EnumVariant) -> HResult,
        clone: unsafe extern "system" fn(*mut EnumVariant, *mut *mut EnumVariant) -> HResult,
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

    /// COM 调用失败：文案 + HRESULT（要区分「成员不存在」，好决定是属性还是方法），
    /// 以及 `DISP_E_EXCEPTION` 里服务器给的错误号。
    struct ComError {
        message: String,
        status: HResult,
        exception_number: i64,
    }

    impl ComError {
        fn message_only(message: String) -> Self {
            Self {
                message,
                status: 0,
                exception_number: 0,
            }
        }

        /// 名字查不到（`GetIDsOfNames` 失败）：VB6 报 438。
        fn member_not_found(message: String) -> Self {
            Self {
                message,
                status: DISP_E_MEMBERNOTFOUND as HResult,
                exception_number: 0,
            }
        }

        /// `DISP_E_MEMBERNOTFOUND`：这个名字按这种方式调不通（属性/方法试错了）。
        fn is_member_not_found(&self) -> bool {
            self.status == DISP_E_MEMBERNOTFOUND as HResult
        }

        fn into_failure(self) -> ComFailure {
            let number = if self.exception_number != 0 {
                self.exception_number
            } else if self.is_member_not_found() {
                438
            } else if self.status as u32 == DISP_E_PARAMCOUNT {
                450
            } else if self.status as u32 == DISP_E_TYPEMISMATCH {
                13
            } else {
                440
            };
            ComFailure::new(self.message, number)
        }
    }

    impl From<String> for ComError {
        fn from(message: String) -> Self {
            Self::message_only(message)
        }
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
        fn LoadTypeLib(file_name: *const u16, library: *mut *mut TypeLib) -> HResult;
        fn SysAllocStringLen(source: *const u16, length: u32) -> *mut u16;
        fn SysStringLen(value: *mut u16) -> u32;
        fn SysFreeString(value: *mut u16);
        fn VariantClear(value: *mut Variant) -> HResult;
        fn SafeArrayCreate(
            element_type: u16,
            dimensions: u32,
            bounds: *const SafeArrayBound,
        ) -> *mut SafeArray;
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
        /// 成员名（小写）→ dispid。`GetIDsOfNames` 每次都是跨 COM 的调用，
        /// 属性访问密集的脚本（反复读同一个成员、`For Each` 里读属性）能省掉大部分。
        dispids: Rc<RefCell<HashMap<String, i32>>>,
    }

    impl ComObject {
        fn wrap(dispatch: NonNull<IDispatch>) -> Self {
            Self {
                dispatch,
                dispids: Rc::new(RefCell::new(HashMap::new())),
            }
        }
    }

    impl Clone for ComObject {
        fn clone(&self) -> Self {
            // SAFETY: `dispatch` 是此包装器持有的有效 IDispatch 引用。
            unsafe { ((*(*self.dispatch.as_ptr()).vtable).add_ref)(self.dispatch.as_ptr()) };
            // 同一个 `IDispatch` 的 dispid 相同，克隆共享缓存。
            Self {
                dispatch: self.dispatch,
                dispids: Rc::clone(&self.dispids),
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
                .map(Self::wrap)
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

        /// `ObjPtr` 用的对象身份：COM 接口指针本身（同一对象相等、不同对象不等）。
        pub fn identity(&self) -> usize {
            self.dispatch.as_ptr() as usize
        }

        /// 把事件实参里保存的 `IDispatch` 指针（已 `AddRef`）重新包成对象。
        pub fn from_raw_owned(pointer: usize) -> Option<Self> {
            NonNull::new(pointer as *mut IDispatch).map(Self::wrap)
        }

        /// `TypeName` 用的类型名。
        ///
        /// 本机 VB6 实测：先问对象的**组件类**名（`Scripting.Dictionary` → `Dictionary`、
        /// `ADODB.Connection` → `Connection`、`Excel.Application` → `Application`）；服务器不支持
        /// `IProvideClassInfo` 时退回它默认接口的名字（`WScript.Shell` → `IWshShell3`、
        /// `Shell.Application` → `IShellDispatch6`）。两条路都取不到才返回 `None`。
        pub fn type_name(&self) -> Option<String> {
            // ADO 之类的服务器报的是带下划线的事件/默认接口名（`_Connection`），VB6 去掉前导下划线。
            self.class_info_name()
                .or_else(|| self.interface_name())
                .map(|name| name.trim_start_matches('_').to_string())
                .filter(|name| !name.is_empty())
        }

        /// `IProvideClassInfo::GetClassInfo` 给出的组件类（coclass）名。
        fn class_info_name(&self) -> Option<String> {
            let mut class_info = ptr::null_mut();
            // SAFETY: `self.dispatch` 是有效的 `IDispatch`，查询失败时不产生引用。
            let status = unsafe {
                ((*(*self.dispatch.as_ptr()).vtable).query_interface)(
                    self.dispatch.as_ptr(),
                    &IID_IPROVIDECLASSINFO,
                    &mut class_info,
                )
            };
            if status < 0 || class_info.is_null() {
                return None;
            }
            let class_info = ProvideClassInfoGuard {
                pointer: NonNull::new(class_info as *mut ProvideClassInfo)?,
            };
            let mut info = ptr::null_mut();
            // SAFETY: `class_info` 有效；`GetClassInfo` 成功时给出一个 `ITypeInfo` 引用。
            let status = unsafe {
                ((*(*class_info.pointer.as_ptr()).vtable).get_class_info)(
                    class_info.pointer.as_ptr(),
                    &mut info,
                )
            };
            if status < 0 || info.is_null() {
                return None;
            }
            // ADO 之类的服务器把**事件接口**（`_Connection`）当组件类名报出来，由 `type_name` 去掉前导下划线。
            documentation_name(NonNull::new(info as *mut TypeInfo)?)
        }

        /// `IDispatch::GetTypeInfo` 给出的默认接口名。
        fn interface_name(&self) -> Option<String> {
            let mut info = ptr::null_mut();
            // SAFETY: 取得一份 `ITypeInfo` 引用，由 `TypeInfoGuard` 释放。
            let status = unsafe {
                ((*(*self.dispatch.as_ptr()).vtable).get_type_info)(
                    self.dispatch.as_ptr(),
                    0,
                    LOCALE_USER_DEFAULT,
                    &mut info,
                )
            };
            if status < 0 || info.is_null() {
                return None;
            }
            documentation_name(NonNull::new(info as *mut TypeInfo)?)
        }

        /// 取 `_NewEnum`（`DISPID_NEWENUM`）枚举器，供 `For Each` 遍历 COM 集合。
        pub fn items(&self) -> Result<ComItems, ComFailure> {
            let (variant, _) = self
                .invoke_variant(
                    "_NewEnum",
                    DISPID_NEWENUM,
                    Vec::new(),
                    &[],
                    &[],
                    DISPATCH_METHOD | DISPATCH_PROPERTYGET,
                )
                .map_err(ComError::into_failure)?;
            // SAFETY: `variant` 里的 `VT_UNKNOWN` / `VT_DISPATCH` 分支都是有效接口指针；
            // 查询到的 `IEnumVARIANT` 自带一个引用，因此 `variant` 随后被 `VariantClear` 也没关系。
            let pointer = unsafe {
                let unknown = match variant.0.vt {
                    13 => variant.0.data.ptr_val as *mut EnumVariant,
                    9 => variant.0.data.dispatch_val as *mut EnumVariant,
                    // 少数服务器直接给出 `I4` 之类的可枚举值：当作没有元素。
                    0 | 1 => return Ok(ComItems { pointer: None }),
                    vt => {
                        return Err(ComFailure::new(
                            format!("COM 对象的 `_NewEnum` 返回了不支持的 Variant 类型 VT={vt}"),
                            451,
                        ));
                    }
                };
                if unknown.is_null() {
                    return Ok(ComItems { pointer: None });
                }
                let mut enumerator: *mut EnumVariant = ptr::null_mut();
                let status = ((*(*unknown).vtable).query_interface)(
                    unknown,
                    &IID_IENUMVARIANT,
                    &mut enumerator as *mut _ as *mut *mut c_void,
                );
                if status < 0 || enumerator.is_null() {
                    return Err(ComFailure::new(
                        format!(
                            "COM 对象的枚举器不是 `IEnumVARIANT`（HRESULT 0x{:08X}）",
                            status as u32
                        ),
                        451,
                    ));
                }
                NonNull::new(enumerator)
            };
            Ok(ComItems { pointer })
        }

        fn dispid(&self, name: &str) -> Result<i32, String> {
            let key = name.to_ascii_lowercase();
            if let Some(dispid) = self.dispids.borrow().get(&key) {
                return Ok(*dispid);
            }
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
            self.dispids.borrow_mut().insert(key, dispid);
            Ok(dispid)
        }

        pub fn get(&self, name: &str) -> Result<Value, ComFailure> {
            self.invoke_named(name, &[], &[], DISPATCH_PROPERTYGET)
                .map(|(value, _)| value)
                .map_err(ComError::into_failure)
        }

        pub fn call_with_argument_modes(
            &self,
            name: &str,
            args: &[(Option<String>, Value, bool)],
        ) -> Result<(Value, Vec<(usize, Value)>), ComFailure> {
            self.invoke_with_argument_modes(name, None, args)
                .map_err(ComError::into_failure)
        }

        /// `obj(下标…)`：调用**默认成员**（`DISPID_VALUE`，`d("k")` 等同于 `d.Item("k")`）。
        pub fn call_default(
            &self,
            args: &[(Option<String>, Value, bool)],
        ) -> Result<(Value, Vec<(usize, Value)>), ComFailure> {
            self.invoke_with_argument_modes("(默认成员)", Some(0), args)
                .map_err(ComError::into_failure)
        }

        /// 拆分位置/命名实参，然后按「先当属性取、失败再当方法调」的顺序 `Invoke`。
        ///
        /// `IDispatch` 不会告诉我们这个名字是属性还是方法（要读类型库才知道），
        /// 所以两种都试一次：`d.Item("k")` 这种**参数化属性**只认 `DISPATCH_PROPERTYGET`
        /// （用 `METHOD|PROPERTYGET` 调 `Scripting.Dictionary` 会静默返回空值），
        /// 而 `d.Add "k", "v"` 这种**方法**只认 `DISPATCH_METHOD`。两者都失败时优先报
        /// 不是「成员不存在」的那个错误，属性取失败时的文案（例如「键不存在」）更有用。
        fn invoke_with_argument_modes(
            &self,
            name: &str,
            dispid: Option<i32>,
            args: &[(Option<String>, Value, bool)],
        ) -> Result<(Value, Vec<(usize, Value)>), ComError> {
            let mut named: Vec<(&str, (usize, Value, bool))> = Vec::new();
            let mut positional = Vec::new();
            for (index, (argument_name, value, by_ref)) in args.iter().enumerate() {
                let argument = (index, value.clone(), *by_ref);
                if let Some(argument_name) = argument_name {
                    if named
                        .iter()
                        .any(|(existing, _)| existing.eq_ignore_ascii_case(argument_name))
                    {
                        return Err(ComError::message_only(format!(
                            "COM 调用重复指定命名参数 `{argument_name}`"
                        )));
                    }
                    named.push((argument_name.as_str(), argument));
                } else {
                    positional.push(argument);
                }
            }
            let invoke = |flags: u16| match dispid {
                Some(dispid) => {
                    self.invoke_dispid(name, dispid, Vec::new(), &positional, &named, flags)
                }
                None => self.invoke_named(name, &positional, &named, flags),
            };
            match invoke(DISPATCH_PROPERTYGET) {
                Ok(result) => Ok(result),
                Err(property_error) => match invoke(DISPATCH_METHOD) {
                    Ok(result) => Ok(result),
                    Err(method_error) => Err(if method_error.is_member_not_found() {
                        property_error
                    } else {
                        method_error
                    }),
                },
            }
        }

        pub fn set(&self, name: &str, value: &Value) -> Result<(), ComFailure> {
            self.set_indexed(name, &[], value)
        }

        pub fn set_indexed(
            &self,
            name: &str,
            indices: &[Value],
            value: &Value,
        ) -> Result<(), ComFailure> {
            self.put(name, None, indices, value)
        }

        /// `obj(下标…) = 值`：给**默认成员**赋值（`d("键") = "值"`，本机 VB6 实测可用）。
        pub fn set_default(&self, indices: &[Value], value: &Value) -> Result<(), ComFailure> {
            self.put("(默认成员)", Some(0), indices, value)
        }

        fn put(
            &self,
            name: &str,
            dispid: Option<i32>,
            indices: &[Value],
            value: &Value,
        ) -> Result<(), ComFailure> {
            let mut args = indices.to_vec();
            args.push(value.clone());
            let positional = args
                .iter()
                .enumerate()
                .map(|(index, value)| (index, value.clone(), false))
                .collect::<Vec<_>>();
            let flags = if value.is_object() {
                DISPATCH_PROPERTYPUTREF
            } else {
                DISPATCH_PROPERTYPUT
            };
            let outcome = match dispid {
                Some(dispid) => {
                    self.invoke_dispid(name, dispid, Vec::new(), &positional, &[], flags)
                }
                None => self.invoke_named(name, &positional, &[], flags),
            };
            outcome.map_err(ComError::into_failure)?;
            Ok(())
        }

        fn invoke_named(
            &self,
            name: &str,
            positional: &[(usize, Value, bool)],
            named: &[(&str, (usize, Value, bool))],
            flags: u16,
        ) -> Result<(Value, Vec<(usize, Value)>), ComError> {
            let (dispid, named_dispids) = self
                .dispids(name, named)
                .map_err(ComError::member_not_found)?;
            self.invoke_dispid(name, dispid, named_dispids, positional, named, flags)
        }

        fn invoke_dispid(
            &self,
            name: &str,
            dispid: i32,
            named_dispids: Vec<i32>,
            positional: &[(usize, Value, bool)],
            named: &[(&str, (usize, Value, bool))],
            flags: u16,
        ) -> Result<(Value, Vec<(usize, Value)>), ComError> {
            let is_property_put = flags & (DISPATCH_PROPERTYPUT | DISPATCH_PROPERTYPUTREF) != 0;
            let (result, by_ref_values) =
                self.invoke_variant(name, dispid, named_dispids, positional, named, flags)?;
            if is_property_put {
                // 属性写入没有返回值（`VARIANT` 里是 `VT_EMPTY`）。
                return Ok((Value::Empty, by_ref_values));
            }
            Ok((
                result.to_value().map_err(ComError::message_only)?,
                by_ref_values,
            ))
        }

        /// `Invoke` 后原样返回结果 `VARIANT`（`_NewEnum` 这类返回 `IUnknown` 的成员用得上）。
        fn invoke_variant(
            &self,
            name: &str,
            dispid: i32,
            mut named_dispids: Vec<i32>,
            positional: &[(usize, Value, bool)],
            named: &[(&str, (usize, Value, bool))],
            flags: u16,
        ) -> Result<(VariantGuard, Vec<(usize, Value)>), ComError> {
            // `DISPPARAMS.rgvarg` 必须是**一段连续的 VARIANT 数组**：VB 的实参在这里是倒序的，
            // `ByRef` 实参传的是「指向 VARIANT 的指针」（`VT_VARIANT | VT_BYREF`），
            // 因此目标 VARIANT 得先 `Box` 起来，调用结束后再把值取回来。
            let mut by_ref_targets: Vec<(usize, Box<VariantGuard>)> = Vec::new();
            let mut variants: Vec<VariantGuard> =
                Vec::with_capacity(named.len() + positional.len());
            let ordered = named
                .iter()
                .map(|(_, (index, value, by_ref))| (*index, value, *by_ref))
                .chain(
                    positional
                        .iter()
                        .rev()
                        .map(|(index, value, by_ref)| (*index, value, *by_ref)),
                );
            for (index, value, by_ref) in ordered {
                if by_ref {
                    let mut target =
                        Box::new(VariantGuard::from_value(value).map_err(ComError::message_only)?);
                    variants.push(VariantGuard(Variant {
                        vt: VT_VARIANT | VT_BYREF,
                        reserved: [0; 3],
                        data: VariantData {
                            ptr_val: &mut target.0 as *mut Variant as *mut c_void,
                        },
                    }));
                    by_ref_targets.push((index, target));
                } else {
                    variants.push(VariantGuard::from_value(value).map_err(ComError::message_only)?);
                }
            }
            let is_property_put = flags & (DISPATCH_PROPERTYPUT | DISPATCH_PROPERTYPUTREF) != 0;
            if is_property_put {
                named_dispids.clear();
                named_dispids.push(DISPID_PROPERTYPUT);
            }
            let mut params = DispParams {
                args: variants
                    .first_mut()
                    .map_or(ptr::null_mut(), |variant| &mut variant.0 as *mut Variant),
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
                // `DISP_E_EXCEPTION` 的真正错误号在 `EXCEPINFO.scode` 里（`d.Add` 重复键是 457）。
                let exception_number = if status as u32 == DISP_E_EXCEPTION {
                    exception_error_number(exception.scode)
                } else {
                    0
                };
                // SAFETY: 这些字段是 Invoke 分配的 BSTR；SysFreeString 接受空指针。
                unsafe {
                    SysFreeString(exception.source);
                    SysFreeString(exception.description);
                    SysFreeString(exception.help_file);
                }
                return Err(ComError {
                    message,
                    status,
                    exception_number,
                });
            }
            let by_ref_values = by_ref_targets
                .iter()
                .map(|(index, target)| {
                    target
                        .to_value()
                        .map(|value| (*index, value))
                        .map_err(ComError::message_only)
                })
                .collect::<Result<Vec<_>, ComError>>()?;
            Ok((result, by_ref_values))
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
            // 命名参数的 dispid 只对这一条调用有用，成员名则值得缓存。
            self.dispids
                .borrow_mut()
                .insert(name.to_ascii_lowercase(), dispid);
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
                // `Nothing` 传给 COM 是 `VT_DISPATCH` + 空指针（Automation 里的 null 接口引用）。
                Value::Nothing => variant.vt = 9,
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
                // UDT 没有类型库，做不到 `VT_RECORD` 封送；VB6 里把标准模块的 UDT 传给
                // 晚期绑定调用同样是编译错误（「只允许定义在公共对象模块中的用户定义类型…」，
                // 本机实测），所以这里报明确的原因，而不是笼统的「不支持」。
                Value::Record(_) => {
                    return Err(format!(
                        "不能把用户定义类型 `{}` 作为 COM 参数：VB6 只允许公共对象模块里的 UDT \
                         与 Variant 互转，QIDE 的 UDT 没有类型库、无法按 `VT_RECORD` 封送",
                        value.type_name()
                    ));
                }
                _ => return Err(format!("不支持把 {} 作为 COM 参数", value.type_name())),
            }
            Ok(Self(variant))
        }

        fn from_array(
            array: &std::rc::Rc<std::cell::RefCell<crate::interpreter::value::VbArray>>,
        ) -> Result<Self, String> {
            let array = array.borrow();
            let dims = array.dims();
            if dims.is_empty() {
                return Err("不能把未初始化的数组传给 COM".into());
            }
            let total = dims.iter().map(|(_, count)| *count).product::<usize>();
            if total > 1_000_000 {
                return Err("COM SAFEARRAY 超过一百万个元素的安全上限".into());
            }
            let cells = array.cells().to_vec();
            // 边界与下标的顺序都按 VB 的维序（`dims[0]` 在最前），与 `SafeArrayGetLBound(a, 1)`
            // 和 `SafeArrayGet/PutElement` 的下标一致（本机 ADODB `GetRows` 实测过）。
            let mut bounds = Vec::with_capacity(dims.len());
            for (lower_bound, count) in dims.iter() {
                bounds.push(SafeArrayBound {
                    element_count: u32::try_from(*count).map_err(|_| "COM 数组元素数量超出上限")?,
                    lower_bound: *lower_bound,
                });
            }
            let dimensions = u32::try_from(bounds.len()).map_err(|_| "COM 数组维数超出上限")?;
            // 元素类型跟着数组的声明类型走：VB6 传 `Long()` 是 `VT_ARRAY|VT_I4`（实测 `VarType` 8195）、
            // `String()` 是 8200、`Double()` 是 8197，`Variant()` 才是 8204。
            let element_type = safe_array_type(array.element_type());
            let safe_array = SafeArrayGuard(unsafe {
                SafeArrayCreate(element_type, dimensions, bounds.as_ptr())
            });
            if safe_array.0.is_null() {
                return Err("创建 COM SAFEARRAY 失败".into());
            }
            for (offset, cell) in cells.iter().enumerate() {
                let indices = indices_at(dims, offset)?;
                put_safe_array_element(safe_array.0, &indices, element_type, &cell.borrow())?;
            }
            let safe_array = safe_array.into_raw();
            Ok(Self(Variant {
                vt: VT_ARRAY | element_type,
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
                        // 空接口指针就是 VB 里的 `Nothing`（`Is Nothing` 为 True）。
                        if self.0.data.dispatch_val.is_null() {
                            return Ok(Value::Nothing);
                        }
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
            if dimensions == 0 {
                return Err("COM 返回了 0 维 SAFEARRAY".into());
            }
            // VB 的 `dims[0]` 对应 `SafeArrayGetLBound(a, 1)`（最左边一维）；
            // 元素下标的顺序不一样：`SafeArrayGetElement` 的 `rgIndices[0]` 是**最右边**那一维，
            // 所以取元素前要把下标反过来。
            let mut dims = vec![(0i32, 0usize); dimensions as usize];
            for dimension in 1..=dimensions {
                let mut lower_bound = 0;
                let mut upper_bound = -1;
                // SAFETY: `dimension` 落在 `1..=SafeArrayGetDim` 范围内。
                let lower_status =
                    unsafe { SafeArrayGetLBound(array, dimension, &mut lower_bound) };
                let upper_status =
                    unsafe { SafeArrayGetUBound(array, dimension, &mut upper_bound) };
                if lower_status < 0 || upper_status < 0 {
                    return Err("读取 COM SAFEARRAY 边界失败".into());
                }
                let count = if upper_bound < lower_bound {
                    0
                } else {
                    usize::try_from(i64::from(upper_bound) - i64::from(lower_bound) + 1)
                        .map_err(|_| "COM SAFEARRAY 元素数量无效")?
                };
                // `SafeArrayGetLBound/UBound` 的维号是 1 基、从**最左边**那一维开始数，
                // 和 VB 的 `LBound(a, 1)` 一致，所以第 `dimension` 维直接对应 `dims[dimension - 1]`。
                let slot =
                    usize::try_from(dimension - 1).map_err(|_| "COM SAFEARRAY 维数超出范围")?;
                dims[slot] = (lower_bound, count);
            }
            let total = dims.iter().map(|(_, count)| *count).product::<usize>();
            if total > 1_000_000 {
                return Err("COM SAFEARRAY 超过一百万个元素的安全上限".into());
            }
            let result = Value::array_new(dims.clone(), vb_type_of_safe_array(element_type), true);
            let result_array = result
                .as_array()
                .map_err(|message| format!("创建脚本数组失败：{message}"))?;
            for offset in 0..total {
                let indices = indices_at(&dims, offset)?;
                let value = safe_array_element(array, &indices, element_type)?;
                result_array
                    .borrow_mut()
                    .set(&indices, value, None)
                    .map_err(|message| format!("写入脚本数组失败：{message}"))?;
            }
            Ok(result)
        }
    }

    /// 从线性偏移还原多维下标（`dims[0]` 变化最慢，与 `VbArray::offset` 一致）。
    fn indices_at(dims: &[(i32, usize)], mut offset: usize) -> Result<Vec<i32>, String> {
        let mut indices = vec![0i32; dims.len()];
        for (dimension, (lower, count)) in dims.iter().enumerate().rev() {
            if *count == 0 {
                return Err("COM SAFEARRAY 某一维长度是 0".into());
            }
            let relative = i32::try_from(offset % count).map_err(|_| "COM SAFEARRAY 下标溢出")?;
            offset /= count;
            indices[dimension] = lower
                .checked_add(relative)
                .ok_or_else(|| "COM SAFEARRAY 下标溢出".to_string())?;
        }
        Ok(indices)
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
        indices: &[i32],
        element_type: u16,
    ) -> Result<Value, String> {
        let mut indices = indices.to_vec();
        let index = indices.as_mut_ptr();
        // SAFETY: `indices` 已按 SAFEARRAY 的顺序（最右维在前）排好且都在界内。
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

    /// `SAFEARRAY` 的元素 `VARTYPE` → 脚本数组的声明类型（`VarType` / `TypeName` 要用）。
    fn vb_type_of_safe_array(element_type: u16) -> crate::interpreter::value::VbType {
        use crate::interpreter::value::VbType;
        match element_type {
            2 => VbType::Integer,
            3 => VbType::Long,
            4 => VbType::Single,
            5 => VbType::Double,
            6 => VbType::Currency,
            7 => VbType::Date,
            8 => VbType::Str,
            11 => VbType::Boolean,
            16 | 17 => VbType::Byte,
            20 => VbType::LongLong,
            _ => VbType::Variant,
        }
    }

    /// 脚本数组的声明类型 → `SAFEARRAY` 的元素 `VARTYPE`。
    fn safe_array_type(ty: crate::interpreter::value::VbType) -> u16 {
        use crate::interpreter::value::VbType;
        match ty {
            VbType::Byte => 17,
            VbType::Integer => 2,
            VbType::Long => 3,
            VbType::LongLong | VbType::LongPtr => 20,
            VbType::Single => 4,
            VbType::Double => 5,
            VbType::Currency => 6,
            VbType::Date => 7,
            VbType::Boolean => 11,
            VbType::Str => 8,
            VbType::Variant | VbType::Object | VbType::Any => VT_VARIANT,
        }
    }

    fn put_safe_array_element(
        array: *mut SafeArray,
        indices: &[i32],
        element_type: u16,
        value: &Value,
    ) -> Result<(), String> {
        let mut indices = indices.to_vec();
        let index = indices.as_mut_ptr();
        let failed =
            |status: HResult| format!("写入 COM SAFEARRAY 失败（HRESULT 0x{:08X}）", status as u32);
        // SAFETY: 下标都在界内，元素缓冲区在调用期间保持有效。
        unsafe {
            let status = match element_type {
                VT_VARIANT => {
                    let mut variant = VariantGuard::from_value(value)?;
                    SafeArrayPutElement(array, index, &mut variant.0 as *mut Variant as *mut c_void)
                }
                8 => {
                    let text = value.to_display();
                    let wide: Vec<u16> = text.encode_utf16().collect();
                    let bstr = SysAllocStringLen(wide.as_ptr(), wide.len() as u32);
                    if bstr.is_null() {
                        return Err("分配 COM 字符串失败".into());
                    }
                    // `SafeArrayPutElement` 会自己复制一份 BSTR。
                    let status = SafeArrayPutElement(array, index, bstr as *mut c_void);
                    SysFreeString(bstr);
                    status
                }
                2 => {
                    let mut element = i16::try_from(value.as_int().map_err(|_| "溢出")?)
                        .map_err(|_| "溢出".to_string())?;
                    SafeArrayPutElement(array, index, &mut element as *mut _ as *mut c_void)
                }
                3 => {
                    let mut element = i32::try_from(value.as_int().map_err(|_| "溢出")?)
                        .map_err(|_| "溢出".to_string())?;
                    SafeArrayPutElement(array, index, &mut element as *mut _ as *mut c_void)
                }
                4 => {
                    let mut element = value.as_float().map_err(|_| "溢出".to_string())? as f32;
                    SafeArrayPutElement(array, index, &mut element as *mut _ as *mut c_void)
                }
                5 => {
                    let mut element = value.as_float().map_err(|_| "溢出".to_string())?;
                    SafeArrayPutElement(array, index, &mut element as *mut _ as *mut c_void)
                }
                6 => {
                    let mut element = value.as_currency().map_err(|_| "溢出".to_string())?;
                    SafeArrayPutElement(array, index, &mut element as *mut _ as *mut c_void)
                }
                7 => {
                    let mut element = value.as_float().map_err(|_| "溢出".to_string())?;
                    SafeArrayPutElement(array, index, &mut element as *mut _ as *mut c_void)
                }
                11 => {
                    let mut element =
                        -i16::from(value.to_bool().map_err(|_| "溢出".to_string())? as i8);
                    SafeArrayPutElement(array, index, &mut element as *mut _ as *mut c_void)
                }
                17 => {
                    let mut element = u8::try_from(value.as_int().map_err(|_| "溢出")?)
                        .map_err(|_| "溢出".to_string())?;
                    SafeArrayPutElement(array, index, &mut element as *mut _ as *mut c_void)
                }
                20 => {
                    let mut element = value.as_int().map_err(|_| "溢出".to_string())?;
                    SafeArrayPutElement(array, index, &mut element as *mut _ as *mut c_void)
                }
                other => return Err(format!("不支持把数组按 VT={other} 传给 COM")),
            };
            if status < 0 {
                return Err(failed(status));
            }
        }
        Ok(())
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

    /// `GetObject(pathname[, class])`。
    ///
    /// VB6 的语义（本机实测）：**单参数时 pathname 一律按路径/moniker 处理**，
    /// 找不到文件报 432，路径不像路径时交给 moniker 解析（失败按原 HRESULT 报，
    /// `new:` 找不到类报 429）；`GetObject(, "ProgID")` 才是「取活动对象」，
    /// `GetObject("", "ProgID")` 是「新建实例」。
    ///
    /// `pathname` 是脚本里写的原文（相对路径按脚本目录解析后的结果另由 `resolved` 给出），
    /// 只用它判断失败号：原文像文件路径就报 432，否则按 moniker 的 HRESULT 报。
    pub fn get_object(
        pathname: &str,
        resolved: &str,
        class: Option<&str>,
    ) -> Result<ComObject, ComFailure> {
        if let Some(class) = class {
            // 先解析类名（缺类报 429），再加载文件（文件不对报 432）——VB6 实测的顺序。
            let clsid = class_id(class).map_err(|message| ComFailure::new(message, 429))?;
            if !std::path::Path::new(resolved).is_file() {
                return Err(ComFailure::new(
                    format!("无法从文件 `{pathname}` 获取 `{class}` 对象：文件不存在"),
                    432,
                ));
            }
            return get_object_from_file(clsid, resolved, pathname, class);
        }

        let mut display_name: Vec<u16> = resolved.encode_utf16().collect();
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
            return Err(ComFailure::new(
                format!(
                    "无法通过 OLE moniker 获取对象 `{pathname}`（HRESULT 0x{:08X}）",
                    status as u32
                ),
                object_failure_number(pathname, status),
            ));
        }
        // SAFETY: CoGetObject 成功时返回由调用方持有的 IDispatch 引用。
        unsafe { ComObject::from_owned(dispatch as *mut IDispatch) }
            .map_err(|message| ComFailure::new(message, 429))
    }

    /// 单参数 `GetObject` 失败时的错误号（本机 VB6 实测）。
    ///
    /// 路径类（文件存在、含路径分隔符、`file:` moniker）一律 432；
    /// `new:` 之类 moniker 找不到类报 429；其余按原 HRESULT 当 `Long` 报
    /// （`clsid:00000000-…` 是 -2147024770，`GetObject("hosts")` 的 moniker 语法错误是 -2147221020）。
    fn object_failure_number(pathname: &str, status: HResult) -> i64 {
        let file_like = pathname.starts_with("file:") || pathname.contains(['\\', '/']);
        if file_like {
            return 432;
        }
        if status as u32 == REGDB_E_CLASSNOTAVAILABLE || status as u32 == CO_E_CLASSSTRING {
            return 429;
        }
        i64::from(status)
    }

    fn get_object_from_file(
        clsid: Guid,
        resolved: &str,
        pathname: &str,
        class: &str,
    ) -> Result<ComObject, ComFailure> {
        let mut filename: Vec<u16> = resolved.encode_utf16().collect();
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
            // 文件读不进来（不存在、不是 OLE 文档）在 VB6 里都是 432。
            return Err(ComFailure::new(
                format!(
                    "无法从文件 `{pathname}` 获取 `{class}` 对象（HRESULT 0x{:08X}）",
                    if status < 0 {
                        status as u32
                    } else {
                        result.status as u32
                    }
                ),
                432,
            ));
        }
        // SAFETY: CoGetInstanceFromFile returned the requested IDispatch reference.
        unsafe { ComObject::from_owned(result.interface as *mut IDispatch) }
            .map_err(|message| ComFailure::new(message, 432))
    }

    /// `EXCEPINFO.scode` → VB6 的 `Err.Number`（本机 VB6 实测，不是「一律取低 16 位」）。
    ///
    /// - `0x800Axxxx`（`vbObjectError` 系列，服务器自己抛的 VB 错误）取低 16 位：
    ///   `Scripting.FileSystemObject` 的 `0x800A0035` 报 53、`0x800A004C` 报 76、`0x800A0044` 报 68；
    /// - `E_INVALIDARG`（`0x80070057`）报 5（`WScript.Shell.Run ""`），`Dictionary` 之类也走这条；
    /// - 其余系统包装码**原样当 `Long` 报**（含负号）：`Run "no-such.exe"` 的
    ///   `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)` 是 -2147024894，`Exec "C:\Windows"` 的
    ///   `ERROR_ACCESS_DENIED` 是 -2147024891，UNC 路径不存在的 `ERROR_BAD_NETPATH` 是 -2147024843。
    fn exception_error_number(scode: HResult) -> i64 {
        let raw = scode as u32;
        if raw & 0xFFFF_0000 == VB_OBJECT_ERROR {
            return i64::from(raw & 0xFFFF);
        }
        if raw == E_INVALIDARG {
            return 5;
        }
        i64::from(scode)
    }

    #[cfg(test)]
    mod number_tests {
        use super::exception_error_number;

        #[test]
        fn exception_scode_maps_the_way_vb6_does() {
            // 服务器自带的 VB 错误号：取低 16 位。
            assert_eq!(exception_error_number(0x800A_0035u32 as i32), 53);
            assert_eq!(exception_error_number(0x800A_004Cu32 as i32), 76);
            assert_eq!(exception_error_number(0x800A_0044u32 as i32), 68);
            assert_eq!(exception_error_number(0x800A_0BBB_u32 as i32), 3003);
            // 标准 Automation 参数错误：VB6 报 5。
            assert_eq!(exception_error_number(0x8007_0057u32 as i32), 5);
            // 其余系统包装码原样报（负数）。
            assert_eq!(
                exception_error_number(0x8007_0002u32 as i32),
                -2_147_024_894
            );
            assert_eq!(
                exception_error_number(0x8007_0005u32 as i32),
                -2_147_024_891
            );
            assert_eq!(
                exception_error_number(0x8007_0035u32 as i32),
                -2_147_024_843
            );
        }
    }

    /// `For Each` 用的 COM 集合枚举器（`_NewEnum` 拿到的 `IEnumVARIANT`）。
    pub struct ComItems {
        pointer: Option<NonNull<EnumVariant>>,
    }

    impl ComItems {
        /// 取下一个元素；枚举结束返回 `None`。
        pub fn next(&mut self) -> Result<Option<Value>, ComFailure> {
            let Some(pointer) = self.pointer else {
                return Ok(None);
            };
            let mut variant = Variant::default();
            let mut fetched = 0;
            // SAFETY: `pointer` 是有效的 `IEnumVARIANT`，两个输出缓冲区归本次调用独占。
            let status = unsafe {
                ((*(*pointer.as_ptr()).vtable).next)(
                    pointer.as_ptr(),
                    1,
                    &mut variant,
                    &mut fetched,
                )
            };
            if status < 0 {
                return Err(ComFailure::new(
                    format!("枚举 COM 集合失败（HRESULT 0x{:08X}）", status as u32),
                    440,
                ));
            }
            if fetched == 0 {
                return Ok(None);
            }
            // `VariantGuard` 负责 `VariantClear`。
            VariantGuard(variant)
                .to_value()
                .map(Some)
                .map_err(|message| ComFailure::new(message, 13))
        }
    }

    impl Drop for ComItems {
        fn drop(&mut self) {
            if let Some(pointer) = self.pointer {
                // SAFETY: 本包装器持有一个 `IEnumVARIANT` 引用。
                unsafe { ((*(*pointer.as_ptr()).vtable).release)(pointer.as_ptr()) };
            }
        }
    }

    /// 一个待分派的事件：变量名 + 事件名 + 实参。
    #[derive(Clone, Debug)]
    pub struct PendingEvent {
        pub event: String,
        pub args: Vec<EventArg>,
    }

    /// 事件实参。要跨线程安全地暂存（COM 可能在消息泵里回调），所以不能直接存 `Value`。
    #[derive(Clone, Debug)]
    pub enum EventArg {
        Empty,
        Null,
        Int(i64),
        Float(f64),
        Bool(bool),
        Str(String),
        /// 对象实参：`IDispatch` 指针（已 `AddRef`，分派时包成 `ComObject`）。
        Object(usize),
    }

    /// 事件接收器（我们自己的 `IDispatch` 实现）：把回调攒进队列，由解释器在语句边界取走。
    #[repr(C)]
    pub struct ComEventSink {
        vtable: *const IDispatchVTable,
        refcount: AtomicU32,
        state: *mut SinkState,
    }

    struct SinkState {
        /// 连接点的事件接口 IID：服务器 `Advise` 时会查这个接口，必须认。
        iid: Guid,
        /// dispid → 事件名（只有脚本里真的写了处理过程的事件才登记）。
        events: std::collections::HashMap<i32, String>,
        queue: Arc<Mutex<Vec<PendingEvent>>>,
    }

    /// 已挂接的事件连接：`Drop` 时 `Unadvise` 并释放连接点。
    pub struct ComEvents {
        queue: Arc<Mutex<Vec<PendingEvent>>>,
        connections: Vec<(NonNull<ConnectionPoint>, u32)>,
    }

    impl ComEvents {
        /// 取走已排队的事件。
        pub fn drain(&self) -> Vec<PendingEvent> {
            let mut queue = self.queue.lock().expect("事件队列被中毒");
            std::mem::take(&mut *queue)
        }
    }

    impl Drop for ComEvents {
        fn drop(&mut self) {
            for (point, cookie) in self.connections.drain(..) {
                // SAFETY: `point` 是 `Advise` 时拿到的连接点引用。
                unsafe {
                    ((*(*point.as_ptr()).vtable).unadvise)(point.as_ptr(), cookie);
                    ((*(*point.as_ptr()).vtable).release)(point.as_ptr());
                }
            }
        }
    }

    impl ComObject {
        /// `WithEvents` 挂接：连接这个对象的所有连接点，把 `handlers` 里的事件名解析成 dispid。
        ///
        /// `handlers` 是脚本里实际写了处理过程的事件名（`x_Timer` 里的 `Timer`）。
        /// 对象不提供 `IConnectionPointContainer` 时按 VB6 报 459（「对象或类不支持事件集」）。
        pub fn connect_events(
            &self,
            variable: &str,
            handlers: &[String],
        ) -> Result<ComEvents, ComFailure> {
            let mut container = ptr::null_mut();
            // SAFETY: `self.dispatch` 是有效的 `IDispatch`。
            let status = unsafe {
                ((*(*self.dispatch.as_ptr()).vtable).query_interface)(
                    self.dispatch.as_ptr(),
                    &IID_ICONNECTIONPOINTCONTAINER,
                    &mut container,
                )
            };
            if status < 0 || container.is_null() {
                return Err(ComFailure::new(
                    format!("`{variable}` 的对象不支持事件（没有 IConnectionPointContainer）"),
                    459,
                ));
            }
            let container = ConnectionPointContainerGuard {
                pointer: NonNull::new(container as *mut ConnectionPointContainer)
                    .expect("上面已确认非空"),
            };
            let mut enumerator = ptr::null_mut();
            // SAFETY: `container` 有效；成功时给出一个 `IEnumConnectionPoints` 引用。
            let status = unsafe {
                ((*(*container.pointer.as_ptr()).vtable).enum_connection_points)(
                    container.pointer.as_ptr(),
                    &mut enumerator,
                )
            };
            if status < 0 || enumerator.is_null() {
                return Err(ComFailure::new(
                    format!("`{variable}` 的对象没有可用的连接点"),
                    459,
                ));
            }
            let enumerator = EnumConnectionPointsGuard {
                pointer: NonNull::new(enumerator).expect("上面已确认非空"),
            };
            let library = self.type_library();
            let queue = Arc::new(Mutex::new(Vec::new()));
            // 同一个事件可能在多个连接点上都有（ADO 就是如此），只挂第一个，避免处理过程被重复调用。
            let mut claimed: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut connections = Vec::new();
            loop {
                let mut point: *mut ConnectionPoint = ptr::null_mut();
                let mut fetched = 0u32;
                // SAFETY: 一次只取一个连接点，两个输出缓冲区归本次调用独占。
                let status = unsafe {
                    ((*(*enumerator.pointer.as_ptr()).vtable).next)(
                        enumerator.pointer.as_ptr(),
                        1,
                        &mut point,
                        &mut fetched,
                    )
                };
                if status < 0 || fetched == 0 || point.is_null() {
                    break;
                }
                let Some(point) = NonNull::new(point) else {
                    break;
                };
                // 这个连接点对应的事件接口没有我们的处理过程时跳过（不挂接也不泄漏）。
                let mut iid = IID_NULL;
                // SAFETY: `point` 有效，`iid` 是本次调用独占的输出缓冲区。
                let status = unsafe {
                    ((*(*point.as_ptr()).vtable).get_connection_interface)(point.as_ptr(), &mut iid)
                };
                let events = (status >= 0)
                    .then(|| Self::event_dispid_map(&iid, library.as_ref(), handlers))
                    .flatten()
                    .map(|mut map| {
                        map.retain(|_, name| claimed.insert(name.clone()));
                        map
                    })
                    .filter(|map| !map.is_empty());
                let Some(events) = events else {
                    // SAFETY: `Next` 返回的引用由我们释放。
                    unsafe {
                        ((*(*point.as_ptr()).vtable).release)(point.as_ptr());
                    }
                    continue;
                };
                let sink = Box::into_raw(Box::new(ComEventSink {
                    vtable: &SINK_VTABLE,
                    refcount: AtomicU32::new(1),
                    state: Box::into_raw(Box::new(SinkState {
                        iid,
                        events,
                        queue: Arc::clone(&queue),
                    })),
                }));
                let mut cookie = 0u32;
                // SAFETY: `sink` 是一个有效的 IDispatch；`Advise` 会 AddRef。
                let status = unsafe {
                    ((*(*point.as_ptr()).vtable).advise)(
                        point.as_ptr(),
                        sink as *mut c_void,
                        &mut cookie,
                    )
                };
                // SAFETY: 我们自己那一份引用交给连接点或在这里释放。
                unsafe {
                    ((*(*sink).vtable).release)(sink as *mut IDispatch);
                }
                if status < 0 {
                    // SAFETY: 挂接失败，释放连接点引用。
                    unsafe {
                        ((*(*point.as_ptr()).vtable).release)(point.as_ptr());
                    }
                    continue;
                }
                connections.push((point, cookie));
            }
            Ok(ComEvents { queue, connections })
        }

        /// 某个连接点上「脚本实现了的事件」的 dispid 映射；取不到类型信息时返回 `None`。
        fn event_dispid_map(
            iid: &Guid,
            library: Option<&TypeLibGuard>,
            handlers: &[String],
        ) -> Option<std::collections::HashMap<i32, String>> {
            // 优先用对象自己的类型库（`IProvideClassInfo`）；ADODB 这类服务器没有，就按接口 IID 查注册表。
            let info = library
                .and_then(|library| {
                    let mut info = ptr::null_mut();
                    // SAFETY: `library` 有效；成功时给出一个 `ITypeInfo` 引用。
                    let status = unsafe {
                        ((*(*library.pointer.as_ptr()).vtable).get_type_info_of_guid)(
                            library.pointer.as_ptr(),
                            iid,
                            &mut info,
                        )
                    };
                    if status < 0 || info.is_null() {
                        return None;
                    }
                    NonNull::new(info as *mut TypeInfo).map(|pointer| TypeInfoGuard { pointer })
                })
                .or_else(|| type_info_for_interface(iid))?;
            let mut map = std::collections::HashMap::new();
            for handler in handlers {
                let mut wide: Vec<u16> = handler.encode_utf16().collect();
                wide.push(0);
                let mut name = wide.as_mut_ptr();
                let mut dispid = 0i32;
                // SAFETY: 名字以 NUL 结尾且调用期间有效；`info` 是有效 `ITypeInfo`。
                let status = unsafe {
                    ((*(*info.pointer.as_ptr()).vtable).get_ids_of_names)(
                        info.pointer.as_ptr(),
                        &mut name,
                        1,
                        &mut dispid,
                    )
                };
                if status >= 0 {
                    map.insert(dispid, handler.clone());
                }
            }
            (!map.is_empty()).then_some(map)
        }

        /// 对象自己的类型库（`IProvideClassInfo` → `GetContainingTypeLib`）。
        ///
        /// `WithEvents` 的事件名要按源接口的类型信息解析 dispid，所以这里必须先拿到类型库；
        /// 服务器不提供 `IProvideClassInfo` 时无法解析名字，调用方按「没有事件」处理。
        fn type_library(&self) -> Option<TypeLibGuard> {
            let mut class_info = ptr::null_mut();
            // SAFETY: `self.dispatch` 是有效 `IDispatch`；查询失败时不产生引用。
            let status = unsafe {
                ((*(*self.dispatch.as_ptr()).vtable).query_interface)(
                    self.dispatch.as_ptr(),
                    &IID_IPROVIDECLASSINFO,
                    &mut class_info,
                )
            };
            if status < 0 || class_info.is_null() {
                return None;
            }
            let class_info = ProvideClassInfoGuard {
                pointer: NonNull::new(class_info as *mut ProvideClassInfo)?,
            };
            let mut info = ptr::null_mut();
            // SAFETY: `class_info` 有效；成功时给出一个 `ITypeInfo` 引用。
            let status = unsafe {
                ((*(*class_info.pointer.as_ptr()).vtable).get_class_info)(
                    class_info.pointer.as_ptr(),
                    &mut info,
                )
            };
            if status < 0 || info.is_null() {
                return None;
            }
            let info = TypeInfoGuard {
                pointer: NonNull::new(info as *mut TypeInfo)?,
            };
            let mut library = ptr::null_mut();
            let mut index = 0u32;
            // SAFETY: `info` 有效；成功时给出一个 `ITypeLib` 引用。
            let status = unsafe {
                ((*(*info.pointer.as_ptr()).vtable).get_containing_type_lib)(
                    info.pointer.as_ptr(),
                    &mut library,
                    &mut index,
                )
            };
            if status < 0 || library.is_null() {
                return None;
            }
            Some(TypeLibGuard {
                pointer: NonNull::new(library)?,
            })
        }
    }

    struct ConnectionPointContainerGuard {
        pointer: NonNull<ConnectionPointContainer>,
    }

    impl Drop for ConnectionPointContainerGuard {
        fn drop(&mut self) {
            // SAFETY: 本包装器持有一个引用。
            unsafe { ((*(*self.pointer.as_ptr()).vtable).release)(self.pointer.as_ptr()) };
        }
    }

    struct EnumConnectionPointsGuard {
        pointer: NonNull<EnumConnectionPoints>,
    }

    impl Drop for EnumConnectionPointsGuard {
        fn drop(&mut self) {
            // SAFETY: 本包装器持有一个引用。
            unsafe { ((*(*self.pointer.as_ptr()).vtable).release)(self.pointer.as_ptr()) };
        }
    }

    /// 按接口 IID 取 `ITypeInfo`。
    ///
    /// 走注册表：`HKCR\Interface\{IID}\TypeLib` 给出类型库 LIBID，
    /// 再从 `HKCR\TypeLib\{LIBID}\<版本>\0\win32` 拿到 .tlb/.dll 路径，`LoadTypeLib` 后按 IID 取接口类型信息。
    /// 服务器不提供 `IProvideClassInfo` 时（例如 ADODB）只能走这条路。
    fn type_info_for_interface(iid: &Guid) -> Option<TypeInfoGuard> {
        let interface_key = format!("Interface\\{}\\TypeLib", guid_text(iid));
        let library_id = registry_string(&interface_key)?;
        let versions_key = format!("TypeLib\\{library_id}");
        for version in registry_subkeys(&versions_key) {
            for platform in ["win32", "win64"] {
                let key = format!("{versions_key}\\{version}\\0\\{platform}");
                let Some(path) = registry_string(&key) else {
                    continue;
                };
                let Some(library) = load_type_library(&path) else {
                    continue;
                };
                let mut info = ptr::null_mut();
                // SAFETY: `library` 有效；成功时给出一个 `ITypeInfo` 引用。
                let status = unsafe {
                    ((*(*library.pointer.as_ptr()).vtable).get_type_info_of_guid)(
                        library.pointer.as_ptr(),
                        iid,
                        &mut info,
                    )
                };
                if status >= 0 && !info.is_null() {
                    return NonNull::new(info as *mut TypeInfo)
                        .map(|pointer| TypeInfoGuard { pointer });
                }
            }
        }
        None
    }

    /// `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` 形式的 GUID 文本（注册表键要用大写）。
    fn guid_text(guid: &Guid) -> String {
        format!(
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            guid.data1,
            guid.data2,
            guid.data3,
            guid.data4[0],
            guid.data4[1],
            guid.data4[2],
            guid.data4[3],
            guid.data4[4],
            guid.data4[5],
            guid.data4[6],
            guid.data4[7]
        )
    }

    /// 读注册表字符串值（`HKCR` 下，`value` 为空名时读默认值）。
    fn registry_string(key: &str) -> Option<String> {
        use std::os::windows::ffi::OsStrExt;
        let wide_key: Vec<u16> = std::ffi::OsStr::new(key)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut handle = 0isize;
        // SAFETY: `wide_key` 以 NUL 结尾；输出参数在调用期间有效。
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CLASSES_ROOT,
                wide_key.as_ptr(),
                0,
                KEY_READ,
                &mut handle,
            )
        };
        if status != 0 {
            return None;
        }
        let mut buffer = [0u8; 1024];
        let mut length = buffer.len() as u32;
        let mut kind = 0u32;
        // SAFETY: 缓冲区与长度都是本次调用独占的。
        let status = unsafe {
            RegQueryValueExW(
                handle,
                ptr::null(),
                ptr::null(),
                &mut kind,
                buffer.as_mut_ptr(),
                &mut length,
            )
        };
        // SAFETY: 上面打开的键在这里关闭。
        unsafe { RegCloseKey(handle) };
        if status != 0 {
            return None;
        }
        let bytes = &buffer[..length as usize];
        let units: Vec<u16> = {
            let mut units = Vec::with_capacity(bytes.len() / 2);
            for pair in bytes.chunks(2) {
                if pair.len() == 2 {
                    units.push(u16::from_le_bytes([pair[0], pair[1]]));
                }
            }
            units
        };
        let text = String::from_utf16_lossy(&units);
        Some(text.trim_end_matches('\0').to_string())
    }

    /// 枚举注册表键的子键名。
    fn registry_subkeys(key: &str) -> Vec<String> {
        use std::os::windows::ffi::OsStrExt;
        let wide_key: Vec<u16> = std::ffi::OsStr::new(key)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut handle = 0isize;
        // SAFETY: `wide_key` 以 NUL 结尾；输出参数在调用期间有效。
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CLASSES_ROOT,
                wide_key.as_ptr(),
                0,
                KEY_READ,
                &mut handle,
            )
        };
        if status != 0 {
            return Vec::new();
        }
        let mut names = Vec::new();
        for index in 0..32u32 {
            let mut buffer = [0u16; 256];
            let mut length = buffer.len() as u32;
            // SAFETY: 缓冲区与长度都是本次调用独占的；`index` 从 0 递增到失败为止。
            let status = unsafe {
                RegEnumKeyExW(
                    handle,
                    index,
                    buffer.as_mut_ptr(),
                    &mut length,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                )
            };
            if status != 0 {
                break;
            }
            names.push(String::from_utf16_lossy(&buffer[..length as usize]));
        }
        // SAFETY: 上面打开的键在这里关闭。
        unsafe { RegCloseKey(handle) };
        names
    }

    /// `LoadTypeLib`：按文件路径加载类型库。
    fn load_type_library(path: &str) -> Option<TypeLibGuard> {
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let mut library = ptr::null_mut();
        // SAFETY: `wide` 以 NUL 结尾；成功时给出一个 `ITypeLib` 引用。
        let status = unsafe { LoadTypeLib(wide.as_ptr(), &mut library) };
        if status < 0 || library.is_null() {
            return None;
        }
        NonNull::new(library).map(|pointer| TypeLibGuard { pointer })
    }

    /// 我们自己的 `IDispatch` 实现：只处理 `Invoke`，把事件攒进队列。
    static SINK_VTABLE: IDispatchVTable = IDispatchVTable {
        query_interface: sink_query_interface,
        add_ref: sink_add_ref,
        release: sink_release,
        get_type_info_count: sink_get_type_info_count,
        get_type_info: sink_get_type_info,
        get_ids_of_names: sink_get_ids_of_names,
        invoke: sink_invoke,
    };

    unsafe extern "system" fn sink_query_interface(
        this: *mut IDispatch,
        iid: *const Guid,
        out: *mut *mut c_void,
    ) -> HResult {
        // SAFETY: `this` 由 COM 按我们的布局传入。
        if out.is_null() {
            return 0x8000_4003u32 as HResult; // E_POINTER
        }
        let wanted = unsafe { &*iid };
        // 连接点会按**事件接口的 IID**（dispinterface）来查；它和我们实现的 `IDispatch` 同一套方法，
        // 所以这里认 `IUnknown` / `IDispatch` / 事件接口本身。
        // SAFETY: `this` 是一个活的 `ComEventSink`。
        let event_iid = unsafe { (*(this as *const ComEventSink)).state.as_ref() }
            .map(|state| state.iid)
            .unwrap_or(IID_NULL);
        if *wanted == IID_IUNKNOWN || *wanted == IID_IDISPATCH || *wanted == event_iid {
            unsafe {
                (*out) = this as *mut c_void;
                sink_add_ref(this);
            }
            0
        } else {
            unsafe {
                (*out) = ptr::null_mut();
            }
            0x8000_4002u32 as HResult // E_NOINTERFACE
        }
    }

    unsafe extern "system" fn sink_add_ref(this: *mut IDispatch) -> u32 {
        // SAFETY: `this` 是一个活的 `ComEventSink`。
        let sink = unsafe { &*(this as *const ComEventSink) };
        sink.refcount.fetch_add(1, AtomicOrdering::SeqCst) + 1
    }

    unsafe extern "system" fn sink_release(this: *mut IDispatch) -> u32 {
        // SAFETY: `this` 是一个活的 `ComEventSink`。
        let sink = unsafe { &*(this as *const ComEventSink) };
        let remaining = sink.refcount.fetch_sub(1, AtomicOrdering::SeqCst) - 1;
        if remaining == 0 {
            // SAFETY: 引用计数到 0，释放内部状态与自身。
            unsafe {
                drop(Box::from_raw(sink.state));
                drop(Box::from_raw(this as *mut ComEventSink));
            }
        }
        remaining
    }

    unsafe extern "system" fn sink_get_type_info_count(
        _: *mut IDispatch,
        count: *mut u32,
    ) -> HResult {
        if !count.is_null() {
            // SAFETY: 输出参数由调用方提供。
            unsafe { *count = 1 };
        }
        0
    }

    unsafe extern "system" fn sink_get_type_info(
        _: *mut IDispatch,
        _: u32,
        _: u32,
        info: *mut *mut c_void,
    ) -> HResult {
        if !info.is_null() {
            // SAFETY: 输出参数由调用方提供。
            unsafe { *info = ptr::null_mut() };
        }
        0x8000_4004u32 as HResult // E_NOTIMPL
    }

    unsafe extern "system" fn sink_get_ids_of_names(
        _: *mut IDispatch,
        _: *const Guid,
        _: *mut *mut u16,
        _: u32,
        _: u32,
        _: *mut i32,
    ) -> HResult {
        0x8000_4004u32 as HResult
    }

    unsafe extern "system" fn sink_invoke(
        this: *mut IDispatch,
        dispid: i32,
        _: *const Guid,
        _: u32,
        _: u16,
        params: *mut DispParams,
        _: *mut Variant,
        _: *mut ExcepInfo,
        _: *mut u32,
    ) -> HResult {
        // SAFETY: `this` 是一个活的 `ComEventSink`，`params` 由调用方提供。
        let sink = unsafe { &*(this as *const ComEventSink) };
        let state = unsafe { &*sink.state };
        let Some(event) = state.events.get(&dispid).cloned() else {
            return 0;
        };
        let mut args = Vec::new();
        if !params.is_null() {
            let params = unsafe { &*params };
            let count = params.arg_count as usize;
            for index in 0..count {
                if params.args.is_null() {
                    break;
                }
                // `rgvarg` 里实参是倒序的（最后一个实参在最前面）。
                let variant = unsafe { &*params.args.add(count - 1 - index) };
                args.push(event_arg(variant));
            }
        }
        if let Ok(mut queue) = state.queue.lock() {
            queue.push(PendingEvent { event, args });
        }
        0
    }

    /// 把事件回调里的 `VARIANT` 转成可以暂存的 `EventArg`。
    fn event_arg(variant: &Variant) -> EventArg {
        // SAFETY: `variant` 由调用方保证有效。
        unsafe {
            match variant.vt {
                0 => EventArg::Empty,
                1 => EventArg::Null,
                2 => EventArg::Int(i64::from(variant.data.i16_val)),
                3 => EventArg::Int(i64::from(variant.data.i32_val)),
                4 => EventArg::Float(f64::from(variant.data.f32_val)),
                5 => EventArg::Float(variant.data.f64_val),
                7 => EventArg::Float(variant.data.f64_val),
                6 => EventArg::Float(variant.data.i64_val as f64 / 10_000.0),
                8 => {
                    let bstr = variant.data.bstr_val;
                    if bstr.is_null() {
                        EventArg::Str(String::new())
                    } else {
                        let length = SysStringLen(bstr) as usize;
                        EventArg::Str(String::from_utf16_lossy(std::slice::from_raw_parts(
                            bstr, length,
                        )))
                    }
                }
                11 => EventArg::Bool(variant.data.i16_val != 0),
                9 => {
                    // 对象实参：加一个引用，等解释器分派时再包成 `ComObject`。
                    let dispatch = variant.data.dispatch_val;
                    if dispatch.is_null() {
                        EventArg::Empty
                    } else {
                        ((*(*dispatch).vtable).add_ref)(dispatch);
                        EventArg::Object(dispatch as usize)
                    }
                }
                _ => EventArg::Empty,
            }
        }
    }

    pub use ApartmentGuard as Guard;
    pub use ComEvents as Events;
    pub use ComObject as Object;
    pub use EventArg as Argument;

    #[cfg(test)]
    mod tests {
        use super::*;

        /// 建一个 `Scripting.Dictionary`；COM 不可用的机器上直接跳过（返回 `None`）。
        fn dictionary() -> Option<ComObject> {
            create_object("Scripting.Dictionary").ok()
        }

        fn call1(object: &ComObject, name: &str, argument: &str) -> Result<Value, ComFailure> {
            object
                .call_with_argument_modes(name, &[(None, Value::Str(argument.into()), false)])
                .map(|(value, _)| value)
        }

        fn default1(object: &ComObject, argument: &str) -> Result<Value, ComFailure> {
            object
                .call_default(&[(None, Value::Str(argument.into()), false)])
                .map(|(value, _)| value)
        }

        #[test]
        fn dictionary_arguments_keep_their_order_and_default_member_reads() {
            let Ok(_guard) = initialize() else {
                return;
            };
            let Some(dictionary) = dictionary() else {
                return;
            };
            // 两个实参：`rgvarg` 必须是连续的 VARIANT 数组，否则第 2 个实参（值）会读成 `Empty`。
            dictionary
                .call_with_argument_modes(
                    "Add",
                    &[
                        (None, Value::Str("k".into()), false),
                        (None, Value::Str("v".into()), false),
                    ],
                )
                .expect("Add 应当成功");
            assert_eq!(
                call1(&dictionary, "Item", "k").ok().map(|v| v.to_display()),
                Some("v".into())
            );
            assert_eq!(
                default1(&dictionary, "k").ok().map(|v| v.to_display()),
                Some("v".into())
            );
            // 不存在的键：VB6 里 `d.Item("nope")` 不报错，而是**新增**这个键。
            assert_eq!(
                call1(&dictionary, "Item", "nope")
                    .ok()
                    .map(|v| v.to_display()),
                Some(String::new())
            );
            assert_eq!(
                call1(&dictionary, "Exists", "nope")
                    .ok()
                    .map(|v| v.to_display()),
                Some("True".into())
            );
        }

        /// `GetIDsOfNames` 的结果按对象缓存（成员名小写），克隆出来的包装器共享缓存。
        #[test]
        fn dispid_lookups_are_cached_per_object() {
            let Ok(_guard) = initialize() else {
                return;
            };
            let Some(dictionary) = dictionary() else {
                return;
            };
            assert!(
                dictionary.dispids.borrow().is_empty(),
                "新对象还没有查过任何成员"
            );
            assert_eq!(
                call1(&dictionary, "Exists", "k")
                    .ok()
                    .map(|value| value.to_display()),
                Some("False".into())
            );
            assert!(
                dictionary.dispids.borrow().contains_key("exists"),
                "查过的成员名要进缓存"
            );
            // 第二次走缓存，结果不变。
            assert_eq!(
                call1(&dictionary, "Exists", "k")
                    .ok()
                    .map(|value| value.to_display()),
                Some("False".into())
            );
            let cloned = dictionary.clone();
            assert_eq!(
                cloned.dispids.borrow().len(),
                dictionary.dispids.borrow().len(),
                "克隆共享同一份 dispid 缓存"
            );
        }

        #[test]
        fn dictionary_failures_carry_vb6_error_numbers() {
            let Ok(_guard) = initialize() else {
                return;
            };
            let Some(dictionary) = dictionary() else {
                return;
            };
            let args = [
                (None, Value::Str("k".into()), false),
                (None, Value::Str("v".into()), false),
            ];
            dictionary
                .call_with_argument_modes("Add", &args)
                .expect("Add 应当成功");
            // 重复键：VB6 报 457（`DISP_E_EXCEPTION` 里 `EXCEPINFO.scode` 的低 16 位）。
            let duplicate = dictionary
                .call_with_argument_modes("Add", &args)
                .unwrap_err();
            assert_eq!(duplicate.number, 457);
            // 成员不存在：438。
            let unknown = dictionary.get("NoSuchMember").unwrap_err();
            assert_eq!(unknown.number, 438);
            // 实参个数不符：450。
            let too_few = dictionary
                .call_with_argument_modes("Add", &args[..1])
                .unwrap_err();
            assert_eq!(too_few.number, 450);
        }

        #[test]
        fn default_member_assignment_updates_the_dictionary() {
            let Ok(_guard) = initialize() else {
                return;
            };
            let Some(dictionary) = dictionary() else {
                return;
            };
            dictionary
                .set_default(&[Value::Str("x".into())], &Value::Str("y".into()))
                .expect("默认成员写入应当成功");
            assert_eq!(
                default1(&dictionary, "x").ok().map(|v| v.to_display()),
                Some("y".into())
            );
        }

        #[test]
        fn multi_dimensional_arrays_round_trip_through_safe_arrays() {
            let Ok(_guard) = initialize() else {
                return;
            };
            // `Dim a(1 To 2, 0 To 2)`：第 1 维 2 个、第 2 维 3 个，`a(2, 1) = 21`。
            let mut array = crate::interpreter::value::VbArray::new(
                vec![(1, 2), (0, 3)],
                crate::interpreter::value::VbType::Variant,
                true,
            );
            for first in 1..=2 {
                for second in 0..=2 {
                    array
                        .set(
                            &[first, second],
                            Value::Int(i64::from(first * 10 + second)),
                            None,
                        )
                        .expect("写入数组");
                }
            }
            let value = Value::Array(std::rc::Rc::new(std::cell::RefCell::new(array)));
            let variant = VariantGuard::from_value(&value).expect("转成 SAFEARRAY");
            let restored = variant.to_value().expect("转回脚本数组");
            let restored_array = restored.as_array().expect("应当还是数组");
            let restored_array = restored_array.borrow();
            assert_eq!(restored_array.dims(), [(1, 2), (0, 3)]);
            assert_eq!(restored_array.get(&[1, 0]).unwrap(), Value::Int(10));
            assert_eq!(restored_array.get(&[2, 1]).unwrap(), Value::Int(21));
            assert_eq!(restored_array.get(&[1, 2]).unwrap(), Value::Int(12));
        }

        #[test]
        fn typed_arrays_keep_their_element_type() {
            let Ok(_guard) = initialize() else {
                return;
            };
            let mut array = crate::interpreter::value::VbArray::new(
                vec![(0, 3)],
                crate::interpreter::value::VbType::Long,
                true,
            );
            array.set(&[0], Value::Int(7), None).expect("写入数组");
            array.set(&[2], Value::Int(-3), None).expect("写入数组");
            let value = Value::Array(std::rc::Rc::new(std::cell::RefCell::new(array)));
            let variant = VariantGuard::from_value(&value).expect("转成 SAFEARRAY");
            // VB6 实测默认 `Long()` 是 `VT_ARRAY|VT_I4`（`VarType` 8195），不是 `VT_ARRAY|VT_VARIANT`。
            assert_eq!(variant.0.vt, VT_ARRAY | 3);
            let restored = variant.to_value().expect("转回脚本数组");
            let restored = restored.as_array().expect("应当还是数组");
            let restored = restored.borrow();
            assert_eq!(
                restored.element_type(),
                crate::interpreter::value::VbType::Long
            );
            assert_eq!(restored.get(&[0]).expect("读元素"), Value::Int(7));
            assert_eq!(restored.get(&[2]).expect("读元素"), Value::Int(-3));
        }

        #[test]
        fn connection_events_are_captured_into_the_queue() {
            let Ok(_guard) = initialize() else {
                return;
            };
            let Ok(connection) = create_object("ADODB.Connection") else {
                return;
            };
            let events = connection
                .connect_events("conn", &["willconnect".to_string()])
                .expect("ADODB.Connection 应当支持事件连接点");
            // 连一个不存在的 Provider：ADO 会先发 `WillConnect` 再报错（3706）。
            let _ = connection.call_with_argument_modes(
                "Open",
                &[(None, Value::Str("Provider=NoSuchProvider;".into()), false)],
            );
            let drained = events.drain();
            assert!(
                drained.iter().any(|event| event.event == "willconnect"),
                "应当收到 willconnect，实际是 {:?}",
                drained
                    .iter()
                    .map(|event| event.event.clone())
                    .collect::<Vec<_>>()
            );
        }

        #[test]
        fn type_name_matches_the_vb6_class_name() {
            let Ok(_guard) = initialize() else {
                return;
            };
            let Some(dictionary) = dictionary() else {
                return;
            };
            // VB6 实测 `TypeName(d)` 是组件类名（不是 `IDictionary` 这个接口名）。
            assert_eq!(dictionary.type_name().as_deref(), Some("Dictionary"));
        }

        #[test]
        fn enumeration_walks_new_enum_and_yields_keys() {
            let Ok(_guard) = initialize() else {
                return;
            };
            let Some(dictionary) = dictionary() else {
                return;
            };
            for (key, value) in [("a", 1), ("b", 2)] {
                dictionary
                    .call_with_argument_modes(
                        "Add",
                        &[
                            (None, Value::Str(key.into()), false),
                            (None, Value::Int(value), false),
                        ],
                    )
                    .expect("Add 应当成功");
            }
            let mut enumerator = dictionary.items().expect("_NewEnum 应当可用");
            let mut keys = Vec::new();
            while let Some(item) = enumerator.next().expect("取元素应当成功") {
                keys.push(item.to_display());
            }
            assert_eq!(keys, ["a", "b"]);
        }
    }
}

#[cfg(not(windows))]
mod windows {
    use super::{ComFailure, Value};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Object;

    impl Object {
        pub fn identity(&self) -> usize {
            0
        }

        pub fn type_name(&self) -> Option<String> {
            None
        }

        pub fn get(&self, _: &str) -> Result<Value, ComFailure> {
            Err(unsupported())
        }

        pub fn call_with_argument_modes(
            &self,
            _: &str,
            _: &[(Option<String>, Value, bool)],
        ) -> Result<(Value, Vec<(usize, Value)>), ComFailure> {
            Err(unsupported())
        }

        pub fn call_default(
            &self,
            _: &[(Option<String>, Value, bool)],
        ) -> Result<(Value, Vec<(usize, Value)>), ComFailure> {
            Err(unsupported())
        }

        pub fn set(&self, _: &str, _: &Value) -> Result<(), ComFailure> {
            Err(unsupported())
        }

        pub fn set_indexed(&self, _: &str, _: &[Value], _: &Value) -> Result<(), ComFailure> {
            Err(unsupported())
        }

        pub fn set_default(&self, _: &[Value], _: &Value) -> Result<(), ComFailure> {
            Err(unsupported())
        }

        pub fn items(&self) -> Result<ComItems, ComFailure> {
            Err(unsupported())
        }
    }

    /// `For Each` 用的 COM 集合枚举器（非 Windows 上不可用）。
    #[derive(Debug)]
    pub struct ComItems;

    impl ComItems {
        pub fn next(&mut self) -> Result<Option<Value>, ComFailure> {
            Err(unsupported())
        }
    }

    fn unsupported() -> ComFailure {
        ComFailure::new("COM 自动化仅支持 Windows".into(), 440)
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

    pub fn get_object(_: &str, _: &str, _: Option<&str>) -> Result<Object, ComFailure> {
        Err(ComFailure::new("COM 自动化仅支持 Windows".into(), 429))
    }
}

pub use windows::{
    Argument as EventArg, Events as ComEvents, Guard as ApartmentGuard, Object as ComObject,
};

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

pub fn get_object(
    pathname: &str,
    resolved: &str,
    class: Option<&str>,
) -> Result<ComObject, ComFailure> {
    #[cfg(windows)]
    {
        windows::get_object(pathname, resolved, class)
    }
    #[cfg(not(windows))]
    {
        let _ = (pathname, resolved, class);
        Err(ComFailure::new("COM 自动化仅支持 Windows".into(), 429))
    }
}
