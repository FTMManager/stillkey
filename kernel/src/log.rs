//! Serial log on COM1 by port I/O. Every line a test reads goes through `log!`.

use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

use sk_serial::{COM1, Port};

static PRESENT: AtomicBool = AtomicBool::new(false);

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

/// Writes one line, ended by CR LF.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        use core::fmt::Write as _;
        let _ = writeln!($crate::log::Writer, $($arg)*);
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
