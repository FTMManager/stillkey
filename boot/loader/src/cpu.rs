//! Privileged CPU state: one-line wrappers (docs/02 section 10).

use core::arch::asm;

pub const CR0_WP: u64 = 1 << 16;
/// Global pages.
pub const CR4_PGE: u64 = 1 << 7;
/// 5-level paging is active.
pub const CR4_LA57: u64 = 1 << 12;

const MSR_EFER: u32 = 0xC000_0080;
const EFER_NXE: u64 = 1 << 11;

/// `EAX` and `EDX` of a CPUID leaf (sub-leaf 0).
fn cpuid(leaf: u32) -> (u32, u32) {
    let eax: u32;
    let edx: u32;
    // SAFETY: CPUID exists on every x86-64 CPU and changes no state but its
    // four output registers. `rbx` cannot be named as an operand, so it is
    // saved and restored around the instruction.
    unsafe {
        asm!(
            "mov {saved}, rbx",
            "cpuid",
            "mov rbx, {saved}",
            saved = out(reg) _,
            inout("eax") leaf => eax,
            inout("ecx") 0u32 => _,
            out("edx") edx,
            options(nomem, nostack, preserves_flags),
        );
    }
    (eax, edx)
}

/// `CPUID.80000001H:EDX` bit 20: the CPU supports the no-execute page bit.
pub fn has_nx() -> bool {
    let (highest_extended, _) = cpuid(0x8000_0000);
    highest_extended >= 0x8000_0001 && cpuid(0x8000_0001).1 & (1 << 20) != 0
}

pub fn read_cr0() -> u64 {
    let value: u64;
    // SAFETY: ring 0; reading a control register changes nothing.
    unsafe { asm!("mov {}, cr0", out(reg) value, options(nomem, nostack)) };
    value
}

pub fn read_cr4() -> u64 {
    let value: u64;
    // SAFETY: ring 0; reading a control register changes nothing.
    unsafe { asm!("mov {}, cr4", out(reg) value, options(nomem, nostack)) };
    value
}

fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    // SAFETY: ring 0; the callers pass only EFER, which exists on every x86-64 CPU.
    unsafe {
        asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high,
             options(nomem, nostack, preserves_flags));
    }
    (u64::from(high) << 32) | u64::from(low)
}

/// Interrupts off. Used once, straight after `ExitBootServices` succeeds.
pub fn disable_interrupts() {
    // SAFETY: ring 0. With boot services gone nothing of ours handles interrupts.
    unsafe { asm!("cli", options(nomem, nostack)) };
}

/// Sets `EFER.NXE`. The loader's page tables use the no-execute bit, which is
/// a reserved bit, and so a fault, while `NXE` is clear.
pub fn enable_no_execute() {
    let efer = rdmsr(MSR_EFER) | EFER_NXE;
    // SAFETY: ring 0; `has_nx` was checked in step 1, so the bit may be set.
    // Setting it does not change the meaning of firmware's tables.
    unsafe {
        asm!("wrmsr", in("ecx") MSR_EFER, in("eax") efer as u32, in("edx") (efer >> 32) as u32,
             options(nomem, nostack, preserves_flags));
    }
}

/// Sets `CR0.WP`, so that ring 0 honours read-only mappings.
pub fn enable_write_protect() {
    let cr0 = read_cr0() | CR0_WP;
    // SAFETY: ring 0, boot services exited. The loader writes only to memory
    // it allocated, which firmware's tables map writable.
    unsafe { asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack)) };
}

fn write_cr4(value: u64) {
    // SAFETY: ring 0, boot services exited, interrupts off. The two callers
    // change only bit 7 (PGE) of the value they read; firmware's tables are
    // valid with or without global pages.
    unsafe { asm!("mov cr4, {}", in(reg) value, options(nomem, nostack)) };
}

/// Clears `CR4.PGE` if it is set and leaves every other bit as it was.
/// Returns `CR4` as it was found and as it is now.
///
/// Firmware may have cached translations marked global. A `CR3` switch keeps
/// those, so without this the kernel could run on stale firmware mappings.
/// Clearing the bit discards every cached translation, global ones included.
pub fn disable_global_pages() -> (u64, u64) {
    let before = read_cr4();
    if before & CR4_PGE != 0 {
        write_cr4(before & !CR4_PGE);
    }
    (before, read_cr4())
}

/// Test only (`test=pge`): sets `CR4.PGE`, the state firmware on real
/// machines hands over in, which the VM's firmware does not. Called after
/// `ExitBootServices`, just before `disable_global_pages`.
pub fn set_global_pages_for_test() {
    write_cr4(read_cr4() | CR4_PGE);
}

/// Stops for good.
pub fn halt() -> ! {
    loop {
        // SAFETY: stopping is the intent; no memory is touched.
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) };
    }
}
