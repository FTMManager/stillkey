//! Text from a program, made safe for one line of the system log.

use core::fmt;

/// A program's bytes as they appear after `SK:APP:OUT ... | `: printable
/// ASCII as it is, and a backslash or any other byte as `\xNN` with
/// upper-case digits. A program can therefore not start a new log line or
/// forge an escape (docs/10 section 8.1).
pub struct AppText<'a>(pub &'a [u8]);

impl fmt::Display for AppText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &byte in self.0 {
            if (0x20..=0x7E).contains(&byte) && byte != b'\\' {
                f.write_str(core::str::from_utf8(&[byte]).unwrap_or("?"))?;
            } else {
                write!(f, "\\x{byte:02X}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_text_is_unchanged() {
        assert_eq!(AppText(b"hello from ring 3 abi=2").to_string(), "hello from ring 3 abi=2");
        assert_eq!(AppText(b" ~\"|=").to_string(), " ~\"|=");
        assert_eq!(AppText(b"").to_string(), "");
    }

    #[test]
    fn a_program_cannot_start_a_line_or_forge_an_escape() {
        assert_eq!(AppText(b"a\nSK:KERNEL:M2_OK").to_string(), "a\\x0ASK:KERNEL:M2_OK");
        assert_eq!(AppText(b"\\x0A").to_string(), "\\x5Cx0A");
        assert_eq!(AppText(&[0, 0x1F, 0x7F, 0xFF, b'\r']).to_string(), "\\x00\\x1F\\x7F\\xFF\\x0D");
    }
}
