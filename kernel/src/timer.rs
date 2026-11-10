//! The interrupt controllers and the 100 Hz tick (docs/10 section 7, steps
//! 14-15 and 20).
//!
//! The tick comes from the 8254 through line 0 of the 8259, which reaches
//! the processor through the local APIC's LINT0 pin in ExtINT mode. The
//! local APIC's own timer stays off.

use core::sync::atomic::{AtomicU64, Ordering};

use stillkey_abi::TICK_NS;

use crate::arch::{self, Global, inb, io_wait, outb};
use crate::log;

const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;
const PIC_EOI: u8 = 0x20;
const PIC_READ_ISR: u8 = 0x0B;

const PIT_CHANNEL0: u16 = 0x40;
const PIT_COMMAND: u16 = 0x43;
/// 1 193 182 Hz / 11 932 = 99.998 Hz.
const PIT_DIVISOR: u16 = 11932;
pub const HZ: u64 = 100;

const APIC_BASE_MSR: u32 = 0x1B;
const APIC_BASE_X2APIC: u64 = 1 << 10;
const APIC_BASE_ENABLED: u64 = 1 << 11;
const APIC_BASE_ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;
const X2APIC_MSR: u32 = 0x800;
const APIC_VERSION: u32 = 0x30;
const APIC_TPR: u32 = 0x80;
const APIC_SVR: u32 = 0xF0;
const APIC_LVT_CMCI: u32 = 0x2F0;
const APIC_LVT_TIMER: u32 = 0x320;
const APIC_LVT_THERMAL: u32 = 0x330;
const APIC_LVT_PMC: u32 = 0x340;
const APIC_LVT_LINT0: u32 = 0x350;
const APIC_LVT_LINT1: u32 = 0x360;
const APIC_LVT_ERROR: u32 = 0x370;
const APIC_TIMER_COUNT: u32 = 0x380;
const LVT_MASKED: u32 = 1 << 16;
const LVT_EXTINT: u32 = 0x700;
const LVT_NMI: u32 = 0x400;

/// Where the local APIC's registers are: memory, or `None` for the x2APIC's
/// model-specific registers.
static APIC_MEMORY: Global<Option<u64>> = Global::new(None);

static TICKS: AtomicU64 = AtomicU64::new(0);
/// The tick count when the programs were started: the zero of `clock`.
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// Both 8259s: lines 0-15 on vectors 0x20-0x2F, every line masked.
pub fn init_pic() {
    let sequence = [
        (PIC1_COMMAND, 0x11), // start, edge triggered, a fourth word follows
        (PIC2_COMMAND, 0x11),
        (PIC1_DATA, 0x20), // first vector of the master
        (PIC2_DATA, 0x28), // first vector of the slave
        (PIC1_DATA, 0x04), // the slave is on line 2
        (PIC2_DATA, 0x02),
        (PIC1_DATA, 0x01), // 8086 mode
        (PIC2_DATA, 0x01),
        (PIC1_DATA, 0xFF), // every line masked
        (PIC2_DATA, 0xFF),
    ];
    for (port, value) in sequence {
        // SAFETY: the initialisation sequence of the 8259, in ring 0, with
        // interrupts off.
        unsafe { outb(port, value) };
        io_wait();
    }
}

fn apic_read(register: u32) -> u32 {
    // SAFETY: one processor; `init_apic` set the location before any read.
    // The registers are the local APIC's, in the mode the firmware left it.
    unsafe {
        match *APIC_MEMORY.get() {
            Some(base) => ((base + u64::from(register)) as *const u32).read_volatile(),
            None => arch::rdmsr(X2APIC_MSR + register / 16) as u32,
        }
    }
}

fn apic_write(register: u32, value: u32) {
    // SAFETY: as in `apic_read`; the callers write documented values.
    unsafe {
        match *APIC_MEMORY.get() {
            Some(base) => ((base + u64::from(register)) as *mut u32).write_volatile(value),
            None => arch::wrmsr(X2APIC_MSR + register / 16, u64::from(value)),
        }
    }
}

