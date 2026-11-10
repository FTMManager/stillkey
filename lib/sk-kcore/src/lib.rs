//! Kernel logic that needs no hardware, so that the host can test it
//! (D-K-12, docs/10 section 8.5).

#![cfg_attr(not(test), no_std)]
