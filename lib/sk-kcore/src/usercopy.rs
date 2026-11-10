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
    let len = dst.len() as u64;
    check_range(addr, len)?;
    if len == 0 {
        return Ok(());
    }
    // First every page is judged; only then is anything copied.
    let first_page = addr & !(PAGE - 1);
    let last_page = (addr + len - 1) & !(PAGE - 1);
    let mut page = first_page;
    loop {
        let mapping = space.translate(mem, page).ok_or(CopyError::Unmapped)?;
        if !mapping.user {
            return Err(CopyError::NotUser);
        }
        if mapping.page_size != PAGE || !is_memory(mapping.phys) {
            return Err(CopyError::NotMemory);
        }
        if page == last_page {
            break;
        }
        page += PAGE;
    }

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
