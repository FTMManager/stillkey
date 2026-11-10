//! Processes: their start, their system calls, their faults and their end
//! (docs/10 sections 8.1 to 8.4).

use alloc::vec::Vec;
use core::ptr;

use sk_bootfs::Entry;
use sk_kcore::account::{Account, Kind};
use sk_kcore::crash::{Record, Ring};
use sk_kcore::handles::Table;
use sk_kcore::logfmt::{AppText, Budget};
use sk_kcore::paging::{ADDRESS, Flags, PAGE, Space};
use sk_kcore::x86::{self, TrapFrame};
use sk_kcore::{MAX_PROCESSES, usercopy};
use stillkey_abi::diag::{self, CrashRecord, ProcessRecord, Totals};
use stillkey_abi::{
    ABI_VERSION, DEBUG_BUDGET_PER_SECOND, DEBUG_WRITE_MAX, DEFAULT_MEM_LIMIT, DIAG_READ_MAX, MEM_ALLOC_MAX,
    SYS_ABI_VERSION, SYS_CLOCK, SYS_DEBUG_WRITE, SYS_DIAG_READ, SYS_EXIT, SYS_HANDLE_CLOSE, SYS_MEM_ALLOC,
    SYS_MEM_FREE, SYS_SLEEP, SYS_YIELD, Status, TICK_NS, USER_HEAP_BASE, USER_IMAGE_FIRST, USER_STACK_SIZE,
    USER_STACK_TOP, rights,
};

use crate::arch::{self, Global};
use crate::virt::DirectMap;
use crate::{heap, log, phys, sched, timer, virt};

/// What a handle can name. One kind exists in this milestone.
#[derive(Clone, Copy)]
enum Object {
    /// The right to read the system's diagnostics.
    Diag,
}

/// Handle slots per process. The limits of docs/03 are far above what this
/// milestone hands out: one handle, to one program.
const HANDLE_SLOTS: usize = 16;

struct Process {
    pid: u32,
    name: [u8; 16],
    class: u32,
    space: Space,
    /// `diag::STATE_*`.
    state: u32,
    exit_code: u32,
    account: Account,
    limit_pages: u64,
    cpu_ticks: u64,
    start_ms: u64,
    handles: Table<Object, HANDLE_SLOTS>,
    budget: Budget,
    /// The first bytes of the SHA-256 of the program file.
    digest: [u8; 8],
    sleeping: bool,
}

impl Process {
    fn name(&self) -> &str {
        diag::name_of(&self.name)
    }

    fn alive(&self) -> bool {
        self.state == diag::STATE_ALIVE
    }
}

struct State {
    /// Every process since boot, ended ones included; the PID is the index
    /// plus one (D-K-17).
    processes: Vec<Process>,
    crashes: Ring,
    /// Rises whenever a process starts or ends or a crash is recorded.
    change: u64,
}

static STATE: Global<Option<State>> = Global::new(None);

/// One processor with interrupts off: no two uses overlap, and no caller
/// keeps a reference across a switch to another thread.
fn state() -> &'static mut State {
    // SAFETY: see above.
    unsafe { (*STATE.get()).as_mut() }.expect("no process table yet")
}

fn process(pid: u32) -> &'static mut Process {
    &mut state().processes[pid as usize - 1]
}

/// The table, with room for every process there can ever be, so that it
/// never moves.
pub fn init() {
    let table = State { processes: Vec::with_capacity(MAX_PROCESSES), crashes: Ring::new(), change: 0 };
    // SAFETY: one processor, interrupts off; set once.
    unsafe { *STATE.get() = Some(table) };
}

fn uptime_ms() -> u64 {
    timer::clock_ns() / 1_000_000
}

// ---- Start ----------------------------------------------------------------

fn flags_of(segment_flags: u32) -> Flags {
    Flags {
        writable: segment_flags & 2 != 0,
        user: true,
        no_execute: segment_flags & 1 == 0,
        cache_disable: false,
    }
}

const DATA: Flags = Flags { writable: true, user: true, no_execute: true, cache_disable: false };

