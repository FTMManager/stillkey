//! The table bounds, isolated.
//!
//! In every case here each byte of a table that lies inside the mapped block
//! is valid content. The only thing that can refuse the block is the bound: a
//! validator that checked the entries it could reach, or that trusted the
//! mapping or `total_size` alone, would accept these blocks.
//!
//! Each test first shows that the same content is accepted when the header
//! tells the truth, so a refusal cannot come from anything else.

mod common;

use common::*;
use stillkey_bootinfo::*;

const PAGE: usize = 4096;

#[test]
fn a_memory_map_one_entry_longer_than_the_block_is_refused_by_its_bound() {
    let spec = full_page();
    let honest = build(&spec, |_| {});
    assert_eq!(validate(&honest, BLOCK_PHYS).expect("a full page is valid").regions().count(), 116);

    // The header is unchanged apart from the count. All 116 entries inside
    // the block are valid; entry 117 would begin at the first byte past it.
    let block = build(&spec, |h| h.memory_map.count = 117);
    assert_eq!(block.len(), PAGE);
    assert_eq!(validate(&block, BLOCK_PHYS).unwrap_err(), Reject::MemoryMap);
}

/// A one-page block whose reserved table is its last 24 bytes and holds the
/// block's own entry; `count` is what the header claims.
fn reserved_table_at_the_end(count: u64) -> Vec<u8> {
    let at = PAGE - ReservedRange::SIZE;
    let mut block =
        build(&good(), |h| h.reserved = Table { phys: BLOCK_PHYS + at as u64, count });
    block[at..].copy_from_slice(&range(BLOCK_PHYS, 1, PURPOSE_BOOT_INFO).encode());
    block
}

#[test]
fn a_reserved_table_one_entry_longer_than_the_block_is_refused_by_its_bound() {
    let honest = reserved_table_at_the_end(1);
    let ok = validate(&honest, BLOCK_PHYS).expect("one entry fits");
    assert_eq!(ok.reserved().count(), 1);

    // Two entries: far below the limit of 32, and the one inside the block is
    // valid. The second would lie wholly past the block.
    let block = reserved_table_at_the_end(2);
    assert_eq!(validate(&block, BLOCK_PHYS).unwrap_err(), Reject::Reserved);
}

/// Two pages are mapped and reserved for the block, and the header says the
/// block is one page long.
fn two_pages_mapped(edit: impl FnOnce(&mut BootInfo)) -> (Spec, Vec<u8>) {
    let mut spec = good();
    spec.reserved[0].pages = 2;
    let mut block = build(&spec, edit);
    block.resize(2 * PAGE, 0);
    (spec, block)
}

#[test]
fn a_total_size_smaller_than_the_mapping_is_accepted() {
    let (_, block) = two_pages_mapped(|_| {});
    let ok = validate(&block, BLOCK_PHYS).expect("total_size may be less than what is mapped");
    assert_eq!(ok.header.total_size, PAGE as u64);
}

#[test]
fn a_command_line_past_total_size_is_refused_although_it_is_mapped() {
    let at = PAGE + 16;
    let (spec, mut block) = two_pages_mapped(|h| h.cmdline.phys = BLOCK_PHYS + at as u64);
    block[at..at + spec.cmdline.len()].copy_from_slice(&spec.cmdline);
    assert_eq!(validate(&block, BLOCK_PHYS).unwrap_err(), Reject::Cmdline);
}

#[test]
fn a_memory_map_past_total_size_is_refused_although_it_is_mapped() {
    let at = PAGE + 64;
    let (spec, mut block) = two_pages_mapped(|h| h.memory_map.phys = BLOCK_PHYS + at as u64);
    for (i, r) in spec.regions.iter().enumerate() {
        let to = at + i * MemoryRegion::SIZE;
        block[to..to + MemoryRegion::SIZE].copy_from_slice(&r.encode());
    }
    assert_eq!(validate(&block, BLOCK_PHYS).unwrap_err(), Reject::MemoryMap);
}

#[test]
fn a_reserved_table_past_total_size_is_refused_although_it_is_mapped() {
    let at = PAGE + 512;
    let (spec, mut block) = two_pages_mapped(|h| h.reserved.phys = BLOCK_PHYS + at as u64);
    for (i, r) in spec.reserved.iter().enumerate() {
        let to = at + i * ReservedRange::SIZE;
        block[to..to + ReservedRange::SIZE].copy_from_slice(&r.encode());
    }
    assert_eq!(validate(&block, BLOCK_PHYS).unwrap_err(), Reject::Reserved);
}
