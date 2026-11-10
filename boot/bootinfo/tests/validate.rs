//! The acceptance rule of docs/02 section 6, case by case.

mod common;

use common::*;
use stillkey_bootinfo::*;

fn rejects_header(edit: impl FnOnce(&mut BootInfo)) -> Reject {
    let block = build(&good(), edit);
    validate(&block, BLOCK_PHYS).expect_err("the edited header must be rejected")
}

fn rejects_spec(change: impl FnOnce(&mut Spec)) -> Reject {
    let mut spec = good();
    change(&mut spec);
    let block = build(&spec, |_| {});
    validate(&block, BLOCK_PHYS).expect_err("the changed tables must be rejected")
}

#[test]
fn accepts_a_block_the_loader_would_build() {
    let spec = good();
    let block = build(&spec, |_| {});
    let ok = validate(&block, BLOCK_PHYS).expect("valid block");
    assert_eq!(ok.header.magic, MAGIC);
    assert_eq!(ok.header.total_size, 4096);
    assert_eq!(ok.regions().collect::<Vec<_>>(), spec.regions);
    assert_eq!(ok.reserved().collect::<Vec<_>>(), spec.reserved);
    assert_eq!(ok.cmdline(), b"qemu_exit=on");
}

#[test]
fn accepts_a_block_with_no_command_line() {
    let mut spec = good();
    spec.cmdline.clear();
    let block = build(&spec, |_| {});
    let ok = validate(&block, BLOCK_PHYS).expect("valid block");
    assert_eq!(ok.cmdline(), b"");
    assert_eq!(ok.header.flags & FLAG_CMDLINE_PRESENT, 0);
}

// ---- The regression this checkpoint asks for -------------------------------

/// The header passes every header check, but the memory-map table it
/// describes runs past the end of the mapped block. The slice handed to
/// `validate` is exactly the mapped block, so any read of the table's tail
/// would be out of bounds; the crate forbids `unsafe`, so such a read would
/// panic and fail this test instead of returning the refusal.
#[test]
fn a_table_that_runs_past_the_mapped_block_is_rejected() {
    let block = build(&good(), |h| h.memory_map.count = 4096 / 32);
    assert_eq!(block.len(), 4096);
    assert_eq!(validate(&block, BLOCK_PHYS).unwrap_err(), Reject::MemoryMap);
}

/// The same refusal when the header also claims a larger block than is
/// mapped: a self-reported `total_size` does not widen what may be read.
#[test]
fn a_total_size_larger_than_the_mapped_block_is_rejected() {
    let block = build(&good(), |h| {
        h.total_size = 2 * 4096;
        h.memory_map.count = 4096 / 32;
    });
    assert_eq!(validate(&block, BLOCK_PHYS).unwrap_err(), Reject::Size);
}

#[test]
fn every_table_is_held_to_the_mapped_block() {
    assert_eq!(rejects_header(|h| h.reserved.count = 4096 / 24), Reject::Reserved);
    assert_eq!(
        rejects_header(|h| {
            h.cmdline.phys = BLOCK_PHYS + 4096 - 4;
            h.cmdline.count = 8;
        }),
        Reject::Cmdline
    );
    // A table that starts past the block.
    assert_eq!(rejects_header(|h| h.memory_map.phys = BLOCK_PHYS + 4096), Reject::MemoryMap);
    // A table held to total_size even when more is mapped.
    let mut block = build(&good(), |h| h.memory_map.count = 4096 / 32);
    block.resize(2 * 4096, 0);
    assert_eq!(validate(&block, BLOCK_PHYS).unwrap_err(), Reject::MemoryMap);
}

// ---- Header ----------------------------------------------------------------

#[test]
fn rejects_a_block_too_short_for_a_header() {
    let block = build(&good(), |_| {});
    assert_eq!(validate(&block[..255], BLOCK_PHYS).unwrap_err(), Reject::Block);
    assert_eq!(validate(&[], BLOCK_PHYS).unwrap_err(), Reject::Block);
}

