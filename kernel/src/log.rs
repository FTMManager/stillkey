//! The kernel log: COM1 by port I/O, and the screen once the console is on
//! (docs/10 section 7, step 13). Every line a test reads goes through `log!`.

use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

use sk_kcore::console::{LINE_MAX, RED, TEXT};
use sk_serial::{COM1, Port};

use crate::fbcon;

static PRESENT: AtomicBool = AtomicBool::new(false);
/// The lines from now on are the banner of a kernel that stops.
static BANNER: AtomicBool = AtomicBool::new(false);

fn port() -> Port {
    // SAFETY: the kernel runs in ring 0, and 0x3F8 is COM1 on a PC or unoccupied.
    unsafe { Port::new(COM1) }
}

/// Programs the port. Nothing is written later if no port answered.
pub fn init() {
    PRESENT.store(port().init(), Ordering::Relaxed);
}

pub struct Writer;

impl fmt::Write for Writer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if PRESENT.load(Ordering::Relaxed) {
            port().write_str(text);
        }
        Ok(())
    }
}

/// The screen shows the lines from here on in `red`.
pub fn banner() {
    BANNER.store(true, Ordering::Relaxed);
}

/// One line of the log. Its text goes to the serial port as it is
/// formatted; the first `LINE_MAX` bytes are kept for the screen.
pub struct Line {
    len: usize,
    buf: [u8; LINE_MAX],
}

impl Line {
    pub const fn new() -> Line {
        Line { len: 0, buf: [0; LINE_MAX] }
    }

    /// Draws the line on the screen, then ends it on the serial port with
    /// CR LF: whoever reads the line end can rely on the screen showing
    /// the line.
    pub fn finish(&self) {
        let colour = if BANNER.load(Ordering::Relaxed) { RED } else { TEXT };
        fbcon::write_line(&self.buf[..self.len], colour);
        let _ = fmt::Write::write_str(&mut Writer, "\n");
    }
}

impl fmt::Write for Line {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let _ = Writer.write_str(text);
        let take = text.len().min(LINE_MAX - self.len);
        self.buf[self.len..self.len + take].copy_from_slice(&text.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

/// Writes one line.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        use core::fmt::Write as _;
        let mut line = $crate::log::Line::new();
        let _ = write!(line, $($arg)*);
        line.finish();
    }};
}

/// Bytes as lower-case hexadecimal digits.
pub struct Hex<'a>(pub &'a [u8]);

impl fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Bytes as text for a quoted log field: anything outside 0x20-0x7E, the
/// double quote and the backslash are written as `\xNN`.
pub struct Escaped<'a>(pub &'a [u8]);

impl fmt::Display for Escaped<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write as _;
        for &byte in self.0 {
            if (0x20..=0x7E).contains(&byte) && byte != b'"' && byte != b'\\' {
                f.write_char(byte as char)?;
            } else {
                write!(f, "\\x{byte:02X}")?;
            }
        }
        Ok(())
    }
}