/// Maps one fresh frame at `virt`, counted as `kind`; page tables that the
/// mapping needs are counted too. `Err` if the process is at its limit or
/// the system has no frame: then nothing was mapped.
fn map_page(space: &Space, account: &mut Account, kind: Kind, virt: u64, flags: Flags) -> Result<u64, ()> {
    account.charge(kind, 1).map_err(|_| ())?;
    let Some(frame) = phys::alloc() else {
        account.credit(kind, 1);
        return Err(());
    };
    let mut table = || {
        let frame = phys::alloc()?;
        if account.charge(Kind::Tables, 1).is_err() {
            phys::free(frame);
            return None;
        }
        Some(frame)
    };
    if space.map_4k(&mut DirectMap, &mut table, virt, frame, flags).is_err() {
        phys::free(frame);
        account.credit(kind, 1);
        return Err(());
    }
    Ok(frame)
}

/// Reads back what the tables really allow for one page and compares it
/// with what was asked: the effective permission, over all four levels
/// (docs/10 section 8.6).
fn check_page(space: &Space, virt: u64, frame: u64, want: Flags) {
    let seen = space.translate(&mut DirectMap, virt);
    let ok = seen.is_some_and(|m| {
        m.phys == frame
            && m.page_size == PAGE
            && m.user
            && m.writable == want.writable
            && m.no_execute == want.no_execute
            && !(m.writable && !m.no_execute)
    });
    assert!(ok, "page {virt:#x} of a program: wanted {want:?}, the tables give {seen:?}");
}

/// The image, each segment with exactly its own permission, and the stack.
fn load(space: &Space, account: &mut Account, segments: &[sk_elf::Segment], file: &[u8]) -> Result<(), ()> {
    for segment in segments {
        let flags = flags_of(segment.flags);
        for page in 0..segment.memsz.div_ceil(PAGE) {
            let from = page * PAGE;
            let frame = map_page(space, account, Kind::Image, segment.vaddr + from, flags)?;
            if from < segment.filesz {
                let take = (segment.filesz - from).min(PAGE) as usize;
                let source = &file[(segment.offset + from) as usize..][..take];
                // SAFETY: the frame is fresh and a whole page; `take` is at
                // most a page; the file is not in that frame.
                unsafe { ptr::copy_nonoverlapping(source.as_ptr(), phys::direct(frame), take) };
            }
            check_page(space, segment.vaddr + from, frame, flags);
        }
    }
    for page in 1..=USER_STACK_SIZE / PAGE {
        let virt = USER_STACK_TOP - page * PAGE;
        let frame = map_page(space, account, Kind::Stack, virt, DATA)?;
        check_page(space, virt, frame, DATA);
    }
    // Nothing below the stack, at its top, or at address 0.
    for hole in [USER_STACK_TOP - USER_STACK_SIZE - PAGE, USER_STACK_TOP, 0] {
        assert!(space.translate(&mut DirectMap, hole).is_none(), "{hole:#x} must not be mapped");
    }
    Ok(())
}

fn destroy(space: &Space) -> u64 {
    assert!(arch::read_cr3() & ADDRESS != space.root, "still on the tables that are about to be freed");
    let freed = space.destroy_user_half(&mut DirectMap, &mut |frame| phys::free(frame));
    phys::free(space.root);
    freed.pages + freed.tables + 1
}

