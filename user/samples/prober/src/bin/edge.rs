//! Probe `edge` (docs/10 section 9): the edges of the system calls, then
//! a software interrupt that ring 3 may not raise.

#![no_std]
#![no_main]

use core::arch::asm;

use libstill::abi::Status;
use libstill::{debug_write, handle_close, mem_alloc, mem_free, out, syscall};

libstill::entry!(main);

const MIB: u64 = 1 << 20;

fn alloc(len: u64) -> &'static str {
    match mem_alloc(len) {
        Ok(_) => "OK",
        Err(status) => status.name(),
    }
}

fn main() -> i32 {
    let unknown = Status::of_return(syscall(999, [0; 5]).0).map_or("UNKNOWN", |status| status.name());
    out!("edge case=unknown status={}", unknown);
    out!("edge case=write0 status={}", debug_write(&[]).name());
    out!("edge case=alloc0 status={}", alloc(0));
    out!("edge case=allocbig status={}", alloc(MIB + 1));
    out!("edge case=freebad status={}", mem_free(0x2_0000_0000u64 as *mut u8, 4096).name());
    // The end of this range is past the top of the address space.
    out!("edge case=freewrap status={}", mem_free(0xFFFF_FFFF_FFFF_F000u64 as *mut u8, 4096).name());
    out!("edge case=close0 status={}", handle_close(0).name());
    // SAFETY: the instruction is the probe: the gate of vector 0x20 is not
    // open to ring 3, and the kernel ends the program here.
    unsafe { asm!("int 0x20", options(nomem, nostack)) };
    out!("edge survived");
    1
}
