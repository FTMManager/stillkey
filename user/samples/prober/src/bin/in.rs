//! Probe `in` (docs/10 section 9): reads I/O port 0x60. Ring 3 may not do
//! that; the kernel must end this program with vector 13.

#![no_std]
#![no_main]

use core::arch::asm;

use libstill::out;

libstill::entry!(main);

fn main() -> i32 {
    // SAFETY: the instruction is the probe. The kernel ends the program at
    // it; if it does not, the next line says so and the test fails.
    unsafe { asm!("in al, 0x60", out("al") _, options(nomem, nostack)) };
    out!("in survived");
    1
}
