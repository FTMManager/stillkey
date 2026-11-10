//! What a `Framebuffer` record means (docs/02 section 6): which records can
//! be mapped and painted, the byte order of the two formats, and how the rows
//! lie in the buffer. The loader applies `is_well_formed` before it hands a
//! record over and the kernel applies it again before it writes a pixel.
//!
//! Tests are in `tests/framebuffer.rs`.

use crate::{
    FB_FORMAT_BGRX, FB_FORMAT_RGBX, FRAMEBUFFER_VIRT, Framebuffer, MAX_FRAMEBUFFER, PAGE_SIZE,
};

/// Each pixel is four bytes in both formats.
const PIXEL: u64 = 4;

impl Framebuffer {
    /// Bytes from the first pixel to the end of the last scan line:
    /// `stride_pixels * 4 * height`. `None` if that overflows.
    pub fn span_bytes(&self) -> Option<u64> {
        u64::from(self.stride_pixels).checked_mul(PIXEL)?.checked_mul(u64::from(self.height))
    }

    /// The physical range the loader maps for the buffer: from `phys_base` to
    /// the end of the page that holds its last byte. `None` if the range wraps.
    pub fn phys_pages(&self) -> Option<(u64, u64)> {
        let end = self.phys_base.checked_add(self.size)?.checked_add(PAGE_SIZE - 1)?;
        Some((self.phys_base, end & !(PAGE_SIZE - 1)))
    }

    /// True if the record describes a buffer that can be mapped and painted:
    /// one of the two 32-bit formats; a mode that is not empty, with a scan
    /// line at least as long as the width; a page-aligned physical address
    /// that is not zero; the virtual address the loader maps it at; and a
    /// size that holds every scan line, is at most [`MAX_FRAMEBUFFER`] and
    /// does not wrap the address space.
    pub fn is_well_formed(&self) -> bool {
        matches!(self.format, FB_FORMAT_RGBX | FB_FORMAT_BGRX)
            && self.width != 0
            && self.height != 0
            && self.stride_pixels >= self.width
            && self.phys_base != 0
            && self.phys_base % PAGE_SIZE == 0
            && self.virt_base == FRAMEBUFFER_VIRT
            && self.size <= MAX_FRAMEBUFFER
            && self.span_bytes().is_some_and(|span| span <= self.size)
            && self.phys_pages().is_some()
    }

    /// The four bytes of one pixel of colour `0xRRGGBB`, in the order this
    /// record's format stores them. The fourth byte is reserved and written
    /// as zero. `None` for a format that has no byte order.
    pub fn pixel(&self, rgb: u32) -> Option<[u8; 4]> {
        let [_, red, green, blue] = rgb.to_be_bytes();
        match self.format {
            FB_FORMAT_RGBX => Some([red, green, blue, 0]),
            FB_FORMAT_BGRX => Some([blue, green, red, 0]),
            _ => None,
        }
    }

    /// Writes colour `0xRRGGBB` to every visible pixel of the rows
    /// `first..end`. `buf` is the buffer from its first byte. A row begins
    /// every `stride_pixels * 4` bytes; the bytes between the width and the
    /// stride are not part of the picture and are left alone.
    ///
    /// Returns `false`, having written nothing, if the format has no byte
    /// order, if `first..end` are not rows of this mode, or if `buf` ends
    /// before the last visible pixel of the last row.
    pub fn fill_rows(&self, buf: &mut [u8], first: u32, end: u32, rgb: u32) -> bool {
        let Some(pixel) = self.pixel(rgb) else {
            return false;
        };
        if first > end
            || end > self.height
            || self.stride_pixels == 0
            || self.stride_pixels < self.width
        {
            return false;
        }
        if first == end {
            return true;
        }
        let row = u64::from(self.stride_pixels) * PIXEL;
        let visible = u64::from(self.width) * PIXEL;
        // Neither product overflows: both factors are below 2^35.
        let start = u64::from(first) * row;
        let stop = u64::from(end - 1) * row + visible;
        let (Ok(row), Ok(visible), Ok(start), Ok(stop)) = (
            usize::try_from(row),
            usize::try_from(visible),
            usize::try_from(start),
            usize::try_from(stop),
        ) else {
            return false;
        };
        let Some(rows) = buf.get_mut(start..stop) else {
            return false;
        };
        // Every chunk is a whole scan line except the last, which ends with
        // its last visible pixel.
        for line in rows.chunks_mut(row) {
            for bytes in line[..visible].chunks_exact_mut(pixel.len()) {
                bytes.copy_from_slice(&pixel);
            }
        }
        true
    }
}
