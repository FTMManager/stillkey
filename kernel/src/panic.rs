//! Kernel panic: three lines on serial, the panic exit value if asked for, halt.

use core::panic::PanicInfo;

use crate::{halt, log, qemu};

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    log!("SK:KERNEL:PANIC_BEGIN");
    match info.location() {
        Some(at) => log!("SK:KERNEL:PANIC at {}:{}: {}", at.file(), at.line(), info.message()),
        None => log!("SK:KERNEL:PANIC at unknown:0: {}", info.message()),
    }
    log!("SK:KERNEL:PANIC_END");
    qemu::exit(qemu::PANIC);
    halt()
}
