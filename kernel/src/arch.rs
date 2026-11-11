//! Instructions and registers that have no Rust spelling, the processor
//! features Milestone 2 looks at, and the control-register settings of
//! boot step 10 (docs/10 section 7).

use core::arch::asm;
use core::cell::UnsafeCell;

/// A kernel global.
///
/// Milestone 2 runs on one processor, and kernel code runs with interrupts
/// off: every gate is an interrupt gate, and the only place that turns them
/// on is the idle loop, which holds no access. So two accesses never overlap.
pub struct Global<T>(UnsafeCell<T>);

// SAFETY: see the type's description: one processor, and no access is held
// while an interrupt can arrive.
unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    pub const fn new(value: T) -> Global<T> {
        Global(UnsafeCell::new(value))
    }

    pub const fn get(&self) -> *mut T {
        self.0.get()
    }
}

/// # Safety
/// Ring 0, and the write must be one the device at `port` expects.
pub unsafe fn outb(port: u16, value: u8) {
    // SAFETY: the caller's contract; the instruction touches no memory.
    unsafe { asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags)) }
}

/// # Safety
/// Ring 0, and reading `port` must have no effect the caller does not want.
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: the caller's contract; the instruction touches no memory.
    unsafe { asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags)) }
    value
}

/// A write to the unused port 0x80: time for an old device to settle.
pub fn io_wait() {
    // SAFETY: port 0x80 is the POST code port; a write has no effect here.
    unsafe { outb(0x80, 0) }
}

/// # Safety
/// The register must exist on this processor.
pub unsafe fn rdmsr(register: u32) -> u64 {
    let (low, high): (u32, u32);
    // SAFETY: the caller's contract; ring 0.
    unsafe {
        asm!("rdmsr", in("ecx") register, out("eax") low, out("edx") high, options(nomem, nostack, preserves_flags));
    }
    u64::from(high) << 32 | u64::from(low)
}

/// # Safety
/// The register must exist and the value must be one the kernel can run with.
pub unsafe fn wrmsr(register: u32, value: u64) {
    // SAFETY: the caller's contract; ring 0.
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") register,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nomem, nostack, preserves_flags)
        );
    }
}

macro_rules! control_register {
    ($read:ident, $write:ident, $name:literal) => {
        pub fn $read() -> u64 {
            let value: u64;
            // SAFETY: ring 0; reading a control register changes nothing.
            unsafe { asm!(concat!("mov {}, ", $name), out(reg) value, options(nomem, nostack)) }
            value
        }

        /// # Safety
        /// The value must be one the kernel can run with.
        pub unsafe fn $write(value: u64) {
            // SAFETY: the caller's contract.
            unsafe { asm!(concat!("mov ", $name, ", {}"), in(reg) value, options(nostack)) }
        }
    };
}

control_register!(read_cr0, write_cr0, "cr0");
control_register!(read_cr3, write_cr3, "cr3");
control_register!(read_cr4, write_cr4, "cr4");

/// The address a page fault was taken for.
/// Drops the translation of one page from the processor's cache.
pub fn invlpg(virt: u64) {
    // SAFETY: the instruction changes no memory and no register; a
    // dropped translation is looked up again in the tables.
    unsafe { asm!("invlpg [{}]", in(reg) virt, options(nostack, preserves_flags)) };
}

/// `RFLAGS.AC`: with `CR4.SMAP` set, ring 0 may touch user pages only
/// while this flag is set.
pub const RFLAGS_AC: u64 = 1 << 18;

pub fn read_rflags() -> u64 {
    let value: u64;
    // SAFETY: the two instructions use eight bytes of this stack and
    // leave the stack pointer and the flags as they were.
    unsafe { asm!("pushfq", "pop {}", out(reg) value, options(nomem, preserves_flags)) }
    value
}

pub fn read_cr2() -> u64 {
    let value: u64;
    // SAFETY: ring 0; reading the register changes nothing.
    unsafe { asm!("mov {}, cr2", out(reg) value, options(nomem, nostack)) }
    value
}

