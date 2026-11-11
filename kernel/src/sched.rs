//! Threads and the switch between them (docs/10 section 7, steps 17a and
//! 20; docs/03 section 5.3).
//!
//! Every thread has a kernel stack of its own. A thread leaves the
//! processor only inside [`context_switch`], called from kernel code that
//! runs on its stack: a system call, a fault, or the timer interrupt that
//! took the processor away from it. The idle thread is the boot thread; it
//! runs when nothing else can.

use core::arch::{asm, naked_asm};
use core::mem::size_of;
use core::ptr;

use sk_kcore::MAX_PROCESSES;
use sk_kcore::paging::ADDRESS;
use sk_kcore::sched::{Queue, Tid};
use sk_kcore::x86::TrapFrame;

use crate::arch::{self, Global};
use crate::{tables, virt};

/// The idle thread and the self-check thread besides one per process.
const MAX_THREADS: usize = MAX_PROCESSES + 2;
const IDLE: usize = 0;

#[derive(Clone, Copy)]
struct Thread {
    /// Where `context_switch` left the stack pointer.
    saved_rsp: u64,
    stack_top: u64,
    /// The address space the thread runs in.
    root: u64,
    /// The process the thread belongs to; 0 for a kernel thread.
    pid: u32,
    alive: bool,
}

struct State {
    threads: [Thread; MAX_THREADS],
    count: usize,
    current: usize,
    queue: Queue<MAX_THREADS>,
    switches: u64,
}

static STATE: Global<State> = Global::new(State {
    threads: [Thread { saved_rsp: 0, stack_top: 0, root: 0, pid: 0, alive: false }; MAX_THREADS],
    count: 0,
    current: IDLE,
    queue: Queue::new(),
    switches: 0,
});

/// The scheduler's state. One processor with interrupts off: no two uses
/// overlap, and no caller keeps the reference across a switch.
fn state() -> &'static mut State {
    // SAFETY: see above.
    unsafe { &mut *STATE.get() }
}

/// Saves the callee-saved registers and the stack pointer of the running
/// thread, and continues the thread whose stack pointer is `load`. Returns
/// when some later switch loads the pointer saved here.
#[unsafe(naked)]
unsafe extern "C" fn context_switch(save: *mut u64, load: u64) {
    naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}

/// Makes the boot thread the idle thread.
pub fn init(stack_top: u64) {
    let state = state();
    state.threads[IDLE] =
        Thread { saved_rsp: 0, stack_top, root: virt::kernel_space().root, pid: 0, alive: true };
    state.count = 1;
    state.current = IDLE;
}

/// Writes `words` just below `top` and returns the address of the first.
///
/// # Safety
/// The memory below `top` is a fresh kernel stack.
unsafe fn push_words(top: u64, words: &[u64]) -> u64 {
    let start = top - 8 * words.len() as u64;
    // SAFETY: the caller's promise; a stack is far larger than `words`.
    unsafe { ptr::copy_nonoverlapping(words.as_ptr(), start as *mut u64, words.len()) };
    start
}

fn new_thread(root: u64, pid: u32, prepare: impl FnOnce(u64) -> u64) -> usize {
    let top = virt::map_kstack();
    let saved_rsp = prepare(top);
    let state = state();
    assert!(state.count < MAX_THREADS, "more threads than the scheduler holds");
    let tid = state.count;
    state.count += 1;
    state.threads[tid] = Thread { saved_rsp, stack_top: top, root, pid, alive: true };
    state.queue.add(Tid(tid as u32));
    tid
}

/// A thread of the kernel itself, ready to run `entry`.
fn spawn_kernel(entry: extern "C" fn() -> !) -> usize {
    new_thread(virt::kernel_space().root, 0, |top| {
        // What `context_switch` pops: six registers, then the address it
        // returns to. The word above stands where a caller's return address
        // would be, which also leaves the stack aligned as at a call.
        // SAFETY: a fresh stack.
        unsafe { push_words(top, &[0, 0, 0, 0, 0, 0, entry as *const () as u64, 0]) }
    })
}

/// The thread of a process: its first run restores `frame` and so enters
/// ring 3 in the address space `root`.
pub fn spawn_user(root: u64, pid: u32, frame: &TrapFrame) -> usize {
    new_thread(root, pid, |top| {
        let frame_at = top - size_of::<TrapFrame>() as u64;
        // SAFETY: a fresh stack. The frame lies at its top, where the
        // processor would have built it for an interrupt from ring 3;
        // below it `context_switch` finds six registers and returns to
        // `isr_return`, which restores the frame.
        unsafe {
            ptr::copy_nonoverlapping(frame, frame_at as *mut TrapFrame, 1);
            push_words(frame_at, &[0, 0, 0, 0, 0, 0, tables::isr_return as *const () as u64])
        }
    })
}

