//! Reads a QEMU `screendump` (a binary PPM file) and checks the picture the
//! Milestone 1 kernel paints: a band of `cyan` along the top edge and `void`
//! below it (docs/07 section 7, kernel step 5).

use sk_kcore::console::{self, Console};
use stillkey_bootinfo::{FB_FORMAT_RGBX, FRAMEBUFFER_VIRT, Framebuffer};

/// Design tokens of docs/05 section 2, as red, green, blue.
pub const VOID: [u8; 3] = [0x06, 0x08, 0x0C];
pub const CYAN: [u8; 3] = [0x3D, 0xDC, 0xFF];
pub const BAND_ROWS: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub struct Picture<'a> {
    pub width: usize,
    pub height: usize,
    /// `width * height` pixels, three bytes each: red, green, blue.
    pub rgb: &'a [u8],
}

/// The next header token: skips white space and `#` comments before it.
fn token<'a>(file: &'a [u8], at: &mut usize) -> Result<&'a [u8], String> {
    loop {
        match file.get(*at) {
            Some(byte) if byte.is_ascii_whitespace() => *at += 1,
            Some(b'#') => {
                while file.get(*at).is_some_and(|&byte| byte != b'\n') {
                    *at += 1;
                }
            }
            Some(_) => break,
            None => return Err("the file ends inside the PPM header".to_string()),
        }
    }
    let start = *at;
    while file.get(*at).is_some_and(|byte| !byte.is_ascii_whitespace()) {
        *at += 1;
    }
    Ok(&file[start..*at])
}

fn number(file: &[u8], at: &mut usize, what: &str) -> Result<usize, String> {
    let text = token(file, at)?;
    std::str::from_utf8(text)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| format!("the PPM header's {what} is not a number"))
}

/// Reads a binary PPM file (`P6`) with one byte per sample, which is what
/// QEMU's `screendump` writes. The pixel data must fill the file exactly.
pub fn parse_ppm(file: &[u8]) -> Result<Picture<'_>, String> {
    let mut at = 0;
    if token(file, &mut at)? != b"P6" {
        return Err("not a binary PPM file (no P6 signature)".to_string());
    }
    let width = number(file, &mut at, "width")?;
    let height = number(file, &mut at, "height")?;
    if number(file, &mut at, "largest sample")? != 255 {
        return Err("the PPM file does not have one byte per sample".to_string());
    }
    // Exactly one white-space byte separates the header from the pixels.
    if !file.get(at).is_some_and(u8::is_ascii_whitespace) {
        return Err("the file ends inside the PPM header".to_string());
    }
    let rgb = &file[at + 1..];
    let bytes = width.checked_mul(height).and_then(|pixels| pixels.checked_mul(3));
    if width == 0 || height == 0 || bytes != Some(rgb.len()) {
        return Err(format!(
            "the PPM file says {width}x{height} and holds {} bytes of pixels",
            rgb.len()
        ));
    }
    Ok(Picture { width, height, rgb })
}

/// Every pixel of the first [`BAND_ROWS`] rows is [`CYAN`] and every other
/// pixel is [`VOID`], exactly.
pub fn check_fill(picture: &Picture<'_>) -> Result<(), String> {
    let hex = |c: &[u8]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let mut wrong = 0usize;
    let mut first = None;
    for (index, pixel) in picture.rgb.chunks_exact(3).enumerate() {
        let (x, y) = (index % picture.width, index / picture.width);
        let want = if y < BAND_ROWS { CYAN } else { VOID };
        if pixel != want {
            wrong += 1;
            first.get_or_insert_with(|| format!("x={x} y={y}: {}, expected {}", hex(pixel), hex(&want)));
        }
    }
    match first {
        None => Ok(()),
        Some(first) => Err(format!("{wrong} pixel(s) differ; the first is {first}")),
    }
}

