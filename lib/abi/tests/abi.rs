//! The numbers are an interface: a change here must be a decision, not a slip.

use stillkey_abi::*;

#[test]
fn call_numbers_are_0_to_9_in_the_documented_order() {
    let calls = [
        SYS_ABI_VERSION,
        SYS_DEBUG_WRITE,
        SYS_EXIT,
        SYS_YIELD,
        SYS_SLEEP,
        SYS_CLOCK,
        SYS_MEM_ALLOC,
        SYS_MEM_FREE,
        SYS_HANDLE_CLOSE,
        SYS_DIAG_READ,
    ];
    for (want, got) in calls.iter().enumerate() {
        assert_eq!(*got, want as u64);
    }
    assert_eq!(ABI_VERSION, 2);
}

#[test]
fn status_codes_and_names() {
    let want = [
        (0, "OK"),
        (1, "BAD_HANDLE"),
        (2, "WRONG_TYPE"),
        (3, "ACCESS_DENIED"),
        (4, "REVOKED"),
        (5, "NOT_FOUND"),
        (6, "INVALID_ARGS"),
        (7, "NO_MEMORY"),
        (8, "PEER_CLOSED"),
        (9, "TIMED_OUT"),
    ];
    assert_eq!(Status::ALL.len(), want.len());
    for (status, (code, name)) in Status::ALL.iter().zip(want) {
        assert_eq!(*status as u32, code);
        assert_eq!(status.name(), name);
    }
}

#[test]
fn a_failing_call_returns_the_negated_code() {
    assert_eq!(Status::InvalidArgs.as_return(), (-6i64) as u64);
    assert_eq!(Status::Ok.as_return(), 0);
    for status in Status::ALL {
        assert_eq!(Status::of_return(status.as_return()), Some(status));
    }
    // A positive result is a value, not a status.
    assert_eq!(Status::of_return(4096), Some(Status::Ok));
    assert_eq!(Status::of_return((-10i64) as u64), None);
}

#[test]
fn rights_are_single_bits_0_to_11() {
    use rights::*;
    let all = [
        READ, WRITE, MAP, LOOKUP, GRANT, EXECUTE, DUPLICATE, TRANSFER, INSPECT, INSPECT_REGISTERS,
        REVOKE, KILL,
    ];
    for (bit, right) in all.iter().enumerate() {
        assert_eq!(*right, 1 << bit);
    }
}

#[test]
fn limits_and_layout() {
    assert_eq!(FIRST_HANDLE, 0x1000);
    assert_eq!(DEBUG_WRITE_MAX, 256);
    assert_eq!(DEBUG_BUDGET_PER_SECOND, 4096);
    assert_eq!(MEM_ALLOC_MAX, 1 << 20);
    assert_eq!(USER_IMAGE_FIRST, 0x40_0000);
    assert_eq!(USER_IMAGE_END, 0x8000_0000);
    assert_eq!(USER_HEAP_BASE, 0x1_0000_0000);
    assert_eq!(USER_STACK_TOP, 0x7FFF_FFFF_F000);
    assert_eq!(USER_STACK_SIZE, 65536);
    assert_eq!(DEFAULT_MEM_LIMIT, 16 << 20);
    assert_eq!(TICK_NS, 10_000_000);
}