#[test]
fn rejects_a_wrong_magic_or_major_version() {
    assert_eq!(rejects_header(|h| h.magic ^= 1), Reject::Magic);
    assert_eq!(rejects_header(|h| h.version_major = 2), Reject::Version);
    assert_eq!(rejects_header(|h| h.version_major = 0), Reject::Version);
}

#[test]
fn accepts_a_newer_minor_version() {
    let block = build(&good(), |h| h.version_minor = 7);
    assert!(validate(&block, BLOCK_PHYS).is_ok());
}

#[test]
fn rejects_inconsistent_sizes() {
    assert_eq!(rejects_header(|h| h.header_size = 248), Reject::Size);
    assert_eq!(rejects_header(|h| h.header_size = 260), Reject::Size); // not a multiple of 8
    assert_eq!(rejects_header(|h| h.header_size = 8192), Reject::Size); // above total_size
    assert_eq!(rejects_header(|h| h.total_size = 255), Reject::Size);
    assert_eq!(rejects_header(|h| h.total_size = u64::MAX), Reject::Size);
}

#[test]
fn rejects_a_block_that_does_not_say_boot_services_were_exited() {
    assert_eq!(rejects_header(|h| h.flags &= !FLAG_BOOT_SERVICES_EXITED), Reject::Flags);
}

#[test]
fn rejects_a_direct_map_base_the_kernel_was_not_built_for() {
    assert_eq!(rejects_header(|h| h.direct_map_virt_base = 0xFFFF_9000_0000_0000), Reject::Layout);
}

#[test]
fn the_first_failing_check_gives_the_reason() {
    assert_eq!(
        rejects_header(|h| {
            h.magic = 0;
            h.version_major = 9;
            h.total_size = 0;
        }),
        Reject::Magic
    );
    assert_eq!(
        rejects_header(|h| {
            h.flags = 0;
            h.memory_map.count = u64::MAX;
        }),
        Reject::Flags
    );
}

// ---- Table descriptors: counts, lengths and address ranges ------------------

#[test]
fn rejects_table_arithmetic_that_overflows() {
    assert_eq!(rejects_header(|h| h.memory_map.count = u64::MAX), Reject::MemoryMap);
    assert_eq!(rejects_header(|h| h.memory_map.count = u64::MAX / 32 + 1), Reject::MemoryMap);
    assert_eq!(rejects_header(|h| h.memory_map.count = (1 << 59) + 6), Reject::MemoryMap); // wraps to 192 bytes
    assert_eq!(rejects_header(|h| h.reserved.count = u64::MAX / 24 + 1), Reject::Reserved);
    assert_eq!(rejects_header(|h| h.memory_map.phys = u64::MAX - 7), Reject::MemoryMap);
    assert_eq!(rejects_header(|h| h.cmdline.phys = u64::MAX), Reject::Cmdline);
}

#[test]
fn rejects_a_table_outside_the_body_of_the_block() {
    // Below the block.
    assert_eq!(rejects_header(|h| h.memory_map.phys = BLOCK_PHYS - 4096), Reject::MemoryMap);
    assert_eq!(rejects_header(|h| h.memory_map.phys = 0), Reject::MemoryMap);
    // Inside the header.
    assert_eq!(rejects_header(|h| h.reserved.phys = BLOCK_PHYS + 8), Reject::Reserved);
    assert_eq!(rejects_header(|h| h.cmdline.phys = BLOCK_PHYS + 255), Reject::Cmdline);
}

#[test]
fn rejects_a_misaligned_table() {
    assert_eq!(rejects_header(|h| h.memory_map.phys += 4), Reject::MemoryMap);
    assert_eq!(rejects_header(|h| h.reserved.phys += 1), Reject::Reserved);
}

