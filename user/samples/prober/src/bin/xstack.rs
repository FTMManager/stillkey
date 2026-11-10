//! Probe `xstack` (docs/10 section 9): puts a `ret` instruction on its stack
//! and calls it. The page is not executable, so the kernel must end the
//! program at the first instruction fetch.

#![no_std]
#![no_main]

use core::ptr::write_volatile;

use libstill::out;

libstill::entry!(main);

/// `ret`.
const RET: u8 = 0xC3;

fn main() -> i32 {
    let mut code = [0u8; 16];
    let addr = code.as_mut_ptr();
    // SAFETY: `addr` is writable memory of this program.
    unsafe { write_volatile(addr, RET) };
    out!("xstack addr={:#x}", addr as u64);
    // SAFETY: none: the call is the probe, and the kernel ends the program
    // at it.
    let call: extern "C" fn() = unsafe { core::mem::transmute(addr) };
    call();
    out!("xstack survived");
    1
}
