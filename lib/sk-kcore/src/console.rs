//! A grid of text on a framebuffer (docs/10 section 7, step 13): 8 by 16
//! pixel cells below the band of the top sixteen pixel rows.
//!
//! A row that was never written, or that a scroll cleared, is `VOID`; only
//! the set pixels of a glyph are drawn. Nothing here panics: a buffer that
//! is shorter than the record says is left alone.

use stillkey_bootinfo::Framebuffer;

use crate::font;

pub const CELL_WIDTH: u32 = 8;
pub const CELL_HEIGHT: u32 = 16;
/// The pixel row of the first text row.
pub const TOP: u32 = 16;

/// Design tokens of docs/05 section 2, as `0xRRGGBB`.
pub const VOID: u32 = 0x06_08_0C;
pub const TEXT: u32 = 0xE9_EE_F5;
pub const RED: u32 = 0xFF_6B_74;

/// The console shows this many bytes of a log line; the rest is cut.
pub const LINE_MAX: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Console {
    cols: u32,
    rows: u32,
    /// Text rows written so far, at most `rows`.
    used: u32,
}

impl Console {
    /// `None` if the record has no byte order or no room for one cell.
    pub fn new(fb: &Framebuffer) -> Option<Console> {
        fb.pixel(0)?;
        let cols = fb.width / CELL_WIDTH;
        let rows = fb.height.checked_sub(TOP)? / CELL_HEIGHT;
        (cols != 0 && rows != 0 && fb.stride_pixels >= fb.width).then_some(Console { cols, rows, used: 0 })
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Writes `text` on the next row, and on further rows if it is longer
    /// than a row. `buf` is the buffer from its first byte. A byte outside
    /// 0x20-0x7E is drawn as `?`.
    pub fn write_line(&mut self, fb: &Framebuffer, buf: &mut [u8], text: &[u8], rgb: u32) {
        let (Some(pixel), Some(span)) = (fb.pixel(rgb), fb.span_bytes()) else {
            return;
        };
        if (buf.len() as u64) < span {
            return;
        }
        let mut rest = text;
        loop {
            let (now, later) = rest.split_at(rest.len().min(self.cols as usize));
            let row = self.next_row(fb, buf);
            for (col, &byte) in now.iter().enumerate() {
                draw(fb, buf, col as u32, row, font::glyph(byte), pixel);
            }
            if later.is_empty() {
                return;
            }
            rest = later;
        }
    }

    /// The row to write next; when every row is used, the text moves up by
    /// one row and the last row is cleared.
    fn next_row(&mut self, fb: &Framebuffer, buf: &mut [u8]) -> u32 {
        if self.used < self.rows {
            self.used += 1;
            return self.used - 1;
        }
        let line = fb.stride_pixels as usize * 4;
        let visible = fb.width as usize * 4;
        let last = TOP + (self.rows - 1) * CELL_HEIGHT;
        for y in TOP..last {
            let to = y as usize * line;
            let from = to + CELL_HEIGHT as usize * line;
            buf.copy_within(from..from + visible, to);
        }
        fb.fill_rows(buf, last, last + CELL_HEIGHT, VOID);
        self.rows - 1
    }
}

/// Sets the pixels of one glyph in the cell at `col`, `row`.
fn draw(fb: &Framebuffer, buf: &mut [u8], col: u32, row: u32, glyph: &[u8; 16], pixel: [u8; 4]) {
    let line = fb.stride_pixels as usize * 4;
    let left = (col * CELL_WIDTH) as usize;
    let top = (TOP + row * CELL_HEIGHT) as usize;
    for (dy, bits) in glyph.iter().enumerate() {
        for dx in 0..CELL_WIDTH as usize {
            if bits & (0x80 >> dx) != 0 {
                let at = (top + dy) * line + (left + dx) * 4;
                buf[at..at + 4].copy_from_slice(&pixel);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use stillkey_bootinfo::{FB_FORMAT_BGRX, FB_FORMAT_NONE, FB_FORMAT_RGBX, FRAMEBUFFER_VIRT};

    use super::*;

    /// A byte no colour of these tests contains.
    const UNTOUCHED: u8 = 0xAA;

    fn screen(width: u32, height: u32, stride: u32, format: u32) -> Framebuffer {
        Framebuffer {
            phys_base: 0x1000_0000,
            size: u64::from(stride) * 4 * u64::from(height),
            virt_base: FRAMEBUFFER_VIRT,
            width,
            height,
            stride_pixels: stride,
            format,
        }
    }

    fn small() -> Framebuffer {
        screen(64, 64, 64, FB_FORMAT_RGBX)
    }

    /// A buffer as the kernel leaves it before the console starts.
    fn cleared(fb: &Framebuffer) -> Vec<u8> {
        let mut buf = vec![0; fb.span_bytes().unwrap() as usize];
        assert!(fb.fill_rows(&mut buf, 0, fb.height, VOID));
        buf
    }

    /// What `lines` look like on a fresh console.
    fn shown(fb: &Framebuffer, lines: &[&[u8]]) -> Vec<u8> {
        let mut buf = cleared(fb);
        let mut console = Console::new(fb).unwrap();
        for line in lines {
            console.write_line(fb, &mut buf, line, TEXT);
        }
        buf
    }

    #[test]
    fn the_grid_is_the_screen_below_the_band() {
        let console = Console::new(&small()).unwrap();
        assert_eq!((console.cols(), console.rows()), (8, 3));
        let console = Console::new(&screen(1280, 800, 1280, FB_FORMAT_BGRX)).unwrap();
        assert_eq!((console.cols(), console.rows()), (160, 49));
    }

    #[test]
    fn a_screen_without_room_or_byte_order_has_no_console() {
        assert_eq!(Console::new(&screen(7, 64, 7, FB_FORMAT_RGBX)), None);
        assert_eq!(Console::new(&screen(64, 31, 64, FB_FORMAT_RGBX)), None);
        assert_eq!(Console::new(&screen(64, 15, 64, FB_FORMAT_RGBX)), None);
        assert_eq!(Console::new(&screen(64, 64, 64, FB_FORMAT_NONE)), None);
        assert!(Console::new(&screen(8, 32, 8, FB_FORMAT_RGBX)).is_some());
    }

    #[test]
    fn a_letter_sets_exactly_its_pixels_in_both_byte_orders() {
        for (format, colour) in [(FB_FORMAT_RGBX, [0xE9, 0xEE, 0xF5, 0]), (FB_FORMAT_BGRX, [0xF5, 0xEE, 0xE9, 0])] {
            // A scan line longer than the width: the bytes between are not the picture's.
            let fb = screen(64, 64, 70, format);
            let mut buf = vec![UNTOUCHED; fb.span_bytes().unwrap() as usize];
            let mut console = Console::new(&fb).unwrap();
            console.write_line(&fb, &mut buf, b"A", TEXT);
            let glyph = font::glyph(b'A');
            assert!(glyph.iter().any(|&row| row != 0));
            for y in 0..64usize {
                for x in 0..70usize {
                    let at = (y * 70 + x) * 4;
                    let set = x < 8 && (16..32).contains(&y) && glyph[y - 16] & (0x80 >> x) != 0;
                    let want = if set { colour } else { [UNTOUCHED; 4] };
                    assert_eq!(buf[at..at + 4], want, "format {format} x={x} y={y}");
                }
            }
        }
    }

    #[test]
    fn a_line_one_byte_longer_than_a_row_uses_two_rows() {
        let fb = small();
        assert_eq!(shown(&fb, &[b"ABCDEFGHI", b"z"]), shown(&fb, &[b"ABCDEFGH", b"I", b"z"]));
    }

    #[test]
    fn a_line_as_long_as_a_row_uses_one_row() {
        let fb = small();
        assert_ne!(shown(&fb, &[b"ABCDEFGH", b"z"]), shown(&fb, &[b"ABCDEFGH", b"", b"z"]));
    }

    #[test]
    fn an_empty_line_uses_a_row() {
        let fb = small();
        assert_eq!(shown(&fb, &[b"", b"z"]), shown(&fb, &[b" ", b"z"]));
        assert_ne!(shown(&fb, &[b"", b"z"]), shown(&fb, &[b"z"]));
    }

    #[test]
    fn one_line_more_than_fits_moves_the_text_up_and_clears_the_last_row() {
        let fb = small();
        // The last line is shorter than the one whose row it takes.
        assert_eq!(shown(&fb, &[b"1111", b"2222", b"3333", b"4"]), shown(&fb, &[b"2222", b"3333", b"4"]));
        assert_eq!(
            shown(&fb, &[b"1111", b"2222", b"3333", b"4", b"55"]),
            shown(&fb, &[b"3333", b"4", b"55"])
        );
    }

    #[test]
    fn a_scroll_leaves_the_band_and_the_padding_alone() {
        let fb = screen(64, 64, 70, FB_FORMAT_RGBX);
        let mut buf = vec![UNTOUCHED; fb.span_bytes().unwrap() as usize];
        assert!(fb.fill_rows(&mut buf, 16, 64, VOID));
        let mut console = Console::new(&fb).unwrap();
        for line in [b"1", b"2", b"3", b"4"] {
            console.write_line(&fb, &mut buf, line, TEXT);
        }
        for y in 0..64usize {
            for x in 0..70usize {
                let at = (y * 70 + x) * 4;
                if y < 16 || x >= 64 {
                    assert_eq!(buf[at..at + 4], [UNTOUCHED; 4], "x={x} y={y}");
                }
            }
        }
    }

    #[test]
    fn a_wrapped_line_scrolls_too() {
        let fb = small();
        assert_eq!(shown(&fb, &[b"1", b"2", b"ABCDEFGHIJKLMNOPQ"]), shown(&fb, &[b"ABCDEFGH", b"IJKLMNOP", b"Q"]));
    }

    #[test]
    fn a_byte_that_is_no_printable_character_is_a_question_mark() {
        let fb = small();
        assert_eq!(shown(&fb, &[&[0x07, 0x7F, 0xFF, 0x1F]]), shown(&fb, &[b"????"]));
        assert_ne!(shown(&fb, &[b"~"]), shown(&fb, &[b"?"]));
    }

    #[test]
    fn the_colour_is_the_one_asked_for() {
        let fb = small();
        let mut buf = cleared(&fb);
        Console::new(&fb).unwrap().write_line(&fb, &mut buf, b"A", RED);
        assert!(buf.chunks_exact(4).any(|p| p == [0xFF, 0x6B, 0x74, 0]));
        assert!(buf.chunks_exact(4).all(|p| p == [0xFF, 0x6B, 0x74, 0] || p == [0x06, 0x08, 0x0C, 0]));
    }

    #[test]
    fn a_buffer_that_is_too_short_is_left_alone() {
        let fb = small();
        let mut buf = vec![UNTOUCHED; fb.span_bytes().unwrap() as usize - 1];
        Console::new(&fb).unwrap().write_line(&fb, &mut buf, b"A", TEXT);
        assert!(buf.iter().all(|&b| b == UNTOUCHED));
    }
}
