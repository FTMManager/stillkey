//! The `bootfs` row of the handoff validation (docs/10 section 5.3): the
//! image the loader read is described by the header and protected by exactly
//! one reserved range that covers it.

mod common;

use common::*;
use stillkey_bootinfo::*;

/// Inside the loader region of `good()`, between the page tables and the
/// loader image.
const BASE: u64 = 0x0140_0000;
/// Three pages and a little: four pages.
const SIZE: u64 = 3 * 4096 + 100;

fn image() -> PhysRange {
    PhysRange { base: BASE, size: SIZE }
}

fn spec_with_range(base: u64, pages: u64) -> Spec {
    let mut spec = good();
    spec.reserved.insert(4, range(base, pages, PURPOSE_BOOTFS)); // sorted by base
    spec
}

fn with(spec: &Spec, bootfs: PhysRange) -> Vec<u8> {
    build(spec, |h| {
        h.flags |= FLAG_BOOTFS_PRESENT;
        h.bootfs = bootfs;
    })
}

fn rejects(spec: &Spec, bootfs: PhysRange) -> Reject {
    validate(&with(spec, bootfs), BLOCK_PHYS).map(|_| ()).expect_err("the record must be rejected")
}

#[test]
fn accepts_an_image_with_its_reserved_range() {
    let block = with(&spec_with_range(BASE, 4), image());
    let ok = validate(&block, BLOCK_PHYS).expect("a sane record");
    assert_eq!(ok.bootfs(), Some(image()));
}

#[test]
fn without_the_flag_there_is_no_image_whatever_the_record_says() {
    let block = build(&good(), |h| h.bootfs = PhysRange { base: 1, size: u64::MAX });
    let ok = validate(&block, BLOCK_PHYS).expect("the record is ignored without the flag");
    assert_eq!(ok.bootfs(), None);
}

#[test]
fn rejects_a_reserved_range_for_an_image_that_is_not_announced() {
    let block = build(&spec_with_range(BASE, 4), |_| {});
    assert_eq!(validate(&block, BLOCK_PHYS).map(|_| ()).unwrap_err(), Reject::Bootfs);
    assert_eq!(Reject::Bootfs.as_str(), "bootfs");
}

#[test]
fn rejects_an_image_without_its_reserved_range() {
    assert_eq!(rejects(&good(), image()), Reject::Bootfs);
}

#[test]
fn rejects_a_base_that_is_zero_or_not_page_aligned() {
    let spec = spec_with_range(BASE, 4);
    assert_eq!(rejects(&spec, PhysRange { base: 0, size: SIZE }), Reject::Bootfs);
    assert_eq!(rejects(&spec, PhysRange { base: BASE + 8, size: SIZE }), Reject::Bootfs);
}

#[test]
fn rejects_a_range_that_is_not_exactly_the_pages_of_the_image() {
    assert_eq!(rejects(&spec_with_range(BASE, 3), image()), Reject::Bootfs);
    assert_eq!(rejects(&spec_with_range(BASE, 5), image()), Reject::Bootfs);
    assert_eq!(rejects(&spec_with_range(BASE + 4096, 4), image()), Reject::Bootfs);
    // A size that is a page multiple needs no extra page.
    let block = with(&spec_with_range(BASE, 4), PhysRange { base: BASE, size: 4 * 4096 });
    assert!(validate(&block, BLOCK_PHYS).is_ok());
    assert_eq!(rejects(&spec_with_range(BASE, 5), PhysRange { base: BASE, size: 4 * 4096 }), Reject::Bootfs);
}

#[test]
fn rejects_two_ranges_for_the_image() {
    let mut spec = spec_with_range(BASE, 4);
    spec.reserved.insert(5, range(BASE + 0x1_0000, 4, PURPOSE_BOOTFS));
    assert_eq!(rejects(&spec, image()), Reject::Bootfs);
}

#[test]
fn rejects_a_size_of_zero_or_above_32_mib() {
    assert_eq!(rejects(&spec_with_range(BASE, 4), PhysRange { base: BASE, size: 0 }), Reject::Bootfs);

    // A loader region large enough that only the limit can refuse the image.
    const HIGH: u64 = 0x1_0000_0000;
    let big = |pages: u64| {
        let mut spec = good();
        spec.regions.push(region(HIGH, 0x3000, KIND_LOADER));
        spec.reserved.push(range(HIGH, pages, PURPOSE_BOOTFS));
        spec
    };
    let limit = 32 << 20;
    assert_eq!(MAX_BOOTFS, limit);
    let at_limit = with(&big(8192), PhysRange { base: HIGH, size: limit });
    assert!(validate(&at_limit, BLOCK_PHYS).is_ok());
    assert_eq!(rejects(&big(8193), PhysRange { base: HIGH, size: limit + 1 }), Reject::Bootfs);
}