fn switch_to(next: usize) {
    let state = STATE.get();
    // SAFETY: one processor, interrupts off. `next` is a thread that was
    // prepared by `new_thread` or left the processor in `context_switch`,
    // so its saved stack pointer leads to six registers and a return
    // address. Its address space has the kernel half of every other, so the
    // kernel keeps running across the change. No reference into the state
    // is alive across the switch.
    unsafe {
        let previous = (*state).current;
        if previous == next {
            return;
        }
        (*state).current = next;
        (*state).switches += 1;
        let Thread { saved_rsp, stack_top, root, .. } = (*state).threads[next];
        tables::set_kernel_stack(stack_top);
        if arch::read_cr3() & ADDRESS != root {
            arch::write_cr3(root);
        }
        context_switch(&raw mut (*state).threads[previous].saved_rsp, saved_rsp);
    }
}

/// Gives the processor to the next ready thread, or to the idle thread.
/// With `requeue` the running thread goes to the back of the ready queue.
fn reschedule(requeue: bool) {
    let state = state();
    if requeue {
        state.queue.add(Tid(state.current as u32));
    }
    let next = state.queue.pick().map_or(IDLE, |tid| tid.0 as usize);
    switch_to(next);
}

/// The running thread lets every other ready thread run first.
pub fn yield_now() {
    reschedule(true);
}

/// The running thread waits until the tick count is `until`.
pub fn sleep_current(until: u64) {
    let state = state();
    state.queue.sleep(Tid(state.current as u32), until);
    reschedule(false);
}

/// The running thread ends. Its stack stays, because this code runs on it.
pub fn exit_current() -> ! {
    let state = state();
    assert!(state.current != IDLE, "the idle thread cannot end");
    state.threads[state.current].alive = false;
    reschedule(false);
    unreachable!("a thread that ended ran again");
}

/// The running thread leaves its process's address space for the kernel's
/// own: from now on a switch back to it loads the kernel's tables.
pub fn leave_address_space() {
    let state = state();
    state.threads[state.current].root = virt::kernel_space().root;
}

/// The process of the running thread.
pub fn current_pid() -> Option<u32> {
    let state = state();
    Some(state.threads[state.current].pid).filter(|&pid| pid != 0)
}

/// The timer interrupt: wakes sleepers, and takes the processor from a
/// program (or from the idle loop) when another thread is ready. Kernel
/// code is never interrupted: it runs with interrupts off.
pub fn on_tick(now: u64, from_user: bool) {
    let state = state();
    state.queue.tick(now);
    let idle = state.current == IDLE;
    if from_user || idle {
        reschedule(!idle);
    }
}

/// Runs the ready threads; then waits for interrupts whenever none is
/// ready. This is the rest of the boot thread's life.
pub fn idle() -> ! {
    loop {
        reschedule(false);
        // SAFETY: `sti` takes effect after the next instruction, so no
        // interrupt is lost between it and `hlt`. The timer handler may
        // switch away on this stack; when the idle thread is chosen again
        // it returns to the `cli`.
        unsafe { asm!("sti", "hlt", "cli", options(nostack)) };
    }
}

/// How often the self-check thread ran.
static CHECK_RUNS: Global<u64> = Global::new(0);

extern "C" fn checker() -> ! {
    for _ in 0..3 {
        // SAFETY: one processor, interrupts off.
        unsafe { *CHECK_RUNS.get() += 1 };
        yield_now();
    }
    exit_current()
}

/// Boot step 17a: a second kernel thread and this one yield to each other
/// three times each. Returns the number of switches that took: six.
pub fn self_check() -> u64 {
    let before = state().switches;
    let other = spawn_kernel(checker);
    let mut mine = 0u64;
    for _ in 0..3 {
        mine += 1;
        yield_now();
    }
    let switches = state().switches - before;
    // SAFETY: one processor, interrupts off.
    let theirs = unsafe { *CHECK_RUNS.get() };
    assert!(mine == 3 && theirs == 3, "the threads did not alternate: {mine} and {theirs} runs");
    // Once more, so that the other thread ends.
    yield_now();
    assert!(!state().threads[other].alive && state().queue.ready_len() == 0, "the second thread did not end");
    switches
}
