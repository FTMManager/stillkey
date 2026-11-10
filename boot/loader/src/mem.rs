//! Page allocation and the raw memory-map call.

use core::ptr;

use uefi::Status;
use uefi::boot::{self, AllocateType, MemoryType};

pub const PAGE: u64 = 4096;

/// Whole pages needed for `bytes`.
pub fn pages_for(bytes: u64) -> Option<usize> {
    usize::try_from(bytes.checked_add(PAGE - 1)? / PAGE).ok()
}

fn allocate(how: AllocateType, pages: usize) -> Option<u64> {
    if pages == 0 {
        return None;
    }
    let base = boot::allocate_pages(how, MemoryType::LOADER_DATA, pages).ok()?;
    // SAFETY: firmware just gave us `pages` whole pages at `base`; its page
    // tables identity-map them writable, and nothing else refers to them.
    unsafe { ptr::write_bytes(base.as_ptr(), 0, pages * PAGE as usize) };
    Some(base.as_ptr() as u64)
}

/// Zeroed `EfiLoaderData` pages at any address. They are never freed.
pub fn alloc_zeroed(pages: usize) -> Option<u64> {
    allocate(AllocateType::AnyPages, pages)
}

/// Zeroed `EfiLoaderData` pages that end at or below `last`.
pub fn alloc_zeroed_below(pages: usize, last: u64) -> Option<u64> {
    allocate(AllocateType::MaxAddress(last), pages)
}

/// One `GetMemoryMap` call through the raw boot-services table. Firmware
/// overwrites `size` with the size of the map, or with the size it needs.
pub fn get_memory_map(
    buf: *mut u8,
    size: &mut usize,
    key: &mut usize,
    desc_size: &mut usize,
) -> Status {
    let mut version = 0u32;
    let table = uefi::table::system_table_raw().expect("the system table is set at entry");
    // SAFETY: called only while boot services are active, so the table and
    // its boot-services pointer are valid. `buf` is null with `*size == 0`,
    // or points to at least `*size` writable bytes.
    unsafe {
        let services = (*table.as_ptr()).boot_services;
        ((*services).get_memory_map)(size, buf.cast(), key, desc_size, &mut version)
    }
}

/// One `ExitBootServices` call through the raw boot-services table.
pub fn exit_boot_services(key: usize) -> Status {
    let image = boot::image_handle().as_ptr();
    let table = uefi::table::system_table_raw().expect("the system table is set at entry");
    // SAFETY: as above; the handle is the one firmware passed to the entry point.
    unsafe {
        let services = (*table.as_ptr()).boot_services;
        ((*services).exit_boot_services)(image, key)
    }
}

/// A memory map read into a buffer of its own.
pub struct Snapshot {
    pub buf: *mut u8,
    pub size: usize,
    pub desc_size: usize,
}

pub enum MapError {
    NoMemory,
    Status(Status),
}

/// Slack, in descriptors, for the map growing between sizing and reading.
pub const SLACK_DESCRIPTORS: usize = 16;

/// Reads the memory map once (step 7). Not the map `ExitBootServices` uses.
pub fn snapshot() -> Result<Snapshot, MapError> {
    let (mut size, mut key, mut desc_size) = (0, 0, 0);
    let mut status = get_memory_map(ptr::null_mut(), &mut size, &mut key, &mut desc_size);
    // Allocating the buffer can itself grow the map: a few tries.
    for _ in 0..4 {
        if status != Status::BUFFER_TOO_SMALL {
            return Err(MapError::Status(status));
        }
        let cap = desc_size
            .checked_mul(SLACK_DESCRIPTORS)
            .and_then(|slack| slack.checked_add(size))
            .ok_or(MapError::NoMemory)?;
        let pages = pages_for(cap as u64).ok_or(MapError::NoMemory)?;
        let buf = alloc_zeroed(pages).ok_or(MapError::NoMemory)? as *mut u8;
        size = pages * PAGE as usize;
        status = get_memory_map(buf, &mut size, &mut key, &mut desc_size);
        if status == Status::SUCCESS {
            return Ok(Snapshot { buf, size, desc_size });
        }
    }
    Err(MapError::Status(status))
}
