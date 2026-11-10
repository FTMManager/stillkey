//! A process's handle table (docs/10 section 8.5, D-K-18). A handle value
//! names a slot and the slot's generation, so a value that was closed stays
//! invalid when its slot is used again. Values below `FIRST_HANDLE` are
//! never valid.

use stillkey_abi::FIRST_HANDLE;

/// Bits of a value, above `FIRST_HANDLE`, that name the slot.
const SLOT_BITS: u32 = 12;
/// Generations wrap here, so that every value fits 32 bits.
const GENERATIONS: u32 = 1 << 19;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadHandle;

#[derive(Clone, Copy)]
struct Slot<T> {
    held: Option<(T, u32)>,
    generation: u32,
}

/// At most `cap` handles, in `N` slots.
pub struct Table<T: Copy, const N: usize> {
    slots: [Slot<T>; N],
    cap: usize,
    len: usize,
}

impl<T: Copy, const N: usize> Table<T, N> {
    pub const fn new(cap: usize) -> Self {
        assert!(N <= 1 << SLOT_BITS, "more slots than a handle value can name");
        Table { slots: [Slot { held: None, generation: 0 }; N], cap: if cap < N { cap } else { N }, len: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Stores an object with its rights and returns the handle value.
    pub fn insert(&mut self, object: T, rights: u32) -> Result<u32, Full> {
        if self.len >= self.cap {
            return Err(Full);
        }
        let index = self.slots.iter().position(|slot| slot.held.is_none()).ok_or(Full)?;
        let slot = &mut self.slots[index];
        slot.held = Some((object, rights));
        self.len += 1;
        Ok(FIRST_HANDLE as u32 + (slot.generation << SLOT_BITS | index as u32))
    }

    fn slot_of(&self, value: u32) -> Result<usize, BadHandle> {
        let rest = value.checked_sub(FIRST_HANDLE as u32).ok_or(BadHandle)?;
        let index = (rest & ((1 << SLOT_BITS) - 1)) as usize;
        let slot = self.slots.get(index).ok_or(BadHandle)?;
        if slot.held.is_some() && slot.generation == rest >> SLOT_BITS { Ok(index) } else { Err(BadHandle) }
    }

    /// The object and the rights behind a handle value.
    pub fn get(&self, value: u32) -> Result<(&T, u32), BadHandle> {
        let index = self.slot_of(value)?;
        self.slots[index].held.as_ref().map(|(object, rights)| (object, *rights)).ok_or(BadHandle)
    }

    /// Closes a handle. Its value is not valid again.
    pub fn remove(&mut self, value: u32) -> Result<T, BadHandle> {
        let index = self.slot_of(value)?;
        let slot = &mut self.slots[index];
        let (object, _) = slot.held.take().ok_or(BadHandle)?;
        slot.generation = (slot.generation + 1) % GENERATIONS;
        self.len -= 1;
        Ok(object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_value_is_0x1000_and_carries_its_rights() {
        let mut table = Table::<char, 8>::new(8);
        assert!(table.is_empty());
        let first = table.insert('a', 0x100).unwrap();
        let second = table.insert('b', 0x300).unwrap();
        assert_eq!((first, second), (0x1000, 0x1001));
        assert_eq!(table.get(first), Ok((&'a', 0x100)));
        assert_eq!(table.get(second), Ok((&'b', 0x300)));
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn nothing_is_valid_in_an_empty_table() {
        let mut table = Table::<char, 8>::new(8);
        for value in 0..=4095 {
            assert_eq!(table.get(value), Err(BadHandle));
            assert_eq!(table.remove(value), Err(BadHandle));
        }
        let mut x: u32 = 0x2545_F491;
        for _ in 0..4096 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            assert_eq!(table.get(x), Err(BadHandle), "value {x:#x}");
        }
    }

    #[test]
    fn values_below_the_first_are_never_valid() {
        let mut table = Table::<char, 8>::new(8);
        table.insert('a', 0).unwrap();
        for value in [0, 1, 0xFFF] {
            assert_eq!(table.get(value), Err(BadHandle));
        }
        assert!(table.get(0x1000).is_ok());
    }

    #[test]
    fn a_closed_value_stays_invalid_when_its_slot_is_used_again() {
        let mut table = Table::<char, 8>::new(8);
        let old = table.insert('a', 0).unwrap();
        assert_eq!(table.remove(old), Ok('a'));
        assert_eq!(table.get(old), Err(BadHandle));
        assert_eq!(table.remove(old), Err(BadHandle));
        let new = table.insert('b', 0).unwrap();
        assert_ne!(new, old);
        assert_eq!(table.get(old), Err(BadHandle));
        assert_eq!(table.get(new), Ok((&'b', 0)));
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn the_table_refuses_one_more_than_its_limit() {
        let mut table = Table::<u32, 8>::new(3);
        for i in 0..3 {
            table.insert(i, 0).unwrap();
        }
        assert_eq!(table.insert(3, 0), Err(Full));
        table.remove(0x1001).unwrap();
        assert!(table.insert(4, 0).is_ok());
        assert_eq!(table.insert(5, 0), Err(Full));

        // The limit cannot be larger than the number of slots.
        let mut small = Table::<u32, 2>::new(100);
        assert!(small.insert(0, 0).is_ok() && small.insert(1, 0).is_ok());
        assert_eq!(small.insert(2, 0), Err(Full));
    }

    #[test]
    fn a_slot_outside_the_table_is_invalid() {
        let mut table = Table::<char, 8>::new(8);
        table.insert('a', 0).unwrap();
        assert_eq!(table.get(0x1000 + 8), Err(BadHandle));
        assert_eq!(table.get(0x1000 + 0xFFF), Err(BadHandle));
        assert_eq!(table.get(u32::MAX), Err(BadHandle));
    }
}
