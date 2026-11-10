//! The boot steps of Milestone 2 (docs/10 section 7), as far as checkpoint 1
//! goes (section 11.1): descriptor tables, control registers, the frame
//! allocator, the timer, BOOTFS.IMG and one program in ring 3.
//!
//! Not yet: the kernel's own page tables and stacks, the heap, the console,
//! the scheduler, and faults charged to a process.

use core::slice;

use sk_kcore::x86;
use stillkey_bootinfo::{Accepted, DIRECT_MAP_BASE};

use crate::log::Escaped;
use crate::{arch, log, phys, tables, timer, user};

/// A scenario: its name and the programs it starts, by path in BOOTFS.IMG.
type Scenario = (&'static [u8], &'static [&'static str]);

/// The scenarios this kernel knows. The first runs when the command line
/// selects none; from checkpoint 2 that is `m2`.
const SCENARIOS: &[Scenario] = &[(b"m2-hello", &["bin/hello"])];

/// The first `test=` word that names a scenario selects it. Other `test=`
/// words are for the loader or for the steps before this one and are
/// ignored here (docs/10 section 6).
fn scenario_of(cmdline: &[u8]) -> Scenario {
    cmdline
        .split(|&b| b == b' ')
        .filter_map(|word| word.strip_prefix(b"test="))
        .find_map(|name| SCENARIOS.iter().find(|(known, _)| *known == name))
        .copied()
        .unwrap_or(SCENARIOS[0])
}

pub fn run(info: &Accepted<'static>) -> ! {
    let h = &info.header;

    // ---- Step 9: descriptor tables and a gate for every vector ----
    tables::init(h.boot_stack.virt_top);
    log!("SK:KERNEL:ARCH gdt={} idt={} ist={}", x86::GDT_SLOTS, x86::IDT_GATES, x86::IST_STACKS);

    // ---- Step 10: control registers ----
    let features = arch::features();
    let cpu = arch::harden(&features);
    if cpu.smap {
        tables::smap_enabled();
    }
    log!(
        "SK:KERNEL:CPU wp={} nxe={} smep={} smap={} x2apic={} tsc_invariant={}",
        u8::from(cpu.wp),
        u8::from(cpu.nxe),
        u8::from(cpu.smep),
        u8::from(cpu.smap),
        u8::from(features.x2apic),
        u8::from(features.tsc_invariant)
    );
    assert!(cpu.wp && cpu.nxe, "write protection and no-execute must be on");

    // ---- Step 11: the frame allocator ----
    let frames = phys::init(info);
    log!("SK:KERNEL:FRAMES total={} free={} bitmap_pages={}", frames.total, frames.free, frames.bitmap_pages);

    // ---- Steps 14-15: interrupt controllers and the tick, still masked ----
    timer::init_pic();
    let mode = timer::init_apic();
    log!("SK:KERNEL:APIC mode={} lint0=extint", mode);
    timer::init_pit();
    log!("SK:KERNEL:TIMER hz={} clock=ticks", timer::HZ);

    // ---- Step 18: BOOTFS.IMG ----
    let Some(range) = info.bootfs() else {
        panic!("the loader handed over no BOOTFS.IMG");
    };
    // SAFETY: the handoff validation showed that the range is one reserved
    // range in loader memory, so the allocator never hands it out and
    // nothing writes to it; the direct map covers it.
    let bytes: &'static [u8] =
        unsafe { slice::from_raw_parts((DIRECT_MAP_BASE + range.base) as *const u8, range.size as usize) };
    let image = match sk_bootfs::parse(bytes) {
        Ok(image) => image,
        Err(reason) => {
            log!("SK:KERNEL:BOOTFS_REJECTED reason={}", reason.as_str());
            panic!("BOOTFS.IMG is refused");
        }
    };
    log!("SK:KERNEL:BOOTFS entries={} bytes={}", image.len(), image.size());

    // ---- Step 19: the start table ----
    let (scenario, programs) = scenario_of(info.cmdline());
    log!("SK:KERNEL:SCENARIO name={} programs={}", Escaped(scenario), programs.len());

    // Checkpoint 1 runs the first program only, on the loader's tables.
    user::init(arch::read_cr3() & sk_kcore::paging::ADDRESS, h.boot_stack.virt_top);
    let path = programs[0];
    let Some(entry) = image.find(path) else {
        panic!("{path} is not in BOOTFS.IMG");
    };
    let frame = user::spawn(1, &entry, image.data(&entry));

    // ---- Step 20: the tick, and the first program ----
    timer::start();
    log!("SK:KERNEL:M2_RUNNING");
    user::enter(frame)
}
