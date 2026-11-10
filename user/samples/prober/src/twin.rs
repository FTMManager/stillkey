//! The source of the probes `twin-a` and `twin-b` (docs/10 section 9): two
//! programs get a page at the same address and each must find only its own
//! bytes in it. `twin-b` then gives the page back and reads it, which must
//! end it without disturbing `twin-a`.

use core::ptr::{read_volatile, write_volatile};

use libstill::abi::Status;
use libstill::{mem_alloc, mem_free, out, sleep};

const PAGE: usize = 4096;
const SECOND_NS: u64 = 1_000_000_000;

/// Whose bytes the page holds.
fn verdict(page: *const u8, own: u8, other: u8) -> &'static str {
    let (mut mine, mut theirs) = (0, 0);
    for at in 0..PAGE {
        // SAFETY: `page` is `PAGE` bytes that `mem_alloc` mapped for this
        // program.
        let byte = unsafe { read_volatile(page.add(at)) };
        if byte == own {
            mine += 1;
        } else if byte == other {
            theirs += 1;
        }
    }
    if mine == PAGE {
        "own"
    } else if theirs == PAGE {
        "other"
    } else {
        "mixed"
    }
}

pub fn run(own: u8, other: u8, frees: bool) -> i32 {
    let page = match mem_alloc(PAGE as u64) {
        Ok(page) => page,
        Err(status) => {
            out!("twin alloc={}", status.name());
            return 1;
        }
    };
    out!("twin addr={:#x}", page as u64);
    for at in 0..PAGE {
        // SAFETY: as in `verdict`.
        unsafe { write_volatile(page.add(at), own) };
    }
    sleep(2 * SECOND_NS);
    out!("twin readback={}", verdict(page, own, other));
    if frees {
        match mem_free(page, PAGE as u64) {
            Status::Ok => out!("twin freed"),
            status => out!("twin free={}", status.name()),
        }
        // SAFETY: none: the page is gone, and this read is the probe. The
        // kernel ends the program here.
        let byte = unsafe { read_volatile(page) };
        out!("twin survived byte={:#x}", byte);
        return 1;
    }
    sleep(SECOND_NS);
    out!("twin final={}", verdict(page, own, other));
    0
}
