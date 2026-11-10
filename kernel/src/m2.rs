//! The boot steps of Milestone 2 (docs/10 section 7).

use stillkey_bootinfo::Accepted;

use crate::{halt, qemu};

pub fn run(_info: &Accepted<'static>) -> ! {
    qemu::exit(qemu::PASS);
    halt()
}
