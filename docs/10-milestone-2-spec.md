# 10 · Milestone 2 specification: run and observe

Status: **implemented; all three checkpoints** (sections 11.2 to 11.4). The acceptance commands of section 10 pass, and the milestone was accepted on 2026-11-11. Written on 2026-11-10 from [03](03-kernel-and-interfaces.md), [04](04-security-and-permissions.md) and the Milestone 2 section of [06](06-milestones.md). It has had the author's own check against those documents (section 13). The independent review of section 11, step 0 took place on 2026-11-11, after the implementation instead of before it; what it found and what was done is in section 11.5. Choices this document adds are marked **D-K-11 … D-K-19**, **D-W-7** and **D-W-8** and are listed in [01 §3](01-overview-and-architecture.md#3-design-decisions).

**Revision, 2026-11-10 (after a review).** Five changes: the GDT has seven slots (section 7, step 9); `bad-elf` is refused with the validator's existing reason `wx`; page permissions are tested as the CPU applies them, not as single table entries hold them (section 8.6); a process's memory is freed only after the CPU has left its page tables (section 8.3); and pre-emption is proved by a program that never yields (`spinner`, test `m2-preempt`). The work is split into checkpoints (section 11.1).

> **For whoever implements this:** work the steps of section 11 in order, test first. Each step names its files, the interfaces it consumes and produces, its tests and the command that shows it done. Never run two `cargo xtask test` commands at once: run directories are keyed by test name.

## 1. Outcome

One command passes: `cargo xtask test m2`. It boots the Milestone 2 image and shows that

- the kernel runs on its own tables, stacks and interrupt handling, and its text is read-only and its data non-executable (`m2-kwrite`, `m2-kexec`);
- three programs run in ring 3, pre-empted by a 100 Hz timer, and one of them crashes without disturbing the others (`m2`);
- a text inspector, holding the only handle in the system, prints process and crash data that agrees with the kernel's own log lines and with the ELF files on the host (`m2`);
- a program that never makes a system call cannot keep the others from running (`m2-preempt`);
- seventeen small probes fail in exactly the stated way: memory isolation, page permissions, privileged and vector instructions, the memory limit, handle guessing, bad pointers, forged log lines (`m2-prober`);
- the kernel log, including a panic banner, is readable on the framebuffer (`m2-screen`).

`cargo xtask test all` runs `m1` and then `m2`. Everything stays in the VM.

**Goal in one sentence:** processes that cannot hurt each other or the kernel, and a truthful view of them.

