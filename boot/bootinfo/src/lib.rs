//! The loader-to-kernel handoff ABI, version 1 (docs/02 sections 5-9).
//!
//! Loader and kernel both depend on this crate. Everything here is pure code
//! over integers and byte slices: it is tested on the host and contains no
//! `unsafe`. The layout rules: `#[repr(C)]`, fixed-width integers only,
//! little-endian, every address physical unless the field name says `virt`.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod convert;
pub mod fb;
pub mod validate;

pub use validate::{Accepted, Reject, check_block, validate};

pub const MAGIC: u64 = u64::from_le_bytes(*b"STILLKEY");
pub const VERSION_MAJOR: u16 = 1;
pub const VERSION_MINOR: u16 = 0;

pub const PAGE_SIZE: u64 = 4096;
/// Size of the `BootInfo` header in version 1.0.
pub const HEADER_SIZE: usize = 256;

/// Virtual base of the direct map of physical memory.
pub const DIRECT_MAP_BASE: u64 = 0xFFFF_8000_0000_0000;
/// The direct map always covers at least this much, whatever the memory map says.
pub const DIRECT_MAP_MIN: u64 = 4 << 30;
/// Where the next window of the kernel's address layout begins (docs/03 section 3).
pub const DIRECT_MAP_MAX: u64 = 64 << 40;
/// The kernel image window: the top 2 GiB.
pub const KERNEL_WINDOW: u64 = 0xFFFF_FFFF_8000_0000;
/// Kernel stacks window. The boot stack sits one page in, so the first page is its guard.
pub const STACK_WINDOW: u64 = 0xFFFF_D000_0000_0000;
pub const BOOT_STACK_SIZE: u64 = 64 * 1024;
/// Where the loader maps the framebuffer.
pub const FRAMEBUFFER_VIRT: u64 = 0xFFFF_E000_0000_0000;

/// The handoff block lies wholly below this physical address, so that it is
/// inside the part of the direct map that exists whatever the memory map says.
pub const BLOCK_PHYS_LIMIT: u64 = DIRECT_MAP_MIN;
/// Upper bound on the size of the handoff block.
pub const MAX_BLOCK: u64 = 16 << 20;
pub const MAX_CMDLINE: u64 = 1024;
/// Largest `BOOTFS.IMG` the loader hands over (docs/10 section 5.3).
pub const MAX_BOOTFS: u64 = 32 << 20;
pub const MAX_RESERVED: u64 = 32;
/// Upper bound on the size of a framebuffer: larger than any mode the loader
/// would be handed, and it bounds the page tables that map it.
pub const MAX_FRAMEBUFFER: u64 = 256 << 20;

pub const FLAG_BOOT_SERVICES_EXITED: u64 = 1 << 0;
pub const FLAG_FRAMEBUFFER_PRESENT: u64 = 1 << 1;
pub const FLAG_ACPI_RSDP_PRESENT: u64 = 1 << 2;
pub const FLAG_BOOTFS_PRESENT: u64 = 1 << 3;
pub const FLAG_CMDLINE_PRESENT: u64 = 1 << 4;

pub const KIND_USABLE: u32 = 1;
pub const KIND_LOADER: u32 = 2;
pub const KIND_BOOT_SERVICES: u32 = 3;
pub const KIND_ACPI_RECLAIM: u32 = 4;
pub const KIND_ACPI_NVS: u32 = 5;
pub const KIND_RUNTIME: u32 = 6;
pub const KIND_RESERVED: u32 = 7;
pub const KIND_DEVICE: u32 = 8;
pub const KIND_UNUSABLE: u32 = 9;
pub const KIND_PERSISTENT: u32 = 10;
pub const KIND_UNACCEPTED: u32 = 11;

pub const PURPOSE_KERNEL_IMAGE: u32 = 1;
pub const PURPOSE_BOOT_STACK: u32 = 2;
pub const PURPOSE_PAGE_TABLES: u32 = 3;
pub const PURPOSE_BOOT_INFO: u32 = 4;
pub const PURPOSE_BOOTFS: u32 = 5;
pub const PURPOSE_LOADER_IMAGE: u32 = 6;

pub const FB_FORMAT_NONE: u32 = 0;
pub const FB_FORMAT_RGBX: u32 = 1;
pub const FB_FORMAT_BGRX: u32 = 2;

