//! UEFI memory descriptors to `MemoryRegion`s, and the direct-map size rule
//! (docs/02 sections 5 and 7).

use crate::{
    DIRECT_MAP_MAX, DIRECT_MAP_MIN, KIND_ACPI_NVS, KIND_ACPI_RECLAIM, KIND_BOOT_SERVICES,
    KIND_DEVICE, KIND_LOADER, KIND_PERSISTENT, KIND_RESERVED, KIND_RUNTIME, KIND_UNACCEPTED,
    KIND_UNUSABLE, KIND_USABLE, MemoryRegion, PAGE_SIZE,
};

/// `EFI_MEMORY_NV`: the range is persistent memory.
pub const EFI_MEMORY_NV: u64 = 0x8000;
/// `EFI_MEMORY_SP`: specific-purpose memory.
pub const EFI_MEMORY_SP: u64 = 0x4_0000;
/// Size of `EFI_MEMORY_DESCRIPTOR` itself; firmware may report a larger stride.
pub const EFI_DESCRIPTOR_SIZE: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertError {
    /// The descriptor size is below 40 or does not divide the buffer.
    Stride,
    /// More descriptors than the output holds.
    Capacity,
    /// A descriptor is not page-aligned or its end overflows.
    Range,
}

/// The direct map would have to exceed [`crate::DIRECT_MAP_MAX`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectMapTooLarge;

/// The Stillkey kind of a UEFI memory type with the given attributes.
pub fn kind_of(uefi_type: u32, attr: u64) -> u32 {
    match uefi_type {
        1 | 2 => KIND_LOADER,
        3 | 4 => KIND_BOOT_SERVICES,
        5 | 6 => KIND_RUNTIME,
        // Conventional memory the firmware set aside for a special purpose, or
        // that is persistent, is not general-purpose RAM.
        7 if attr & (EFI_MEMORY_SP | EFI_MEMORY_NV) != 0 => KIND_RESERVED,
        7 => KIND_USABLE,
        8 => KIND_UNUSABLE,
        9 => KIND_ACPI_RECLAIM,
        10 => KIND_ACPI_NVS,
        11 | 12 => KIND_DEVICE,
        14 => KIND_PERSISTENT,
        15 => KIND_UNACCEPTED,
        // Reserved (0), PAL code (13) and every type this table does not know.
        _ => KIND_RESERVED,
    }
}

/// The end address of `pages` pages at `base`, or `None` if it overflows.
pub fn end_of(base: u64, pages: u64) -> Option<u64> {
    base.checked_add(pages.checked_mul(PAGE_SIZE)?)
}

fn field<const N: usize>(desc: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&desc[at..at + N]);
    out
}

/// Converts the descriptors in `raw`, stepping by `desc_size`, into `out`.
/// Descriptors of zero pages are dropped. Returns the number written.
pub fn convert(
    raw: &[u8],
    desc_size: usize,
    out: &mut [MemoryRegion],
) -> Result<usize, ConvertError> {
    if desc_size < EFI_DESCRIPTOR_SIZE || raw.len() % desc_size != 0 {
        return Err(ConvertError::Stride);
    }
    let mut n = 0;
    for desc in raw.chunks_exact(desc_size) {
        let uefi_type = u32::from_le_bytes(field(desc, 0));
        let base = u64::from_le_bytes(field(desc, 8));
        let pages = u64::from_le_bytes(field(desc, 24));
        let uefi_attr = u64::from_le_bytes(field(desc, 32));
        if pages == 0 {
            continue;
        }
        if base % PAGE_SIZE != 0 || end_of(base, pages).is_none() {
            return Err(ConvertError::Range);
        }
        let slot = out.get_mut(n).ok_or(ConvertError::Capacity)?;
        *slot = MemoryRegion { base, pages, kind: kind_of(uefi_type, uefi_attr), uefi_type, uefi_attr };
        n += 1;
    }
    Ok(n)
}

