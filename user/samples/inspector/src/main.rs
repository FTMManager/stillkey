//! The inspector (docs/10 section 9): reads the system's diagnostics through
//! the one handle it was started with, and prints them as tables.

#![no_std]
#![no_main]

use libstill::abi::diag::{self, CrashRecord, ProcessRecord, Totals};
use libstill::abi::{FIRST_HANDLE, Status};
use libstill::{abi_version, diag_read, out, sleep};

libstill::entry!(main);

/// The handle the kernel gave this program at its start.
const DIAG: u32 = FIRST_HANDLE;
const POLL_NS: u64 = 100_000_000;
/// A table is printed at least this often.
const TABLE_EVERY_MS: u64 = 1000;
/// Records read with one call.
const BATCH: usize = 8;

fn result_name(ret: u64) -> &'static str {
    Status::of_return(ret).map_or("UNKNOWN", Status::name)
}

fn read_totals() -> Option<Totals> {
    let mut buf = [0u8; Totals::SIZE];
    let (ret, _) = diag_read(DIAG, diag::KIND_TOTALS, 0, buf.as_mut_ptr(), buf.len());
    if ret != Totals::SIZE as u64 {
        return None;
    }
    Totals::decode(&buf)
}

/// Reads every record of one kind, `BATCH` at a time, and hands each to
/// `each`. Returns false if a call failed.
fn read_all<const N: usize>(kind: u64, size: usize, each: &mut dyn FnMut(&[u8])) -> bool {
    let mut buf = [0u8; N];
    let mut cursor = 0;
    loop {
        let (ret, next) = diag_read(DIAG, kind, cursor, buf.as_mut_ptr(), buf.len());
        if ret > buf.len() as u64 {
            return false;
        }
        buf[..ret as usize].chunks_exact(size).for_each(&mut *each);
        if next == 0 {
            return true;
        }
        cursor = next;
    }
}

fn class_name(class: u32) -> &'static str {
    if class == diag::CLASS_SYSTEM { "System" } else { "App" }
}

fn state_name(state: u32) -> &'static str {
    match state {
        diag::STATE_ALIVE => "Running",
        diag::STATE_EXITED => "Exited",
        diag::STATE_CRASHED => "Crashed",
        _ => "Ended",
    }
}

fn print_table(totals: &Totals) {
    out!(
        "table change={} processes={} crashes={} free={} kheap={} total={} uptime_ms={}",
        totals.change,
        totals.processes,
        totals.crashes,
        totals.free_pages,
        totals.kernel_pages,
        totals.total_pages,
        totals.uptime_ms
    );
    let mut whole = read_all::<{ BATCH * ProcessRecord::SIZE }>(
        diag::KIND_PROCESSES,
        ProcessRecord::SIZE,
        &mut |raw| {
            if let Some(p) = ProcessRecord::decode(raw) {
                out!(
                    "ps pid={} name={} class={} state={} image={} heap={} stack={} tables={} cpu={}",
                    p.pid,
                    diag::name_of(&p.name),
                    class_name(p.class),
                    state_name(p.state),
                    p.image_pages,
                    p.heap_pages,
                    p.stack_pages,
                    p.table_pages,
                    p.cpu_ticks
                );
            }
        },
    );
    whole &= read_all::<{ BATCH * CrashRecord::SIZE }>(diag::KIND_CRASHES, CrashRecord::SIZE, &mut |raw| {
        if let Some(c) = CrashRecord::decode(raw) {
            out!(
                "crash seq={} pid={} name={} vector={} addr={:#x} rip={:#x} offset={:#x}",
                c.seq,
                c.pid,
                diag::name_of(&c.name),
                c.vector,
                c.addr,
                c.rip,
                c.image_offset
            );
        }
    });
    // The change counter once more: a reader takes the table as whole only
    // if it is the one the table began with.
    match read_totals() {
        Some(after) if whole => out!("end change={}", after.change),
        _ => out!("end failed"),
    }
}

fn main() -> i32 {
    // Two calls that must be refused: registers need a right this handle
    // does not carry, and the kernel must not write into read-only memory,
    // here this program's own code.
    let mut scratch = [0u8; diag::REGISTERS_SIZE];
    let (registers, _) = diag_read(DIAG, diag::KIND_CRASH_REGISTERS, 0, scratch.as_mut_ptr(), scratch.len());
    let code = main as *const () as *mut u8;
    let (rotext, _) = diag_read(DIAG, diag::KIND_TOTALS, 0, code, Totals::SIZE);
    out!("inspector abi={} registers={} rotext={}", abi_version(), result_name(registers), result_name(rotext));

    // The change counter and the time of the last table printed.
    let mut printed: Option<(u64, u64)> = None;
    loop {
        if let Some(totals) = read_totals() {
            let due = printed
                .is_none_or(|(change, at)| totals.change != change || totals.uptime_ms - at >= TABLE_EVERY_MS);
            if due {
                print_table(&totals);
                printed = Some((totals.change, totals.uptime_ms));
            }
        }
        sleep(POLL_NS);
    }
}
