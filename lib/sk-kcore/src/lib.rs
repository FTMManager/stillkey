//! Kernel logic that needs no hardware, so that the host can test it
//! (D-K-12, docs/10 section 8.5). The kernel supplies the memory and the
//! instructions; the decisions are made here.

#![cfg_attr(not(test), no_std)]

pub mod frames;
pub mod logfmt;
pub mod paging;
pub mod usercopy;
pub mod x86;

#[cfg(test)]
mod testmem;
