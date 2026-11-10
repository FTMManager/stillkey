//! Following a VM that does not end by itself (docs/10 section 10): the
//! harness reads the serial log while the VM runs, and ends the VM through
//! the monitor once a test has seen enough.

use std::fs;
use std::io::Write;
use std::time::Duration;

use crate::qemu::{self, Vm};
use crate::tests::sk_lines;

/// What a test makes of the `SK:` lines so far.
pub enum Verdict {
    Waiting,
    /// Everything the test waits for is there.
    Done,
    /// The test cannot pass any more; the text says why.
    Failed(String),
}

/// A serial log up to its last line end. The bytes after it are a line
/// that is still being written, or that was cut when the VM was ended.
pub fn whole(raw: &[u8]) -> &[u8] {
    &raw[..raw.iter().rposition(|&b| b == b'\n').map_or(0, |at| at + 1)]
}

/// The whole `SK:` lines of a serial log.
pub fn whole_lines(raw: &[u8]) -> Vec<String> {
    sk_lines(whole(raw)).0
}

/// Starts the VM and calls `step` with the `SK:` lines every 50 ms until it
/// says `Done` or `Failed`; then sends `at_end` to the monitor. Returns
/// QEMU's exit status: 0 if the harness ended the VM, the guest's own
/// status if the guest ended first, 124 or 137 if the time limit did.
/// `Failed` comes back as the error, after the VM has ended.
pub fn follow(vm: &Vm, step: &mut dyn FnMut(&[String]) -> Verdict, at_end: &str) -> Result<i32, String> {
    let mut child = qemu::spawn_monitored(vm)?;
    let serial_log = vm.dir.join("serial.log");
    let mut failure = None;
    loop {
        let lines = whole_lines(&fs::read(&serial_log).unwrap_or_default());
        let verdict = step(&lines);
        if !matches!(verdict, Verdict::Waiting) {
            if let Verdict::Failed(why) = verdict {
                failure = Some(why);
            }
            // The monitor reads its commands from standard input.
            let mut monitor = child.stdin.take().ok_or("QEMU has no standard input")?;
            monitor.write_all(at_end.as_bytes()).map_err(|e| format!("QEMU monitor: {e}"))?;
            break;
        }
        // The time limit, or a guest that ended the VM by itself.
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    match failure {
        Some(why) => Err(why),
        None => Ok(status.code().unwrap_or(-1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_that_is_still_being_written_is_left_out() {
        assert_eq!(whole_lines(b"SK:A one\nSK:B tw"), ["SK:A one"]);
        assert_eq!(whole_lines(b"SK:A one\nSK:B two\n"), ["SK:A one", "SK:B two"]);
    }

    #[test]
    fn a_log_is_cut_after_its_last_line_end() {
        assert_eq!(whole(b"SK:A one\r\nSK:APP:OUT pid=3 name=inspector | table cha"), b"SK:A one\r\n");
        assert_eq!(whole(b"SK:A one\r\n"), b"SK:A one\r\n");
        assert_eq!(whole(b"SK:A on"), b"");
    }

    #[test]
    fn a_log_without_a_line_end_has_no_lines() {
        assert!(whole_lines(b"SK:A on").is_empty());
        assert!(whole_lines(b"").is_empty());
    }
}
