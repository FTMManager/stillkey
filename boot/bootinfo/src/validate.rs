//! The kernel's acceptance rule for the handoff block (docs/02 section 6).
//!
//! The rule never trusts a length the block states about itself. The caller
//! supplies the block as a slice of exactly the memory that is mapped for it,
//! and every table is shown to lie inside that slice, with checked
//! arithmetic, before a single byte of the table is read. The crate forbids
//! `unsafe`, so a mistake here is a panic on a bounds check, not a stray read.
//!
//! Tests are in `tests/validate.rs`, `tests/guard_page.rs`, `tests/bounds.rs`
//! and `tests/framebuffer.rs`.

use crate::convert::end_of;
use crate::{
    BLOCK_PHYS_LIMIT, BootInfo, DIRECT_MAP_BASE, FLAG_BOOT_SERVICES_EXITED, FLAG_CMDLINE_PRESENT,
    FLAG_FRAMEBUFFER_PRESENT, Framebuffer, HEADER_SIZE, KIND_LOADER, KIND_UNACCEPTED, KIND_USABLE,
    MAGIC, MAX_BLOCK, MAX_CMDLINE, MAX_RESERVED, MemoryRegion, PAGE_SIZE, PURPOSE_BOOT_INFO,
    PURPOSE_KERNEL_IMAGE, PURPOSE_LOADER_IMAGE, ReservedRange, Table, VERSION_MAJOR,
};

/// Why a handoff was refused. The words are the `reason=` values of
/// `SK:KERNEL:BOOTINFO_REJECTED`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    Block,
    Magic,
    Version,
    Size,
    Flags,
    Layout,
    MemoryMap,
    Reserved,
    Cmdline,
    Framebuffer,
    Bootfs,
}

impl Reject {
    pub const fn as_str(self) -> &'static str {
        match self {
            Reject::Block => "block",
            Reject::Magic => "magic",
            Reject::Version => "version",
            Reject::Size => "size",
            Reject::Flags => "flags",
            Reject::Layout => "layout",
            Reject::MemoryMap => "memory_map",
            Reject::Reserved => "reserved",
            Reject::Cmdline => "cmdline",
            Reject::Framebuffer => "framebuffer",
            Reject::Bootfs => "bootfs",
        }
    }
}

/// A handoff block that passed every check. The tables are views into the
/// block the caller supplied; entries are decoded by value.
#[derive(Clone, Copy, Debug)]
pub struct Accepted<'a> {
    pub header: BootInfo,
    regions: &'a [u8],
    reserved: &'a [u8],
    cmdline: &'a [u8],
}

fn region_at(entry: &[u8]) -> MemoryRegion {
    let mut bytes = [0u8; MemoryRegion::SIZE];
    bytes.copy_from_slice(entry);
    MemoryRegion::decode(&bytes)
}

fn reserved_at(entry: &[u8]) -> ReservedRange {
    let mut bytes = [0u8; ReservedRange::SIZE];
    bytes.copy_from_slice(entry);
    ReservedRange::decode(&bytes)
}

fn regions_in(table: &[u8]) -> impl ExactSizeIterator<Item = MemoryRegion> + Clone + use<'_> {
    table.chunks_exact(MemoryRegion::SIZE).map(region_at)
}

fn reserved_in(table: &[u8]) -> impl ExactSizeIterator<Item = ReservedRange> + Clone + use<'_> {
    table.chunks_exact(ReservedRange::SIZE).map(reserved_at)
}

