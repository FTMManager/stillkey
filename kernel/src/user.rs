//! One ring-3 program: its address space, its start, its system calls and
//! its end (docs/10 sections 8.1 and 8.3).
//!
//! Checkpoint 1 runs one program at a time on the boot stack. There is no
//! scheduler: `yield` returns at once, and when the program ends the kernel
//! idles.

use core::arch::asm;
use core::ptr;

use sk_bootfs::Entry;
use sk_kcore::logfmt::AppText;
use sk_kcore::paging::{ADDRESS, Flags, PAGE, Space};
use sk_kcore::usercopy;
use sk_kcore::x86::TrapFrame;
use stillkey_abi::{
    ABI_VERSION, DEBUG_WRITE_MAX, DEFAULT_MEM_LIMIT, SYS_ABI_VERSION, SYS_CLOCK, SYS_DEBUG_WRITE,
    SYS_EXIT, SYS_YIELD, Status, USER_STACK_SIZE, USER_STACK_TOP,
};

use crate::arch::{self, Global};
use crate::virt::DirectMap;
use crate::{log, phys, tables, timer};

struct Process {
    pid: u32,
    name: [u8; 16],
    name_len: usize,
    space: Space,
    /// Free frames before anything was allocated for the process.
    free_before: u64,
}

impl Process {
    fn name(&self) -> &str {
        // The name comes from BOOTFS.IMG, whose parser accepts only
        // lower-case letters, digits and `-`.
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("?")
    }
}

static CURRENT: Global<Option<Process>> = Global::new(None);
/// The address space the kernel uses when no program runs.
static KERNEL_ROOT: Global<u64> = Global::new(0);
/// Where the stack starts when the kernel idles.
static IDLE_STACK_TOP: Global<u64> = Global::new(0);

pub fn init(kernel_root: u64, idle_stack_top: u64) {
    // SAFETY: one processor, interrupts off; set once before any program.
    unsafe {
        *KERNEL_ROOT.get() = kernel_root;
        *IDLE_STACK_TOP.get() = idle_stack_top;
    }
}

fn flags_of(segment_flags: u32) -> Flags {
    Flags {
        writable: segment_flags & 2 != 0,
        user: true,
        no_execute: segment_flags & 1 == 0,
        cache_disable: false,
    }
}

const STACK_FLAGS: Flags = Flags { writable: true, user: true, no_execute: true, cache_disable: false };

