//! The framebuffer record: its acceptance rule (docs/02 section 6), the byte
//! order of the two formats, and filling rows.

mod common;

use common::*;
use stillkey_bootinfo::*;

/// The mode this VM's firmware selects: 1280x800, blue first, in device
/// memory at 2 GiB (the kind-8 region of `good()`).
fn vm_mode() -> Framebuffer {
    Framebuffer {
        phys_base: 0x8000_0000,
        size: 0x40_0000,
        virt_base: FRAMEBUFFER_VIRT,
        width: 1280,
        height: 800,
        stride_pixels: 1280,
        format: FB_FORMAT_BGRX,
    }
}

/// A 16x16 mode, 1024 bytes of pixels, for cases about where the buffer lies.
fn small_mode(phys_base: u64, size: u64) -> Framebuffer {
    Framebuffer { phys_base, size, width: 16, height: 16, stride_pixels: 16, ..vm_mode() }
}

fn spec_with(spec: &Spec, fb: Framebuffer) -> Vec<u8> {
    build(spec, |h| {
        h.flags |= FLAG_FRAMEBUFFER_PRESENT;
        h.framebuffer = fb;
    })
}

fn with(fb: Framebuffer) -> Vec<u8> {
    spec_with(&good(), fb)
}

fn rejects(fb: Framebuffer) -> Reject {
    validate(&with(fb), BLOCK_PHYS).expect_err("the framebuffer record must be rejected")
}

// ---- Acceptance ------------------------------------------------------------

#[test]
fn accepts_the_mode_the_vm_reports() {
    let block = with(vm_mode());
    let ok = validate(&block, BLOCK_PHYS).expect("a sane record");
    assert_eq!(ok.framebuffer(), Some(vm_mode()));
}

#[test]
fn without_the_flag_there_is_no_framebuffer_whatever_the_record_says() {
    // The record is nonsense, and nobody may look at it.
    let block = build(&good(), |h| {
        h.framebuffer = Framebuffer { format: 99, width: 0, ..vm_mode() };
    });
    let ok = validate(&block, BLOCK_PHYS).expect("the record is ignored without the flag");
    assert_eq!(ok.framebuffer(), None);
}

#[test]
fn accepts_a_stride_wider_than_the_width_when_the_size_covers_it() {
    let needed = 1288 * 4 * 800;
    let fb = Framebuffer { stride_pixels: 1288, size: needed, ..vm_mode() };
    assert_eq!(validate(&with(fb), BLOCK_PHYS).expect("exactly enough").framebuffer(), Some(fb));
}

#[test]
fn rejects_a_size_one_byte_short_of_the_last_row() {
    let needed = 1288 * 4 * 800;
    let fb = Framebuffer { stride_pixels: 1288, size: needed - 1, ..vm_mode() };
    assert_eq!(rejects(fb), Reject::Framebuffer);
}

#[test]
fn rejects_formats_other_than_the_two_32_bit_ones() {
    assert_eq!(rejects(Framebuffer { format: FB_FORMAT_NONE, ..vm_mode() }), Reject::Framebuffer);
    assert_eq!(rejects(Framebuffer { format: 3, ..vm_mode() }), Reject::Framebuffer);
    let rgbx = Framebuffer { format: FB_FORMAT_RGBX, ..vm_mode() };
    assert_eq!(validate(&with(rgbx), BLOCK_PHYS).expect("red first").framebuffer(), Some(rgbx));
}

#[test]
fn rejects_an_empty_mode_and_a_stride_narrower_than_the_width() {
    assert_eq!(rejects(Framebuffer { width: 0, ..vm_mode() }), Reject::Framebuffer);
    assert_eq!(rejects(Framebuffer { height: 0, ..vm_mode() }), Reject::Framebuffer);
    assert_eq!(rejects(Framebuffer { stride_pixels: 1279, ..vm_mode() }), Reject::Framebuffer);
}

#[test]
fn rejects_dimensions_chosen_to_overflow() {
    let fb = Framebuffer { width: 1, stride_pixels: u32::MAX, height: u32::MAX, ..vm_mode() };
    assert_eq!(rejects(fb), Reject::Framebuffer);
}

