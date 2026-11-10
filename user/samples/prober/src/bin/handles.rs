//! Probe `handles` (docs/10 section 9): a program without handles tries
//! 8192 handle values, twice each. Every call must be refused with
//! `BAD_HANDLE`.

#![no_std]
#![no_main]

use libstill::abi::Status;
use libstill::abi::diag::KIND_PROCESSES;
use libstill::{diag_read, handle_close, out};

libstill::entry!(main);

fn main() -> i32 {
    let mut buf = [0u8; 64];
    let (mut bad, mut other) = (0u32, 0u32);
    let mut probe = |handle: u32| {
        let read = diag_read(handle, KIND_PROCESSES, 0, buf.as_mut_ptr(), buf.len()).0;
        let closed = handle_close(handle);
        let refused =
            [matches!(Status::of_return(read), Some(Status::BadHandle)), matches!(closed, Status::BadHandle)];
        for refused in refused {
            if refused {
                bad += 1;
            } else {
                other += 1;
            }
        }
    };
    for handle in 0..4096 {
        probe(handle);
    }
    // xorshift32.
    let mut x: u32 = 0x2545_F491;
    for _ in 0..4096 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        probe(x);
    }
    out!("handles calls=16384 bad_handle={} other={}", bad, other);
    0
}
