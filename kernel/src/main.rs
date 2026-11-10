//! Stillkey kernel, Milestone 1: take the handoff from the loader, validate
//! it, summarise memory, fill the screen, drop the low mappings, report, halt.

#![no_std]
#![no_main]

mod bootcheck;
mod fb;
mod log;
mod lowmap;
mod memsum;
mod panic;
mod qemu;

use core::arch::asm;

use stillkey_bootinfo::Accepted;

use crate::log::{Escaped, Hex};

const PROFILE: &str = if cfg!(debug_assertions) { "dev" } else { "release" };

/// Stops the CPU for good. Interrupts stay off; `hlt` in a loop covers an NMI.
pub fn halt() -> ! {
    loop {
        // SAFETY: ring 0; stopping is the intent. No memory is touched.
        unsafe {
            asm!("cli", "hlt", options(nomem, nostack));
        }
    }
}

/// Entry from the loader (docs/02 section 9). System V AMD64 convention:
/// `RDI` is the address of the handoff block in the direct map and `RSI` the
/// number of bytes mapped for it. Both are checked before the block is read.
#[unsafe(no_mangle)]
pub extern "sysv64" fn _start(block: *const u8, block_len: u64) -> ! {
    log::init();
    log!("SK:KERNEL:START version={} profile={}", env!("CARGO_PKG_VERSION"), PROFILE);
    match bootcheck::accept(block, block_len) {
        Ok(info) => kmain(&info),
        Err(reason) => {
            // The command line inside a rejected block is not trusted, so the
            // debug-exit port is not written: a test sees this line and then
            // its time limit.
            log!("SK:KERNEL:BOOTINFO_REJECTED reason={}", reason.as_str());
            halt()
        }
    }
}

fn has_option(cmdline: &[u8], option: &[u8]) -> bool {
    cmdline.split(|&b| b == b' ').any(|word| word == option)
}

fn kmain(info: &Accepted<'static>) -> ! {
    let h = &info.header;
    log!(
        "SK:KERNEL:BOOTINFO_OK version={}.{} size={} flags={:#x}",
        h.version_major,
        h.version_minor,
        h.total_size,
        h.flags
    );

    let cmdline = info.cmdline();
    log!("SK:KERNEL:CMDLINE \"{}\"", Escaped(cmdline));
    if has_option(cmdline, b"qemu_exit=on") {
        qemu::enable();
    }

    // The loader hands over with global pages off, so that the CR3 reload
    // below leaves no translation behind.
    let pge = (lowmap::read_cr4() & lowmap::CR4_PGE != 0) as u8;
    log!("SK:KERNEL:CR4 pge={pge}");
    assert!(pge == 0, "CR4.PGE is set at entry");

    log!(
        "SK:KERNEL:IMAGE sha256={} phys={:#x} pages={}",
        Hex(&h.kernel.sha256),
        h.kernel.phys_base,
        h.kernel.size / stillkey_bootinfo::PAGE_SIZE
    );

    let direct_map_bytes = memsum::summarise(info);

    // The record was checked with the rest of the handoff; without the flag
    // there is no framebuffer and the record is never looked at.
    match info.framebuffer() {
        Some(screen) => {
            fb::fill(&screen);
            log!(
                "SK:KERNEL:FB_FILLED {}x{} format={}",
                screen.width,
                screen.height,
                fb::format_name(&screen)
            );
        }
        None => log!("SK:KERNEL:FB none"),
    }

    lowmap::drop_lower_half(direct_map_bytes, h.page_table_root);
    log!("SK:KERNEL:LOWMAP_DROPPED");

    if has_option(cmdline, b"test=panic") {
        panic!("test=panic requested");
    }

    log!("SK:KERNEL:M1_OK attempts={}", h.exit_attempts);
    qemu::exit(qemu::PASS);
    halt()
}