/// Starts the program in `file`. With `diag` it gets the handle that reads
/// the system's diagnostics. `Err` is the reason for `SPAWN_REFUSED`.
pub fn spawn(entry: &Entry<'_>, file: &[u8], diag: bool) -> Result<u32, &'static str> {
    if state().processes.len() >= MAX_PROCESSES {
        return Err("table");
    }
    let limit = match entry.mem_limit_mib {
        0 => DEFAULT_MEM_LIMIT,
        mib => u64::from(mib) << 20,
    };
    let image = sk_elf::validate(file, &sk_elf::user_policy(limit)).map_err(|reason| reason.as_str())?;

    let limit_pages = limit / PAGE;
    let mut account = Account::new(limit_pages);
    // The top-level table counts as one of the process's tables.
    account.charge(Kind::Tables, 1).map_err(|_| "memory")?;
    let kernel_root = virt::kernel_space().root;
    let space =
        Space::with_kernel_half(&mut DirectMap, &mut || phys::alloc(), kernel_root).ok_or("memory")?;
    if load(&space, &mut account, image.segments(), file).is_err() {
        destroy(&space);
        return Err("memory");
    }

    let mut handles = Table::new(HANDLE_SLOTS);
    if diag {
        handles.insert(Object::Diag, rights::INSPECT).map_err(|_| "handles")?;
    }
    let mut name = [0u8; 16];
    let len = entry.name.len().min(name.len());
    name[..len].copy_from_slice(&entry.name.as_bytes()[..len]);
    let mut digest = [0u8; 8];
    digest.copy_from_slice(&entry.sha256[..8]);

    let state = state();
    let pid = state.processes.len() as u32 + 1;
    let process = Process {
        pid,
        name,
        class: entry.class,
        space,
        state: diag::STATE_ALIVE,
        exit_code: 0,
        account,
        limit_pages,
        cpu_ticks: 0,
        start_ms: uptime_ms(),
        handles,
        // One second of the timer per window, counted from the start.
        budget: Budget::new(DEBUG_BUDGET_PER_SECOND, timer::HZ, timer::ticks()),
        digest,
        sleeping: false,
    };
    log!(
        "SK:KERNEL:SPAWN pid={} name={} class={} handles={}",
        pid,
        process.name(),
        if process.class == sk_bootfs::CLASS_SYSTEM { "System" } else { "App" },
        process.handles.len()
    );
    state.processes.push(process);
    state.change += 1;
    sched::spawn_user(space.root, pid, &TrapFrame::user_entry(image.entry, USER_STACK_TOP));
    Ok(pid)
}

// ---- End ------------------------------------------------------------------

/// Ends the running process. The order matters (docs/10 section 8.3): the
/// processor leaves the process's tables before any of them is freed,
/// because a freed frame is overwritten at once.
fn end(pid: u32, crashed: bool) -> ! {
    let process = process(pid);
    process.state = if crashed { diag::STATE_CRASHED } else { diag::STATE_EXITED };
    process.sleeping = false;
    if process.budget.pending() != 0 {
        log!("SK:APP:DROPPED pid={} bytes={}", pid, process.budget.pending());
    }

    sched::leave_address_space();
    // SAFETY: the kernel's own address space; the kernel half is the same
    // in both, so the kernel keeps running.
    unsafe { arch::write_cr3(virt::kernel_space().root) };
    let frames = destroy(&process.space);
    assert!(
        frames == process.account.total(),
        "pid {pid} gave back {frames} frames but was charged {}",
        process.account.total()
    );
    process.account = Account::new(0);
    process.handles = Table::new(0);
    log!("SK:KERNEL:REAPED pid={} frames={}", pid, frames);
    state().change += 1;
    sched::exit_current()
}

/// A fault that is charged to the running process (docs/10 section 8.2):
/// a record, a line, and the process ends. The kernel carries on.
pub fn crash(frame: &TrapFrame) -> ! {
    let pid = sched::current_pid().expect("a fault from ring 3 without a process");
    let addr = if frame.vector == u64::from(x86::VECTOR_PAGE_FAULT) { arch::read_cr2() } else { 0 };
    let process = process(pid);
    // The registers can hold the program's data. They are kept apart from
    // the summary, and nothing in this milestone can read them.
    let registers = (process.class != sk_bootfs::CLASS_SYSTEM).then_some([
        frame.rax,
        frame.rbx,
        frame.rcx,
        frame.rdx,
        frame.rsi,
        frame.rdi,
        frame.rbp,
        frame.r8,
        frame.r9,
        frame.r10,
        frame.r11,
        frame.r12,
        frame.r13,
        frame.r14,
        frame.r15,
        frame.rip,
        frame.rsp,
        frame.rflags,
    ]);
    let record = Record {
        seq: 0,
        time_ms: uptime_ms(),
        pid: u64::from(pid),
        rip: frame.rip,
        addr,
        error: frame.error,
        vector: frame.vector as u32,
        digest: process.digest,
        name: process.name,
        registers,
    };
    log!(
        "SK:KERNEL:CRASH pid={} name={} vector={} addr={:#x} rip={:#x} error={:#x}",
        pid,
        process.name(),
        frame.vector,
        addr,
        frame.rip,
        frame.error
    );
    let state = state();
    state.crashes.push(record);
    state.change += 1;
    end(pid, true)
}

/// Once per timer tick: the running process is charged the tick, and the
/// output budgets move on.
pub fn on_tick(now: u64) {
    // SAFETY: one processor, interrupts off.
    let Some(state) = (unsafe { (*STATE.get()).as_mut() }) else {
        return;
    };
    if let Some(pid) = sched::current_pid() {
        state.processes[pid as usize - 1].cpu_ticks += 1;
    }
    for process in state.processes.iter_mut().filter(|p| p.alive()) {
        if let Some(bytes) = process.budget.roll(now) {
            log!("SK:APP:DROPPED pid={} bytes={}", process.pid, bytes);
        }
    }
}

// ---- System calls -----------------------------------------------------------

const OK: u64 = Status::Ok.as_return();
const INVALID_ARGS: u64 = Status::InvalidArgs.as_return();
const NO_MEMORY: u64 = Status::NoMemory.as_return();
const BAD_HANDLE: u64 = Status::BadHandle.as_return();
const ACCESS_DENIED: u64 = Status::AccessDenied.as_return();

/// Vector 0x80 from ring 3: number in `RAX`, result in `RAX`. A number the
/// kernel does not know is refused like a wrong argument (D-K-15).
pub fn syscall(frame: &mut TrapFrame) {
    let pid = sched::current_pid().expect("a system call without a process");
    frame.rax = match frame.rax {
        SYS_ABI_VERSION => ABI_VERSION,
        SYS_DEBUG_WRITE => debug_write(pid, frame.rdi, frame.rsi),
        SYS_EXIT => {
            let code = frame.rdi as u32;
            log!("SK:KERNEL:EXIT pid={} name={} code={}", pid, process(pid).name(), code as i32);
            process(pid).exit_code = code;
            end(pid, false)
        }
        SYS_YIELD => {
            sched::yield_now();
            OK
        }
        SYS_SLEEP => sleep(pid, frame.rdi),
        SYS_CLOCK => timer::clock_ns(),
        SYS_MEM_ALLOC => mem_alloc(pid, frame.rdi),
        SYS_MEM_FREE => mem_free(pid, frame.rdi, frame.rsi),
        SYS_HANDLE_CLOSE => handle_close(pid, frame.rdi),
        SYS_DIAG_READ => {
            let (bytes, cursor) = diag_read(pid, frame.rdi, frame.rsi, frame.rdx, frame.r10, frame.r8);
            frame.rdx = cursor;
            bytes
        }
        _ => INVALID_ARGS,
    };
}

/// Copies from a physical address inside one frame.
fn read_frame(phys: u64, out: &mut [u8]) {
    // SAFETY: `usercopy` passes an address inside one frame of allocator
    // memory that the program has mapped, with `out` no longer than the
    // rest of that frame.
    unsafe { ptr::copy_nonoverlapping(phys::direct(phys).cast_const(), out.as_mut_ptr(), out.len()) }
}

/// Copies to a physical address inside one frame.
fn write_frame(phys: u64, bytes: &[u8]) {
    // SAFETY: as `read_frame`; the program itself may write the page.
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), phys::direct(phys), bytes.len()) }
}

