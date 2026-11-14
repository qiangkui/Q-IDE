//! `AddressOf` 需要的机器码跳板。
//!
//! Win32 回调是原生函数指针，VB 过程却活在解释器里。这里为每个回调生成一段
//! x64 机器码：保存整数与浮点参数寄存器及栈参数，把上下文和寄存器快照传给
//! [`crate::interpreter::runtime`] 里的转发函数。转发函数按 VB 过程的形参类型还原成
//! [`crate::interpreter::value::Value`]。
//!
//! 直接作为 `Declare` 实参的跳板只在该次调用期间有效（同步回调）；
//! 写进 UDT 字段的跳板由运行时保管到脚本结束（`runtime::PersistentCallback`）。

use std::ffi::c_void;

/// Win64 下调用桩与回调跳板统一支持的实参数量上限。
pub const MAX_ABI_ARGS: usize = 64;

#[repr(C)]
pub struct CallbackRegisters {
    pub integer: [usize; MAX_ABI_ARGS],
    pub float: [u64; MAX_ABI_ARGS],
}

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
    fn FlushInstructionCache(process: *mut c_void, base: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
}

/// 可执行内存里的一个回调跳板；`Drop` 时释放。
pub struct Trampoline {
    code: *mut u8,
}

impl Trampoline {
    /// 生成一个把上下文与原始寄存器快照转发给 `thunk` 的跳板。
    pub fn new(context: usize, thunk: usize, arg_count: usize) -> Result<Self, String> {
        if arg_count > MAX_ABI_ARGS {
            return Err(format!("回调最多支持 {MAX_ABI_ARGS} 个参数"));
        }
        let bytes = stub_bytes(context, thunk, arg_count);
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

pub enum InvokeArg {
    Integer(usize),
    Single(f32),
    Double(f64),
}

pub enum InvokeResult {
    Integer(usize),
    Single(f32),
    Double(f64),
}

struct ExecutableCode {
    code: *mut u8,
}

impl ExecutableCode {
    fn new(bytes: &[u8]) -> Result<Self, String> {
        let code = unsafe {
            VirtualAlloc(
                std::ptr::null_mut(),
                PAGE_SIZE,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        } as *mut u8;
        if code.is_null() {
            return Err("无法申请可执行内存（Win32 ABI 调用）".into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), code, bytes.len());
            FlushInstructionCache(GetCurrentProcess(), code.cast(), bytes.len());
        }
        Ok(Self { code })
    }
}

impl Drop for ExecutableCode {
    fn drop(&mut self) {
        unsafe {
            VirtualFree(self.code.cast(), 0, MEM_RELEASE);
        }
    }
}

/// 通过运行时生成的 Windows x64 调用桩处理任意标量参数组合。
pub fn invoke(
    address: usize,
    args: &[InvokeArg],
    return_type: crate::interpreter::value::VbType,
) -> Result<InvokeResult, String> {
    if args.len() > MAX_ABI_ARGS {
        return Err(format!("Win32 调用最多支持 {MAX_ABI_ARGS} 个标量参数"));
    }
    let code = ExecutableCode::new(&invoke_stub(address, args))?;
    unsafe {
        match return_type {
            crate::interpreter::value::VbType::Single => {
                let function: unsafe extern "system" fn() -> f32 = std::mem::transmute(code.code);
                Ok(InvokeResult::Single(function()))
            }
            crate::interpreter::value::VbType::Double | crate::interpreter::value::VbType::Date => {
                let function: unsafe extern "system" fn() -> f64 = std::mem::transmute(code.code);
                Ok(InvokeResult::Double(function()))
            }
            _ => {
                let function: unsafe extern "system" fn() -> usize = std::mem::transmute(code.code);
                Ok(InvokeResult::Integer(function()))
            }
        }
    }
}

fn emit_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn emit_rax(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&[0x48, 0xB8]);
    emit_u64(bytes, value);
}

fn invoke_stub(address: usize, args: &[InvokeArg]) -> Vec<u8> {
    let stack_args = args.len().saturating_sub(4);
    let mut frame_size = 32 + stack_args * 8;
    if frame_size.is_multiple_of(16) {
        frame_size += 8;
    }
    let mut bytes = vec![0x48, 0x81, 0xEC];
    bytes.extend_from_slice(&(frame_size as u32).to_le_bytes());

    for (index, arg) in args.iter().enumerate().skip(4) {
        let bits = match arg {
            InvokeArg::Integer(value) => *value as u64,
            InvokeArg::Single(value) => u64::from(value.to_bits()),
            InvokeArg::Double(value) => value.to_bits(),
        };
        emit_rax(&mut bytes, bits);
        let stack_offset = 0x20 + (index - 4) * 8;
        if stack_offset <= i8::MAX as usize {
            bytes.extend_from_slice(&[0x48, 0x89, 0x44, 0x24, stack_offset as u8]);
        } else {
            bytes.extend_from_slice(&[0x48, 0x89, 0x84, 0x24]);
            bytes.extend_from_slice(&(stack_offset as u32).to_le_bytes());
        }
    }
    for (index, arg) in args.iter().take(4).enumerate() {
        match arg {
            InvokeArg::Integer(value) => {
                let (rex, opcode) = match index {
                    0 => (0x48, 0xB9),
                    1 => (0x48, 0xBA),
                    2 => (0x49, 0xB8),
                    _ => (0x49, 0xB9),
                };
                bytes.extend_from_slice(&[rex, opcode]);
                emit_u64(&mut bytes, *value as u64);
            }
            InvokeArg::Single(value) => {
                emit_rax(&mut bytes, u64::from(value.to_bits()));
                bytes.extend_from_slice(&[0x66, 0x48, 0x0F, 0x6E, 0xC0 + index as u8 * 8]);
            }
            InvokeArg::Double(value) => {
                emit_rax(&mut bytes, value.to_bits());
                bytes.extend_from_slice(&[0x66, 0x48, 0x0F, 0x6E, 0xC0 + index as u8 * 8]);
            }
        }
    }
    emit_rax(&mut bytes, address as u64);
    bytes.extend_from_slice(&[0xFF, 0xD0, 0x48, 0x81, 0xC4]);
    bytes.extend_from_slice(&(frame_size as u32).to_le_bytes());
    bytes.push(0xC3);
    bytes
}

/// 保存前四个位置参数寄存器，并把后续栈参数复制到同一快照布局。
fn stub_bytes(context: usize, thunk: usize, arg_count: usize) -> Vec<u8> {
    const REGISTER_SNAPSHOT_OFFSET: u32 = 0x20;
    const FLOAT_SNAPSHOT_OFFSET: u32 =
        REGISTER_SNAPSHOT_OFFSET + (MAX_ABI_ARGS * std::mem::size_of::<usize>()) as u32;
    const FRAME_SIZE: u32 =
        FLOAT_SNAPSHOT_OFFSET + (MAX_ABI_ARGS * std::mem::size_of::<usize>()) as u32;
    const ALIGNED_FRAME_SIZE: u32 = FRAME_SIZE + 8;

    let mut bytes = vec![0x48, 0x81, 0xEC];
    bytes.extend_from_slice(&ALIGNED_FRAME_SIZE.to_le_bytes());
    bytes.extend_from_slice(&[
        0x48,
        0x89,
        0x4C,
        0x24,
        REGISTER_SNAPSHOT_OFFSET as u8, // mov [rsp+20h], rcx
        0x48,
        0x89,
        0x54,
        0x24,
        (REGISTER_SNAPSHOT_OFFSET + 8) as u8, // mov [rsp+28h], rdx
        0x4C,
        0x89,
        0x44,
        0x24,
        (REGISTER_SNAPSHOT_OFFSET + 16) as u8, // mov [rsp+30h], r8
        0x4C,
        0x89,
        0x4C,
        0x24,
        (REGISTER_SNAPSHOT_OFFSET + 24) as u8, // mov [rsp+38h], r9
    ]);
    for index in 0..4u8 {
        let offset = FLOAT_SNAPSHOT_OFFSET + u32::from(index) * 8;
        bytes.extend_from_slice(&[0x66, 0x0F, 0xD6, 0x84 + index * 8, 0x24]);
        bytes.extend_from_slice(&offset.to_le_bytes());
    }
    for index in 4..arg_count {
        let source = ALIGNED_FRAME_SIZE + 0x28 + ((index - 4) * 8) as u32;
        let integer_target =
            REGISTER_SNAPSHOT_OFFSET + (index * std::mem::size_of::<usize>()) as u32;
        let float_target = FLOAT_SNAPSHOT_OFFSET + (index * std::mem::size_of::<usize>()) as u32;
        for target in [integer_target, float_target] {
            bytes.extend_from_slice(&[0x48, 0x8B, 0x84, 0x24]);
            bytes.extend_from_slice(&source.to_le_bytes());
            bytes.extend_from_slice(&[0x48, 0x89, 0x84, 0x24]);
            bytes.extend_from_slice(&target.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&[0x48, 0xB9]);
    bytes.extend_from_slice(&(context as u64).to_le_bytes());
    bytes.extend_from_slice(&[0x48, 0x8D, 0x94, 0x24]);
    bytes.extend_from_slice(&REGISTER_SNAPSHOT_OFFSET.to_le_bytes());
    bytes.extend_from_slice(&[0x48, 0xB8]);
    bytes.extend_from_slice(&(thunk as u64).to_le_bytes());
    bytes.extend_from_slice(&[0xFF, 0xD0, 0x48, 0x81, 0xC4]);
    bytes.extend_from_slice(&ALIGNED_FRAME_SIZE.to_le_bytes());
    bytes.push(0xC3);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "system" fn callback_stack_probe(
        arg_count: usize,
        args: *const CallbackRegisters,
    ) -> usize {
        let args = unsafe { &*args };
        args.integer[..arg_count].iter().sum()
    }

    extern "system" fn sum_forty(
        a0: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
        a6: usize,
        a7: usize,
        a8: usize,
        a9: usize,
        a10: usize,
        a11: usize,
        a12: usize,
        a13: usize,
        a14: usize,
        a15: usize,
        a16: usize,
        a17: usize,
        a18: usize,
        a19: usize,
        a20: usize,
        a21: usize,
        a22: usize,
        a23: usize,
        a24: usize,
        a25: usize,
        a26: usize,
        a27: usize,
        a28: usize,
        a29: usize,
        a30: usize,
        a31: usize,
        a32: usize,
        a33: usize,
        a34: usize,
        a35: usize,
        a36: usize,
        a37: usize,
        a38: usize,
        a39: usize,
    ) -> usize {
        [
            a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, a16, a17, a18,
            a19, a20, a21, a22, a23, a24, a25, a26, a27, a28, a29, a30, a31, a32, a33, a34, a35,
            a36, a37, a38, a39,
        ]
        .iter()
        .sum()
    }

    extern "system" fn callback_probe(_context: usize, args: *const CallbackRegisters) -> usize {
        let args = unsafe { &*args };
        args.integer[0] + args.integer[1] + args.integer[2] + args.integer[3]
    }

    extern "system" fn callback_float_probe(
        _context: usize,
        args: *const CallbackRegisters,
    ) -> f64 {
        let args = unsafe { &*args };
        f64::from_bits(args.float[0]) + f64::from_bits(args.float[1])
    }

    extern "system" fn callback_single_probe(
        _context: usize,
        args: *const CallbackRegisters,
    ) -> f32 {
        let args = unsafe { &*args };
        f32::from_bits(args.float[0] as u32) + f32::from_bits(args.float[1] as u32)
    }

    extern "system" fn mixed_probe(
        first: f32,
        second: usize,
        third: f64,
        fourth: i32,
        fifth: f32,
    ) -> f64 {
        f64::from(first) + second as f64 + third + f64::from(fourth) + f64::from(fifth)
    }

    extern "system" fn single_probe(first: f64, second: usize, third: f32, fourth: usize) -> f32 {
        (first + second as f64 + f64::from(third) + fourth as f64) as f32
    }

    #[test]
    fn dynamic_call_stub_handles_mixed_floats_and_stack_arguments() {
        let args = [
            InvokeArg::Single(1.5),
            InvokeArg::Integer(2),
            InvokeArg::Double(3.0),
            InvokeArg::Integer(4),
            InvokeArg::Single(5.5),
        ];
        let result = invoke(
            mixed_probe as *const () as usize,
            &args,
            crate::interpreter::value::VbType::Double,
        )
        .unwrap();
        let InvokeResult::Double(value) = result else {
            panic!("expected Double result");
        };
        assert_eq!(value, 16.0);
    }

    #[test]
    fn dynamic_call_stub_preserves_single_return_register() {
        let args = [
            InvokeArg::Double(1.5),
            InvokeArg::Integer(2),
            InvokeArg::Single(3.0),
            InvokeArg::Integer(4),
        ];
        let result = invoke(
            single_probe as *const () as usize,
            &args,
            crate::interpreter::value::VbType::Single,
        )
        .unwrap();
        let InvokeResult::Single(value) = result else {
            panic!("expected Single result");
        };
        assert_eq!(value, 10.5);
    }

    #[test]
    fn dynamic_call_stub_handles_forty_integer_arguments() {
        let args: Vec<_> = (1..=40).map(InvokeArg::Integer).collect();
        let result = invoke(
            sum_forty as *const () as usize,
            &args,
            crate::interpreter::value::VbType::LongLong,
        )
        .unwrap();
        let InvokeResult::Integer(value) = result else {
            panic!("expected integer result");
        };
        assert_eq!(value, 820);
    }

    #[test]
    fn trampoline_captures_integer_registers() {
        let thunk: extern "system" fn(usize, *const CallbackRegisters) -> usize = callback_probe;
        let trampoline = Trampoline::new(0, thunk as usize, 4).unwrap();
        let expected = stub_bytes(0, thunk as usize, 4);
        let actual = unsafe { std::slice::from_raw_parts(trampoline.code, expected.len()) };
        assert_eq!(actual, expected);
        type F4 = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
        let callback: F4 = unsafe { std::mem::transmute(trampoline.address()) };
        assert_eq!(unsafe { callback(1, 2, 3, 4) }, 10);
    }

    #[test]
    fn trampoline_captures_stack_arguments_through_fourteen_parameters() {
        let thunk: extern "system" fn(usize, *const CallbackRegisters) -> usize =
            callback_stack_probe;
        let trampoline = Trampoline::new(14, thunk as usize, 14).unwrap();
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
        let callback: F14 = unsafe { std::mem::transmute(trampoline.address()) };
        assert_eq!(
            unsafe { callback(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14) },
            105
        );
    }

    #[test]
    fn trampoline_captures_stack_arguments_past_byte_displacement_range() {
        let thunk: extern "system" fn(usize, *const CallbackRegisters) -> usize =
            callback_stack_probe;
        let trampoline = Trampoline::new(40, thunk as usize, 40).unwrap();
        type F40 = unsafe extern "system" fn(
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
        let callback: F40 = unsafe { std::mem::transmute(trampoline.address()) };
        assert_eq!(
            unsafe {
                callback(
                    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                    23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
                )
            },
            820
        );
    }

    #[test]
    fn trampoline_captures_floating_registers() {
        let thunk: extern "system" fn(usize, *const CallbackRegisters) -> f64 =
            callback_float_probe;
        let trampoline = Trampoline::new(0, thunk as usize, 2).unwrap();
        type F2 = unsafe extern "system" fn(f64, f64) -> f64;
        let callback: F2 = unsafe { std::mem::transmute(trampoline.address()) };
        assert_eq!(unsafe { callback(1.25, 2.5) }, 3.75);
    }

    #[test]
    fn trampoline_preserves_single_return_register() {
        let thunk: extern "system" fn(usize, *const CallbackRegisters) -> f32 =
            callback_single_probe;
        let trampoline = Trampoline::new(0, thunk as usize, 2).unwrap();
        type F2 = unsafe extern "system" fn(f32, f32) -> f32;
        let callback: F2 = unsafe { std::mem::transmute(trampoline.address()) };
        assert_eq!(unsafe { callback(1.25, 2.5) }, 3.75);
    }

    #[test]
    fn stub_saves_gp_and_xmm_arguments() {
        let bytes = stub_bytes(0x1122_3344_5566_7788, 0x99AA_BBCC_DDEE_FF00, 4);
        assert_eq!(&bytes[0..7], &[0x48, 0x81, 0xEC, 0x28, 0x04, 0, 0]);
        assert_eq!(&bytes[7..12], &[0x48, 0x89, 0x4C, 0x24, 0x20]);
        assert!(
            bytes
                .windows(9)
                .any(|window| { window == [0x66, 0x0F, 0xD6, 0x84, 0x24, 0x20, 0x02, 0, 0] })
        );
        assert!(
            bytes
                .windows(8)
                .any(|window| { window == 0x1122_3344_5566_7788u64.to_le_bytes() })
        );
        assert!(
            bytes
                .windows(8)
                .any(|window| { window == 0x99AA_BBCC_DDEE_FF00u64.to_le_bytes() })
        );
        assert_eq!(
            &bytes[bytes.len() - 9..],
            &[0xD0, 0x48, 0x81, 0xC4, 0x28, 0x04, 0, 0, 0xC3]
        );
        assert_eq!(bytes.last(), Some(&0xC3));
    }
}
