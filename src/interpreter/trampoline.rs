//! `AddressOf` 需要的机器码跳板。
//!
//! Win32 回调是原生函数指针，VB 过程却活在解释器里。这里为每个回调生成一段
//! x64 机器码：把上下文指针塞进 `RCX`、把前三个实参寄存器整体后移一位，
//! 第四个实参挪到栈上的第五个参数位置，再调用 [`crate::interpreter::runtime`]
//! 里的转发函数。转发函数按 VB 过程的形参类型把寄存器值还原成
//! [`crate::interpreter::value::Value`]。
//!
//! 直接作为 `Declare` 实参的跳板只在该次调用期间有效（同步回调）；
//! 写进 UDT 字段的跳板由运行时保管到脚本结束（`runtime::PersistentCallback`）。

use std::ffi::c_void;

/// 一段跳板的机器码长度（字节）。
pub const STUB_LEN: usize = 45;

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
/// sub rsp, 0x28         48 83 EC 28   ; 影子空间 + 第 5 个参数槽
/// mov [rsp+0x20], r9    4C 89 4C 24 20 ; 原第 4 个实参 → 栈上第 5 个参数
/// mov r9, r8            4D 8B C8      ; 原第 3 个实参 → r9
/// mov r8, rdx           4C 8B C2      ; 原第 2 个实参 → r8
/// mov rdx, rcx          48 8B D1      ; 原第 1 个实参 → rdx
/// mov rcx, <context>    48 B9 ..      ; 上下文指针 → rcx
/// mov rax, <thunk>      48 B8 ..
/// call rax              FF D0         ; 调用转发函数
/// add rsp, 0x28         48 83 C4 28
/// ret                   C3
/// ```
///
/// 因此转发函数签名是 `extern "system" fn(ctx, a1, a2, a3, a4) -> usize`，
/// 最多支持 4 个回调参数。分配 `0x28` 字节后栈仍按 16 字节对齐：
/// 入口 `rsp % 16 == 8`，减 40 后为 0，`call` 压入返回地址后转发函数看到 8。
/// 四个搬移都用 `8B`（`MOV r64, r/m64`，目标在 `reg` 字段）这一种形式，
/// 免得两种编码方向混着写。
fn stub_bytes(context: usize, thunk: usize) -> [u8; STUB_LEN] {
    let mut bytes = [0u8; STUB_LEN];
    bytes[0..4].copy_from_slice(&[0x48, 0x83, 0xEC, 0x28]);
    bytes[4..9].copy_from_slice(&[0x4C, 0x89, 0x4C, 0x24, 0x20]);
    bytes[9..12].copy_from_slice(&[0x4D, 0x8B, 0xC8]);
    bytes[12..15].copy_from_slice(&[0x4C, 0x8B, 0xC2]);
    bytes[15..18].copy_from_slice(&[0x48, 0x8B, 0xD1]);
    bytes[18..20].copy_from_slice(&[0x48, 0xB9]);
    bytes[20..28].copy_from_slice(&(context as u64).to_le_bytes());
    bytes[28..30].copy_from_slice(&[0x48, 0xB8]);
    bytes[30..38].copy_from_slice(&(thunk as u64).to_le_bytes());
    bytes[38..40].copy_from_slice(&[0xFF, 0xD0]);
    bytes[40..44].copy_from_slice(&[0x48, 0x83, 0xC4, 0x28]);
    bytes[44] = 0xC3;
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static CAPTURED: Mutex<[usize; 5]> = Mutex::new([0; 5]);

    extern "system" fn probe(
        context: usize,
        first: usize,
        second: usize,
        third: usize,
        fourth: usize,
    ) -> usize {
        let mut slot = CAPTURED.lock().unwrap();
        slot.copy_from_slice(&[context, first, second, third, fourth]);
        drop(slot);
        first + second + third + fourth
    }

    extern "system" fn probe_digits(
        _context: usize,
        first: usize,
        second: usize,
        third: usize,
        fourth: usize,
    ) -> usize {
        first * 1000 + second * 100 + third * 10 + fourth
    }

    #[test]
    fn trampoline_passes_registers_in_order() {
        let thunk: extern "system" fn(usize, usize, usize, usize, usize) -> usize = probe_digits;
        let trampoline = Trampoline::new(0, thunk as usize).unwrap();
        let expected = stub_bytes(0, thunk as usize);
        let actual =
            unsafe { std::slice::from_raw_parts(trampoline.code as *const u8, STUB_LEN) };
        assert_eq!(actual, &expected[..], "写入的机器码不一致");
        type F4 = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
        // SAFETY: 跳板末尾跳到 `probe_digits`，签名与其一致。
        let callback: F4 = unsafe { std::mem::transmute(trampoline.address()) };
        assert_eq!(unsafe { callback(1, 2, 3, 4) }, 1234);
        assert_eq!(unsafe { callback(7, 5, 9, 2) }, 7592);
    }

    #[test]
    fn stub_shifts_registers_and_embeds_pointers() {
        let bytes = stub_bytes(0x1122_3344_5566_7788, 0x99AA_BBCC_DDEE_FF00);
        assert_eq!(&bytes[0..4], &[0x48, 0x83, 0xEC, 0x28]);
        assert_eq!(&bytes[4..9], &[0x4C, 0x89, 0x4C, 0x24, 0x20]);
        assert_eq!(&bytes[9..12], &[0x4D, 0x8B, 0xC8]);
        assert_eq!(&bytes[12..15], &[0x4C, 0x8B, 0xC2]);
        assert_eq!(&bytes[15..18], &[0x48, 0x8B, 0xD1]);
        assert_eq!(&bytes[18..20], &[0x48, 0xB9]);
        assert_eq!(
            u64::from_le_bytes(bytes[20..28].try_into().unwrap()),
            0x1122_3344_5566_7788
        );
        assert_eq!(&bytes[28..30], &[0x48, 0xB8]);
        assert_eq!(
            u64::from_le_bytes(bytes[30..38].try_into().unwrap()),
            0x99AA_BBCC_DDEE_FF00
        );
        assert_eq!(&bytes[38..40], &[0xFF, 0xD0]);
        assert_eq!(&bytes[40..44], &[0x48, 0x83, 0xC4, 0x28]);
        assert_eq!(bytes[44], 0xC3);
    }

    #[test]
    fn trampoline_forwards_context_and_four_arguments() {
        let thunk: extern "system" fn(usize, usize, usize, usize, usize) -> usize = probe;
        let trampoline = Trampoline::new(0xDEAD_BEEF, thunk as usize).unwrap();
        type F4 = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
        // SAFETY: 跳板末尾跳到 `probe`，签名与其一致。
        let callback: F4 = unsafe { std::mem::transmute(trampoline.address()) };
        let sum = unsafe { callback(1, 2, 3, 4) };
        let captured = *CAPTURED.lock().unwrap();
        assert_eq!(captured, [0xDEAD_BEEF, 1, 2, 3, 4]);
        assert_eq!(sum, 10);
    }
}