/// Call 1: one line of at most 256 bytes for the system log.
fn debug_write(pid: u32, addr: u64, len: u64) -> u64 {
    if len > DEBUG_WRITE_MAX {
        return INVALID_ARGS;
    }
    let process = process(pid);
    let mut buffer = [0u8; DEBUG_WRITE_MAX as usize];
    let text = &mut buffer[..len as usize];
    let copied =
        usercopy::copy_in(&process.space, &mut DirectMap, addr, text, &phys::is_memory, &mut read_frame);
    if copied.is_err() {
        return INVALID_ARGS;
    }
    if len == 0 {
        return OK;
    }
    if let Some(bytes) = process.budget.roll(timer::ticks()) {
        log!("SK:APP:DROPPED pid={} bytes={}", pid, bytes);
    }
    // A line over the budget is dropped whole and counted; the caller is
    // not told (docs/10 section 8.1).
    if process.budget.take(len) {
        log!("SK:APP:OUT pid={} name={} | {}", pid, process.name(), AppText(text));
    }
    OK
}

/// Call 4: whole ticks, rounded up; no time at all is a `yield`.
fn sleep(pid: u32, ns: u64) -> u64 {
    let ticks = ns.div_ceil(TICK_NS);
    if ticks == 0 {
        sched::yield_now();
        return OK;
    }
    process(pid).sleeping = true;
    sched::sleep_current(timer::ticks().saturating_add(ticks));
    process(pid).sleeping = false;
    OK
}

