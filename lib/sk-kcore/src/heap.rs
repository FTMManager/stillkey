//! The kernel heap's bookkeeping (docs/10 section 8.5). The kernel gives it
//! whole pages; it hands out blocks of 16 to 2048 bytes from free lists, one
//! list per power of two, and whole pages for anything larger. This is the
//! one module of the crate with `unsafe`: a heap is pointers into memory
//! its caller vouches for.

use core::alloc::Layout;
use core::ptr;

const PAGE: usize = 4096;
/// The smallest block: it must hold the link of its free list.
const MIN_BLOCK: usize = 16;
const MAX_BLOCK: usize = 2048;
const CLASSES: usize = 8;

/// A free block of a size class.
struct Block {
    next: *mut Block,
}

/// A run of free pages. Runs are kept in address order and never touch:
/// neighbours are joined when a run is added.
struct Run {
    next: *mut Run,
    pages: usize,
}

pub struct Heap {
    classes: [*mut Block; CLASSES],
    runs: *mut Run,
    free: usize,
}

/// The size the heap really uses for a layout: a power of two up to 2048,
/// or whole pages. `None` for an alignment above a page.
fn block_size(layout: Layout) -> Option<usize> {
    if layout.align() > PAGE {
        return None;
    }
    let size = layout.size().max(layout.align()).max(MIN_BLOCK);
    if size <= MAX_BLOCK { Some(size.next_power_of_two()) } else { size.checked_next_multiple_of(PAGE) }
}

fn class_of(size: usize) -> usize {
    (size.trailing_zeros() - MIN_BLOCK.trailing_zeros()) as usize
}

impl Default for Heap {
    fn default() -> Self {
        Self::empty()
    }
}

impl Heap {
    pub const fn empty() -> Heap {
        Heap { classes: [ptr::null_mut(); CLASSES], runs: ptr::null_mut(), free: 0 }
    }

    /// Bytes that could still be handed out.
    pub fn free_bytes(&self) -> usize {
        self.free
    }

    /// Gives the heap `len` bytes at `start`.
    ///
    /// # Safety
    /// `start` and `len` are multiples of 4096, `len` is not zero, and the
    /// range is memory that may be read and written, that nothing else uses
    /// and that stays so for as long as the heap or a block from it is used.
    pub unsafe fn add(&mut self, start: *mut u8, len: usize) {
        debug_assert!(start as usize % PAGE == 0 && len % PAGE == 0 && len != 0);
        // SAFETY: the caller's promise is the one `insert_run` needs.
        unsafe { self.insert_run(start, len / PAGE) };
        self.free += len;
    }