impl<'a> Accepted<'a> {
    pub fn regions(&self) -> impl ExactSizeIterator<Item = MemoryRegion> + Clone + use<'a> {
        regions_in(self.regions)
    }

    pub fn reserved(&self) -> impl ExactSizeIterator<Item = ReservedRange> + Clone + use<'a> {
        reserved_in(self.reserved)
    }

    pub fn cmdline(&self) -> &'a [u8] {
        self.cmdline
    }

    /// The framebuffer, if the loader passed one. It has passed
    /// [`Framebuffer::is_well_formed`] and overlaps no reserved range.
    pub fn framebuffer(&self) -> Option<Framebuffer> {
        (self.header.flags & FLAG_FRAMEBUFFER_PRESENT != 0).then_some(self.header.framebuffer)
    }

    /// `BOOTFS.IMG` in memory, if the loader read one. Its pages are one
    /// reserved range, so they are no page of [`Self::usable_after_reserve`].
    pub fn bootfs(&self) -> Option<crate::PhysRange> {
        (self.header.flags & crate::FLAG_BOOTFS_PRESENT != 0).then_some(self.header.bootfs)
    }

    /// The number of pages an allocator may take (docs/02 section 7): the
    /// regions of kinds `usable` and `loader`, without the first 1 MiB, the
    /// reserved ranges and the pages of the framebuffer.
    pub fn usable_after_reserve(&self) -> u64 {
        let framebuffer = self.framebuffer().and_then(|fb| fb.phys_pages());
        let mut pages = 0;
        for r in self.regions().filter(|r| r.kind == KIND_USABLE || r.kind == KIND_LOADER) {
            // `check_regions` showed the end does not overflow.
            let (start, end) = (r.base.max(LOW_LIMIT), r.base + r.pages * PAGE_SIZE);
            if start >= end {
                continue;
            }
            // The reserved ranges do not overlap each other, and the
            // framebuffer overlaps none of them, so no page is held twice.
            let mut held = 0;
            for range in self.reserved() {
                held += overlap(start, end, range.base, range.base + range.pages * PAGE_SIZE);
            }
            if let Some((fb_start, fb_end)) = framebuffer {
                held += overlap(start, end, fb_start, fb_end);
            }
            pages += (end - start).saturating_sub(held) / PAGE_SIZE;
        }
        pages
    }
}

/// The first 1 MiB of physical memory is never handed to an allocator.
const LOW_LIMIT: u64 = 0x10_0000;

/// The number of bytes `[a_start, a_end)` and `[b_start, b_end)` share.
fn overlap(a_start: u64, a_end: u64, b_start: u64, b_end: u64) -> u64 {
    a_end.min(b_end).saturating_sub(a_start.max(b_start))
}

/// Checks the pointer and length the loader passed in registers, before
/// anything is read through the pointer. Returns the block's physical address.
///
/// The block must be whole pages of the direct map, wholly below
/// [`BLOCK_PHYS_LIMIT`]: that part of the direct map exists whatever the
/// memory map says, so the kernel can vouch for the mapping before it has
/// read the memory map.
pub fn check_block(virt: u64, len: u64) -> Result<u64, Reject> {
    let phys = virt.checked_sub(DIRECT_MAP_BASE).ok_or(Reject::Block)?;
    if virt % PAGE_SIZE != 0 || phys < PAGE_SIZE {
        return Err(Reject::Block);
    }
    if len == 0 || len % PAGE_SIZE != 0 || len > MAX_BLOCK {
        return Err(Reject::Block);
    }
    let end = phys.checked_add(len).ok_or(Reject::Block)?;
    if end > BLOCK_PHYS_LIMIT {
        return Err(Reject::Block);
    }
    Ok(phys)
}

/// Where a table lies in the block, as byte offsets from its start.
#[derive(Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}

