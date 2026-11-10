//! Minimal 16550 driver: initialise, check that a port answers, blocking write.
//!
//! Loader and kernel both write their log lines through this, by port I/O
//! only, never through a firmware protocol.

#![no_std]

use core::arch::asm;

/// COM1 on a PC.
pub const COM1: u16 = 0x3F8;

/// How many times a status register is polled before a byte is given up.
/// A missing or stuck port therefore costs a bounded delay, never a hang.
const POLL_LIMIT: u32 = 100_000;

const THR: u16 = 0; // transmit holding (write), receive buffer (read)
const IER: u16 = 1; // interrupt enable
const FCR: u16 = 2; // FIFO control
const LCR: u16 = 3; // line control
const MCR: u16 = 4; // modem control
const LSR: u16 = 5; // line status

const LSR_DATA_READY: u8 = 1 << 0;
const LSR_THR_EMPTY: u8 = 1 << 5;

/// A 16550 at a fixed I/O port base.
#[derive(Clone, Copy)]
pub struct Port {
    base: u16,
}

impl Port {
    /// # Safety
    /// The caller runs in ring 0, and `base` is the I/O port base of a 16550
    /// or of nothing at all (reads of an unoccupied port return `0xFF`).
    pub const unsafe fn new(base: u16) -> Port {
        Port { base }
    }

    fn out(&self, reg: u16, value: u8) {
        // SAFETY: `new` promised ring 0 and that these eight ports belong to a
        // 16550 or to nothing. The instruction touches no memory.
        unsafe {
            asm!("out dx, al", in("dx") self.base + reg, in("al") value,
                 options(nomem, nostack, preserves_flags));
        }
    }

    fn inp(&self, reg: u16) -> u8 {
        let value: u8;
        // SAFETY: as in `out`.
        unsafe {
            asm!("in al, dx", in("dx") self.base + reg, out("al") value,
                 options(nomem, nostack, preserves_flags));
        }
        value
    }

    /// Programs 115200 baud, 8 data bits, no parity, 1 stop bit, with the
    /// chip's interrupts off, then checks that a byte sent in loopback mode
    /// comes back. Returns whether a port answered.
    ///
    /// Nothing else may read the port's receive buffer while this runs: a
    /// reader that takes the loopback byte makes a working port look absent.
    /// The kernel is alone on the machine. The loader shares the port with
    /// firmware and must hold firmware off for the length of the call.
    pub fn init(&self) -> bool {
        self.init_with(|| {})
    }

    /// `init`, with `pause` called between sending the loopback byte and
    /// looking for it. A test uses the pause to hold that window open.
    pub fn init_with(&self, pause: impl FnOnce()) -> bool {
        self.out(IER, 0x00); // no interrupts
        self.out(LCR, 0x80); // divisor latch access
        self.out(THR, 0x01); // divisor low byte: 115200 baud
        self.out(IER, 0x00); // divisor high byte
        self.out(LCR, 0x03); // 8N1, divisor latch off
        self.out(FCR, 0xC7); // FIFOs on and cleared
        self.out(MCR, 0x1E); // loopback
        self.out(THR, 0xAE);
        pause();
        let mut echoed = false;
        for _ in 0..POLL_LIMIT {
            if self.inp(LSR) & LSR_DATA_READY != 0 {
                echoed = self.inp(THR) == 0xAE;
                break;
            }
        }
        self.out(MCR, 0x03); // normal operation: DTR and RTS, no interrupt gate
        echoed
    }

    /// Sends one byte, waiting a bounded time for the transmitter.
    pub fn write_byte(&self, byte: u8) {
        for _ in 0..POLL_LIMIT {
            if self.inp(LSR) & LSR_THR_EMPTY != 0 {
                self.out(THR, byte);
                return;
            }
        }
    }

    /// Sends text, turning each `\n` into CR LF.
    pub fn write_str(&self, text: &str) {
        for byte in text.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
    }
}
