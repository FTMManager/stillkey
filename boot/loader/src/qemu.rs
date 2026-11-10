//! QEMU's `isa-debug-exit` device, written only when the boot option
//! `qemu_exit=on` has been read (docs/07 section 6).

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

const PORT: u16 = 0xF4;
/// QEMU exits with `(value << 1) | 1`: 35.
const FAILED: u32 = 0x11;

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Reports a loader refusal or failure to the test harness, if asked to.
pub fn fail() {
    if ENABLED.load(Ordering::Relaxed) {
        // SAFETY: ring 0. The option is only given on a VM that has the
        // device at this port; the write touches no memory.
        unsafe {
            asm!("out dx, eax", in("dx") PORT, in("eax") FAILED,
                 options(nomem, nostack, preserves_flags));
        }
    }
}
