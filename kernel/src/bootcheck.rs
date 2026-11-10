//! Accepting the handoff block (docs/02 section 6).
//!
//! The checks themselves are in `stillkey_bootinfo::validate`, which is pure,
//! contains no `unsafe` and is tested on the host. This file holds the one
//! step that cannot be: turning the two register values into a byte slice.

use stillkey_bootinfo::{Accepted, Reject, check_block, validate};

/// Validates the block the loader described in `RDI` (address) and `RSI`
/// (length). Nothing is read through `block` until `check_block` has passed,
/// and no table is read until its bounds inside the block have been checked.
pub fn accept(block: *const u8, len: u64) -> Result<Accepted<'static>, Reject> {
    let phys = check_block(block as u64, len)?;
    // SAFETY: `check_block` established that `block` is non-null and
    // page-aligned, that `len` is a non-zero number of whole pages no larger
    // than `MAX_BLOCK` (so it fits `usize` and `isize`), and that the whole
    // range lies in the first 4 GiB of the direct map. The loader's page
    // tables always map that part of the direct map, readable, whatever the
    // memory map contains (docs/02 section 5). The bytes are plain `u8`, so
    // any contents are valid. Nothing writes the block: one CPU, interrupts
    // off, and the kernel never frees or reuses it in this milestone, which
    // is what makes the `'static` lifetime true.
    let bytes: &'static [u8] = unsafe { core::slice::from_raw_parts(block, len as usize) };
    validate(bytes, phys)
}
