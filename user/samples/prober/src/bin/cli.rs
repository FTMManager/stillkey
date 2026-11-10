//! Probe `cli` (docs/10 section 9): executes `cli`. Ring 3 may not do
//! that; the kernel must end this program with vector 13.

#![no_std]
#![no_main]

use core::arch::asm;

use libstill::out;

libstill::entry!(main);

fn main() -> i32 {
    // SAFETY: the instruction is the probe. The kernel ends the program at
    // it; if it does not, the next line says so and the test fails.
    unsafe { asm!("cli", options(nomem, nostack)) };
    out!("cli survived");
    1
}
