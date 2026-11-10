//! Crash records (docs/10 sections 8.2 and 8.4): what the kernel keeps
//! about a program that faulted, in a ring of the last [`CRASH_RING`].

use crate::CRASH_RING;

/// One crash. The registers are kept apart from the summary on purpose:
/// they can hold a program's data, and only a handle with the right to
/// inspect registers may read them (docs/04 section 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    /// Set by [`Ring::push`]: 1 for the first crash since boot.
    pub seq: u64,
    pub time_ms: u64,
    pub pid: u64,
    pub rip: u64,
    pub addr: u64,
    pub error: u64,
    pub vector: u32,
    pub digest: [u8; 8],
    pub name: [u8; 16],
    /// `rax`, `rbx`, `rcx`, `rdx`, `rsi`, `rdi`, `rbp`, `r8` to `r15`,
    /// `rip`, `rsp`, `rflags`; `None` when the register tier was not kept.
    pub registers: Option<[u64; 18]>,
}

pub struct Ring {
    records: [Option<Record>; CRASH_RING],
    /// Where the next record goes; the oldest kept one when the ring is full.
    next: usize,
    last_seq: u64,
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

impl Ring {
    pub const fn new() -> Ring {
        Ring { records: [None; CRASH_RING], next: 0, last_seq: 0 }
    }

    /// Keeps a record, dropping the oldest when the ring is full. Returns
    /// the sequence number it gave the record.
    pub fn push(&mut self, mut record: Record) -> u64 {
        self.last_seq += 1;
        record.seq = self.last_seq;
        self.records[self.next] = Some(record);
        self.next = (self.next + 1) % CRASH_RING;
        self.last_seq
    }

    /// The kept records whose sequence number is at least `seq`, oldest
    /// first. `from(0)` is every kept record.
    pub fn from(&self, seq: u64) -> impl Iterator<Item = &Record> {
        (0..CRASH_RING)
            .filter_map(move |i| self.records[(self.next + i) % CRASH_RING].as_ref())
            .filter(move |record| record.seq >= seq)
    }

    /// The record with exactly this sequence number, if it is still kept.
    pub fn get(&self, seq: u64) -> Option<&Record> {
        self.from(seq).next().filter(|record| record.seq == seq)
    }

    pub fn len(&self) -> usize {
        self.records.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crash(pid: u64) -> Record {
        Record {
            seq: 999,
            time_ms: pid * 10,
            pid,
            rip: 0x40_1000,
            addr: 0,
            error: 6,
            vector: 14,
            digest: [0; 8],
            name: [0; 16],
            registers: None,
        }
    }

    #[test]
    fn sequence_numbers_start_at_1_and_never_repeat() {
        let mut ring = Ring::new();
        assert!(ring.is_empty());
        let seqs: Vec<u64> = (0..100).map(|pid| ring.push(crash(pid))).collect();
        assert_eq!(seqs, (1..=100).collect::<Vec<u64>>());
    }

    #[test]
    fn the_record_carries_the_number_the_ring_gave_it() {
        let mut ring = Ring::new();
        ring.push(crash(7));
        let kept: Vec<&Record> = ring.from(0).collect();
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].seq, kept[0].pid), (1, 7));
    }

    #[test]
    fn the_33rd_crash_drops_the_first() {
        let mut ring = Ring::new();
        for pid in 1..=32 {
            ring.push(crash(pid));
        }
        assert_eq!(ring.len(), 32);
        assert_eq!(ring.from(0).next().unwrap().pid, 1);
        ring.push(crash(33));
        assert_eq!(ring.len(), 32);
        let seqs: Vec<u64> = ring.from(0).map(|r| r.seq).collect();
        assert_eq!(seqs, (2..=33).collect::<Vec<u64>>());
        assert_eq!(ring.get(1), None);
        assert_eq!(ring.get(2).unwrap().pid, 2);
        assert_eq!(ring.get(34), None);
    }

    #[test]
    fn a_reader_continues_from_its_cursor() {
        let mut ring = Ring::new();
        for pid in 1..=40 {
            ring.push(crash(pid));
        }
        let seqs: Vec<u64> = ring.from(38).map(|r| r.seq).collect();
        assert_eq!(seqs, [38, 39, 40]);
        // A cursor older than anything kept starts at the oldest kept.
        assert_eq!(ring.from(3).next().unwrap().seq, 9);
        assert_eq!(ring.from(41).count(), 0);
    }
}
