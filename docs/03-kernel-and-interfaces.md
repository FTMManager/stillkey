# 03 · Kernel architecture and the minimum interfaces

Status: design proposal. Architecture facts are cited to the Intel SDM, the AMD64 manual and the System V ABI in [09](09-references.md#cpu-architecture-abi-and-elf); firmware facts are cited `[UEFI §n]` as in [02](02-boot-and-handoff.md). Choices marked **D-K-n** are design decisions ([01 §3](01-overview-and-architecture.md#3-design-decisions)).

## 1. Shape of the kernel

Stillkey uses a **small capability kernel**: the kernel owns the CPU, memory, scheduling, handles and message passing. Policy and anything with a user interface live in ring-3 services. It is not a pure microkernel in Milestones 1–4, because three tiny pieces stay in the kernel for practical reasons, each with an exit plan.

| Component | Runs in | Why there | Later |
|---|---|---|---|
| CPU setup, exceptions, interrupt controller, timer | Kernel | Privileged instructions; nothing else can | Stays |
| Physical and virtual memory, kernel heap | Kernel | Isolation depends on it | Stays |
| Scheduler, threads, processes | Kernel | Must pre-empt untrusted code | Stays |
| Handles, rights, grants, channels | Kernel | The enforcement point for every access decision ([04](04-security-and-permissions.md)) | Stays |
| Diagnostics snapshots, crash ring, audit log | Kernel | Must be truthful even when user space is broken | Stays |
| Serial port | Kernel | Log output is needed before anything else works. From Milestone 3 an interrupt handler also queues received bytes onto the `Console` object, which is the operator's input until the desktop exists (D-K-10) | Stays as debug sink |
| Read-only boot filesystem (`bootfs`) | Kernel (about 200 lines, no writes, no parsing of untrusted input after boot) | `init` must be loaded from somewhere before any service exists | Moves behind a user-space file service when writable storage arrives |
| PS/2 keyboard interrupt handler | Kernel (handler only: reads one byte, queues it) | An interrupt must be acknowledged in ring 0; routing and key mapping are done by `desktop` | Becomes a user-space driver with an interrupt handle |
| Framebuffer | No driver. A physical memory range the kernel exposes as a mappable object | Firmware already set the mode | Replaced by a user-space display driver |
| Permission broker `permd`, `init`, `desktop`, Workbench, shell, apps | User space | Policy, parsing and UI are where bugs live; a crash there is contained and visible in the Workbench | Stays |
| Future drivers (block storage, USB, GPU, network) | User space, one process each, holding handles for their device memory and interrupts | Fits the product: a driver is a process you can inspect, with access you can list | – |

The justification in one sentence: **anything that can be a process should be, because processes are the unit the Workbench can show, limit and restart.**

## 2. Subsystems and their boundaries

```
kernel/src
├─ arch/x86_64/   entry, GDT/TSS, IDT, exception and interrupt stubs, system-call entry,
│                 context switch, control registers, MSRs, CPUID, port I/O,
│                 interrupt controllers, timer
├─ mm/            phys (frame allocator), virt (address spaces and mappings), heap, usercopy
├─ sched/         threads, run queue, time slices, sleep and wait queues
├─ proc/          process lifecycle, user ELF loading, crash records
├─ object/        object types, per-process handle tables, rights, grants
├─ ipc/           channels (bytes plus handles)
├─ fs/            bootfs (read-only)
├─ diag/          snapshots, crash ring, audit log
├─ drivers/       serial16550 (log out; console in from Milestone 3), ps2kbd,
│                 fbcon (kernel log on the framebuffer)
├─ syscall/       the dispatch table and argument validation; the only way in from ring 3
└─ log.rs, panic.rs
```

Boundary rules, enforced by module visibility and review:

1. Only `arch` contains assembly or privileged instructions. Everything else calls safe wrappers.
2. Only `mm::virt` edits page tables. Only `mm::usercopy` acts on an address supplied by user space, and it never dereferences one: it walks the caller's page tables and copies through the direct map (section 3).
3. Only `object` creates handles or checks rights. A subsystem receives an already-checked object reference, never a raw handle value.
4. `syscall` is the only code reachable from ring 3. Each entry validates arguments, resolves handles through `object`, then calls a subsystem.
5. `diag` has read-only access to other subsystems' state through snapshot functions they export.
6. Every `unsafe` block carries a comment stating the invariant that makes it sound. `unsafe` outside `arch`, `mm` and `drivers` needs a justification in review.

## 3. Virtual address layout

48-bit canonical addresses with 4-level paging. Firmware may run with 5-level paging, and the depth cannot be changed in long mode, so the loader refuses to hand over when `CR4.LA57` is set (`SK:LOADER:CPU_UNSUPPORTED feature=la57_active`; the entry contract in [02 §9](02-boot-and-handoff.md#9-the-jump-and-the-kernels-entry-contract) says "Paging: 4-level, CR4.LA57 = 0"). The loader builds the first set of tables ([02 §5](02-boot-and-handoff.md#5-loading-segments-mappings-and-the-stack)). The Milestone 1 kernel runs on them; from Milestone 2 the kernel builds its own.

| Range | Use | Permissions |
|---|---|---|
| `0x0000_0000_0000_0000` – `0x0000_0000_003F_FFFF` | Never mapped (catches null and small-offset bugs) | – |
| `0x0000_0000_0040_0000` – `0x0000_7FFF_FFFF_EFFF` | User space, one set of tables per process | User; per segment: code R+X, read-only data R, data and stack R+W and no-execute |
| `0xFFFF_8000_0000_0000` + physical address | Direct map of RAM (2 MiB pages; 4 KiB pages at the ragged edges of a region) | Kernel only, R+W, no-execute |
| `0xFFFF_C000_0000_0000` … | Kernel heap | Kernel only, R+W, no-execute |
| `0xFFFF_D000_0000_0000` … | Kernel stacks, each with an unmapped guard page below | Kernel only, R+W, no-execute |
| `0xFFFF_E000_0000_0000` … | Framebuffer as seen by the kernel log console | Kernel only, R+W, no-execute |
| `0xFFFF_F000_0000_0000` … | Device registers, mapped on demand in 4 KiB pages: the local APIC page in Milestone 2; the I/O APIC and the HPET in stage 5-prep ([08](08-hardware-and-dual-boot.md)) | Kernel only, R+W, no-execute, cache-disabled (`PCD` + `PWT`) |
| `0xFFFF_FFFF_8000_0000` … | Kernel image (top 2 GiB, so the `kernel` code model applies) | Text R+X; read-only data R; data and bss R+W, no-execute |

**What the direct map covers.** In Milestone 1 it is the loader's: physical 0 … D, where D follows the rule in [02 §5](02-boot-and-handoff.md#5-loading-segments-mappings-and-the-stack) (at least 4 GiB, at most the 64 TiB this window holds). `BootInfo` carries the base (`direct_map_virt_base`) and not the size; the kernel computes D with the function the loader used, `stillkey_bootinfo::direct_map_size`. From Milestone 2 the kernel's own tables direct-map only memory regions of kinds 1–4 and 6 ([02 §7](02-boot-and-handoff.md#7-memory-the-kernel-must-reserve-before-starting-its-allocator)): usable, loader, boot-services, ACPI-reclaimable and firmware-runtime memory. Device memory, holes and every other kind are left out, so a stray pointer cannot reach a device through a cacheable 2 MiB page. Nothing outside the direct map is reachable except through the framebuffer window and the device window.

Kernel-wide settings: `EFER.NXE` on and `CR0.WP` on (set by the loader after `ExitBootServices`, before the jump); `CR4.SMEP` and `CR4.SMAP` turned on in Milestone 2 when CPUID reports them.

No virtual mapping is both writable and executable. Kernel text and read-only data remain writable through the direct-map alias (ring 0 only) in Milestones 1–4. The permissions of the kernel window are tested from Milestone 2 by two kernel self-tests, `test=kwrite_text` and `test=kexec_data`; each must end in a kernel panic naming vector 14 (status 37 under test).

**User memory under SMAP (D-K-9).** With `CR4.SMAP` on, ring 0 faults when it touches a user page unless `RFLAGS.AC` is set. Ring 3 can set that flag itself with `popfq`, and interrupt delivery does not clear it. Three rules follow:

1. Every entry stub reachable from ring 3 (exceptions, interrupts, `int 0x80`) executes `clac` next to `cld`, before any Rust code, when CPUID reports SMAP. The stub tests a flag set at boot, because `clac` is an invalid opcode on a CPU without SMAP.
2. `mm::usercopy` never dereferences a user address. It checks that the range lies in the user half and does not wrap, then for each page walks the caller's page tables, requires present + user (and writable for a copy out), and copies through the direct map. A failed check returns `INVALID_ARGS` and never faults. A page whose frame is outside the direct map (a device mapping such as the framebuffer) fails the same check.
3. No `stac` exists in Milestones 2–4.

The alternative is the conventional one: bracket a direct copy with `stac` … `clac` and recover from the fault. It is faster for large copies and needs a fault-fixup table.

## 4. Assembly and `unsafe` at the CPU boundary

Everything else is safe Rust. The full list for Milestones 1–4:

| Item | Form | Why it cannot be safe Rust |
|---|---|---|
| Kernel entry `_start` | Plain `extern "sysv64" fn _start(block: *const u8, block_len: u64) -> !`: the address of the handoff block in `RDI` and its mapped length in `RSI` | No assembly is needed for the entry: the loader already set the stack and the arguments. A `&BootInfo` parameter would assert a valid, full-size structure before anything was checked. So the kernel initialises serial, prints `SK:KERNEL:START`, checks the two values without reading memory (`stillkey_bootinfo::check_block`), and only then makes the one `unsafe` step of the handoff: a byte slice of exactly the mapped length. Everything after that is safe code in `stillkey-bootinfo`, which forbids `unsafe`: it applies the acceptance rules of [02 §6](02-boot-and-handoff.md#6-bootinfo-version-1) to the slice and decodes the header and every table entry by value, so no reference into the block is ever formed from a self-reported size. A rejected block prints `SK:KERNEL:BOOTINFO_REJECTED` with a reason and halts |
| Loading the GDT, TSS and IDT | A few inline `asm!` lines of our own (D-K-8), in this order: `lgdt`; load `SS` with kernel data (`0x10`) and `DS`, `ES`, `FS`, `GS` with 0; reload `CS` with a far return (`push 0x08`, `lea rax, [rip + 2f]`, `push rax`, `retfq`, label `2:`); `ltr 0x28`; `lidt`. The `x86_64` crate is the alternative; on stable Rust it must be used with its default features off, and interrupt handlers are then registered by address | Privileged instructions. Every segment register is reloaded because firmware's selectors index a different table: under OVMF `SS` is `0x30`, which in our GDT is the upper half of the TSS descriptor, and the first `iretq` back to ring 0 would fault on it |
| Exception and interrupt stubs | One naked function per vector (generated by a macro): push a dummy error code where the CPU pushes none, save the general registers, `cld`, `clac` when SMAP is on, call a Rust handler with a pointer to the frame, restore, `iretq` | The CPU's interrupt frame is not a Rust calling convention |
| System-call entry | The same stub mechanism on vector `0x80` (D-K-4) | As above |
| Context switch | One naked function of about fifteen instructions: save callee-saved registers and `RSP` of the outgoing thread, load the incoming thread's | Swaps stacks under the compiler's feet |
| First entry to ring 3 | Build an interrupt frame and `iretq`: `SS` `0x1B`, the user `RSP`, `RFLAGS` `0x202` (interrupts on, IOPL 0), `CS` `0x23`, the entry `RIP`; all general registers zeroed | Changes privilege level |
| Control registers, MSRs, `invlpg`, `cli`/`sti` | One-line inline `asm!` wrappers | Privileged instructions |
| `cpuid`, `rdtsc` | One-line wrappers | Feature detection (SMEP, SMAP, x2APIC) and the time base for `clock()` |
| Idle loop | One block, `asm!("sti", "hlt", "cli")` | `sti` takes effect after the following instruction, so no interrupt can land between enabling interrupts and halting |
| Port I/O (`in`, `out`) for serial, the 8259 pair, the 8254, PS/2 | One-line wrappers | No memory-mapped equivalent |
| Local APIC registers | Volatile reads and writes through the device window, or `rdmsr`/`wrmsr` in x2APIC mode | Device registers |
| Page-table edits, frame zeroing, the direct map | `unsafe` pointer writes inside `mm` | Raw physical memory |
| Copying to and from user memory | `unsafe` inside `mm::usercopy`: a walk of the caller's page tables, then a copy through the direct map (D-K-9, section 3) | User pointers are untrusted |

Compiler settings that matter at this boundary: the kernel is built with no red zone and without SSE, MMX or x87 (the defaults of the `x86_64-unknown-none` target), because interrupts push onto the current stack and the kernel does not save vector state for itself.

## 5. Minimum interfaces by concern

### 5.1 Faults

- A GDT with kernel code and data, user data and code, and a TSS. The TSS provides `RSP0` (the kernel stack used when an interrupt arrives from ring 3) and separate interrupt-stack-table entries for double fault, NMI and machine check, so those handlers run on a known-good stack.
- GDT order: null, kernel code `0x08`, kernel data `0x10`, user data `0x18`, user code `0x20`, TSS `0x28` (two slots). This order is the one `SYSCALL`/`SYSRET` require, so adopting them later needs no change. After `lgdt` every segment register is reloaded (section 4).
- **No port access from ring 3.** The TSS limit is `0x67` and its I/O map base field (offset `0x66`) is `0x68`, so no I/O permission bitmap exists. Every user `RFLAGS` image has IOPL 0, and the kernel keeps `CR4.PVI` clear. `in`, `out`, `ins`, `outs`, `cli` and `sti` in ring 3 therefore raise general protection (vector 13). A zero-filled TSS would not do: with the base field at 0 the CPU reads the TSS's own bytes as the bitmap and gives ring 3 every port whose bit happens to be 0.
- An IDT in which **all 256 vectors have a gate**, each a 64-bit interrupt gate (type `0xE`), so interrupts are off on entry. Vectors 0–31 are the exceptions, `0x20`–`0x2F` the legacy controller's lines, `0x80` the system call, `0xFF` the local APIC's spurious vector. Each handler receives the saved frame. Stubs push a dummy error code for vectors where the CPU pushes none, so every frame has one layout, and execute `cld` and (with SMAP) `clac` before calling Rust. Every gate has privilege level 0 except the system-call vector, so ring 3 cannot invoke exception handlers with `int n`.
- Spurious and unexpected interrupts. Vectors `0x27` and `0x2F` read the 8259's in-service register; if the line's bit is clear the interrupt is spurious and the handler returns, sending no EOI for `0x27` and an EOI to the master only for `0x2F`. Vector `0xFF` returns without EOI. Any other vector with no registered handler is logged and is a kernel panic; it is never charged to a process.
- Nothing about interrupt state at handoff is assumed. Firmware's timer under OVMF is the local APIC timer, and the legacy controller may be unmasked or not remapped. The kernel sets every piece itself before enabling interrupts (the list is in 5.3).
- Policy. **A fault is charged to a process** only if all three hold: the saved `CS` has RPL 3; the vector is a synchronous exception of the process's own instruction, one of 0, 1, 3, 4, 5, 6, 7, 11, 12, 13, 14, 16, 17, 19; and the error code's EXT bit is 0 where the error code has one. The kernel then writes a crash record ([04 §6](04-security-and-permissions.md#6-crash-records)), prints `SK:KERNEL:CRASH pid=<n> name=<s> vector=<n> addr=<hex> rip=<hex>` (hexadecimal in lower case with a `0x` prefix and no leading zeros; `addr` is `CR2` for vector 14 and `0x0` for every other vector), ends the process and schedules another. **Everything else is a kernel panic**, whatever the saved `CS`: a fault taken in ring 0, vectors 2, 8 and 18, EXT = 1, or an unhandled vector. A panic turns interrupts off, prints a banner with a distinct first line on serial and on the framebuffer and a register dump, then halts (or exits QEMU with status 37 under test).
- The page-fault handler decodes the error code (present, write, user, reserved bit, instruction fetch) and `CR2` into the crash record.

### 5.2 Paging and allocation

- **Frame allocator**: a bitmap over usable physical memory, initialised from the boot memory map after reserving everything in [02 §7](02-boot-and-handoff.md#7-memory-the-kernel-must-reserve-before-starting-its-allocator). Interface: `alloc_frame`, `alloc_frames(n)`, `free_frame`, counters for the Workbench.
- **Address spaces**: one top-level table per process. The upper half is shared by copying the kernel's 256 top-level entries at creation; the lower half is private. Copying is sound only if those entries never change afterwards. So before the first process is created the kernel gives every kernel-half top-level entry its next-level table (256 tables, 1 MiB), and it never adds, removes or replaces one later; `mm::virt` panics if a kernel mapping would need a new one. Interface: `map(range, frames, flags)`, `unmap`, `protect`, `translate`. Mappings are eager; demand paging and copy-on-write are deferred.
- **Kernel heap**: a simple allocator behind Rust's `#[global_allocator]`, growing page by page in the heap window. Boot-time allocation failure is a panic. Allocation inside a system call is fallible and returns `NO_MEMORY` to the caller: those paths use `try_reserve` and the kernel's own fallible constructors, never a call that ends in the allocation-error handler.
- **User memory**: `mem_alloc` and `mem_free` give zeroed, non-executable, read-write pages, at most 1 MiB per call; a larger request returns `INVALID_ARGS`.
- **Per-process memory limit**: exceeding it returns `NO_MEMORY`. The figure comes from the image's bootfs table entry (section 6): 16 MiB when the entry says 0, which is the case for every App-class program (D-K-6), otherwise the entry's value. `desktop`'s entry says 64 MiB and carries the `display` flag, so its limit is 64 MiB, or 8 × the framebuffer size if that is larger. The limit counts every frame charged to the process: image, heap, stack, page tables, the `Memory` objects it created, and kernel memory used on its behalf (handle slots, channel objects, and the bytes of messages it has queued until they are read). A shared `Memory` object is charged to its creator only; mapping someone else's costs page tables and nothing more. Device memory, such as the framebuffer mapping, is not charged.
- **Fixed limits**: at most 256 handles per App process (4096 for class System). A message is at most 64 KiB and 16 handles. A channel direction queues at most 64 messages and 256 KiB; beyond that `channel_write` returns `NO_MEMORY`. With these and the memory limit, a process that floods channels or handles runs into its own limit and gets `NO_MEMORY`; it cannot panic the kernel.
- Per-process counters (image, heap, stack, shared, page tables) feed the Workbench. The shared counter shows what the process has mapped, whoever is charged for it.

### 5.3 Scheduling

- One CPU through Milestone 4 (D-K-2). Other cores are left halted as firmware left them.
- Threads have states Ready, Running, Waiting (with a reason code that the Workbench shows in its detail pane), Exited. One thread per process until needed.
- Pre-emptive round-robin with a 10 ms time slice driven by a periodic timer interrupt. The idle thread is the single `sti`/`hlt`/`cli` block from section 4.

**Timer (D-K-3).** In the VM milestones (2–4) the tick is the legacy interval timer (8254) at 100 Hz, delivered through the legacy interrupt controller (8259 pair) remapped to vectors `0x20`–`0x2F`. It has a fixed, documented frequency and needs no calibration. The local APIC timer, which firmware used, is masked. An 8259 interrupt reaches the CPU only through the local APIC's LINT0 input, so the kernel programs that path too instead of inheriting it. Before interrupts are first enabled it writes:

| Device | Setting |
|---|---|
| Local APIC | Read `IA32_APIC_BASE`. If bit 10 is set the APIC is in x2APIC mode and its registers are the MSRs from `0x800`; otherwise map the base page in the device window (section 3). Spurious-interrupt vector register = `0x1FF` (software-enabled, spurious vector `0xFF`). Task priority = 0. LVT timer = `0x0001_0000` (masked), initial count = 0. LVT LINT0 = `0x700` (ExtINT, unmasked). LVT LINT1 = `0x400` (NMI). The other LVT entries that exist (error, performance counter, thermal, CMCI) are masked. The software-enable bit is never cleared, because a software-disabled APIC masks every LVT entry and the 8259 would no longer reach the CPU |
| 8259 pair | ICW1 `0x11`, ICW2 `0x20` / `0x28`, ICW3 `0x04` / `0x02`, ICW4 `0x01`. Masks `0xFE` / `0xFF`: everything masked except the timer. Milestone 3 also unmasks the serial port, IRQ 4 (master mask `0xEE`); Milestone 4 the keyboard, IRQ 1 (`0xEC`) |
| 8254 | Port `0x43` ← `0x34`; port `0x40` ← `0x9C`, then `0x2E` (divisor 11932, 99.998 Hz) |
| I/O APIC | Redirection entries are left masked |

On hardware the 8254 is never used. Stage 5-prep of Milestone 5 ([08](08-hardware-and-dual-boot.md)), done in the VM before the first hardware boot, moves the tick to the local APIC timer. It is calibrated against the ACPI PM timer, falling back to the HPET, or its rate is taken from CPUID leaf 15H/16H when present. Interrupt routing then comes from the MADT. Real machines may not provide the legacy devices at all, which is why none of this may depend on them.

- **The kernel is not pre-emptible (D-K-7).** Interrupts are disabled while executing kernel code and enabled in user mode and in the idle loop. A system call therefore runs without interleaving from entry until it returns or blocks. Consequences: no kernel locks are needed on one CPU, and the revocation guarantee in [04 §4.1](04-security-and-permissions.md#41-behaviour-case-by-case) needs no lock either: a read either completes before the revoke or returns `REVOKED` after it. With several CPUs the same guarantee will need the grant's lock.
- **Every system call does a bounded amount of work.** `file_read` and a channel message move at most 64 KiB, `mem_alloc` zeroes at most 1 MiB, `debug_write` takes at most 256 bytes. A larger job returns a count and `libstill` loops. The kernel never loops over chunks inside one call: with interrupts off that would shorten nothing.
- **Blocking calls** (`wait`, `wait_many`, `sleep`, `channel_read` with nothing queued, `console_read` with nothing queued) put the thread on a wait queue and switch away. A blocked call holds only its wait-queue entry; when woken it resolves its handles again and restarts. Ending a Waiting thread is unlinking that entry and freeing the thread. Sleeps and deadlines are rounded up to the next tick.
- **Time.** In Milestone 2 `clock()` counts timer ticks of 10 ms; what follows is the design and is not implemented yet ([10 §11.5](10-milestone-2-spec.md#115-independent-review-and-fixes-2026-11-11)). `clock()` reads the TSC, scaled by a calibration made once at boot against the tick's reference (the 8254 in the VM), so a tick lost while interrupts are off cannot make it drift. **Assumption A-17** ([01 §2](01-overview-and-architecture.md#2-assumptions-to-verify)): the CPU has an invariant TSC (CPUID leaf `0x8000_0007`, EDX bit 8), so the rate measured at boot holds for the whole session. The kernel logs the bit at Milestone 2 boot. Without it `clock()` does not use the TSC: it falls back to counting timer ticks, in steps of 10 ms. The VM's `-cpu max` is not expected to report the bit, so the VM milestones are expected to run on the fallback.

### 5.4 Process isolation

- A process is an address space, a handle table, one or more threads, counters and a class (System or App).
- A PID is a 64-bit number, assigned in increasing order and never reused within a boot, so a PID in a grant, crash record, audit entry or Workbench message always means the same process. Grant ids follow the same rule. `init` is PID 1.
- A handle value is 32 bits: bits 0–11 are the slot index in the caller's table and bits 12–31 a generation, so a stale value does not become valid again when its slot is reused. The value 0 is never valid. Objects and rights are defined in [04 §3](04-security-and-permissions.md#3-objects-handles-and-rights).
- User programs are static ELF64 executables loaded by the kernel from `bootfs` with the same strict validator the loader uses for the kernel image (`sk-elf`, [02 §4](02-boot-and-handoff.md#4-validating-the-kernel-elf)). The validator takes a policy. The user policy is: every `PT_LOAD` inside the window `0x40_0000` … `0x8000_0000`, `max_file` of 16 MiB, and `max_mem` equal to the process's memory limit. The window ends at 2 GiB because the target's `kernel` code model reaches symbols through sign-extended 32-bit addresses. As for the kernel image: no segment both writable and executable; program headers other than `PT_LOAD` are ignored, except `PT_INTERP`, `PT_DYNAMIC` and `PT_TLS`, which are rejected.
- A new process gets its image, a stack with a guard page, and exactly the handles its creator passed. In Milestone 2 that is none (the `inspector` gets the `Diag` handle). From Milestone 3 it is one, the process's end of its bootstrap channel, always the value `0x1000` (D-K-25); everything else arrives as messages on that channel (section 7.1). The kernel prints `SK:KERNEL:SPAWN pid=<n> name=<s> class=<s> handles=<n>` for every start; `class` is `App` or `System`.
- **Vector state policy for Milestones 2–4 (D-K-5)**: user programs are built without SSE or x87, and the kernel switches those instruction sets off before the first entry to ring 3. Firmware hands over with x87 and SSE enabled (`CR0.EM` = 0, `CR0.TS` = 0, control words initialised, `[UEFI §2.3.4]`), so leaving the state alone would let ring 3 use registers that nothing saves. In Milestone 2 `arch` sets `CR0.EM`, `CR0.MP` and `CR0.TS`, clears `CR4.OSFXSR`, `CR4.OSXMMEXCPT` and `CR4.OSXSAVE`, and never changes them again. The setting is global; that is safe because the kernel itself contains no vector or floating-point code. In an app, an SSE or AVX instruction then raises invalid-opcode (vector 6) and an x87 instruction raises device-not-available (vector 7); either ends the app with a crash record. In the same step the kernel clears `EFER.SCE` and `IA32_SYSENTER_CS`, so `syscall` and `sysenter` in ring 3 fault instead of entering ring 0 at an address nobody set. This is a security decision, not only a simplification: if SSE were enabled without saving and restoring the registers per thread, one process could read another's leftover vector registers. The alternative is to do the saving in Milestone 2: enable SSE for ring 3 and keep a 512-byte `FXSAVE` area per thread, saved and restored at every thread switch. That is about fifty lines and lets user programs use hardware floating point and SIMD (software floating point already works on this target).
- Address-space layout randomisation is deferred and stated as not present.

### 5.5 System calls

- Entry: software interrupt `int 0x80` through an IDT gate callable from ring 3 (D-K-4). The CPU switches to the thread's kernel stack via the TSS and builds the same frame as for any interrupt, so there is one entry path to get right. `SYSCALL`/`SYSRET` is faster and can replace it later without changing the ABI; it is deferred because it needs manual stack switching, and because returning with `SYSRET` to a non-canonical address faults in ring 0 on Intel processors, which the kernel would have to guard against. Returning with `IRETQ`, as the interrupt path does, has neither problem.
- Register convention: number in `RAX`; arguments in `RDI`, `RSI`, `RDX`, `R10`, `R8`, `R9`; result in `RAX` (non-negative on success, negated status code on failure); a second result in `RDX` for the calls that return two values. `RCX` and `R11` are undefined after a system call: they are reserved for a later move to `SYSCALL`, which overwrites them. All other registers are preserved. `libstill`'s wrapper declares both as clobbered (`lateout("rcx") _, lateout("r11") _`).
- Every pointer argument is a user address and length, checked and copied by `mm::usercopy` (section 3). No call takes a path, PID or device name without a handle that authorises it.
- `abi_version` lets a program refuse to run on a kernel it does not understand. The numbers live in the shared `abi` crate.

| # | Call | Arrives | Notes |
|---|---|---|---|
| 0 | `abi_version()` | M2 | Interface version |
| 1 | `debug_write(ptr, len)` | M2 | At most 256 bytes, else `INVALID_ARGS`. The kernel emits one line `SK:APP:OUT pid=<n> name=<name> \| <text>`; every byte outside 0x20–0x7E, and the backslash, is written as `\xNN` (two upper-case hexadecimal digits). Budget 4 KiB per second per process; excess is dropped and reported once a second as `SK:APP:DROPPED pid=<n> bytes=<n>` |
| 2 | `exit(code)` | M2 | End the calling process |
| 3 | `yield()` | M2 | Give up the time slice |
| 4 | `sleep(nanoseconds)` | M2 | Block for a duration |
| 5 | `clock()` | M2 | Nanoseconds since boot, from the TSC calibrated at boot, or from the tick count when the TSC is not invariant (5.3) |
| 6 | `mem_alloc(len)` → address | M2 | Zeroed read-write pages. At most 1 MiB per call, else `INVALID_ARGS` |
| 7 | `mem_free(addr, len)` | M2 | Return pages |
| 8 | `handle_close(h)` | M2 | Drop a handle |
| 9 | `diag_read(diag, kind, cursor, buf, len)` | M2 | Copy out a snapshot: processes, totals, crashes; from Milestone 3 also grants and the audit log. Requires `INSPECT`. The register tier of a crash record also requires `INSPECT_REGISTERS` (else `ACCESS_DENIED`), a right minted only in debug mode ([04 §6](04-security-and-permissions.md#6-crash-records)) |
| 10 | `handle_duplicate(h, rights)` → handle | M3 | Same object, equal or fewer rights. Requires `DUPLICATE` (else `ACCESS_DENIED`); rights not a subset is `INVALID_ARGS` |
| 11 | `channel_create()` → two handles | M3 | Message pipe. At the handle limit it returns `NO_MEMORY`, like every call that needs a new handle slot |
| 12 | `channel_write(h, bytes, len, items, n)` | M3 | Send bytes and transfer handles. Each item is `{handle, rights}`. The sender's handle must carry `TRANSFER` (else `ACCESS_DENIED`); it leaves the sender and arrives with exactly `rights`, which must be a subset (else `INVALID_ARGS`). An item that is an end of the channel written to, or a channel end with messages waiting, is refused with `INVALID_ARGS` (D-K-23). On any failure nothing moves |
| 13 | `channel_read(h, buf, cap, handles, cap)` | M3 | Receive one message. A call that fails leaves the message queued. A buffer that is too small is `INVALID_ARGS`, with the sizes needed in `RDX` (D-K-24) |
| 14 | `wait(h, signals, deadline)` | M3 | Block until readable, peer closed, process exited (on a `Process` handle), or timeout |
| 15 | `dir_open(dir, name, len, rights)` → handle | M3 | Open a child by name; `name` is a path below `dir`, one or more components separated by `/`. Requires `LOOKUP`; `rights` must be a subset of the rights on the `Dir` handle (else `INVALID_ARGS`); a name the boot filesystem does not hold is `NOT_FOUND`. Only `init` and `permd` hold a `Dir` |
| 16 | `file_read(h, offset, buf, len)` → bytes | M3 | Read through a `File` or `Grant` handle. At most 64 KiB per call, returns the count; requires `READ`; through a `Grant` the grant must be Active, else `REVOKED` |
| 17 | `file_stat(h, out)` | M3 | Size and name. Requires `READ`; through a `Grant` the grant must be Active, else `REVOKED` |
| 18 | `grant_create(target, rights, actor)` → grant, control | M3 | Make a revocable forwarder. `target` must be a `File` (any other type: `WRONG_TYPE`) carrying `GRANT` (else `ACCESS_DENIED`). `rights` is `READ`, which the `File` handle must carry (else `INVALID_ARGS`), optionally with `DUPLICATE`, a right over the new `Grant` object that only the `m3-dup` test policy asks for. The returned `Grant` handle carries `rights` plus `TRANSFER`. The kernel takes the label's path from the `File` object and the action word from the rights, and records as grantee the first process other than the creator whose table the handle enters; at that moment, not inside this call, it writes the `granted` audit entry with the actor given here |
| 19 | `grant_revoke(control, actor)` | M3 | Requires `REVOKE`; the kernel writes the `revoked` audit entry |
| 20 | `process_spawn(image, bootstrap)` → process, PID | M3 | Start a program with one initial channel. Requires `EXECUTE` on a `File` (a `Grant` handle: `WRONG_TYPE`). Returns the `Process` handle in `RAX` and the new PID in `RDX`. Name, class, memory limit and flags come from the image's bootfs table entry |
| 21 | `process_kill(process, actor)` | M3 | Requires `KILL`; the kernel writes the `ended` audit entry |
| 22 | `diag_note(diag, kind, text, len)` | M3 | Append an audit entry. Requires `WRITE` on `Diag` (only `permd`); kinds `asked` and `denied` only |
| 23 | `memory_create(len)` → handle | M4 | Shareable memory |
| 24 | `memory_map(h, flags)` → address | M4 | Requires `MAP` |
| 25 | `input_read(input, buf, cap)` → events | M4 | Key events. Only `desktop` has an `Input` |
| 26 | `wait_many(items, n, deadline)` | M3 | Wait on several handles |
| 27 | `console_read(console, buf, cap)` → bytes | M3 | Operator input. Requires `READ`; only `shell` holds the `Console` |
| 28 | `memory_unmap(addr, len)` | M4 | Remove a mapping made by `memory_map` |
| 29 | `link_read(link, buf, cap)` → bytes | M4b | Bytes received on the model link (5.9). Requires `READ`; only `modeld` holds the `Link` |
| 30 | `link_write(link, buf, len)` → bytes | M4b | Queue bytes for sending and return how many were taken; the queue is bounded. Requires `WRITE` |

Milestone ranges: Milestone 2 = calls 0–9; Milestone 3 = 10–22, 26 and 27; Milestone 4 = 23–25 and 28; its stage 4b = 29 and 30. The three calls that return two values (`channel_create`, `grant_create`, `process_spawn`) put the second in `RDX`.

`actor` is a code from a fixed list in the `abi` crate, not text: `prompt`, `shell`, `workbench`, `init`, `permd`, from stage 4b `desktop` and `assistant`, or a built-in test policy (`policy:<name>`). The kernel copies it into the audit entry it writes and uses `kernel` for entries it originates itself. The audit line format and the full list of kinds are in [04 §8](04-security-and-permissions.md#8-audit-log). For `diag_note`, `text` is a wire structure from the `abi` crate, not a finished line: the subject's PID, the action, the path as the app sent it, an actor code and a reason code. The kernel looks the name up from the PID, cuts and escapes the path, and formats the entry.

Text that comes from user space and reaches the log (`debug_write` text, `diag_note` text, the path in a label) is escaped as in call 1, so nothing an app writes can start a line; inside the quoted `object="..."` field of an `AUDIT` line the double quote is escaped as well. Tests accept `SK:` and `AUDIT` only at column 0. Every ring-3 program, System class included, has only `debug_write`; there is no privileged logging call. The markers that the services print (`SK:PERMD:POLICY ...`, `SK:DESKTOP:PROMPT ...`, `SK:WORKBENCH:PROBE ...`) are therefore the text of an `SK:APP:OUT` line, for example `SK:APP:OUT pid=2 name=permd | SK:PERMD:POLICY name=m3`, and a test accepts one only when the PID on that line is the System-class process of that name according to its `SK:KERNEL:SPAWN` line.

In Milestone 2 the kernel itself starts the first programs from `bootfs` (there is no `process_spawn` yet) and gives the `inspector` its `Diag` handle. From Milestone 3 the kernel starts only `init`. The kernel's first message on `init`'s bootstrap channel carries the boot command line and `init`'s endowment; `init` acts on the `test=` options that name a program to start, forwards `test=` to `permd` and `ui.*` to `desktop`, and no other process can read boot options (section 7.1).

### 5.6 Input

- **Operator console, Milestone 3 (D-K-10).** The `Console` object is the receive side of COM1. The kernel enables the 16550's receive interrupt (interrupt-enable register bit 0, and modem-control bit 3, OUT2, which PC hardware uses to pass the UART's interrupt line) and unmasks IRQ 4, vector `0x24`. The handler moves received bytes into a bounded ring; when the ring is full further bytes are dropped. `console_read` returns the queued bytes and blocks while there are none. There is one `Console` handle, and `init` gives it to `shell` only.
- The interactive shell is used with `cargo xtask run`, which puts the serial port on the terminal. The automated `test m3` uses a scripted policy in `permd` instead, because the test VM's `-serial file:` carries no input; `permd` prints `SK:PERMD:POLICY name=<name>` whenever a test policy is active.
- **Keyboard, Milestone 4.** The kernel's PS/2 handler runs on IRQ 1. It reads the status port first: if bit 0 is clear it returns; otherwise it reads port `0x60` and discards the byte if status bit 5 (auxiliary data) was set, so a mouse byte is never queued as a key. Scan codes go onto the single `Input` object's queue.
- The keyboard controller's state after firmware is not relied on (QEMU's controller starts with translation off and the keyboard in scan-code set 2; firmware changes that only if its keyboard driver ran). At start-up the kernel: (1) sends `0xAD` and `0xA7` to port `0x64` to disable both ports; (2) reads port `0x60` until status bit 0 is clear; (3) reads the configuration byte (command `0x20`), sets bit 0 (IRQ 1) and bit 6 (translation), clears bit 1 (IRQ 12) and bit 4 (keyboard clock on), sets bit 5 (auxiliary clock off), and writes it back (command `0x60`); (4) sends `0xAE` to enable the keyboard port; (5) sends the keyboard `0xF5`, then `0xF0` `0x02` (scan-code set 2, which translation turns into set 1), then `0xF4`, checking for `0xFA` after each byte; (6) empties the output buffer again and unmasks IRQ 1.
- `desktop` holds the only `Input` handle. It turns scan codes into key events, applies the keyboard layout, consumes the keys it reserves (the Still key `F12`; `F11`; `F9`, and `Alt+Tab` where the host passes it through; `Ctrl+F12`; D-UI-3, D-UI-6), and forwards other keys to the focused app over that app's window channel (5.8).
- `Console` and `Input` handles signal *readable* to `wait` and `wait_many`.
- USB keyboards need a USB host-controller driver and are out of scope until after the demo. This matters for hardware ([08](08-hardware-and-dual-boot.md)).

### 5.7 Graphics

- The loader passes the framebuffer's physical address, size, resolution, stride and pixel format in `BootInfo`.
- The kernel's `fbcon` draws the log on it from Milestone 2, using the Spleen 8×16 glyph table compiled into the kernel (the font's licence file ships in the image from the same milestone), so real hardware without a serial port still shows output.
- At Milestone 4 the kernel wraps the framebuffer range in a `Memory` object and gives the only handle to `desktop` (through `init`). When `desktop` maps it, `fbcon` stops drawing. On a kernel panic the kernel takes the framebuffer back and draws the panic banner.
- Apps draw into surfaces and send "present this rectangle" messages. They never touch the framebuffer. `desktop` creates every surface `Memory` object at the size it chose and sends the app a handle with `MAP`, `READ` and `WRITE`, and without `DUPLICATE` or `TRANSFER`. Apps never supply memory to `desktop`, so `desktop` never has to trust a size an app claims.

### 5.8 User-space message sets

These are conventions between user programs, carried by `channel_write` and `channel_read`. The kernel does not interpret them. Their encodings live in the `abi` crate and `libstill`, and every message fits the channel limits in 5.2. A receiver identifies its peer by the channel a message arrives on, never by anything the message says. Which process holds which end is set up by `init` (section 7.1).

| Channel | Ends | From | Messages |
|---|---|---|---|
| Bootstrap (called the control channel when the far end is a service) | `init` ↔ each process it starts | M3 | `init` → process: the handles and options in the process's launch-table row. `init` → `permd` and `init` → `desktop`: *new client* {PID, registry id}, with the server end of that app's request or window channel. `desktop` → `init`: *launch* {registry id} (Milestone 4) |
| Request | app ↔ `permd` | M3 | app → `permd`: *request* {action, path}. It carries no authority. `permd` → app: *reply* {status}, with the `Grant` handle on success; the handle arrives with `READ` only. `ACCESS_DENIED` covers both "no" and "no such file" (D-S-2). An app has at most one request waiting ([04 §5](04-security-and-permissions.md#5-the-trusted-prompt-path)) |
| Prompt | `permd` ↔ `desktop` (↔ `shell` in Milestone 3) | M3 | `permd` → `desktop`: *ask* {request id, PID, registry id, canonical path, action}; *cancel* {request id} when the requester has exited, crashed or been ended; *result* {request id, granted or no such file} after an answer. `desktop` → `permd`: *answer* {request id, allow or deny} |
| Window | app ↔ `desktop` | M4 | app → `desktop`: *create surface*; *present* {x, y, width, height}. `desktop` → app: *surface* {width, height, stride, pixel format}, with the `Memory` handle; *key event* {key, modifiers, down or up}, to the focused app only |
| Grant admin | Workbench ↔ `permd` (`shell` ↔ `permd` in Milestone 3) | M3 | Three messages only: *list grants*; *list waiting requests* (id, app, PID, path, action, time asked; read-only); *revoke grant* {grant id} |
| Process control | Workbench → `init` | M4 | One message: *end process* {PID}, honoured only for class App. `init` calls `process_kill` with actor `workbench` |

In Milestone 3, before `desktop` and the Workbench exist, `shell` holds the deciding end of the prompt channel and the grant-admin channel. Its `grant`, `deny`, `revoke` and `grants` commands are these same messages, and the audit actor is `shell`. From Milestone 4 those two ends belong to `desktop` and the Workbench.

### 5.9 Assistant (stage 4b)

What stage 4b adds to the interfaces above; the reasons are in [04 §11](04-security-and-permissions.md#11-delegation-to-an-assistant-stage-4b).

- **The `Link` object (D-A-5).** A second 16550: COM2 at ports `0x2F8`–`0x2FF`, IRQ 3 at vector `0x23`, with receive and transmit interrupts and a bounded ring in each direction (assumption A-23). There is one handle, with `READ` and `WRITE`, and `init` gives it to `modeld`. It signals *readable* to `wait`. Calls 29 and 30 use it. The kernel does not interpret the bytes.
- **A `Grant` over a `Channel` end (D-A-3).** `grant_create` also accepts a `Channel` end that carries `GRANT`, with the rights `READ` and `WRITE`. `channel_create` puts `GRANT` on both ends only when a System-class process calls it, so an app's call of `grant_create` on a channel of its own fails with `ACCESS_DENIED` (the Milestone 3 probe, which today expects `WRONG_TYPE`, changes with it). Through such a `Grant` handle `channel_write`, `channel_read`, `wait` and `wait_many` first check that the grant is Active (else `REVOKED`) and then act on the channel; a write through it carries no handles. The kernel counts the messages that pass (`uses`, given by `diag_read` with the grant). The label, a target and an action as codes from the `abi` crate, comes from the creator, which is `permd`, never from the grantee.
- **Injected keys (D-A-8).** The control service in `desktop` takes *key* messages from a Type delegation and sends the app a *key event* with the flag `injected`. It does not put them through its own key routing, refuses the reserved keys, and drops them while a sheet is open.
- **Outlines (D-A-4).** The `ui` library sends `desktop` an *outline* message when the controls of a surface change: for each control its kind, label, value and whether it has focus, at most 4 KiB in all. `desktop` keeps the newest outline of each app and returns it to a See delegation.

| Channel | Ends | Messages |
|---|---|---|
| Control | assistant ↔ `desktop`, through a `Grant` | One channel for each delegation {app, right}. assistant → `desktop`: *start*; *outline?*; *key* {key, modifiers, down or up}; *end*. `desktop` → assistant: *reply* {done, dropped, not allowed, not running}, and the text for *outline?* |
| Model | assistant ↔ `modeld` | assistant → `modeld`: *turn* {conversation, tools held}. `modeld` → assistant: *reply* {text or one tool call}; *no model* |
| Request (added) | assistant ↔ `permd` | A *request* may carry up to 8 lines {right, target}. The *reply* has a status and, for each line granted, a `Grant` handle |
| Prompt (added) | `permd` ↔ `desktop` | *ask* carries the lines and whether the requester is Outside; *answer* carries allow or deny for each line. `permd` → `desktop`: *bind* {request id, line}, answered with the control channel end to wrap. `desktop` → `permd`: *revoke all delegations* (hands-off) |
| Control (added) | `desktop` → `init` | *end process* {PID}, honoured only for class App; `init` calls `process_kill` with the actor `assistant` |

New programs: `modeld` (System; `init` sends it the `Link` and, for each Outside program, the server end of that program's model channel; `init` restarts it as it restarts `permd`), `assistant` (App, declared Outside in the registry; it gets a request channel, a window channel and its end of a model channel, and nothing else), and the sample `assist-probe`, which `init` starts only under `test=m4-assist-hostile`.

## 6. Filesystem and persistence

**Milestones 2–4 use a read-only image held in memory.** `xtask` packs the `bootfs/` directory and the user programs into `BOOTFS.IMG` (a flat format of our own: header, table of path, offset, length, SHA-256, name, display name, class, memory limit and flags, then page-aligned data). The class is App or System. The memory limit is in MiB, and 0 means the class default of 16 MiB. There are two flags: `critical` (the kernel stops with "system service lost" if the process ends; set for `init` and `desktop`) and `display` (the limit is the larger of the entry's value and 8 × the framebuffer size; set for `desktop`, whose entry says 64 MiB). These fields are written by `xtask` and nothing else; the kernel takes them from the image's entry, so no caller of `process_spawn` can choose them (5.5, call 20), and the kernel never matches a program by name. The loader reads the image from the EFI System Partition and passes its location in `BootInfo`. The kernel exposes it as `Dir` and `File` objects. Nothing in Stillkey can write to any disk in these milestones, which is also a safety property for hardware testing.

**Writable storage becomes necessary when any one of these is wanted:**

| Need | Why memory-only is not enough |
|---|---|
| Remembering grants or denials across restarts | Decisions are lost at power-off |
| Apps that save documents, and therefore `Write` requests in the prompt | Nothing to write to |
| Crash records and the audit log surviving the crash that reboots the machine | The ring lives in RAM |
| Settings such as reduced motion and contrast | Set by boot options; a change made at run time is lost at power-off |
| Installing or updating apps without rebuilding the image | The image is read-only |

The planned order when that time comes: a block-device driver for the VM (virtio-blk) as a user-space process, an append-only log store for audit and settings, then a real filesystem behind a file service, then drivers for physical disks. Each step is its own milestone after the demo. Before any code writes to a physical disk, the hardware plan in [08](08-hardware-and-dual-boot.md) must name the exact partition it may touch.

## 7. User-space pieces we write

| Piece | Role |
|---|---|
| `abi` | Shared `#![no_std]` crate: system-call numbers, status codes, rights, actor codes, wire structures. Used by the kernel and by user space |
| `libstill` | User runtime: entry point, system-call wrappers, owned handle types that close on drop, allocator, formatting and logging, channel message encoding |
| `init` | First process. Receives the boot command line and its handles from the kernel, starts every other process, hands each exactly the handles in its row of the launch table, restarts `permd` if it ends (7.1) |
| `permd` | Permission broker ([04](04-security-and-permissions.md)) |
| `inspector` | Text inspector (Milestone 2 only): prints the process table and the crash list from `Diag`. It becomes the shell's `ps` in Milestone 3 |
| `shell` | Serial operator shell (Milestone 3): `ps`, `grants`, `grant`, `deny`, `revoke`, `audit`, typed on the `Console` |
| `ui` | Drawing primitives, text, widgets, theme tokens ([05](05-ui-design.md)) |
| `desktop` | Compositor, launcher, permission prompt, reserved keys |
| `workbench` | Graphical inspector |
| `modeld` | Stage 4b: holds the model link and passes the assistant's exchanges over it (5.9) |
| `assistant` | Stage 4b: the desktop assistant, an App that acts through delegations ([04 §11](04-security-and-permissions.md#11-delegation-to-an-assistant-stage-4b)) |
| `samples/*` | `ticker`, `faulter`, `prober` (Milestone 2), `notes` (Milestone 3), `imposter` (Milestone 4), `assist-probe` (stage 4b) |

User programs are built for `x86_64-unknown-none` (soft-float, no red zone) with `-C relocation-model=static` and the shared linker script `user/user.ld`: base `0x40_0000`, 4 KiB-aligned text, read-only data and data segments, explicit program headers. Each binary crate passes the script from its own `build.rs`. The result is a static `ET_EXEC` that the validator's user policy accepts (5.4).

### 7.1 Launch topology

**Milestone 2.** The kernel starts the programs itself. All sample programs are in the one boot filesystem image; a start table built into the Milestone 2 kernel lists the image paths of each scenario, and the boot option `test=<scenario>` selects the row (`m2` when there is none). The programs have no handles, except `inspector`, which the table marks to receive the `Diag` handle with `INSPECT`. The table is removed in Milestone 3, when `init` takes over and starts these scenarios as well (D-K-20).

**From Milestone 3** the kernel starts only `init` (PID 1), and `init` spawns every other process.

- `init` starts with one handle, its bootstrap channel, whose far end the kernel holds. The kernel's first message on it carries the boot command line and `init`'s endowment: the `bootfs` root `Dir` (rights `LOOKUP`, `READ`, `EXECUTE`, `GRANT`, `DUPLICATE` and `TRANSFER`), the full `Diag`, the `Console`, and from Milestone 4 the framebuffer `Memory` and the `Input`. `init` acts on the `test=` options that name a program to start, and forwards `test=` to `permd` and `ui.*` to `desktop`. No other process can read boot options.
- Class comes from the image's bootfs table entry (section 6): `init`, `permd`, `desktop`, `workbench` and `shell` are System; everything else is App.
- Under a `test=` option `init` does what the scenario of that name in [06](06-milestones.md) needs, and nothing else changes: it starts that scenario's sample programs (the `prober` binaries for `m3-prober`, `imposter` for `m4-imposter`), passes `notes` its self-test flag and the Workbench its probe switch (`m4-wbprobe`) in their launch-table rows, and ends `permd` (`m3-brokerkill`) or `desktop` (`m4-desktopkill`) with `process_kill` and the actor `init`.
- Only `init` and `permd` hold `Dir` handles ([04 §2](04-security-and-permissions.md#2-invariants), invariant I5). `init` uses its root only to open program images (`EXECUTE`) and to endow `permd`; it never creates grants. `init` also mints every `Diag` handle below from the full one it received.

The launch table. Each process is started with exactly one handle (`handles=1`), its end of a bootstrap channel whose far end stays with `init`; the rest arrives over that channel.

| Process | Class | From | What `init` sends it |
|---|---|---|---|
| `permd` (started first, so PID 2) | System | M3 | A root `Dir` with `LOOKUP`, `READ` and `GRANT` only (no `EXECUTE`, so `permd` cannot start a program); `Diag` with `WRITE`; its ends of the prompt and grant-admin channels; the `test=` option; then, for each app, the server end of the app's request channel with {PID, registry id} |
| `shell` | System | M3 only | The `Console`; `Diag` with `INSPECT`; the deciding end of the prompt channel and the grant-admin channel (5.8). It is not started from Milestone 4, when `desktop` and the Workbench hold those two ends |
| `desktop` | System | M4 | The framebuffer `Memory` and the `Input`; `Diag` with `INSPECT`; the deciding end of the prompt channel; the `ui.*` options and whether `debug=registers` was given (for the top bar's debug marker); then, for each app, the server end of the app's window channel with {PID, registry id} |
| `workbench` | System | M4 | `Diag` with `INSPECT`, and with `INSPECT_REGISTERS` only when the boot option `debug=registers` is present (debug mode, [04 §6](04-security-and-permissions.md#6-crash-records)); the grant-admin channel to `permd`; a process-control channel to `init`. These three are in addition to what every windowed program has (bootstrap channel, window channel, surface) |
| An app | App | M3 | Its end of a request channel to `permd`; from Milestone 4 its end of a window channel to `desktop`. Nothing else |

Starting an app:

1. In Milestone 4 `desktop` asks for it with *launch* {registry id} on its control channel; the registry id names the app's entry in the launcher registry, which is part of the read-only image. `init` also starts the programs its launch table or a `test=` option names: the Milestone 3 samples, and `imposter`, which is not in the launcher and is started by `test=m4-imposter`.
2. `init` opens the image through its root `Dir` with `EXECUTE`, creates the bootstrap channel and calls `process_spawn(image, bootstrap)`. The call returns the `Process` handle and the new PID (5.5, call 20), so `init` needs no other source for the PID.
3. `init` creates a request channel, sends the app's end to the app over the bootstrap channel and the server end to `permd` on `permd`'s control channel, with {PID, registry id}. From Milestone 4 it does the same with a window channel and `desktop`.
4. `permd` and `desktop` now know which app a message comes from by the channel it arrives on. `permd` and `desktop` talk to each other over the one prompt channel `init` created.

When a process ends:

- **An app**: its handles close. `permd` and `desktop` see `PEER_CLOSED` on its channels, and its grants become Released ([04 §4.1](04-security-and-permissions.md#41-behaviour-case-by-case)).
- **`permd`**: its grants are revoked (`AUDIT ... kind=revoked ... actor=kernel reason=broker-exit`), client channels report `PEER_CLOSED`, and waiting requests end as denied. `init`, which waits on `permd`'s `Process` handle, restarts it and sends fresh channel ends to every running app, to `desktop` and to the Workbench (in Milestone 3, to `shell`) over the bootstrap and control channels it kept.
- **`init` or `desktop`**: the kernel stops with the panic screen "system service lost" (fail-stop; status 37 under test). The panic path already takes the framebuffer back. The kernel recognises the two by the `critical` flag in their bootfs table entries (section 6), never by name.

## 8. Deferred on purpose

Multiple CPUs; kernel pre-emption; demand paging, swap and copy-on-write; shared libraries and dynamic linking; hardware floating point and SIMD in user space; `SYSCALL`/`SYSRET`; ACPI beyond static tables (MADT, FADT, HPET), with no AML interpreter; power management; USB; networking (stage 4b reaches a model over one serial link and no further, 5.9); GPU acceleration; POSIX or any third-party app compatibility; address-space randomisation; a wall-clock time source (D-UI-7: every time shown is time since boot); resource accounting beyond the memory limit and the fixed limits in 5.2.
