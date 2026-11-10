//! Step 9: the final `GetMemoryMap` / `ExitBootServices` sequence
//! (docs/02 section 8), on the raw boot-services table.
//!
//! The rules the code must make obvious:
//! 1. Between a successful `GetMemoryMap` and the `ExitBootServices` call
//!    that uses its key there is no firmware call. Logging is port I/O.
//! 2. The loop is bounded: eight passes, at most one exit call in each.
//! 3. After success nothing allocates.
//! 4. A failure before the first exit call returns to firmware; after one,
//!    the loader halts.
//! 5. `size` is reset to the buffer capacity before every `GetMemoryMap`.
//! 6. Only the raw-map buffer is ever reallocated.

use core::ptr;

use uefi::Status;

use crate::{Stop, cpu, log, mem, qemu};

const MAX_PASSES: u32 = 8;

pub struct Exited {
    /// `ExitBootServices` calls made; the last one succeeded.
    pub calls: u32,
    /// The raw UEFI descriptors of the final map.
    pub map: *const u8,
    pub size: usize,
    pub desc_size: usize,
}

/// Rule 4.
fn stop_or_halt(calls: u32) -> Result<Exited, Stop> {
    if calls == 0 {
        return Err(Stop);
    }
    qemu::fail();
    cpu::halt()
}

macro_rules! fail {
    ($calls:expr, $($arg:tt)*) => {{
        log!($($arg)*);
        return stop_or_halt($calls);
    }};
}

/// A buffer for a map of `size` bytes, with slack for the map growing.
fn map_buffer(size: usize, desc_size: usize) -> Option<(*mut u8, usize)> {
    let cap = desc_size.checked_mul(mem::SLACK_DESCRIPTORS)?.checked_add(size)?;
    let buf = mem::alloc_zeroed(mem::pages_for(cap as u64)?)?;
    Some((buf as *mut u8, cap))
}

/// Leaves boot services. `desc_size` is the descriptor size of the
/// preliminary map and `region_cap` the capacity of the region array.
/// `stale_key_test` is the boot option `test=stalekey`.
pub fn exit_boot_services(
    desc_size: usize,
    region_cap: usize,
    stale_key_test: bool,
) -> Result<Exited, Stop> {
    let mut desc_size = desc_size;
    let mut key = 0;
    let mut calls = 0;

    let mut needed = 0;
    let status = mem::get_memory_map(ptr::null_mut(), &mut needed, &mut key, &mut desc_size);
    if status != Status::BUFFER_TOO_SMALL {
        fail!(calls, "SK:LOADER:EBS_FAILED reason=map status={:#x}", status.0);
    }
    let Some((mut map_buf, mut cap)) = map_buffer(needed, desc_size) else {
        fail!(calls, "SK:LOADER:NO_MEMORY what=map");
    };
    log!("SK:LOADER:EBS_BEGIN");

    for _pass in 0..MAX_PASSES {
        let mut size = cap; // rule 5
        let status = mem::get_memory_map(map_buf, &mut size, &mut key, &mut desc_size);
        if status == Status::BUFFER_TOO_SMALL {
            // `size` now holds the bytes needed. The old buffer is left behind.
            let Some(larger) = map_buffer(size, desc_size) else {
                fail!(calls, "SK:LOADER:NO_MEMORY what=map");
            };
            (map_buf, cap) = larger;
            continue;
        }
        if status != Status::SUCCESS {
            fail!(calls, "SK:LOADER:EBS_FAILED reason=map status={:#x}", status.0);
        }
        if desc_size == 0 || size / desc_size > region_cap {
            fail!(calls, "SK:LOADER:NO_MEMORY what=regions");
        }
        if stale_key_test && calls == 0 {
            // Test only: breaks rule 1 on purpose, so the key is stale.
            let _ = mem::alloc_zeroed(1);
        }
        calls += 1;
        let status = mem::exit_boot_services(key); // no firmware call since GetMemoryMap
        if status == Status::SUCCESS {
            return Ok(Exited { calls, map: map_buf, size, desc_size });
        }
        if status != Status::INVALID_PARAMETER {
            fail!(calls, "SK:LOADER:EBS_FAILED reason=exit status={:#x}", status.0);
        }
        log!("SK:LOADER:EBS_RETRY n={}", calls); // stale key: fetch the map again
    }
    fail!(calls, "SK:LOADER:EBS_FAILED attempts={}", calls);
}
