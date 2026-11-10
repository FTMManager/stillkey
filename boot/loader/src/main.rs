//! Stillkey UEFI loader (docs/02). Twelve steps: check the CPU, read and
//! validate the kernel, load it, build page tables and the handoff block,
//! leave boot services, and jump.

#![no_std]
#![no_main]

mod acpi;
mod cpu;
mod exit;
mod files;
mod gop;
mod jump;
mod load;
mod log;
mod mem;
mod paging;
mod panic;
mod qemu;

use core::convert::Infallible;
use core::slice;

use sha2::{Digest, Sha256};
use sk_elf::KERNEL_POLICY;
use stillkey_bootinfo::convert::{convert, direct_map_size, first_overlap, sort_by_base};
use stillkey_bootinfo::{
    self as bootinfo, BootInfo, KernelImage, MemoryRegion, ReservedRange, Stack, Table,
};
use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;

use crate::log::Hex;
use crate::mem::PAGE;

const PROFILE: &str = if cfg!(debug_assertions) { "dev" } else { "release" };

/// A refusal whose line is already on the serial port, made while boot
/// services are intact: the loader returns an error status to firmware.
pub struct Stop;

/// Prints the failure line and returns to firmware (docs/02 section 11).
macro_rules! stop {
    ($($arg:tt)*) => {{
        log!($($arg)*);
        return Err(Stop);
    }};
}

/// Prints the failure line and halts: for failures after `ExitBootServices`.
macro_rules! halt {
    ($($arg:tt)*) => {{
        log!($($arg)*);
        qemu::fail();
        cpu::halt()
    }};
}

#[entry]
fn main() -> Status {
    match run() {
        Ok(never) => match never {},
        Err(Stop) => {
            qemu::fail();
            Status::LOAD_ERROR
        }
    }
}

fn has_option(cmdline: &[u8], option: &[u8]) -> bool {
    cmdline.split(|&b| b == b' ').any(|word| word == option)
}

/// `ImageBase` and `ImageSize` of the loader itself.
fn own_image() -> Result<(u64, u64), Status> {
    let image = boot::open_protocol_exclusive::<LoadedImage>(boot::image_handle())
        .map_err(|e| e.status())?;
    let (base, size) = image.info();
    Ok((base as u64, size))
}

/// Steps 2-3 for the kernel: size by `GetInfo`, rule V1, then the bytes.
fn read_kernel(volume: &mut files::Volume) -> Result<&'static [u8], Stop> {
    let (mut file, size) = match volume.open_kernel() {
        Ok(opened) => opened,
        Err(status) => stop!("SK:LOADER:KERNEL_MISSING status={:#x}", status.0),
    };
    log!("SK:LOADER:KERNEL_FILE size={}", size);
    // V1, before anything is allocated for the file.
    if let Err(reason) = sk_elf::check_file_size(size, &KERNEL_POLICY) {
        stop!("SK:LOADER:KERNEL_REJECTED reason={}", reason.as_str());
    }
    let Some(phys) = mem::pages_for(size).and_then(mem::alloc_zeroed) else {
        stop!("SK:LOADER:NO_MEMORY what=file");
    };
    // SAFETY: `phys` is a fresh allocation of at least `size` bytes that is
    // never freed and that nothing else refers to; V1 bounds `size`.
    let bytes = unsafe { slice::from_raw_parts_mut(phys as *mut u8, size as usize) };
    if let Err(status) = files::read_exact(&mut file, bytes) {
        stop!("SK:LOADER:KERNEL_MISSING status={:#x}", status.0);
    }
    Ok(bytes)
}

