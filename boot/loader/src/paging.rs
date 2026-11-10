//! Step 7: the loader's own 4-level page tables (docs/02 section 5).
//!
//! Complete tables, built in a pool allocated beforehand; firmware's tables
//! are never touched. No entry sets the global bit, and no mapping is both
//! writable and executable.

use sk_elf::Image;
use stillkey_bootinfo::{BOOT_STACK_SIZE, DIRECT_MAP_BASE, FRAMEBUFFER_VIRT, STACK_WINDOW};

use crate::mem::{self, PAGE};

const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const LARGE: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
/// Bits 12-51 of an entry: the physical address it points to.
const ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;
const LARGE_PAGE: u64 = 2 << 20;

/// The boot stack sits one page into the stack window; the page below it is
/// left unmapped as a guard.
pub const BOOT_STACK_VIRT: u64 = STACK_WINDOW + PAGE;

pub struct Plan<'a> {
    /// Size D of the direct map; a multiple of 1 GiB.
    pub direct_map_bytes: u64,
    pub image: &'a Image,
    pub image_phys: u64,
    pub stack_phys: u64,
    /// Physical base and size in bytes.
    pub framebuffer: Option<(u64, u64)>,
    /// The loader's own image, as whole pages.
    pub loader_base: u64,
    pub loader_bytes: u64,
}

pub struct Tables {
    pub root: u64,
    pub pool: u64,
    pub pool_pages: usize,
    pub used: usize,
}

/// An upper bound on the tables below the root that a mapping needs.
fn tables_for(virt: u64, bytes: u64, large: bool) -> u64 {
    if bytes == 0 {
        return 0;
    }
    let last = virt + (bytes - 1);
    let spanned = |shift: u32| (last >> shift) - (virt >> shift) + 1;
    spanned(39) + spanned(30) + if large { 0 } else { spanned(21) }
}

struct Builder {
    pool: u64,
    pages: usize,
    used: usize,
    root: u64,
}

impl Builder {
    fn table(&mut self) -> u64 {
        assert!(self.used < self.pages, "page-table pool exhausted");
        let table = self.pool + self.used as u64 * PAGE;
        self.used += 1;
        table
    }

    /// The table that entry `index` of `table` points to, made if absent.
    fn next(&mut self, table: u64, index: usize) -> u64 {
        // SAFETY: `table` is a page of the pool: 512 zero-initialised entries
        // that firmware's identity mapping makes writable and that only this
        // builder refers to. `index` is below 512.
        let entry = unsafe { (table as *const u64).add(index).read() };
        if entry & PRESENT != 0 {
            assert!(entry & LARGE == 0, "a large page is in the way");
            return entry & ADDRESS;
        }
        let new = self.table();
        // Writable and executable here; the leaf entry decides.
        // SAFETY: as above.
        unsafe { (table as *mut u64).add(index).write(new | PRESENT | WRITABLE) };
        new
    }

    fn map(&mut self, virt: u64, phys: u64, bits: u64, large: bool) {
        let index = |shift: u32| ((virt >> shift) & 0x1FF) as usize;
        let pdpt = self.next(self.root, index(39));
        let pd = self.next(pdpt, index(30));
        let (table, slot, bits) = if large {
            (pd, index(21), bits | LARGE)
        } else {
            (self.next(pd, index(21)), index(12), bits)
        };
        // SAFETY: as in `next`.
        unsafe {
            let entry = (table as *mut u64).add(slot);
            assert!(entry.read() == 0, "an address is mapped twice");
            entry.write(phys | bits | PRESENT);
        }
    }

    /// Maps `bytes` (whole pages) at `virt` to `phys` with 4 KiB pages.
    fn range(&mut self, virt: u64, phys: u64, bytes: u64, bits: u64) {
        let mut offset = 0;
        while offset < bytes {
            self.map(virt + offset, phys + offset, bits, false);
            offset += PAGE;
        }
    }
}

/// Allocates the pool and builds every mapping. `None` if the pool cannot be
/// allocated.
pub fn build(plan: &Plan) -> Option<Tables> {
    let image = plan.image;
    let mut count = 1; // the root
    count += tables_for(DIRECT_MAP_BASE, plan.direct_map_bytes, true);
    count += tables_for(image.base, image.span, false);
    count += tables_for(BOOT_STACK_VIRT, BOOT_STACK_SIZE, false);
    if let Some((_, bytes)) = plan.framebuffer {
        count += tables_for(FRAMEBUFFER_VIRT, bytes, false);
    }
    count += tables_for(plan.loader_base, plan.loader_bytes, false);

    let pages = usize::try_from(count).ok()?;
    let pool = mem::alloc_zeroed(pages)?;
    let mut b = Builder { pool, pages, used: 0, root: 0 };
    b.root = b.table();

    // Direct map: all physical memory below D, writable, never executable.
    let mut phys = 0;
    while phys < plan.direct_map_bytes {
        b.map(DIRECT_MAP_BASE + phys, phys, WRITABLE | NO_EXECUTE, true);
        phys += LARGE_PAGE;
    }

    // Kernel image, by segment flags. Rule V7 admits only these three.
    for segment in image.segments() {
        let bits = match segment.flags {
            5 => 0,                     // text: executable, not writable
            6 => WRITABLE | NO_EXECUTE, // data and bss
            _ => NO_EXECUTE,            // read-only data
        };
        let bytes = (segment.memsz + (PAGE - 1)) & !(PAGE - 1);
        b.range(segment.vaddr, plan.image_phys + (segment.vaddr - image.base), bytes, bits);
    }

    b.range(BOOT_STACK_VIRT, plan.stack_phys, BOOT_STACK_SIZE, WRITABLE | NO_EXECUTE);

    if let Some((base, bytes)) = plan.framebuffer {
        let bytes = (bytes + (PAGE - 1)) & !(PAGE - 1);
        b.range(FRAMEBUFFER_VIRT, base, bytes, WRITABLE | NO_EXECUTE);
    }

    // Transition mapping: the loader's own image at its physical address,
    // read and execute, so the instructions after the CR3 switch are mapped.
    b.range(plan.loader_base, plan.loader_base, plan.loader_bytes, 0);

    Some(Tables { root: b.root, pool, pool_pages: pages, used: b.used })
}
