//! Probe `hlt` (docs/10 section 9): executes `hlt`. Ring 3 may not do
//! that; the kernel must end this program with vector 13.

#![no_std]
#![no_main]

use core::arch::asm;

use libstill::out;

libstill::entry!(main);

fn main() -> i32 {
    // SAFETY: the instruction is the probe. The kernel ends the program at
    // it; if it does not, the next line says so and the test fails.
    unsafe { asm!("hlt", options(nomem, nostack)) };
    out!("hlt survived");
    1
}
