//! QEMU's debug-exit device. The kernel writes to it only after the boot
//! option `qemu_exit=on` has been read from an accepted `BootInfo`, so an
//! unknown I/O port is never written on a real machine.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

/// QEMU exits with status `(value << 1) | 1`: 33 for a pass, 37 for a panic.
pub const PASS: u32 = 0x10;
pub const PANIC: u32 = 0x12;

const PORT: u16 = 0xF4;

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Ends the VM with `value` if the boot option asked for it; otherwise returns.
pub fn exit(value: u32) {
    if ENABLED.load(Ordering::Relaxed) {
        // SAFETY: ring 0; the boot option states that port 0xF4 is QEMU's
        // isa-debug-exit device. One 32-bit port write, no memory access.
        unsafe {
            asm!("out dx, eax", in("dx") PORT, in("eax") value,
                 options(nomem, nostack, preserves_flags));
        }
    }
}
