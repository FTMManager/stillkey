//! Probe `badptr` (docs/10 section 9): hands `debug_write` four buffers
//! the kernel must refuse, the last one with `EFLAGS.AC` set, which would
//! switch off SMAP for a kernel that does not clear it.

#![no_std]
#![no_main]

use core::arch::asm;

use libstill::abi::{SYS_DEBUG_WRITE, Status};
use libstill::{mem_alloc, out, syscall};

libstill::entry!(main);

/// Where the kernel's image starts.
const KERNEL: u64 = 0xFFFF_FFFF_8000_0000;
const PAGE: u64 = 4096;

fn name(ret: u64) -> &'static str {
    Status::of_return(ret).map_or("UNKNOWN", |status| status.name())
}

fn write(addr: u64, len: u64) -> u64 {
    syscall(SYS_DEBUG_WRITE, [addr, len, 0, 0, 0]).0
}

/// The same call with `EFLAGS.AC` (bit 18) set at the `int 0x80`.
fn write_with_ac(addr: u64, len: u64) -> u64 {
    let ret: u64;
    // SAFETY: ring 3 may set and clear AC. The flag is cleared again before
    // any other code of this program runs. The rest is `libstill::syscall`:
    // the kernel takes its arguments from these registers and returns here.
    unsafe {
        asm!(
            "pushfq",
            "or qword ptr [rsp], 0x40000",
            "popfq",
            "int 0x80",
            "pushfq",
            "and qword ptr [rsp], -0x40001",
            "popfq",
            inlateout("rax") SYS_DEBUG_WRITE => ret,
            in("rdi") addr,
            in("rsi") len,
            inlateout("rdx") 0u64 => _,
            in("r10") 0u64,
            in("r8") 0u64,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
    ret
}

fn main() -> i32 {
    out!("badptr case=kernel status={}", name(write(KERNEL, 16)));
    let Ok(page) = mem_alloc(PAGE) else {
        return 1;
    };
    let page = page as u64;
    // The page after this one is not mapped.
    out!("badptr case=straddle status={}", name(write(page + PAGE - 8, 16)));
    out!("badptr case=long status={}", name(write(page, 257)));
    out!("badptr case=ac status={}", name(write_with_ac(KERNEL, 16)));
    0
}
