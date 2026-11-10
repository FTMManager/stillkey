//! The kernel heap (docs/10 section 7, step 15): `sk_kcore::heap` over
//! pages mapped one after another from the start of the heap window.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;

use sk_kcore::HEAP_BASE;
use sk_kcore::heap::Heap;
use sk_kcore::paging::PAGE;

use crate::virt;

struct KernelHeap {
    heap: UnsafeCell<Heap>,
    /// Pages mapped so far.
    pages: UnsafeCell<u64>,
}

// SAFETY: one processor, and the kernel runs with interrupts off, so no two
// uses of the heap overlap.
unsafe impl Sync for KernelHeap {}

#[global_allocator]
static HEAP: KernelHeap = KernelHeap { heap: UnsafeCell::new(Heap::empty()), pages: UnsafeCell::new(0) };

/// The heap grows by at least this many pages at a time.
const GROW_PAGES: u64 = 4;

// SAFETY: `Heap` hands out blocks that do not overlap, of at least the size
// and alignment asked for, from pages this module mapped for it alone; a
// request it cannot serve returns null.
unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: see `Sync` above: this is the only access.
        let (heap, pages) = unsafe { (&mut *self.heap.get(), &mut *self.pages.get()) };
        let block = heap.alloc(layout);
        if !block.is_null() || layout.align() as u64 > PAGE {
            return block;
        }
        // Enough new pages to serve the request even if nothing was free.
        let wanted = ((layout.size() as u64).div_ceil(PAGE) + 1).max(GROW_PAGES);
        let start = HEAP_BASE + *pages * PAGE;
        let mut mapped = 0;
        while mapped < wanted && virt::map_heap_page(start + mapped * PAGE) {
            mapped += 1;
        }
        if mapped != 0 {
            *pages += mapped;
            // SAFETY: the pages were mapped just now, writable, for the heap
            // alone, and stay mapped; address and length are whole pages.
            unsafe { heap.add(start as *mut u8, (mapped * PAGE) as usize) };
        }
        heap.alloc(layout)
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        // SAFETY: the caller returns a block of this heap with its layout.
        unsafe { (*self.heap.get()).dealloc(block, layout) }
    }
}

/// Frames the heap holds.
pub fn pages() -> u64 {
    // SAFETY: see `Sync` above.
    unsafe { *HEAP.pages.get() }
}
