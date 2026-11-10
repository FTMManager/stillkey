//! Probe `x87` (docs/10 section 9): executes `fninit`, given as bytes because the assembler of
//! a soft-float target may refuse the mnemonic. Ring 3 may not do
//! that; the kernel must end this program with vector 7.

#![no_std]
#![no_main]

use core::arch::asm;

use libstill::out;

libstill::entry!(main);

fn main() -> i32 {
    // SAFETY: the instruction is the probe. The kernel ends the program at
    // it; if it does not, the next line says so and the test fails.
    unsafe { asm!(".byte 0xdb, 0xe3", options(nomem, nostack)) };
    out!("x87 survived");
    1
}
