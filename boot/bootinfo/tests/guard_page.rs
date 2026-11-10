//! Regression: a valid header whose table extends beyond the mapped block is
//! refused before the table is touched.
//!
//! The block is placed in the last readable page before a page with no access
//! rights. The header is valid; the table it describes reaches into the
//! inaccessible page. Every entry of the table that lies inside the block is
//! valid, up to the block's last byte, so a reader that walked the entries
//! would not stop on bad content: it would walk into the inaccessible page,
//! fault, and kill the test process. Validation must return the refusal
//! instead, which it can only do from the table's bound.

#![cfg(target_os = "linux")]

mod common;

use common::*;
use stillkey_bootinfo::*;

use core::ffi::{c_int, c_void};

const PROT_NONE: c_int = 0;
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_PRIVATE: c_int = 2;
const MAP_ANONYMOUS: c_int = 0x20;
const PAGE: usize = 4096;

unsafe extern "C" {
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i64)
    -> *mut c_void;
    fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
}

/// One readable page followed by one page that faults on any access.
struct Guarded {
    base: *mut u8,
}

impl Guarded {
    fn new(contents: &[u8]) -> Guarded {
        assert_eq!(contents.len(), PAGE);
        // SAFETY: an anonymous private mapping of two pages; the result is checked.
        let base = unsafe {
            mmap(
                core::ptr::null_mut(),
                2 * PAGE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        } as *mut u8;
        assert!(!base.is_null() && base as isize != -1, "mmap failed");
        // SAFETY: the first page is mapped read-write and is PAGE bytes long;
        // the second page lies inside the mapping made above.
        unsafe {
            core::ptr::copy_nonoverlapping(contents.as_ptr(), base, PAGE);
            assert_eq!(mprotect(base.add(PAGE).cast(), PAGE, PROT_NONE), 0, "mprotect failed");
        }
        Guarded { base }
    }

    /// Exactly the mapped block: the first page and nothing more.
    fn block(&self) -> &[u8] {
        // SAFETY: the first page is mapped, readable, initialised and lives as long as `self`.
        unsafe { core::slice::from_raw_parts(self.base, PAGE) }
    }
}

impl Drop for Guarded {
    fn drop(&mut self) {
        // SAFETY: unmaps exactly the mapping made in `new`.
        unsafe {
            munmap(self.base.cast(), 2 * PAGE);
        }
    }
}

#[test]
fn a_valid_block_at_the_guard_page_is_accepted() {
    let guarded = Guarded::new(&build(&good(), |_| {}));
    let ok = validate(guarded.block(), BLOCK_PHYS).expect("valid block");
    assert_eq!(ok.regions().count(), 6);
}

#[test]
fn a_table_reaching_into_the_unmapped_page_is_rejected_without_a_fault() {
    // 116 valid regions fill the page to its last byte; the header claims 63
    // more, which would end 2,016 bytes inside the inaccessible page.
    let spec = full_page();
    let count = spec.regions.len() as u64 + 63;
    let guarded = Guarded::new(&build(&spec, |h| h.memory_map.count = count));

    // The header alone is valid, and so is every entry inside the block: the
    // same bytes with the true count pass.
    let mut honest = guarded.block().to_vec();
    let mut header = BootInfo::decode(honest[..HEADER_SIZE].try_into().unwrap());
    header.memory_map.count = spec.regions.len() as u64;
    honest[..HEADER_SIZE].copy_from_slice(&header.encode());
    assert_eq!(validate(&honest, BLOCK_PHYS).expect("honest count").regions().count(), 116);

    assert_eq!(validate(guarded.block(), BLOCK_PHYS).unwrap_err(), Reject::MemoryMap);
}

#[test]
fn the_reserved_and_command_line_tables_get_the_same_protection() {
    // The reserved table is the last 24 bytes of the page and holds the
    // block's own, valid entry. The header claims two entries, far below the
    // limit of 32; the second would lie in the inaccessible page.
    let at = PAGE - ReservedRange::SIZE;
    let mut block =
        build(&good(), |h| h.reserved = Table { phys: BLOCK_PHYS + at as u64, count: 2 });
    block[at..].copy_from_slice(&range(BLOCK_PHYS, 1, PURPOSE_BOOT_INFO).encode());
    let guarded = Guarded::new(&block);
    assert_eq!(validate(guarded.block(), BLOCK_PHYS).unwrap_err(), Reject::Reserved);

    let guarded = Guarded::new(&build(&good(), |h| {
        h.cmdline.phys = BLOCK_PHYS + PAGE as u64 - 8;
        h.cmdline.count = 64;
    }));
    assert_eq!(validate(guarded.block(), BLOCK_PHYS).unwrap_err(), Reject::Cmdline);
}
