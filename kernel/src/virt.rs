//! The kernel's own address space (docs/10 section 7, step 12, and section
//! 8.6): its image by segment, the direct map of memory, the framebuffer
//! window, kernel stacks, the heap and a window for devices. Nothing in it
//! is open to ring 3, and nothing is both writable and executable.

use core::arch::asm;

use sk_kcore::paging::{ENTRIES, Flags, LARGE_PAGE, Mapping, PAGE, PhysMem, Space};
use sk_kcore::{KSTACK_BASE, KSTACK_SIZE, KSTACK_SLOT, MMIO_BASE};
use stillkey_bootinfo::{
    Accepted, DIRECT_MAP_BASE, KIND_ACPI_RECLAIM, KIND_BOOT_SERVICES, KIND_LOADER, KIND_RUNTIME, KIND_USABLE,
    PURPOSE_BOOT_STACK, PURPOSE_LOADER_IMAGE, PURPOSE_PAGE_TABLES,
};

use crate::arch::{self, Global};
use crate::phys;

/// Page tables, reached through the direct map.
pub struct DirectMap;

impl PhysMem for DirectMap {
    fn table(&mut self, phys: u64) -> &mut [u64; ENTRIES] {
        // SAFETY: the callers pass addresses of page-table frames: frames
        // this kernel allocated as tables. The direct map covers them, and
        // one processor with interrupts off means nothing else uses the
        // table while the reference lives.
        unsafe { &mut *phys::direct(phys).cast::<[u64; ENTRIES]>() }
    }
}

const fn kernel(writable: bool, no_execute: bool, cache_disable: bool) -> Flags {
    Flags { writable, user: false, no_execute, cache_disable }
}

const TEXT: Flags = kernel(false, false, false);
const READ_ONLY: Flags = kernel(false, true, false);
const DATA: Flags = kernel(true, true, false);
const DEVICE: Flags = kernel(true, true, true);

unsafe extern "C" {
    // Set by kernel/linker.ld. Only their addresses are used.
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __bss_end: u8;
}

struct State {
    root: u64,
    /// Frames that hold page tables of the kernel half.
    tables: u64,
    /// Kernel stacks mapped so far: slots 0 .. `stacks`.
    stacks: u64,
    /// Device pages mapped so far.
    devices: u64,
}

static STATE: Global<State> = Global::new(State { root: 0, tables: 0, stacks: 0, devices: 0 });

fn state() -> *mut State {
    STATE.get()
}

/// The first byte of the kernel's code. The self-test writes to it.
pub fn text_start() -> u64 {
    &raw const __text_start as u64
}

/// The three parts of the image, each with whole pages of its own.
fn sections() -> [(u64, u64, Flags); 3] {
    let up = |end: u64| end.next_multiple_of(PAGE);
    // The linker script defines the symbols; only their addresses are taken.
    [
        (&raw const __text_start as u64, up(&raw const __text_end as u64), TEXT),
        (&raw const __rodata_start as u64, up(&raw const __rodata_end as u64), READ_ONLY),
        (&raw const __data_start as u64, up(&raw const __bss_end as u64), DATA),
    ]
}

/// A frame for a page table of the kernel half.
fn table_frame() -> Option<u64> {
    let frame = phys::alloc()?;
    // SAFETY: one processor, interrupts off; no reference to the state is held.
    unsafe { (*state()).tables += 1 };
    Some(frame)
}

pub fn kernel_space() -> Space {
    // SAFETY: one processor, interrupts off.
    Space { root: unsafe { (*state()).root } }
}

/// Frames that hold page tables of the kernel half.
pub fn table_pages() -> u64 {
    // SAFETY: one processor, interrupts off.
    unsafe { (*state()).tables }
}

fn map(virt: u64, phys: u64, flags: Flags) {
    if let Err(why) = kernel_space().map_4k(&mut DirectMap, &mut table_frame, virt, phys, flags) {
        panic!("kernel mapping {virt:#x} -> {phys:#x}: {why:?}");
    }
}

pub struct Built {
    pub direct_map_regions: u64,
    /// The top of the main kernel stack.
    pub stack_top: u64,
}

/// Builds the tables. The processor still runs on the loader's.
pub fn build(info: &Accepted<'_>) -> Built {
    let root = table_frame().expect("no memory for the kernel's page tables");
    // SAFETY: one processor, interrupts off.
    unsafe { (*state()).root = root };

    // The image, by segment. The handoff validation showed that the record
    // is the one reserved range of the image.
    let image = info.header.kernel;
    let parts = sections();
    assert!(
        parts[0].0 == image.virt_base
            && parts[0].1 <= parts[1].0
            && parts[1].1 <= parts[2].0
            && parts[2].1 <= image.virt_base + image.size.next_multiple_of(PAGE),
        "the linker symbols do not describe the image the loader placed"
    );
    for (start, end, flags) in parts {
        assert!(start % PAGE == 0, "an image part does not start on a page");
        for virt in (start..end).step_by(PAGE as usize) {
            map(virt, image.phys_base + (virt - image.virt_base), flags);
        }
    }

    // Memory, and nothing else: no device and no hole is reachable through
    // the direct map.
    let mut direct_map_regions = 0;
    let memory =
        [KIND_USABLE, KIND_LOADER, KIND_BOOT_SERVICES, KIND_ACPI_RECLAIM, KIND_RUNTIME];
    for region in info.regions().filter(|r| memory.contains(&r.kind)) {
        direct_map_regions += 1;
        let (mut at, end) = (region.base, region.base + region.pages * PAGE);
        while at < end {
            if at % LARGE_PAGE == 0 && end - at >= LARGE_PAGE {
                let mapped =
                    kernel_space().map_2m(&mut DirectMap, &mut table_frame, DIRECT_MAP_BASE + at, at, DATA);
                if let Err(why) = mapped {
                    panic!("direct map of {at:#x}: {why:?}");
                }
                at += LARGE_PAGE;
            } else {
                map(DIRECT_MAP_BASE + at, at, DATA);
                at += PAGE;
            }
        }
    }

    // The framebuffer, where the loader had it.
    if let Some(fb) = info.framebuffer() {
        let (start, end) = fb.phys_pages().expect("an accepted framebuffer has pages");
        for phys in (start..end).step_by(PAGE as usize) {
            map(fb.virt_base + (phys - start), phys, DATA);
        }
    }

    Built { direct_map_regions, stack_top: map_kstack() }
}