#[test]
fn rejects_a_size_above_the_limit() {
    let ok = Framebuffer { size: MAX_FRAMEBUFFER, ..vm_mode() };
    assert_eq!(validate(&with(ok), BLOCK_PHYS).expect("the limit itself").framebuffer(), Some(ok));
    let fb = Framebuffer { size: MAX_FRAMEBUFFER + 1, ..vm_mode() };
    assert_eq!(rejects(fb), Reject::Framebuffer);
}

#[test]
fn rejects_an_address_the_loader_cannot_have_mapped() {
    assert_eq!(rejects(Framebuffer { phys_base: 0, ..vm_mode() }), Reject::Framebuffer);
    assert_eq!(rejects(Framebuffer { phys_base: 0x8000_0800, ..vm_mode() }), Reject::Framebuffer);
    // The physical range wraps.
    let top = Framebuffer { phys_base: 0xFFFF_FFFF_FFFF_F000, ..vm_mode() };
    assert_eq!(rejects(top), Reject::Framebuffer);
    // Not the window the loader maps it in.
    let elsewhere = Framebuffer { virt_base: FRAMEBUFFER_VIRT + 0x1000, ..vm_mode() };
    assert_eq!(rejects(elsewhere), Reject::Framebuffer);
    assert_eq!(rejects(Framebuffer { virt_base: 0, ..vm_mode() }), Reject::Framebuffer);
}

/// Writing pixels must never be able to change the kernel image, the boot
/// stack, the page tables, the handoff block or the loader image.
#[test]
fn rejects_a_framebuffer_that_overlaps_a_reserved_range() {
    for r in good().reserved {
        assert_eq!(rejects(small_mode(r.base, 0x1000)), Reject::Framebuffer, "{r:?}");
    }
    // The last page of a larger buffer is the first page of the kernel image.
    assert_eq!(rejects(small_mode(0x010F_0000, 0x1_1000)), Reject::Framebuffer);
}

/// The loader maps whole pages, so a buffer that ends inside a page reaches
/// to the end of that page.
#[test]
fn the_overlap_rule_counts_the_whole_last_page() {
    // Ends exactly where the kernel image begins.
    let beside = small_mode(0x010F_F000, 0x1000);
    assert_eq!(validate(&with(beside), BLOCK_PHYS).expect("adjacent").framebuffer(), Some(beside));
    // One byte more: its last page is the first page of the kernel image.
    assert_eq!(rejects(small_mode(0x010F_F000, 0x1001)), Reject::Framebuffer);
}

// ---- What an allocator may use (docs/02 section 7) --------------------------

/// `good()`: 0xF00 usable pages above 1 MiB, and 0x1000 loader pages of which
/// 1 + 16 + 16 + 12 + 32 are reserved.
const GOOD_USABLE: u64 = 0xF00 + 0x1000 - 77;

fn usable(block: &[u8]) -> u64 {
    validate(block, BLOCK_PHYS).expect("valid block").usable_after_reserve()
}

#[test]
fn the_first_mib_and_the_reserved_ranges_are_not_usable() {
    assert_eq!(usable(&build(&good(), |_| {})), GOOD_USABLE);
}

#[test]
fn a_framebuffer_in_device_memory_takes_nothing_from_the_usable_pages() {
    assert_eq!(usable(&with(vm_mode())), GOOD_USABLE);
}

#[test]
fn a_framebuffer_inside_usable_memory_is_subtracted() {
    let fb = Framebuffer { phys_base: 0x20_0000, ..vm_mode() }; // 0x400 pages
    assert_eq!(usable(&with(fb)), GOOD_USABLE - 0x400);
}

#[test]
fn a_framebuffer_inside_loader_memory_is_subtracted_by_whole_pages() {
    // 0x1001 bytes are two pages.
    assert_eq!(usable(&with(small_mode(0x01F0_0000, 0x1001))), GOOD_USABLE - 2);
}

#[test]
fn a_framebuffer_page_below_1_mib_is_not_subtracted_twice() {
    // One usable region across the 1 MiB line; the same number of usable pages as `good()`.
    let mut spec = good();
    spec.regions[0] = region(0, 0x200, KIND_USABLE);
    spec.regions[1] = region(0x20_0000, 0xE00, KIND_USABLE);
    assert_eq!(usable(&build(&spec, |_| {})), GOOD_USABLE);
    // Two pages: the last below 1 MiB, which was never usable, and the first above it.
    assert_eq!(usable(&spec_with(&spec, small_mode(0xF_F000, 0x2000))), GOOD_USABLE - 1);
}