/// `EAX`, `EBX`, `ECX`, `EDX` of a `cpuid` leaf.
pub fn cpuid(leaf: u32, subleaf: u32) -> [u32; 4] {
    let (eax, ecx, edx): (u32, u32, u32);
    let ebx: u64;
    // SAFETY: `cpuid` exists on every 64-bit processor and changes only the
    // four registers. `RBX` cannot be named as an operand, so it is swapped
    // with a scratch register around the instruction.
    unsafe {
        asm!(
            "mov {scratch}, rbx",
            "cpuid",
            "xchg {scratch}, rbx",
            scratch = out(reg) ebx,
            inout("eax") leaf => eax,
            inout("ecx") subleaf => ecx,
            out("edx") edx,
            options(nomem, nostack, preserves_flags)
        );
    }
    [eax, ebx as u32, ecx, edx]
}

pub struct Features {
    /// The kernel cannot execute a page ring 3 can reach.
    pub smep: bool,
    /// The kernel cannot read or write a page ring 3 can reach.
    pub smap: bool,
    pub x2apic: bool,
    /// The time-stamp counter runs at one rate in every power state.
    pub tsc_invariant: bool,
}

pub fn features() -> Features {
    let structured = if cpuid(0, 0)[0] >= 7 { cpuid(7, 0)[1] } else { 0 };
    let extended = cpuid(0x8000_0000, 0)[0];
    Features {
        smep: structured & 1 << 7 != 0,
        smap: structured & 1 << 20 != 0,
        x2apic: cpuid(1, 0)[2] & 1 << 21 != 0,
        tsc_invariant: extended >= 0x8000_0007 && cpuid(0x8000_0007, 0)[3] & 1 << 8 != 0,
    }
}

const CR0_MP: u64 = 1 << 1;
const CR0_EM: u64 = 1 << 2;
const CR0_TS: u64 = 1 << 3;
const CR0_WP: u64 = 1 << 16;
const CR4_PVI: u64 = 1 << 1;
const CR4_OSFXSR: u64 = 1 << 9;
const CR4_OSXMMEXCPT: u64 = 1 << 10;
const CR4_OSXSAVE: u64 = 1 << 18;
const CR4_SMEP: u64 = 1 << 20;
const CR4_SMAP: u64 = 1 << 21;
const EFER: u32 = 0xC000_0080;
const EFER_SCE: u64 = 1 << 0;
const EFER_NXE: u64 = 1 << 11;
const SYSENTER_CS: u32 = 0x174;

/// What the control registers say after [`harden`].
pub struct Hardened {
    pub wp: bool,
    pub nxe: bool,
    pub smep: bool,
    pub smap: bool,
}

/// Boot step 10: the kernel honours read-only pages, floating-point and
/// vector instructions fault in every ring (no program has such state in
/// Milestone 2, and none is saved), `syscall` and `sysenter` are off (the
/// only way in is `int 0x80`), and the kernel can neither run nor touch
/// memory that ring 3 can reach, where the processor offers that.
pub fn harden(features: &Features) -> Hardened {
    // SAFETY: ring 0. Each value keeps what the kernel runs with: paging and
    // long mode are not changed; the kernel has no floating-point code, its
    // pages are supervisor pages, and it reaches a program's memory through
    // its own direct map only.
    unsafe {
        write_cr0(read_cr0() | CR0_WP | CR0_MP | CR0_EM | CR0_TS);
        let mut cr4 = read_cr4() & !(CR4_PVI | CR4_OSFXSR | CR4_OSXMMEXCPT | CR4_OSXSAVE);
        if features.smep {
            cr4 |= CR4_SMEP;
        }
        if features.smap {
            cr4 |= CR4_SMAP;
        }
        write_cr4(cr4);
        wrmsr(EFER, rdmsr(EFER) & !EFER_SCE);
        wrmsr(SYSENTER_CS, 0);
    }
    let cr4 = read_cr4();
    Hardened {
        wp: read_cr0() & CR0_WP != 0,
        // SAFETY: the register exists on every 64-bit processor.
        nxe: unsafe { rdmsr(EFER) } & EFER_NXE != 0,
        smep: cr4 & CR4_SMEP != 0,
        smap: cr4 & CR4_SMAP != 0,
    }
}
