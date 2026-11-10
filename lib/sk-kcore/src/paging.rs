//! Four-level page tables with 4 KiB and 2 MiB pages.
//!
//! The code reads and writes tables only through [`PhysMem`], so the kernel
//! runs it over the direct map and the host tests run it over plain memory.

pub const PAGE: u64 = 4096;
pub const ENTRIES: usize = 512;
/// The size of a page mapped by a second-level entry.
pub const LARGE_PAGE: u64 = 2 << 20;

pub const PRESENT: u64 = 1 << 0;
pub const WRITABLE: u64 = 1 << 1;
pub const USER: u64 = 1 << 2;
pub const CACHE_DISABLE: u64 = 1 << 4;
/// In a level-3 or level-2 entry: the entry maps 1 GiB or 2 MiB itself.
pub const HUGE: u64 = 1 << 7;
pub const NO_EXECUTE: u64 = 1 << 63;
/// The physical address in an entry.
pub const ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;

/// The first address of the upper half.
pub const KERNEL_HALF: u64 = 0xFFFF_8000_0000_0000;
/// One past the last address of the lower half.
pub const USER_HALF_END: u64 = 0x0000_8000_0000_0000;

/// Access to page-table frames by physical address.
pub trait PhysMem {
    fn table(&mut self, phys: u64) -> &mut [u64; ENTRIES];
}

/// What a mapping allows. Reading is always allowed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub writable: bool,
    pub user: bool,
    pub no_execute: bool,
    pub cache_disable: bool,
}

/// What the processor would do with an address: where it leads and what all
/// four levels together allow (docs/10 section 8.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    /// The physical address of the byte.
    pub phys: u64,
    /// 4 KiB, 2 MiB or 1 GiB.
    pub page_size: u64,
    /// Every level allows writing.
    pub writable: bool,
    /// Every level allows ring 3.
    pub user: bool,
    /// Some level forbids execution.
    pub no_execute: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// No frame for a page table.
    NoMemory,
    /// The address is mapped already.
    AlreadyMapped,
    /// The address lies in neither half.
    NotCanonical,
    /// An address is not a multiple of the page size.
    Unaligned,
}

/// What [`Space::destroy_user_half`] gave back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Freed {
    pub pages: u64,
    pub tables: u64,
}

pub const fn is_canonical(virt: u64) -> bool {
    virt < USER_HALF_END || virt >= KERNEL_HALF
}

/// The index into the table of `level` (4 = top, 1 = last).
const fn index(virt: u64, level: u32) -> usize {
    (virt >> (12 + 9 * (level - 1)) & 0x1FF) as usize
}

/// An address space: the physical address of its top-level table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Space {
    pub root: u64,
}

impl Space {
    /// A new space whose upper half is the upper half of `kernel_root` and
    /// whose lower half is empty. `alloc` hands out one frame per call.
    pub fn with_kernel_half(
        mem: &mut impl PhysMem,
        alloc: &mut dyn FnMut() -> Option<u64>,
        kernel_root: u64,
    ) -> Option<Space> {
        let root = alloc()?;
        let mut upper = [0u64; ENTRIES / 2];
        upper.copy_from_slice(&mem.table(kernel_root)[ENTRIES / 2..]);
        let table = mem.table(root);
        table[..ENTRIES / 2].fill(0);
        table[ENTRIES / 2..].copy_from_slice(&upper);
        Some(Space { root })
    }

    /// Maps one page. Tables that are missing are taken from `alloc`; in the
    /// lower half their entries allow ring 3 and writing, so that the last
    /// level alone decides, and in the upper half they never allow ring 3.
    pub fn map_4k(
        &self,
        mem: &mut impl PhysMem,
        alloc: &mut dyn FnMut() -> Option<u64>,
        virt: u64,
        phys: u64,
        flags: Flags,
    ) -> Result<(), MapError> {
        if !is_canonical(virt) {
            return Err(MapError::NotCanonical);
        }
        if virt % PAGE != 0 || phys & !ADDRESS != 0 {
            return Err(MapError::Unaligned);
        }
        let link = PRESENT | WRITABLE | if virt < USER_HALF_END { USER } else { 0 };
        let mut table = self.root;
        for level in [4, 3, 2] {
            let entry = mem.table(table)[index(virt, level)];
            table = if entry & PRESENT == 0 {
                let new = alloc().ok_or(MapError::NoMemory)?;
                mem.table(new).fill(0);
                mem.table(table)[index(virt, level)] = new | link;
                new
            } else if entry & HUGE != 0 {
                return Err(MapError::AlreadyMapped);
            } else {
                entry & ADDRESS
            };
        }
        let slot = &mut mem.table(table)[index(virt, 1)];
        if *slot & PRESENT != 0 {
            return Err(MapError::AlreadyMapped);
        }
        *slot = phys
            | PRESENT
            | if flags.writable { WRITABLE } else { 0 }
            | if flags.user { USER } else { 0 }
            | if flags.cache_disable { CACHE_DISABLE } else { 0 }
            | if flags.no_execute { NO_EXECUTE } else { 0 };
        Ok(())
    }