#[test]
fn rejects_tables_that_overlap_each_other() {
    assert_eq!(rejects_header(|h| h.reserved.phys = h.memory_map.phys), Reject::Reserved);
    assert_eq!(rejects_header(|h| h.cmdline.phys = h.reserved.phys), Reject::Cmdline);
}

// ---- Memory map contents ----------------------------------------------------

#[test]
fn rejects_an_empty_memory_map() {
    assert_eq!(rejects_header(|h| h.memory_map.count = 0), Reject::MemoryMap);
}

#[test]
fn rejects_malformed_regions() {
    assert_eq!(rejects_spec(|s| s.regions[1].base += 0x800), Reject::MemoryMap); // not page-aligned
    assert_eq!(rejects_spec(|s| s.regions[5].pages = 0), Reject::MemoryMap);
    assert_eq!(rejects_spec(|s| s.regions[5].pages = 1 << 52), Reject::MemoryMap); // bytes overflow
    assert_eq!(rejects_spec(|s| s.regions[5].base = u64::MAX & !0xFFF), Reject::MemoryMap); // end overflows
    assert_eq!(rejects_spec(|s| s.regions[0].kind = 0), Reject::MemoryMap);
    assert_eq!(rejects_spec(|s| s.regions[0].kind = 12), Reject::MemoryMap);
}

#[test]
fn rejects_regions_that_are_unsorted_or_overlap() {
    assert_eq!(rejects_spec(|s| s.regions.swap(3, 4)), Reject::MemoryMap);
    assert_eq!(rejects_spec(|s| s.regions[1].pages += 1), Reject::MemoryMap); // runs into the loader region
}

// ---- Reserved ranges ---------------------------------------------------------

#[test]
fn rejects_malformed_reserved_ranges() {
    assert_eq!(rejects_spec(|s| s.reserved[1].base += 0x10), Reject::Reserved);
    assert_eq!(rejects_spec(|s| s.reserved[1].pages = 0), Reject::Reserved);
    assert_eq!(rejects_spec(|s| s.reserved[4].pages = u64::MAX), Reject::Reserved);
    assert_eq!(rejects_spec(|s| s.reserved[1].purpose = 0), Reject::Reserved);
    assert_eq!(rejects_spec(|s| s.reserved[1].purpose = 7), Reject::Reserved);
    assert_eq!(rejects_spec(|s| s.reserved.swap(1, 2)), Reject::Reserved); // unsorted
    assert_eq!(rejects_spec(|s| s.reserved[1].pages = 0x101), Reject::Reserved); // overlaps the next
}

#[test]
fn rejects_a_reserved_range_outside_loader_memory() {
    // Wholly in usable memory.
    assert_eq!(
        rejects_spec(|s| s.reserved.insert(0, range(0x20_0000, 4, PURPOSE_KERNEL_IMAGE))),
        Reject::Reserved
    );
    // Starts in the loader region and runs into boot-services memory.
    assert_eq!(rejects_spec(|s| s.reserved[4].pages = 0x900), Reject::Reserved);
    // In a hole the map does not describe.
    assert_eq!(
        rejects_spec(|s| s.reserved.push(range(0x4000_0000, 1, PURPOSE_BOOTFS))),
        Reject::Reserved
    );
}

#[test]
fn accepts_a_reserved_range_that_spans_adjacent_loader_regions() {
    let mut spec = good();
    // Split the loader region in two where the loader image lies.
    spec.regions[2] = region(0x0100_0000, 0x810, KIND_LOADER);
    spec.regions.insert(3, region(0x0181_0000, 0x7F0, KIND_LOADER));
    let block = build(&spec, |_| {});
    assert!(validate(&block, BLOCK_PHYS).is_ok());
}

