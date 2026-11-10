//! Prints a line every second, for ever (docs/10 section 9). The other
//! programs of a scenario must not stop it.

#![no_std]
#![no_main]

use libstill::{out, sleep};

libstill::entry!(main);

const SECOND_NS: u64 = 1_000_000_000;

fn main() -> i32 {
    let mut n = 0u64;
    loop {
        sleep(SECOND_NS);
        n += 1;
        out!("tick {}", n);
    }
}