    /// Removes the mapping of one 4 KiB page and returns its frame. The
    /// tables stay. The caller invalidates the address in the TLB.
    pub fn unmap_4k(&self, mem: &mut impl PhysMem, virt: u64) -> Option<u64> {
        if !is_canonical(virt) {
            return None;
        }
        let mut table = self.root;
        for level in [4, 3, 2] {
            let entry = mem.table(table)[index(virt, level)];
            if entry & PRESENT == 0 || entry & HUGE != 0 {
                return None;
            }
            table = entry & ADDRESS;
        }
        let slot = &mut mem.table(table)[index(virt, 1)];
        if *slot & PRESENT == 0 {
            return None;
        }
        let frame = *slot & ADDRESS;
        *slot = 0;
        Some(frame)
    }

    /// Walks the tables as the processor does.
    pub fn translate(&self, mem: &mut impl PhysMem, virt: u64) -> Option<Mapping> {
        if !is_canonical(virt) {
            return None;
        }
        let (mut writable, mut user, mut no_execute) = (true, true, false);
        let mut table = self.root;
        for level in [4, 3, 2, 1] {
            let entry = mem.table(table)[index(virt, level)];
            if entry & PRESENT == 0 {
                return None;
            }
            writable &= entry & WRITABLE != 0;
            user &= entry & USER != 0;
            no_execute |= entry & NO_EXECUTE != 0;
            let last = level == 1 || (level != 4 && entry & HUGE != 0);
            if last {
                let page_size = PAGE << (9 * (level - 1));
                let base = entry & ADDRESS & !(page_size - 1);
                return Some(Mapping {
                    phys: base | virt & (page_size - 1),
                    page_size,
                    writable,
                    user,
                    no_execute,
                });
            }
            table = entry & ADDRESS;
        }
        None
    }

    /// Maps 2 MiB with one entry of the second level. Both addresses are
    /// multiples of 2 MiB.
    pub fn map_2m(
        &self,
        mem: &mut impl PhysMem,
        alloc: &mut dyn FnMut() -> Option<u64>,
        virt: u64,
        phys: u64,
        flags: Flags,
    ) -> Result<(), MapError> {
        if !is_canonical(virt) {
            return Err(MapError::NotCanonical);
        }
        if virt % LARGE_PAGE != 0 || phys % LARGE_PAGE != 0 || phys & !ADDRESS != 0 {
            return Err(MapError::Unaligned);
        }
        let link = PRESENT | WRITABLE | if virt < USER_HALF_END { USER } else { 0 };
        let mut table = self.root;
        for level in [4, 3] {
            let entry = mem.table(table)[index(virt, level)];
            table = if entry & PRESENT == 0 {
                let new = alloc().ok_or(MapError::NoMemory)?;
                mem.table(new).fill(0);
                mem.table(table)[index(virt, level)] = new | link;
                new
            } else if entry & HUGE != 0 {
                return Err(MapError::AlreadyMapped);
            } else {
                entry & ADDRESS
            };
        }
        let slot = &mut mem.table(table)[index(virt, 2)];
        if *slot & PRESENT != 0 {
            return Err(MapError::AlreadyMapped);
        }
        *slot = phys
            | PRESENT
            | HUGE
            | if flags.writable { WRITABLE } else { 0 }
            | if flags.user { USER } else { 0 }
            | if flags.cache_disable { CACHE_DISABLE } else { 0 }
            | if flags.no_execute { NO_EXECUTE } else { 0 };
        Ok(())
    }

