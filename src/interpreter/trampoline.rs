//! `AddressOf` 需要的机器码跳板。
//!
//! Win32 回调是原生函数指针，VB 过程却活在解释器里。这里为每个回调生成一段
//! x64 机器码：把上下文指针塞进 `RCX`、把三个实参寄存器整体后移一位，
//! 再跳到 [`crate::interpreter::runtime`] 里的转发函数。转发函数按 VB 过程的
//! 形参类型把寄存器值还原成 [`crate::interpreter::value::Value`]。
//!
//! 跳板只在一次 `Declare` 调用期间有效（调用返回即释放），
//! 因此只适用于「调用期间同步回调」的 API（`EnumWindows`、`InitOnceExecuteOnce` 等）。

use std::ffi::c_void;

/// 一段跳板的机器码长度（字节）。
pub const STUB_LEN: usize = 31;

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const PAGE_SIZE: usize = 4096;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualAlloc(
        address: *mut c_void,
        size: usize,
        allocation_type: u32,
        protect: u32,
    ) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, free_type: u32) -> i32;
    fn FlushInstructionCache(
        process: *mut c_void,
        base: *const c_void,
        size: usize,
    ) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
}

/// 可执行内存里的一个回调跳板；`Drop` 时释放。
pub struct Trampoline {
    code: *mut u8,
}

impl Trampoline {
    /// 生成一个把回调转发到 `thunk`、并把 `context` 作为第一个实参的跳板。
    pub fn new(context: usize, thunk: usize) -> Result<Self, String> {
        let bytes = stub_bytes(context, thunk);
        // SAFETY: 只是申请一页可执行内存并写入我们自己生成的机器码。
        let code = unsafe {
            VirtualAlloc(
                std::ptr::null_mut(),
                PAGE_SIZE,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        } as *mut u8;
        if code.is_null() {
            return Err("无法申请可执行内存（`AddressOf` 跳板）".into());
        }
        let trampoline = Self { code };
        // SAFETY: `code` 是我们刚申请的一页可写内存，`bytes` 正好放得下。
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), code, bytes.len());
            FlushInstructionCache(GetCurrentProcess(), code.cast(), bytes.len());
        }
        Ok(trampoline)
    }

    /// 交给 Win32 的函数指针。
    pub fn address(&self) -> usize {
        self.code as usize
    }
}

impl Drop for Trampoline {
    fn drop(&mut self) {
        // SAFETY: `code` 来自 `VirtualAlloc`，且只释放一次。
        unsafe {
            VirtualFree(self.code.cast(), 0, MEM_RELEASE);
        }
    }
}

/// x64 跳板的机器码：
///
/// ```text
/// mov r9, r8           4D 8B C8   ; 原第 3 个实参 → r9
/// mov r8, rdx          4C 8B C2   ; 原第 2 个实参 → r8
/// mov rdx, rcx         48 8B D1   ; 原第 1 个实参 → rdx
/// mov rcx, <context>   48 B9 ..   ; 上下文指针 → rcx
/// mov rax, <thunk>     48 B8 ..
/// jmp rax              FF E0      ; 尾跳，栈与影子空间原样交给转发函数
/// ```
///
/// 因此转发函数签名是 `extern "system" fn(ctx, a1, a2, a3) -> usize`，
/// 最多支持 3 个回调参数（第 4 个起要落在栈上，会覆盖调用方的数据）。
/// 三个搬移都用 `8B`（`MOV r64, r/m64`，目标在 `reg` 字段）这一种形式，
/// 免得两种编码方向混着写。
fn stub_bytes(context: usize, thunk: usize) -> [u8; STUB_LEN] {
    let mut bytes = [0u8; STUB_LEN];
    bytes[0..3].copy_from_slice(&[0x4D, 0x8B, 0xC8]);
    bytes[3..6].copy_from_slice(&[0x4C, 0x8B, 0xC2]);
    bytes[6..9].copy_from_slice(&[0x48, 0x8B, 0xD1]);
    bytes[9..11].copy_from_slice(&[0x48, 0xB9]);
    bytes[11..19].copy_from_slice(&(context as u64).to_le_bytes());
    bytes[19..21].copy_from_slice(&[0x48, 0xB8]);
    bytes[21..29].copy_from_slice(&(thunk as u64).to_le_bytes());
    bytes[29..31].copy_from_slice(&[0xFF, 0xE0]);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static CAPTURED: Mutex<[usize; 4]> = Mutex::new([0; 4]);

    extern "system" fn probe(context: usize, first: usize, second: usize, third: usize) -> usize {
        let mut slot = CAPTURED.lock().unwrap();
        slot.copy_from_slice(&[context, first, second, third]);
        drop(slot);
        first + second + third
    }

    extern "system" fn probe_digits(
        _context: usize,
        first: usize,
        second: usize,
        third: usize,
    ) -> usize {
        first * 100 + second * 10 + third
    }

    #[test]
    fn trampoline_passes_registers_in_order() {
        let thunk: extern "system" fn(usize, usize, usize, usize) -> usize = probe_digits;
        let trampoline = Trampoline::new(0, thunk as usize).unwrap();
        let expected = stub_bytes(0, thunk as usize);
        let actual =
            unsafe { std::slice::from_raw_parts(trampoline.code as *const u8, STUB_LEN) };
        assert_eq!(actual, &expected[..], "写入的机器码不一致");
        type F3 = unsafe extern "system" fn(usize, usize, usize) -> usize;
        // SAFETY: 跳板尾部跳到 `probe_digits`，签名与其一致。
        let callback: F3 = unsafe { std::mem::transmute(trampoline.address()) };
        assert_eq!(unsafe { callback(1, 2, 3) }, 123);
        assert_eq!(unsafe { callback(7, 5, 9) }, 759);
    }

    #[test]
    fn stub_shifts_registers_and_embeds_pointers() {
        let bytes = stub_bytes(0x1122_3344_5566_7788, 0x99AA_BBCC_DDEE_FF00);
        assert_eq!(&bytes[0..3], &[0x4D, 0x8B, 0xC8]);
        assert_eq!(&bytes[3..6], &[0x4C, 0x8B, 0xC2]);
        assert_eq!(&bytes[6..9], &[0x48, 0x8B, 0xD1]);
        assert_eq!(&bytes[9..11], &[0x48, 0xB9]);
        assert_eq!(
            u64::from_le_bytes(bytes[11..19].try_into().unwrap()),
            0x1122_3344_5566_7788
        );
        assert_eq!(&bytes[19..21], &[0x48, 0xB8]);
        assert_eq!(
            u64::from_le_bytes(bytes[21..29].try_into().unwrap()),
            0x99AA_BBCC_DDEE_FF00
        );
        assert_eq!(&bytes[29..31], &[0xFF, 0xE0]);
    }

    #[test]
    fn trampoline_forwards_context_and_three_arguments() {
        let thunk: extern "system" fn(usize, usize, usize, usize) -> usize = probe;
        let trampoline = Trampoline::new(0xDEAD_BEEF, thunk as usize).unwrap();
        type F3 = unsafe extern "system" fn(usize, usize, usize) -> usize;
        // SAFETY: 跳板尾部跳到 `probe`，签名与其一致。
        let callback: F3 = unsafe { std::mem::transmute(trampoline.address()) };
        let sum = unsafe { callback(1, 2, 3) };
        let captured = *CAPTURED.lock().unwrap();
        assert_eq!(captured, [0xDEAD_BEEF, 1, 2, 3]);
        assert_eq!(sum, 6);
    }
}