/// The name a memory kind has in the kernel's log, or `None` for an unknown value.
pub const fn kind_name(kind: u32) -> Option<&'static str> {
    Some(match kind {
        KIND_USABLE => "usable",
        KIND_LOADER => "loader",
        KIND_BOOT_SERVICES => "boot_services",
        KIND_ACPI_RECLAIM => "acpi_reclaim",
        KIND_ACPI_NVS => "acpi_nvs",
        KIND_RUNTIME => "runtime",
        KIND_RESERVED => "reserved",
        KIND_DEVICE => "device",
        KIND_UNUSABLE => "unusable",
        KIND_PERSISTENT => "persistent",
        KIND_UNACCEPTED => "unaccepted",
        _ => return None,
    })
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BootInfo {
    pub magic: u64,
    pub version_major: u16,
    pub version_minor: u16,
    /// Size of this struct as the loader was built.
    pub header_size: u32,
    /// The whole boot info block.
    pub total_size: u64,
    pub flags: u64,
    /// Virtual base of the direct map; its size is the rule in docs/02 section 5.
    pub direct_map_virt_base: u64,
    /// Physical address of the top-level page table.
    pub page_table_root: u64,
    /// `[MemoryRegion]`, sorted by base.
    pub memory_map: Table,
    /// `[ReservedRange]`, sorted by base.
    pub reserved: Table,
    /// Bytes; `count` is the length.
    pub cmdline: Table,
    pub kernel: KernelImage,
    pub boot_stack: Stack,
    pub framebuffer: Framebuffer,
    /// 0 if not found.
    pub acpi_rsdp: u64,
    /// Zero in Milestone 1.
    pub bootfs: PhysRange,
    /// Number of `ExitBootServices` calls made.
    pub exit_attempts: u32,
    pub uefi_descriptor_size: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Table {
    pub phys: u64,
    pub count: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhysRange {
    pub base: u64,
    pub size: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryRegion {
    /// Physical, 4 KiB aligned.
    pub base: u64,
    /// 4 KiB pages.
    pub pages: u64,
    /// Stillkey classification (`KIND_*`).
    pub kind: u32,
    /// Original `EFI_MEMORY_TYPE`, kept for diagnosis.
    pub uefi_type: u32,
    /// Original attribute bits.
    pub uefi_attr: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReservedRange {
    pub base: u64,
    pub pages: u64,
    /// `PURPOSE_*`.
    pub purpose: u32,
    pub _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KernelImage {
    pub phys_base: u64,
    pub virt_base: u64,
    pub size: u64,
    pub entry: u64,
    pub sha256: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stack {
    pub virt_top: u64,
    pub phys_base: u64,
    pub size: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Framebuffer {
    pub phys_base: u64,
    pub size: u64,
    pub virt_base: u64,
    pub width: u32,
    pub height: u32,
    /// `PixelsPerScanLine`, which can exceed `width`.
    pub stride_pixels: u32,
    /// `FB_FORMAT_*`.
    pub format: u32,
}

const _: () = assert!(core::mem::size_of::<BootInfo>() == HEADER_SIZE);
const _: () = assert!(core::mem::size_of::<MemoryRegion>() == MemoryRegion::SIZE);
const _: () = assert!(core::mem::size_of::<ReservedRange>() == ReservedRange::SIZE);
const _: () = assert!(core::mem::size_of::<Table>() == 16);
const _: () = assert!(core::mem::size_of::<KernelImage>() == 64);
const _: () = assert!(core::mem::size_of::<Stack>() == 24);
const _: () = assert!(core::mem::size_of::<Framebuffer>() == 40);

/// Writes fields one after another, little-endian. The structs have no
/// padding, so this is the `#[repr(C)]` layout; `tests/layout.rs` holds it to that.
struct Writer<const N: usize> {
    buf: [u8; N],
    at: usize,
}

impl<const N: usize> Writer<N> {
    fn new() -> Self {
        Writer { buf: [0; N], at: 0 }
    }

    fn put(&mut self, bytes: &[u8]) {
        self.buf[self.at..self.at + bytes.len()].copy_from_slice(bytes);
        self.at += bytes.len();
    }

    fn u16(&mut self, v: u16) {
        self.put(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.put(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.put(&v.to_le_bytes());
    }

    fn table(&mut self, t: Table) {
        self.u64(t.phys);
        self.u64(t.count);
    }

    fn finish(self) -> [u8; N] {
        assert!(self.at == N);
        self.buf
    }
}

/// The counterpart of [`Writer`].
struct Reader<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, at: 0 }
    }

    fn take<const N: usize>(&mut self) -> [u8; N] {
        let mut out = [0u8; N];
        out.copy_from_slice(&self.buf[self.at..self.at + N]);
        self.at += N;
        out
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take())
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }

    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take())
    }

    fn table(&mut self) -> Table {
        Table { phys: self.u64(), count: self.u64() }
    }
}

impl BootInfo {
    /// The header as the bytes the loader places at the start of the block.
    pub fn encode(&self) -> [u8; HEADER_SIZE] {
        let mut w = Writer::<HEADER_SIZE>::new();
        w.u64(self.magic);
        w.u16(self.version_major);
        w.u16(self.version_minor);
        w.u32(self.header_size);
        w.u64(self.total_size);
        w.u64(self.flags);
        w.u64(self.direct_map_virt_base);
        w.u64(self.page_table_root);
        w.table(self.memory_map);
        w.table(self.reserved);
        w.table(self.cmdline);
        w.u64(self.kernel.phys_base);
        w.u64(self.kernel.virt_base);
        w.u64(self.kernel.size);
        w.u64(self.kernel.entry);
        w.put(&self.kernel.sha256);
        w.u64(self.boot_stack.virt_top);
        w.u64(self.boot_stack.phys_base);
        w.u64(self.boot_stack.size);
        w.u64(self.framebuffer.phys_base);
        w.u64(self.framebuffer.size);
        w.u64(self.framebuffer.virt_base);
        w.u32(self.framebuffer.width);
        w.u32(self.framebuffer.height);
        w.u32(self.framebuffer.stride_pixels);
        w.u32(self.framebuffer.format);
        w.u64(self.acpi_rsdp);
        w.u64(self.bootfs.base);
        w.u64(self.bootfs.size);
        w.u32(self.exit_attempts);
        w.u32(self.uefi_descriptor_size);
        w.finish()
    }

    /// Reads a version 1.0 header. It checks nothing: see [`validate`].
    pub fn decode(bytes: &[u8; HEADER_SIZE]) -> BootInfo {
        let mut r = Reader::new(bytes);
        BootInfo {
            magic: r.u64(),
            version_major: r.u16(),
            version_minor: r.u16(),
            header_size: r.u32(),
            total_size: r.u64(),
            flags: r.u64(),
            direct_map_virt_base: r.u64(),
            page_table_root: r.u64(),
            memory_map: r.table(),
            reserved: r.table(),
            cmdline: r.table(),
            kernel: KernelImage {
                phys_base: r.u64(),
                virt_base: r.u64(),
                size: r.u64(),
                entry: r.u64(),
                sha256: r.take(),
            },
            boot_stack: Stack { virt_top: r.u64(), phys_base: r.u64(), size: r.u64() },
            framebuffer: Framebuffer {
                phys_base: r.u64(),
                size: r.u64(),
                virt_base: r.u64(),
                width: r.u32(),
                height: r.u32(),
                stride_pixels: r.u32(),
                format: r.u32(),
            },
            acpi_rsdp: r.u64(),
            bootfs: PhysRange { base: r.u64(), size: r.u64() },
            exit_attempts: r.u32(),
            uefi_descriptor_size: r.u32(),
        }
    }
}

impl MemoryRegion {
    pub const SIZE: usize = 32;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut w = Writer::<{ Self::SIZE }>::new();
        w.u64(self.base);
        w.u64(self.pages);
        w.u32(self.kind);
        w.u32(self.uefi_type);
        w.u64(self.uefi_attr);
        w.finish()
    }

    pub fn decode(bytes: &[u8; Self::SIZE]) -> MemoryRegion {
        let mut r = Reader::new(bytes);
        MemoryRegion {
            base: r.u64(),
            pages: r.u64(),
            kind: r.u32(),
            uefi_type: r.u32(),
            uefi_attr: r.u64(),
        }
    }
}

impl ReservedRange {
    pub const SIZE: usize = 24;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut w = Writer::<{ Self::SIZE }>::new();
        w.u64(self.base);
        w.u64(self.pages);
        w.u32(self.purpose);
        w.u32(self._pad);
        w.finish()
    }

    pub fn decode(bytes: &[u8; Self::SIZE]) -> ReservedRange {
        let mut r = Reader::new(bytes);
        ReservedRange { base: r.u64(), pages: r.u64(), purpose: r.u32(), _pad: r.u32() }
    }
}

/// Where each part of the handoff block lies, as byte offsets from its start:
/// header, command line, reserved table, then the region array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockLayout {
    pub cmdline_off: usize,
    pub reserved_off: usize,
    pub regions_off: usize,
    /// Whole pages.
    pub total_size: usize,
}

/// Lays out a block for a command line of `cmdline_len` bytes and tables of
/// the given capacities. `None` if the sizes overflow or exceed [`MAX_BLOCK`].
pub fn block_layout(
    cmdline_len: usize,
    reserved_cap: usize,
    region_cap: usize,
) -> Option<BlockLayout> {
    let page = PAGE_SIZE as usize;
    let cmdline_off = HEADER_SIZE;
    // The tables hold 64-bit fields: keep them 8-byte aligned.
    let reserved_off = cmdline_off.checked_add(cmdline_len)?.checked_add(7)? & !7;
    let regions_off = reserved_off.checked_add(reserved_cap.checked_mul(ReservedRange::SIZE)?)?;
    let end = regions_off.checked_add(region_cap.checked_mul(MemoryRegion::SIZE)?)?;
    let total_size = end.checked_add(page - 1)? & !(page - 1);
    if total_size as u64 > MAX_BLOCK {
        return None;
    }
    Some(BlockLayout { cmdline_off, reserved_off, regions_off, total_size })
}
