//! Probe `wtext` (docs/10 section 9): writes to its own code. The page is
//! read-only for the program, so the kernel must end it there.

#![no_std]
#![no_main]

use core::ptr::write_volatile;

use libstill::out;

libstill::entry!(main);

fn main() -> i32 {
    let addr = _start as *const () as *mut u8;
    out!("wtext addr={:#x}", addr as u64);
    // SAFETY: none: the write is the probe, and the kernel ends the program
    // at it.
    unsafe { write_volatile(addr, 0x90) };
    out!("wtext survived");
    1
}