impl Span {
    fn overlaps(self, other: Span) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// Bounds a table descriptor against the block without reading the table:
/// `count * elem` bytes at `table.phys` must lie after the header and inside
/// `total_size`, at an address that is a multiple of `align`.
fn locate(
    table: Table,
    elem: usize,
    align: u64,
    block_phys: u64,
    header_size: u64,
    total_size: u64,
) -> Option<Span> {
    let bytes = table.count.checked_mul(elem as u64)?;
    // The physical range itself must not wrap.
    table.phys.checked_add(bytes)?;
    let start = table.phys.checked_sub(block_phys)?;
    let end = start.checked_add(bytes)?;
    if start < header_size || end > total_size || table.phys % align != 0 {
        return None;
    }
    Some(Span { start: usize::try_from(start).ok()?, end: usize::try_from(end).ok()? })
}

/// Checks a handoff block. `block` is exactly the memory that is mapped for
/// it and `block_phys` its physical address; neither comes from the header.
pub fn validate(block: &[u8], block_phys: u64) -> Result<Accepted<'_>, Reject> {
    // ---- Header ----
    let head: &[u8; HEADER_SIZE] =
        block.get(..HEADER_SIZE).and_then(|h| h.try_into().ok()).ok_or(Reject::Block)?;
    let header = BootInfo::decode(head);

    if header.magic != MAGIC {
        return Err(Reject::Magic);
    }
    if header.version_major != VERSION_MAJOR {
        return Err(Reject::Version);
    }
    let header_size = u64::from(header.header_size);
    let total_size = header.total_size;
    if header_size < HEADER_SIZE as u64
        || header_size % 8 != 0
        || header_size > total_size
        || total_size > block.len() as u64
    {
        return Err(Reject::Size);
    }
    if header.flags & FLAG_BOOT_SERVICES_EXITED == 0 {
        return Err(Reject::Flags);
    }
    if header.direct_map_virt_base != DIRECT_MAP_BASE {
        return Err(Reject::Layout);
    }

    // ---- Memory map ----
    let map = locate(header.memory_map, MemoryRegion::SIZE, 8, block_phys, header_size, total_size)
        .ok_or(Reject::MemoryMap)?;
    if header.memory_map.count == 0 {
        return Err(Reject::MemoryMap);
    }
    let regions = block.get(map.start..map.end).ok_or(Reject::MemoryMap)?;
    check_regions(regions)?;

    // ---- Reserved ranges ----
    let res = locate(header.reserved, ReservedRange::SIZE, 8, block_phys, header_size, total_size)
        .ok_or(Reject::Reserved)?;
    if header.reserved.count > MAX_RESERVED || res.overlaps(map) {
        return Err(Reject::Reserved);
    }
    let reserved = block.get(res.start..res.end).ok_or(Reject::Reserved)?;
    check_reserved(reserved, regions, block_phys, block.len() as u64)?;

    // ---- Command line ----
    let cmd = locate(header.cmdline, 1, 1, block_phys, header_size, total_size)
        .ok_or(Reject::Cmdline)?;
    let flagged = header.flags & FLAG_CMDLINE_PRESENT != 0;
    if header.cmdline.count > MAX_CMDLINE
        || flagged != (header.cmdline.count > 0)
        || cmd.overlaps(map)
        || cmd.overlaps(res)
    {
        return Err(Reject::Cmdline);
    }
    let cmdline = block.get(cmd.start..cmd.end).ok_or(Reject::Cmdline)?;

    // ---- Framebuffer ----
    if header.flags & FLAG_FRAMEBUFFER_PRESENT != 0 {
        check_framebuffer(&header.framebuffer, reserved)?;
    }

    // ---- BOOTFS.IMG ----
    check_bootfs(&header, reserved)?;

    Ok(Accepted { header, regions, reserved, cmdline })
}

/// With the flag: the image has a page-aligned base that is not zero and a
/// size of 1 byte to 32 MiB, and exactly one reserved range has the purpose
/// `bootfs`, which covers exactly the pages of the image. Without the flag no
/// range has that purpose. `reserved` has passed [`check_reserved`], so the
/// range lies in loader memory and overlaps no other range.
fn check_bootfs(header: &BootInfo, reserved: &[u8]) -> Result<(), Reject> {
    let mut ranges = reserved_in(reserved).filter(|r| r.purpose == crate::PURPOSE_BOOTFS);
    if header.flags & crate::FLAG_BOOTFS_PRESENT == 0 {
        return if ranges.next().is_none() { Ok(()) } else { Err(Reject::Bootfs) };
    }
    let crate::PhysRange { base, size } = header.bootfs;
    let well_formed =
        base != 0 && base % PAGE_SIZE == 0 && size != 0 && size <= crate::MAX_BOOTFS;
    let covered = match (ranges.next(), ranges.next()) {
        (Some(range), None) => range.base == base && range.pages == size.div_ceil(PAGE_SIZE),
        _ => false,
    };
    if well_formed && covered { Ok(()) } else { Err(Reject::Bootfs) }
}

