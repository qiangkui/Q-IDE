//! API Key 的安全存取。
//!
//! Windows 下把密钥写进当前用户的凭据管理器（Credential Manager），
//! 不落进 `layout.txt`、不写日志、不进 `Debug` 输出。
//! 读取失败（没有配置、凭据被删除）统一按“未配置”处理。

use std::io;

/// 凭据管理器里的目标名。发布后不要改动，否则旧密钥读不回来。
const CREDENTIAL_TARGET: &str = "Q-IDE/DeepSeek API Key";

/// 读取已保存的 API Key。未配置或读取失败时返回 `None`。
pub fn load_api_key() -> Option<String> {
    load(CREDENTIAL_TARGET).filter(|key| !key.trim().is_empty())
}

/// 保存 API Key；传入空白字符串等价于删除。
pub fn save_api_key(key: &str) -> io::Result<()> {
    let key = key.trim();
    if key.is_empty() {
        delete_api_key()
    } else {
        save(CREDENTIAL_TARGET, key)
    }
}

/// 删除已保存的 API Key；本来就不存在也算成功。
pub fn delete_api_key() -> io::Result<()> {
    delete(CREDENTIAL_TARGET)
}

/// 按目标名读一条通用凭据。测试也用它验证系统调用是否通。
pub(crate) fn load(target: &str) -> Option<String> {
    platform::load(target)
}

/// 按目标名写一条通用凭据；调用方负责先去掉首尾空白。
pub(crate) fn save(target: &str, key: &str) -> io::Result<()> {
    platform::save(target, key)
}

/// 按目标名删除一条通用凭据。
pub(crate) fn delete(target: &str) -> io::Result<()> {
    platform::delete(target)
}

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::c_void;
    use std::io;

    const CRED_TYPE_GENERIC: u32 = 1;
    const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
    const ERROR_NOT_FOUND: i32 = 1168;

    #[repr(C)]
    struct FileTime {
        low_date_time: u32,
        high_date_time: u32,
    }

    /// 与 `CREDENTIALW` 内存布局一致的最小子集，只用于读写通用凭据。
    #[repr(C)]
    struct CredentialW {
        flags: u32,
        type_: u32,
        target_name: *mut u16,
        comment: *mut u16,
        last_written: FileTime,
        credential_blob_size: u32,
        credential_blob: *mut u8,
        persist: u32,
        attribute_count: u32,
        attributes: *mut c_void,
        target_alias: *mut u16,
        user_name: *mut u16,
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn CredWriteW(credential: *const CredentialW, flags: u32) -> i32;
        fn CredReadW(
            target_name: *const u16,
            type_: u32,
            flags: u32,
            credential: *mut *mut CredentialW,
        ) -> i32;
        fn CredDeleteW(target_name: *const u16, type_: u32, flags: u32) -> i32;
        fn CredFree(buffer: *const c_void);
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn load(target: &str) -> Option<String> {
        let target = wide(target);
        let mut raw: *mut CredentialW = std::ptr::null_mut();
        // SAFETY: target 以 0 结尾，raw 是有效的可写指针。
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } == 0
            || raw.is_null()
        {
            return None;
        }

        // SAFETY: CredReadW 成功后返回一块由 CredFree 释放的内存，
        // 其中 blob 指针与长度在释放前一直有效。
        let key = unsafe {
            let credential = &*raw;
            if credential.credential_blob.is_null() {
                None
            } else {
                let bytes = std::slice::from_raw_parts(
                    credential.credential_blob,
                    credential.credential_blob_size as usize,
                );
                String::from_utf8(bytes.to_vec()).ok()
            }
        };

        // SAFETY: 指针来自 CredReadW，且只释放一次。
        unsafe { CredFree(raw as *const c_void) };
        key
    }

    pub(super) fn save(target: &str, key: &str) -> io::Result<()> {
        let mut target = wide(target);
        let mut user_name = wide("Q-IDE");
        let mut blob = key.as_bytes().to_vec();
        let credential = CredentialW {
            flags: 0,
            type_: CRED_TYPE_GENERIC,
            target_name: target.as_mut_ptr(),
            comment: std::ptr::null_mut(),
            last_written: FileTime {
                low_date_time: 0,
                high_date_time: 0,
            },
            credential_blob_size: blob.len() as u32,
            credential_blob: blob.as_mut_ptr(),
            persist: CRED_PERSIST_LOCAL_MACHINE,
            attribute_count: 0,
            attributes: std::ptr::null_mut(),
            target_alias: std::ptr::null_mut(),
            user_name: user_name.as_mut_ptr(),
        };

        // SAFETY: 结构体与 Windows 的 CREDENTIALW 布局一致，
        // 内部指针指向的缓冲区在调用期间都存活。
        let ok = unsafe { CredWriteW(&credential, 0) };
        blob.fill(0);
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub(super) fn delete(target: &str) -> io::Result<()> {
        let target = wide(target);
        // SAFETY: target 以 0 结尾，其余参数与平台约定一致。
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
            return Ok(());
        }

        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_NOT_FOUND) {
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use std::io;

    pub(super) fn load(_target: &str) -> Option<String> {
        None
    }

    pub(super) fn save(_target: &str, _key: &str) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "当前平台不支持安全保存 API Key",
        ))
    }

    pub(super) fn delete(_target: &str) -> io::Result<()> {
        Ok(())
    }
}
