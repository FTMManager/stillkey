//! Step 5: the first pixels (docs/07 section 7). Every visible pixel is
//! written once: a band of `cyan` along the top edge and `void` below it.
//!
//! Which bytes make a pixel and where a row begins is decided by
//! `stillkey_bootinfo::Framebuffer::fill_rows`, which is pure and tested on
//! the host. This file holds the one step that cannot be: turning the
//! record's address into a byte slice.

use stillkey_bootinfo::{FB_FORMAT_RGBX, Framebuffer};

/// Design tokens of docs/05 section 2, as `0xRRGGBB`.
const VOID: u32 = 0x06_08_0C;
const CYAN: u32 = 0x3D_DC_FF;
/// Height of the band, in rows.
const BAND_ROWS: u32 = 8;

/// The name of the record's format in the log.
pub fn format_name(fb: &Framebuffer) -> &'static str {
    if fb.format == FB_FORMAT_RGBX { "rgbx" } else { "bgrx" }
}

/// Fills the screen. `fb` must come from `Accepted::framebuffer`.
pub fn fill(fb: &Framebuffer) {
    let span = fb
        .span_bytes()
        .and_then(|bytes| usize::try_from(bytes).ok())
        .expect("an accepted framebuffer has a span");
    // SAFETY: `validate` accepted the record, so `virt_base` is the
    // framebuffer window, where the loader's page tables map `size` bytes
    // (rounded up to whole pages) writable, and `span` is at most `size`,
    // which is at most 256 MiB (docs/02 sections 5 and 6). The pointer is
    // therefore non-null, the range is mapped and fits `isize`, and any
    // byte is a valid `u8`. Nothing else in the kernel refers to this
    // window, there is one CPU and interrupts are off, and the slice ends
    // with this function. The display reads the memory at its own pace; that
    // is hardware, not a Rust reference, and every store below is to memory
    // the compiler cannot prove unobserved.
    let buf = unsafe { core::slice::from_raw_parts_mut(fb.virt_base as *mut u8, span) };
    let band = BAND_ROWS.min(fb.height);
    let filled = fb.fill_rows(buf, 0, band, CYAN) && fb.fill_rows(buf, band, fb.height, VOID);
    assert!(filled, "the accepted framebuffer record does not describe its buffer");
}
