//! Probe `out` (docs/10 section 9): writes I/O port 0x21. Ring 3 may not do
//! that; the kernel must end this program with vector 13.

#![no_std]
#![no_main]

use core::arch::asm;

use libstill::out;

libstill::entry!(main);

fn main() -> i32 {
    // SAFETY: the instruction is the probe. The kernel ends the program at
    // it; if it does not, the next line says so and the test fails.
    unsafe { asm!("out 0x21, al", in("al") 0u8, options(nomem, nostack)) };
    out!("out survived");
    1
}