/// Pages of a program's heap lie in this many pages from `USER_HEAP_BASE`:
/// twice the limit, so that a search for a gap always ends.
fn heap_span(process: &Process) -> u64 {
    2 * process.limit_pages * PAGE
}

/// Call 6: fresh zeroed pages at the lowest address where they fit.
fn mem_alloc(pid: u32, len: u64) -> u64 {
    if len == 0 || len > MEM_ALLOC_MAX {
        return INVALID_ARGS;
    }
    let pages = len.div_ceil(PAGE);
    let process = process(pid);
    let space = process.space;
    let end = USER_HEAP_BASE + heap_span(process);
    let mut base = USER_HEAP_BASE;
    let mut clear = 0;
    while clear < pages {
        if base + pages * PAGE > end {
            return NO_MEMORY;
        }
        if space.translate(&mut DirectMap, base + clear * PAGE).is_some() {
            base += (clear + 1) * PAGE;
            clear = 0;
        } else {
            clear += 1;
        }
    }
    for page in 0..pages {
        if map_page(&space, &mut process.account, Kind::Heap, base + page * PAGE, DATA).is_err() {
            for done in 0..page {
                unmap_heap_page(&space, &mut process.account, base + done * PAGE);
            }
            return NO_MEMORY;
        }
    }
    base
}

fn unmap_heap_page(space: &Space, account: &mut Account, virt: u64) {
    let frame = space.unmap_4k(&mut DirectMap, virt).expect("a heap page that is not mapped");
    arch::invlpg(virt);
    phys::free(frame);
    account.credit(Kind::Heap, 1);
}

/// Call 7: every page of the range must have come from `mem_alloc`.
fn mem_free(pid: u32, addr: u64, len: u64) -> u64 {
    if addr % PAGE != 0 || len == 0 || len > MEM_ALLOC_MAX {
        return INVALID_ARGS;
    }
    let pages = len.div_ceil(PAGE);
    let process = process(pid);
    let space = process.space;
    let inside = addr >= USER_HEAP_BASE && addr + pages * PAGE <= USER_HEAP_BASE + heap_span(process);
    if !inside || (0..pages).any(|page| space.translate(&mut DirectMap, addr + page * PAGE).is_none()) {
        return INVALID_ARGS;
    }
    for page in 0..pages {
        unmap_heap_page(&space, &mut process.account, addr + page * PAGE);
    }
    OK
}

/// Call 8.
fn handle_close(pid: u32, handle: u64) -> u64 {
    let closed = u32::try_from(handle).ok().and_then(|value| process(pid).handles.remove(value).ok());
    if closed.is_some() { OK } else { BAD_HANDLE }
}

fn process_record(process: &Process) -> ProcessRecord {
    let counters = process.account.counters();
    ProcessRecord {
        pid: u64::from(process.pid),
        parent: 0,
        start_ms: process.start_ms,
        cpu_ticks: process.cpu_ticks,
        image_pages: counters.image as u32,
        heap_pages: counters.heap as u32,
        stack_pages: counters.stack as u32,
        table_pages: counters.tables as u32,
        shared_pages: 0,
        state: process.state,
        wait_reason: if process.sleeping { diag::WAIT_SLEEP } else { diag::WAIT_NONE },
        class: process.class,
        threads: u32::from(process.alive()),
        exit_code: process.exit_code,
        handles: process.handles.len() as u32,
        name: process.name,
    }
}