    /// A block for `layout`, or null: no memory, or an alignment above 4096.
    pub fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let Some(size) = block_size(layout) else {
            return ptr::null_mut();
        };
        if size > MAX_BLOCK {
            let block = self.take_pages(size / PAGE);
            if !block.is_null() {
                self.free -= size;
            }
            return block;
        }
        let class = class_of(size);
        if self.classes[class].is_null() {
            let page = self.take_pages(1);
            if page.is_null() {
                return ptr::null_mut();
            }
            for offset in (0..PAGE).step_by(size) {
                // SAFETY: the page came from a run, so it is the heap's own
                // memory; every block lies inside it and is aligned to its
                // size, which is at least the alignment of a `Block`.
                unsafe { self.push_block(class, page.add(offset)) };
            }
        }
        let block = self.classes[class];
        // SAFETY: the list is not empty, and every block in it is free
        // memory of the heap that holds a valid link.
        self.classes[class] = unsafe { (*block).next };
        self.free -= size;
        block.cast()
    }

    /// Takes a block back.
    ///
    /// # Safety
    /// `block` came from `alloc` of this heap with the same `layout`, and is
    /// not used afterwards.
    pub unsafe fn dealloc(&mut self, block: *mut u8, layout: Layout) {
        let size = block_size(layout).expect("a layout this heap never served");
        if size > MAX_BLOCK {
            // SAFETY: the block is `size / PAGE` whole pages from `take_pages`.
            unsafe { self.insert_run(block, size / PAGE) };
        } else {
            // SAFETY: the block is `size` bytes of the heap, aligned to `size`.
            unsafe { self.push_block(class_of(size), block) };
        }
        self.free += size;
    }

    /// # Safety
    /// `block` is free memory of the heap, large and aligned enough for the class.
    unsafe fn push_block(&mut self, class: usize, block: *mut u8) {
        let block = block.cast::<Block>();
        // SAFETY: see above; nothing else refers to the block.
        unsafe { block.write(Block { next: self.classes[class] }) };
        self.classes[class] = block;
    }

    /// The first run with at least `pages` pages gives up its first
    /// `pages`. Null if there is none. Does not change the free count.
    fn take_pages(&mut self, pages: usize) -> *mut u8 {
        let mut link: *mut *mut Run = &raw mut self.runs;
        // SAFETY: `link` points at the heap's own list head or at the `next`
        // field of a run; every run is free memory of the heap with a valid
        // header, and the list ends in null.
        unsafe {
            while !(*link).is_null() {
                let run = *link;
                if (*run).pages >= pages {
                    if (*run).pages == pages {
                        *link = (*run).next;
                    } else {
                        let rest = run.cast::<u8>().add(pages * PAGE).cast::<Run>();
                        rest.write(Run { next: (*run).next, pages: (*run).pages - pages });
                        *link = rest;
                    }
                    return run.cast();
                }
                link = &raw mut (*run).next;
            }
        }
        ptr::null_mut()
    }

    /// Adds a run in address order and joins it with the runs it touches.
    ///
    /// # Safety
    /// `pages` whole pages at `start` are free memory of the heap and part
    /// of no run.
    unsafe fn insert_run(&mut self, start: *mut u8, pages: usize) {
        let new = start.cast::<Run>();
        let mut previous: *mut Run = ptr::null_mut();
        // SAFETY: as in `take_pages` for the list; `new` is the caller's
        // free memory, page-aligned, so a `Run` fits and is aligned.
        unsafe {
            let mut next = self.runs;
            while !next.is_null() && next < new {
                previous = next;
                next = (*next).next;
            }
            new.write(Run { next, pages });
            if !next.is_null() && start.add(pages * PAGE) == next.cast() {
                (*new).pages += (*next).pages;
                (*new).next = (*next).next;
            }
            if previous.is_null() {
                self.runs = new;
            } else if previous.cast::<u8>().add((*previous).pages * PAGE) == start {
                (*previous).pages += (*new).pages;
                (*previous).next = (*new).next;
            } else {
                (*previous).next = new;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Page-aligned memory for a heap, freed when dropped.
    struct Arena {
        start: *mut u8,
        layout: Layout,
    }

    impl Arena {
        fn new(pages: usize) -> Arena {
            let layout = Layout::from_size_align(pages * PAGE, PAGE).unwrap();
            // SAFETY: the layout has a size above zero.
            let start = unsafe { std::alloc::alloc(layout) };
            assert!(!start.is_null());
            Arena { start, layout }
        }

        fn heap(&self) -> Heap {
            let mut heap = Heap::empty();
            // SAFETY: the arena is unused memory of that size, and each test
            // drops its heap before its arena.
            unsafe { heap.add(self.start, self.layout.size()) };
            heap
        }

        fn holds(&self, block: *mut u8, size: usize) -> bool {
            let (start, at) = (self.start as usize, block as usize);
            at >= start && at + size <= start + self.layout.size()
        }
    }

    impl Drop for Arena {
        fn drop(&mut self) {
            // SAFETY: allocated in `new` with this layout.
            unsafe { std::alloc::dealloc(self.start, self.layout) };
        }
    }

    fn layout(size: usize, align: usize) -> Layout {
        Layout::from_size_align(size, align).unwrap()
    }

    #[test]
    fn every_alignment_up_to_a_page_is_honoured() {
        let arena = Arena::new(64);
        let mut heap = arena.heap();
        for shift in 0..=12 {
            let align = 1usize << shift;
            for size in [1, 7, 16, 100, 2048, 2049, 5000] {
                let block = heap.alloc(layout(size, align));
                assert!(!block.is_null(), "size {size} align {align}");
                assert_eq!(block as usize % align, 0, "size {size} align {align}");
                assert!(arena.holds(block, size));
                // The whole block is writable memory of its own.
                unsafe { ptr::write_bytes(block, 0xAB, size) };
            }
        }
        assert!(heap.alloc(layout(16, 8192)).is_null(), "an alignment above a page is refused");
    }

    #[test]
    fn blocks_do_not_overlap() {
        let arena = Arena::new(16);
        let mut heap = arena.heap();
        let mut blocks: Vec<(usize, usize)> = Vec::new();
        for (i, size) in [16, 24, 100, 2048, 3000, 16, 9000, 64, 64].into_iter().enumerate() {
            let block = heap.alloc(layout(size, 8));
            assert!(!block.is_null());
            unsafe { ptr::write_bytes(block, i as u8, size) };
            blocks.push((block as usize, size));
        }
        for (i, &(at, size)) in blocks.iter().enumerate() {
            let bytes = unsafe { core::slice::from_raw_parts(at as *const u8, size) };
            assert!(bytes.iter().all(|&b| b == i as u8), "block {i} was overwritten");
        }
    }

    #[test]
    fn a_freed_block_is_used_again() {
        let arena = Arena::new(4);
        let mut heap = arena.heap();
        let small = heap.alloc(layout(48, 8));
        unsafe { heap.dealloc(small, layout(48, 8)) };
        assert_eq!(heap.alloc(layout(60, 4)), small, "the same class of 64 bytes");

        let large = heap.alloc(layout(3 * PAGE, PAGE));
        assert!(!large.is_null());
        assert!(heap.alloc(layout(PAGE, PAGE)).is_null(), "three of the four pages and one split page are in use");
        unsafe { heap.dealloc(large, layout(3 * PAGE, PAGE)) };
        assert_eq!(heap.alloc(layout(3 * PAGE, PAGE)), large);
    }

    #[test]
    fn exhaustion_returns_null() {
        let arena = Arena::new(2);
        let mut heap = arena.heap();
        assert!(heap.alloc(layout(3 * PAGE, 8)).is_null());
        let mut got = 0;
        while !heap.alloc(layout(2048, 8)).is_null() {
            got += 1;
            assert!(got <= 4, "more blocks than two pages hold");
        }
        assert_eq!(got, 4);
        assert!(heap.alloc(layout(16, 8)).is_null());
        assert_eq!(heap.free_bytes(), 0);
        assert!(Heap::empty().alloc(layout(16, 8)).is_null());
    }

    #[test]
    fn freed_runs_join_their_neighbours() {
        let arena = Arena::new(8);
        let mut heap = arena.heap();
        let page = layout(PAGE, PAGE);
        let pages: Vec<*mut u8> = (0..8).map(|_| heap.alloc(page)).collect();
        assert!(pages.iter().all(|p| !p.is_null()));
        // Freed out of order: the eight pages must become one run again.
        for index in [3, 1, 6, 0, 2, 7, 5, 4] {
            unsafe { heap.dealloc(pages[index], page) };
        }
        let whole = heap.alloc(layout(8 * PAGE, PAGE));
        assert_eq!(whole, arena.start);
    }

    #[test]
    fn ten_thousand_mixed_calls_leave_the_free_count_where_it_started() {
        let arena = Arena::new(256);
        let mut heap = arena.heap();
        let start = heap.free_bytes();
        assert_eq!(start, 256 * PAGE);

        let mut live: Vec<(*mut u8, Layout)> = Vec::new();
        let mut x: u32 = 0x9E37_79B9;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as usize
        };
        for _ in 0..10_000 {
            if live.len() < 64 && next() % 3 != 0 {
                let size = match next() % 4 {
                    0 => 1 + next() % 64,
                    1 => 1 + next() % 2048,
                    2 => 2049 + next() % 6000,
                    _ => 16 << (next() % 8),
                };
                let wanted = layout(size, 1 << (next() % 7));
                let block = heap.alloc(wanted);
                if !block.is_null() {
                    unsafe { ptr::write_bytes(block, 0x5A, size) };
                    live.push((block, wanted));
                }
            } else if !live.is_empty() {
                let (block, used) = live.swap_remove(next() % live.len());
                unsafe { heap.dealloc(block, used) };
            }
        }
        for (block, used) in live {
            unsafe { heap.dealloc(block, used) };
        }
        assert_eq!(heap.free_bytes(), start);
    }

    #[test]
    fn the_largest_small_block_goes_back_to_its_class() {
        let arena = Arena::new(1);
        let mut heap = arena.heap();
        let half = layout(MAX_BLOCK, 8);
        let (a, b) = (heap.alloc(half), heap.alloc(half));
        assert!(!a.is_null() && !b.is_null() && a != b);
        // SAFETY: `a` came from this heap with this layout and is not used
        // before the heap hands it out again.
        unsafe { heap.dealloc(a, half) };
        assert_eq!(heap.alloc(half), a);
    }
}
