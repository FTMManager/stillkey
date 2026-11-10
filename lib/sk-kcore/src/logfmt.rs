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

/// How much a program may write to the log in each window of time
/// (docs/10 section 8.1). A write that does not fit is dropped whole and
/// counted; the count is reported once, when its window has ended.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    per_window: u64,
    window_ticks: u64,
    /// The first tick of the current window.
    start: u64,
    used: u64,
    dropped: u64,
}

impl Budget {
    /// `per_window` bytes in every `window_ticks` ticks, counted from `start_tick`.
    pub const fn new(per_window: u64, window_ticks: u64, start_tick: u64) -> Budget {
        assert!(window_ticks != 0);
        Budget { per_window, window_ticks, start: start_tick, used: 0, dropped: 0 }
    }

    /// Moves to the window that holds `tick`. If that ends a window in
    /// which bytes were dropped, returns how many.
    pub fn roll(&mut self, tick: u64) -> Option<u64> {
        let passed = tick.saturating_sub(self.start) / self.window_ticks;
        if passed == 0 {
            return None;
        }
        self.start += passed * self.window_ticks;
        self.used = 0;
        let dropped = core::mem::take(&mut self.dropped);
        (dropped != 0).then_some(dropped)
    }

    /// Whether a write of `len` bytes fits the current window. A write that
    /// does not is counted as dropped. Call [`Budget::roll`] first.
    pub fn take(&mut self, len: u64) -> bool {
        if self.used + len <= self.per_window {
            self.used += len;
            true
        } else {
            self.dropped += len;
            false
        }
    }

    /// Bytes dropped in the current window so far.
    pub const fn pending(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod budget_tests {
    use super::Budget;

    #[test]
    fn seventeen_full_lines_in_one_window_drop_the_last() {
        let mut budget = Budget::new(4096, 100, 0);
        let taken = (0..17).filter(|_| budget.take(256)).count();
        assert_eq!(taken, 16);
        assert_eq!(budget.pending(), 256);
        assert_eq!(budget.roll(99), None, "the window has not ended");
        assert_eq!(budget.roll(100), Some(256));
        assert_eq!(budget.roll(100), None, "reported once");
        assert_eq!(budget.roll(250), None);
        assert!(budget.take(256), "a new window is empty");
    }

    #[test]
    fn a_write_that_does_not_fit_is_dropped_whole() {
        let mut budget = Budget::new(100, 10, 0);
        assert!(budget.take(90));
        assert!(!budget.take(11));
        assert!(budget.take(10), "a smaller write still fits");
        assert!(budget.take(0));
        assert_eq!(budget.roll(10), Some(11));
    }

    #[test]
    fn windows_are_counted_from_the_start_tick() {
        let mut budget = Budget::new(10, 100, 250);
        assert!(budget.take(10));
        assert!(!budget.take(1));
        assert_eq!(budget.roll(349), None);
        assert_eq!(budget.roll(350), Some(1));
        // Several windows at once: the next one starts at 650, not at 700.
        assert!(budget.take(10));
        assert_eq!(budget.roll(700), None);
        assert!(budget.take(10));
        assert!(!budget.take(5));
        assert_eq!(budget.roll(749), None);
        assert_eq!(budget.roll(750), Some(5));
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