/// Maps the next kernel stack and returns its top. A stack is the upper
/// half of its slot; the lower half stays unmapped, so running off the end
/// of a stack is a page fault, not a write into the stack below.
pub fn map_kstack() -> u64 {
    // SAFETY: one processor, interrupts off.
    let slot = unsafe {
        let state = state();
        (*state).stacks += 1;
        (*state).stacks - 1
    };
    let top = KSTACK_BASE + (slot + 1) * KSTACK_SLOT;
    for virt in (top - KSTACK_SIZE..top).step_by(PAGE as usize) {
        let frame = phys::alloc().expect("no memory for a kernel stack");
        map(virt, frame, DATA);
    }
    top
}

/// Maps one page of a device and returns the address of `phys` in it.
pub fn map_mmio(phys: u64) -> u64 {
    // SAFETY: one processor, interrupts off.
    let index = unsafe {
        let state = state();
        (*state).devices += 1;
        (*state).devices - 1
    };
    let virt = MMIO_BASE + index * PAGE;
    map(virt, phys & !(PAGE - 1), DEVICE);
    virt + phys % PAGE
}

/// Maps a fresh frame for the heap. False if there is no memory.
pub fn map_heap_page(virt: u64) -> bool {
    let Some(frame) = phys::alloc() else {
        return false;
    };
    match kernel_space().map_4k(&mut DirectMap, &mut table_frame, virt, frame, DATA) {
        Ok(()) => true,
        Err(why) => panic!("heap page {virt:#x}: {why:?}"),
    }
}

/// Gives every top-level entry of the kernel half a table, so that no later
/// kernel mapping changes the top level and every address space shares
/// them all. Returns how many entries are present: 256.
pub fn seal() -> u64 {
    let space = kernel_space();
    if let Err(why) = space.fill_kernel_half(&mut DirectMap, &mut table_frame) {
        panic!("sealing the kernel half: {why:?}");
    }
    space.kernel_top_entries(&mut DirectMap)
}

/// Loads the kernel's tables and continues in `next` on `stack_top`.
///
/// # Safety
/// `build` made the tables, and `stack_top` is a stack it mapped. Nothing
/// on the stack in use is needed again: it is not mapped afterwards.
pub unsafe fn enter(stack_top: u64, next: extern "C" fn() -> !) -> ! {
    let root = kernel_space().root;
    // SAFETY: the new tables map this code, the data and the new stack, so
    // execution continues; the caller gave up the old stack. The stack top
    // is a multiple of 16, as a call expects.
    unsafe {
        asm!(
            "mov cr3, {root}",
            "mov rsp, {top}",
            "xor ebp, ebp",
            "call {next}",
            "ud2",
            root = in(reg) root,
            top = in(reg) stack_top,
            next = in(reg) next,
            options(noreturn)
        );
    }
}

/// Frees what only the loader needed: the boot stack, the loader's page
/// tables and the loader's image. Returns the number of frames.
///
/// Call only on the kernel's own tables and stack.
pub fn release_loader_memory(info: &Accepted<'_>) -> u64 {
    assert!(arch::read_cr3() & sk_kcore::paging::ADDRESS == kernel_space().root, "still on the loader's tables");
    let mut released = 0;
    let purposes = [PURPOSE_BOOT_STACK, PURPOSE_PAGE_TABLES, PURPOSE_LOADER_IMAGE];
    for range in info.reserved().filter(|r| purposes.contains(&r.purpose)) {
        for page in 0..range.pages {
            phys::free(range.base + page * PAGE);
            released += 1;
        }
    }
    released
}

/// What a page of the kernel half must allow, by where it is.
fn expected(virt: u64) -> Flags {
    sections()
        .into_iter()
        .find(|&(start, end, _)| (start..end).contains(&virt))
        .map_or(DATA, |(_, _, flags)| flags)
}

/// Reads back every mapped page of the kernel half and compares what the
/// tables really allow, over all levels, with what the page is for
/// (docs/10 section 8.6). Returns the number of 4 KiB units checked.
///
/// # Panics
/// On any mismatch.
pub fn audit() -> u64 {
    let space = kernel_space();
    let mut pages = 0;
    space.walk_kernel_half(&mut DirectMap, &mut |virt, seen: Mapping| {
        let want = expected(virt);
        let ok = !seen.user
            && !(seen.writable && !seen.no_execute)
            && seen.writable == want.writable
            && seen.no_execute == want.no_execute;
        assert!(ok, "kernel page {virt:#x}: wanted {want:?}, the tables give {seen:?}");
        pages += seen.page_size / PAGE;
    });
    // SAFETY: one processor, interrupts off.
    let stacks = unsafe { (*state()).stacks };
    for slot in 0..stacks {
        let bottom = KSTACK_BASE + (slot + 1) * KSTACK_SLOT - KSTACK_SIZE;
        assert!(space.translate(&mut DirectMap, bottom - PAGE).is_none(), "no guard page below kernel stack {slot}");
        assert!(space.translate(&mut DirectMap, bottom).is_some(), "kernel stack {slot} is not mapped");
    }
    pages
}
