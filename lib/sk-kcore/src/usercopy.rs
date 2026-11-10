//! Reading a program's memory on its behalf.
//!
//! The kernel never dereferences an address a program passed. It looks the
//! address up in the program's own tables, requires that ring 3 may read
//! every page, and copies through its own mapping of the frames. A range
//! with any page that fails is refused before a byte is copied.

use stillkey_abi::{USER_IMAGE_FIRST, USER_STACK_TOP};

use crate::paging::{PAGE, PhysMem, Space};

/// The addresses a program can own: from the lowest image address to the
/// top of its stack.
pub const USER_FIRST: u64 = USER_IMAGE_FIRST;
pub const USER_END: u64 = USER_STACK_TOP;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyError {
    /// The range leaves the addresses a program can own, or wraps.
    Range,
    /// A page of the range is not mapped.
    Unmapped,
    /// A page of the range is mapped for the kernel only.
    NotUser,
    /// A page of the range is not ordinary memory.
    NotMemory,
    /// A page of a range to be written is not writable by the program.
    ReadOnly,
}

/// Whether `addr..addr + len` lies inside the addresses a program can own.
/// An empty range touches no memory and is accepted wherever it points.
pub const fn check_range(addr: u64, len: u64) -> Result<(), CopyError> {
    if len == 0 {
        return Ok(());
    }
    let Some(end) = addr.checked_add(len) else {
        return Err(CopyError::Range);
    };
    if addr < USER_FIRST || end > USER_END {
        return Err(CopyError::Range);
    }
    Ok(())
}

/// Judges every page of `addr..addr + len`: mapped, open to ring 3,
/// ordinary memory, and, for `write`, writable by ring 3 — each as all
/// levels of the tables together allow it.
pub fn check_pages(
    space: &Space,
    mem: &mut impl PhysMem,
    addr: u64,
    len: u64,
    write: bool,
    is_memory: &dyn Fn(u64) -> bool,
) -> Result<(), CopyError> {
    check_range(addr, len)?;
    if len == 0 {
        return Ok(());
    }
    let last_page = (addr + len - 1) & !(PAGE - 1);
    let mut page = addr & !(PAGE - 1);
    loop {
        let mapping = space.translate(mem, page).ok_or(CopyError::Unmapped)?;
        if !mapping.user {
            return Err(CopyError::NotUser);
        }
        if mapping.page_size != PAGE || !is_memory(mapping.phys) {
            return Err(CopyError::NotMemory);
        }
        if write && !mapping.writable {
            return Err(CopyError::ReadOnly);
        }
        if page == last_page {
            return Ok(());
        }
        page += PAGE;
    }
}

/// Copies `src` to where the program in `space` has `addr`. Nothing is
/// written unless the program itself may write every page of the range.
///
/// `write` copies to a physical address inside one frame.
pub fn copy_out(
    space: &Space,
    mem: &mut impl PhysMem,
    addr: u64,
    src: &[u8],
    is_memory: &dyn Fn(u64) -> bool,
    write: &mut dyn FnMut(u64, &[u8]),
) -> Result<(), CopyError> {
    check_pages(space, mem, addr, src.len() as u64, true, is_memory)?;
    let mut done = 0usize;
    while done < src.len() {
        let at = addr + done as u64;
        let take = ((PAGE - at % PAGE) as usize).min(src.len() - done);
        let mapping = space.translate(mem, at).ok_or(CopyError::Unmapped)?;
        write(mapping.phys, &src[done..done + take]);
        done += take;
    }
    Ok(())
}