    /// Gives every upper top-level entry that is still empty a zeroed table,
    /// and returns how many it added. After this no kernel mapping changes a
    /// top-level entry, so every address space made with
    /// [`Space::with_kernel_half`] sees every later kernel mapping
    /// (docs/10 section 7, step 15).
    pub fn fill_kernel_half(
        &self,
        mem: &mut impl PhysMem,
        alloc: &mut dyn FnMut() -> Option<u64>,
    ) -> Result<u64, MapError> {
        let mut added = 0;
        for slot in ENTRIES / 2..ENTRIES {
            if mem.table(self.root)[slot] & PRESENT == 0 {
                let new = alloc().ok_or(MapError::NoMemory)?;
                mem.table(new).fill(0);
                mem.table(self.root)[slot] = new | PRESENT | WRITABLE;
                added += 1;
            }
        }
        Ok(added)
    }

    /// Calls `each` for every mapped page of the upper half, in address
    /// order, with its first address and what all levels together allow.
    /// `Mapping::phys` is the first byte of the page.
    pub fn walk_kernel_half(&self, mem: &mut impl PhysMem, each: &mut dyn FnMut(u64, Mapping)) {
        for slot in ENTRIES / 2..ENTRIES {
            let entry = mem.table(self.root)[slot];
            if entry & PRESENT != 0 {
                let base = 0xFFFF_0000_0000_0000 | (slot as u64) << 39;
                let inherited = Inherited {
                    writable: entry & WRITABLE != 0,
                    user: entry & USER != 0,
                    no_execute: entry & NO_EXECUTE != 0,
                };
                walk_table(mem, entry & ADDRESS, 3, base, inherited, each);
            }
        }
    }

    /// How many upper top-level entries are present.
    pub fn kernel_top_entries(&self, mem: &mut impl PhysMem) -> u64 {
        mem.table(self.root)[ENTRIES / 2..].iter().filter(|&&entry| entry & PRESENT != 0).count() as u64
    }

    /// Empties the lower half: every 4 KiB page and every table below the
    /// top-level table goes to `release`, and the 256 lower entries of the
    /// top-level table become zero. The upper half is not read.
    ///
    /// The caller must not be running on this space: the processor may walk
    /// a table at any time, and a freed table is filled with other data
    /// (docs/10 section 8.3, "Ending a process").
    pub fn destroy_user_half(&self, mem: &mut impl PhysMem, release: &mut dyn FnMut(u64)) -> Freed {
        let mut freed = Freed::default();
        for slot in 0..ENTRIES / 2 {
            let entry = mem.table(self.root)[slot];
            if entry & PRESENT != 0 {
                free_table(mem, entry & ADDRESS, 3, release, &mut freed);
                mem.table(self.root)[slot] = 0;
            }
        }
        freed
    }
}

/// What the levels above a table allow.
#[derive(Clone, Copy)]
struct Inherited {
    writable: bool,
    user: bool,
    no_execute: bool,
}

fn walk_table(
    mem: &mut impl PhysMem,
    table: u64,
    level: u32,
    base: u64,
    above: Inherited,
    each: &mut dyn FnMut(u64, Mapping),
) {
    for slot in 0..ENTRIES {
        let entry = mem.table(table)[slot];
        if entry & PRESENT == 0 {
            continue;
        }
        let page_size = PAGE << (9 * (level - 1));
        let virt = base | slot as u64 * page_size;
        let here = Inherited {
            writable: above.writable && entry & WRITABLE != 0,
            user: above.user && entry & USER != 0,
            no_execute: above.no_execute || entry & NO_EXECUTE != 0,
        };
        if level == 1 || entry & HUGE != 0 {
            each(
                virt,
                Mapping {
                    phys: entry & ADDRESS & !(page_size - 1),
                    page_size,
                    writable: here.writable,
                    user: here.user,
                    no_execute: here.no_execute,
                },
            );
        } else {
            walk_table(mem, entry & ADDRESS, level - 1, virt, here, each);
        }
    }
}

