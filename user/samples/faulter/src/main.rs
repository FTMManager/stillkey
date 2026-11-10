//! Runs for three seconds, then writes to address 0 (docs/10 section 9).
//! The kernel must end this program and nothing else.

#![no_std]
#![no_main]

use core::arch::asm;

use libstill::{out, sleep};

libstill::entry!(main);

const WAIT_NS: u64 = 3_000_000_000;

fn main() -> i32 {
    out!("faulter start");
    sleep(WAIT_NS);
    // SAFETY: none is claimed: the write must fault, because no program has
    // a page at address 0.
    unsafe { asm!("mov qword ptr [0], 1", options(nostack)) };
    1
}
