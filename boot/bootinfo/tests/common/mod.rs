//! Builds handoff blocks for the tests, the way the loader lays them out.
#![allow(dead_code)]

use stillkey_bootinfo::*;

/// Physical address the test block pretends to live at (16 MiB).
pub const BLOCK_PHYS: u64 = 0x0100_0000;

pub struct Spec {
    pub regions: Vec<MemoryRegion>,
    pub reserved: Vec<ReservedRange>,
    pub cmdline: Vec<u8>,
}

pub fn region(base: u64, pages: u64, kind: u32) -> MemoryRegion {
    MemoryRegion { base, pages, kind, uefi_type: 7, uefi_attr: 0xF }
}

pub fn range(base: u64, pages: u64, purpose: u32) -> ReservedRange {
    ReservedRange { base, pages, purpose, _pad: 0 }
}

/// A memory map shaped like the VM's: the five reserved ranges of Milestone 1
/// lie in one loader region at 16-32 MiB, and the block is the first of them.
pub fn good() -> Spec {
    Spec {
        regions: vec![
            region(0, 0xA0, KIND_USABLE),
            region(0x10_0000, 0xF00, KIND_USABLE),
            region(0x0100_0000, 0x1000, KIND_LOADER),
            region(0x0200_0000, 0x2000, KIND_BOOT_SERVICES),
            region(0x0F00_0000, 0x100, KIND_RUNTIME),
            region(0x8000_0000, 0x1000, KIND_DEVICE),
        ],
        reserved: vec![
            range(BLOCK_PHYS, 1, PURPOSE_BOOT_INFO),
            range(0x0110_0000, 16, PURPOSE_KERNEL_IMAGE),
            range(0x0120_0000, 16, PURPOSE_BOOT_STACK),
            range(0x0130_0000, 12, PURPOSE_PAGE_TABLES),
            range(0x0180_0000, 32, PURPOSE_LOADER_IMAGE),
        ],
        cmdline: b"qemu_exit=on".to_vec(),
    }
}

/// `good()` with a region array that fills the page: 116 valid regions whose
/// last byte is the last byte of the block. Nothing inside the block is
/// invalid, so a table that claims one entry more can only be refused by its
/// bound.
pub fn full_page() -> Spec {
    let mut spec = good();
    spec.cmdline = b"test=one".to_vec(); // 8 bytes, so the regions start at byte 384
    for i in 0..110 {
        spec.regions.push(region(0x1_0000_0000 + i * 0x10_0000, 0x10, KIND_USABLE));
    }
    let layout = block_layout(spec.cmdline.len(), spec.reserved.len(), spec.regions.len())
        .expect("the full page has a layout");
    assert_eq!(layout.regions_off + spec.regions.len() * MemoryRegion::SIZE, 4096);
    spec
}

/// Encodes `spec` into a block of exactly one page, then lets `edit` change
/// the header before it is written.
pub fn build(spec: &Spec, edit: impl FnOnce(&mut BootInfo)) -> Vec<u8> {
    let layout = block_layout(spec.cmdline.len(), spec.reserved.len(), spec.regions.len())
        .expect("the test block has a layout");
    assert_eq!(layout.total_size, 4096, "test blocks are one page");
    let mut block = vec![0u8; layout.total_size];

    block[layout.cmdline_off..layout.cmdline_off + spec.cmdline.len()]
        .copy_from_slice(&spec.cmdline);
    for (i, r) in spec.reserved.iter().enumerate() {
        let at = layout.reserved_off + i * ReservedRange::SIZE;
        block[at..at + ReservedRange::SIZE].copy_from_slice(&r.encode());
    }
    for (i, r) in spec.regions.iter().enumerate() {
        let at = layout.regions_off + i * MemoryRegion::SIZE;
        block[at..at + MemoryRegion::SIZE].copy_from_slice(&r.encode());
    }

    let mut flags = FLAG_BOOT_SERVICES_EXITED;
    if !spec.cmdline.is_empty() {
        flags |= FLAG_CMDLINE_PRESENT;
    }
    let mut header = BootInfo {
        magic: MAGIC,
        version_major: VERSION_MAJOR,
        version_minor: VERSION_MINOR,
        header_size: HEADER_SIZE as u32,
        total_size: layout.total_size as u64,
        flags,
        direct_map_virt_base: DIRECT_MAP_BASE,
        page_table_root: 0x0130_0000,
        memory_map: Table {
            phys: BLOCK_PHYS + layout.regions_off as u64,
            count: spec.regions.len() as u64,
        },
        reserved: Table {
            phys: BLOCK_PHYS + layout.reserved_off as u64,
            count: spec.reserved.len() as u64,
        },
        cmdline: Table {
            phys: BLOCK_PHYS + layout.cmdline_off as u64,
            count: spec.cmdline.len() as u64,
        },
        exit_attempts: 1,
        uefi_descriptor_size: 48,
        ..BootInfo::default()
    };
    edit(&mut header);
    block[..HEADER_SIZE].copy_from_slice(&header.encode());
    block
}