// ---- Byte order and filling ------------------------------------------------

const CYAN: u32 = 0x3D_DC_FF;

#[test]
fn rgbx_stores_red_first_and_bgrx_stores_blue_first() {
    let rgbx = Framebuffer { format: FB_FORMAT_RGBX, ..vm_mode() };
    let bgrx = Framebuffer { format: FB_FORMAT_BGRX, ..vm_mode() };
    assert_eq!(rgbx.pixel(CYAN), Some([0x3D, 0xDC, 0xFF, 0x00]));
    assert_eq!(bgrx.pixel(CYAN), Some([0xFF, 0xDC, 0x3D, 0x00]));
    assert_eq!(Framebuffer { format: FB_FORMAT_NONE, ..vm_mode() }.pixel(CYAN), None);
    assert_eq!(Framebuffer { format: 7, ..vm_mode() }.pixel(CYAN), None);
}

/// Three pixels wide, five per scan line, four rows: 80 bytes.
fn narrow() -> Framebuffer {
    Framebuffer { width: 3, stride_pixels: 5, height: 4, format: FB_FORMAT_BGRX, ..vm_mode() }
}

const UNTOUCHED: u8 = 0xAA;

#[test]
fn fill_rows_writes_the_visible_pixels_of_the_rows_named_and_nothing_else() {
    let mut buf = [UNTOUCHED; 80];
    assert!(narrow().fill_rows(&mut buf, 1, 3, CYAN));
    let pixel = [0xFF, 0xDC, 0x3D, 0x00];
    for row in 0..4 {
        let bytes = &buf[row * 20..(row + 1) * 20];
        if row == 1 || row == 2 {
            assert_eq!(&bytes[..12], [pixel, pixel, pixel].concat(), "row {row}");
            // The two pixels of padding belong to the scan line, not to the picture.
            assert_eq!(&bytes[12..], [UNTOUCHED; 8], "padding of row {row}");
        } else {
            assert_eq!(bytes, [UNTOUCHED; 20], "row {row}");
        }
    }
}

#[test]
fn fill_rows_steps_by_the_stride_not_by_the_width() {
    let mut buf = [UNTOUCHED; 80];
    assert!(narrow().fill_rows(&mut buf, 3, 4, 0x01_02_03));
    // Row 3 begins at byte 3 * 5 * 4; a step of width * 4 would begin at 36.
    assert_eq!(&buf[60..64], [0x03, 0x02, 0x01, 0x00]);
    assert_eq!(&buf[36..40], [UNTOUCHED; 4]);
}

#[test]
fn fill_rows_accepts_a_buffer_that_ends_with_the_last_visible_pixel() {
    let mut buf = [UNTOUCHED; 72];
    assert!(narrow().fill_rows(&mut buf, 0, 4, CYAN));
    assert_eq!(&buf[68..], [0xFF, 0xDC, 0x3D, 0x00]);
}

#[test]
fn fill_rows_refuses_what_does_not_fit_and_writes_nothing() {
    let mut short = [UNTOUCHED; 71];
    assert!(!narrow().fill_rows(&mut short, 0, 4, CYAN));
    assert_eq!(short, [UNTOUCHED; 71]);

    // Room for a fifth row, which the mode does not have.
    let mut buf = [UNTOUCHED; 100];
    assert!(!narrow().fill_rows(&mut buf, 0, 5, CYAN), "row 4 does not exist");
    assert!(!narrow().fill_rows(&mut buf, 4, 5, CYAN), "row 4 does not exist");
    assert!(!narrow().fill_rows(&mut buf, 3, 2, CYAN), "the rows run backwards");
    assert!(!narrow().fill_rows(&mut buf, 1, 0, CYAN), "the rows run backwards");
    let unknown = Framebuffer { format: 9, ..narrow() };
    assert!(!unknown.fill_rows(&mut buf, 0, 4, CYAN), "no byte order is known");
    assert_eq!(buf, [UNTOUCHED; 100]);
}

#[test]
fn fill_rows_of_no_rows_writes_nothing() {
    let mut buf = [UNTOUCHED; 80];
    assert!(narrow().fill_rows(&mut buf, 2, 2, CYAN));
    assert_eq!(buf, [UNTOUCHED; 80]);
}
