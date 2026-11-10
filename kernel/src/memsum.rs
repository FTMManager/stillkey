//! Memory summary: what the map contains and what an allocator may use
//! (docs/02 section 7). Milestone 1 starts no allocator; it prints the numbers.

use stillkey_bootinfo::convert::direct_map_size;
use stillkey_bootinfo::{Accepted, kind_name};

use crate::log;

/// Prints the summary and returns the size of the direct map in bytes.
///
/// Every range was checked by `validate`: bases are page-aligned, ends do not
/// overflow and regions are disjoint, so the sums below cannot overflow.
pub fn summarise(info: &Accepted<'_>) -> u64 {
    for kind in 1..=11u32 {
        let (mut regions, mut pages) = (0u64, 0u64);
        for r in info.regions().filter(|r| r.kind == kind) {
            regions += 1;
            pages += r.pages;
        }
        if regions > 0 {
            let name = kind_name(kind).unwrap_or("unknown");
            log!("SK:KERNEL:MEM kind={name} regions={regions} pages={pages}");
        }
    }

    let (mut ranges, mut reserved_pages) = (0u64, 0u64);
    for r in info.reserved() {
        ranges += 1;
        reserved_pages += r.pages;
    }
    log!("SK:KERNEL:MEM reserved ranges={ranges} pages={reserved_pages}");

    // Kinds 1 and 2 without the first 1 MiB, the reserved ranges and the
    // framebuffer. The rule is in `stillkey-bootinfo`, where it is tested.
    log!("SK:KERNEL:MEM usable_after_reserve pages={}", info.usable_after_reserve());

    match direct_map_size(info.regions()) {
        Ok(bytes) => {
            log!("SK:KERNEL:MEM direct_map_gib={}", bytes >> 30);
            bytes
        }
        Err(_) => panic!("the memory map needs a direct map above 64 TiB"),
    }
}