/// Sorts by base address, in place, without allocating.
pub fn sort_by_base(regions: &mut [MemoryRegion]) {
    // Insertion sort: firmware maps are short and nearly sorted already.
    for i in 1..regions.len() {
        let mut j = i;
        while j > 0 && regions[j - 1].base > regions[j].base {
            regions.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// The index of the first region that overlaps its predecessor, in a slice
/// sorted by base.
pub fn first_overlap(regions: &[MemoryRegion]) -> Option<usize> {
    (1..regions.len()).find(|&i| {
        let prev = regions[i - 1];
        // An end that overflows overlaps everything after it.
        end_of(prev.base, prev.pages).is_none_or(|end| end > regions[i].base)
    })
}

/// The size of the direct map for a memory map: the highest end address among
/// regions of kinds 1-6 and 10, at least 4 GiB, rounded up to 1 GiB.
pub fn direct_map_size<I>(regions: I) -> Result<u64, DirectMapTooLarge>
where
    I: IntoIterator<Item = MemoryRegion>,
{
    const GIB: u64 = 1 << 30;
    let mut top = DIRECT_MAP_MIN;
    for r in regions {
        let mapped = matches!(
            r.kind,
            KIND_USABLE
                | KIND_LOADER
                | KIND_BOOT_SERVICES
                | KIND_ACPI_RECLAIM
                | KIND_ACPI_NVS
                | KIND_RUNTIME
                | KIND_PERSISTENT
        );
        if mapped {
            top = top.max(end_of(r.base, r.pages).ok_or(DirectMapTooLarge)?);
        }
    }
    let size = top.checked_add(GIB - 1).ok_or(DirectMapTooLarge)? & !(GIB - 1);
    if size > DIRECT_MAP_MAX {
        return Err(DirectMapTooLarge);
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    const GIB: u64 = 1 << 30;

    /// One raw descriptor of `stride` bytes; the bytes past 40 are filler.
    fn raw(stride: usize, ty: u32, phys: u64, pages: u64, attr: u64) -> Vec<u8> {
        let mut d = vec![0xEEu8; stride];
        d[0..4].copy_from_slice(&ty.to_le_bytes());
        d[4..8].fill(0);
        d[8..16].copy_from_slice(&phys.to_le_bytes());
        d[16..24].copy_from_slice(&0u64.to_le_bytes());
        d[24..32].copy_from_slice(&pages.to_le_bytes());
        d[32..40].copy_from_slice(&attr.to_le_bytes());
        d
    }

    fn region(base: u64, pages: u64, kind: u32) -> MemoryRegion {
        MemoryRegion { base, pages, kind, uefi_type: 0, uefi_attr: 0 }
    }

    #[test]
    fn converts_stepping_by_the_reported_descriptor_size() {
        // OVMF reports 48 bytes for the 40-byte structure.
        let mut buf = raw(48, 7, 0x10_0000, 0x700, 0xF);
        buf.extend(raw(48, 2, 0x80_0000, 0x10, 0xF));
        buf.extend(raw(48, 11, 0x8000_0000, 0x100, 0x8000_0000_0000_0001));
        let mut out = [MemoryRegion::default(); 8];
        assert_eq!(convert(&buf, 48, &mut out), Ok(3));
        assert_eq!(
            out[0],
            MemoryRegion { base: 0x10_0000, pages: 0x700, kind: KIND_USABLE, uefi_type: 7, uefi_attr: 0xF }
        );
        assert_eq!(out[1].kind, KIND_LOADER);
        assert_eq!(
            out[2],
            MemoryRegion {
                base: 0x8000_0000,
                pages: 0x100,
                kind: KIND_DEVICE,
                uefi_type: 11,
                uefi_attr: 0x8000_0000_0000_0001
            }
        );
    }

    #[test]
    fn a_wrong_stride_is_an_error_not_a_misread() {
        let buf = raw(48, 7, 0, 1, 0);
        let mut out = [MemoryRegion::default(); 2];
        assert_eq!(convert(&buf, 39, &mut out), Err(ConvertError::Stride));
        assert_eq!(convert(&buf, 40, &mut out), Err(ConvertError::Stride)); // 48 % 40 != 0
        assert_eq!(convert(&buf, 0, &mut out), Err(ConvertError::Stride));
    }

    #[test]
    fn more_descriptors_than_capacity_is_an_error() {
        let mut buf = raw(40, 7, 0, 1, 0);
        buf.extend(raw(40, 7, 0x1000, 1, 0));
        let mut out = [MemoryRegion::default(); 1];
        assert_eq!(convert(&buf, 40, &mut out), Err(ConvertError::Capacity));
    }

    #[test]
    fn empty_descriptors_are_dropped_and_bad_ranges_refused() {
        let mut buf = raw(40, 7, 0x1000, 0, 0);
        buf.extend(raw(40, 7, 0x2000, 1, 0));
        let mut out = [MemoryRegion::default(); 4];
        assert_eq!(convert(&buf, 40, &mut out), Ok(1));
        assert_eq!(out[0].base, 0x2000);

        let unaligned = raw(40, 7, 0x2800, 1, 0);
        assert_eq!(convert(&unaligned, 40, &mut out), Err(ConvertError::Range));
        let overflow = raw(40, 7, 0xFFFF_FFFF_FFFF_F000, 2, 0);
        assert_eq!(convert(&overflow, 40, &mut out), Err(ConvertError::Range));
        let huge = raw(40, 7, 0, u64::MAX, 0);
        assert_eq!(convert(&huge, 40, &mut out), Err(ConvertError::Range));
    }

    #[test]
    fn every_uefi_type_maps_to_its_kind() {
        let table = [
            (0, KIND_RESERVED),
            (1, KIND_LOADER),
            (2, KIND_LOADER),
            (3, KIND_BOOT_SERVICES),
            (4, KIND_BOOT_SERVICES),
            (5, KIND_RUNTIME),
            (6, KIND_RUNTIME),
            (7, KIND_USABLE),
            (8, KIND_UNUSABLE),
            (9, KIND_ACPI_RECLAIM),
            (10, KIND_ACPI_NVS),
            (11, KIND_DEVICE),
            (12, KIND_DEVICE),
            (13, KIND_RESERVED),
            (14, KIND_PERSISTENT),
            (15, KIND_UNACCEPTED),
            (16, KIND_RESERVED),
            (0x7000_0001, KIND_RESERVED),
            (0x8000_0000, KIND_RESERVED),
        ];
        for (ty, kind) in table {
            assert_eq!(kind_of(ty, 0), kind, "uefi type {ty:#x}");
        }
    }

    #[test]
    fn conventional_memory_marked_special_purpose_or_non_volatile_is_reserved() {
        assert_eq!(kind_of(7, EFI_MEMORY_SP), KIND_RESERVED);
        assert_eq!(kind_of(7, EFI_MEMORY_NV | 0xF), KIND_RESERVED);
        assert_eq!(kind_of(7, 0xF), KIND_USABLE);
        // The attributes change nothing for other types.
        assert_eq!(kind_of(2, EFI_MEMORY_SP), KIND_LOADER);
    }

    #[test]
    fn sorts_by_base_and_reports_the_first_overlap() {
        let mut r = [
            region(0x30_0000, 0x10, 1),
            region(0x10_0000, 0x10, 2),
            region(0x20_0000, 0x100, 3),
        ];
        sort_by_base(&mut r);
        assert_eq!([r[0].base, r[1].base, r[2].base], [0x10_0000, 0x20_0000, 0x30_0000]);
        assert_eq!(r[0].kind, 2);
        assert_eq!(first_overlap(&r), None); // 0x20_0000 + 0x100 pages ends exactly at 0x30_0000

        r[1].pages = 0x101;
        assert_eq!(first_overlap(&r), Some(2));
        assert_eq!(first_overlap(&r[..2]), None);
        assert_eq!(first_overlap(&[]), None);
    }

    #[test]
    fn direct_map_is_4_gib_when_memory_ends_below_that() {
        let map = [region(0, 0xA0, 1), region(0x10_0000, 0xFF00, 1), region(0xE000_0000, 0x100, 6)];
        assert_eq!(direct_map_size(map), Ok(4 * GIB));
        assert_eq!(direct_map_size([]), Ok(4 * GIB));
    }

    #[test]
    fn direct_map_rounds_up_to_1_gib_above_4_gib() {
        // RAM up to 5.5 GiB.
        let map = [region(0x1_0000_0000, (GIB + GIB / 2) / 4096, 1)];
        assert_eq!(direct_map_size(map), Ok(6 * GIB));
        // Ends exactly on a boundary: no extra gigabyte.
        let map = [region(0x1_0000_0000, 2 * GIB / 4096, 2)];
        assert_eq!(direct_map_size(map), Ok(6 * GIB));
    }

    #[test]
    fn direct_map_ignores_kinds_that_are_not_memory_the_kernel_reads() {
        let high = 0x100_0000_0000; // 1 TiB
        for kind in [KIND_RESERVED, KIND_DEVICE, KIND_UNUSABLE, KIND_UNACCEPTED] {
            assert_eq!(direct_map_size([region(high, 0x1000, kind)]), Ok(4 * GIB), "kind {kind}");
        }
        for kind in [1, 2, 3, 4, 5, 6, KIND_PERSISTENT] {
            assert_eq!(
                direct_map_size([region(high, 0x1000, kind)]),
                Ok(high + GIB),
                "kind {kind}"
            );
        }
    }

    #[test]
    fn a_direct_map_above_64_tib_is_an_error() {
        let limit = 64u64 << 40;
        assert_eq!(direct_map_size([region(limit - GIB, GIB / 4096, 1)]), Ok(limit));
        assert_eq!(direct_map_size([region(limit, 1, 1)]), Err(DirectMapTooLarge));
        // An end address that overflows is too large, not a wrap to zero.
        assert_eq!(direct_map_size([region(u64::MAX & !0xFFF, 16, 1)]), Err(DirectMapTooLarge));
    }
}