/// The record is well formed, and the pages the loader mapped for the buffer
/// are no page of a reserved range: a pixel written through the framebuffer
/// mapping cannot change the kernel image, the boot stack, the page tables,
/// the handoff block or the loader image. `reserved` has passed
/// [`check_reserved`].
fn check_framebuffer(fb: &Framebuffer, reserved: &[u8]) -> Result<(), Reject> {
    if !fb.is_well_formed() {
        return Err(Reject::Framebuffer);
    }
    let (start, end) = fb.phys_pages().ok_or(Reject::Framebuffer)?;
    for r in reserved_in(reserved) {
        if overlap(start, end, r.base, r.base + r.pages * PAGE_SIZE) != 0 {
            return Err(Reject::Framebuffer);
        }
    }
    Ok(())
}

/// Every region is page-aligned, non-empty, of a known kind, and the regions
/// are sorted by base without overlap.
fn check_regions(table: &[u8]) -> Result<(), Reject> {
    let mut prev_end = 0;
    for r in regions_in(table) {
        let end = end_of(r.base, r.pages).ok_or(Reject::MemoryMap)?;
        if r.base % PAGE_SIZE != 0
            || r.pages == 0
            || !(KIND_USABLE..=KIND_UNACCEPTED).contains(&r.kind)
            || r.base < prev_end
        {
            return Err(Reject::MemoryMap);
        }
        prev_end = end;
    }
    Ok(())
}

/// Every reserved range is page-aligned, non-empty, of a known purpose and
/// lies in loader memory; the ranges are sorted without overlap; and exactly
/// one of them is the handoff block itself, at the address and of the length
/// the kernel was handed.
fn check_reserved(
    table: &[u8],
    regions: &[u8],
    block_phys: u64,
    block_len: u64,
) -> Result<(), Reject> {
    let mut prev_end = 0;
    let mut block_entries = 0;
    for r in reserved_in(table) {
        let end = end_of(r.base, r.pages).ok_or(Reject::Reserved)?;
        if r.base % PAGE_SIZE != 0
            || r.pages == 0
            || !(PURPOSE_KERNEL_IMAGE..=PURPOSE_LOADER_IMAGE).contains(&r.purpose)
            || r.base < prev_end
            || !in_loader_memory(regions, r.base, end)
        {
            return Err(Reject::Reserved);
        }
        if r.purpose == PURPOSE_BOOT_INFO {
            if r.base != block_phys || end - r.base != block_len {
                return Err(Reject::Reserved);
            }
            block_entries += 1;
        }
        prev_end = end;
    }
    if block_entries != 1 {
        return Err(Reject::Reserved);
    }
    Ok(())
}

/// True if `[start, end)` is covered, without a gap, by regions of kind
/// `loader`. `regions` has passed [`check_regions`]: sorted, no overlap.
fn in_loader_memory(regions: &[u8], start: u64, end: u64) -> bool {
    let mut cursor = start;
    for r in regions_in(regions) {
        // `check_regions` showed this does not overflow.
        let region_end = r.base + r.pages * PAGE_SIZE;
        if region_end <= cursor {
            continue;
        }
        if r.base > cursor || r.kind != KIND_LOADER {
            return false;
        }
        cursor = region_end;
        if cursor >= end {
            return true;
        }
    }
    false
}
