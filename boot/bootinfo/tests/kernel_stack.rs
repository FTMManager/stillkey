//! The `kernel` and `stack` rows of the handoff validation (docs/10 section
//! 5.4). The kernel builds its own page tables from these two records and
//! frees the boot stack by its record, so each must agree with the reserved
//! range that protects it. Every condition is broken once.

mod common;

use common::*;
use stillkey_bootinfo::*;

fn rejects(spec: &Spec, edit: impl FnOnce(&mut BootInfo)) -> Reject {
    validate(&build(spec, edit), BLOCK_PHYS).map(|_| ()).expect_err("the record must be rejected")
}

fn without(purpose: u32) -> Spec {
    let mut spec = good();
    spec.reserved.retain(|r| r.purpose != purpose);
    spec
}

fn resized(purpose: u32, pages: u64) -> Spec {
    let mut spec = good();
    for r in spec.reserved.iter_mut().filter(|r| r.purpose == purpose) {
        r.pages = pages;
    }
    spec
}

/// A second range of the same purpose, in the free part of the loader region.
fn doubled(purpose: u32) -> Spec {
    let mut spec = good();
    spec.reserved.insert(4, range(0x0150_0000, 16, purpose)); // sorted by base
    spec
}

#[test]
fn accepts_the_records_the_loader_writes() {
    let block = build(&good(), |_| {});
    let ok = validate(&block, BLOCK_PHYS).expect("a sane handoff");
    assert_eq!(ok.header.kernel.virt_base, KERNEL_WINDOW);
    assert_eq!(ok.header.boot_stack.virt_top, STACK_WINDOW + 4096 + BOOT_STACK_SIZE);
}

#[test]
fn accepts_a_kernel_whose_last_page_is_partly_used() {
    let block = build(&good(), |h| h.kernel.size -= 100);
    validate(&block, BLOCK_PHYS).expect("the range is the size rounded up to a page");
}

// ---- kernel ----

#[test]
fn kernel_outside_its_window() {
    assert_eq!(rejects(&good(), |h| h.kernel.virt_base += 4096), Reject::Kernel);
    assert_eq!(rejects(&good(), |h| h.kernel.virt_base = DIRECT_MAP_BASE), Reject::Kernel);
}

#[test]
fn kernel_physical_base_not_on_a_page() {
    assert_eq!(rejects(&good(), |h| h.kernel.phys_base += 8), Reject::Kernel);
}

#[test]
fn kernel_of_no_size() {
    assert_eq!(rejects(&good(), |h| h.kernel.size = 0), Reject::Kernel);
}

#[test]
fn kernel_without_its_reserved_range() {
    assert_eq!(rejects(&without(PURPOSE_KERNEL_IMAGE), |_| {}), Reject::Kernel);
}

#[test]
fn kernel_with_two_reserved_ranges() {
    assert_eq!(rejects(&doubled(PURPOSE_KERNEL_IMAGE), |_| {}), Reject::Kernel);
}

#[test]
fn kernel_range_shorter_or_longer_than_the_image() {
    assert_eq!(rejects(&resized(PURPOSE_KERNEL_IMAGE, 15), |_| {}), Reject::Kernel);
    assert_eq!(rejects(&resized(PURPOSE_KERNEL_IMAGE, 17), |_| {}), Reject::Kernel);
}

#[test]
fn kernel_range_somewhere_else() {
    assert_eq!(rejects(&good(), |h| h.kernel.phys_base += 4096), Reject::Kernel);
}

#[test]
fn kernel_entry_outside_the_image() {
    assert_eq!(rejects(&good(), |h| h.kernel.entry = h.kernel.virt_base + h.kernel.size), Reject::Kernel);
    assert_eq!(rejects(&good(), |h| h.kernel.entry = h.kernel.virt_base - 1), Reject::Kernel);
    let first = build(&good(), |h| h.kernel.entry = h.kernel.virt_base);
    validate(&first, BLOCK_PHYS).expect("the first byte is inside");
}

#[test]
fn kernel_size_that_wraps() {
    assert_eq!(rejects(&good(), |h| h.kernel.size = u64::MAX), Reject::Kernel);
}

// ---- boot stack ----

#[test]
fn stack_of_another_size() {
    assert_eq!(rejects(&good(), |h| h.boot_stack.size -= 4096), Reject::Stack);
    assert_eq!(rejects(&good(), |h| h.boot_stack.size += 4096), Reject::Stack);
}

#[test]
fn stack_top_elsewhere() {
    assert_eq!(rejects(&good(), |h| h.boot_stack.virt_top -= 4096), Reject::Stack);
    assert_eq!(rejects(&good(), |h| h.boot_stack.virt_top = 0), Reject::Stack);
}

#[test]
fn stack_without_its_reserved_range() {
    assert_eq!(rejects(&without(PURPOSE_BOOT_STACK), |_| {}), Reject::Stack);
}

#[test]
fn stack_with_two_reserved_ranges() {
    assert_eq!(rejects(&doubled(PURPOSE_BOOT_STACK), |_| {}), Reject::Stack);
}

#[test]
fn stack_range_that_does_not_match() {
    assert_eq!(rejects(&resized(PURPOSE_BOOT_STACK, 15), |_| {}), Reject::Stack);
    assert_eq!(rejects(&good(), |h| h.boot_stack.phys_base += 4096), Reject::Stack);
}

#[test]
fn the_words_of_the_two_rows() {
    assert_eq!(Reject::Kernel.as_str(), "kernel");
    assert_eq!(Reject::Stack.as_str(), "stack");
}
