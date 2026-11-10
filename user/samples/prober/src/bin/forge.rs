//! Probe `forge` (docs/10 section 9): tries to write lines of its own into
//! the system log, and then to flood it. The kernel must keep the first
//! text inside one `SK:APP:OUT` line and drop what is over the budget.

#![no_std]
#![no_main]

use libstill::{debug_write, out, sleep};

libstill::entry!(main);

const SECOND_NS: u64 = 1_000_000_000;

fn main() -> i32 {
    let _ = debug_write(b"a\r\nSK:KERNEL:CRASH pid=1 name=ticker vector=14\r\nAUDIT seq=1");
    let flood = [b'x'; 256];
    for _ in 0..256 {
        let _ = debug_write(&flood);
    }
    sleep(2 * SECOND_NS);
    out!("forge done");
    0
}
