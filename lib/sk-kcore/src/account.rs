//! What a process holds, in pages, and its limit (docs/10 section 8.3).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Pages of the program file's segments.
    Image,
    /// Pages from `mem_alloc`.
    Heap,
    Stack,
    /// Page tables of the lower half, and the top-level table.
    Tables,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub image: u64,
    pub heap: u64,
    pub stack: u64,
    pub tables: u64,
}

impl Counters {
    pub const fn total(&self) -> u64 {
        self.image + self.heap + self.stack + self.tables
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverLimit;

#[derive(Clone, Copy, Debug)]
pub struct Account {
    limit: u64,
    counters: Counters,
}

impl Account {
    pub const fn new(limit_pages: u64) -> Account {
        Account { limit: limit_pages, counters: Counters { image: 0, heap: 0, stack: 0, tables: 0 } }
    }

    fn slot(&mut self, kind: Kind) -> &mut u64 {
        match kind {
            Kind::Image => &mut self.counters.image,
            Kind::Heap => &mut self.counters.heap,
            Kind::Stack => &mut self.counters.stack,
            Kind::Tables => &mut self.counters.tables,
        }
    }

    /// Counts `pages` more, unless the sum would pass the limit: then
    /// nothing changes.
    pub fn charge(&mut self, kind: Kind, pages: u64) -> Result<(), OverLimit> {
        let total = self.counters.total().checked_add(pages).ok_or(OverLimit)?;
        if total > self.limit {
            return Err(OverLimit);
        }
        *self.slot(kind) += pages;
        Ok(())
    }

    /// Counts `pages` fewer.
    ///
    /// # Panics
    /// If the process does not hold that many: the kernel freed a page twice.
    pub fn credit(&mut self, kind: Kind, pages: u64) {
        let slot = self.slot(kind);
        *slot = slot.checked_sub(pages).unwrap_or_else(|| panic!("credit of {pages} {kind:?} pages below zero"));
    }

    pub const fn counters(&self) -> Counters {
        self.counters
    }

    pub const fn total(&self) -> u64 {
        self.counters.total()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charges_add_up_by_kind() {
        let mut account = Account::new(100);
        account.charge(Kind::Image, 4).unwrap();
        account.charge(Kind::Stack, 16).unwrap();
        account.charge(Kind::Tables, 7).unwrap();
        account.charge(Kind::Heap, 3).unwrap();
        assert_eq!(account.counters(), Counters { image: 4, heap: 3, stack: 16, tables: 7 });
        assert_eq!(account.total(), 30);
    }

    #[test]
    fn a_charge_that_would_pass_the_limit_changes_nothing() {
        let mut account = Account::new(10);
        account.charge(Kind::Heap, 6).unwrap();
        let before = account.counters();
        assert_eq!(account.charge(Kind::Heap, 5), Err(OverLimit));
        assert_eq!(account.charge(Kind::Tables, u64::MAX), Err(OverLimit));
        assert_eq!(account.counters(), before);
        // Exactly the limit is allowed.
        assert_eq!(account.charge(Kind::Tables, 4), Ok(()));
        assert_eq!(account.charge(Kind::Image, 1), Err(OverLimit));
    }

    #[test]
    fn a_credit_makes_room_again() {
        let mut account = Account::new(10);
        account.charge(Kind::Heap, 10).unwrap();
        account.credit(Kind::Heap, 4);
        assert_eq!(account.total(), 6);
        assert_eq!(account.charge(Kind::Heap, 4), Ok(()));
    }

    #[test]
    #[should_panic(expected = "below zero")]
    fn a_credit_below_zero_is_a_kernel_bug() {
        let mut account = Account::new(10);
        account.charge(Kind::Heap, 2).unwrap();
        account.credit(Kind::Stack, 1);
    }
}