fn crash_record(record: &Record) -> CrashRecord {
    CrashRecord {
        seq: record.seq,
        time_ms: record.time_ms,
        pid: record.pid,
        rip: record.rip,
        addr: record.addr,
        image_offset: record.rip.checked_sub(USER_IMAGE_FIRST).unwrap_or(diag::NO_IMAGE_OFFSET),
        error: record.error,
        vector: record.vector,
        flags: if record.registers.is_some() { diag::CRASH_HAS_REGISTERS } else { 0 },
        digest: record.digest,
        name: record.name,
    }
}

fn totals(state: &State) -> Totals {
    Totals {
        change: state.change,
        uptime_ms: uptime_ms(),
        total_pages: phys::total(),
        free_pages: phys::free_count(),
        kernel_pages: heap::pages() + virt::table_pages(),
        processes: state.processes.iter().filter(|p| p.alive()).count() as u32,
        crashes: state.crashes.len() as u32,
    }
}

/// Call 9 (docs/10 section 8.4). Returns the bytes written, always whole
/// records, and the cursor for the next call: 0 when nothing is left.
fn diag_read(pid: u32, handle: u64, kind: u64, cursor: u64, buf: u64, len: u64) -> (u64, u64) {
    // The checks on a handle, in the order of docs/04 section 3: it exists,
    // it names the right kind of object, it carries the right.
    let process = process(pid);
    let held = u32::try_from(handle).ok().and_then(|value| process.handles.get(value).ok());
    let Some((object, held_rights)) = held else {
        return (BAD_HANDLE, 0);
    };
    match object {
        Object::Diag => {}
    }
    if held_rights & rights::INSPECT == 0 {
        return (ACCESS_DENIED, 0);
    }
    if kind == diag::KIND_CRASH_REGISTERS && held_rights & rights::INSPECT_REGISTERS == 0 {
        return (ACCESS_DENIED, 0);
    }
    let size = match kind {
        diag::KIND_PROCESSES => ProcessRecord::SIZE,
        diag::KIND_TOTALS => Totals::SIZE,
        diag::KIND_CRASHES => CrashRecord::SIZE,
        diag::KIND_CRASH_REGISTERS => diag::REGISTERS_SIZE,
        _ => return (INVALID_ARGS, 0),
    };
    if len > DIAG_READ_MAX || len < size as u64 {
        return (INVALID_ARGS, 0);
    }
    // The program itself must be able to write the whole buffer.
    let space = process.space;
    if usercopy::check_pages(&space, &mut DirectMap, buf, len, true, &phys::is_memory).is_err() {
        return (INVALID_ARGS, 0);
    }

    let room = len as usize / size;
    let mut out = [0u8; DIAG_READ_MAX as usize];
    let (mut used, mut next) = (0, 0);
    let state = state();
    match kind {
        diag::KIND_PROCESSES => {
            for other in state.processes.iter().filter(|p| u64::from(p.pid) >= cursor) {
                if used == room * size {
                    next = u64::from(other.pid);
                    break;
                }
                out[used..used + size].copy_from_slice(&process_record(other).encode());
                used += size;
            }
        }
        diag::KIND_TOTALS => {
            out[..size].copy_from_slice(&totals(state).encode());
            used = size;
        }
        diag::KIND_CRASHES => {
            for record in state.crashes.from(cursor) {
                if used == room * size {
                    next = record.seq;
                    break;
                }
                out[used..used + size].copy_from_slice(&crash_record(record).encode());
                used += size;
            }
        }
        _ => {
            let Some(registers) = state.crashes.get(cursor).and_then(|record| record.registers) else {
                return (INVALID_ARGS, 0);
            };
            for (i, value) in registers.iter().enumerate() {
                out[8 * i..8 * i + 8].copy_from_slice(&value.to_le_bytes());
            }
            used = size;
        }
    }
    let written =
        usercopy::copy_out(&space, &mut DirectMap, buf, &out[..used], &phys::is_memory, &mut write_frame);
    if written.is_err() {
        return (INVALID_ARGS, 0);
    }
    (used as u64, next)
}
