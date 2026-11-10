//! Kernel logic that needs no hardware, so that the host can test it
//! (D-K-12, docs/10 section 8.5). The kernel supplies the memory and the
//! instructions; the decisions are made here.

#![cfg_attr(not(test), no_std)]

pub mod account;
pub mod console;
pub mod crash;
pub mod fault;
pub mod font;
pub mod frames;
pub mod handles;
pub mod heap;
pub mod logfmt;
pub mod paging;
pub mod sched;
pub mod usercopy;
pub mod x86;

/// Processes that can exist at once, ended ones included (D-K-17).
pub const MAX_PROCESSES: usize = 64;
/// Crash records kept.
pub const CRASH_RING: usize = 32;
/// A kernel stack, and the slot of the stack window it is the upper part of.
pub const KSTACK_SIZE: u64 = 32 << 10;
pub const KSTACK_SLOT: u64 = 64 << 10;
pub const KSTACK_BASE: u64 = 0xFFFF_D000_0010_0000;
/// Where the kernel heap grows from.
pub const HEAP_BASE: u64 = 0xFFFF_C000_0000_0000;
/// Where device pages are mapped.
pub const MMIO_BASE: u64 = 0xFFFF_F000_0000_0000;

#[cfg(test)]
mod testmem;