/// The screen after the kernel's console drew `lines`, three bytes per
/// pixel: the band of kernel step 5, then each line in `text`, and in
/// `red` from the line that begins a panic banner (docs/10 section 10).
pub fn render_console(width: usize, height: usize, lines: &[String]) -> Result<Vec<u8>, String> {
    let size = |n: usize| u32::try_from(n).map_err(|_| "the screen is too large".to_string());
    let fb = Framebuffer {
        phys_base: 0x1000,
        size: (width * height * 4) as u64,
        virt_base: FRAMEBUFFER_VIRT,
        width: size(width)?,
        height: size(height)?,
        stride_pixels: size(width)?,
        format: FB_FORMAT_RGBX,
    };
    let colour = |c: [u8; 3]| u32::from_be_bytes([0, c[0], c[1], c[2]]);
    let mut buf = vec![0u8; width * height * 4];
    let band = BAND_ROWS.min(height) as u32;
    if !(fb.fill_rows(&mut buf, 0, band, colour(CYAN)) && fb.fill_rows(&mut buf, band, fb.height, colour(VOID))) {
        return Err(format!("a {width}x{height} screen cannot be painted"));
    }
    let mut console = Console::new(&fb).ok_or(format!("a {width}x{height} screen has no room for text"))?;
    let mut banner = false;
    for line in lines {
        banner |= line.starts_with("SK:KERNEL:PANIC_BEGIN");
        let text = &line.as_bytes()[..line.len().min(console::LINE_MAX)];
        console.write_line(&fb, &mut buf, text, if banner { console::RED } else { console::TEXT });
    }
    Ok(buf.chunks_exact(4).flat_map(|pixel| [pixel[0], pixel[1], pixel[2]]).collect())
}

