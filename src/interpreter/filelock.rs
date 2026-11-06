//! `Lock #` / `Unlock #` 的字节范围锁。
//!
//! `std` 的 `File::lock` 只能整文件加锁，而 VB 的 `Lock #n, start To end` 只锁一段范围，
//! 所以这里直接调 `LockFileEx` / `UnlockFileEx`。

use std::ffi::c_void;
use std::fs::File;
use std::os::windows::io::AsRawHandle;

/// `OVERLAPPED`：范围锁只用到「起始偏移 + 事件」几个字段，其余补零。
#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: *mut c_void,
}

const LOCKFILE_FAIL_IMMEDIATELY: u32 = 0x0000_0001;
const LOCKFILE_EXCLUSIVE_LOCK: u32 = 0x0000_0002;
/// `ERROR_LOCK_VIOLATION`：这段范围已经被别的句柄锁住了。
const ERROR_LOCK_VIOLATION: i32 = 33;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LockFileEx(
        file: *mut c_void,
        flags: u32,
        reserved: u32,
        low: u32,
        high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn UnlockFileEx(
        file: *mut c_void,
        reserved: u32,
        low: u32,
        high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
}

/// 范围锁失败的原因：被占用（对应 VB 错误号 70）或其它 I/O 错误。
#[derive(Debug)]
pub enum LockFailure {
    Busy,
    Io(std::io::Error),
}

/// 锁定（`unlock == false`）或解锁 `[offset, offset + length)`。
///
/// `length` 取 `u64::MAX` 表示一直锁到文件末尾。锁是按「文件句柄 + 范围」记的，
/// 同一个句柄重复锁同一段不会累加，因此解锁必须用加锁时的同一段范围。
pub fn lock_range(file: &File, offset: u64, length: u64, unlock: bool) -> Result<(), LockFailure> {
    let mut overlapped = Overlapped {
        internal: 0,
        internal_high: 0,
        offset: offset as u32,
        offset_high: (offset >> 32) as u32,
        event: std::ptr::null_mut(),
    };
    let low = length as u32;
    let high = (length >> 32) as u32;
    let handle = file.as_raw_handle();
    // SAFETY: 句柄来自仍然存活的 `File`，`OVERLAPPED` 在调用期间一直有效。
    let ok = unsafe {
        if unlock {
            UnlockFileEx(handle, 0, low, high, &mut overlapped)
        } else {
            LockFileEx(
                handle,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                low,
                high,
                &mut overlapped,
            )
        }
    };
    if ok != 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION) {
        return Err(LockFailure::Busy);
    }
    Err(LockFailure::Io(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::Write;

    fn sample_file(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        let mut file = File::create(&path).unwrap();
        file.write_all(&[0u8; 64]).unwrap();
        path
    }

    fn open(path: &std::path::Path) -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap()
    }

    #[test]
    fn ranges_conflict_only_when_they_overlap() {
        let path = sample_file("vb_interp_filelock.dat");
        let first = open(&path);
        let second = open(&path);

        lock_range(&first, 0, 10, false).unwrap();
        assert!(matches!(
            lock_range(&second, 5, 5, false),
            Err(LockFailure::Busy)
        ));
        lock_range(&second, 10, 10, false).unwrap();

        lock_range(&first, 0, 10, true).unwrap();
        lock_range(&second, 5, 5, false).unwrap();
        lock_range(&second, 5, 5, true).unwrap();
        lock_range(&second, 10, 10, true).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lock_to_end_of_file_blocks_later_ranges() {
        let path = sample_file("vb_interp_filelock_eof.dat");
        let first = open(&path);
        let second = open(&path);

        lock_range(&first, 0, u64::MAX, false).unwrap();
        assert!(matches!(
            lock_range(&second, 1000, 10, false),
            Err(LockFailure::Busy)
        ));
        lock_range(&first, 0, u64::MAX, true).unwrap();
        lock_range(&second, 1000, 10, false).unwrap();
        lock_range(&second, 1000, 10, true).unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