/// The local APIC: enabled, accepting every priority, its own interrupt
/// sources off, LINT0 passing the 8259's interrupts and LINT1 the NMI.
/// Returns the mode's name.
pub fn init_apic() -> &'static str {
    // SAFETY: the register exists on every processor with a local APIC,
    // which every 64-bit processor has.
    let base = unsafe { arch::rdmsr(APIC_BASE_MSR) };
    assert!(base & APIC_BASE_ENABLED != 0, "the firmware left the local APIC disabled");
    let x2apic = base & APIC_BASE_X2APIC != 0;
    // In memory mode the registers get a page of the device window: the
    // direct map holds memory only.
    let memory = (!x2apic).then(|| crate::virt::map_mmio(base & APIC_BASE_ADDRESS));
    // SAFETY: one processor, interrupts off.
    unsafe { *APIC_MEMORY.get() = memory };

    let entries = apic_read(APIC_VERSION) >> 16 & 0xFF; // index of the last LVT entry
    apic_write(APIC_SVR, 0x100 | u32::from(sk_kcore::x86::VECTOR_APIC_SPURIOUS));
    apic_write(APIC_TPR, 0);
    apic_write(APIC_LVT_TIMER, LVT_MASKED);
    apic_write(APIC_TIMER_COUNT, 0);
    apic_write(APIC_LVT_ERROR, LVT_MASKED);
    if entries >= 4 {
        apic_write(APIC_LVT_PMC, LVT_MASKED);
    }
    if entries >= 5 {
        apic_write(APIC_LVT_THERMAL, LVT_MASKED);
    }
    if entries >= 6 {
        apic_write(APIC_LVT_CMCI, LVT_MASKED);
    }
    apic_write(APIC_LVT_LINT0, LVT_EXTINT);
    apic_write(APIC_LVT_LINT1, LVT_NMI);
    if x2apic { "x2apic" } else { "xapic" }
}

/// The 8254: channel 0 as a rate generator at 100 Hz.
pub fn init_pit() {
    let [low, high] = PIT_DIVISOR.to_le_bytes();
    // SAFETY: the documented programming sequence of the 8254, in ring 0.
    unsafe {
        outb(PIT_COMMAND, 0x34); // channel 0, low then high byte, mode 2
        outb(PIT_CHANNEL0, low);
        outb(PIT_CHANNEL0, high);
    }
}

/// Boot step 20: lets the tick through. It is taken as soon as interrupts
/// are on, which is when the first program starts or the kernel idles.
pub fn start() {
    EPOCH.store(TICKS.load(Ordering::Relaxed), Ordering::Relaxed);
    // SAFETY: unmasks line 0 of the master 8259 and nothing else.
    unsafe { outb(PIC1_DATA, 0xFE) };
}

/// Vector 0x20.
pub fn on_tick() {
    let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    if ticks == HZ {
        log!("SK:KERNEL:TIMER_OK ticks={}", ticks);
    }
    // SAFETY: ends the interrupt at the master 8259, which raised it. An
    // ExtINT interrupt is not recorded in the local APIC, so nothing is
    // owed there.
    unsafe { outb(PIC1_COMMAND, PIC_EOI) };
}

/// Vectors 0x27 and 0x2F: line 7 of an 8259, which it also raises when an
/// interrupt went away before it was acknowledged.
pub fn on_spurious(vector: u8) {
    let slave = vector == sk_kcore::x86::VECTOR_PIC_SPURIOUS_SLAVE;
    let command = if slave { PIC2_COMMAND } else { PIC1_COMMAND };
    // SAFETY: asks the 8259 which lines are in service; the answer is read
    // from the same port.
    let in_service = unsafe {
        outb(command, PIC_READ_ISR);
        inb(command)
    };
    // SAFETY: an interrupt that is in service is ended like any other; a
    // spurious one of the slave still leaves line 2 of the master to end.
    unsafe {
        if in_service & 0x80 != 0 {
            outb(command, PIC_EOI);
        }
        if slave {
            outb(PIC1_COMMAND, PIC_EOI);
        }
    }
}

/// Nanoseconds since the programs were started, in steps of one tick.
pub fn clock_ns() -> u64 {
    (TICKS.load(Ordering::Relaxed) - EPOCH.load(Ordering::Relaxed)) * TICK_NS
}