/// Maps one fresh frame at `virt` and returns it.
fn map_fresh(space: &Space, virt: u64, flags: Flags) -> u64 {
    let frame = phys::alloc().expect("no memory for a program's page");
    if let Err(why) = space.map_4k(&mut DirectMap, &mut || phys::alloc(), virt, frame, flags) {
        panic!("mapping {virt:#x} for a program: {why:?}");
    }
    frame
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

/// Builds the address space of the program in `file` and makes it the
/// current process. Returns the frame that starts it.
pub fn spawn(pid: u32, entry: &Entry<'_>, file: &[u8]) -> TrapFrame {
    let free_before = phys::free_count();
    let limit = match entry.mem_limit_mib {
        0 => DEFAULT_MEM_LIMIT,
        mib => u64::from(mib) << 20,
    };
    let image = match sk_elf::validate(file, &sk_elf::user_policy(limit)) {
        Ok(image) => image,
        Err(reason) => panic!("program {} is refused: {}", entry.name, reason.as_str()),
    };

    // SAFETY: set by `init`; one processor, interrupts off.
    let kernel_root = unsafe { *KERNEL_ROOT.get() };
    let space = Space::with_kernel_half(&mut DirectMap, &mut || phys::alloc(), kernel_root)
        .expect("no memory for an address space");

    // The image: each segment with exactly its own permission.
    for segment in image.segments() {
        let flags = flags_of(segment.flags);
        for page in 0..segment.memsz.div_ceil(PAGE) {
            let from = page * PAGE;
            let frame = map_fresh(&space, segment.vaddr + from, flags);
            if from < segment.filesz {
                let take = (segment.filesz - from).min(PAGE) as usize;
                let source = &file[(segment.offset + from) as usize..][..take];
                // SAFETY: the frame is fresh and a whole page; `take` is at
                // most a page; the file is not in that frame.
                unsafe { ptr::copy_nonoverlapping(source.as_ptr(), phys::direct(frame), take) };
            }
            check_page(&space, segment.vaddr + from, frame, flags);
        }
    }
    // The stack, with nothing mapped below it or at its top.
    for page in 1..=USER_STACK_SIZE / PAGE {
        let virt = USER_STACK_TOP - page * PAGE;
        let frame = map_fresh(&space, virt, STACK_FLAGS);
        check_page(&space, virt, frame, STACK_FLAGS);
    }
    for hole in [USER_STACK_TOP - USER_STACK_SIZE - PAGE, USER_STACK_TOP] {
        assert!(space.translate(&mut DirectMap, hole).is_none(), "{hole:#x} must not be mapped");
    }

    let mut name = [0u8; 16];
    let name_len = entry.name.len().min(name.len());
    name[..name_len].copy_from_slice(&entry.name.as_bytes()[..name_len]);
    let process = Process { pid, name, name_len, space, free_before };
    log!(
        "SK:KERNEL:SPAWN pid={} name={} class={} handles=0",
        pid,
        process.name(),
        if entry.class == sk_bootfs::CLASS_SYSTEM { "System" } else { "App" }
    );
    // SAFETY: one processor, interrupts off.
    unsafe { *CURRENT.get() = Some(process) };
    TrapFrame::user_entry(image.entry, USER_STACK_TOP)
}

/// Starts the current process in ring 3 with `frame`. Does not return: the
/// kernel is entered again only through an interrupt gate.
pub fn enter(frame: TrapFrame) -> ! {
    // SAFETY: one processor, interrupts off.
    let root = unsafe { (*CURRENT.get()).as_ref() }.expect("no program to start").space.root;
    // SAFETY: `root` is an address space made by `spawn`: its upper half is
    // the kernel's, so the kernel keeps running across the switch.
    // `isr_return` restores the fifteen registers of `frame` and returns
    // from the interrupt to ring 3 with the selectors, flags, stack and
    // instruction pointer `TrapFrame::user_entry` chose. `frame` lives on
    // this stack until then, because nothing can interrupt before `iretq`.
    unsafe {
        arch::write_cr3(root);
        asm!(
            "mov rsp, {frame}",
            "jmp {restore}",
            frame = in(reg) &raw const frame,
            restore = sym tables::isr_return,
            options(noreturn)
        );
    }
}

/// The kernel has nothing to run: wait for interrupts on a fresh stack.
fn idle() -> ! {
    // SAFETY: set by `init`.
    let top = unsafe { *IDLE_STACK_TOP.get() };
    // SAFETY: nothing on the kernel stack is needed any more (the caller
    // does not return), so the stack restarts at its top. `sti` takes
    // effect after the next instruction, so no interrupt is lost between
    // it and `hlt`; handlers return to the `cli`.
    unsafe {
        asm!("mov rsp, {top}", "2:", "sti", "hlt", "cli", "jmp 2b", top = in(reg) top, options(noreturn));
    }
}

const INVALID_ARGS: u64 = Status::InvalidArgs.as_return();

/// Vector 0x80 from ring 3: number in `RAX`, result in `RAX`. A number the
/// kernel does not know is refused like a wrong argument (D-K-15).
pub fn syscall(frame: &mut TrapFrame) {
    frame.rax = match frame.rax {
        SYS_ABI_VERSION => ABI_VERSION,
        SYS_DEBUG_WRITE => debug_write(frame.rdi, frame.rsi),
        SYS_EXIT => exit(frame.rdi as u32 as i32),
        SYS_YIELD => 0,
        SYS_CLOCK => timer::clock_ns(),
        _ => INVALID_ARGS,
    };
}

/// Call 1: one line of at most 256 bytes for the system log.
fn debug_write(addr: u64, len: u64) -> u64 {
    if len > DEBUG_WRITE_MAX {
        return INVALID_ARGS;
    }
    let mut buffer = [0u8; DEBUG_WRITE_MAX as usize];
    let text = &mut buffer[..len as usize];
    // SAFETY: one processor, interrupts off; the access ends with this call.
    let process = unsafe { (*CURRENT.get()).as_ref() }.expect("a system call without a process");
    let copied = usercopy::copy_in(
        &process.space,
        &mut DirectMap,
        addr,
        text,
        &phys::is_memory,
        &mut |phys, out| {
            // SAFETY: `copy_in` passes an address inside one frame of
            // allocator memory that the program has mapped, with `out` no
            // longer than the rest of that frame.
            unsafe { ptr::copy_nonoverlapping(phys::direct(phys).cast_const(), out.as_mut_ptr(), out.len()) }
        },
    );
    if copied.is_err() {
        return INVALID_ARGS;
    }
    log!("SK:APP:OUT pid={} name={} | {}", process.pid, process.name(), AppText(text));
    0
}

/// Call 2. The order matters (docs/10 section 8.3, "Ending a process"):
/// the processor leaves the program's tables before any of them is freed,
/// because a freed frame is overwritten at once.
fn exit(code: i32) -> ! {
    // SAFETY: one processor, interrupts off.
    let process = unsafe { (*CURRENT.get()).take() }.expect("exit without a process");
    log!("SK:KERNEL:EXIT pid={} name={} code={}", process.pid, process.name(), code);

    // SAFETY: the kernel's own address space; the kernel half is the same
    // in both, so the kernel keeps running.
    unsafe { arch::write_cr3(*KERNEL_ROOT.get()) };
    assert!(arch::read_cr3() & ADDRESS != process.space.root, "still on the tables that are about to be freed");

    let freed = process.space.destroy_user_half(&mut DirectMap, &mut |frame| phys::free(frame));
    phys::free(process.space.root);
    log!("SK:KERNEL:REAPED pid={} frames={}", process.pid, freed.pages + freed.tables + 1);
    assert_eq!(phys::free_count(), process.free_before, "a program's frames were lost or freed twice");
    idle()
}