/// Frees the table at `table` of `level` and everything it refers to.
fn free_table(
    mem: &mut impl PhysMem,
    table: u64,
    level: u32,
    release: &mut dyn FnMut(u64),
    freed: &mut Freed,
) {
    for slot in 0..ENTRIES {
        let entry = mem.table(table)[slot];
        if entry & PRESENT == 0 {
            continue;
        }
        if level == 1 {
            release(entry & ADDRESS);
            freed.pages += 1;
        } else if entry & HUGE == 0 {
            free_table(mem, entry & ADDRESS, level - 1, release, freed);
        }
        // A large page is not a frame this code handed out: it is left alone.
    }
    release(table);
    freed.tables += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testmem::{Frames, Memory};

    const USER_RW: Flags = Flags { writable: true, user: true, no_execute: true, cache_disable: false };
    const USER_RX: Flags = Flags { writable: false, user: true, no_execute: false, cache_disable: false };

    fn space(mem: &mut Memory, frames: &mut Frames) -> Space {
        let kernel_root = frames.take().unwrap();
        for slot in 256..512 {
            mem.table(kernel_root)[slot] = (0x9000_0000 + slot as u64 * PAGE) | PRESENT | WRITABLE;
        }
        Space::with_kernel_half(mem, &mut || frames.take(), kernel_root).unwrap()
    }

    #[test]
    fn a_new_space_shares_the_upper_half_and_has_no_lower_half() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let kernel_root = frames.take().unwrap();
        mem.table(kernel_root)[0] = 0x1234_5000 | PRESENT; // the loader's identity map
        mem.table(kernel_root)[256] = 0x7000 | PRESENT | WRITABLE;
        mem.table(kernel_root)[511] = 0x8000 | PRESENT | WRITABLE;
        let space = Space::with_kernel_half(&mut mem, &mut || frames.take(), kernel_root).unwrap();
        assert_ne!(space.root, kernel_root);
        let table = *mem.table(space.root);
        assert!(table[..256].iter().all(|&entry| entry == 0));
        assert_eq!(table[256..], mem.table(kernel_root)[256..]);
    }

    #[test]
    fn a_mapped_page_translates_with_its_flags() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let space = space(&mut mem, &mut frames);
        space.map_4k(&mut mem, &mut || frames.take(), 0x40_0000, 0x55_5000, USER_RX).unwrap();
        space.map_4k(&mut mem, &mut || frames.take(), 0x40_1000, 0x66_6000, USER_RW).unwrap();

        let code = space.translate(&mut mem, 0x40_0123).unwrap();
        assert_eq!(code, Mapping { phys: 0x55_5123, page_size: 4096, writable: false, user: true, no_execute: false });
        let data = space.translate(&mut mem, 0x40_1FFF).unwrap();
        assert_eq!(data, Mapping { phys: 0x66_6FFF, page_size: 4096, writable: true, user: true, no_execute: true });
        assert_eq!(space.translate(&mut mem, 0x40_2000), None);
        assert_eq!(space.translate(&mut mem, 0x3F_FFFF), None);
        assert_eq!(space.translate(&mut mem, 0x0000_8000_0000_0000), None);
    }

    #[test]
    fn the_effective_permission_is_what_every_level_allows() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let space = space(&mut mem, &mut frames);
        let virt = 0x40_0000;
        space.map_4k(&mut mem, &mut || frames.take(), virt, 0x55_5000, Flags { writable: true, user: true, ..Flags::default() }).unwrap();
        let open = space.translate(&mut mem, virt).unwrap();
        assert!(open.writable && open.user && !open.no_execute);

        // The tables on the way to the page: levels 4, 3 and 2.
        let l4 = space.root;
        let l3 = mem.table(l4)[0] & ADDRESS;
        let l2 = mem.table(l3)[0] & ADDRESS;
        for (table, slot) in [(l4, 0), (l3, 0), (l2, 2)] {
            let saved = mem.table(table)[slot];

            mem.table(table)[slot] = saved & !WRITABLE;
            let seen = space.translate(&mut mem, virt).unwrap();
            assert!(!seen.writable && seen.user, "a read-only level makes the page read-only");

            mem.table(table)[slot] = saved & !USER;
            let seen = space.translate(&mut mem, virt).unwrap();
            assert!(!seen.user && seen.writable, "a kernel-only level hides the page from ring 3");

            mem.table(table)[slot] = saved | NO_EXECUTE;
            let seen = space.translate(&mut mem, virt).unwrap();
            assert!(seen.no_execute, "one level that forbids execution is enough");

            mem.table(table)[slot] = saved;
        }
        assert_eq!(space.translate(&mut mem, virt), Some(open));
    }

    #[test]
    fn upper_half_tables_never_allow_ring_3() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let root = frames.take().unwrap();
        let space = Space { root };
        let virt = 0xFFFF_9000_0000_0000;
        // Even if the last level were to ask for it.
        space.map_4k(&mut mem, &mut || frames.take(), virt, 0x77_7000, Flags { user: true, writable: true, ..Flags::default() }).unwrap();
        let seen = space.translate(&mut mem, virt).unwrap();
        assert!(!seen.user && seen.writable);
    }

    #[test]
    fn large_pages_translate_with_their_size() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let root = frames.take().unwrap();
        let (l3, l2) = (frames.take().unwrap(), frames.take().unwrap());
        mem.table(root)[256] = l3 | PRESENT | WRITABLE;
        mem.table(l3)[0] = 0x4000_0000 | PRESENT | WRITABLE | HUGE | NO_EXECUTE; // 1 GiB
        mem.table(l3)[1] = l2 | PRESENT | WRITABLE;
        mem.table(l2)[3] = 0x0A60_0000 | PRESENT | HUGE; // 2 MiB, read-only
        let space = Space { root };

        let big = space.translate(&mut mem, KERNEL_HALF + 0x1234_5678).unwrap();
        assert_eq!((big.phys, big.page_size), (0x4000_0000 + 0x1234_5678, 1 << 30));
        assert!(big.writable && !big.user && big.no_execute);

        let medium = space.translate(&mut mem, KERNEL_HALF + (1 << 30) + 3 * (2 << 20) + 0x1_2345).unwrap();
        assert_eq!((medium.phys, medium.page_size), (0x0A60_0000 + 0x1_2345, 2 << 20));
        assert!(!medium.writable && !medium.no_execute);

        // A page cannot be mapped inside a large one.
        let inside = KERNEL_HALF + 0x20_0000;
        assert_eq!(space.map_4k(&mut mem, &mut || frames.take(), inside, 0x1000, Flags::default()), Err(MapError::AlreadyMapped));
    }

    #[test]
    fn mapping_refuses_what_it_cannot_do() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let space = space(&mut mem, &mut frames);
        let mut alloc = || frames.take();
        assert_eq!(space.map_4k(&mut mem, &mut alloc, 0x40_0000, 0x5000, USER_RW), Ok(()));
        assert_eq!(space.map_4k(&mut mem, &mut alloc, 0x40_0000, 0x6000, USER_RW), Err(MapError::AlreadyMapped));
        assert_eq!(space.map_4k(&mut mem, &mut alloc, 0x0000_8000_0000_0000, 0x6000, USER_RW), Err(MapError::NotCanonical));
        assert_eq!(space.map_4k(&mut mem, &mut alloc, 0x40_1008, 0x6000, USER_RW), Err(MapError::Unaligned));
        assert_eq!(space.map_4k(&mut mem, &mut alloc, 0x40_1000, 0x6008, USER_RW), Err(MapError::Unaligned));
        // Without frames a new table cannot be made, but a page in a table
        // that exists can still be mapped.
        let mut none = || None;
        assert_eq!(space.map_4k(&mut mem, &mut none, 0x8000_0000, 0x6000, USER_RW), Err(MapError::NoMemory));
        assert_eq!(space.map_4k(&mut mem, &mut none, 0x40_1000, 0x6000, USER_RW), Ok(()));
    }

    #[test]
    fn unmapping_returns_the_frame_and_leaves_the_neighbours() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let space = space(&mut mem, &mut frames);
        space.map_4k(&mut mem, &mut || frames.take(), 0x40_0000, 0x5000, USER_RW).unwrap();
        space.map_4k(&mut mem, &mut || frames.take(), 0x40_1000, 0x6000, USER_RW).unwrap();
        assert_eq!(space.unmap_4k(&mut mem, 0x40_0000), Some(0x5000));
        assert_eq!(space.unmap_4k(&mut mem, 0x40_0000), None);
        assert_eq!(space.translate(&mut mem, 0x40_0000), None);
        assert_eq!(space.translate(&mut mem, 0x40_1000).unwrap().phys, 0x6000);
        assert_eq!(space.unmap_4k(&mut mem, 0x7000_0000), None);
    }

    #[test]
    fn destroying_the_lower_half_gives_back_every_page_and_table_once() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(128));
        let space = space(&mut mem, &mut frames);
        let before = frames.taken();

        // A program as the kernel lays it out: four image pages at 4 MiB and
        // sixteen stack pages below the top of the lower half.
        let mut pages = Vec::new();
        let stack_top = 0x0000_7FFF_FFFF_F000u64;
        let image = (0..4).map(|i| 0x40_0000 + i * PAGE);
        let stack = (1..=16).map(|i| stack_top - i * PAGE);
        for virt in image.chain(stack) {
            let frame = frames.take().unwrap();
            pages.push(frame);
            space.map_4k(&mut mem, &mut || frames.take(), virt, frame, USER_RW).unwrap();
        }
        let tables = frames.taken() - before - pages.len() as u64;
        assert_eq!(tables, 6, "three tables below the top level for each of the two areas");

        let upper = mem.table(space.root)[256..].to_vec();
        let mut released = Vec::new();
        let freed = space.destroy_user_half(&mut mem, &mut |frame| released.push(frame));
        assert_eq!(freed, Freed { pages: 20, tables: 6 });

        // Every frame taken since `before` came back, each once, and the
        // top-level table did not.
        let mut expected: Vec<u64> = frames.since(before);
        expected.sort_unstable();
        released.sort_unstable();
        assert_eq!(released, expected);
        assert!(!released.contains(&space.root));
        for frame in &pages {
            assert!(released.contains(frame));
        }

        assert!(mem.table(space.root)[..256].iter().all(|&entry| entry == 0));
        assert_eq!(mem.table(space.root)[256..], upper[..]);
        assert_eq!(space.translate(&mut mem, 0x40_0000), None);
        // A second call finds nothing.
        assert_eq!(space.destroy_user_half(&mut mem, &mut |_| panic!("nothing is left")), Freed::default());
    }
}

