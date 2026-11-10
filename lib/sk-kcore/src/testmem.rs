//! Plain memory for the host tests: page tables and data frames by
//! physical address.

use std::collections::HashMap;

use crate::paging::{ENTRIES, PAGE, PhysMem};

#[derive(Default)]
pub struct Memory {
    tables: HashMap<u64, Box<[u64; ENTRIES]>>,
    pub data: HashMap<u64, Box<[u8; PAGE as usize]>>,
}

impl PhysMem for Memory {
    fn table(&mut self, phys: u64) -> &mut [u64; ENTRIES] {
        assert_eq!(phys % PAGE, 0, "a table address is page-aligned");
        self.tables.entry(phys).or_insert_with(|| Box::new([0; ENTRIES]))
    }
}

impl Memory {
    /// The data frame at `phys`, filled with `fill` when first used.
    pub fn frame(&mut self, phys: u64, fill: u8) -> &mut [u8; PAGE as usize] {
        assert_eq!(phys % PAGE, 0);
        self.data.entry(phys).or_insert_with(|| Box::new([fill; PAGE as usize]))
    }
}

/// Hands out frames from 1 MiB upward and remembers which.
pub struct Frames {
    next: u64,
    end: u64,
    first: u64,
}

impl Frames {
    pub fn new(count: u64) -> Frames {
        let first = 0x10_0000;
        Frames { next: first, end: first + count * PAGE, first }
    }

    pub fn take(&mut self) -> Option<u64> {
        (self.next < self.end).then(|| {
            self.next += PAGE;
            self.next - PAGE
        })
    }

    /// How many frames were handed out.
    pub fn taken(&self) -> u64 {
        (self.next - self.first) / PAGE
    }

    /// The frames handed out after the first `count`.
    pub fn since(&self, count: u64) -> Vec<u64> {
        (count..self.taken()).map(|i| self.first + i * PAGE).collect()
    }
}
