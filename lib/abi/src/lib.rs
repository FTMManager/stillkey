//! The system-call interface shared by the kernel and by ring-3 programs
//! (docs/03 section 5.5, docs/10 section 4). Numbers only: no code runs here.
//!
//! A call is `int 0x80` with the number in `RAX` and arguments in `RDI`,
//! `RSI`, `RDX`, `R10`, `R8`, `R9`. The result is in `RAX`: zero or a
//! positive value, or the negated [`Status`] code. A second result, where a
//! call has one, is in `RDX`.

#![no_std]
#![forbid(unsafe_code)]

/// The interface of Milestone 2: calls 0-9.
pub const ABI_VERSION: u64 = 2;

pub const SYS_ABI_VERSION: u64 = 0;
pub const SYS_DEBUG_WRITE: u64 = 1;
pub const SYS_EXIT: u64 = 2;
pub const SYS_YIELD: u64 = 3;
pub const SYS_SLEEP: u64 = 4;
pub const SYS_CLOCK: u64 = 5;
pub const SYS_MEM_ALLOC: u64 = 6;
pub const SYS_MEM_FREE: u64 = 7;
pub const SYS_HANDLE_CLOSE: u64 = 8;
pub const SYS_DIAG_READ: u64 = 9;

/// Status codes (docs/04 section 3). A failing call returns `-(code)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
    Ok = 0,
    BadHandle = 1,
    WrongType = 2,
    AccessDenied = 3,
    Revoked = 4,
    NotFound = 5,
    InvalidArgs = 6,
    NoMemory = 7,
    PeerClosed = 8,
    TimedOut = 9,
}

impl Status {
    pub const ALL: [Status; 10] = [
        Status::Ok,
        Status::BadHandle,
        Status::WrongType,
        Status::AccessDenied,
        Status::Revoked,
        Status::NotFound,
        Status::InvalidArgs,
        Status::NoMemory,
        Status::PeerClosed,
        Status::TimedOut,
    ];

    /// The name tests and programs print.
    pub const fn name(self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::BadHandle => "BAD_HANDLE",
            Status::WrongType => "WRONG_TYPE",
            Status::AccessDenied => "ACCESS_DENIED",
            Status::Revoked => "REVOKED",
            Status::NotFound => "NOT_FOUND",
            Status::InvalidArgs => "INVALID_ARGS",
            Status::NoMemory => "NO_MEMORY",
            Status::PeerClosed => "PEER_CLOSED",
            Status::TimedOut => "TIMED_OUT",
        }
    }

    /// The value a failing call leaves in `RAX`.
    pub const fn as_return(self) -> u64 {
        (-(self as i64)) as u64
    }

    /// The status a returned `RAX` stands for: `Ok` for zero and for every
    /// positive value, `None` for a negative value that is no status code.
    pub const fn of_return(rax: u64) -> Option<Status> {
        let value = rax as i64;
        if value >= 0 {
            return Some(Status::Ok);
        }
        let mut i = 1;
        while i < Status::ALL.len() {
            if value == -(Status::ALL[i] as i64) {
                return Some(Status::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

/// Rights on a handle (docs/04 section 3).
pub mod rights {
    pub const READ: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    pub const MAP: u32 = 1 << 2;
    pub const LOOKUP: u32 = 1 << 3;
    pub const GRANT: u32 = 1 << 4;
    pub const EXECUTE: u32 = 1 << 5;
    pub const DUPLICATE: u32 = 1 << 6;
    pub const TRANSFER: u32 = 1 << 7;
    pub const INSPECT: u32 = 1 << 8;
    pub const INSPECT_REGISTERS: u32 = 1 << 9;
    pub const REVOKE: u32 = 1 << 10;
    pub const KILL: u32 = 1 << 11;
}

/// A program's first handle in Milestone 2: slot 0, generation 1 (D-K-18).
pub const FIRST_HANDLE: u32 = 0x1000;

pub const PAGE_SIZE: u64 = 4096;
/// Longest text one `debug_write` takes.
pub const DEBUG_WRITE_MAX: u64 = 256;
/// Bytes of `debug_write` text a process may have printed per second.
pub const DEBUG_BUDGET_PER_SECOND: u64 = 4096;
pub const MEM_ALLOC_MAX: u64 = 1 << 20;
pub const DIAG_READ_MAX: u64 = 4096;

/// User address layout (D-K-14). The image lies in
/// `USER_IMAGE_FIRST..USER_IMAGE_END`, `mem_alloc` areas start at
/// `USER_HEAP_BASE`, and the stack ends at `USER_STACK_TOP`.
pub const USER_IMAGE_FIRST: u64 = 0x40_0000;
pub const USER_IMAGE_END: u64 = 0x8000_0000;
pub const USER_HEAP_BASE: u64 = 0x1_0000_0000;
pub const USER_STACK_TOP: u64 = 0x0000_7FFF_FFFF_F000;
pub const USER_STACK_SIZE: u64 = 64 << 10;

/// Memory limit of an App process whose table entry says 0 (D-K-6).
pub const DEFAULT_MEM_LIMIT: u64 = 16 << 20;
pub const MAX_HANDLES_APP: u32 = 256;
pub const MAX_HANDLES_SYSTEM: u32 = 4096;

/// Nanoseconds per timer tick: the clock's step when the TSC is not invariant.
pub const TICK_NS: u64 = 10_000_000;