/// Copies `dst.len()` bytes that the program in `space` has at `addr`.
///
/// `is_memory` tells whether a frame is ordinary memory the kernel manages;
/// `read` copies from a physical address inside one such frame.
pub fn copy_in(
    space: &Space,
    mem: &mut impl PhysMem,
    addr: u64,
    dst: &mut [u8],
    is_memory: &dyn Fn(u64) -> bool,
    read: &mut dyn FnMut(u64, &mut [u8]),
) -> Result<(), CopyError> {
    // First every page is judged; only then is anything copied.
    check_pages(space, mem, addr, dst.len() as u64, false, is_memory)?;

    let mut done = 0usize;
    while done < dst.len() {
        let at = addr + done as u64;
        let in_page = (PAGE - at % PAGE) as usize;
        let take = in_page.min(dst.len() - done);
        // Judged above; a mapping cannot change in between, because the
        // kernel does not run anything else while it serves a call.
        let mapping = space.translate(mem, at).ok_or(CopyError::Unmapped)?;
        read(mapping.phys, &mut dst[done..done + take]);
        done += take;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paging::{ADDRESS, Flags, USER};
    use crate::testmem::{Frames, Memory};

    const USER_RO: Flags = Flags { writable: false, user: true, no_execute: true, cache_disable: false };

    /// A space with two neighbouring user pages in frames that are not
    /// neighbours, filled with `a` and `b`.
    fn two_pages() -> (Memory, Frames, Space) {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let space = Space { root: frames.take().unwrap() };
        for (virt, phys, fill) in [(0x40_0000, 0x90_0000, b'a'), (0x40_1000, 0x70_0000, b'b')] {
            space.map_4k(&mut mem, &mut || frames.take(), virt, phys, USER_RO).unwrap();
            mem.frame(phys, fill);
        }
        (mem, frames, space)
    }

    fn copy(mem: &mut Memory, space: &Space, addr: u64, dst: &mut [u8]) -> Result<(), CopyError> {
        // The data frames are moved out so that `read` can use them while
        // the tables are walked.
        let data = core::mem::take(&mut mem.data);
        let result = copy_in(
            space,
            mem,
            addr,
            dst,
            &|phys| data.contains_key(&(phys & !(PAGE - 1))),
            &mut |phys, out| {
                let offset = (phys % PAGE) as usize;
                out.copy_from_slice(&data[&(phys & !(PAGE - 1))][offset..offset + out.len()]);
            },
        );
        mem.data = data;
        result
    }

    #[test]
    fn the_range_must_lie_where_a_program_can_own_memory() {
        assert_eq!(check_range(0x40_0000, 1), Ok(()));
        assert_eq!(check_range(0x3F_FFFF, 1), Err(CopyError::Range));
        assert_eq!(check_range(0, 16), Err(CopyError::Range));
        assert_eq!(check_range(USER_END - 1, 1), Ok(()));
        assert_eq!(check_range(USER_END - 1, 2), Err(CopyError::Range));
        assert_eq!(check_range(0xFFFF_FFFF_8000_0000, 8), Err(CopyError::Range));
        assert_eq!(check_range(u64::MAX, 2), Err(CopyError::Range));
        assert_eq!(check_range(0x40_0000, u64::MAX), Err(CopyError::Range));
        // Nothing is touched, so nothing can be wrong.
        assert_eq!(check_range(0, 0), Ok(()));
    }

    #[test]
    fn a_copy_follows_the_tables_across_a_page_boundary() {
        let (mut mem, _frames, space) = two_pages();
        let mut dst = [0u8; 6];
        assert_eq!(copy(&mut mem, &space, 0x40_0FFD, &mut dst), Ok(()));
        assert_eq!(&dst, b"aaabbb");
        let mut whole = [0u8; 8192];
        assert_eq!(copy(&mut mem, &space, 0x40_0000, &mut whole), Ok(()));
        assert!(whole[..4096].iter().all(|&b| b == b'a') && whole[4096..].iter().all(|&b| b == b'b'));
        assert_eq!(copy(&mut mem, &space, 0x40_0000, &mut []), Ok(()));
    }

    #[test]
    fn nothing_is_copied_when_a_later_page_is_missing() {
        let (mut mem, _frames, space) = two_pages();
        let mut dst = [b'-'; 6];
        // The range starts in a mapped page and ends in an unmapped one.
        assert_eq!(copy(&mut mem, &space, 0x40_1FFD, &mut dst), Err(CopyError::Unmapped));
        assert_eq!(&dst, b"------");
        assert_eq!(copy(&mut mem, &space, 0x50_0000, &mut dst), Err(CopyError::Unmapped));
    }

    #[test]
    fn a_page_ring_3_cannot_read_is_refused_at_any_level() {
        let (mut mem, _frames, space) = two_pages();
        let mut dst = [b'-'; 4];
        // Kernel-only at the top level, although the last level says user.
        let saved = mem.table(space.root)[0];
        mem.table(space.root)[0] = saved & !USER;
        assert_eq!(copy(&mut mem, &space, 0x40_0000, &mut dst), Err(CopyError::NotUser));
        mem.table(space.root)[0] = saved;
        assert_eq!(copy(&mut mem, &space, 0x40_0000, &mut dst), Ok(()));

        // Kernel-only at the last level.
        let l3 = saved & ADDRESS;
        let l2 = mem.table(l3)[0] & ADDRESS;
        let l1 = mem.table(l2)[2] & ADDRESS;
        mem.table(l1)[1] &= !USER;
        let mut dst = [b'-'; 4];
        assert_eq!(copy(&mut mem, &space, 0x40_0FFE, &mut dst), Err(CopyError::NotUser));
        assert_eq!(&dst, b"----");
    }

    #[test]
    fn a_page_that_is_not_memory_is_refused() {
        let (mut mem, mut frames, space) = two_pages();
        // A user mapping of a device address: no frame stands behind it.
        space.map_4k(&mut mem, &mut || frames.take(), 0x40_2000, 0xFEE0_0000, USER_RO).unwrap();
        let mut dst = [b'-'; 4];
        assert_eq!(copy(&mut mem, &space, 0x40_2000, &mut dst), Err(CopyError::NotMemory));
        assert_eq!(copy(&mut mem, &space, 0x40_1FFE, &mut dst), Err(CopyError::NotMemory));
        assert_eq!(&dst, b"----");
    }

    #[test]
    fn a_kernel_address_is_refused_before_any_table_is_read() {
        let (mut mem, _frames, space) = two_pages();
        let mut dst = [0u8; 4];
        assert_eq!(copy(&mut mem, &space, 0xFFFF_8000_0000_0000, &mut dst), Err(CopyError::Range));
        assert_eq!(copy(&mut mem, &space, 0x1000, &mut dst), Err(CopyError::Range));
    }
}

#[cfg(test)]
mod out_tests {
    use super::*;
    use crate::paging::Flags;
    use crate::testmem::{Frames, Memory};

    const RW: Flags = Flags { writable: true, user: true, no_execute: true, cache_disable: false };
    const RO: Flags = Flags { writable: false, user: true, no_execute: true, cache_disable: false };
    const KERNEL: Flags = Flags { writable: true, user: false, no_execute: true, cache_disable: false };

    /// Three neighbouring pages: writable, writable, read-only; then a
    /// kernel-only page.
    fn pages() -> (Memory, Space) {
        let (mut mem, mut frames) = (Memory::default(), Frames::new(64));
        let space = Space { root: frames.take().unwrap() };
        for (virt, phys, flags) in [
            (0x40_0000, 0x90_0000, RW),
            (0x40_1000, 0x70_0000, RW),
            (0x40_2000, 0x80_0000, RO),
            (0x40_3000, 0x60_0000, KERNEL),
        ] {
            space.map_4k(&mut mem, &mut || frames.take(), virt, phys, flags).unwrap();
        }
        (mem, space)
    }

    fn out(mem: &mut Memory, space: &Space, addr: u64, src: &[u8]) -> (Result<(), CopyError>, Vec<(u64, Vec<u8>)>) {
        let mut writes = Vec::new();
        let result =
            copy_out(space, mem, addr, src, &|_| true, &mut |phys, bytes| {
                writes.push((phys, bytes.to_vec()))
            });
        (result, writes)
    }

    #[test]
    fn a_range_over_two_pages_is_written_in_two_pieces() {
        let (mut mem, space) = pages();
        let (result, writes) = out(&mut mem, &space, 0x40_0FFE, b"abcde");
        assert_eq!(result, Ok(()));
        assert_eq!(writes, [(0x90_0FFE, b"ab".to_vec()), (0x70_0000, b"cde".to_vec())]);
    }

    #[test]
    fn a_read_only_page_anywhere_in_the_range_stops_the_whole_write() {
        let (mut mem, space) = pages();
        let (result, writes) = out(&mut mem, &space, 0x40_1FFF, b"xy");
        assert_eq!(result, Err(CopyError::ReadOnly));
        assert!(writes.is_empty(), "nothing is written before every page is judged");
        let (result, writes) = out(&mut mem, &space, 0x40_2000, b"x");
        assert_eq!((result, writes.len()), (Err(CopyError::ReadOnly), 0));
    }

    #[test]
    fn kernel_pages_unmapped_pages_and_kernel_addresses_are_refused() {
        let (mut mem, space) = pages();
        assert_eq!(out(&mut mem, &space, 0x40_3000, b"x").0, Err(CopyError::NotUser));
        assert_eq!(out(&mut mem, &space, 0x40_4000, b"x").0, Err(CopyError::Unmapped));
        assert_eq!(out(&mut mem, &space, 0xFFFF_FFFF_8000_0000, b"x").0, Err(CopyError::Range));
        assert_eq!(out(&mut mem, &space, u64::MAX, b"xy").0, Err(CopyError::Range));
    }

    #[test]
    fn a_device_frame_is_refused() {
        let (mut mem, space) = pages();
        let result = copy_out(&space, &mut mem, 0x40_0000, b"x", &|phys| phys != 0x90_0000, &mut |_, _| {
            panic!("a device frame was written")
        });
        assert_eq!(result, Err(CopyError::NotMemory));
    }

    #[test]
    fn the_levels_are_combined_for_writing() {
        let (mut mem, space) = pages();
        // The last level says writable; the top level says read-only.
        let top = &mut mem.table(space.root)[0];
        *top &= !crate::paging::WRITABLE;
        assert_eq!(out(&mut mem, &space, 0x40_0000, b"x").0, Err(CopyError::ReadOnly));
        assert_eq!(check_pages(&space, &mut mem, 0x40_0000, 1, false, &|_| true), Ok(()));
    }

    #[test]
    fn check_pages_judges_the_whole_range() {
        let (mut mem, space) = pages();
        assert_eq!(check_pages(&space, &mut mem, 0x40_0000, 0x2000, true, &|_| true), Ok(()));
        assert_eq!(check_pages(&space, &mut mem, 0x40_0000, 0x2001, true, &|_| true), Err(CopyError::ReadOnly));
        assert_eq!(check_pages(&space, &mut mem, 0x40_0000, 0x3000, false, &|_| true), Ok(()));
        assert_eq!(check_pages(&space, &mut mem, 0x40_0000, 0x3001, false, &|_| true), Err(CopyError::NotUser));
        assert_eq!(check_pages(&space, &mut mem, 0, 0, true, &|_| true), Ok(()));
    }
}
