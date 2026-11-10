//! What every Stillkey program links: the entry point, the system calls and
//! a way to print a line. No allocator and no channels yet (docs/10 section 3).

#![no_std]

use core::arch::asm;
use core::fmt;

pub use stillkey_abi as abi;
use stillkey_abi::{
    DEBUG_WRITE_MAX, SYS_ABI_VERSION, SYS_CLOCK, SYS_DEBUG_WRITE, SYS_EXIT, SYS_YIELD, Status,
};

/// One system call: `int 0x80`, number in `RAX`, arguments in `RDI`, `RSI`,
/// `RDX`, `R10`, `R8`. Returns `RAX` and `RDX`.
pub fn syscall(number: u64, args: [u64; 5]) -> (u64, u64) {
    let (first, second): (u64, u64);
    // SAFETY: the kernel's entry for vector 0x80 takes its arguments from
    // these registers, changes no memory of this program except through a
    // pointer argument (which the kernel checks), and returns here. `RCX`
    // and `R11` are documented as undefined after a call. The kernel runs on
    // its own stack.
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") number => first,
            in("rdi") args[0],
            in("rsi") args[1],
            inlateout("rdx") args[2] => second,
            in("r10") args[3],
            in("r8") args[4],
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    (first, second)
}

pub fn abi_version() -> u64 {
    syscall(SYS_ABI_VERSION, [0; 5]).0
}

/// Writes one line of at most 256 bytes to the system log.
pub fn debug_write(text: &[u8]) -> Status {
    let ret = syscall(SYS_DEBUG_WRITE, [text.as_ptr() as u64, text.len() as u64, 0, 0, 0]).0;
    Status::of_return(ret).unwrap_or(Status::InvalidArgs)
}

/// Ends the program.
pub fn exit(code: i32) -> ! {
    syscall(SYS_EXIT, [code as u32 as u64, 0, 0, 0, 0]);
    loop {
        // SAFETY: `exit` does not return; if it ever did, this instruction
        // ends the program with a fault instead of running on.
        unsafe { asm!("ud2", options(nomem, nostack)) }
    }
}

pub fn yield_now() {
    syscall(SYS_YIELD, [0; 5]);
}

/// Nanoseconds since the system started its programs.
pub fn clock() -> u64 {
    syscall(SYS_CLOCK, [0; 5]).0
}

/// A line being put together for [`debug_write`]. Text beyond 256 bytes is cut.
pub struct Line {
    buf: [u8; DEBUG_WRITE_MAX as usize],
    len: usize,
}

impl Line {
    pub const fn new() -> Line {
        Line { buf: [0; DEBUG_WRITE_MAX as usize], len: 0 }
    }

    pub fn send(&self) -> Status {
        debug_write(&self.buf[..self.len])
    }
}

impl Default for Line {
    fn default() -> Line {
        Line::new()
    }
}

impl fmt::Write for Line {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let room = self.buf.len() - self.len;
        let take = text.len().min(room);
        self.buf[self.len..self.len + take].copy_from_slice(&text.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

/// Prints one line: `out!("tick {}", n)`.
#[macro_export]
macro_rules! out {
    ($($arg:tt)*) => {{
        use ::core::fmt::Write as _;
        let mut line = $crate::Line::new();
        let _ = ::core::write!(line, $($arg)*);
        let _ = line.send();
    }};
}

/// Defines the program's entry point. `$main` is a `fn() -> i32`; its result
/// is the exit code.
///
/// The kernel starts a program with `RSP` at the top of its stack and every
/// other register zero. The entry aligns the stack as the calling convention
/// expects at a call and never returns.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[unsafe(no_mangle)]
        #[unsafe(naked)]
        pub unsafe extern "C" fn _start() -> ! {
            ::core::arch::naked_asm!(
                "xor ebp, ebp",
                "and rsp, -16",
                "call {run}",
                "ud2",
                run = sym __still_run,
            )
        }

        extern "C" fn __still_run() -> ! {
            let main: fn() -> i32 = $main;
            $crate::exit(main())
        }
    };
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = debug_write(b"panic");
    exit(101)
}
