//! The byte layout of the handoff structures is the ABI: `encode` must
//! produce exactly the bytes of the `#[repr(C)]` struct.

use core::mem::{offset_of, size_of};
use stillkey_bootinfo::*;

fn sample() -> BootInfo {
    BootInfo {
        magic: MAGIC,
        version_major: 0x0102,
        version_minor: 0x0304,
        header_size: 0x0506_0708,
        total_size: 0x1111_2222_3333_4444,
        flags: 0x5555_6666_7777_8888,
        direct_map_virt_base: DIRECT_MAP_BASE,
        page_table_root: 0x9999_AAAA_BBBB_C000,
        memory_map: Table { phys: 0x0101_0101_0101_0101, count: 0x0202_0202_0202_0202 },
        reserved: Table { phys: 0x0303_0303_0303_0303, count: 0x0404_0404_0404_0404 },
        cmdline: Table { phys: 0x0505_0505_0505_0505, count: 0x0606_0606_0606_0606 },
        kernel: KernelImage {
            phys_base: 0x0707_0707_0707_0707,
            virt_base: KERNEL_WINDOW,
            size: 0x0808_0808_0808_0808,
            entry: 0x0909_0909_0909_0909,
            sha256: core::array::from_fn(|i| 0xA0 + i as u8),
        },
        boot_stack: Stack {
            virt_top: 0x0A0A_0A0A_0A0A_0A0A,
            phys_base: 0x0B0B_0B0B_0B0B_0B0B,
            size: 0x0C0C_0C0C_0C0C_0C0C,
        },
        framebuffer: Framebuffer {
            phys_base: 0x0D0D_0D0D_0D0D_0D0D,
            size: 0x0E0E_0E0E_0E0E_0E0E,
            virt_base: FRAMEBUFFER_VIRT,
            width: 0x1011_1213,
            height: 0x1415_1617,
            stride_pixels: 0x1819_1A1B,
            format: 0x1C1D_1E1F,
        },
        acpi_rsdp: 0x2021_2223_2425_2627,
        bootfs: PhysRange { base: 0x2829_2A2B_2C2D_2E2F, size: 0x3031_3233_3435_3637 },
        exit_attempts: 0x3839_3A3B,
        uefi_descriptor_size: 0x3C3D_3E3F,
    }
}

#[test]
fn the_sizes_are_the_ones_the_contract_names() {
    assert_eq!(size_of::<BootInfo>(), 256);
    assert_eq!(size_of::<MemoryRegion>(), 32);
    assert_eq!(size_of::<ReservedRange>(), 24);
    assert_eq!(HEADER_SIZE, 256);
    assert_eq!(MAGIC.to_le_bytes(), *b"STILLKEY");
}

#[test]
fn the_documented_field_offsets_hold() {
    assert_eq!(offset_of!(BootInfo, total_size), 16);
    assert_eq!(offset_of!(BootInfo, direct_map_virt_base), 32);
    assert_eq!(offset_of!(BootInfo, memory_map), 48);
    assert_eq!(offset_of!(BootInfo, reserved), 64);
    assert_eq!(offset_of!(BootInfo, cmdline), 80);
    assert_eq!(offset_of!(BootInfo, kernel), 96);
    assert_eq!(offset_of!(BootInfo, boot_stack), 160);
    assert_eq!(offset_of!(BootInfo, framebuffer), 184);
    assert_eq!(offset_of!(BootInfo, acpi_rsdp), 224);
    assert_eq!(offset_of!(BootInfo, bootfs), 232);
    assert_eq!(offset_of!(BootInfo, exit_attempts), 248);
    assert_eq!(offset_of!(BootInfo, uefi_descriptor_size), 252);
}

#[test]
fn encode_produces_the_bytes_of_the_repr_c_struct() {
    let info = sample();
    // SAFETY: BootInfo is repr(C), 256 bytes, made only of integers and has no padding.
    let raw: [u8; 256] = unsafe { core::mem::transmute(info) };
    assert_eq!(info.encode(), raw);

    let region =
        MemoryRegion { base: 0x1000, pages: 0x77, kind: 3, uefi_type: 4, uefi_attr: 0x8000_0000_0000_000F };
    // SAFETY: as above, 32 bytes.
    let raw: [u8; 32] = unsafe { core::mem::transmute(region) };
    assert_eq!(region.encode(), raw);

    let reserved = ReservedRange { base: 0x2000, pages: 0x88, purpose: 4, _pad: 0 };
    // SAFETY: as above, 24 bytes.
    let raw: [u8; 24] = unsafe { core::mem::transmute(reserved) };
    assert_eq!(reserved.encode(), raw);
}

#[test]
fn decode_reverses_encode() {
    let info = sample();
    assert_eq!(BootInfo::decode(&info.encode()), info);
    let region = MemoryRegion { base: 0x1000, pages: 9, kind: 2, uefi_type: 1, uefi_attr: 7 };
    assert_eq!(MemoryRegion::decode(&region.encode()), region);
    let reserved = ReservedRange { base: 0x2000, pages: 5, purpose: 6, _pad: 0 };
    assert_eq!(ReservedRange::decode(&reserved.encode()), reserved);
}

#[test]
fn the_block_is_laid_out_header_command_line_reserved_regions() {
    let l = block_layout(12, 8, 100).expect("layout");
    assert_eq!(l.cmdline_off, 256);
    assert_eq!(l.reserved_off, 272); // 12 bytes padded to 16
    assert_eq!(l.regions_off, 272 + 8 * 24);
    assert_eq!(l.total_size, 4096); // 464 + 3200 = 3664, rounded up to a page

    let l = block_layout(0, 5, 116).expect("layout");
    assert_eq!((l.cmdline_off, l.reserved_off, l.regions_off), (256, 256, 376));
    assert_eq!(l.total_size, 4096); // 376 + 3712 = 4088: still one page

    let l = block_layout(0, 5, 117).expect("layout");
    assert_eq!(l.total_size, 8192); // 376 + 3744 = 4120: a second page
}

#[test]
fn a_block_layout_that_would_be_too_large_is_refused() {
    assert_eq!(block_layout(usize::MAX, 1, 1), None);
    assert_eq!(block_layout(0, usize::MAX / 8, 1), None);
    assert_eq!(block_layout(0, 1, usize::MAX / 16), None);
    assert_eq!(block_layout(0, 0, (MAX_BLOCK as usize) / 32), None); // header pushes it past the limit
    assert!(block_layout(1024, 32, 1000).is_some());
}