/// Every pixel of the picture is the pixel of `want`, exactly.
pub fn check_same(picture: &Picture<'_>, want: &[u8]) -> Result<(), String> {
    if want.len() != picture.rgb.len() {
        return Err(format!("the picture has {} bytes, expected {}", picture.rgb.len(), want.len()));
    }
    let hex = |c: &[u8]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let mut differing = picture.rgb.chunks_exact(3).zip(want.chunks_exact(3)).enumerate().filter(|(_, (got, want))| got != want);
    match differing.next() {
        None => Ok(()),
        Some((index, (got, want))) => Err(format!(
            "{} pixel(s) differ; the first is x={} y={}: {}, expected {}",
            1 + differing.count(),
            index % picture.width,
            index / picture.width,
            hex(got),
            hex(want)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppm(width: usize, height: usize, pixel: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let mut file = format!("P6\n{width} {height}\n255\n").into_bytes();
        for y in 0..height {
            for x in 0..width {
                file.extend_from_slice(&pixel(x, y));
            }
        }
        file
    }

    fn painted(_x: usize, y: usize) -> [u8; 3] {
        if y < BAND_ROWS { CYAN } else { VOID }
    }

    const TEXT: [u8; 3] = [0xE9, 0xEE, 0xF5];
    const RED: [u8; 3] = [0xFF, 0x6B, 0x74];

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| line.to_string()).collect()
    }

    /// The colours in the pixel rows `rows` of a 64-pixel-wide picture.
    fn colours(rgb: &[u8], rows: std::ops::Range<usize>) -> Vec<[u8; 3]> {
        let mut found: Vec<[u8; 3]> =
            rgb[rows.start * 64 * 3..rows.end * 64 * 3].chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    #[test]
    fn a_console_without_lines_is_the_painted_screen() {
        let rgb = render_console(64, 64, &[]).unwrap();
        assert_eq!(check_fill(&Picture { width: 64, height: 64, rgb: &rgb }), Ok(()));
    }

    #[test]
    fn lines_are_text_until_the_banner_and_red_from_it() {
        // Six rows of eight columns: the banner's first line takes three.
        let rgb = render_console(64, 112, &lines(&["SK:A", "SK:KERNEL:PANIC_BEGIN", "SK:B"])).unwrap();
        assert_eq!(colours(&rgb, 0..8), [CYAN]);
        assert_eq!(colours(&rgb, 8..16), [VOID]);
        assert_eq!(colours(&rgb, 16..32), [VOID, TEXT]);
        for row in 2..6 {
            assert_eq!(colours(&rgb, row * 16..row * 16 + 16), [VOID, RED], "row {row}");
        }
        assert_eq!(colours(&rgb, 96..112), [VOID]);
    }

    #[test]
    fn a_screen_too_small_for_text_is_refused() {
        assert!(render_console(64, 20, &[]).unwrap_err().contains("no room"));
    }

    #[test]
    fn the_same_picture_passes_and_one_wrong_pixel_is_named() {
        let want = render_console(64, 64, &lines(&["SK:A"])).unwrap();
        let mut got = want.clone();
        assert_eq!(check_same(&Picture { width: 64, height: 64, rgb: &got }, &want), Ok(()));
        got[(20 * 64 + 5) * 3] ^= 0xFF;
        got[(40 * 64 + 9) * 3] ^= 0xFF;
        let why = check_same(&Picture { width: 64, height: 64, rgb: &got }, &want).unwrap_err();
        assert!(why.starts_with("2 pixel(s) differ; the first is x=5 y=20: "), "{why}");
    }

    #[test]
    fn a_picture_of_another_size_fails() {
        let want = render_console(64, 64, &[]).unwrap();
        assert!(check_same(&Picture { width: 64, height: 63, rgb: &want[..64 * 63 * 3] }, &want).is_err());
    }

    #[test]
    fn reads_the_size_and_the_pixels() {
        let file = ppm(4, 3, |x, y| [x as u8, y as u8, 9]);
        let picture = parse_ppm(&file).expect("a P6 file");
        assert_eq!((picture.width, picture.height), (4, 3));
        assert_eq!(picture.rgb.len(), 36);
        assert_eq!(&picture.rgb[3 * (2 * 4 + 3)..], [3, 2, 9], "the last pixel");
    }

    #[test]
    fn reads_a_header_with_comments_and_other_white_space() {
        let mut file = b"P6\n# made by a test\n2\t1\r\n255\n".to_vec();
        file.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let picture = parse_ppm(&file).expect("still a P6 file");
        assert_eq!((picture.width, picture.height, picture.rgb), (2, 1, &[1, 2, 3, 4, 5, 6][..]));
    }

    #[test]
    fn refuses_what_is_not_a_whole_8_bit_p6_file() {
        let good = ppm(4, 3, |_, _| [1, 2, 3]);
        assert!(parse_ppm(&[]).is_err());
        assert!(parse_ppm(b"P5\n4 3\n255\n").is_err(), "a grey map");
        assert!(parse_ppm(b"P6\n4 3\n65535\n").is_err(), "two bytes per sample");
        assert!(parse_ppm(b"P6\n0 3\n255\n").is_err(), "no pixels");
        assert!(parse_ppm(b"P6\n99999999999999999999 3\n255\n").is_err(), "a size that overflows");
        for len in 0..good.len() {
            assert!(parse_ppm(&good[..len]).is_err(), "length {len}");
        }
        let mut longer = good.clone();
        longer.push(0);
        assert!(parse_ppm(&longer).is_err(), "bytes after the last pixel");
    }

    #[test]
    fn the_painted_screen_passes() {
        let file = ppm(64, 40, painted);
        check_fill(&parse_ppm(&file).unwrap()).expect("band and ground");
    }

    #[test]
    fn a_band_one_row_short_or_one_row_tall_fails() {
        let short = ppm(64, 40, |_, y| if y < BAND_ROWS - 1 { CYAN } else { VOID });
        let why = check_fill(&parse_ppm(&short).unwrap()).expect_err("7 rows");
        assert!(why.contains("x=0 y=7"), "{why}");
        let tall = ppm(64, 40, |_, y| if y <= BAND_ROWS { CYAN } else { VOID });
        let why = check_fill(&parse_ppm(&tall).unwrap()).expect_err("9 rows");
        assert!(why.contains("x=0 y=8"), "{why}");
    }

    #[test]
    fn one_wrong_pixel_fails_and_is_named() {
        let file = ppm(64, 40, |x, y| if (x, y) == (63, 39) { [0, 0, 0] } else { painted(x, y) });
        let why = check_fill(&parse_ppm(&file).unwrap()).expect_err("the last pixel is black");
        assert!(why.contains("x=63 y=39"), "{why}");
        assert!(why.contains("1 pixel"), "{why}");
    }

    #[test]
    fn red_and_blue_swapped_fails() {
        let file = ppm(64, 40, |x, y| {
            let [r, g, b] = painted(x, y);
            [b, g, r]
        });
        let why = check_fill(&parse_ppm(&file).unwrap()).expect_err("the wrong byte order");
        assert!(why.contains("x=0 y=0"), "{why}");
        assert!(why.contains("#ffdc3d"), "{why}");
    }

    #[test]
    fn a_screen_lower_than_the_band_is_all_band() {
        let file = ppm(16, 5, |_, _| CYAN);
        check_fill(&parse_ppm(&file).unwrap()).expect("five rows of band");
    }
}
