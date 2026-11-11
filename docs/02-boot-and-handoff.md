# 02 · Loader, kernel image and the handoff

Status: specification for Milestone 1, extended in later milestones where noted. Firmware behaviour is cited to the UEFI Specification 2.11 (the current release) by section number, written `[UEFI §n]`; full links are in [09](09-references.md#uefi-and-acpi). Lines marked **Confirmed** come from a specification. Lines marked **Decision** are ours and can change. An **Assumption** is cited by its number (A-nn) from the list in [01 §2](01-overview-and-architecture.md#2-assumptions-to-verify), which names the check for each.

## 1. The path from power-on to the kernel

```
 OVMF firmware                Stillkey loader (BOOTX64.EFI)                     Stillkey kernel
 -------------                ------------------------------                     ---------------
 finds \EFI\BOOT\BOOTX64.EFI
 on the EFI System Partition
 loads it, calls its entry -> 1  start serial log, turn off the watchdog, check the CPU
                              2  open the volume it was loaded from
                              3  read CMDLINE.TXT, then KERNEL.ELF (later BOOTFS.IMG)
                              4  validate the ELF, log its SHA-256
                              5  allocate memory, copy segments, zero bss
                              6  read the graphics mode, find the ACPI table
                              7  read a preliminary memory map, build Stillkey page tables
                              8  fill BootInfo (everything but the memory map)
                              9  GetMemoryMap -> ExitBootServices (retry if stale)
 boot services are gone  <--  ---------------------------------------------------
                              10 interrupts off, convert the memory map
                              11 enable no-execute and write-protect,
                                 turn global pages off (clear CR4.PGE)
                              12 switch page tables and stack, jump  ----------> _start(block, mapped length)
                                                                                 start serial log
                                                                                 check the pointer and length,
                                                                                 then validate BootInfo
                                                                                 summarise memory
                                                                                 fill the screen, if there is one
                                                                                 drop the low mappings
                                                                                 halt (Milestone 1)
```

## 2. What firmware guarantees when it starts the loader

All **Confirmed**, `[UEFI §2.3.4]` unless noted.

| Fact | Consequence for us |
|---|---|
| The CPU is in 64-bit long mode, one processor active, paging on, and every range in the UEFI memory map is identity-mapped. Firmware may mark ranges read-only or non-executable. | The loader can use physical addresses directly. It must not edit firmware's page tables; it builds its own. |
| The specification does not say how many paging levels are active. Firmware may be running with 5-level paging (`CR4.LA57` = 1), and that bit cannot be changed while the CPU is in long mode (SDM, on enabling the paging modes). | The loader builds 4-level tables, so in step 1 it reads `CR4` and refuses to continue if `LA57` is set (section 9). |
| `CR0.EM` = 0 and `CR0.TS` = 0, and the floating-point and SSE control words are initialised: x87 and SSE instructions work. | The loader does not change these bits, so the kernel receives them as firmware left them (section 9). |
| Interrupts are **enabled** during boot services (firmware's timer). | The loader disables them itself after `ExitBootServices`. |
| At least 128 KiB of stack, 16-byte aligned. | Enough for the loader. The kernel gets its own stack. |
| Entry is `EFI_STATUS entry(EFI_HANDLE image, EFI_SYSTEM_TABLE *table)` with the Microsoft x64 convention: `RCX`, `RDX`, `R8`, `R9`, then stack; the caller reserves 32 bytes of shadow space (the size is from Microsoft's ABI documentation). | In Rust this is `extern "efiapi"`. Every firmware call uses it. |
| A 5-minute watchdog is armed before the boot option runs; it is disabled by `SetWatchdogTimer(0, …)` or by a successful `ExitBootServices`. `[UEFI §3.1.2, §7.5.1]` | The loader disables it first, so pausing in a debugger does not reset the VM. |
| Console output routing is platform policy; nothing guarantees it reaches a serial port, and the console pointers are null after exit. `[UEFI §12.1, §4.3]` | The loader drives the serial port itself for every line a test depends on. |
| The image is PE32+, subsystem 10 (EFI application), machine `0x8664`. `[UEFI §2.1.1]` | This is what the `x86_64-unknown-uefi` target produces. |

## 3. Files on the EFI System Partition

| Path | Purpose | Required |
|---|---|---|
| `\EFI\BOOT\BOOTX64.EFI` | The loader. This is the default path firmware tries on a disk with no boot entry `[UEFI §3.5.1.1]`. | Yes |
| `\EFI\STILLKEY\KERNEL.ELF` | The kernel | Yes |
| `\EFI\STILLKEY\CMDLINE.TXT` | Boot options, ASCII, at most 1024 bytes, for example `test=panic` | No |
| `\EFI\STILLKEY\BOOTFS.IMG` | Read-only filesystem image | From Milestone 2 |
| `\EFI\STILLKEY\MANIFEST.SIG` | Signed list of the SHA-256 digests of `KERNEL.ELF`, `BOOTFS.IMG` and, if present, `CMDLINE.TXT`. The loader itself is not listed: checking it is firmware's part ([08 §6](08-hardware-and-dual-boot.md#6-how-the-loader-verifies-the-kernel)) | From Milestone 5 |

The loader finds its own volume through the Loaded Image protocol's `DeviceHandle`, opens the Simple File System protocol on it, and opens files relative to the root `[UEFI §9.1, §13.4, §13.5]`. It reads `CMDLINE.TXT` first, so that `qemu_exit=on` is known before any later step can fail, then `KERNEL.ELF`. It reads each file's size with `GetInfo`, allocates, reads the whole file, and closes every handle before step 9. Of `CMDLINE.TXT` it reads at most the first 1024 bytes, and a `CMDLINE.TXT` that cannot be opened or read counts as absent. The kernel's size is checked against rule V1 of section 4 before anything is allocated for it.

**Decision:** fixed paths, no configuration file, no menu. The disk image uses a GPT with one FAT32 partition of type EFI System Partition, because the specification names FAT32 for system partitions `[UEFI §13.3]`.

**Decision:** the loader only reads. It opens files read-only and contains no call to file write, delete, flush or set-info, to Block I/O write, or to `SetVariable`. Before any hardware boot this is checked in two ways ([08 §1](08-hardware-and-dual-boot.md#1-safety-rules), rule 6): `cargo xtask test m5-readonly` scans the loader's sources and symbols, which together count as one check, and `cargo xtask test m5-vmdisk` boots disposable VM disks through a successful boot and through every loader failure path of section 11 and compares each disk image with its bytes before the run.

## 4. Validating the kernel ELF

The loader accepts a deliberately narrow subset of ELF64 and refuses everything else with a reason code. The validator is the `sk-elf` crate: small, `#![no_std]`, with host-side unit tests. It takes a policy `{window, max_file, max_mem}`, so that from Milestone 2 the kernel can apply the same rules to user programs.

| Policy | Used by | `window` | `max_file` and `max_mem` |
|---|---|---|---|
| Kernel | The loader, on `KERNEL.ELF` | From `0xFFFF_FFFF_8000_0000` up (the top 2 GiB of the address space) | 64 MiB each |
| User | The kernel, on user programs, from Milestone 2 | `0x40_0000` … `0x8000_0000` | `max_file` 16 MiB; `max_mem` the process's memory limit ([03 §5.4](03-kernel-and-interfaces.md#54-process-isolation)) |

| # | Check | Reject reason |
|---|---|---|
| V1 | File size is at least 64 bytes and at most `max_file`. The loader applies this to the size `GetInfo` reports, before it allocates or reads | `size` |
| V2 | `e_ident`: magic `7F 45 4C 46`, class 2 (64-bit), data 1 (little-endian), version 1 | `ident` |
| V3 | `e_type` = 2 (`ET_EXEC`), `e_machine` = 62 (x86-64), `e_version` = 1 | `type` |
| V4 | `e_ehsize` = 64, `e_phentsize` = 56, 1 ≤ `e_phnum` ≤ 16, and the program-header table lies inside the file | `phdr` |
| V5 | Every `PT_LOAD`: `p_filesz` ≤ `p_memsz`, `p_memsz` > 0, `p_offset + p_filesz` inside the file, `p_vaddr` page-aligned, and `p_offset ≡ p_vaddr (mod 4096)` | `segment` |
| V6 | Every `PT_LOAD` lies inside `window`. Segments are in ascending order and do not share a page: `round_up(prev.p_vaddr + prev.p_memsz, 4096) ≤ next.p_vaddr` | `range` |
| V7 | `p_flags` of every `PT_LOAD` is exactly 4 (read), 5 (read + execute) or 6 (read + write) | `wx` |
| V8 | `e_entry` lies in `[p_vaddr, p_vaddr + p_filesz)` of a segment with flags 5 | `entry` |
| V9 | No `PT_INTERP`, `PT_DYNAMIC` or `PT_TLS` header is present. Any other header that is not `PT_LOAD` is ignored | `dynamic` |
| V10 | `round_up(end of the last segment, 4096) − lowest p_vaddr ≤ max_mem`. For the kernel this is the size of the single allocation in section 5 | `size` |

The rules are applied in order and the first one that fails gives the reason. Every sum and rounding in V4–V10 uses checked arithmetic; an overflow rejects with that rule's reason.

**Decision (D-B-1):** the kernel is a statically linked, fixed-address executable (`ET_EXEC`), not position-independent. The loader therefore applies no relocations. Kernel address randomisation is deferred. **Confirmed:** the Rust target `x86_64-unknown-none` produces a position-independent executable by default, so the kernel is built with `-C relocation-model=static` and a linker script; a kernel built without that flag is `ET_DYN` and fails V3, which is the intended safety net.

After V1 the loader reads the file, computes SHA-256 over all of it and logs the digest, then applies V2–V10. In Milestone 1 the digest is a measurement: the test compares it with the digest of the file the build produced, which proves the loader read the image intact. It is not yet an authenticity check; that arrives with the signed manifest in Milestone 5 ([08](08-hardware-and-dual-boot.md)).

## 5. Loading segments, mappings and the stack

All memory is obtained with `AllocatePages` as `EfiLoaderData`, the type the specification tells loaders to use `[UEFI §7.2.1]`. Every range the kernel must keep is also written into the `reserved` table of `BootInfo`, so the kernel does not depend on firmware-specific memory types.

| Step | Detail |
|---|---|
| Image | One contiguous allocation of `round_up(end of the last segment, 4096) − lowest p_vaddr` bytes, which V10 bounds at 64 MiB. Zero it, then copy `p_filesz` bytes of each segment to `phys_base + (p_vaddr − lowest_vaddr)`. The zeroing provides `.bss`. |
| Kernel mapping | 4 KiB pages, by segment flags. 4 (read-only data): present, no-execute. 5 (text): present, executable, not writable. 6 (data and bss): present, writable, no-execute. V7 admits no other flags. |
| Direct map | Physical 0 … D at `0xFFFF_8000_0000_0000`, with 2 MiB pages, writable, no-execute. D = max(4 GiB, the highest end address among regions of kinds 1–6 and 10 in section 7), rounded up to 1 GiB. Kinds 7, 8, 9 and 11 do not extend it. If D > 64 TiB, where the next window of the kernel's address layout begins ([03 §3](03-kernel-and-interfaces.md#3-virtual-address-layout)), the loader stops with `SK:LOADER:NO_MEMORY what=tables`. Everything below D is mapped, holes and device memory included. The kernel must not assume that device memory above D is mapped. |
| Boot stack | 64 KiB, mapped one page into the stack window (`0xFFFF_D000_0000_1000`), writable, no-execute, with the page below it left unmapped as a guard. |
| Boot info block | No mapping of its own: it is allocated below 4 GiB and the kernel reads it through the direct map (section 6). |
| Framebuffer | If present, `Framebuffer.size` bytes, rounded up to whole pages, are mapped at `0xFFFF_E000_0000_0000`, writable, no-execute. It is mapped explicitly because the specification does not promise that the framebuffer appears in the memory map `[UEFI §2.3.4, §12.9]`. |
| Transition mapping | The loader's own image (`ImageBase`, `ImageSize` from the Loaded Image protocol) is identity-mapped read-and-execute, so the instruction after the page-table switch is still mapped. The kernel removes it. |
| Page-table memory | Allocated in one pool before step 9, zeroed, recorded in `reserved`. |

**The direct-map size is part of the contract.** `BootInfo` carries the base of the direct map but not its size, so the rule in the table is normative for the kernel too. One function, `stillkey_bootinfo::direct_map_size(regions)`, implements it; loader and kernel both call it, and it is tested on the host. The loader calls it on the preliminary memory map it reads in step 7, converted into a scratch buffer with the same code as the final map, to size the page-table pool. It calls it again on the final regions in section 8. The second result is not expected to be larger, because between the two maps memory only changes type inside ranges that already exist; if it is larger the tables are too short and the loader stops with `SK:LOADER:EBS_FAILED reason=direct_map`. The size is logged as `direct_map_gib=<n>` ([07 §7](07-milestone-1-spec.md#7-boot-behaviour-and-expected-serial-output)). In the Milestone 1 VM the memory map tops out below 4 GiB, so it is 4 (assumption A-15).

From Milestone 2 the kernel's own tables direct-map only memory of kinds 1–4 and 6, and device memory is reached through the kernel's MMIO window ([03 §3](03-kernel-and-interfaces.md#3-virtual-address-layout)).

**Decision:** the loader builds complete 4-level tables and never touches firmware's. No virtual mapping is both writable and executable. Kernel text and read-only data remain writable through the direct-map alias (ring 0 only) in Milestones 1–4.

## 6. `BootInfo`, version 1

Defined in the `stillkey-bootinfo` crate (`boot/bootinfo`, Rust path `stillkey_bootinfo`), which both loader and kernel depend on. Rules that make it a stable ABI: `#[repr(C)]`; only fixed-width integers and arrays of them; no Rust references, `usize`, `bool` or enums in the layout; every address is **physical** unless the field name contains `virt` (or is `entry`, which is the kernel's virtual entry address); little-endian; compile-time assertions pin the sizes (`BootInfo` 256 bytes, `MemoryRegion` 32, `ReservedRange` 24).

```rust
pub const MAGIC: u64 = u64::from_le_bytes(*b"STILLKEY");
pub const VERSION_MAJOR: u16 = 1;   // kernel refuses a different major
pub const VERSION_MINOR: u16 = 0;   // fields are only ever appended within a major

#[repr(C)]
pub struct BootInfo {
    pub magic: u64,
    pub version_major: u16,
    pub version_minor: u16,
    pub header_size: u32,           // size of this struct as the loader was built
    pub total_size: u64,            // the whole boot info block (described below)
    pub flags: u64,                 // see below
    pub direct_map_virt_base: u64,  // virtual base of the direct map; its size is the rule in section 5
    pub page_table_root: u64,       // physical address of the top-level table
    pub memory_map: Table,          // [MemoryRegion], sorted by base (section 7)
    pub reserved: Table,            // [ReservedRange], sorted by base (section 7)
    pub cmdline: Table,             // bytes; count = length
    pub kernel: KernelImage,
    pub boot_stack: Stack,
    pub framebuffer: Framebuffer,
    pub acpi_rsdp: u64,             // 0 if not found
    pub bootfs: PhysRange,          // zero in Milestone 1
    pub exit_attempts: u32,         // number of ExitBootServices calls made
    pub uefi_descriptor_size: u32,
}

#[repr(C)] pub struct Table     { pub phys: u64, pub count: u64 }
#[repr(C)] pub struct PhysRange { pub base: u64, pub size: u64 }

#[repr(C)]
pub struct MemoryRegion {
    pub base: u64,        // physical, 4 KiB aligned
    pub pages: u64,       // 4 KiB pages
    pub kind: u32,        // Stillkey classification, table in section 7
    pub uefi_type: u32,   // original EFI_MEMORY_TYPE, kept for diagnosis
    pub uefi_attr: u64,   // original attribute bits
}

#[repr(C)]
pub struct ReservedRange { pub base: u64, pub pages: u64, pub purpose: u32, pub _pad: u32 }
// purpose: 1 kernel image, 2 boot stack, 3 page tables, 4 boot info block,
//          5 boot filesystem, 6 loader image

#[repr(C)]
pub struct KernelImage {
    pub phys_base: u64, pub virt_base: u64, pub size: u64, pub entry: u64,
    pub sha256: [u8; 32],
}

#[repr(C)] pub struct Stack { pub virt_top: u64, pub phys_base: u64, pub size: u64 }

#[repr(C)]
pub struct Framebuffer {
    pub phys_base: u64, pub size: u64, pub virt_base: u64,
    pub width: u32, pub height: u32,
    pub stride_pixels: u32,       // PixelsPerScanLine, which can exceed width
    pub format: u32,              // 0 none, 1 RGBX 8:8:8:8, 2 BGRX 8:8:8:8
}
```

The header has no implicit padding: 48 bytes of scalars up to `page_table_root` (`direct_map_virt_base` is at offset 32), three `Table`s of 16, `KernelImage` 64, `Stack` 24, `Framebuffer` 40, `acpi_rsdp` 8, `bootfs` 16 and two `u32`, which is 256 bytes.

Flags: bit 0 `BOOT_SERVICES_EXITED`, bit 1 `FRAMEBUFFER_PRESENT`, bit 2 `ACPI_RSDP_PRESENT`, bit 3 `BOOTFS_PRESENT`, bit 4 `CMDLINE_PRESENT`.

**The boot info block.** One allocation, made in step 8, holds the `BootInfo` header, the command-line bytes (padded to a multiple of 8), the `ReservedRange` table and, last, the `MemoryRegion` array. The array has a fixed capacity: the descriptor count of the preliminary map read in step 7, plus 64. It is filled after `ExitBootServices` (section 8) and never moves. `total_size` is the size of the whole block, including the array's unused capacity, and the block is one entry in `reserved` (purpose 4). One function, `stillkey_bootinfo::block_layout`, gives the offsets; the block is whole pages and at most 16 MiB.

**Where the block lies, and its mapped length (Decision).** The loader allocates the block wholly below 4 GiB of physical memory. That part of the direct map exists whatever the memory map says (section 5), so the kernel can tell that the address it is handed is mapped before it has read anything. The loader passes two values to the kernel: the block's address in the direct map and the number of bytes it allocated for the block, a multiple of 4096 (section 9). The second value is the *mapped length*. It does not come from the block, and it, not `total_size`, is the limit for every read. This is the one change to the handoff that the first boot checkpoint made: the `BootInfo` structure and its version are unchanged, and the entry point gained a second argument.

**Acceptance rule (Decision).** The kernel accepts a handoff only if every check below passes, in this order. The rule is the function `stillkey_bootinfo::validate` (`boot/bootinfo/src/validate.rs`). The crate forbids `unsafe`, and the function works on a byte slice of exactly the mapped length, so it cannot read outside the block even if it is wrong. It is tested on the host by `m1-unit`.

| Step | Check | Reject reason |
|---|---|---|
| Pointer and length, before any read | The address is page-aligned and lies in the direct map at a physical address of at least 4096; the length is a non-zero multiple of 4096, at most 16 MiB; the block ends at or below 4 GiB. Sums are checked for overflow | `block` |
| Header | The mapped length is at least 256 bytes | `block` |
| | `magic == MAGIC` | `magic` |
| | `version_major == 1` | `version` |
| | `header_size >= 256`, `header_size % 8 == 0`, `header_size <= total_size`, and `total_size` is at most the mapped length | `size` |
| | `BOOT_SERVICES_EXITED` is set in `flags` | `flags` |
| | `direct_map_virt_base` is the address the kernel was built for | `layout` |
| Memory map | The table descriptor passes the table rule below (entries of 32 bytes, 8-byte aligned), and `count` is at least 1. Then every region: `base` page-aligned, `pages` at least 1, an end address that does not overflow, `kind` 1–11, and the regions sorted by `base` without overlap | `memory_map` |
| Reserved ranges | The table rule (entries of 24 bytes, 8-byte aligned), at most 32 entries, no overlap with the memory-map table. Then every range: page-aligned, at least one page, an end that does not overflow, `purpose` 1–6, sorted without overlap, and covered without a gap by kind-2 regions. Exactly one range has purpose 4, and it is the block itself: its base is the block's physical address and its size is the mapped length | `reserved` |
| Command line | The table rule (bytes, no alignment), at most 1024 bytes, no overlap with the other two tables, and `CMDLINE_PRESENT` is set exactly when `count` is not zero | `cmdline` |
| Framebuffer | Only if `FRAMEBUFFER_PRESENT` is set; without the flag the record is ignored and the kernel has no framebuffer. `format` is 1 or 2; `width` and `height` are not zero; `stride_pixels` is at least `width`; `phys_base` is not zero and is page-aligned; `virt_base` is the framebuffer window of section 5; `size` is at most 256 MiB and at least `stride_pixels × 4 × height`, a product that must not overflow; `phys_base + size`, rounded up to a whole page, does not overflow; and the pages from `phys_base` to the end of `size`, rounded up to a whole page, overlap no reserved range | `framebuffer` |

**The table rule.** A `Table { phys, count }` is located before any byte of it is read. With checked arithmetic at each step: `bytes = count × entry size`; `phys + bytes` does not overflow; `offset = phys − block address` does not underflow; `offset >= header_size`; `offset + bytes <= total_size`; and `phys` has the required alignment. Only then is the table's slice taken from the block, with a bounds-checked operation. A header that looks right, or a `total_size` the block states about itself, is therefore never enough: `total_size` is itself held to the mapped length, and the slice operation is held to it again.

The kernel reads the 256 bytes of the version 1.0 header. A larger `header_size` or a higher `version_minor` means a newer loader, and the extra bytes are ignored; tables must lie after them. The kernel makes no reference into the block before the pointer-and-length check has passed, and no reference to a table before that table's row has passed. The kernel copies each table entry out by value; it never forms a reference to a `MemoryRegion` or `ReservedRange` inside the block.

The ACPI pointer and the kernel-image record are not checked by this rule. Milestone 1 only logs them; the code that first uses them adds their checks.

The framebuffer row without its last condition is one function, `Framebuffer::is_well_formed` (`boot/bootinfo/src/fb.rs`). The loader applies it to the mode firmware reports before it sets `FRAMEBUFFER_PRESENT`, so the two sides cannot disagree about what a usable mode is: a mode that fails it is handed over as no framebuffer (`SK:LOADER:GOP none`). The same file holds the byte order of the two formats and the row layout (`pixel`, `fill_rows`), as pure code that is tested on the host.

**Framebuffer rules (Confirmed, `[UEFI §12.9.2]`):** the two 32-bit formats are byte order red-green-blue-reserved and blue-green-red-reserved; row stride is `PixelsPerScanLine × 4`, never width × 4; a mode reported as "Blt only" has no linear framebuffer. **Decision:** Milestone 1 keeps whatever mode firmware selected, accepts only the two 32-bit formats and only a mode that passes the framebuffer row of the acceptance rule above without its last condition, and otherwise clears `FRAMEBUFFER_PRESENT` and continues with serial only. The last condition, no overlap with a reserved range, is not checked by the loader. A handoff that breaks it is refused by the kernel (`reason=framebuffer`): a framebuffer that lies inside the kernel image, the stack or the page tables means the handoff is wrong, not that there is no screen. All graphics protocol calls happen before step 9, since no protocol may be used afterwards `[UEFI §7.4.6]`.

**ACPI (Confirmed):** the loader looks in the configuration table for the ACPI 2.0 GUID and passes the pointer on; the ACPI specification requires it to be fetched before `ExitBootServices` (ACPI 6.6 §5.2.5.2). Milestone 1 only logs it.

## 7. Memory the kernel must reserve before starting its allocator

The loader converts each UEFI descriptor into a `MemoryRegion`, stepping through the firmware's array by the descriptor size it reported, not by the structure size (they differ: OVMF reports 48 bytes for a 40-byte structure) `[UEFI §7.2.3]`. The conversion is a pure function in `stillkey-bootinfo` (`boot/bootinfo/src/convert.rs`), tested on the host with a 48-byte stride.

**Ordering (Decision).** The specification gives no ordering for the descriptor array `[UEFI §7.2.3]`. After conversion the loader sorts the `MemoryRegion` array by `base`, in place and without allocating, and stops with `SK:LOADER:EBS_FAILED reason=overlap` if two regions overlap. `ReservedRange` entries are page-aligned, sorted by `base`, do not overlap, and each lies wholly inside kind-2 regions. The kernel may rely on all of this.

| `kind` | From UEFI types | Kernel may allocate from it? | Basis |
|---|---|---|---|
| 1 Usable | Conventional | **Yes**, minus the reserved ranges below | Confirmed `[UEFI §7.2, Table 7.6]` |
| 2 Loader | LoaderCode, LoaderData | Only the parts not listed in `reserved`, and only after the kernel has dropped the low mappings | Confirmed that the type is reusable; our kernel image and tables live here |
| 3 Boot services | BootServicesCode, BootServicesData | **Not in Milestones 1–4** (Decision) | The specification says it is free after exit. We hold it back to stay conservative on real hardware. (Firmware's GDT and IDT are unusable from the loader's `CR3` switch in any case, because the loader's tables do not map them.) |
| 4 ACPI reclaimable | ACPIReclaimMemory | No, until the kernel has finished reading ACPI tables | Confirmed |
| 5 ACPI NVS | ACPIMemoryNVS | Never | Confirmed |
| 6 Firmware runtime | RuntimeServicesCode, RuntimeServicesData | Never | Confirmed |
| 7 Reserved | Reserved, PalCode, any unknown type, and Conventional memory with the attributes named below the table | Never | Confirmed for the listed types; Decision for the attribute case |
| 8 Device memory | MemoryMappedIO, MemoryMappedIOPortSpace | Never (not RAM) | Confirmed |
| 9 Unusable | UnusableMemory | Never | Confirmed |
| 10 Persistent | PersistentMemory | Never as ordinary RAM | Decision (the specification treats it as conventional memory that is also non-volatile) |
| 11 Unaccepted | UnacceptedMemoryType | Never until accepted (not implemented) | Confirmed |

A Conventional descriptor whose attributes include `EFI_MEMORY_SP` or `EFI_MEMORY_NV` is classified kind 7 (**Decision**). The specification describes specific-purpose memory as memory an operating system should avoid for core data, and `EFI_MEMORY_NV` marks persistent memory `[UEFI §7.2.3]`.

Ranges the kernel subtracts from kinds 1 and 2 before the allocator's first allocation:

| Range | Source | Hold until |
|---|---|---|
| Physical 0 to 1 MiB | Fixed policy (Decision) | Always |
| Kernel image | `reserved`, purpose 1 | Always |
| Boot stack | `reserved`, purpose 2 | The kernel is on its own stacks |
| Loader-built page tables | `reserved`, purpose 3 | The kernel has built replacements |
| Boot info block: header, command line, reserved table and the `MemoryRegion` array | `reserved`, purpose 4 | The kernel has copied what it needs |
| Boot filesystem image | `reserved`, purpose 5 | Always (it is the filesystem) |
| Loader image | `reserved`, purpose 6 | The low mappings are dropped |
| Framebuffer | `BootInfo.framebuffer` (usually not in the map at all) | Always |
| Anything not described by the map | – | Always (not RAM) `[UEFI §7.2.3]` |

In Milestone 1 `reserved` has five entries (purposes 1, 2, 3, 4 and 6). Loader allocations that are not in `reserved` need no protection: file buffers that have been consumed, and the buffer of raw UEFI descriptors used in section 8, which the kernel never reads. They are ordinary kind-2 memory.

The kernel logs the totals per kind and the final free page count. The Milestone 1 kernel does not start an allocator; it performs this calculation and prints it so the numbers can be checked. The calculation is `Accepted::usable_after_reserve` in `stillkey-bootinfo`, tested on the host: the pages of kinds 1 and 2, less the first 1 MiB, less every `reserved` entry, less the framebuffer's pages where they lie inside such a region. No page is subtracted twice.

## 8. The final `GetMemoryMap` / `ExitBootServices` sequence

**Confirmed, `[UEFI §7.4.6, §7.2.3]`:** `ExitBootServices(image, key)` succeeds only if `key` is the key of the current memory map. Otherwise it returns `EFI_INVALID_PARAMETER` and the loader must fetch the map again. Firmware may already have shut services down partially on the first call, so after a first call **only memory allocation services** (which include `GetMemoryMap`) may be used. `GetMemoryMap` takes the buffer size as input and overwrites it: with the size of the map it returned, or with the size needed if the buffer was too small. Allocating the map buffer can itself change the map, so the buffer needs slack. After success the loader owns all memory and no boot service or protocol may be called.

```
precondition: every file is read and closed, the graphics mode and ACPI pointer are captured,
              all memory except map_buf is allocated, and BootInfo is complete except for
              the fields set at the end of this listing
known:        desc_size         descriptor size, from the preliminary map of step 7
              region_cap        capacity of the MemoryRegion array in the BootInfo block
                                (descriptor count of the preliminary map + 64)
              direct_map_bytes  the direct-map size D that step 7 built tables for

needed  = size reported by GetMemoryMap(size = 0)            // EFI_BUFFER_TOO_SMALL
cap     = needed + 16 × desc_size                            // map buffer capacity in bytes, with slack
map_buf = AllocatePages(cap, rounded up to whole pages)      // EfiLoaderData, not in `reserved`
          if that fails: fail "SK:LOADER:NO_MEMORY what=map"
calls   = 0                                                  // ExitBootServices calls made so far
serial "SK:LOADER:EBS_BEGIN"

for pass in 1..=8:
    size   = cap                                             // before EVERY call: firmware overwrites it
    status = GetMemoryMap(&size, map_buf, &key, &desc_size, &desc_version)
    if status == EFI_BUFFER_TOO_SMALL:                       // size now holds the bytes needed
        cap     = size + 16 × desc_size
        map_buf = AllocatePages(cap, rounded up)             // map_buf only; the old buffer is left behind
                  if that fails: fail "SK:LOADER:NO_MEMORY what=map"
        continue
    if status != EFI_SUCCESS:
        fail "SK:LOADER:EBS_FAILED reason=map status=<efi status>"
    if size / desc_size > region_cap:
        fail "SK:LOADER:NO_MEMORY what=regions"
    if test=stalekey and calls == 0:
        AllocatePages(1 page)                                // test only: makes the key stale
    calls += 1
    status = ExitBootServices(image, key)                    // no firmware call since GetMemoryMap
    if status == EFI_SUCCESS: go to exited
    if status != EFI_INVALID_PARAMETER:
        fail "SK:LOADER:EBS_FAILED reason=exit status=<efi status>"
    serial "SK:LOADER:EBS_RETRY n=<calls>"                   // stale key: fetch the map again
fail "SK:LOADER:EBS_FAILED attempts=<calls>"                 // eight passes used up

exited:
disable interrupts
count = size / desc_size
convert the count descriptors in map_buf into the MemoryRegion array      // pure computation
if the map cannot be converted:                  fail "SK:LOADER:EBS_FAILED reason=convert"
sort the array by base, in place
if two regions overlap:                          fail "SK:LOADER:EBS_FAILED reason=overlap"
if direct_map_size(array) > direct_map_bytes:    fail "SK:LOADER:EBS_FAILED reason=direct_map"
BootInfo.memory_map = { array, count };  uefi_descriptor_size = desc_size
flags |= BOOT_SERVICES_EXITED;  exit_attempts = calls
serial "SK:LOADER:EBS_OK attempts=<calls> regions=<count>"

fail <line>:
    serial <line>;  write the debug-exit port if qemu_exit=on (section 11)
    if calls == 0: return an error status to firmware        // boot services are intact
    else:          halt for ever                             // rule 4
```

Rules that follow from the specification and that the code must make obvious:

1. Between a successful `GetMemoryMap` and the `ExitBootServices` call that uses its key there is no firmware call. The one exception is the test-only allocation, which exists to break this rule on purpose. Pure computation is allowed there, and so is the loader's own logging: its lines go straight to COM1 by port I/O, never through a firmware protocol, so they are legal at any point, including after a failed `ExitBootServices` call.
2. The loop is bounded: at most eight passes, each with at most one `ExitBootServices` call. A firmware that never accepts the key produces a clear serial message, not a hang with no output. A stale key on the first call is normal on firmware whose before-exit handlers change the map, so two attempts is not an error and nothing may depend on `attempts=1`.
3. After success nothing allocates. Every buffer was sized beforehand.
4. A failure before the first `ExitBootServices` call returns an error status to firmware. A failure after a first call cannot return to firmware safely; the loader halts.
5. `size` is set to the buffer capacity before every `GetMemoryMap` call. Without the reset, a retry would offer a buffer only as large as the previous map and fail as soon as the map had grown.
6. Only `map_buf` is ever reallocated. The `MemoryRegion` array is the tail of the BootInfo block (section 6) and never moves. `map_buf` holds only raw UEFI descriptors, is a separate `EfiLoaderData` allocation, and is deliberately not in `reserved`, because the kernel never reads it.

`exit_attempts`, and `attempts=` in the `EBS_OK` line, count `ExitBootServices` calls, not loop passes. Each failed call prints one `SK:LOADER:EBS_RETRY n=<k>` line, where `k` is the number of that call.

**Decision (D-B-2):** the loader implements this loop itself, calling `GetMemoryMap` and `ExitBootServices` through the raw boot-services table. The `uefi` crate (0.41) offers a helper, but its documentation states that it retries once and resets the machine if that fails, and it returns no attempt count. Our loop gives bounded retries, a diagnostic line on failure, and a test (`m1-stalekey`, [07 §9](07-milestone-1-spec.md#9-acceptance-tests)) that forces a stale key by allocating one page between the two calls, before the first `ExitBootServices` call. That test requires at least one `EBS_RETRY` line followed by the kernel's success marker (`HANDOFF_OK` at the first boot checkpoint, `M1_OK` for the full milestone). The expectation file of `m1-boot` does not pin the attempt count. The alternative is to call the helper and accept its behaviour.

## 9. The jump and the kernel's entry contract

**CPU checks (step 1).** The jump depends on two properties that firmware does not promise. The loader checks both in step 1, before any allocation:

| Check | If it fails |
|---|---|
| `CPUID.80000001H:EDX` bit 20 is set: the CPU supports no-execute | `SK:LOADER:CPU_UNSUPPORTED feature=nx` |
| `CR4` bit 12 (`LA57`) is clear: 4-level paging is active | `SK:LOADER:CPU_UNSUPPORTED feature=la57_active` |

Either failure returns an error status to firmware (section 11). With `LA57` set, the CPU would read the loader's 4-level root as a 5-level root at the first instruction after `mov cr3` and reset with no message. The VM's OVMF build is expected to run with four levels (in EDK II, 5-level paging is a build option that is off by default), so the QEMU command keeps `-cpu max`. Handing over under 5-level paging is not supported; section 12 records the option for Milestone 5.

**Control bits (step 11).** After `ExitBootServices`, with interrupts off and still on firmware's page tables, the loader sets `EFER.NXE` and then `CR0.WP` through ordinary one-line wrappers. `EFER.NXE` must be set before the switch: the loader's tables use the no-execute bit, which is a reserved bit, and therefore a fault, while `NXE` is clear.

**Global pages (step 11).** Then the loader reads `CR4` and, if bit 7 (`PGE`) is set, writes the same value back with only that bit cleared. Every other bit of `CR4` is left as firmware set it. The reason: a write to `CR3` does not discard cached translations that firmware's tables marked global, so with `PGE` set the kernel could run on a stale firmware translation for an address that the loader's tables map differently or not at all. Clearing `PGE` discards every cached translation, global ones included (SDM, on invalidating translations), and while it is clear no new global one can be cached. The loader logs `SK:LOADER:CR4 pge_was=<0|1> before=0x… after=0x…`. The step comes after `ExitBootServices` because firmware owns `CR4` until then, and before the `CR3` switch because that is the write the stale translations would survive. The VM's firmware hands over with `PGE` clear, so the boot option `test=pge` sets it first, in the same place, and `m1-pge` checks that only bit 7 then changes ([07 §9](07-milestone-1-spec.md#9-acceptance-tests)).

**The jump (step 12).** The last instructions in the loader are one inline-assembly block. Its operands are in fixed registers, so nothing in it reads memory. `RDI` already holds the virtual address of the handoff block in the direct map and `RSI` its mapped length:

```
mov cr3, rax         ; the root table; executing from the identity-mapped loader image
mov rsp, rcx         ; boot stack top − 8; a zero "return address" sits at [rsp]
xor ebp, ebp
jmp rdx              ; the kernel's entry point
```

State the kernel can rely on at `_start` (this table is the contract; anything not listed is undefined):

| Item | State |
|---|---|
| Signature | `extern "sysv64" fn _start(block: *const u8, block_len: u64) -> !`, System V AMD64 convention. `RDI` is the virtual address, in the direct map, of the handoff block, whose first 256 bytes are the `BootInfo` header. `RSI` is the mapped length: the number of bytes the loader allocated for the block, a multiple of 4096. The block lies below 4 GiB of physical memory. Both values are untrusted until the first row of the acceptance rule of section 6 has passed; the kernel makes one slice from them after that, and nothing before |
| Mode | 64-bit long mode, ring 0, boot processor only |
| Paging | On, 4-level, `CR4.LA57` = 0. `EFER.NXE` = 1, `CR0.WP` = 1. `CR4.PGE` = 0: global pages are off, and the kernel turns them on only after it runs on tables of its own. The other bits of `CR4` are as firmware left them |
| Mappings | The loader's tables: kernel image, direct map, boot stack, framebuffer, and the loader's identity mapping (section 5). No global-page bits are set, and no translation cached under firmware's tables is still held |
| Stack | `RSP + 8` is a multiple of 16, as at any System V function entry; `[RSP]` = 0; 64 KiB with a guard page below |
| Interrupts | Disabled. Direction flag clear |
| GDT, IDT | `GDTR` and `IDTR` still hold firmware's values, which are not mapped in the loader's tables. From the `CR3` switch until the kernel loads its own tables, any exception, NMI or segment-register load resets the machine; `CS` and `SS` work only through their cached descriptors. The kernel installs its own tables before it enables interrupts or loads a segment register |
| Vector and floating-point state | Register contents are undefined. The control bits are as firmware left them, which means x87 and SSE are enabled (`CR0.EM` = 0, `CR0.TS` = 0, `[UEFI §2.3.4]`). The kernel is built not to use these instructions and turns them off before the first entry to ring 3 in Milestone 2 ([03 §5.4](03-kernel-and-interfaces.md#54-process-isolation)) |
| Firmware | Boot services are gone. The kernel never calls UEFI runtime services (Milestones 1–5), and `SetVirtualAddressMap` is never called (it is optional, `[UEFI §8.4]`). The loader calls no runtime service in Milestones 1–4 and exactly one from Milestone 5, on its failure paths (section 11) |

The kernel's first actions, in order: initialise its serial driver; print `SK:KERNEL:START`; apply the acceptance rule of section 6 to the pointer, the length and then the block, and stop as section 11 describes if it fails; log; print `SK:KERNEL:CR4 pge=0` and treat a set `PGE` as a bug in the handoff (a panic); compute the memory summary; write the framebuffer once, which proves that mapping (from the full Milestone 1; the first boot checkpoint does not draw); clear the lower half of the top-level page table and reload `CR3` so that only higher-half mappings remain; then act on the command line. `CR4.PGE` is clear and the loader's tables set no global-page bits, so the `CR3` reload removes every stale translation.

## 10. Assembly and `unsafe` in the loader

| Where | What | Why |
|---|---|---|
| Serial output | `in`/`out` on ports `0x3F8`–`0x3FD` | No firmware involvement is allowed near the exit, and tests need deterministic output. A presence check with a timeout avoids hanging on machines with no serial port. The loader makes that check at `TPL_HIGH_LEVEL`, where no firmware event is dispatched `[UEFI §7.1]`: firmware's console driver polls the same port for input from a timer event while boot services are active (observed with OVMF), and a poll that falls inside the check takes the loopback byte, which makes a working port look absent and silences the loader. The kernel's check runs after `ExitBootServices` and needs no such care |
| Firmware calls | `extern "efiapi"` function pointers from the system table | Foreign ABI |
| Reading files into allocated pages; copying segments; zeroing | Raw pointer writes to memory just obtained from `AllocatePages` | Physical memory |
| Building page tables | Raw pointer writes into the page-table pool | Physical memory |
| CPU checks, interrupt flag, `EFER`, `CR0` and `CR4` updates | `cpuid`, a read of `CR4`, `cli`, `rdmsr`/`wrmsr`, `mov cr0`, `mov cr4` (bit 7 only), each a one-line wrapper | Privileged state |
| The jump | The four-instruction block in section 9 | Changes address space and stack; cannot be expressed as a Rust call |

Everything else in the loader is safe Rust. The logic that is easiest to get wrong lives in two library crates as pure `no_std` code over slices, unit-tested on the host by `m1-unit` ([07 §9](07-milestone-1-spec.md#9-acceptance-tests)): ELF validation in `sk-elf`, and in `stillkey-bootinfo` the memory-map conversion with its sort, its overlap check and `direct_map_size`. SHA-256 comes from the `sha2` crate. The loader crate itself is a binary only: it has no library target, is never built for the host, and is tested by booting it.

## 11. Failure behaviour

Every refusal prints one line to the serial port and then stops in a way a test can detect. Where the table says "return an error status", the status is `EFI_LOAD_ERROR`; nothing reads it, and the serial line is the diagnosis. Lines have the form `SK:<COMPONENT>:<EVENT> key=value ...`, in ASCII, starting at column 0. When the boot option `qemu_exit=on` is present the loader also writes the failure value to QEMU's debug-exit port before returning or halting ([07 §6](07-milestone-1-spec.md#6-vm-launch-command-and-exit-codes)). The two CPU checks, and a failure to open the loader's image record or its boot volume, come before the command line has been read, so they never write to that port. In Milestone 1 nothing is printed through the firmware console, because OVMF mirrors it to the same serial port (assumption A-15); on-screen failure text arrives in Milestone 5.

| Condition | Line | Then |
|---|---|---|
| The CPU has no no-execute support (step 1) | `SK:LOADER:CPU_UNSUPPORTED feature=nx` | Return an error status to firmware |
| Firmware is running with 5-level paging (step 1) | `SK:LOADER:CPU_UNSUPPORTED feature=la57_active` | Return an error status to firmware |
| The loader's own image record or its boot volume cannot be opened (steps 1 and 2, before the command line is read), or the kernel file is missing or unreadable | `SK:LOADER:KERNEL_MISSING status=<efi status>` | Return an error status to firmware |
| ELF validation fails | `SK:LOADER:KERNEL_REJECTED reason=<size\|ident\|type\|phdr\|segment\|range\|wx\|entry\|dynamic>` | Return an error status to firmware |
| An allocation fails; or the direct map would exceed 64 TiB (`tables`); or the final map has more descriptors than the region array holds (`regions`). `what` names the allocation: `file` a file buffer, `image` the kernel image, `stack` the boot stack, `tables` the page-table pool, `bootinfo` the boot info block, `map` a memory-map buffer in step 7 or in the exit loop | `SK:LOADER:NO_MEMORY what=<file\|image\|stack\|tables\|bootinfo\|map\|regions>` | Return an error status to firmware. `map` and `regions` can occur inside the exit loop: if `ExitBootServices` has already been called, halt instead |
| `GetMemoryMap` or `ExitBootServices` returns an unexpected status | `SK:LOADER:EBS_FAILED reason=<map\|exit> status=<efi status>` | Return an error status if `ExitBootServices` has not been called yet, otherwise halt |
| Exit loop exhausted after eight passes | `SK:LOADER:EBS_FAILED attempts=<n>` | Halt. If no `ExitBootServices` call was ever made (`attempts=0`), return an error status instead |
| The memory map firmware returned cannot be converted: a descriptor size below 40 or one that does not divide the map, a descriptor that is not page-aligned, or one whose end overflows | `SK:LOADER:EBS_FAILED reason=convert` | For the preliminary map of step 7, return an error status to firmware. For the final map, after exit, halt |
| After exit: two regions overlap, or the final map needs a larger direct map than was built | `SK:LOADER:EBS_FAILED reason=<overlap\|direct_map>` | Halt |
| A Rust panic in the loader, which is a bug and not a refusal | `SK:LOADER:PANIC at <file>:<line>: <message>` | Halt, after writing the failure value if `qemu_exit=on` has been read |
| Kernel rejects the handoff | `SK:KERNEL:BOOTINFO_REJECTED reason=<block\|magic\|version\|size\|flags\|layout\|memory_map\|reserved\|cmdline\|framebuffer>` | Halt for ever. The kernel does not write the debug-exit port, because the command line inside a rejected block is not trusted. A test sees the line and then the timeout (status 124) |

**From Milestone 5.** "Return an error status to firmware" applies to the loader that VM tests boot. The image for real hardware is built with `cargo xtask image --hardware`, which compiles the loader with the cargo feature `hardware`. That loader contains no write to the debug-exit port and never returns to firmware once it has started (decision D-H-4): where the table says return, it draws the failure line on the framebuffer, waits for a key press, then calls `ResetSystem(EfiResetShutdown)`. That is the only runtime service the loader ever calls. Returning would let firmware go on to the next boot entry, Windows Boot Manager, in the same power-on, with the Stillkey loader already measured into the TPM; BitLocker can then ask for its recovery key ([08](08-hardware-and-dual-boot.md)). The rows that halt are unchanged: a halted machine never reaches the next boot entry, and the line stays on screen until the machine is powered off. Stage 5-prep rehearses the shutdown path in the VM first: there a failure of the hardware image ends QEMU with status 0, not 35 ([06](06-milestones.md)).

## 12. What changes after Milestone 1

| Milestone | Addition |
|---|---|
| 2 | Loader reads `BOOTFS.IMG` and sets `bootfs`. Kernel installs its own GDT and IDT, replaces the loader's page tables, releases the boot stack. Its own direct map covers only memory of kinds 1–4 and 6 (section 5). The kernel starts using `sk-elf` with the user policy (section 4). |
| 4 | Loader may choose a graphics mode (prefer 1280×800, else the highest 32-bit mode) before exit. |
| 5 | Stage 5-prep, in the VM: `cargo xtask image --hardware` builds the loader with the cargo feature `hardware`. That loader contains no debug-exit write, draws its failure lines on the framebuffer, since real machines rarely have a serial port, and after a failure shuts the machine down instead of returning to firmware (section 11). To have somewhere to draw before anything can fail, it locates the Graphics Output Protocol and reads the current mode at the start of step 1, ahead of the CPU check; step 6 still prints the `GOP` line. Then: the loader verifies `MANIFEST.SIG` against a public key compiled into it, then the digest of each file the manifest lists, before step 5; a mismatch is refused with `SK:LOADER:VERIFY_FAILED file=<name>` ([08 §6](08-hardware-and-dual-boot.md#6-how-the-loader-verifies-the-kernel)). `BootInfo` minor version gains the signer's key identifier, appended, so `header_size` grows and the rule of section 6 still accepts it. If the target machine's firmware runs with `CR4.LA57` = 1, the option is one extra top-level table whose entries 0 and 511 lead to the identity-map tree and the higher-half tree; the kernel's own page-table code would then need the fifth level as well. The loader image meets the header rules that current firmware and signing expect from Milestone 1 onwards: the NX-compatible flag, section alignment of at least 4096 and no section both writable and executable. These are the linker's defaults for the target (assumption A-16) and `m1-unit` checks them. While firmware's page tables are active nothing executes from the stack or from allocated memory; the kernel image runs only on the loader's own tables. |