#[cfg(test)]
mod kernel_half_tests {
    use super::*;
    use crate::testmem::{Frames, Memory};

    const KERNEL_RW: Flags = Flags { writable: true, user: false, no_execute: true, cache_disable: false };
    const KERNEL_RX: Flags = Flags { writable: false, user: false, no_execute: false, cache_disable: false };
    const DIRECT: u64 = 0xFFFF_8000_0000_0000;
    const TEXT: u64 = 0xFFFF_FFFF_8000_0000;

    fn empty(frames: &mut Frames) -> Space {
        Space { root: frames.take().unwrap() }
    }

    #[test]
    fn a_large_page_translates_with_its_flags() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let space = empty(&mut frames);
        space.map_2m(&mut mem, &mut || frames.take(), DIRECT + 0x20_0000, 0x4020_0000, KERNEL_RW).unwrap();
        assert_eq!(frames.taken(), 3, "the root and two levels of tables");

        let seen = space.translate(&mut mem, DIRECT + 0x21_2345).unwrap();
        assert_eq!(
            seen,
            Mapping { phys: 0x4021_2345, page_size: LARGE_PAGE, writable: true, user: false, no_execute: true }
        );
        assert_eq!(space.translate(&mut mem, DIRECT + 0x1F_FFFF), None);
        assert_eq!(space.translate(&mut mem, DIRECT + 0x40_0000), None);
    }

    #[test]
    fn a_large_page_needs_addresses_on_two_mebibytes() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let space = empty(&mut frames);
        let mut map = |virt, phys| space.map_2m(&mut mem, &mut || frames.take(), virt, phys, KERNEL_RW);
        assert_eq!(map(DIRECT + 0x1000, 0x4020_0000), Err(MapError::Unaligned));
        assert_eq!(map(DIRECT + 0x20_0000, 0x4020_1000), Err(MapError::Unaligned));
        assert_eq!(map(0x0000_8000_0000_0000, 0x4020_0000), Err(MapError::NotCanonical));
    }

    #[test]
    fn large_and_small_pages_do_not_cover_each_other() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let space = empty(&mut frames);
        space.map_2m(&mut mem, &mut || frames.take(), DIRECT, 0x4000_0000, KERNEL_RW).unwrap();
        space.map_4k(&mut mem, &mut || frames.take(), DIRECT + 0x20_0000, 0x5000_0000, KERNEL_RW).unwrap();

        let small_in_large = space.map_4k(&mut mem, &mut || frames.take(), DIRECT + 0x3000, 0x5000_1000, KERNEL_RW);
        assert_eq!(small_in_large, Err(MapError::AlreadyMapped));
        let large_twice = space.map_2m(&mut mem, &mut || frames.take(), DIRECT, 0x4020_0000, KERNEL_RW);
        assert_eq!(large_twice, Err(MapError::AlreadyMapped));
        let large_over_small = space.map_2m(&mut mem, &mut || frames.take(), DIRECT + 0x20_0000, 0x4020_0000, KERNEL_RW);
        assert_eq!(large_over_small, Err(MapError::AlreadyMapped));
        assert_eq!(space.translate(&mut mem, DIRECT).unwrap().phys, 0x4000_0000);
        assert_eq!(space.translate(&mut mem, DIRECT + 0x20_0000).unwrap().phys, 0x5000_0000);
    }

    #[test]
    fn a_filled_kernel_half_never_changes_at_the_top() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(600));
        let space = empty(&mut frames);
        space.map_4k(&mut mem, &mut || frames.take(), TEXT, 0x10_0000, KERNEL_RX).unwrap();
        space.map_2m(&mut mem, &mut || frames.take(), DIRECT, 0x4000_0000, KERNEL_RW).unwrap();
        assert_eq!(space.kernel_top_entries(&mut mem), 2);

        let added = space.fill_kernel_half(&mut mem, &mut || frames.take()).unwrap();
        assert_eq!(added, 254);
        assert_eq!(space.kernel_top_entries(&mut mem), 256);
        let top = *mem.table(space.root);
        assert!(top[..256].iter().all(|&entry| entry == 0), "the lower half stays empty");
        assert!(top[256..].iter().all(|&entry| entry & PRESENT != 0 && entry & USER == 0));

        // Mappings in windows that were empty before: the top level stays.
        for virt in [0xFFFF_C000_0000_0000u64, 0xFFFF_D000_0010_0000, 0xFFFF_F000_0000_0000] {
            space.map_4k(&mut mem, &mut || frames.take(), virt, 0x20_0000, KERNEL_RW).unwrap();
            assert!(space.translate(&mut mem, virt).is_some());
        }
        assert_eq!(*mem.table(space.root), top);
        assert_eq!(space.fill_kernel_half(&mut mem, &mut || frames.take()), Ok(0));
    }

    #[test]
    fn filling_without_memory_is_an_error() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(10));
        let space = empty(&mut frames);
        assert_eq!(space.fill_kernel_half(&mut mem, &mut || frames.take()), Err(MapError::NoMemory));
    }

    #[test]
    fn the_walk_reports_every_kernel_page_and_no_other() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let space = empty(&mut frames);
        let user = Flags { writable: true, user: true, no_execute: true, cache_disable: false };
        space.map_4k(&mut mem, &mut || frames.take(), 0x40_0000, 0x77_7000, user).unwrap();
        space.map_2m(&mut mem, &mut || frames.take(), DIRECT + 0x20_0000, 0x4020_0000, KERNEL_RW).unwrap();
        space.map_4k(&mut mem, &mut || frames.take(), TEXT, 0x10_0000, KERNEL_RX).unwrap();
        space.map_4k(&mut mem, &mut || frames.take(), TEXT + 0x1000, 0x10_1000, KERNEL_RW).unwrap();

        let mut seen = Vec::new();
        space.walk_kernel_half(&mut mem, &mut |virt, mapping| seen.push((virt, mapping)));
        let page = |phys, page_size, writable, no_execute| Mapping { phys, page_size, writable, user: false, no_execute };
        assert_eq!(
            seen,
            [
                (DIRECT + 0x20_0000, page(0x4020_0000, LARGE_PAGE, true, true)),
                (TEXT, page(0x10_0000, PAGE, false, false)),
                (TEXT + 0x1000, page(0x10_1000, PAGE, true, true)),
            ]
        );
        // The walk and the translation agree, page by page.
        for (virt, mapping) in seen {
            assert_eq!(space.translate(&mut mem, virt), Some(mapping));
        }
    }

    #[test]
    fn the_walk_combines_the_levels() {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let space = empty(&mut frames);
        space.map_4k(&mut mem, &mut || frames.take(), TEXT, 0x10_0000, KERNEL_RW).unwrap();
        // A writable last level under a read-only, no-execute top level.
        let top = &mut mem.table(space.root)[511];
        *top = (*top & !WRITABLE) | NO_EXECUTE;
        // A page a level above claims is open to ring 3 stays closed.
        let third = mem.table(space.root)[511] & ADDRESS;
        mem.table(third)[510] |= USER;

        let mut seen = Vec::new();
        space.walk_kernel_half(&mut mem, &mut |virt, mapping| seen.push((virt, mapping)));
        assert_eq!(
            seen,
            [(TEXT, Mapping { phys: 0x10_0000, page_size: PAGE, writable: false, user: false, no_execute: true })]
        );
        assert_eq!(space.translate(&mut mem, TEXT), Some(seen[0].1));
    }
}
