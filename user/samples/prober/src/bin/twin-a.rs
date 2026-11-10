//! Probe `twin-a` (docs/10 section 9); the source is `../twin.rs`.

#![no_std]
#![no_main]

#[path = "../twin.rs"]
mod twin;

libstill::entry!(main);

fn main() -> i32 {
    twin::run(0xA5, 0x5B, false)
}
