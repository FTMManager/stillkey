//! Probe `memcap` (docs/10 section 9): asks for memory until the kernel
//! says no. The limit of a program must stop it, with `NO_MEMORY`, and the
//! program must still be able to use what it got.

#![no_std]
#![no_main]

use core::ptr::write_volatile;

use libstill::abi::Status;
use libstill::{mem_alloc, out};

libstill::entry!(main);

const MIB: u64 = 1 << 20;
/// Where the probe gives up on a kernel that never says no.
const TOO_MANY: u64 = 64;

fn main() -> i32 {
    let mut first: *mut u8 = core::ptr::null_mut();
    let mut last = first;
    let mut obtained = 0u64;
    let status = loop {
        match mem_alloc(MIB) {
            Ok(block) => {
                if obtained == 0 {
                    first = block;
                }
                last = block;
                obtained += 1;
            }
            Err(status) => break status,
        }
        if obtained == TOO_MANY {
            break Status::Ok;
        }
    };
    out!("memcap obtained_mib={} status={}", obtained, status.name());
    if obtained > 0 {
        // SAFETY: `first` and `last` each start a megabyte that `mem_alloc`
        // mapped for this program.
        unsafe {
            write_volatile(first, 1);
            write_volatile(last.add(MIB as usize - 1), 1);
        }
    }
    out!("memcap alive");
    0
}
