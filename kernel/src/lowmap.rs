//! Removing the lower-half mappings the loader left for its own last
//! instructions (docs/02 section 9).

use core::arch::asm;

use stillkey_bootinfo::{DIRECT_MAP_BASE, PAGE_SIZE};

/// Bits 12-51 of `CR3`: the physical address of the top-level table.
const CR3_ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;

/// `CR4.PGE`, global pages.
pub const CR4_PGE: u64 = 1 << 7;

pub fn read_cr4() -> u64 {
    let value: u64;
    // SAFETY: ring 0; reading a control register changes nothing.
    unsafe {
        asm!("mov {}, cr4", out(reg) value, options(nomem, nostack));
    }
    value
}

fn read_cr3() -> u64 {
    let value: u64;
    // SAFETY: ring 0; reading a control register changes nothing.
    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nomem, nostack));
    }
    value
}

/// Clears entries 0-255 of the active top-level page table and reloads `CR3`.
/// From here only higher-half addresses are mapped.
///
/// `direct_map_bytes` is the size of the direct map computed from the
/// validated memory map; `expected_root` is what the loader recorded.
pub fn drop_lower_half(direct_map_bytes: u64, expected_root: u64) {
    let cr3 = read_cr3();
    let root = cr3 & CR3_ADDRESS;
    assert!(root == expected_root, "CR3 does not hold the page-table root the loader recorded");
    assert!(
        root.checked_add(PAGE_SIZE).is_some_and(|end| end <= direct_map_bytes),
        "the page-table root is outside the direct map"
    );
    let table = (DIRECT_MAP_BASE + root) as *mut u64;
    // SAFETY: `root` is the table the CPU is using, so it is a whole 4 KiB
    // page of 512 entries, and the check above places it inside the direct
    // map, which the loader mapped writable. Entries 0-255 cover only
    // lower-half addresses. The kernel's code, stack, the handoff block and
    // this table are all reached through higher-half addresses, so nothing
    // in use loses its mapping. Volatile writes: the CPU reads this memory.
    unsafe {
        for index in 0..256 {
            table.add(index).write_volatile(0);
        }
    }
    // SAFETY: ring 0. Reloading the same root discards every cached
    // translation that is not global, and the handoff contract guarantees
    // there are no global ones (CR4.PGE is clear and no entry sets the bit).
    unsafe {
        asm!("mov cr3, {}", in(reg) cr3, options(nostack));
    }
}
