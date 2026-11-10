//! Probe `kread` (docs/10 section 9): reads the first byte of the kernel's
//! image. The kernel must end this program with a page fault at that address.

#![no_std]
#![no_main]

use core::arch::asm;

use libstill::out;

libstill::entry!(main);

/// Where the kernel's image starts.
const KERNEL: u64 = 0xFFFF_FFFF_8000_0000;

fn main() -> i32 {
    let value: u64;
    // SAFETY: the read is the probe. The kernel ends the program at it; if
    // it does not, the next line says so and the test fails.
    unsafe {
        asm!("mov {value}, [{addr}]", addr = in(reg) KERNEL, value = out(reg) value, options(nostack, readonly));
    }
    out!("kread value={:#x}", value);
    1
}
