//! The `diag_read` records keep the sizes and offsets of docs/10 section 8.4.

use stillkey_abi::diag::*;

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn name(text: &str) -> [u8; 16] {
    let mut out = [0; 16];
    out[..text.len()].copy_from_slice(text.as_bytes());
    out
}

#[test]
fn sizes_and_kinds() {
    assert_eq!((ProcessRecord::SIZE, Totals::SIZE, CrashRecord::SIZE, REGISTERS_SIZE), (96, 48, 88, 144));
    assert_eq!((KIND_PROCESSES, KIND_TOTALS, KIND_CRASHES, KIND_CRASH_REGISTERS), (1, 2, 3, 4));
    assert_eq!((STATE_ALIVE, STATE_EXITED, STATE_CRASHED, STATE_ENDED), (1, 2, 3, 4));
    assert_eq!((WAIT_NONE, WAIT_SLEEP), (0, 1));
}

#[test]
fn a_process_record_has_its_fields_where_the_spec_puts_them() {
    let record = ProcessRecord {
        pid: 0x0101,
        parent: 0x0202,
        start_ms: 0x0303,
        cpu_ticks: 0x0404,
        image_pages: 5,
        heap_pages: 6,
        stack_pages: 7,
        table_pages: 8,
        shared_pages: 9,
        state: 10,
        wait_reason: 11,
        class: 12,
        threads: 13,
        exit_code: 14,
        handles: 15,
        name: name("ticker"),
    };
    let bytes = record.encode();
    assert_eq!([0, 8, 16, 24].map(|at| u64_at(&bytes, at)), [0x0101, 0x0202, 0x0303, 0x0404]);
    let words: Vec<u32> = (0..12).map(|i| u32_at(&bytes, 32 + 4 * i)).collect();
    assert_eq!(words, [5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0]);
    assert_eq!(&bytes[80..86], b"ticker");
    assert!(bytes[86..].iter().all(|&b| b == 0));
    assert_eq!(ProcessRecord::decode(&bytes), Some(record));
    assert_eq!(ProcessRecord::decode(&bytes[..95]), None);
}

#[test]
fn totals_have_their_fields_where_the_spec_puts_them() {
    let totals = Totals {
        change: 1,
        uptime_ms: 2,
        total_pages: 3,
        free_pages: 4,
        kernel_pages: 5,
        processes: 6,
        crashes: 7,
    };
    let bytes = totals.encode();
    assert_eq!([0, 8, 16, 24, 32].map(|at| u64_at(&bytes, at)), [1, 2, 3, 4, 5]);
    assert_eq!((u32_at(&bytes, 40), u32_at(&bytes, 44)), (6, 7));
    assert_eq!(Totals::decode(&bytes), Some(totals));
    assert_eq!(Totals::decode(&bytes[..47]), None);
}

#[test]
fn a_crash_record_has_its_fields_where_the_spec_puts_them() {
    let record = CrashRecord {
        seq: 1,
        time_ms: 2,
        pid: 3,
        rip: 0x40_1234,
        addr: 5,
        image_offset: 0x1234,
        error: 7,
        vector: 14,
        flags: CRASH_HAS_REGISTERS,
        digest: [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7],
        name: name("faulter"),
    };
    let bytes = record.encode();
    assert_eq!([0, 8, 16, 24, 32, 40, 48].map(|at| u64_at(&bytes, at)), [1, 2, 3, 0x40_1234, 5, 0x1234, 7]);
    assert_eq!((u32_at(&bytes, 56), u32_at(&bytes, 60)), (14, 1));
    assert_eq!(bytes[64..72], record.digest);
    assert_eq!(&bytes[72..79], b"faulter");
    assert_eq!(CrashRecord::decode(&bytes), Some(record));
    assert_eq!(CrashRecord::decode(&bytes[..87]), None);
}

#[test]
fn a_name_ends_at_its_first_zero() {
    assert_eq!(name_of(&name("inspector")), "inspector");
    assert_eq!(name_of(&[b'x'; 16]), "xxxxxxxxxxxxxxxx");
    assert_eq!(name_of(&[0; 16]), "");
}