fn run() -> Result<Infallible, Stop> {
    // ---- Step 1: serial, watchdog, CPU checks, own image ----
    log::init();
    let _ = boot::set_watchdog_timer(0, 0x1_0000, None);
    log!("SK:LOADER:START version={} profile={}", env!("CARGO_PKG_VERSION"), PROFILE);
    // Before anything is allocated. The command line is not read yet, so
    // these refusals never write the debug-exit port.
    if !cpu::has_nx() {
        stop!("SK:LOADER:CPU_UNSUPPORTED feature=nx");
    }
    if cpu::read_cr4() & cpu::CR4_LA57 != 0 {
        stop!("SK:LOADER:CPU_UNSUPPORTED feature=la57_active");
    }
    let (loader_image_base, loader_image_size) = match own_image() {
        Ok(image) => image,
        Err(status) => stop!("SK:LOADER:KERNEL_MISSING status={:#x}", status.0),
    };
    log!("SK:LOADER:IMAGE base={:#x} size={:#x}", loader_image_base, loader_image_size);

    // ---- Steps 2-3: the command line, then the kernel file ----
    let mut cmdline_buf = [0u8; bootinfo::MAX_CMDLINE as usize];
    let (cmdline, file) = {
        let mut volume = match files::Volume::open() {
            Ok(volume) => volume,
            Err(status) => stop!("SK:LOADER:KERNEL_MISSING status={:#x}", status.0),
        };
        let len = volume.read_cmdline(&mut cmdline_buf);
        let cmdline = &cmdline_buf[..len];
        log!("SK:LOADER:CMDLINE len={}", len);
        // Known before any later step can fail.
        if has_option(cmdline, b"qemu_exit=on") {
            qemu::enable();
        }
        if has_option(cmdline, b"test=serial") {
            let answered = log::recheck();
            log!("SK:LOADER:SERIAL recheck={}", if answered { "ok" } else { "lost" });
        }
        (cmdline, read_kernel(&mut volume)?)
        // The volume and every file handle are closed here.
    };

    // ---- Step 4: digest, then rules V2-V10 ----
    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(&Sha256::digest(file));
    log!("SK:LOADER:KERNEL_SHA256 {}", Hex(&sha256));
    let image = match sk_elf::validate(file, &KERNEL_POLICY) {
        Ok(image) => image,
        Err(reason) => stop!("SK:LOADER:KERNEL_REJECTED reason={}", reason.as_str()),
    };
    log!("SK:LOADER:KERNEL_VALID entry={:#x} segments={}", image.entry, image.segments().len());

    // ---- Step 5: the image and the boot stack ----
    let loaded = match load::load(file, &image) {
        Ok(loaded) => loaded,
        Err(load::LoadError::Image) => stop!("SK:LOADER:NO_MEMORY what=image"),
        Err(load::LoadError::Stack) => stop!("SK:LOADER:NO_MEMORY what=stack"),
    };
    log!("SK:LOADER:KERNEL_LOADED phys={:#x} pages={}", loaded.image_phys, loaded.image_pages);

    // ---- Step 6: graphics mode and ACPI pointer ----
    let framebuffer = gop::capture();
    match &framebuffer {
        Some(fb) => log!(
            "SK:LOADER:GOP mode={}x{} stride={} format={} fb={:#x} size={:#x}",
            fb.width,
            fb.height,
            fb.stride_pixels,
            if fb.format == bootinfo::FB_FORMAT_RGBX { "rgbx" } else { "bgrx" },
            fb.phys_base,
            fb.size
        ),
        None => log!("SK:LOADER:GOP none"),
    }
    let rsdp = acpi::rsdp();
    match rsdp {
        Some(address) => log!("SK:LOADER:ACPI rsdp={:#x}", address),
        None => log!("SK:LOADER:ACPI none"),
    }

    // ---- Step 7: preliminary map, direct-map size, page tables ----
    let preliminary = match mem::snapshot() {
        Ok(map) => map,
        Err(mem::MapError::NoMemory) => stop!("SK:LOADER:NO_MEMORY what=map"),
        Err(mem::MapError::Status(status)) => {
            stop!("SK:LOADER:EBS_FAILED reason=map status={:#x}", status.0)
        }
    };
    let desc_size = preliminary.desc_size;
    if desc_size == 0 {
        stop!("SK:LOADER:EBS_FAILED reason=convert");
    }
    let preliminary_count = preliminary.size / desc_size;
    let scratch_bytes = (preliminary_count * MemoryRegion::SIZE) as u64;
    let Some(scratch) = mem::pages_for(scratch_bytes.max(1)).and_then(mem::alloc_zeroed) else {
        stop!("SK:LOADER:NO_MEMORY what=map");
    };
    // SAFETY: `snapshot` returned a buffer holding `size` bytes of map.
    let raw = unsafe { slice::from_raw_parts(preliminary.buf, preliminary.size) };
    // SAFETY: a fresh, zeroed, page-aligned allocation with room for
    // `preliminary_count` regions; all-zero bytes are a valid `MemoryRegion`.
    let scratch =
        unsafe { slice::from_raw_parts_mut(scratch as *mut MemoryRegion, preliminary_count) };
    let Ok(count) = convert(raw, desc_size, scratch) else {
        stop!("SK:LOADER:EBS_FAILED reason=convert");
    };
    // The same function the kernel uses: the rule is part of the contract.
    let Ok(direct_map_bytes) = direct_map_size(scratch[..count].iter().copied()) else {
        stop!("SK:LOADER:NO_MEMORY what=tables");
    };

    let loader_base = loader_image_base & !(PAGE - 1);
    let loader_end = (loader_image_base + loader_image_size + (PAGE - 1)) & !(PAGE - 1);
    let Some(tables) = paging::build(&paging::Plan {
        direct_map_bytes,
        image: &image,
        image_phys: loaded.image_phys,
        stack_phys: loaded.stack_phys,
        framebuffer: framebuffer.map(|fb| (fb.phys_base, fb.size)),
        loader_base,
        loader_bytes: loader_end - loader_base,
    }) else {
        stop!("SK:LOADER:NO_MEMORY what=tables");
    };
    log!(
        "SK:LOADER:PAGING root={:#x} tables={} direct_map_gib={}",
        tables.root,
        tables.used,
        direct_map_bytes >> 30
    );

    // ---- Step 8: the handoff block, complete except for the memory map ----
    let mut reserved = [
        ReservedRange {
            base: loaded.image_phys,
            pages: loaded.image_pages as u64,
            purpose: bootinfo::PURPOSE_KERNEL_IMAGE,
            _pad: 0,
        },
        ReservedRange {
            base: loaded.stack_phys,
            pages: loaded.stack_pages as u64,
            purpose: bootinfo::PURPOSE_BOOT_STACK,
            _pad: 0,
        },
        ReservedRange {
            base: tables.pool,
            pages: tables.pool_pages as u64,
            purpose: bootinfo::PURPOSE_PAGE_TABLES,
            _pad: 0,
        },
        // The block itself; its address is filled in below.
        ReservedRange { base: 0, pages: 0, purpose: bootinfo::PURPOSE_BOOT_INFO, _pad: 0 },
        ReservedRange {
            base: loader_base,
            pages: (loader_end - loader_base) / PAGE,
            purpose: bootinfo::PURPOSE_LOADER_IMAGE,
            _pad: 0,
        },
    ];
    // The final map may hold more descriptors than the preliminary one.
    let region_cap = preliminary_count + 64;
    let Some(layout) = bootinfo::block_layout(cmdline.len(), reserved.len(), region_cap) else {
        stop!("SK:LOADER:NO_MEMORY what=bootinfo");
    };
    // Below 4 GiB: the part of the direct map that exists whatever the memory
    // map says, so the kernel can check the address it is handed before it
    // has read anything.
    let Some(block_phys) =
        mem::alloc_zeroed_below(layout.total_size / PAGE as usize, bootinfo::BLOCK_PHYS_LIMIT - 1)
    else {
        stop!("SK:LOADER:NO_MEMORY what=bootinfo");
    };
    reserved[3].base = block_phys;
    reserved[3].pages = (layout.total_size / PAGE as usize) as u64;
    reserved.sort_unstable_by_key(|range| range.base);

    // SAFETY: a fresh, zeroed allocation of `total_size` bytes that is never
    // freed and that nothing else refers to.
    let block = unsafe { slice::from_raw_parts_mut(block_phys as *mut u8, layout.total_size) };
    let (head, tail) = block.split_at_mut(layout.regions_off);
    head[layout.cmdline_off..][..cmdline.len()].copy_from_slice(cmdline);
    for (i, range) in reserved.iter().enumerate() {
        head[layout.reserved_off + i * ReservedRange::SIZE..][..ReservedRange::SIZE]
            .copy_from_slice(&range.encode());
    }
    assert!(tail.len() >= region_cap * MemoryRegion::SIZE && tail.as_ptr() as usize % 8 == 0);
    // SAFETY: `tail` is the rest of the block: zeroed, 8-byte aligned, with
    // room for `region_cap` regions, and not referred to again as bytes.
    // `MemoryRegion` is plain integers with no padding.
    let regions =
        unsafe { slice::from_raw_parts_mut(tail.as_mut_ptr().cast::<MemoryRegion>(), region_cap) };

    let mut flags = 0;
    if !cmdline.is_empty() {
        flags |= bootinfo::FLAG_CMDLINE_PRESENT;
    }
    if framebuffer.is_some() {
        flags |= bootinfo::FLAG_FRAMEBUFFER_PRESENT;
    }
    if rsdp.is_some() {
        flags |= bootinfo::FLAG_ACPI_RSDP_PRESENT;
    }
    let stack_top = paging::BOOT_STACK_VIRT + bootinfo::BOOT_STACK_SIZE;
    let mut header = BootInfo {
        magic: bootinfo::MAGIC,
        version_major: bootinfo::VERSION_MAJOR,
        version_minor: bootinfo::VERSION_MINOR,
        header_size: bootinfo::HEADER_SIZE as u32,
        total_size: layout.total_size as u64,
        flags,
        direct_map_virt_base: bootinfo::DIRECT_MAP_BASE,
        page_table_root: tables.root,
        memory_map: Table { phys: block_phys + layout.regions_off as u64, count: 0 },
        reserved: Table {
            phys: block_phys + layout.reserved_off as u64,
            count: reserved.len() as u64,
        },
        cmdline: Table { phys: block_phys + layout.cmdline_off as u64, count: cmdline.len() as u64 },
        kernel: KernelImage {
            phys_base: loaded.image_phys,
            virt_base: image.base,
            size: image.span,
            entry: image.entry,
            sha256,
        },
        boot_stack: Stack {
            virt_top: stack_top,
            phys_base: loaded.stack_phys,
            size: bootinfo::BOOT_STACK_SIZE,
        },
        framebuffer: framebuffer.unwrap_or_default(),
        acpi_rsdp: rsdp.unwrap_or(0),
        bootfs: Default::default(),
        exit_attempts: 0,
        uefi_descriptor_size: 0,
    };
    log!("SK:LOADER:BOOTINFO phys={:#x} size={}", block_phys, layout.total_size);

    // ---- Step 9: leave boot services ----
    // Every file and protocol is closed and every buffer is allocated,
    // except the raw-map buffer the exit sequence owns.
    let stale_key_test = has_option(cmdline, b"test=stalekey");
    let exited = exit::exit_boot_services(desc_size, region_cap, stale_key_test)?;

    // ---- Step 10: firmware is gone. No firmware call from here on. ----
    cpu::disable_interrupts();
    // SAFETY: the buffer the final `GetMemoryMap` filled with `size` bytes.
    let raw = unsafe { slice::from_raw_parts(exited.map, exited.size) };
    let Ok(count) = convert(raw, exited.desc_size, regions) else {
        halt!("SK:LOADER:EBS_FAILED reason=convert");
    };
    let regions = &mut regions[..count];
    sort_by_base(regions);
    if first_overlap(regions).is_some() {
        halt!("SK:LOADER:EBS_FAILED reason=overlap");
    }
    // The tables of step 7 must cover the final map.
    match direct_map_size(regions.iter().copied()) {
        Ok(needed) if needed <= direct_map_bytes => {}
        _ => halt!("SK:LOADER:EBS_FAILED reason=direct_map"),
    }
    header.memory_map.count = count as u64;
    header.uefi_descriptor_size = exited.desc_size as u32;
    header.flags |= bootinfo::FLAG_BOOT_SERVICES_EXITED;
    header.exit_attempts = exited.calls;
    head[..bootinfo::HEADER_SIZE].copy_from_slice(&header.encode());
    log!("SK:LOADER:EBS_OK attempts={} regions={}", exited.calls, count);

    // ---- Step 11: control bits, still on firmware's page tables ----
    cpu::enable_no_execute();
    cpu::enable_write_protect();
    if has_option(cmdline, b"test=pge") {
        // Test only: the VM's firmware leaves CR4.PGE clear, so set it to
        // exercise the path a real machine takes.
        cpu::set_global_pages_for_test();
    }
    // Cached global translations from firmware must not survive the CR3
    // switch. Only CR4.PGE changes.
    let (before, after) = cpu::disable_global_pages();
    log!(
        "SK:LOADER:CR4 pge_was={} before={:#x} after={:#x}",
        (before & cpu::CR4_PGE != 0) as u8,
        before,
        after
    );

    // ---- Step 12: the jump ----
    log!("SK:LOADER:JUMP entry={:#x}", image.entry);
    // SAFETY: boot services are exited and interrupts are off; the tables
    // identity-map this image, map the zeroed boot stack below `stack_top`
    // and map the entry point executable (rule V8); NXE is set and PGE clear.
    unsafe {
        jump::enter_kernel(
            tables.root,
            stack_top,
            image.entry,
            bootinfo::DIRECT_MAP_BASE + block_phys,
            layout.total_size as u64,
        )
    }
}
