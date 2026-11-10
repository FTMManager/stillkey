//! The kernel log on the screen (docs/10 section 7, step 13). The grid and
//! the glyphs are `sk_kcore::console`, which is pure and tested on the host;
//! this file holds the one step that cannot be: turning the framebuffer
//! record's address into a byte slice.

use core::slice;
use core::sync::atomic::{AtomicBool, Ordering};

use sk_kcore::console::Console;
use stillkey_bootinfo::Framebuffer;

use crate::arch::Global;

struct Screen {
    fb: Framebuffer,
    console: Console,
    /// Bytes from the first pixel to the end of the last scan line.
    span: usize,
}

static SCREEN: Global<Option<Screen>> = Global::new(None);
/// Lines are drawn from the moment `start` was called.
static ON: AtomicBool = AtomicBool::new(false);
/// A line is being drawn. A fault in the middle of that prints a banner,
/// which must not come back here.
static BUSY: AtomicBool = AtomicBool::new(false);

/// Lays the grid over the screen. Returns its columns and rows, or `None`
/// if the screen has no room for one cell. `fb` must come from
/// `Accepted::framebuffer`, and the kernel's own tables must be in use.
pub fn init(fb: &Framebuffer) -> Option<(u32, u32)> {
    let console = Console::new(fb)?;
    let span = usize::try_from(fb.span_bytes()?).ok()?;
    // SAFETY: one processor, interrupts off; nothing reads the screen yet.
    unsafe { *SCREEN.get() = Some(Screen { fb: *fb, console, span }) };
    Some((console.cols(), console.rows()))
}

/// Every later log line is drawn too.
pub fn start() {
    ON.store(true, Ordering::Relaxed);
}

/// Draws one line of the log in the colour `0xRRGGBB`.
pub fn write_line(text: &[u8], rgb: u32) {
    if !ON.load(Ordering::Relaxed) || BUSY.swap(true, Ordering::Relaxed) {
        return;
    }
    // SAFETY: one processor, interrupts off, and `BUSY` keeps a banner that
    // interrupts this function out: the reference is the only one.
    if let Some(screen) = unsafe { (*SCREEN.get()).as_mut() } {
        // SAFETY: `validate` accepted the record, so `virt_base` is the
        // framebuffer window. The kernel's tables map `size` bytes of it
        // writable, in its own address space and, through the shared upper
        // half, in every program's; `span` is at most `size`, which is at
        // most 256 MiB. Nothing else in the kernel refers to the window
        // after boot step 5, and the slice ends with this function.
        let buf = unsafe { slice::from_raw_parts_mut(screen.fb.virt_base as *mut u8, screen.span) };
        screen.console.write_line(&screen.fb, buf, text, rgb);
    }
    BUSY.store(false, Ordering::Relaxed);
}