#[test]
fn the_block_itself_must_be_reserved_exactly_once() {
    // No entry for the block.
    assert_eq!(rejects_spec(|s| { s.reserved.remove(0); }), Reject::Reserved);
    // An entry of the wrong size.
    assert_eq!(rejects_spec(|s| s.reserved[0].pages = 2), Reject::Reserved);
    // A second entry with the same purpose.
    assert_eq!(
        rejects_spec(|s| s.reserved.push(range(0x01A0_0000, 1, PURPOSE_BOOT_INFO))),
        Reject::Reserved
    );
    // The entry describes another address than the one the block was found at.
    let block = build(&good(), |_| {});
    assert_eq!(validate(&block, BLOCK_PHYS + 4096).unwrap_err(), Reject::MemoryMap);
}

#[test]
fn rejects_more_reserved_ranges_than_the_limit() {
    let got = rejects_spec(|s| {
        s.reserved.truncate(1);
        for i in 0..MAX_RESERVED {
            s.reserved.push(range(0x0110_0000 + i * 0x1000, 1, PURPOSE_KERNEL_IMAGE));
        }
    });
    assert_eq!(got, Reject::Reserved);
}

// ---- Command line -------------------------------------------------------------

#[test]
fn rejects_a_command_line_longer_than_the_limit() {
    assert_eq!(rejects_spec(|s| s.cmdline = vec![b'a'; 1025]), Reject::Cmdline);
    let mut spec = good();
    spec.cmdline = vec![b'a'; 1024];
    assert!(validate(&build(&spec, |_| {}), BLOCK_PHYS).is_ok());
}

#[test]
fn the_command_line_flag_must_match_the_table() {
    assert_eq!(rejects_header(|h| h.flags &= !FLAG_CMDLINE_PRESENT), Reject::Cmdline);
    assert_eq!(rejects_header(|h| h.cmdline.count = 0), Reject::Cmdline);
}

// ---- The pointer and length passed in registers --------------------------------

#[test]
fn check_block_accepts_a_page_aligned_block_low_in_the_direct_map() {
    assert_eq!(check_block(DIRECT_MAP_BASE + BLOCK_PHYS, 4096), Ok(BLOCK_PHYS));
    assert_eq!(check_block(DIRECT_MAP_BASE + 0x1000, MAX_BLOCK), Ok(0x1000));
    // Ends exactly at 4 GiB.
    assert_eq!(check_block(DIRECT_MAP_BASE + (4u64 << 30) - 4096, 4096), Ok((4u64 << 30) - 4096));
}

#[test]
fn check_block_rejects_a_pointer_or_length_it_cannot_vouch_for() {
    let ok = DIRECT_MAP_BASE + BLOCK_PHYS;
    for (virt, len) in [
        (0, 4096),                                  // null
        (BLOCK_PHYS, 4096),                         // a physical address, not in the direct map
        (DIRECT_MAP_BASE, 4096),                    // physical page 0
        (ok + 8, 4096),                             // not page-aligned
        (ok, 0),                                    // empty
        (ok, 4095),                                 // not whole pages
        (ok, 4097),
        (ok, MAX_BLOCK + 4096),                     // larger than any block
        (DIRECT_MAP_BASE + (4u64 << 30), 4096),     // starts at 4 GiB
        (DIRECT_MAP_BASE + (4u64 << 30) - 4096, 8192), // crosses 4 GiB
        (0xFFFF_FFFF_8000_0000, 4096),              // the kernel window
        (u64::MAX & !0xFFF, 4096),                  // end overflows
        (ok, u64::MAX & !0xFFF),
    ] {
        assert_eq!(check_block(virt, len), Err(Reject::Block), "virt={virt:#x} len={len:#x}");
    }
}

#[test]
fn reason_words_match_the_kernel_log() {
    let words: Vec<&str> = [
        Reject::Block, Reject::Magic, Reject::Version, Reject::Size, Reject::Flags,
        Reject::Layout, Reject::MemoryMap, Reject::Reserved, Reject::Cmdline,
    ]
    .iter()
    .map(|r| r.as_str())
    .collect();
    assert_eq!(
        words,
        ["block", "magic", "version", "size", "flags", "layout", "memory_map", "reserved", "cmdline"]
    );
}
