//! The boot steps of Milestone 2 (docs/10 section 7).
//!
//! Steps 9 to 12 run on the loader's tables and stack. The rest runs on the
//! kernel's own, where nothing of the first part's stack exists any more.

use core::slice;

use sk_kcore::paging::ADDRESS;
use sk_kcore::x86;
use stillkey_bootinfo::{Accepted, DIRECT_MAP_BASE};

use crate::arch::{self, Global};
use crate::log::Escaped;
use crate::{fbcon, has_option, log, phys, proc, sched, tables, timer, virt};

/// A program of a scenario: its path in BOOTFS.IMG, and whether it gets the
/// handle that reads the system's diagnostics.
type Start = (&'static str, bool);
/// A scenario: its name and the programs it starts, in PID order.
type Scenario = (&'static [u8], &'static [Start]);

/// The start table (docs/10 section 9). The first scenario runs when the
/// command line selects none.
const SCENARIOS: &[Scenario] = &[
    (b"m2", &[("bin/ticker", false), ("bin/faulter", false), ("bin/inspector", true)]),
    (b"m2-hello", &[("bin/hello", false)]),
    (b"m2-preempt", &[("bin/ticker", false), ("bin/spinner", false), ("bin/inspector", true)]),
    (
        b"m2-prober",
        &[
            ("bin/ticker", false),
            ("bin/prober-kread", false),
            ("bin/prober-hlt", false),
            ("bin/prober-in", false),
            ("bin/prober-out", false),
            ("bin/prober-cli", false),
            ("bin/prober-sse", false),
            ("bin/prober-x87", false),
            ("bin/prober-twin-a", false),
            ("bin/prober-twin-b", false),
            ("bin/prober-memcap", false),
            ("bin/prober-badptr", false),
            ("bin/prober-handles", false),
            ("bin/prober-forge", false),
            ("bin/prober-edge", false),
            ("bin/prober-wtext", false),
            ("bin/prober-xstack", false),
            ("bin/prober-xheap", false),
            ("bin/bad-elf", false),
        ],
    ),
];

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

/// What the second part needs from the first. The handoff block itself is
/// loader memory that stays reserved and mapped.
struct Carried {
    info: Accepted<'static>,
    direct_map_regions: u64,
    stack_top: u64,
}

static CARRIED: Global<Option<Carried>> = Global::new(None);

pub fn run(info: &Accepted<'static>) -> ! {
    // ---- Step 9: descriptor tables and a gate for every vector ----
    tables::init(info.header.boot_stack.virt_top);
    log!("SK:KERNEL:ARCH gdt={} idt={} ist={}", x86::GDT_SLOTS, x86::IDT_GATES, x86::IST_STACKS);

    // ---- Step 10: control registers; the 8259 pair remapped and masked ----
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
    timer::init_pic();

    // ---- Step 11: the frame allocator ----
    let frames = phys::init(info);
    log!("SK:KERNEL:FRAMES total={} free={} bitmap_pages={}", frames.total, frames.free, frames.bitmap_pages);

    // ---- Step 12: the kernel's own tables and stack ----
    let built = virt::build(info);
    tables::set_kernel_stack(built.stack_top);
    // SAFETY: one processor, interrupts off; set once.
    unsafe {
        *CARRIED.get() = Some(Carried {
            info: *info,
            direct_map_regions: built.direct_map_regions,
            stack_top: built.stack_top,
        });
    }
    // SAFETY: `build` made the tables and mapped the stack. Everything the
    // rest of the boot needs was copied into `CARRIED`; `info` and every
    // other value on the boot stack is not used again.
    unsafe { virt::enter(built.stack_top, on_own_tables) }
}

/// A byte of data that is also a `ret` instruction. `test=kexec_data` calls
/// it: data must not run.
static RET_IN_DATA: Global<[u8; 16]> = Global::new([0xC3; 16]);

/// Step 14. Each test must end in a page fault taken by the kernel itself.
fn selftest(cmdline: &[u8]) {
    if has_option(cmdline, b"test=kwrite_text") {
        let addr = virt::text_start();
        log!("SK:KERNEL:SELFTEST name=kwrite_text addr={:#x}", addr);
        // SAFETY: none is claimed: the write must fault, because the
        // kernel's code is mapped read-only and `CR0.WP` is set.
        unsafe { (addr as *mut u8).write_volatile(0x90) };
        panic!("the kernel wrote to its own code");
    }
    if has_option(cmdline, b"test=kexec_data") {
        let addr = RET_IN_DATA.get() as u64;
        log!("SK:KERNEL:SELFTEST name=kexec_data addr={:#x}", addr);
        // SAFETY: none is claimed: the call must fault, because data is
        // mapped no-execute and `EFER.NXE` is set.
        let call: extern "C" fn() = unsafe { core::mem::transmute(addr) };
        call();
        panic!("the kernel executed its own data");
    }
}

extern "C" fn on_own_tables() -> ! {
    // SAFETY: set by `run`; one processor, interrupts off. The reference is
    // to a static and nothing writes it again.
    let carried = unsafe { (*CARRIED.get()).as_ref() }.expect("nothing was carried over");
    let info = &carried.info;
    let root = arch::read_cr3() & ADDRESS;
    assert!(root == virt::kernel_space().root, "not on the kernel's own tables");
    log!(
        "SK:KERNEL:PAGING root={:#x} tables={} direct_map_regions={}",
        root,
        virt::table_pages(),
        carried.direct_map_regions
    );
    let released = virt::release_loader_memory(info);
    log!("SK:KERNEL:FRAMES released={} free={}", released, phys::free_count());
    log!("SK:KERNEL:PAGING_AUDIT pages={} wx=0 user=0", virt::audit());

    // ---- Step 13: the log on the screen, from the next line on ----
    match info.framebuffer().and_then(|screen| fbcon::init(&screen)) {
        Some((cols, rows)) => {
            log!("SK:KERNEL:FBCON on cols={} rows={}", cols, rows);
            fbcon::start();
        }
        None => log!("SK:KERNEL:FBCON off"),
    }

    // ---- Step 14: self-test, if selected ----
    selftest(info.cmdline());

    // ---- Step 15: the heap; the kernel half gets its final top level ----
    log!("SK:KERNEL:HEAP base={:#x}", sk_kcore::HEAP_BASE);
    log!("SK:KERNEL:KSPACE sealed tables={}", virt::seal());

    // ---- Steps 16-17: the local APIC and the tick, still masked ----
    let mode = timer::init_apic();
    log!("SK:KERNEL:APIC mode={} lint0=extint", mode);
    timer::init_pit();
    log!("SK:KERNEL:TIMER hz={} clock=ticks", timer::HZ);

    // ---- Step 17a: two kernel threads take turns ----
    sched::init(carried.stack_top);
    log!("SK:KERNEL:SCHED_OK switches={}", sched::self_check());

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

    proc::init();
    for &(path, diag) in programs {
        let Some(entry) = image.find(path) else {
            panic!("{path} is not in BOOTFS.IMG");
        };
        if let Err(reason) = proc::spawn(&entry, image.data(&entry), diag) {
            log!("SK:KERNEL:SPAWN_REFUSED name={} reason={}", entry.name, reason);
        }
    }

    // The mappings made since step 12 obey the same rules.
    virt::audit();

    // ---- Step 20: the tick; this thread becomes the idle thread ----
    timer::start();
    log!("SK:KERNEL:M2_RUNNING");
    sched::idle()
}
