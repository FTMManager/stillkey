//! A panic in the loader is a bug, not a refusal (docs/02 section 11).

use core::panic::PanicInfo;

use crate::{cpu, log, qemu};

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    match info.location() {
        Some(at) => log!("SK:LOADER:PANIC at {}:{}: {}", at.file(), at.line(), info.message()),
        None => log!("SK:LOADER:PANIC at ?:0: {}", info.message()),
    }
    qemu::fail();
    cpu::halt()
}
