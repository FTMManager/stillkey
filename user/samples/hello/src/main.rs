//! The smallest Stillkey program (docs/10 section 9): it prints, waits until
//! the clock has advanced by three timer ticks, prints again and ends.
//!
//! The wait makes no progress unless timer interrupts are taken while this
//! program runs in ring 3 and control comes back to it afterwards.

#![no_std]
#![no_main]

use libstill::{abi_version, clock, out};

libstill::entry!(main);

/// Three ticks of the 100 Hz timer.
const WAIT_NS: u64 = 30_000_000;

fn main() -> i32 {
    out!("hello from ring 3 abi={}", abi_version());
    let start = clock();
    let mut now = start;
    while now - start < WAIT_NS {
        now = clock();
    }
    out!("hello ticks={}", (now - start) / libstill::abi::TICK_NS);
    0
}
