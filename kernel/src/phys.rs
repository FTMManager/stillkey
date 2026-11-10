//! The physical frame allocator (docs/10 section 7, step 11): the bitmap of
//! `sk_kcore::frames` over the memory the handoff calls usable.

use core::{ptr, slice};

use sk_kcore::frames::Bitmap;
use stillkey_bootinfo::{Accepted, DIRECT_MAP_BASE, KIND_LOADER, KIND_USABLE, PAGE_SIZE};

use crate::arch::Global;

/// The first megabyte is never handed out (docs/02 section 7).
const LOW_LIMIT: u64 = 0x10_0000;
const MAX_SPANS: usize = 128;

struct State {
    map: Bitmap<'static>,
    /// The memory the allocator was built from, as `start..end` addresses.
    spans: [(u64, u64); MAX_SPANS],
    span_count: usize,
}

static STATE: Global<Option<State>> = Global::new(None);

pub struct Summary {
    pub total: u64,
    pub free: u64,
    pub bitmap_pages: u64,
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    // SAFETY: one processor and interrupts off (see `Global`); `f` gets the
    // only access, and no caller keeps it.
    let state = unsafe { (*STATE.get()).as_mut() };
    f(state.expect("the frame allocator is not set up"))
}

/// The address at which the kernel reads and writes the frame at `phys`.
pub fn direct(phys: u64) -> *mut u8 {
    (DIRECT_MAP_BASE + phys) as *mut u8
}

/// Builds the allocator. Every figure is checked against the handoff's own
/// count, which `stillkey_bootinfo` computes in a different way.
pub fn init(info: &Accepted<'_>) -> Summary {
    let mut spans = [(0u64, 0u64); MAX_SPANS];
    let mut span_count = 0;
    let mut largest = (0u64, 0u64);
    for region in info.regions().filter(|r| r.kind == KIND_USABLE || r.kind == KIND_LOADER) {
        let (start, end) = (region.base.max(LOW_LIMIT), region.base + region.pages * PAGE_SIZE);
        if start >= end {
            continue;
        }
        if region.kind == KIND_USABLE && end - start > largest.1 - largest.0 {
            largest = (start, end);
        }
        // The regions are sorted; neighbours become one span.
        if span_count > 0 && spans[span_count - 1].1 == start {
            spans[span_count - 1].1 = end;
        } else {
            assert!(span_count < MAX_SPANS, "the memory map has too many separate pieces");
            spans[span_count] = (start, end);
            span_count += 1;
        }
    }
    assert!(span_count > 0, "the memory map has no usable memory");
    let frames = spans[span_count - 1].1 / PAGE_SIZE;
    let words = Bitmap::words_for(frames);
    let bitmap_pages = (words as u64 * 8).div_ceil(PAGE_SIZE);

    // The bitmap itself: the end of the largest usable region.
    assert!(largest.1 - largest.0 > bitmap_pages * PAGE_SIZE, "no room for the frame bitmap");
    let storage = largest.1 - bitmap_pages * PAGE_SIZE;
    let apart = |start: u64, end: u64| end <= storage || start >= largest.1;
    for range in info.reserved() {
        assert!(apart(range.base, range.base + range.pages * PAGE_SIZE), "the bitmap would lie on a reserved range");
    }
    let framebuffer = info.framebuffer().and_then(|fb| fb.phys_pages());
    if let Some((start, end)) = framebuffer {
        assert!(apart(start, end), "the bitmap would lie on the framebuffer");
    }
    // SAFETY: `storage..largest.1` is usable memory by the validated map,
    // lies on no reserved range and not on the framebuffer, and nothing has
    // been allocated yet, so nothing else refers to it. The loader's direct
    // map covers all memory, writable, and `storage` is page-aligned.
    let storage_words = unsafe { slice::from_raw_parts_mut(direct(storage).cast::<u64>(), words) };

    let mut map = Bitmap::new(storage_words, frames);
    for &(start, end) in &spans[..span_count] {
        map.release(start / PAGE_SIZE, (end - start) / PAGE_SIZE);
    }
    for range in info.reserved() {
        map.reserve(range.base / PAGE_SIZE, range.pages);
    }
    if let Some((start, end)) = framebuffer {
        map.reserve(start / PAGE_SIZE, (end - start) / PAGE_SIZE);
    }
    assert_eq!(map.free_count(), info.usable_after_reserve(), "the allocator and the handoff count differently");
    map.reserve(storage / PAGE_SIZE, bitmap_pages);

    let free = map.free_count();
    // SAFETY: one processor, interrupts off; set once, before any use.
    unsafe { *STATE.get() = Some(State { map, spans, span_count }) };
    Summary { total: free, free, bitmap_pages }
}

/// One frame, filled with zeros. Returns its physical address.
pub fn alloc() -> Option<u64> {
    let phys = with(|state| state.map.alloc())? * PAGE_SIZE;
    // SAFETY: the frame was free, so nothing refers to it, and the direct
    // map covers it.
    unsafe { ptr::write_bytes(direct(phys), 0, PAGE_SIZE as usize) };
    Some(phys)
}

/// Gives a frame back. Its contents are overwritten first, so that whatever
/// still refers to it fails loudly (D-K-19).
///
/// # Panics
/// If the frame is not one that is handed out.
pub fn free(phys: u64) {
    assert!(phys % PAGE_SIZE == 0, "free of an address that is not a frame: {phys:#x}");
    with(|state| {
        if let Err(why) = state.map.free(phys / PAGE_SIZE) {
            panic!("free of frame {phys:#x}: {why:?}");
        }
    });
    // SAFETY: the frame was handed out and its owner has given it up; the
    // direct map covers it.
    unsafe { ptr::write_bytes(direct(phys), 0xFA, PAGE_SIZE as usize) };
}

pub fn free_count() -> u64 {
    with(|state| state.map.free_count())
}

/// Whether `phys` lies in memory the allocator was built from.
pub fn is_memory(phys: u64) -> bool {
    with(|state| state.spans[..state.span_count].iter().any(|&(start, end)| (start..end).contains(&phys)))
}
