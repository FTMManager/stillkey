//! Prints one line, then never makes a system call again (docs/10 section
//! 9). Only the timer can take the processor from this program.

#![no_std]
#![no_main]

use core::arch::asm;

use libstill::out;

libstill::entry!(main);

fn main() -> i32 {
    out!("spinner start");
    // SAFETY: a jump to itself touches no memory and no stack.
    unsafe { asm!("2:", "jmp 2b", options(noreturn, nomem, nostack)) }
}
