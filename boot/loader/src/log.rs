//! Serial log on COM1 by port I/O, never through a firmware protocol, so a
//! line may be written at any point, including after `ExitBootServices`.

use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

use sk_serial::{COM1, Port};

static PRESENT: AtomicBool = AtomicBool::new(false);

fn port() -> Port {
    // SAFETY: UEFI applications run in ring 0, and 0x3F8 is COM1 on a PC or unoccupied.
    unsafe { Port::new(COM1) }
}

/// Programs the port and checks that it answers; `pause` runs between the
/// loopback byte and its read-back.
///
/// Firmware's console driver polls this port for input from a timer event
/// every 20 ms while boot services are active. If that event ran inside the
/// check it would take the loopback byte, and a working port would look
/// absent. The check therefore runs at `TPL_HIGH_LEVEL`, where no event is
/// dispatched and interrupts are off.
fn probe(pause: impl FnOnce()) -> bool {
    // SAFETY: the loader runs at `TPL_APPLICATION`, the lowest level, so this
    // raises the level and never lowers it. The guard restores the previous
    // level when it is dropped at the end of this function. In between there
    // are only port accesses and, for the test, one `Stall`, which the
    // specification allows at this level `[UEFI §7.1, Table 7.3]`; no other boot
    // service is called. The time at this level is a few port accesses when a
    // port answers and a bounded poll when none does.
    let _held = unsafe { uefi::boot::raise_tpl(uefi::boot::Tpl::HIGH_LEVEL) };
    port().init_with(pause)
}

/// Programs the port. Nothing is written later if no port answered.
pub fn init() {
    let present = probe(|| {});
    PRESENT.store(present, Ordering::Relaxed);
    if present {
        // Firmware may have left the line half written: start at column 0.
        port().write_str("\n");
    }
}

/// The boot option `test=serial`: the port check again, with the window
/// between the loopback byte and its read-back held open for 50 ms, more than
/// two periods of firmware's console poll. Returns whether the byte came back.
pub fn recheck() -> bool {
    probe(|| uefi::boot::stall(core::time::Duration::from_millis(50)))
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