**Architecture in three sentences.** Logic that needs no privileged instruction is written as pure `no_std` library crates and tested on the host, as Milestone 1 did with `stillkey-bootinfo` (D-K-12): the system-call ABI (`stillkey-abi`), the boot filesystem format (`sk-bootfs`) and kernel logic (`sk-kcore`: descriptor encoding, fault policy, frame bitmap, page-table editing over a memory trait, heap, scheduler queues, handle table, log escaping and rate limit, crash ring, console). The kernel binary holds only what must touch the CPU or raw memory, in the module tree of [03 §2](03-kernel-and-interfaces.md#2-subsystems-and-their-boundaries). User programs are static ELF files packed by `xtask` into `BOOTFS.IMG`, which the loader reads and the kernel starts from a table selected by the boot option `test=`.

## 2. Decisions this specification adds

| ID | Decision | Alternative | Cost if changed later |
|---|---|---|---|
| D-K-11 | **`stop=m1`.** With this boot option the kernel ends after the Milestone 1 steps exactly as it does today (`SK:KERNEL:M1_OK`, status 33 under `qemu_exit=on`). Without it the kernel continues into Milestone 2. Every Milestone 1 boot test and `m1-screen` adds the option, and their images carry no `BOOTFS.IMG`, so those tests keep proving the handoff alone and their screen stays the plain band | Let the harness stop the VM at `M1_OK` and drop the status-33 check | The six `m1` expectation files, the `m1-screen` command line and one branch in the kernel |
| D-K-12 | **Pure logic in host-tested crates** `lib/abi`, `lib/sk-bootfs`, `lib/sk-kcore`. `sk-kcore` denies `unsafe` except in its `heap` module | Everything in the kernel binary, tested only by booting | Moving modules |
| D-K-13 | **The kernel does not hash program images** in Milestones 2–4. It takes each program's SHA-256 from the boot filesystem table for crash records. The loader hashes the whole `BOOTFS.IMG` and logs it; the harness compares that with the host's digest, and Milestone 5 puts it under the signed manifest | Add `sha2` to the kernel and verify each image at start | About 30 lines and one dependency in the kernel |
| D-K-14 | **User address layout.** Image inside `0x40_0000` … `0x8000_0000`; `mem_alloc` areas from `0x1_0000_0000` upward, lowest free address first; stack of 64 KiB ending at `0x7FFF_FFFF_F000` with nothing mapped below it. Kernel thread stacks are 32 KiB, one per 64 KiB slot of the stack window from `0xFFFF_D000_0010_0000`, so each has unmapped pages below it | Other bases or sizes | Constants in `stillkey-abi` and `sk-kcore` |
| D-K-15 | **An unknown system-call number returns `INVALID_ARGS`.** No tenth status code | A `NOT_SUPPORTED` code | One constant |
| D-K-16 | **`diag_read` copies fixed-size records** (section 8.4), at most 4096 bytes per call, with a cursor for the next call. A change counter in the totals record tells a reader when to read again | Variable-length text | The record layouts in `stillkey-abi` |
| D-K-17 | **A process that has ended stays in the process table** for the rest of the boot, so that the inspector can show it. At most 64 processes are started per boot in Milestone 2; a 65th start is refused (`SK:KERNEL:SPAWN_REFUSED … reason=table`). The kernel stack of an ended process stays with its record (at most 64 × 32 KiB in all), so ending a process frees exactly the pages its counters show | Remove ended processes after a while | A constant |
| D-K-18 | **Handles are announced by convention in Milestone 2.** A program's first handle is always `0x1000` (slot 0, generation 1; `stillkey_abi::FIRST_HANDLE`). The inspector's `Diag` handle is that value. From Milestone 3 handles arrive on the bootstrap channel and the convention is unused | Pass the value in a register at entry | One constant |
| D-K-19 | **Freed frames are filled with the byte `0xFA`**, and a frame is zeroed when it is handed out. A page table that is used after it was freed then stops the boot at once (section 8.3) | No fill: faster, and such a defect can stay hidden | One loop in `mm::phys` |
| D-W-7 | **A scenario run ends when the harness has seen every line it requires.** It follows `serial.log`, then tells QEMU to quit through the monitor; the expected status is 0. A forbidden line or the time limit fails the run ([06](06-milestones.md), "How a run ends") | The guest ends the VM | The `follow` function in `xtask` |

These additions go beyond the criteria of [06](06-milestones.md) and are named there now: the probe `edge` and the refused image `bad-elf` (section 9); the tests `m2-unit`, `m2-repro`, `m2-screen` and `m2-badfs` (section 10); the kernel lines `SK:KERNEL:EXIT`, `SK:KERNEL:REAPED`, `SK:KERNEL:TIMER_OK` and `SK:KERNEL:SCHED_OK`; and the inspector's two refusal checks (`registers=`, `rotext=`). The revision of 2026-11-10 adds the programs `hello` and `spinner`, the probes `wtext`, `xstack` and `xheap`, the tests `m2-hello` and `m2-preempt`, the line `SK:KERNEL:PAGING_AUDIT` and the field `error=` at the end of the `CRASH` line.

## 3. Files this milestone creates or changes

```
lib/abi/                    stillkey-abi: numbers, status codes, rights, limits, diag records   (new)
  src/lib.rs  src/diag.rs  tests/abi.rs
lib/sk-bootfs/              BOOTFS.IMG format: parse (no_std) and build (feature "std")          (new)
  src/lib.rs  src/build.rs  tests/format.rs
lib/sk-kcore/               kernel logic without privileged instructions                          (new)
  src/lib.rs
  src/x86.rs                GDT, TSS and IDT encodings; TrapFrame; page-fault error bits
  src/fault.rs              who a fault is charged to
  src/frames.rs             frame bitmap
  src/paging.rs             4-level tables over the PhysMem trait
  src/usercopy.rs           range check and page walk for user buffers
  src/heap.rs               the allocator behind #[global_allocator] (the one module with unsafe)
  src/sched.rs              run queue, sleep queue, thread states
  src/handles.rs            per-process handle table
  src/logfmt.rs             \xNN escaping and the 4 KiB per second budget
  src/crash.rs              crash record and the ring of 32
  src/account.rs            per-process page counters and the memory limit
  src/console.rs            text grid on a framebuffer slice
  src/font.rs               Spleen 8x16 glyphs 0x20-0x7E, generated by `cargo xtask font`
  tests/*.rs                one file per module
lib/sk-elf/src/lib.rs       + `user_policy(max_mem)`                                              (changed)
boot/bootinfo/src/validate.rs   + rows `kernel`, `stack`, `bootfs`                                (changed)
boot/loader/src/files.rs, main.rs   + read, hash and hand over BOOTFS.IMG                         (changed)
kernel/linker.ld            + symbols __text_start/_end, __rodata_start/_end, __data_start, __bss_end
kernel/src/
  main.rs                   Milestone 1 steps, then `stop=m1` or the Milestone 2 sequence
  arch/x86_64/{mod,gdt,idt,stubs,cpu,pic,pit,apic,switch,user}.rs
  mm/{mod,phys,virt,heap,usercopy}.rs
  sched/mod.rs   proc/{mod,load,start}.rs   object/mod.rs   diag/mod.rs
  fs/bootfs.rs   syscall/mod.rs   drivers/fbcon.rs   selftest.rs
  log.rs, panic.rs          log to serial and console; exception banner                           (changed)
user/user.ld                base 0x40_0000; text, rodata, data segments, 4 KiB aligned             (new)
user/libstill/              entry, system-call wrappers, `out!`, panic handler                     (new)
user/inspector/             text inspector                                                         (new)
user/samples/{hello,ticker,faulter,spinner}/   one binary each                                                   (new)
user/samples/prober/        one package, one binary per probe (src/bin/<probe>.rs)                 (new)
bootfs/MANIFEST             what goes into BOOTFS.IMG                                              (new)
assets/spleen-8x16.bdf, assets/spleen-LICENSE                                                     (new)
tools/fetch-font.sh         one-time download of Spleen 2.2.0, pinned by SHA-256                   (new)
xtask/src/{bootfs,follow,font,m2}.rs   packer, log follower, glyph generator, Milestone 2 tests    (new)
xtask/src/{build,image,qemu,tests,main}.rs                                                         (changed)
tests/m2.expect, m2-prober.expect, m2-kwrite.expect, m2-kexec.expect, m2-badfs.expect             (new)
tests/m1-*.expect           the command-line lines gain `stop=m1`                                  (changed)
```

`user/init`, `permd`, `shell`, `ui`, `desktop`, `workbench`, `notes` and `imposter` are not created. `libstill` has no allocator and no channel code yet; no Milestone 2 program needs them.

## 4. Pinned configuration

- **Workspace members added:** `lib/abi`, `lib/sk-bootfs`, `lib/sk-kcore`, `user/libstill`, `user/inspector`, `user/samples/hello`, `user/samples/ticker`, `user/samples/faulter`, `user/samples/spinner`, `user/samples/prober`. Workspace dependencies added: `stillkey-abi`, `sk-bootfs`, `sk-kcore`, `libstill`, each by path. No new third-party crate. `xtask` gains `sk-bootfs` (feature `std`), `sk-kcore`, `sk-elf` and `stillkey-abi`.
- **User programs** are built for `x86_64-unknown-none` with the flags that target already has in `.cargo/config.toml`. Each binary crate's `build.rs` passes `-T<workspace>/user/user.ld`. `user.ld` has three `PT_LOAD` headers with flags 5, 4 and 6, as `kernel/linker.ld` has, and starts at `0x40_0000`.
- **One Cargo call per target**, as now: `xtask` builds the kernel and all user packages in one `cargo build --target x86_64-unknown-none -p stillkey-kernel -p inspector -p hello -p ticker -p faulter -p spinner -p prober`.
- **The kernel gains a heap**, so it may use `alloc`. It still has no SSE, no x87 and no red zone.
- **Constants** (all in `stillkey-abi` unless a crate is named):

| Name | Value |
|---|---|
| `ABI_VERSION` | 2 (the interface of this milestone: calls 0–9) |
| `SYS_ABI_VERSION` … `SYS_DIAG_READ` | 0 … 9, in the order of [03 §5.5](03-kernel-and-interfaces.md#55-system-calls) |
| `Status` | `OK` 0, `BAD_HANDLE` 1, `WRONG_TYPE` 2, `ACCESS_DENIED` 3, `REVOKED` 4, `NOT_FOUND` 5, `INVALID_ARGS` 6, `NO_MEMORY` 7, `PEER_CLOSED` 8, `TIMED_OUT` 9. A failing call returns the negated code in `RAX` |
| Rights | `READ` bit 0, `WRITE` 1, `MAP` 2, `LOOKUP` 3, `GRANT` 4, `EXECUTE` 5, `DUPLICATE` 6, `TRANSFER` 7, `INSPECT` 8, `INSPECT_REGISTERS` 9, `REVOKE` 10, `KILL` 11 |
| `FIRST_HANDLE` | `0x1000` |
| `DEBUG_WRITE_MAX` | 256 bytes |
| `DEBUG_BUDGET_PER_SECOND` | 4096 bytes |
| `MEM_ALLOC_MAX` | 1 MiB |
| `DIAG_READ_MAX` | 4096 bytes |
| `USER_IMAGE_FIRST`, `USER_IMAGE_END` | `0x40_0000`, `0x8000_0000` |
| `USER_HEAP_BASE` | `0x1_0000_0000` |
| `USER_STACK_TOP`, `USER_STACK_SIZE` | `0x7FFF_FFFF_F000`, 64 KiB |
| `DEFAULT_MEM_LIMIT` | 16 MiB (D-K-6) |
| `MAX_HANDLES_APP`, `MAX_HANDLES_SYSTEM` | 256, 4096 |
| `sk_kcore::MAX_PROCESSES` | 64 |
| `sk_kcore::CRASH_RING` | 32 |
| `sk_kcore::KSTACK_SIZE`, `KSTACK_SLOT`, `KSTACK_BASE` | 32 KiB, 64 KiB, `0xFFFF_D000_0010_0000` |
| `sk_kcore::HEAP_BASE`, `MMIO_BASE` | `0xFFFF_C000_0000_0000`, `0xFFFF_F000_0000_0000` |
| `sk_bootfs::MAX_ENTRIES`, `MAX_IMAGE` | 64, 32 MiB |
| Tick | 100 Hz from the 8254, divisor 11932 ([03 §5.3](03-kernel-and-interfaces.md#53-scheduling)); time slice one tick |

## 5. Build, `BOOTFS.IMG` and the disk image

### 5.1 Format of `BOOTFS.IMG` (version 1)

Little-endian, no implicit padding. `sk-bootfs` encodes and decodes every field by value; it forbids `unsafe`.

| Part | Layout |
|---|---|
| Header, 64 bytes | `magic` u64 = `b"SKBOOTFS"`; `version` u32 = 1; `header_size` u32 = 64; `entry_count` u32; `entry_size` u32 = 192; `total_size` u64; `table_offset` u64 = 64; `data_offset` u64; 16 zero bytes |
| Table, `entry_count` × 192 bytes | `path` [u8; 64]; `offset` u64; `length` u64; `sha256` [u8; 32]; `name` [u8; 16]; `display_name` [u8; 32]; `class` u32 (1 App, 2 System); `mem_limit_mib` u32 (0 = 16); `flags` u32 (bit 0 `critical`, bit 1 `display`); `kind` u32 (1 program, 2 file); 16 zero bytes |
| Data | Each file at its `offset`, which is a multiple of 4096; the gap to the next file is zero |

`sk_bootfs::parse(image: &[u8]) -> Result<Image<'_>, Reject>` accepts an image only if, in this order: it is at least 64 bytes and the magic matches (`magic`); version, header size and entry size are the values above (`version`); `total_size` equals the slice length and is at most 32 MiB (`size`); `entry_count` is 1–64 and the table lies inside the image and before `data_offset`, which is a multiple of 4096 (`table`); for every entry: `path` is 1–63 bytes of `[a-z0-9._/-]` followed only by zeros, with no empty component, and `name` is 1–15 bytes of `[a-z0-9-]` followed only by zeros (`name`); `display_name` is zero-terminated printable ASCII (`name`); `class` is 1 or 2, `kind` is 1 or 2, no unknown flag bit, and the reserved bytes are zero (`field`); `offset` is a multiple of 4096 and at least `data_offset`, `length` is at least 1, and `offset + length` does not overflow or pass `total_size` (`range`); entries are sorted by `path` with no duplicate, their data ranges do not overlap, and no two programs share a `name` (`order`). `Image::entries()` yields `Entry` values; `Image::find(path)` and `Image::data(&Entry) -> &[u8]` read them.

`sk_bootfs::build::pack(files: &[Source]) -> Vec<u8>` (feature `std`) sorts by path, computes the digests and writes the image. Its output for the same input is byte-identical.

### 5.2 `bootfs/MANIFEST`

One line per entry, fields separated by `|`, `#` starts a comment: `path | source | name | display name | class | mem_mib | flags`. `source` is `program:<binary name>` or `file:<path from the workspace root>`. Milestone 2 content: `bin/hello`, `bin/ticker`, `bin/faulter`, `bin/spinner`, `bin/inspector` (class System), `bin/prober-<probe>` for each probe of section 9, `bin/bad-elf` (`program:ticker`, marked `corrupt:wx`: `xtask` sets the write bit in the flags of its text segment, as `m1-reject` does for the kernel, so that `sk-elf` refuses it with its existing reason `wx`), and `licenses/spleen.txt` (`file:assets/spleen-LICENSE`, kind 2). Every program is class App with `mem_mib` 0 and no flags, except `inspector`.

### 5.3 Commands

- `cargo xtask build` also builds the user programs and writes `BOOTFS.IMG` next to the two binaries (`build/artifacts/<profile>/`).
- `cargo xtask image` copies `BOOTFS.IMG` to `\EFI\STILLKEY\BOOTFS.IMG`. The Milestone 1 tests ask for an image without it.
- `cargo xtask font` regenerates `lib/sk-kcore/src/font.rs` from `assets/spleen-8x16.bdf`. The generated file is committed; `m2-unit` fails if regenerating changes it.
- `tools/fetch-font.sh` downloads the two Spleen files once, checks their SHA-256 against values written in the script, and stops on a mismatch. **It needs the network and is run only on purpose** (assumption A-21). Nothing else in this milestone downloads anything.

### 5.4 Loader changes ([02 §12](02-boot-and-handoff.md#12-what-changes-after-milestone-1))

After the kernel file, the loader opens `\EFI\STILLKEY\BOOTFS.IMG`. Absent: `SK:LOADER:BOOTFS none`, flag clear. Present: a size of 0 or more than 32 MiB stops with `SK:LOADER:BOOTFS_REJECTED reason=size` (status 35). Otherwise it allocates whole pages, reads the file, and logs `SK:LOADER:BOOTFS_FILE size=<n>` and `SK:LOADER:BOOTFS_SHA256 <hex>`. It sets `BootInfo.bootfs` (base, exact size), `BOOTFS_PRESENT`, and a reserved range of purpose 5. The loader does not parse the image.

`stillkey_bootinfo::validate` gains three rows, checked after the framebuffer row, because Milestone 2 is the first code that uses these records:

| Record | Check | Reason |
|---|---|---|
| Kernel image | `virt_base` is the kernel window; `phys_base` is a page multiple; `size` is not zero; exactly one reserved range has purpose 1 and it is `phys_base` … `phys_base + size`, rounded up to a page; `entry` lies inside `virt_base` … `virt_base + size` | `kernel` |
| Boot stack | `size` is 64 KiB, `virt_top` is the stack window + 4096 + 64 KiB, and exactly one reserved range has purpose 2 and it is `phys_base` … `phys_base + size` | `stack` |
| Boot filesystem | Only with `BOOTFS_PRESENT`: `base` is page-aligned and not zero, `size` is 1 … 32 MiB, and exactly one reserved range has purpose 5 and it is `base` … `base + size` rounded up to a page. Without the flag no range may have purpose 5 | `bootfs` |

## 6. VM launch and how a run ends

The QEMU command is that of [07 §6](07-milestone-1-spec.md#6-vm-launch-command-and-exit-codes). Three ways to end:

| Run | Boot options | Ends by | Expected status |
|---|---|---|---|
| Milestone 1 tests | `… stop=m1` | The guest, as before | 33, 35 or 37 as before; 0 for `m1-screen` |
| `m2-kwrite`, `m2-kexec`, `m2-badfs` | `qemu_exit=on test=kwrite_text` / `test=kexec_data` / `test=m2` | The guest's panic | 37 |
| `m2-hello`, `m2`, `m2-preempt`, `m2-prober`, `m2-screen` | `test=<scenario>`; `test=kwrite_text` for the screen | The harness (D-W-7) | 0 |

`xtask/src/follow.rs`: `follow(vm: &Vm, step: &mut dyn FnMut(&[String]) -> Verdict, at_end: &str) -> Result<i32, String>` starts QEMU with the monitor on standard input, reads `serial.log` every 50 ms, passes the `SK:` lines seen so far to `step`, and when `step` returns `Done` or `Failed(reason)` writes `at_end` and `quit` to the monitor. `capture_screen` becomes a caller of it. `Verdict` is `Waiting`, `Done` or `Failed(String)`.

**Scenario selection in the kernel.** The first word of the command line that is `test=m2`, `test=m2-hello`, `test=m2-preempt`, `test=m2-prober`, `test=kwrite_text` or `test=kexec_data` selects the scenario; other `test=` words are for the loader and are ignored here; with none the scenario is `m2`. So `cargo xtask run` boots the three-program demo.

## 7. Boot behaviour and serial output

Steps 1–8 are those of [07 §7](07-milestone-1-spec.md#7-boot-behaviour-and-expected-serial-output), unchanged up to `SK:KERNEL:M1_OK attempts=<n>`. With `stop=m1` the kernel then ends as in Milestone 1. Otherwise it continues; interrupts stay off until step 20. Every line below is also drawn on the screen once step 13 has run.

| Step | What the kernel does | Line |
|---|---|---|
| 9 | GDT, TSS and IDT ([03 §5.1](03-kernel-and-interfaces.md#51-faults)): seven GDT slots: null, `0x08` kernel code, `0x10` kernel data, `0x18` user data, `0x20` user code, and the 16-byte TSS descriptor in `0x28` and `0x30`; TSS limit `0x67`, I/O map base `0x68`, IST 1 double fault, 2 NMI, 3 machine check, each a 16 KiB stack in `.bss`; 256 interrupt gates, all privilege 0 except `0x80`; every segment register reloaded | `SK:KERNEL:ARCH gdt=7 idt=256 ist=3` |
| 10 | CPUID, then control registers: `CR0.WP` set and `EFER.NXE` confirmed set, because without them page permissions do not bind the kernel (a panic if either is missing); `CR4.SMEP` and `CR4.SMAP` where reported; `CR0.EM`, `MP`, `TS` set; `CR4.OSFXSR`, `OSXMMEXCPT`, `OSXSAVE` and `PVI` clear; `EFER.SCE` and `IA32_SYSENTER_CS` clear. The 8259 pair is remapped to `0x20`/`0x28` and fully masked | `SK:KERNEL:CPU wp=1 nxe=1 smep=<0\|1> smap=<0\|1> x2apic=<0\|1> tsc_invariant=<0\|1>` |
| 11 | Frame allocator over kinds 1 and 2, less the first 1 MiB, the reserved ranges and the framebuffer. The bitmap is carved from the end of the largest usable region | `SK:KERNEL:FRAMES total=<n> free=<n> bitmap_pages=<n>`; `free` + `bitmap_pages` equals the `usable_after_reserve` figure of step 6 |
| 12 | The kernel's own tables: its image by segment (text R+X, read-only data R, data and bss R+W, no-execute, from the linker symbols); the direct map of regions of kinds 1–4 and 6 only, 2 MiB pages with 4 KiB pages at the edges; the framebuffer window; the main kernel stack in slot 0 of the stack window. Then: load `CR3`, switch stack, free the boot stack, the loader's page tables and the loader image (reserved purposes 2, 3 and 6) | `SK:KERNEL:PAGING root=<hex> tables=<n> direct_map_regions=<n>`, then `SK:KERNEL:FRAMES released=<n> free=<n>`, then `SK:KERNEL:PAGING_AUDIT pages=<n> wx=0 user=0` (section 8.6) |
| 13 | Console on the framebuffer: text from pixel row 16, 8×16 cells, `text` on `void`; banner lines in `red`. Without a framebuffer: `SK:KERNEL:FBCON off` | `SK:KERNEL:FBCON on cols=<n> rows=<n>` (160 and 49 on the 1280×800 screen) |
| 14 | Self-test, if selected. `test=kwrite_text` writes one byte to `__text_start`; `test=kexec_data` calls a `static mut` byte array in `.data` that holds `0xC3` | `SK:KERNEL:SELFTEST name=<kwrite_text\|kexec_data> addr=<hex>`, then the exception banner below |
| 15 | Kernel heap in its window, grown a page at a time; then every kernel-half top-level entry that is still empty gets a zeroed next-level table, so that no later kernel mapping changes a top-level entry and every address space copies the same 256 | `SK:KERNEL:HEAP base=0xffffc00000000000` and `SK:KERNEL:KSPACE sealed tables=256` |
| 16 | Local APIC, per the table in [03 §5.3](03-kernel-and-interfaces.md#53-scheduling), through the device window or MSRs | `SK:KERNEL:APIC mode=<xapic\|x2apic> lint0=extint` |
| 17 | 8254 at 100 Hz, master mask `0xFE`. `clock` counts ticks. The TSC clock of [03 §5.3](03-kernel-and-interfaces.md#53-scheduling) is not implemented in this milestone (section 11.5) | `SK:KERNEL:TIMER hz=100 clock=ticks` |
| 17a | Scheduler self-check, interrupts still off: a second kernel thread and the main thread yield to each other three times each; the second thread then ends | `SK:KERNEL:SCHED_OK switches=6` |
| 18 | Boot filesystem: `sk_bootfs::parse` on the handed-over range. A rejected image is a kernel panic after `SK:KERNEL:BOOTFS_REJECTED reason=<word>`; no image at all is a panic too | `SK:KERNEL:BOOTFS entries=<n> bytes=<n>` |
| 19 | Start table: for each path of the scenario, in order, start the program. A file that `sk-elf` refuses, or a 65th process, is skipped with `SK:KERNEL:SPAWN_REFUSED name=<s> reason=<word>` | `SK:KERNEL:SCENARIO name=<s> programs=<n>`, then one `SK:KERNEL:SPAWN pid=<n> name=<s> class=<App\|System> handles=<n>` each |
| 20 | Enter the idle thread; interrupts on | `SK:KERNEL:M2_RUNNING` |

Lines after step 20:

| Line | When |
|---|---|
| `SK:APP:OUT pid=<n> name=<s> \| <text>` | Each accepted `debug_write` ([06](06-milestones.md), "Lines that tests read") |
| `SK:APP:DROPPED pid=<n> bytes=<n>` | At the end of a second in which that process's output went over the budget |
| `SK:KERNEL:CRASH pid=<n> name=<s> vector=<n> addr=<hex> rip=<hex> error=<hex>` | A fault charged to a process ([03 §5.1](03-kernel-and-interfaces.md#51-faults)) |
| `SK:KERNEL:EXIT pid=<n> name=<s> code=<n>` | `exit` |
| `SK:KERNEL:TIMER_OK ticks=100` | Once, when the tick count reaches 100, one second after step 20 |
| `SK:KERNEL:REAPED pid=<n> frames=<n>` | After either of the two lines above, when the process's memory has been freed. `frames` is the number returned to the allocator and equals the sum of its page counters |

**Exception banner.** A fault that is not charged to a process prints, with interrupts off, on serial and screen:

```
SK:KERNEL:PANIC_BEGIN
SK:KERNEL:PANIC exception vector=<n> error=<hex> rip=<hex> addr=<hex> cs=<hex>
SK:KERNEL:REGS rax=<hex> rbx=<hex> rcx=<hex> rdx=<hex> rsi=<hex> rdi=<hex> rbp=<hex> rsp=<hex>
SK:KERNEL:REGS r8=<hex> r9=<hex> r10=<hex> r11=<hex> r12=<hex> r13=<hex> r14=<hex> r15=<hex> rflags=<hex>
SK:KERNEL:PANIC_END
```

then status 37 under `qemu_exit=on`, else halt. A Rust panic keeps its Milestone 1 banner. These are the kernel's own registers; a process's registers never reach the log ([04 §6](04-security-and-permissions.md#6-crash-records)).

## 8. Interfaces

### 8.1 System calls 0–9

Entry, registers and the table are in [03 §5.5](03-kernel-and-interfaces.md#55-system-calls). What that table leaves open:

| Call | Detail fixed here |
|---|---|
| `abi_version()` | Returns 2 |
| `debug_write(ptr, len)` | `len` 0 is `OK` and prints nothing. `len` > 256, or a buffer that fails the user-copy check, is `INVALID_ARGS` and prints nothing. The budget counts `len` before escaping, in windows of 100 ticks from the process's start; a call that does not fit entirely is dropped entirely, counted, and still returns `OK` |
| `exit(code)` | `code` is the low 32 bits, printed as a signed number. Does not return |
| `yield()` | Returns `OK` |
| `sleep(ns)` | Rounded up to whole ticks; 0 is `yield` |
| `clock()` | Nanoseconds since step 20: ticks × 10,000,000 (`clock=ticks`, the only clock of this milestone) |
| `mem_alloc(len)` | `len` 0 or > 1 MiB is `INVALID_ARGS`. Rounded up to pages. Returns the lowest address at or above `USER_HEAP_BASE` where the pages fit. Over the limit, or out of frames: `NO_MEMORY`, and nothing is mapped |
| `mem_free(addr, len)` | `addr` page-aligned, `len` 1 … 1 MiB rounded up; every page in the range must have come from `mem_alloc`, else `INVALID_ARGS` and nothing is freed |
| `handle_close(h)` | `BAD_HANDLE` if `h` is not in the caller's table |
| `diag_read(h, kind, cursor, buf, len)` | Section 8.4 |

Any other number returns `INVALID_ARGS` (D-K-15). The checks on a handle run in the order of [04 §3](04-security-and-permissions.md#3-objects-handles-and-rights).

### 8.2 Faults

`sk_kcore::fault::classify(vector: u8, cs: u64, error: u64) -> Charge` returns `Charge::Process` only when the saved `CS` has RPL 3, the vector is one of 0, 1, 3, 4, 5, 6, 7, 11, 12, 13, 14, 16, 17, 19, and, for vectors 11, 12 and 13, bit 0 of the error code (EXT) is clear. Everything else is `Charge::Kernel`. Vectors `0x20`–`0x2F`, `0x80` and `0xFF` never reach it.

A charged fault: build the crash record (8.4), push it on the ring, print the `CRASH` line, mark the process Crashed, end it as section 8.3 describes (which leaves its page tables before it frees them), print `REAPED`, schedule. The register tier is kept for App-class processes and never read in this milestone: no handle carries `INSPECT_REGISTERS`.

### 8.3 Process start

`sk_elf::user_policy(max_mem: u64) -> Policy` is the window `0x40_0000` … `0x7FFF_FFFF`, `max_file` 16 MiB, `max_mem` as given. For an accepted image the kernel creates an address space (lower half empty, upper 256 entries copied), maps each segment into fresh zeroed frames with user + its flags, maps the stack, creates the handle table with what the start table says, and makes the thread ready with `RIP` = entry, `RSP` = `USER_STACK_TOP`, `RFLAGS` = `0x202`, `CS` = `0x23`, `SS` = `0x1B` and every general register zero. `libstill`'s `_start` aligns the stack, calls the program's `main`, and calls `exit` with its result.

Counters per process (`sk_kcore::account`): `image`, `heap`, `stack`, `tables` in pages. `image` is the number of pages the `PT_LOAD` segments cover; `tables` counts every page-table page of the lower half plus the top-level table. Their sum is held to the limit; a start that would pass it is refused with `reason=memory`.

**Ending a process** (`exit` or a charged fault), in this order: mark it ended; load the kernel's own top-level table into `CR3`; only then free its pages, its lower-half page tables and its top-level table; print `REAPED`; pick the next thread. This code runs on the ended thread's kernel stack, which is kernel-half memory and stays (D-K-17). `mm::virt::destroy` panics if the table it is given is the one in `CR3`, and every frame returned to the allocator is filled with `0xFA` (D-K-19), so a table that is still in use after it was freed stops the boot at the next address translation instead of working by luck. Owners: the `paging` host test that a destroyed space gives back every frame exactly once, and every scenario in which a process ends.

### 8.4 `Diag`

One `Diag` object exists. `diag_read(h, kind, cursor, buf, len)`: `h` must be in the table (`BAD_HANDLE`), be a `Diag` (`WRONG_TYPE`) and carry `INSPECT` (`ACCESS_DENIED`); kind 4 also needs `INSPECT_REGISTERS` (`ACCESS_DENIED`); an unknown kind, `len` > 4096, `len` smaller than one record, or a buffer that fails the user-copy check for writing is `INVALID_ARGS`. It returns the bytes written in `RAX`, always whole records, and in `RDX` the cursor for the next call, 0 when nothing is left.

| Kind | Record (`stillkey_abi::diag`, little-endian, fixed size) | Cursor |
|---|---|---|
| 1 processes | `ProcessRecord`, 96 bytes: `pid` u64, `parent` u64 (0), `start_ms` u64, `cpu_ticks` u64, `image_pages`, `heap_pages`, `stack_pages`, `table_pages`, `shared_pages` u32 each, `state` u32 (1 alive, 2 exited, 3 crashed, 4 ended), `wait_reason` u32 (0 none, 1 sleep), `class` u32, `threads` u32, `exit_code` u32, `handles` u32, 4 zero bytes, `name` [u8; 16] | Lowest PID to return; 0 starts at the first |
| 2 totals | `Totals`, 48 bytes: `change` u64, `uptime_ms` u64, `total_pages` u64, `free_pages` u64, `kernel_pages` u64 (frames held by the kernel heap and by page tables of the kernel half), `processes` u32 (alive), `crashes` u32 (in the ring) | Ignored |
| 3 crashes | `CrashRecord`, 88 bytes: `seq` u64, `time_ms` u64, `pid` u64, `rip` u64, `addr` u64, `image_offset` u64 (`rip` − `0x40_0000`, or all ones outside the image), `error` u64, `vector` u32, `flags` u32 (bit 0: a register tier exists), `digest` [u8; 8] (first bytes of the image's SHA-256), `name` [u8; 16] | Lowest `seq`; 0 starts at the oldest kept |
| 4 crash registers | 144 bytes, 18 values of 8: `rax`, `rbx`, `rcx`, `rdx`, `rsi`, `rdi`, `rbp`, `r8` … `r15`, `rip`, `rsp`, `rflags` | The crash's `seq` |

`change` rises by one whenever a process starts or ends or a crash is recorded. The state names an inspector prints are `Running` (1), `Exited` (2), `Crashed` (3).

### 8.5 `sk-kcore` modules

Each is a pure module with its tests in `lib/sk-kcore/tests/<module>.rs`. Signatures are the contract between steps; bodies are the implementer's.

| Module | Produces | Tests must show |
|---|---|---|
| `x86` | `gdt(tss_base: u64) -> [u64; 7]` (null, the four segments, then the two halves of the TSS descriptor); `tss_descriptor(base: u64) -> [u64; 2]`; `Tss` (104 bytes, `set_rsp0`, `set_ist`); `idt_gate(handler: u64, ist: u8, dpl: u8) -> [u64; 2]`; `TrapFrame` (`#[repr(C)]`: `r15` … `rax`, `vector`, `error`, `rip`, `cs`, `rflags`, `rsp`, `ss`; 176 bytes); `PageFault::decode(error) -> {present, write, user, reserved, fetch}` | Seven GDT slots, the first five holding `0`, `0x00AF_9A00_0000_FFFF`, `0x00CF_9200_0000_FFFF`, `0x00CF_F200_0000_FFFF`, `0x00AF_FA00_0000_FFFF`; the TSS descriptor has type `0x89` and limit `0x67`; a new `Tss` has `0x68` at offset `0x66`; a gate for `0xFFFF_FFFF_8012_3456` with IST 1 splits the address correctly and has type `0x8E`, or `0xEE` for privilege 3 |
| `fault` | `classify` (8.2) | Every vector 0–31 with RPL 0 and RPL 3; EXT set on 11, 12, 13; vectors 2, 8, 18 from ring 3 are `Kernel` |
| `frames` | `Bitmap<'a>` over `&'a mut [u64]`: `new(storage, frames)` (all used), `release(first, count)`, `alloc() -> Option<u64>`, `alloc_run(n) -> Option<u64>`, `free(frame) -> Result<(), FreeError>`, `free_count()`, `total()` | Nothing is free at first; a frame is never handed out twice; double free and out-of-range free are errors; `alloc_run` finds a run that spans two words; exhaustion returns `None` |
| `paging` | `trait PhysMem { fn table(&mut self, frame: u64) -> &mut [u64; 512]; }`; `Flags { writable, user, no_execute, cache_disable }`; `Space { root }` with `map_4k`, `map_2m`, `unmap_4k -> Option<u64>`, `translate(virt) -> Option<Mapping>` (the frame and the permissions the CPU applies: writable and user only if set at every level, no-execute if set at any), `destroy_user_half` (hands back every page and table of the lower half), `fill_kernel_half`, each taking `&mut impl PhysMem` and a frame source for new tables; `MapError::{NoMemory, AlreadyMapped, NotCanonical}` | Against a `HashMap` of fake frames: a mapping translates back with its flags; a user mapping is user and writable at every level and a kernel one is user at none; `translate` reports a writable leaf under a read-only table as read-only, a user leaf under a supervisor table as supervisor, and an executable leaf under a no-execute table as no-execute; `destroy_user_half` hands back every frame exactly once and leaves the upper half untouched; mapping over an existing page is an error; after `fill_kernel_half` all 256 upper top-level entries are present and a later kernel mapping changes none of them; `unmap_4k` returns the frame and the address no longer translates; a non-canonical address is refused |
| `usercopy` | `check_range(addr, len) -> Result<(), Fault>` (inside `0x40_0000` … `0x7FFF_FFFF_EFFF`, no wrap); `plan(space, mem, addr, len, write) -> Result<Chunks, Fault>` giving `(frame, offset, len)` pieces, each page present + user (+ writable for `write`) and inside a caller-given "is direct-mapped" test | A kernel address, a wrapping range, a range ending in an unmapped page, a read-only page for a write, and a device frame each fail; a three-page range gives three chunks with the right offsets |
| `heap` | `Heap::empty()`, `unsafe add(start, len)`, `alloc(Layout) -> *mut u8`, `dealloc(ptr, Layout)`, `free_bytes()`. Size classes of powers of two from 16 to 2048 bytes with free lists; larger requests take whole pages from a first-fit list of runs | On a `Vec`-backed arena: alignment up to 4096 is honoured; freed blocks are reused; exhaustion returns null and never panics; 10,000 mixed allocations and frees leave `free_bytes` where it started |
| `sched` | `Tid(u32)`; `State::{Ready, Running, Sleeping{until}, Dead}`; `Queue::new()`, `add(tid)`, `pick() -> Option<Tid>` (round-robin), `sleep(tid, until_tick)`, `tick(now) -> impl Iterator<Item = Tid>` (woken, in deadline then insertion order), `remove(tid)` | Three ready threads rotate in order; a sleeper is not picked before its tick and is picked at it; two sleepers with one deadline both wake; a removed thread is never picked |
| `handles` | `Table<T>::new(cap)`, `insert(T, rights: u32) -> Result<u32, Full>`, `get(value) -> Result<(&T, u32), BadHandle>`, `remove(value) -> Result<T, BadHandle>`, `len()` | The first value is `0x1000`; 0 is never valid; a removed value stays invalid after its slot is reused; every value 0–4095 and 4096 random values are `BadHandle` on an empty table; the table refuses entry `cap` + 1 |
| `logfmt` | `Escaped(&[u8])` (`Display`); `Budget::new(per_window)`, `take(len, tick) -> bool`, `roll(tick) -> Option<u64>` (bytes dropped in the window that just ended) | CR, LF, backslash, `0x7F` and `0xFF` come out as `\x0D`, `\x0A`, `\x5C`, `\x7F`, `\xFF`; `0x20` and `0x7E` pass; 17 writes of 256 bytes in one window: 16 taken, 1 dropped, `roll` reports 256 once |
| `crash` | `Record` (the fields of 8.4 plus registers), `Ring` of 32: `push(Record) -> u64` (seq from 1), `from(seq) -> impl Iterator` | The 33rd push drops the first; `from(0)` starts at the oldest kept; sequence numbers never repeat |
| `account` | `Counters { image, heap, stack, tables }`, `Account::new(limit_pages)`, `charge(kind, pages) -> Result<(), OverLimit>`, `credit(kind, pages)`, `total()` | A charge that would pass the limit changes nothing; credit below zero panics in tests |
| `console` | `Console::new(fb: &Framebuffer) -> Option<Console>` (`cols` = width / 8, `rows` = (height − 16) / 16), `write_line(&mut self, fb, buf: &mut [u8], text: &[u8], rgb: u32)`: wraps at `cols`, and each further row of a line starts after two empty cells, so that only the start of a line, which the kernel writes, begins a row (section 11.5); scrolls by one text row when full, draws bytes outside `0x20`–`0x7E` as `?` | On a 64×64 buffer: `A` in the first cell sets exactly the glyph's pixels in both byte orders and nothing above row 16; a line of `cols` + 1 bytes uses two rows; writing `rows` + 1 lines moves the second line to the first row and clears the last |
| `font` | `GLYPHS: [[u8; 16]; 95]`, `glyph(byte) -> &[u8; 16]` | The space glyph is empty; `m2-unit` regenerates the file and compares |

### 8.6 Effective page permissions

A permission is what the CPU applies after it has combined all four levels of the tables, with `CR0.WP` and `EFER.NXE` on. An entry that looks right can be undone by a level above it, so permissions are tested as combined, in four places.

| Where | Check |
|---|---|
| Host, `paging` tests | `translate` combines the levels as the CPU does (8.5) |
| Kernel, at boot (step 12) | `PAGING_AUDIT`: every mapped page of the kernel half is read back with `translate`. Kernel text must be read-only and executable; read-only data read-only and no-execute; data, bss, stacks, the direct map, the framebuffer and the device window writable and no-execute; no page both writable and executable; no page user-accessible; the page below each kernel stack unmapped. A mismatch is a kernel panic. `pages=` counts the 4 KiB units checked |
| Kernel, at each process start | The same read-back over the new lower half: each segment's pages are user pages with exactly the write and execute permission of its `PT_LOAD` flags; the stack is writable and no-execute; the page below the stack and the page at address 0 are unmapped. A mismatch is a kernel panic |
| In the VM, by the fault itself | Kernel: `m2-kwrite`, `m2-kexec`. Ring 3: the probes `wtext`, `xstack` and `xheap` (section 9), whose `CRASH` lines must carry the error code of a protection fault, not of a missing page |

## 9. Programs

Every program prints through `libstill`'s `out!`, one `debug_write` per line. `<NAME>` is a status code's name as in section 4.

| Program | Behaviour | Text it prints |
|---|---|---|
| `hello` | Checkpoint 1's program, kept as the smallest smoke test. Prints, calls `clock` until it has advanced by 30 ms, prints, returns 0 | `hello from ring 3 abi=2`, then `hello ticks=3` |
| `ticker` | Forever: sleep one second, print the next number | `tick <n>`, from 1 |
| `faulter` | Prints, sleeps three seconds, then `mov qword ptr [0], 1` in `asm!` | `faulter start` |
| `spinner` | Prints, then executes a jump to itself for ever: no system call, no yield, no sleep | `spinner start` |
| `inspector` | At start reads kind 4 once, then reads the totals into a buffer that lies in its own text segment, and prints both results. Then every 100 ms reads the totals; when `change` differs from the last table printed, or a second has passed, prints a table | `inspector abi=2 registers=<NAME> rotext=<NAME>`; per table: `table change=<n> processes=<n> crashes=<n> free=<n> kheap=<n> total=<n> uptime_ms=<n>`, one `ps pid=<n> name=<s> class=<App\|System> state=<Running\|Exited\|Crashed> image=<n> heap=<n> stack=<n> tables=<n> cpu=<n>` per process, one `crash seq=<n> pid=<n> name=<s> vector=<n> addr=<hex> rip=<hex> offset=<hex>` per record, then `end change=<n>` |

Probes (`user/samples/prober/src/bin/<probe>.rs`, programs `prober-<probe>`). Faulting accesses and privileged or vector instructions are written in `asm!`; the vector instructions as bytes, because the assembler for a soft-float target may refuse the mnemonic (assumption A-18).

| Probe | Does | Must end with |
|---|---|---|
| `kread` | `mov rax, [0xFFFF_FFFF_8000_0000]` | `CRASH … vector=14 addr=0xffffffff80000000` |
| `hlt`, `in`, `out`, `cli` | `hlt`; `in al, 0x60`; `out 0x21, al`; `cli` | `CRASH … vector=13 addr=0x0` each |
| `sse` | `.byte 0x0f, 0x57, 0xc0` (`xorps xmm0, xmm0`) | `vector=6` |
| `x87` | `.byte 0xdb, 0xe3` (`fninit`) | `vector=7` |
| `twin-a`, `twin-b` | One source, `cfg` per binary. `mem_alloc(4096)`; print `twin addr=<hex>`; fill the page with `0xA5` (a) or `0x5B` (b); sleep two seconds; read every byte and print `twin readback=<own\|other\|mixed>`. `twin-b` then calls `mem_free`, prints `twin freed`, and reads the address. `twin-a` sleeps one more second, reads again, prints `twin final=<own\|other\|mixed>` and exits with 0 | Both print the same `addr`; both `readback=own`; `twin-b` ends with `vector=14` at that address; after that line `twin-a` prints `final=own` and `EXIT … code=0` |
| `memcap` | `mem_alloc(1 MiB)` until a call fails; prints `memcap obtained_mib=<n> status=<NAME>`; writes one byte to the first and last obtained megabyte; prints `memcap alive`; exits with 0 | `status=NO_MEMORY`, `n` from 1 to 15 (15 is expected), then `memcap alive` |
| `badptr` | Four `debug_write` calls: a kernel address; a 16-byte range starting 8 bytes before the end of a page from `mem_alloc(4096)`; a valid buffer with `len` 257; a kernel address after setting `EFLAGS.AC` with `pushfq`/`popfq`. Prints `badptr case=<kernel\|straddle\|long\|ac> status=<NAME>` after each; exits with 0 | Four lines, each `status=INVALID_ARGS`; the process has exactly these four output lines; no panic |
| `handles` | `diag_read` and `handle_close` with every value 0–8191 and 4096 values from xorshift32 seeded `0x2545_F491`; prints `handles calls=24576 bad_handle=<n> other=<n>`; exits with 0. The `ticker` of the scenario holds a `Diag` handle, the value `0x1000`, while the probe runs (section 11.5) | `bad_handle=24576 other=0` |
| `forge` | One `debug_write` of `a\r\nSK:KERNEL:CRASH pid=1 name=ticker vector=14\r\nAUDIT seq=1`; then 256 writes of 256 `x` bytes without sleeping; sleeps two seconds; prints `forge done`; exits with 0 | The first text appears once, inside one `SK:APP:OUT` line, with `\x0D\x0A` in it; no log line starts with text the probe chose; at most 32 of the `x` lines appear, and 256 × their number plus the `bytes` of this PID's `SK:APP:DROPPED` lines is exactly 65,536; `forge done` appears |
| `edge` (added by this document) | Prints `edge case=<name> status=<NAME>` for: `unknown` (call number 999), `write0` (`debug_write` with `len` 0), `alloc0` (`mem_alloc(0)`), `allocbig` (`mem_alloc(1 MiB + 1)`), `freebad` (`mem_free` of `0x2_0000_0000`), `freewrap` (`mem_free` of `0xFFFF_FFFF_FFFF_F000`, a range whose end is past the top of the address space; section 11.5), `close0` (`handle_close(0)`); then executes `int 0x20` | `INVALID_ARGS`, `OK`, `INVALID_ARGS`, `INVALID_ARGS`, `INVALID_ARGS`, `INVALID_ARGS`, `BAD_HANDLE`, then `CRASH … vector=13` |
| `wtext` (added by the revision) | Prints `wtext addr=<hex>`, the address of its own entry point, then writes one byte there | `CRASH … vector=14 addr=<that address> … error=0x7` (present, write, user) |
| `xstack`, `xheap` (added by the revision) | Writes the byte `0xC3` (`ret`) to its stack, or to a page from `mem_alloc`, prints `<probe> addr=<hex>` and calls that address | `CRASH … vector=14` with `addr` and `rip` both that address and `error=0x15` (present, user, instruction fetch) |

Start table (`kernel/src/proc/start.rs`), PIDs in this order from 1:

- `m2-hello`: `bin/hello`.
- `m2`: `bin/ticker`, `bin/faulter`, `bin/inspector` (given `Diag` with `INSPECT`; `handles=1`).
- `m2-preempt`: `bin/ticker`, `bin/spinner`, `bin/inspector` (with the same handle).
- `m2-prober`: `bin/ticker` (given `Diag` too, `handles=1`, so that another process holds the value the probe `handles` guesses), then `bin/prober-kread`, `-hlt`, `-in`, `-out`, `-cli`, `-sse`, `-x87`, `-twin-a`, `-twin-b`, `-memcap`, `-badptr`, `-handles`, `-forge`, `-edge`, `-wtext`, `-xstack`, `-xheap`, then `bin/bad-elf`.

## 10. Acceptance tests

`xtask/src/m2.rs`. Each test prints `RESULT test=<name> … result=<pass|fail>` lines as the Milestone 1 tests do and keeps its evidence in `build\<test>\`. Expectation files hold the ordered boot prefixes of section 7 and the forbidden texts; the checks below are in code.

| Test | Runs | Passes when |
|---|---|---|
| `m2-unit` | Once, no VM | `cargo test --locked` for `stillkey-abi`, `sk-bootfs`, `sk-kcore` (and the Milestone 1 crates), none running zero tests. Then artifact checks: every program in `BOOTFS.IMG` except `bad-elf` passes `sk_elf::validate` with the user policy; `bad-elf` fails it with `wx`; `sk_bootfs::parse` accepts the image and each entry's digest equals the SHA-256 of its bytes; no program file contains a host path; `cargo xtask font` reproduces `font.rs` |
| `m2-repro` | Once, no VM | Two release builds in fresh target directories give byte-identical `BOOTFS.IMG` |
| `m2-screen` | Once | Boot `test=kwrite_text` without `qemu_exit`; after `SK:KERNEL:PANIC_END` take the screen. Every pixel equals the picture `xtask` renders with `sk_kcore::console` from the serial lines that follow `SK:KERNEL:FBCON on`: rows 0–7 `cyan`, rows 8–15 `void`, banner lines in `red`, the rest in `text` |
| `m2-hello` | Repeated | Checkpoint 1. Boot `test=m2-hello`: the boot lines of section 7 that exist so far, in order; `SPAWN pid=1 name=hello class=App handles=0`; from PID 1 the lines `hello from ring 3 abi=2` and `hello ticks=3`, which needs timer interrupts taken in ring 3; `EXIT pid=1 name=hello code=0`; `REAPED pid=1 frames=<n>`, where `n` is the image pages `xtask` computes from the ELF file + 16 stack pages + 7 page tables, and the frames the process gave back are the frames it was charged (the kernel checks that itself and panics otherwise); `TIMER_OK ticks=100`. No `PANIC` line. The loader's `BOOTFS_SHA256` equals the host's. Then the harness ends the VM |
| `m2` | Repeated | Criteria 1–4 and 7 of [06](06-milestones.md): the three `SPAWN` lines with `handles=0`, `0`, `1`; a table in which `ticker` and `faulter` are `Running` with `image`, `stack` and `tables` above zero and `image` equal to the page count `xtask` computes from each ELF file; `inspector … registers=ACCESS_DENIED rotext=INVALID_ARGS`; the `CRASH` line for `faulter` with `vector=14 addr=0x0`, and in the first complete table after it `faulter` is `Crashed` with a `crash` line carrying the same vector, address and `rip`, and `offset` = `rip` − `0x40_0000`; the `frames` of `faulter`'s `REAPED` line equals its `image` + `heap` + `stack` + `tables` in the last table before the crash; in every table of the run, `free` + `kheap` + the pages of all `Running` processes is the same number, so the pages of an ended process are back in the allocator; five more `tick` lines after the crash, all ticks of the run consecutive from 1 and from one PID. Then the harness ends the VM |
| `m2-preempt` | Repeated | Scenario `ticker`, `spinner`, `inspector`. After `spinner start`: five more `tick` lines with consecutive numbers; `spinner` is `Running` in every table; between two tables whose `uptime_ms` differ by at least 1000, `spinner`'s `cpu` rises by at least half the ticks that passed, so it really holds the CPU, and `ticker`'s by less than a tenth of them; no `EXIT` or `CRASH` line for any of the three |
| `m2-prober` | Repeated | Criterion 5: every row of section 9; `SPAWN_REFUSED name=bad-elf reason=wx`; every `CRASH` line belongs to a probe that is expected to crash; `ticker`'s numbers are consecutive from 1 through two ticks after the last probe has ended |
| `m2-kwrite`, `m2-kexec` | Repeated | Criterion 6: status 37; the banner has `vector=14`; for `kwrite` the error code has bits 0 and 1 set and bit 2 clear and `addr` is the `SELFTEST` address; for `kexec` the error code has bits 0 and 4 set and `rip` and `addr` are the `SELFTEST` address |
| `m2-badfs` | Repeated | An image whose `BOOTFS.IMG` has a wrong magic: `SK:KERNEL:BOOTFS_REJECTED reason=magic`, no `SPAWN` line, status 37 |

Forbidden in `m2` and `m2-prober`: `PANIC`, `REJECTED`, `FAILED`, a status of 37. In every Milestone 2 run the loader's `BOOTFS_SHA256` must equal the host's digest of the image. In every scenario with a `ticker` the harness notes when it first saw each line, and the `tick` lines must have come 600 to 1600 ms apart on average: the timer runs at 100 Hz, not at half or twice that (section 11.5).

**Order and repeats.** `cargo xtask test m2` runs `m2-unit`, `m2-repro` and `m2-screen` once, then `m2-hello`, `m2`, `m2-preempt`, `m2-prober`, `m2-kwrite`, `m2-kexec` and `m2-badfs` `--repeat` times. `cargo xtask test all` runs `m1`, then `m2`. The default time limit stays 60 seconds; `m2` needs about ten. The scenario `m2` alone is `cargo xtask test m2-crash` (section 11.4).

**Acceptance (D-W-5):** with no failure,

```bash
cargo xtask test all --repeat 20 --accel kvm
cargo xtask test all --repeat 20 --accel tcg
cargo xtask test all --release --repeat 20 --accel kvm
cargo xtask test all --release --repeat 20 --accel tcg
```

A single odd run is a defect to trace, not a run to repeat: Milestone 1's only real defect showed once in about 800 boots ([07 §0](07-milestone-1-spec.md#0-status-2026-11-10-implemented)).

## 11. Implementation order

Each step is committed on the branch `milestone-2` when its check passes. "RED" is the failure to see before the code exists.

| Step | Work | Consumes → produces | Check |
|---|---|---|---|
| 0 | Independent review of this document against 03, 04 and 06; correct it | – | Findings recorded in `docs/records/` |
| A | `stillkey-abi` with `tests/abi.rs`: every number of section 4 pinned; `Status::name`; `diag` records encode and decode with the sizes and offsets of 8.4 | → all constants, `Status`, `diag::{ProcessRecord, Totals, CrashRecord}` | RED: the crate does not exist. `cargo test --locked -p stillkey-abi` |
| B | `sk-bootfs` (`parse`, `build::pack`) with `tests/format.rs`: one test per reject word of 5.1, a pack-then-parse round trip, identical output for shuffled input. `sk_elf::user_policy` with tests for the window ends and W+X. `xtask/src/bootfs.rs` reads `bootfs/MANIFEST` and packs; `user/user.ld`, `libstill` (`_start`, raw `syscall0`–`syscall5`, `out!`, panic handler that prints and exits with 101) and `ticker`, so that there is one real program to pack | abi → `Image`, `Entry`, `Reject`, `pack`, `Artifacts.bootfs` | `cargo test --locked -p sk-bootfs -p sk-elf`; `cargo xtask build` writes `BOOTFS.IMG` and `sk_bootfs::parse` accepts it |
| C | Handoff: the three `validate` rows of 5.4 with host tests (each condition broken once); the loader reads `BOOTFS.IMG`; `image` copies it; kernel option `stop=m1`; the `m1` expectation files and tests gain `stop=m1` and an image without `BOOTFS.IMG` | B → `Accepted::bootfs() -> Option<PhysRange>`; loader lines of 5.4 | RED: `m1-boot` fails once the kernel no longer stops by itself. Then `cargo xtask test m1` passes in both profiles, and a `test=m2` boot logs `BOOTFS_FILE` and `BOOTFS_SHA256` |
| D | `sk_kcore::{x86, fault}` with tests. Kernel `arch`: GDT, TSS, IDT, stubs for all 256 vectors, the exception banner; `selftest.rs`; `follow.rs`; tests `m2-kwrite`, `m2-kexec` | → `TrapFrame`, `classify`, `arch::init()`, `follow` | RED: both tests end in a reset or a time limit. Then both pass. (They run on the loader's tables until step F and must still pass after it) |
| E | Control registers and CPUID line; 8259 remap and mask; `sk_kcore::frames` with tests; `mm::phys` | C, D → `phys::alloc()`, `phys::free()`, `phys::counts()` | The `FRAMES` line appears and `free` + `bitmap_pages` equals `usable_after_reserve` (checked by `m2-kwrite`'s expectation from now on) |
| F | `sk_kcore::paging` with tests; `mm::virt`: the kernel's tables, the switch, the new stack, the three releases. `sk_kcore::{console, font}` with tests, `tools/fetch-font.sh`, `xtask font`, `drivers::fbcon`, `log.rs` writes to both; test `m2-screen` | E → `virt::kernel_space()`, `virt::map_mmio(phys) -> u64`, `fbcon` | RED: `m2-screen` sees the plain band. Then `m2-screen`, `m2-kwrite`, `m2-kexec` pass |
| G | `sk_kcore::heap` with tests; `mm::heap` (`#[global_allocator]`, grows by mapping pages); kernel-half seal; local APIC; 8254; tick counter; `clock`; idle loop. The kernel logs `SK:KERNEL:TIMER_OK ticks=100` once, one second after step 20 | F → `alloc`, `arch::ticks()`, `arch::clock_ns()` | RED: no `TIMER_OK` (no interrupt arrives). Then a `test=m2` boot shows it under KVM and TCG. `m2.expect` requires it from now on |
| H | `sk_kcore::sched` with tests; kernel threads on stack-window slots; the context switch; `sched::{spawn_kernel, yield_now, sleep_until, exit_thread}`. A boot self-check thread yields three times with the main thread and the kernel logs `SK:KERNEL:SCHED_OK switches=6` | G → the scheduler | RED: no `SCHED_OK`. Then present in both profiles (a callee-saved register lost in the switch shows only under optimisation) |
| I | `sk_kcore::{usercopy, logfmt, account}` with tests. `mm::usercopy`; address spaces; `fs::bootfs`; `proc::load`; first entry to ring 3; vector `0x80`; calls 0–5; `proc::start` with the scenario table; `RSP0` set at every switch. Test `m2`, written whole, reporting the first unmet requirement | A, B, H → running `ticker` | RED: `m2` fails with `no SPAWN line`. Then it fails later, with `no CRASH line for faulter`, and the log shows `tick 1` … |
| J | `sk_kcore::crash` with tests. Faults charged to a process; `exit`; teardown and `REAPED`; `faulter` | I → `diag::crashes()`, `proc::end(pid, how)` | `m2` now fails with `no inspector table`; `ticker` keeps counting after the `CRASH` line |
| K | `sk_kcore::handles` with tests; `object` with the `Diag` type; calls 6–9; `diag` snapshots; `inspector` | A, J → all of section 8 | `m2` passes, in both profiles and under both accelerators |
| L | The seventeen probe binaries and `bad-elf`; test `m2-prober` | K | RED: `m2-prober` names the first probe without its end line. Then it passes |
| M | `m2-unit`, `m2-repro`, `m2-badfs`; `test m2` and `test all`; status paragraphs in this document, 06, 01 and the README; the memory files | all | The acceptance commands of section 10 |

### 11.1 Checkpoints

The steps are delivered in three checkpoints, each with its own report.

| Checkpoint | Shows | Contains | Left for a later checkpoint |
|---|---|---|---|
| 1 | The timer fires, and one ring-3 program prints to serial and exits: test `m2-hello` | Steps A and B. Of C: the `bootfs` row, the loader, `stop=m1`. Of D: GDT, TSS, IDT and the exception banner. E. Of F: `sk_kcore::paging` with 4 KiB pages, and process address spaces whose upper half is the loader's. Of G: the local APIC, the 8254 and the tick count. Of I: user copy in, the ELF load with its permission read-back, ring 3 entry, calls 0, 1, 2, 3 and 5 | The kernel's own page tables and stacks and `PAGING_AUDIT`; the console; the heap; the scheduler, so one process runs at a time on the boot stack and `sleep` is refused; faults charged to a process, so any fault is a kernel panic; the output budget; calls 4 and 6–9; the `kernel` and `stack` handoff rows; the kernel self-tests. Until the device window exists the local APIC is reached through the loader's direct map |
| 2 | Crash containment, the inspector and pre-emption: tests `m2`, `m2-preempt`, `m2-kwrite`, `m2-kexec`, `m2-screen` | The rest of C, D, F, G and I; H; J; K; `spinner` | Probes; the matrix |
| 3 | The probes and the acceptance matrix | L and M | — |

### 11.2 Checkpoint 1 result (2026-11-10)

Checkpoint 1 is implemented as its row above says.

| Command | Result |
|---|---|
| `cargo xtask test m2-hello` | `RESULT test=m2-hello run=1/1 profile=dev accel=kvm status=0 firmware_lines=2 result=pass` |
| `cargo xtask test m2-hello --release --repeat 3` | three `result=pass` lines, `accel=kvm` |
| `cargo xtask test m2-hello --accel tcg` and `--release --accel tcg` | `result=pass` each |
| `cargo xtask test m1` | `SUMMARY test=m1 profile=dev repeat=1 result=pass accel=kvm`: the Milestone 1 tests pass with `stop=m1` |
| `cargo test -p stillkey-abi -p sk-bootfs -p sk-kcore -p sk-elf -p stillkey-bootinfo -p xtask` | 174 host tests pass |

The lines of the passing run that this checkpoint adds, after `SK:KERNEL:M1_OK`:

```
SK:KERNEL:ARCH gdt=7 idt=256 ist=3
SK:KERNEL:CPU wp=1 nxe=1 smep=1 smap=1 x2apic=1 tsc_invariant=0
SK:KERNEL:FRAMES total=53626 free=53626 bitmap_pages=2
SK:KERNEL:APIC mode=xapic lint0=extint
SK:KERNEL:TIMER hz=100 clock=ticks
SK:KERNEL:BOOTFS entries=1 bytes=790528
SK:KERNEL:SCENARIO name=m2-hello programs=1
SK:KERNEL:SPAWN pid=1 name=hello class=App handles=0
SK:KERNEL:M2_RUNNING
SK:APP:OUT pid=1 name=hello | hello from ring 3 abi=2
SK:APP:OUT pid=1 name=hello | hello ticks=3
SK:KERNEL:EXIT pid=1 name=hello code=0
SK:KERNEL:REAPED pid=1 frames=27
SK:KERNEL:TIMER_OK ticks=100
```

**That the test can fail.** Six defects were put in one at a time, and `m2-hello` failed each time for the reason expected: ending a process without leaving its page tables (the kernel's own assertion); not freeing the top-level table (the free count is not back at its start value); leaving the tick masked (`hello ticks=3` never appears, time limit); closing the system-call gate to ring 3 (general-protection fault from `cs=0x23`); mapping the stack executable (the read-back of the effective permission); and a `hello` with a writable code segment (refused at build time with `wx`). Ten defects in `sk-kcore` each fail a host test, among them one for each level of the effective-permission rule.

**Where checkpoint 1 differs from the full design**, beyond what the row in 11.1 lists:

- The kernel runs on the address space the loader built and on the boot stack. A process's top-level table copies the loader's upper half.
- `FRAMES total=` is the number of frames the allocator can hand out, which at boot equals `free`.
- With no scenario on the command line the kernel runs `m2-hello`, because `m2` does not exist yet.
- A read error on `BOOTFS.IMG` stops the loader with `SK:LOADER:BOOTFS_REJECTED reason=read status=<hex>`.
- `sk_kcore::paging::MapError` has a fourth value, `Unaligned`.
- `m1-unit` does not run the host tests of the new crates yet; `m2-unit` will.

**Not done or not checked:** the acceptance matrix (20 repeats in every profile and accelerator) was not run for `m2-hello`; the abi and `sk-bootfs` tests were written with their code and not seen failing first; no independent review of this specification or of the code has taken place (section 11, step 0); nothing was run on hardware.

### 11.3 Checkpoint 2 result (2026-11-10)

Checkpoint 2 is implemented as its row in 11.1 says: the kernel runs on its own page tables and stacks, schedules several programs with pre-emption, ends a program that faults and carries on, answers system calls 0 to 9, and draws its log on the screen.

| Command | Result |
|---|---|
| `cargo xtask test m2` | `RESULT test=m2 run=1/1 profile=dev accel=kvm status=0 firmware_lines=2 result=pass` |
| `cargo xtask test m2-preempt` | `RESULT test=m2-preempt run=1/1 profile=dev accel=kvm status=0 firmware_lines=2 result=pass` |
| `cargo xtask test m2-kwrite`, `cargo xtask test m2-kexec` | `status=37 … result=pass` each: the kernel stopped itself with its banner |
| `cargo xtask test m2-screen` | `RESULT test=m2-screen profile=dev accel=kvm status=0 screen=1280x800 lines=6 result=pass` |
| `cargo xtask test m2-hello` | `result=pass`: checkpoint 1's test on the new kernel |
| `cargo xtask test m2 --release --repeat 5`, `cargo xtask test m2-preempt --release --repeat 3` | `result=pass` every time, `accel=kvm` |
| The other four tests with `--release`; `m2`, `m2-preempt` and `m2-screen` with `--accel tcg` | `result=pass` each |
| `cargo xtask test m1` | `SUMMARY test=m1 profile=dev repeat=1 result=pass accel=kvm` |
| `cargo test -p stillkey-bootinfo -p stillkey-abi -p sk-kcore -p sk-bootfs -p sk-elf -p sk-serial -p xtask` | 292 host tests pass |

Lines of the passing `m2` run, after `SK:KERNEL:M1_OK`. Between the second table and the crash two more tables and `tick 2` are left out:

```
SK:KERNEL:ARCH gdt=7 idt=256 ist=3
SK:KERNEL:CPU wp=1 nxe=1 smep=1 smap=1 x2apic=1 tsc_invariant=0
SK:KERNEL:FRAMES total=52809 free=52809 bitmap_pages=2
SK:KERNEL:PAGING root=0x100000 tables=27 direct_map_regions=115
SK:KERNEL:FRAMES released=56 free=52830
SK:KERNEL:PAGING_AUDIT pages=65857 wx=0 user=0
SK:KERNEL:FBCON on cols=160 rows=49
SK:KERNEL:HEAP base=0xffffc00000000000
SK:KERNEL:KSPACE sealed tables=256
SK:KERNEL:APIC mode=xapic lint0=extint
SK:KERNEL:TIMER hz=100 clock=ticks
SK:KERNEL:SCHED_OK switches=6
SK:KERNEL:BOOTFS entries=6 bytes=4075520
SK:KERNEL:SCENARIO name=m2 programs=3
SK:KERNEL:SPAWN pid=1 name=ticker class=App handles=0
SK:KERNEL:SPAWN pid=2 name=faulter class=App handles=0
SK:KERNEL:SPAWN pid=3 name=inspector class=System handles=1
SK:KERNEL:M2_RUNNING
SK:APP:OUT pid=2 name=faulter | faulter start
SK:APP:OUT pid=3 name=inspector | inspector abi=2 registers=ACCESS_DENIED rotext=INVALID_ARGS
SK:APP:OUT pid=3 name=inspector | table change=3 processes=3 crashes=0 free=52454 kheap=290 total=52865 uptime_ms=10
SK:APP:OUT pid=3 name=inspector | ps pid=1 name=ticker class=App state=Running image=4 heap=0 stack=16 tables=7 cpu=1
SK:APP:OUT pid=3 name=inspector | ps pid=2 name=faulter class=App state=Running image=3 heap=0 stack=16 tables=7 cpu=0
SK:APP:OUT pid=3 name=inspector | ps pid=3 name=inspector class=System state=Running image=5 heap=0 stack=16 tables=7 cpu=0
SK:APP:OUT pid=3 name=inspector | end change=3
SK:KERNEL:TIMER_OK ticks=100
SK:APP:OUT pid=1 name=ticker | tick 1
SK:KERNEL:CRASH pid=2 name=faulter vector=14 addr=0x0 rip=0x400071 error=0x6
SK:KERNEL:REAPED pid=2 frames=26
SK:APP:OUT pid=1 name=ticker | tick 3
SK:APP:OUT pid=3 name=inspector | table change=5 processes=2 crashes=1 free=52480 kheap=290 total=52865 uptime_ms=3020
SK:APP:OUT pid=3 name=inspector | ps pid=1 name=ticker class=App state=Running image=4 heap=0 stack=16 tables=7 cpu=1
SK:APP:OUT pid=3 name=inspector | ps pid=2 name=faulter class=App state=Crashed image=0 heap=0 stack=0 tables=0 cpu=0
SK:APP:OUT pid=3 name=inspector | ps pid=3 name=inspector class=System state=Running image=5 heap=0 stack=16 tables=7 cpu=1
SK:APP:OUT pid=3 name=inspector | crash seq=1 pid=2 name=faulter vector=14 addr=0x0 rip=0x400071 offset=0x71
SK:APP:OUT pid=3 name=inspector | end change=5
```

The ticker then prints `tick 4` to `tick 8`. The 26 frames of the faulter are back: `free` goes from 52454 to 52480. In `m2-preempt` the `spinner`, which makes no system call after its first line, has `cpu=1`, `101`, `201`, `301`, `401`, `502` in tables one second apart, while the ticker prints `tick 1` to `tick 5` and stays at `cpu=1`.

In `m2-kwrite` the banner is `SK:KERNEL:PANIC exception vector=14 error=0x3 … addr=0xffffffff80000000`, the address announced by `SK:KERNEL:SELFTEST name=kwrite_text`; in `m2-kexec` it is `error=0x11` with `rip` and `addr` both the announced address in the kernel's data. The screen that `m2-screen` compares pixel by pixel is kept as [records/evidence/m2-screen-checkpoint2.png](records/evidence/m2-screen-checkpoint2.png) (its top 160 rows).

**That the tests can fail.** `m2-screen` was run first with the console set up but not drawing, and failed: `14784 pixel(s) differ; the first is x=1 y=18: #06080c, expected #e9eef5`. With the pre-emption of programs taken out of the timer handler, `m2-preempt` failed: after `spinner start` no `tick` line came, and the time limit ended the run (status 124). The checks of `m2` and `m2-preempt` are pure functions with 21 host tests that change one fact of a good run at a time (the image size, the crash record, the reaped frames, a lost frame, a missing tick, the spinner's share).

**One defect found by the tests, in the harness.** A release run of `m2` failed with `no change= in table cha`: the harness had ended the VM while the kernel was in the middle of a line, and then read that half line as a table. The harness now leaves out a last line that has no line end (`follow::whole`, with a host test). After the change `m2` passed five of five release runs.

**Where checkpoint 2 differs from the specification:**

- The kernel's modules are flat files (`kernel/src/sched.rs`, `proc.rs`, `heap.rs`, `fbcon.rs`, `virt.rs`, `phys.rs`), not the tree of [03 §2](03-kernel-and-interfaces.md#2-subsystems-and-their-boundaries). The boundaries are the same.
- `follow` returns the reason of `Verdict::Failed` as its error, after it ended the VM.
- A table of the inspector ends with `end change=<n>`, read again from the kernel: the harness takes a table as whole only if that number is the one the table began with. If a read fails the inspector prints `end failed`.
- The screen shows the first 512 bytes of a log line. Every line from `SK:KERNEL:PANIC_BEGIN` on is red.
- A scroll of the console copies the text inside the framebuffer with interrupts off. On the test screen that is quick enough under KVM and under software emulation; a back buffer in RAM is the fix if it is not.
- The Spleen files came from `https://raw.githubusercontent.com/fcambus/spleen/2.2.0/`, and `tools/fetch-font.sh` pins their SHA-256.

**Not done or not checked:** the probes and `m2-prober`, `m2-unit`, `m2-repro`, `m2-badfs` and the acceptance matrix belong to checkpoint 3; `bad-elf` is not in the manifest yet; `m1-unit` still does not run the host tests of the new crates. Most host tests of this checkpoint were written together with their code and were not seen failing first; the mutation passes owed for them followed in checkpoint 3 (section 11.4). No independent review of this specification or of the code has taken place (section 11, step 0). Nothing was run on hardware, and nothing outside the project folder and its disposable VM images was written.

### 11.4 Checkpoint 3 result (2026-11-11)

Checkpoint 3 is implemented as its row in 11.1 says, and the four acceptance commands of section 10 pass. With it every step of section 11 from A to M is done.

**Acceptance (D-W-5).** Run by `tools/acceptance.sh`, which keeps each command's whole output in `build/acceptance/`; its summary is [records/milestone-2-matrix.txt](records/milestone-2-matrix.txt).

| Command | Result |
|---|---|
| `cargo xtask test all --repeat 20 --accel kvm` | `SUMMARY test=all … repeat=20 result=pass`: 357 result lines, none failed, 20 min |
| `cargo xtask test all --repeat 20 --accel tcg` | `SUMMARY test=all … repeat=20 result=pass`: 357 result lines, none failed, 21 min |
| `cargo xtask test all --release --repeat 20 --accel kvm` | `SUMMARY test=all … repeat=20 result=pass`: 357 result lines, none failed, 20 min |
| `cargo xtask test all --release --repeat 20 --accel tcg` | `SUMMARY test=all … repeat=20 result=pass`: 357 result lines, none failed, 21 min |

One run of `test all` is, in this order: `m1-unit`, `m1-repro`, `m1-screen`, the six boot tests of Milestone 1 twenty times, then `m2-unit`, `m2-repro`, `m2-screen`, and `m2-hello`, `m2`, `m2-preempt`, `m2-prober`, `m2-kwrite`, `m2-kexec`, `m2-badfs` twenty times.

**The new tests, each alone:**

| Command | Result |
|---|---|
| `cargo xtask test m2-prober` | `RESULT test=m2-prober run=1/1 profile=dev accel=kvm status=0 firmware_lines=2 result=pass`; also with `--release`, `--accel tcg` and both |
| `cargo xtask test m2-badfs` | `SK:KERNEL:BOOTFS_REJECTED reason=magic`, then the kernel's banner; `status=37 … result=pass` |
| `cargo xtask test m2-unit` | host tests: `stillkey-abi` 10, `sk-bootfs` 8, `sk-kcore` 90, `sk-elf` 17, `stillkey-bootinfo` 102, `xtask` 95; then `RESULT test=m2-unit artifact=BOOTFS.IMG profile=dev entries=24 programs=23 refused=bad-elf:wx host_paths=none result=pass` |
| `cargo xtask test m2-repro` | `RESULT test=m2-repro artifact=BOOTFS.IMG sha256_a=… sha256_b=… host_paths=none result=pass`: the same digest twice |

Lines of a passing `m2-prober` run after `SK:KERNEL:M2_RUNNING`. The `REAPED` lines, fourteen more lines of 256 `x`, and `tick 3`, `tick 4` are left out:

```
SK:KERNEL:SPAWN_REFUSED name=bad-elf reason=wx
SK:KERNEL:M2_RUNNING
SK:KERNEL:CRASH pid=2 name=prober-kread vector=14 addr=0xffffffff80000000 rip=0x400043 error=0x5
SK:KERNEL:CRASH pid=3 name=prober-hlt vector=13 addr=0x0 rip=0x40003c error=0x0
SK:KERNEL:CRASH pid=5 name=prober-out vector=13 addr=0x0 rip=0x40003e error=0x0
SK:KERNEL:CRASH pid=6 name=prober-cli vector=13 addr=0x0 rip=0x40003c error=0x0
SK:KERNEL:CRASH pid=7 name=prober-sse vector=6 addr=0x0 rip=0x40003c error=0x0
SK:APP:OUT pid=9 name=prober-twin-a | twin addr=0x100000000
SK:APP:OUT pid=10 name=prober-twin-b | twin addr=0x100000000
SK:APP:OUT pid=12 name=prober-badptr | badptr case=kernel status=INVALID_ARGS
SK:APP:OUT pid=12 name=prober-badptr | badptr case=straddle status=INVALID_ARGS
SK:APP:OUT pid=12 name=prober-badptr | badptr case=long status=INVALID_ARGS
SK:APP:OUT pid=12 name=prober-badptr | badptr case=ac status=INVALID_ARGS
SK:KERNEL:EXIT pid=12 name=prober-badptr code=0
SK:APP:OUT pid=13 name=prober-handles | handles calls=24576 bad_handle=24576 other=0
SK:KERNEL:EXIT pid=13 name=prober-handles code=0
SK:APP:OUT pid=14 name=prober-forge | a\x0D\x0ASK:KERNEL:CRASH pid=1 name=ticker vector=14\x0D\x0AAUDIT seq=1
SK:APP:OUT pid=15 name=prober-edge | edge case=unknown status=INVALID_ARGS
SK:APP:OUT pid=15 name=prober-edge | edge case=write0 status=OK
SK:APP:OUT pid=15 name=prober-edge | edge case=alloc0 status=INVALID_ARGS
SK:APP:OUT pid=15 name=prober-edge | edge case=allocbig status=INVALID_ARGS
SK:APP:OUT pid=15 name=prober-edge | edge case=freebad status=INVALID_ARGS
SK:APP:OUT pid=15 name=prober-edge | edge case=freewrap status=INVALID_ARGS
SK:APP:OUT pid=15 name=prober-edge | edge case=close0 status=BAD_HANDLE
SK:KERNEL:CRASH pid=15 name=prober-edge vector=13 addr=0x0 rip=0x4003d5 error=0x102
SK:APP:OUT pid=16 name=prober-wtext | wtext addr=0x400000
SK:KERNEL:CRASH pid=16 name=prober-wtext vector=14 addr=0x400000 rip=0x400085 error=0x7
SK:APP:OUT pid=18 name=prober-xheap | xheap addr=0x100000000
SK:KERNEL:CRASH pid=18 name=prober-xheap vector=14 addr=0x100000000 rip=0x100000000 error=0x15
SK:KERNEL:CRASH pid=4 name=prober-in vector=13 addr=0x0 rip=0x40003c error=0x0
SK:KERNEL:CRASH pid=8 name=prober-x87 vector=7 addr=0x0 rip=0x40003c error=0x0
SK:APP:OUT pid=11 name=prober-memcap | memcap obtained_mib=15 status=NO_MEMORY
SK:APP:OUT pid=11 name=prober-memcap | memcap alive
SK:KERNEL:EXIT pid=11 name=prober-memcap code=0
SK:APP:OUT pid=17 name=prober-xstack | xstack addr=0x7fffffffefc0
SK:KERNEL:CRASH pid=17 name=prober-xstack vector=14 addr=0x7fffffffefc0 rip=0x7fffffffefc0 error=0x15
SK:APP:OUT pid=14 name=prober-forge | xxxx… (256 bytes; fifteen such lines)
SK:KERNEL:TIMER_OK ticks=100
SK:APP:DROPPED pid=14 bytes=61696
SK:APP:OUT pid=1 name=ticker | tick 1
SK:APP:OUT pid=9 name=prober-twin-a | twin readback=own
SK:APP:OUT pid=10 name=prober-twin-b | twin readback=own
SK:APP:OUT pid=10 name=prober-twin-b | twin freed
SK:KERNEL:CRASH pid=10 name=prober-twin-b vector=14 addr=0x100000000 rip=0x4004bf error=0x4
SK:APP:OUT pid=1 name=ticker | tick 2
SK:APP:OUT pid=14 name=prober-forge | forge done
SK:KERNEL:EXIT pid=14 name=prober-forge code=0
SK:APP:OUT pid=9 name=prober-twin-a | twin final=own
SK:KERNEL:EXIT pid=9 name=prober-twin-a code=0
```

Every row of the table in section 9 held at the first run; the kernel needed no change for any probe. 15 lines of 256 bytes and 61,696 dropped bytes are the 65,536 bytes of the flood: the budget of 4,096 bytes a second let 3,840 through.

**That the tests can fail.** With the probe `hlt` changed to spin for ever, `m2-prober` failed and named it: `QEMU exit status 124, expected 0; the probe hlt (pid 3) has no end line`. With the image left whole, `m2-badfs` failed: `no line starting with SK:KERNEL:BOOTFS_REJECTED reason=magic`. The check of `m2-prober` is a pure function with 19 host tests that change one fact of a good run at a time; the check of what `BOOTFS.IMG` holds has 5.

**The mutation passes that checkpoints 1 and 2 left owed.** `tools/mutate.py` changes one comparison, one `&&` or `||`, one `+ 1` or one `true` or `false` at a time, at evenly spaced places in the code that host tests cover, and runs the host tests of that crate. Of 80 such changes 71 made a test fail ([records/milestone-2-mutation.txt](records/milestone-2-mutation.txt)). Of the other nine, four were real gaps at a limit and have a test now, and the four changes made again are caught: a full table of 64 entries in `BOOTFS.IMG`, a table that ends exactly at a page end, a name that fills its field, and a 2,048-byte block given back to the kernel heap. Three change nothing that can be observed. Two are in the packer of `xtask`, which has no host test; either makes every build fail (by reading, not run). This is a sample, not every place.

**One defect found by the acceptance runs, in the harness.** The first run of the first command failed once, in `m2-hello`, run 14 of 20: `no line starting with SK:KERNEL:TIMER_OK ticks=100`. The log ended in `SK:KERNEL:TIMER_OK ticks=1`: the harness had seen the first bytes of the line and ended the VM while the kernel was still writing it. The harness now counts a marker only in the log up to its last line end (`qemu::marker_seen`, with a host test that failed on the old search). The matrix above is the run after that change, from the first command.

**Where checkpoint 3 differs from the specification:**

- `cargo xtask test m2` is the suite, as "Order and repeats" in section 10 says. The crash-containment test alone, which sections 10 and 11.3 also call `m2`, is `cargo xtask test m2-crash`; its result line still says `test=m2`.
- The probes are the binaries of one package, `user/samples/prober`. `twin-a` and `twin-b` share `src/twin.rs` and differ in the arguments they pass, not in a `cfg`.
- `kread` reads the address through a register, not with the absolute form of `mov`.
- `bad-elf` is made by the manifest flag `corrupt:wx`, which makes the last loadable segment of `ticker` writable and executable (the corruption `m1-reject` uses), not the text segment writable. The reason the kernel gives is `wx` either way.
- The start table is in `kernel/src/m2.rs`, not `kernel/src/proc/start.rs`.
- "No log line starts with text the probe chose" is checked as three things: after `SK:KERNEL:M2_RUNNING` every line of the serial log starts with `SK:`; the first line of `forge` is exactly its text with both line ends written as `\x0D\x0A`; and every `CRASH` line is the end of a probe that must crash.
- `m1-repro` and `m2-repro` compare different files of the same two release builds.
- A run that the time limit ends also says what its check still missed.
- A 65th start is refused with `reason=table`, as D-K-17 says; before this checkpoint the word was `processes`.

**Not done or not checked:** when this note was written no independent review had taken place (section 11, step 0); it followed on 2026-11-11 (section 11.5). No test starts 65 programs, so the refusal `reason=table` is not exercised. The two mutants in the packer were judged by reading. Nothing was run on hardware, and nothing outside the project folder and its disposable VM images was written.

### 11.5 Independent review and fixes (2026-11-11)

The review that step 0 of section 11 asks for took place after the implementation, on 2026-11-11. Three reviewers read the branch, each with a fresh context and without the author's notes: one the memory code and the CPU tables, one processes, system calls and the scheduler, one the tests, the harness and the documents against the code. Each said "with fixes". The full record, with every finding that was left as it is, is in [records/milestone-2-review.md](records/milestone-2-review.md).

**What was wrong and is fixed.** Each fix was seen failing first.

| Finding | Seen failing | Change |
|---|---|---|
| **Any program could stop the kernel.** `mem_free` added the length to the address without a check, and the kernel is built with overflow checks | New case `freewrap` of the probe `edge`: `SK:KERNEL:PANIC at kernel/src/proc.rs:506: attempt to add with overflow` | The end of the range is computed with `checked_add`; the call returns `INVALID_ARGS` |
| A line longer than a row of the screen went on in the first column of the next row, so a program could make a row begin with text of its choice, such as the start of a kernel line. The serial log was not affected | Host tests of `console` | Each further row of a line starts after two empty cells |
| The probe `handles` could not fail: it tried only values below the first handle value, and no process of its scenario held a handle | With `diag_read` made to look in the table of process 1: `handles calls=24576 bad_handle=24575 other=1` | The probe tries 0–8191 and 4096 others; `ticker` holds the `Diag` handle, the value `0x1000`, in `m2-prober` |
| Nothing noticed when the `clac` of the interrupt entry was removed | With `clac` replaced by a no-op, at the `ac` case of `badptr`: `SK:KERNEL:PANIC at kernel/src/tables.rs:185: RFLAGS.AC is set in the kernel` | With `CR4.SMAP` set, the dispatcher stops the kernel if `RFLAGS.AC` is set |
| No check measured time, so nothing showed the timer runs at 100 Hz | With the 8254 set to 200 Hz: `the ticker's lines came 501 ms apart on average, expected 600 to 1600` | The harness notes when it first saw each line; in a scenario with a `ticker` the `tick` lines must come 600 to 1600 ms apart on average |
| The harness's own line checks had no host test: the filter for `SK:` lines, the expect files, the frame sum, the fault code of `m2-kwrite` and `m2-kexec` | 10 new host tests; of 30 small changes the mutation tool then made to these checks, the scenario checks and the console, 27 made a test fail and 3 change nothing a test could see ([records/milestone-2-mutation-review.txt](records/milestone-2-mutation-review.txt)) | The expect check and the fault-code check are pure functions with tests |
| `m2-kwrite` and `m2-kexec` had never been seen to fail | With the kernel's code mapped writable, and with its data mapped executable, each test fails: the kernel's own audit of its page tables stops the boot first (`SK:KERNEL:PANIC at kernel/src/virt.rs:290: kernel page … wanted … the tables give …`), and the test reports `no line starting with SK:KERNEL:PAGING_AUDIT`. The fault-code check behind them is covered by the host tests above | None needed |
| The record of the acceptance runs did not name the commit, and a row that ran fewer tests would have passed | — | `tools/acceptance.sh` prints the commit first and compares the result counts of the rows |
| Two `asm!` options promised the compiler more than the instructions keep (`preserves_flags` on moves to and from control registers, `nomem` on `sti; hlt; cli`) | No test can show it | Removed, in the kernel and in the loader |

**What the documents said and the code does not do.** `clock()` counts timer ticks of 10 ms in every case; the TSC clock of [03 §5.3](03-kernel-and-interfaces.md#53-scheduling) is not implemented, and the boot line always says `clock=ticks`. The VM reports no invariant TSC, so that path would have run in no test. It is owed before a hardware boot. Sections 7, 8.1, 13 and 14 now say so.

**Known limits that stand, by decision.** The frames that hold the kernel's code are also writable through the direct map: "nothing is writable and executable" holds for each mapping, not for each frame. The serial port is written without waiting for the transmitter, which only a VM forgives. There is no UMIP, no address randomisation, and `rdtsc` is open to programs. The system keeps no memory in reserve. These are to be decided before a hardware boot; each has a line in the review record with what it costs if it is wrong.

**Acceptance after the fixes.** The four commands of D-W-5 passed again on 2026-11-11, on the tree of commit `c5ba337`, which has every fix above and no changed file: each printed 357 `RESULT` lines and no failure (dev KVM 1190 s, dev TCG 1256 s, release KVM 1169 s, release TCG 1281 s; [records/milestone-2-matrix-review.txt](records/milestone-2-matrix-review.txt)). The record names that tree by the ID it had before the documents were moved out of the repository's history, `50e9eea`; the files outside `docs/` are the same.

**Not checked:** whether a triple fault under `-no-reboot` could look like the harness ending the VM; whether firmware leaves `CR4` bits set that the kernel keeps; whether one scroll of the screen takes longer than a tick under software emulation. No test starts 65 programs. Nothing was run on hardware.

## 12. What to review hardest

The five conditions most likely to hurt that a test must pin, each with the test that owns it:

1. **A second process's kernel stack is corrupted when `RSP0` is stale.** `m2` has three processes making system calls across hundreds of switches; `sched` tests cannot see this. Owner: step I, test `m2` (ticks consecutive, no panic) in release and dev.
2. **Memory is not returned when a process ends.** Owner: steps J and K, the `REAPED frames=` figure and the constant sum of `free`, `kheap` and process pages in `m2`.
3. **A user pointer reaches the kernel unchecked on one path.** Every call that takes a pointer goes through `mm::usercopy`. Owners: the `usercopy` host tests (step I); `badptr` (step L) for the read direction; and for the write direction the inspector's `diag_read` into its own read-only text (step K), which `m2` requires to print `rotext=INVALID_ARGS`.
4. **An interrupt gate ring 3 can call.** Owner: `edge`'s `int 0x20` (step L) and the `x86` gate tests.
5. **A program image the validator should refuse is started.** Owner: `user_policy` tests (step B), `bad-elf` in `m2-prober` (step L), `m2-unit`.

## 13. Coverage check against the design

| Requirement | Where it is met |
|---|---|
| [06](06-milestones.md) "Depends on" chain, in order | Steps D → E → F → G → H → I → J → K |
| Deliverables: subsystems `arch`, `mm`, `sched`, `proc`, `diag`, `syscall`, `fs::bootfs` | Section 3; steps D–K |
| Self-test options `test=kwrite_text`, `test=kexec_data` | Step 14 of section 7; `m2-kwrite`, `m2-kexec` |
| Boot line saying whether the TSC is invariant (A-17) | `SK:KERNEL:CPU … tsc_invariant=` |
| Framebuffer console with Spleen 8×16 and its licence in `assets/` and the boot filesystem | Step F; `licenses/spleen.txt` in the manifest; `m2-screen` |
| Loader loads `BOOTFS.IMG`; `xtask` packs name, display name, class, limit, flags | 5.1–5.4; steps B, C |
| `abi`, `libstill`, `user/user.ld` | Steps A, B |
| `ticker`, `faulter`, `prober`, `inspector`; faults by `asm!` | Section 9 |
| Pass criteria 1–7 | Section 10, named there |
| Every probe of 06's table | Section 9, plus `edge` |
| [03 §5.1](03-kernel-and-interfaces.md#51-faults) spurious interrupts `0x27`, `0x2F`, `0xFF` | Step G, in `arch::pic`; not tested (no way to raise one in the VM) |
| [03 §5.2](03-kernel-and-interfaces.md#52-paging-and-allocation) fallible allocation inside system calls | Steps I–K use `try_reserve`; `memcap` and `alloc_run` tests |
| [04](04-security-and-permissions.md) I1, I2, I7 and the Milestone 2 row of §9 | Criteria 1, 5 (`handles`), 3–4 |
| D-S-3: register tier kept apart, off by default | 8.2, 8.4; `registers=ACCESS_DENIED` in `m2` |

Not covered by any test: the TSC clock, which is not implemented (section 11.5); x2APIC mode if the VM starts in xAPIC mode; spurious interrupts; I8.

## 14. Assumptions this milestone verifies

| # | Assumption | Check, and fallback |
|---|---|---|
| A-17 | Invariant TSC | Logged at step 10. Not verified: the clock counts ticks in every case in this milestone (section 11.5) |
| A-18 | `asm!` for `x86_64-unknown-none` accepts `hlt`, `cli`, `in`, `out`, `iretq`, `lgdt`, `ltr`, `wrmsr`, `rdmsr`, `invlpg`; `clac`, `xorps` and `fninit` may be refused | First build of step D and of step L; fallback is `.byte` (already specified for the two vector instructions; `clac` is `0x0f, 0x01, 0xca`) |
| A-19 | `-cpu max` reports SMEP and SMAP under TCG; under KVM it depends on the host CPU | Logged at step 10. Every test passes with either value; `badptr`'s `ac` case is only meaningful with SMAP |
| A-20 | OVMF leaves the local APIC in xAPIC mode at `0xFEE0_0000` | Logged at step 16; both modes are implemented |
| A-21 | Spleen 2.2.0 can be fetched from its repository, and the download is acceptable | `tools/fetch-font.sh`, run once on purpose. Fallback: place the two files in `assets/` by hand |
| A-22 | The 8254 interrupt reaches the CPU through LINT0 under KVM's in-kernel interrupt controller | `TIMER_OK` under `--accel kvm` in step G |

## 15. Not in this milestone

Channels, `init`, `permd`, files, grants and the audit log (Milestone 3). `process_spawn`, `wait`, `Process` handles. Serial input. The keyboard, surfaces and the desktop (Milestone 4). An allocator in `libstill`. Hardware floating point in user space (D-K-5). The local APIC timer (stage 5-prep). Reading ACPI tables. More than one CPU. Reuse of boot-services memory (D-B-5). Release of the boot info block, which the kernel keeps reading.
