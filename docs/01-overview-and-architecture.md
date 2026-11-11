# 01 · Overview, architecture, repository and workflow

Stillkey is a private desktop workspace with a built-in Workbench. Apps start with no access, receive explicit access to individual resources, and that access can be inspected and revoked from the same screen that shows what each app is doing. This document set is the design and build plan. It covers development in a virtual machine; work on real hardware is planned in [08](08-hardware-and-dual-boot.md) and deliberately not started.

Every technical claim is tagged by where it comes from:

- **Confirmed** — a stated requirement of the project, or read from an official specification or tool's documentation ([09](09-references.md)).
- **Assumption** — believed true, not yet verified on the development machine or this hardware. Each has an ID (A-nn) and a way to check it.
- **Decision** — a choice made in this design, recorded with the alternative that was considered. Each has an ID (D-…).

## 1. Requirements

| ID | Requirement |
|---|---|
| R-01 | Develop from a Windows host; first guest target is x86-64 UEFI in QEMU with OVMF and disposable disk images |
| R-02 | We write the UEFI loader, kernel, user-space interfaces, shell and desktop; standard firmware, compilers, debuggers and documented libraries may be reused and must be explained |
| R-03 | Freestanding Rust; loader for `x86_64-unknown-uefi`, kernel for `x86_64-unknown-none` |
| R-04 | A versioned boot information structure; a specified handoff including the memory map, framebuffer, the `ExitBootServices` sequence with stale-key retry, and the entry calling convention |
| R-05 | Untrusted apps; access denied by default; per-process handles with explicit rights; defined grant, revoke, duplicate and in-progress-read behaviour; enforcement at the kernel or a trusted service |
| R-06 | A Workbench with a narrow diagnostics interface showing processes, memory, permissions and recent crashes |
| R-07 | Three connected screens: desktop and launcher, Workbench inspector, file-access request naming app, file and action |
| R-08 | Near-black glass look with cyan accent, red and amber for errors and warnings; keyboard navigation, clear focus, high contrast, reduced motion; simple framebuffer rendering first |
| R-09 | A small read-only test filesystem first |
| R-10 | Networking, GPU acceleration, broad hardware support, third-party app compatibility and production-grade security are deferred |
| R-11 | Five milestones with acceptance tests; Windows Boot Manager, its EFI files and recovery partitions preserved; read-only checks first on hardware |
| R-12 | A desktop assistant in Milestone 4 (added 2026-11-11): an AI helper that the person gives access to whatever they choose, app by app and file by file, and that then operates those things for them |

## 2. Assumptions to verify

| ID | Assumption | How and when it is checked |
|---|---|---|
| A-01 | The eventual hardware target is x86-64 with UEFI firmware | Read-only host report, stage 5a of Milestone 5, before any hardware test ([08 §2](08-hardware-and-dual-boot.md#2-what-we-need-to-know-about-the-machine-first)) |
| A-02 | The WSL2 distribution `Ubuntu-26.04` on the development machine can be used for builds. Checked read-only on 2026-11-09: Ubuntu 26.04.1, `/mnt/d/OS` reachable; Rust, a C linker, QEMU, OVMF, `mtools` and GDB are **not installed yet** | `tools/setup-wsl.sh`, first step of Milestone 1 ([07 §2](07-milestone-1-spec.md#2-one-time-setup)). **Result, 2026-11-10: holds**; the setup script has installed them |
| A-03 | KVM is usable inside WSL2. Checked: `/dev/kvm` exists (`root:kvm`, mode 660) and the user is not yet in the `kvm` group | The setup script adds the group; `xtask setup-check` confirms. If KVM is not usable, `xtask` selects software emulation and logs `accel=tcg`; QEMU is never left to fall back by itself. Software emulation alone is then sufficient to accept Milestones 1–4, and the test summary says `accel=tcg only`. **Result, 2026-11-10: holds.** The user is in the `kvm` group and the Milestone 1 acceptance runs use both accelerators |
| A-04 | Observed: `apt` on the development machine offers QEMU 10.2.1 and OVMF 2025.11 ([07 §2](07-milestone-1-spec.md#2-one-time-setup)). Assumed: once installed, the firmware files are `/usr/share/OVMF/OVMF_CODE_4M.fd` and `OVMF_VARS_4M.fd` (paths taken from the package file list; the package is not installed yet) and they boot this image | `xtask setup-check` records versions and file hashes; `m1-boot` boots with these files. **Result, 2026-11-10: holds**; the package is installed and both files are at these paths |
| A-05 | Rust 1.99.1 (the current stable release) builds the loader, kernel and the `sha2` crate for both soft-float targets | First build in Milestone 1. **Result, 2026-11-10: holds** |
| A-06 | The `uefi` crate (0.41) exposes the raw system-table pointer needed for our own exit loop | Checked against its documentation when the loader is written; fallback is the crate's own exit helper. **Result, 2026-11-10: holds** (`uefi::table::system_table_raw()`) |
| A-07 | Read in the OVMF and QEMU sources, not yet observed on the development machine: OVMF's default display is 1280×800, 32-bit, blue-green-red order | The loader reads and logs the actual mode. **Result, 2026-11-10: holds** (`GOP mode=1280x800 … format=bgrx`) |
| A-08 | The linker accepts `/Brepro` (deterministic PE timestamp) plus `/DEBUG:NONE` (no PDB and no CodeView record, overriding the `/DEBUG` that `rustc` always passes for this target), so the loader binary is byte-identical across builds | Milestone 1 pass criterion 6 (`m1-repro`). **Result, 2026-11-10: holds** |
| A-09 | Building from `/mnt/d/OS` with build output kept on the Linux filesystem is fast enough | Measured in Milestone 1; alternative in D-W-2 |
| A-10 | QEMU's window works acceptably under WSLg for interactive desktop use (neither vendor documents the combination) | Milestone 4; fallback is screenshots or a VNC viewer |
| A-11 | Exception logging with QEMU's `-d int` needs software emulation, not KVM (from memory, not found in documentation) | Debug runs with `--trace` always use software emulation |
| A-12 | GDB cannot use the loader's symbols, which are in PDB format; it can use the kernel's DWARF symbols (from memory) | Loader is debugged by its serial log |
| A-13 | The eventual hardware has a PS/2-style built-in keyboard | Hardware inventory, stage 5a of Milestone 5: Device Manager › Keyboards ([08 §2](08-hardware-and-dual-boot.md#2-what-we-need-to-know-about-the-machine-first)) |
| A-14 | QEMU 10.2.1 accepts `-device VGA` and `virtio-blk-pci` under `-nodefaults`, and OVMF with a fresh variables file starts `\EFI\BOOT\BOOTX64.EFI` from the disk | Step B of [07 §10](07-milestone-1-spec.md#10-implementation-order). **Result, 2026-11-10: holds** |
| A-15 | OVMF mirrors its text console to COM1, and this VM's memory map tops out below 4 GiB (direct map = 4 GiB) | `m1-boot`. **Result, 2026-11-10: holds** (`firmware_lines=2`, `direct_map_gib=4`) |
| A-16 | `rust-lld`'s defaults for `x86_64-unknown-uefi` give `NX_COMPAT`, 4 KiB section alignment and no writable-and-executable section | `m1-unit`. **Result, 2026-11-10: holds**, in both profiles |
| A-17 | The CPU has an invariant TSC (CPUID leaf `0x8000_0007`, EDX bit 8), which `clock()` relies on ([03 §5.3](03-kernel-and-interfaces.md#53-scheduling)) | The kernel logs the bit at Milestone 2 boot; without it `clock()` falls back to counting timer ticks. Expected in the VM (read in QEMU 10.2.1's CPU model source, not found in its documentation and not yet observed on the development machine): `-cpu max` does not report the bit under either accelerator, so the VM tests run on the fallback and the TSC path is first exercised on hardware |
| A-18 | The assembler behind `asm!` for `x86_64-unknown-none` accepts the privileged instructions the kernel needs; it may refuse `clac`, `xorps` and `fninit` for a soft-float target | First builds of Milestone 2 ([10 §14](10-milestone-2-spec.md#14-assumptions-this-milestone-verifies)); the fallback is the instruction's bytes |
| A-19 | `-cpu max` reports SMEP and SMAP under software emulation; under KVM that depends on the host CPU | The kernel logs both bits at Milestone 2 boot; every test passes with either value |
| A-20 | OVMF leaves the local APIC in xAPIC mode at `0xFEE0_0000` | Logged at Milestone 2 boot; x2APIC mode is implemented as well |
| A-21 | Spleen 2.2.0 can be fetched once from its repository | `tools/fetch-font.sh` checks the SHA-256 of both files; the fallback is to place them in `assets/` by hand |
| A-22 | The 8254's interrupt reaches the CPU through LINT0 under KVM's in-kernel interrupt controller | `SK:KERNEL:TIMER_OK` under `--accel kvm` |
| A-23 | QEMU can give the guest a second 16550 (COM2, ports `0x2F8`–`0x2FF`, IRQ 3) joined to a host socket, under KVM and under software emulation, and bytes pass in both directions without loss at the sizes the model link uses | Stage 4b: `m4-assist` exchanges every scripted message; the fallback is a framed protocol with acknowledgements |

## 3. Design decisions

| ID | Decision | Alternative |
|---|---|---|
| **Workflow** | | |
| D-W-1 | One environment: WSL2 Ubuntu 26.04 for compiling, building images, running QEMU and debugging, driven from Windows by a wrapper script | Native Windows (needs Visual Studio Build Tools or the GNU host toolchain for host-side Rust code, plus QEMU for Windows) |
| D-W-2 | Source stays in `D:\OS` so Windows tools can edit it; Cargo's build directory, disk images and firmware-variable copies live on the Linux filesystem under `~/.cache/stillkey`; `serial.log`, `qemu.log` and the two binaries are copied back to `D:\OS\build` | Move the whole repository into the Linux filesystem (Microsoft's general recommendation for Linux tools) and reach it from Windows through `\\wsl.localhost\` |
| D-W-3 | Stable Rust pinned to 1.99.1, edition 2024, no nightly features. Consequences: no custom target files, no `build-std`, interrupt stubs written as naked functions | Nightly, for `extern "x86-interrupt"` and custom targets |
| D-W-4 | Disk images built with `sgdisk` and `mtools` (no root needed) | Pure-Rust image building in `xtask` (`gpt` and `fatfs` crates), which would also work on native Windows |
| D-W-5 | A milestone is accepted after 20 consecutive passes in both dev and release builds. Milestones 1–4 run each of them on every usable accelerator: KVM and software emulation when `/dev/kvm` is usable, software emulation alone otherwise, in which case the test summary says `accel=tcg only` | A different count |
| D-W-6 | The VM uses `-cpu max` (every feature the accelerator offers), 256 MiB of memory and one processor | A named CPU model, for identical features under KVM and software emulation |
| D-W-7 | From Milestone 2 a scenario run ends when the harness has seen every line it requires: it follows the serial log, then tells QEMU to quit through the monitor, and the expected status is 0 ([10 §6](10-milestone-2-spec.md#6-vm-launch-and-how-a-run-ends)) | The guest ends the VM |
| D-W-8 | `cargo xtask test m2` is the whole suite of Milestone 2, as `m1` is for Milestone 1, and `cargo xtask test all` runs both. The crash-containment scenario alone is `cargo xtask test m2-crash` ([10 §11.4](10-milestone-2-spec.md#114-checkpoint-3-result-2026-11-11)) | One word, `m2`, for both the scenario and the suite |
| D-W-9 | The operator shell is also tested by the tool (`m3-shell`), through a serial socket the harness writes to; D-K-10 stands for every other test ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | Leave the shell to a person |
| D-W-10 | Milestone 3 adds the tests `m3-unit`, `m3-initexit` and `m3-shell`; `abi_version()` returns 3 ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | Only the four scenarios of 06 |
| **Boot** | | |
| D-B-1 | Kernel is a fixed-address static executable in the top 2 GiB; no relocation, no address randomisation | Position-independent kernel with relocation in the loader |
| D-B-2 | The loader implements the `GetMemoryMap`/`ExitBootServices` loop itself on the raw service table: bounded retries and our own diagnostics. The `uefi` crate's helper retries once and resets the machine on failure | Call the crate's helper |
| D-B-3 | The loader uses no heap: every allocation is an explicit `AllocatePages`, so nothing can change the memory map behind our back | Use the crate's pool allocator and collections |
| D-B-4 | `BootInfo` carries a Stillkey-defined region list and an explicit list of reserved ranges, not raw UEFI descriptors or custom UEFI memory types (the `uefi` crate documents that some firmware mishandles custom types) | Pass the raw map |
| D-B-5 | Boot-services memory is not reused in Milestones 1–4 although the specification allows it | Reclaim it once the kernel has its own GDT and IDT |
| D-B-6 | We write the ELF validator and the 16550 serial driver ourselves; SHA-256 comes from the `sha2` crate | Use `elf`/`xmas-elf` and `uart_16550` crates; or write SHA-256 ourselves |
| D-B-7 | Milestone 1 keeps the firmware's graphics mode | Select a mode in the loader |
| **Kernel** | | |
| D-K-1 | Small capability kernel; boot filesystem and a keyboard interrupt handler stay in the kernel until writable storage; all later drivers are user-space processes | Monolithic kernel; or pure microkernel from the start |
| D-K-2 | One CPU through Milestone 4 | Multiprocessor earlier |
| D-K-3 | Legacy interrupt controller and timer in the VM milestones; local APIC timer and ACPI interrupt table before hardware | APIC from the start |
| D-K-4 | System calls by `int 0x80` first; `SYSCALL`/`SYSRET` later behind the same ABI | `SYSCALL` from the start |
| D-K-5 | No hardware floating point or SIMD in user space until the kernel saves vector state per thread. Firmware hands over with x87 and SSE enabled, so the kernel switches them off before the first entry to ring 3; an app that uses them is ended with a crash record, and user programs are built soft-float | Save and restore vector state per thread from Milestone 2, which lets user programs use hardware floating point and SIMD |
| D-K-6 | 16 MiB memory limit per App-class process; a System-class program's limit is the figure in its boot filesystem table entry, written by `xtask` (0 means the same 16 MiB; `desktop`: 64 MiB, or 8 × the framebuffer size if that is larger) | Another figure, or none |
| D-K-7 | Kernel code is not pre-emptible | Pre-emptible kernel with locks |
| D-K-8 | Our own thin wrappers for privileged instructions and tables | The `x86_64` crate with default features off |
| D-K-9 | With SMAP on, the kernel copies to and from user memory through the direct map after walking the caller's page tables; it never dereferences a user address and never executes `stac` | Access user addresses directly inside a `stac`/`clac` window, with fault recovery in the copy routine |
| D-K-10 | Operator input in Milestone 3 is a `Console` object (the receive side of the serial port) read with `console_read`; the automated test uses a scripted policy, because its serial port is captured to a file and carries no input | No serial input: scripted policies only until the desktop exists |
| D-K-11 | Boot option `stop=m1`: the kernel ends after the Milestone 1 steps as it did in Milestone 1; without it, it continues into Milestone 2. The Milestone 1 tests pass the option and boot images without `BOOTFS.IMG` ([10 §2](10-milestone-2-spec.md#2-decisions-this-specification-adds)) | The harness stops the VM at `SK:KERNEL:M1_OK`, and the status-33 check goes |
| D-K-12 | Kernel logic that needs no privileged instruction lives in host-tested library crates: `stillkey-abi`, `sk-bootfs`, `sk-kcore` | Everything in the kernel binary, tested only by booting |
| D-K-13 | The kernel does not hash program images in Milestones 2–4; it takes each digest from the boot filesystem table, and the loader hashes the whole `BOOTFS.IMG` for the harness to compare | The kernel verifies each image when it starts it |
| D-K-14 | User address layout: image below `0x8000_0000`, `mem_alloc` areas from `0x1_0000_0000` at the lowest free address, a 64 KiB stack ending at `0x7FFF_FFFF_F000`; kernel thread stacks of 32 KiB in 64 KiB slots | Other bases and sizes |
| D-K-15 | An unknown system-call number returns `INVALID_ARGS` | A separate `NOT_SUPPORTED` status |
| D-K-16 | `diag_read` copies fixed-size records, at most 4096 bytes per call, with a cursor and a change counter | Variable-length text |
| D-K-17 | An ended process stays in the process table, with its kernel stack, for the rest of the boot; at most 64 processes per boot in Milestone 2 | Remove ended processes after a while |
| D-K-18 | In Milestone 2 a program's first handle is always `0x1000`; from Milestone 3 handles arrive on the bootstrap channel | Pass the value in a register at entry |
| D-K-19 | Freed frames are filled with the byte `0xFA` and a frame is zeroed when handed out, so a page table used after it was freed stops the boot at once ([10 §8.3](10-milestone-2-spec.md#83-process-start)) | No fill |
| D-K-20 | From Milestone 3 `init` starts the Milestone 2 scenarios too; the kernel's start table is removed. Their PIDs are one higher and each program starts with `handles=1` ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | A frozen Milestone 2 mode in the kernel behind a boot option |
| D-K-21 | Kernel objects live in typed slabs, named by index and generation; a slab grows with `try_reserve` and a failure is `NO_MEMORY` ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | `Arc` with unstable fallible constructors |
| D-K-22 | Kernel memory is charged in bytes against the process's limit: 4096 per handle-table page, 512 per channel, 64 + length + 24 per handle for a queued message, 256 per grant ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | Other figures |
| D-K-23 | A message cannot carry an end of the channel it is written to, nor a channel end that has messages waiting (`INVALID_ARGS`), so no channel end can be lost while it holds memory ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | A collector for unreachable channel ends |
| D-K-24 | `channel_read` into a buffer that is too small returns `INVALID_ARGS` with the sizes needed in `RDX`; the message stays queued ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | A separate call that asks for the size |
| D-K-25 | A process's bootstrap handle is always the value `0x1000`; D-K-18 is narrowed to this one handle ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | Pass the value in a register at entry |
| D-K-26 | A blocked system call starts again from its first check when a signal it waits for changes or its deadline passes; a wait covers at most 16 objects ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | Wait queues per object |
| D-K-27 | Under a `test=` option a program may send `init` a test event on its bootstrap channel, which drives the scripted scenarios; without the option `init` ignores it ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | Scripts driven by time |
| D-K-28 | The kernel keeps a record of the last 256 grants; with 256 Active grants `grant_create` returns `NO_MEMORY` ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | Keep every record |
| **Security** | | |
| D-S-1 | File grants cannot be duplicated or transferred by default | Allow delegation (revocation still follows the grant) |
| D-S-2 | "Denied" and "no such file" are indistinguishable to apps: one status code, and the prompt is shown for any well-formed path with existence checked only after the decision, so the time to an answer does not reveal it either | Distinct codes |
| D-S-3 | The Workbench shows crash summaries by default. Crash records keep a register snapshot under a separate right, which exists only in debug mode (boot option `debug=registers`), because registers can hold app data; never memory ([04 §6](04-security-and-permissions.md#6-crash-records)) | No registers at all |
| D-S-4 | The Workbench is not trusted for granting access: it can view, ask the broker to revoke and ask `init` to end an app, nothing more. What it displays can be checked against the top bar's grant count and the serial audit log ([04 §7](04-security-and-permissions.md#7-workbench-diagnostics-interface)) | Give the Workbench administrative rights |
| D-S-5 | Milestones 3–4 use a confirmation prompt for a path the app names; a file chooser drawn by `desktop`, where the person's choice is the grant and the app names no path (the powerbox pattern), is the planned replacement ([04 §10](04-security-and-permissions.md#10-open-questions)) | Build the file chooser first |
| D-S-6 | In each second the first 8 refusals of a process each make a `denied` entry; further ones are added to its newest `denied` entry as a repeat count ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | One entry per process and reason, whatever the path |
| D-S-7 | `permd` takes at most 8 requests per second from one app for a decision and refuses more at once with `reason=busy` ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | Make test policies slow |
| D-S-8 | A grant whose `GrantControl` handle is closed by a running process is revoked with `reason=control-closed`; `broker-exit` is for a process that is ending ([11 §2](11-milestone-3-spec.md#2-decisions-this-specification-adds)) | One reason for both |
| **Interface** | | |
| D-UI-1 | Palette and tokens in [05 §2](05-ui-design.md#2-design-tokens) | Any |
| D-UI-2 | Spleen bitmap font first; Inter and JetBrains Mono (OFL, no reserved font name) when anti-aliased text arrives | IBM Plex (its reserved name means a pre-rasterized build must be renamed); Terminus |
| D-UI-3 | The Still key is `F12` | Another key |
| D-UI-4 | One visible app at a time in the Milestone 4 demo | Overlapping windows |
| D-UI-5 | 1280×800 reference canvas | Other |
| D-UI-6 | `desktop` reserves `F11` (launcher from anywhere) and `F9` (cycle apps; `Alt+Tab` too where the host passes it through) next to `F12`; `Ctrl+F12` switches Contrast+ ([05 §3](05-ui-design.md#3-screens)) | Other keys |
| D-UI-7 | The UI shows time since boot (`up 02:14`, `+02:11`) until a wall-clock source exists: no firmware time call and no `BootInfo` change | Read the firmware clock in the loader before `ExitBootServices` and pass it in a new `BootInfo` field |
| D-UI-8 | `desktop` reserves `F10` (hands off: every delegation an assistant holds is revoked at once) and `F8` (the access sheet of the assistant in front) ([05 §3.4](05-ui-design.md#34-assistant-stage-4b)) | Other keys; no hands-off key, and revoking one grant at a time in the Workbench |
| **Assistant (stage 4b)** | | |
| D-A-1 | The assistant is a program of class App. It starts with no access, and everything it does for the person goes through grants the person makes on the trusted sheet ([04 §11](04-security-and-permissions.md#11-delegation-to-an-assistant-stage-4b)) | A System-class assistant with standing access to everything |
| D-A-2 | What can be delegated: for each app Start, See, Type and End; for each file Read. Never: answering a request, the Workbench, the launcher, the reserved keys, the framebuffer, the keyboard itself, the model link, crash registers, another program's grants | Give the assistant the keyboard and the screen, which is simpler and lets it answer every prompt |
| D-A-3 | A delegation is a kernel `Grant` that forwards to a `Channel` end, so that the kernel ends it at once on a revoke (I6) and writes its audit entries, as for a file | `desktop` keeps a table of delegations and stops forwarding when told |
| D-A-4 | See gives the assistant a text outline of the app's surface (labels, values, focus), which the app's `ui` library publishes; no pixels in stage 4b | A picture of the surface: it is what the person sees, but it is large for the first link and needs a model that reads images |
| D-A-5 | The model is outside the machine. It is reached over one link (COM2 in the VM), which only `modeld` holds. The host side of the link is a bridge in `xtask` that holds the API key and speaks the provider's protocol; tests use a scripted model. On hardware there is no link until Stillkey has a network stack | A network stack and TLS inside Stillkey first; a model that runs on the machine |
| D-A-6 | A program that can reach `modeld` is marked Outside in the launcher, on every sheet for it and in the Workbench: what it is allowed to read can leave the machine | No mark |
| D-A-7 | One request of the assistant can carry up to 8 delegations. The sheet lists them, each with a tick the person can remove, and the person can open the same sheet without being asked (`F8`) | One sheet for each delegation |
| D-A-8 | A key event the assistant sends is marked as injected for the app that receives it, goes straight to that app's window channel, and is dropped while any sheet is open | Unmarked events through the same routing as real keys |
| D-A-9 | A delegation names an app, not one process of it, and lasts until it is revoked, until hands-off, or until the assistant ends. None is remembered across a restart of the system | "Allow once"; remembered delegations (these need writable storage) |
| **Hardware** | | |
| D-H-1 | Dual boot on a separate drive with its own EFI partition; no firmware boot entry at first | See [08 §7](08-hardware-and-dual-boot.md#7-stage-5d-dual-boot-design) |
| D-H-2 | Firmware trust route chosen at Milestone 5 between "Secure Boot off for testing" and "enrol our own certificate" | – |
| D-H-3 | Kernel verification by an Ed25519-signed manifest, public key built into the loader | Pin a kernel hash in the loader |
| D-H-4 | An external USB drive is preferred over an internal second drive; on hardware the loader shuts the machine down after a failure instead of returning to firmware, so firmware never goes on to start Windows in the same power-on | An internal second drive; return to firmware and let it try the next boot entry |

## 4. Boot-to-desktop architecture

```
 HOST: Windows 11                                           GUEST: QEMU (q35) + OVMF; later a real PC
 +----------------------------------------------+
 | D:\OS  source, docs, build\ (logs, binaries) |
 | tools\sk.ps1 --> wsl.exe --> Ubuntu 26.04    |
 |   tools/sk.sh --> cargo xtask                |
 |     build | image | run | test               | ---- disk image (GPT, FAT32 EFI partition) ------>
 |   rustc 1.99.1, rust-lld, QEMU 10.2, gdb     | <--- serial log, QEMU exit code, screenshots -----
 |   ~/.cache/stillkey: build dir, disk images  |
 +----------------------------------------------+

 power on
    |
    v
 +--------------------------------------------------------------------------------------------+
 | UEFI firmware (OVMF in the VM)                                               not ours      |
 |   finds \EFI\BOOT\BOOTX64.EFI on the EFI System Partition  [Secure Boot check: Milestone 5]|
 +--------------------------------------------------------------------------------------------+
    | calls efi_main (Microsoft x64 convention)
    v
 +--------------------------------------------------------------------------------------------+
 | Stillkey loader  BOOTX64.EFI   target x86_64-unknown-uefi                    ours          |
 |   check the CPU (NX, 4-level paging)  ->  read CMDLINE.TXT, KERNEL.ELF, BOOTFS.IMG         |
 |   validate ELF, SHA-256 [signature: M5]  ->  allocate, copy segments  ->  build page tables|
 |   fill BootInfo v1  ->  GetMemoryMap -> ExitBootServices (retry on stale key)              |
 |   ======= firmware boot services end here =======                                          |
 |   set EFER.NXE and CR0.WP, clear CR4.PGE  ->  our page tables and the kernel stack         |
 +--------------------------------------------------------------------------------------------+
    | jmp _start (System V convention, RDI = handoff block, RSI = its mapped length)
    v
 +--------------------------------------------------------------------------------------------+
 | Stillkey kernel  KERNEL.ELF    target x86_64-unknown-none, ring 0            ours          |
 |                                                                                            |
 |   arch  | mm          | sched    | proc      | object     | ipc      | fs      | diag      |
 |   GDT   | frames      | threads  | processes | handles    | channels | bootfs  | snapshots |
 |   IDT   | page tables | timer    | ELF load  | rights     |          | (read-  | crash ring|
 |   traps | heap        | waits    | crashes   | grants     |          |  only)  | audit log |
 |                                                                                            |
 |   syscall: the only entry from ring 3     drivers: serial out and in, keyboard IRQ, fbcon  |
 +--------------------------------------------------------------------------------------------+
    | starts init with one handle, its bootstrap channel. The first message on it carries the
    | boot command line and init's endowment: the boot filesystem root, Diag, Console and,
    | from Milestone 4, the framebuffer and Input objects
    v
 =========================================== ring 3 ===========================================
 +--------------------------------------------------------------------------------------------+
 | init   spawns every process and keeps the far end of each bootstrap channel; holds the     |
 |        filesystem root only to open program images and to endow permd; mints Diag handles  |
 +--------------------------------------------------------------------------------------------+
         |                          |                                         |
         v                          v                                         v
 +----------------+  +------------------------------+ prompt  +-------------------------------+
 | shell          |  | permd  permission broker     |<------->| desktop  owns framebuffer and |
 | operator shell |  | holds the filesystem root    | channel | Input: compositor, launcher,  |
 | on the serial  |  | creates and revokes grants   |         | trusted prompt, top bar,      |
 | Console, in    |  +------------------------------+         | Still key                     |
 | Milestone 3    |     ^ request      | Grant handle         +-------------------------------+
 +----------------+     | channel      v (READ only)             ^ window channel:        ^
                     +------------------------------+            | surfaces, key events   |
                     | apps: notes, ticker, ...     |------------+                        |
                     | start with one handle, the   |         +-------------------------------+
                     | bootstrap channel to init    |         | workbench  Diag (inspect),    |
                     +------------------------------+         | grant-admin channel to permd, |
                            untrusted                         | "end app" channel to init     |
                                                              +-------------------------------+
                                                      privileged viewer, not trusted for grants
```

Milestones reach this picture from the top down: Milestone 1 ends at the kernel box; 2 adds processes, which the kernel starts itself, and a text `inspector` that holds the `Diag` handle; 3 adds `init`, `permd`, the `shell`, handles and grants, and from then on the kernel starts only `init`; 4 adds `desktop` and the graphical Workbench, which take the shell's place.

Who holds what in ring 3 (details in [03 §7](03-kernel-and-interfaces.md#7-user-space-pieces-we-write) and [04](04-security-and-permissions.md)):

| Process | Class | What it holds |
|---|---|---|
| `init` | System | Everything the kernel hands to user space. It keeps the filesystem root only to open program images and to give `permd` its root, and never creates a grant. It mints the `Diag` handles: `WRITE` for `permd`, `INSPECT` for `shell` (Milestone 3), `desktop` and the Workbench, and `INSPECT_REGISTERS` for the Workbench only and only under the boot option `debug=registers` (D-S-3). It keeps the far end of every bootstrap channel. It acts on the `test=` boot options that name a program to start, and it forwards `test=` to `permd`, and `ui.*` and `debug=` to `desktop`; no other process can read boot options |
| `permd` | System | A filesystem root without the right to start programs; it is the only component that creates grants. One request channel per app, and one prompt channel shared with `desktop` (with `shell` in Milestone 3) |
| `shell` | System | Milestone 3 only. The only `Console` handle: operator input on the serial port (D-K-10). `Diag` with `INSPECT`, for `ps` and `audit`. The deciding end of the prompt channel and a grant-admin channel to `permd`; from Milestone 4 these two belong to `desktop` and the Workbench |
| `desktop` | System | The framebuffer and the only `Input` handle. A control channel to `init`, on which it asks for an app to be started (`launch <registry id>`), and one window channel per app |
| `workbench` | System | `Diag` for viewing, a grant-admin channel to `permd` (list grants, list waiting requests, revoke) and an "end app" channel to `init`, in addition to what every windowed program has |
| Apps | App | From Milestone 3, exactly one handle at start: the bootstrap channel. Over it `init` sends the app's end of a request channel to `permd` and, from Milestone 4, of a window channel to `desktop`; the matching server ends go to `permd` and `desktop` with the app's PID and registry id. File access arrives only as a `Grant` handle from `permd` |

Only `init` and `permd` ever hold a directory handle. A program's name, class and memory limit come from its entry in the boot filesystem table, which `xtask` writes; the kernel never matches a program by name. If `permd` ends, the kernel revokes its grants and `init` restarts it and hands out fresh channel ends; if `init` or `desktop` ends, the kernel stops with a panic screen.

## 5. Proposed repository layout

```
D:\OS
├─ README.md
├─ rust-toolchain.toml          pinned toolchain, its components and the two targets
├─ Cargo.toml  Cargo.lock       one workspace; default member is xtask only
├─ .cargo/config.toml           the xtask alias; compiler and linker flags per target
├─ .gitattributes  .gitignore   LF line endings (CRLF for *.ps1); build/ and target/ ignored
├─ docs/                        this plan; decisions recorded as they change
├─ boot/
│  ├─ bootinfo/                 stillkey-bootinfo: handoff ABI shared by loader and kernel, and the
│  │                            memory-map conversion; unit-tested on the host  (no_std library)
│  └─ loader/                   UEFI loader -> BOOTX64.EFI                     (x86_64-unknown-uefi)
├─ kernel/                      kernel -> KERNEL.ELF, with linker.ld            (x86_64-unknown-none)
├─ lib/
│  ├─ sk-elf/                   strict ELF64 validator, unit-tested on the host (no_std library)
│  ├─ sk-serial/                16550 driver used by loader and kernel          (no_std library)
│  ├─ sk-bootfs/                BOOTFS.IMG format: parser and packer            (from Milestone 2)
│  ├─ sk-kcore/                 kernel logic without privileged instructions    (from Milestone 2)
│  └─ abi/                      system-call numbers, status codes, rights       (from Milestone 2)
├─ user/                        programs for ring 3                             (from Milestone 2)
│  ├─ user.ld                   linker script shared by every user program (base 0x40_0000)
│  ├─ libstill/                 user-space runtime
│  ├─ inspector/                text inspector                                  (Milestone 2 only)
│  ├─ init/  permd/  shell/     trusted services                                (Milestone 3)
│  ├─ ui/  desktop/  workbench/ interface                                       (Milestone 4)
│  └─ samples/                  ticker, faulter, prober (M2); notes (M3); imposter (M4)
├─ bootfs/                      contents of the read-only filesystem            (from Milestone 2)
├─ assets/                      fonts with their licence files: the 8×16 console font from
│  │                            Milestone 2, the UI sizes in Milestone 4
│  └─ icons/                    our own 1-bit icons                             (Milestone 4)
├─ xtask/                       host build tool: setup-check, build, image, run, test, clean
├─ tools/                       setup-wsl.sh, sk.sh, sk.ps1, gdb scripts; host-report.ps1 and
│                               write-usb.ps1 (Milestone 5)
├─ tests/                       scenario definitions and expected marker lists
└─ build/                       generated: logs, the two binaries, screenshots  (ignored by git)
```

Notes:

- The loader and the kernel cannot be built for the host, so plain `cargo build` builds only `xtask`, and `xtask` invokes Cargo once per target (Cargo's per-package targets and artifact dependencies are still unstable). The pure `no_std` libraries `sk-elf` and `stillkey-bootinfo` also compile for the host, which is how their unit tests run. The loader has no library target, so everything in the boot path that is worth a unit test lives in those two crates.
- User programs are built for `x86_64-unknown-none` (soft-float) with the shared linker script `user/user.ld` (base `0x40_0000`).
- `build/` receives only what a person or a Windows tool needs to read: the two binaries of the last build in `build/artifacts/<dev|release>/`; `serial.log`, `qemu.log` and the two binaries of each test in `build/<test>/` (`build/run/` for `cargo xtask run`); and later screenshots and the Milestone 5 host report. Everything else that is generated stays on the Linux filesystem (section 6).
- The project is a git repository. `.gitattributes` is part of the first commit: it keeps shell scripts and expected-output files on LF line endings when a checkout is made from the Windows side.

## 6. Development workflow

**Recommendation: do everything inside WSL2 Ubuntu 26.04, started from Windows** (D-W-1). Reasons: one set of tools at versions the distribution pins; no Visual Studio Build Tools needed (on a Windows host they are required just to link host-side Rust code such as build scripts, procedural macros and `xtask`); QEMU, OVMF, GDB, `sgdisk` and `mtools` are ordinary packages; KVM is available; and the development machine already has the distribution installed.

One-time setup (Milestone 1 delivers the script):

```
wsl.exe -d Ubuntu-26.04 --cd /mnt/d/OS -- bash tools/setup-wsl.sh
```

It installs `build-essential qemu-system-x86 ovmf mtools gdisk gdb curl ca-certificates`; installs `rustup` with the minimal profile and no default toolchain, then runs `rustup toolchain install` in the project directory, which installs exactly what `rust-toolchain.toml` pins; creates `Cargo.lock` if there is none yet; adds the user to the `kvm` group; and ends with `bash tools/sk.sh setup-check`, which prints every tool version and fails if anything is missing. It asks for the `sudo` password, changes only the WSL distribution, and changes nothing in Windows. The exact steps are in [07 §2](07-milestone-1-spec.md#2-one-time-setup).

Daily use from PowerShell, through a one-line wrapper that passes its arguments unchanged to `tools/sk.sh` in a login shell in the project directory:

```
.\tools\sk.ps1 test m1             # = wsl.exe -d Ubuntu-26.04 --cd /mnt/d/OS --exec bash -l tools/sk.sh test m1
.\tools\sk.ps1 run                 # boot interactively, serial on the terminal
.\tools\sk.ps1 run --gdb           # boot paused, with QEMU's GDB stub listening on 127.0.0.1:1234
```

`tools/sk.sh` sets `CARGO_TARGET_DIR` to `~/.cache/stillkey/target` unless it is already set, then runs `cargo xtask` with the same arguments; inside WSL it is called directly. `xtask` has six commands: `setup-check`, `build`, `image`, `run`, `test` and `clean`. `run` and `test` take `--accel kvm|tcg`. The default is `kvm` when `/dev/kvm` is usable by the user and `tcg` (software emulation) otherwise; QEMU is given exactly one accelerator and the harness logs `accel=<x>`, so a run never changes accelerator silently. `run` also takes `--cmdline "<boot options>"`, which writes `CMDLINE.TXT` for that run so that any test scenario can be booted by hand, `--gui`, which opens a QEMU window instead of running with no display, and the debugging flags `--gdb`, `--monitor` and `--trace`. `image` takes `--cmdline` too. `test` also takes `--repeat N` and `--timeout SECONDS` (default 60). From Milestone 5, `image --hardware` builds the loader with its cargo feature `hardware`, for a real machine ([02 §11](02-boot-and-handoff.md#11-failure-behaviour)).

Cargo's target directory, disk images and firmware-variable copies are on the Linux filesystem under `~/.cache/stillkey/`. After each run `xtask` copies `serial.log`, `qemu.log` and the two binaries to `D:\OS\build\<test>\`, where Windows tools can read them.

What makes it reproducible:

| Element | Mechanism |
|---|---|
| Compiler | `rust-toolchain.toml` pins 1.99.1 with both targets; `Cargo.lock` is committed and every build uses `--locked` |
| Build flags | Set only in `.cargo/config.toml` and by `xtask`, which remaps source paths on the Cargo command line and removes `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` from the environment of every Cargo call, so a stray variable cannot replace them |
| Tools | `xtask setup-check` records QEMU, OVMF, GDB, `mtools`, `sgdisk` versions and the SHA-256 of the firmware files at the top of every test log, and refuses to run if a tool is missing |
| Firmware state | Every run starts from a fresh copy of `OVMF_VARS_4M.fd`, so there are never stored boot entries; firmware code is opened read-only |
| Disk | The image is rebuilt from scratch for every run and never reused |
| VM | Fixed machine type and memory size; CPU model `max`, which varies with accelerator and host (D-W-6), so every log names the accelerator; no network device; bounded by a timeout |
| Results | Decided by marker lines in the serial log and QEMU's exit code, never by a person watching |

Debugging tools: the serial log; QEMU's exception log (`run --trace`: `-d int,cpu_reset,guest_errors`, software emulation); GDB against the kernel's ELF symbols through QEMU's GDB stub (`run --gdb`); QEMU's monitor (`run --monitor`); a QEMU window that shows the framebuffer (`run --gui`); any test's boot options on an interactive run (`run --cmdline`). Details are in [07 §8](07-milestone-1-spec.md#8-debugging).

## 7. Dependencies: what each does, and what we own

| Dependency | What it does for us | Licence | Ours? |
|---|---|---|---|
| WSL2 with Ubuntu 26.04 | The Linux environment everything is built and tested in | Various, per package | No |
| OVMF (TianoCore EDK II), from Ubuntu's `ovmf` package | UEFI firmware for the VM | BSD-2-Clause-Patent | No. Used unmodified |
| QEMU 10.2 | The virtual PC, serial capture, exit device, GDB stub, screenshots | GPL-2.0 | No |
| GDB 17 | Kernel debugging | GPL-3.0 | No |
| `build-essential` (GCC as `cc`) | Links host-side Rust code: `xtask`, build scripts, procedural macros | GPL-3.0 | No |
| `rustup`, downloaded with `curl` and `ca-certificates` | Installs the pinned toolchain | MIT or Apache-2.0 | No |
| Rust 1.99.1: `rustc`, `cargo`, `rust-lld`, `core`, `alloc` | Compiler, build system, linker, the language's freestanding library | MIT or Apache-2.0 | No |
| `llvm-tools` toolchain component | `llvm-objdump` and `llvm-readobj`, for inspecting the two binaries by hand ([07 §8](07-milestone-1-spec.md#8-debugging)). No test depends on them | Apache-2.0 with LLVM exception | No |
| `sgdisk` (package `gdisk`), `mtools` | Create the GPT and the FAT32 partition contents without root | GPL | No |
| `timeout` (already present; on Ubuntu 26.04 it comes from the Rust implementation of coreutils, package `rust-coreutils`) | Ends a VM run that hangs. Its exit statuses 124 and 137 are the ones [07 §6](07-milestone-1-spec.md#6-vm-launch-command-and-exit-codes) relies on | MIT | No |
| `uefi` crate 0.41 (with `uefi-raw`, `uefi-macros`) | Typed Rust definitions of UEFI tables and protocols and the entry-point macro | MIT or Apache-2.0 | No. **The loader's logic is ours**: which files, validation, memory layout, page tables, exit loop, handoff |
| `sha2` crate 0.11 | SHA-256 in the loader (portable software backend) and in `xtask` | MIT or Apache-2.0 | No |
| Transitive crates (what `uefi` and `sha2` depend on) | Listed in `Cargo.lock`. `xtask setup-check` prints `cargo tree` for the loader and the kernel, so the full set inside each binary is on record | Per crate | No |
| `ed25519-dalek` 3.0 | Signature verification in the loader, Milestone 5 | BSD-3-Clause | No |
| Spleen font 2.2.0 | Bitmap glyphs: the 8×16 kernel console font from Milestone 2, the UI sizes in Milestone 4 | BSD-2-Clause | No. Licence text ships in the image |
| Inter 4.1, JetBrains Mono 2.304 | Anti-aliased UI text later | OFL 1.1 | No. Licence text and each font's copyright line ship in the image |

Everything else is ours: the loader; the `BootInfo` ABI and the memory-map conversion; the ELF validator; the serial driver; the kernel and every subsystem in it; the system-call ABI; the handle, rights and grant model; `libstill`; `init`, `permd`, the shell, the text inspector, the desktop, the Workbench and the samples; the boot filesystem format; the UI library, the theme and the 1-bit icons; `xtask` and the scripts in `tools/`.

Crates considered and not used at first, with the reason: `x86_64` (its default features need nightly; we need only a few instructions at first), `uart_16550` (forty lines we would rather own), `elf`/`xmas-elf`/`goblin` (we want a deliberately strict subset), `fatfs` and `gpt` inside the kernel (not usable without `std` on stable; no kernel filesystem driver is needed yet), `fontdue` (needs a heap and floating point; revisit in user space).

## 8. Document map

| # | Document | Contents |
|---|---|---|
| 01 | This file | Requirements, assumptions, decisions, architecture, repository, workflow, dependencies |
| 02 | [Boot and handoff](02-boot-and-handoff.md) | Loader steps, ELF validation, mappings, `BootInfo` v1, reserved memory, exit sequence, entry contract |
| 03 | [Kernel and interfaces](03-kernel-and-interfaces.md) | Subsystems, where drivers run, CPU-boundary code, system calls, input, graphics, persistence |
| 04 | [Security and permissions](04-security-and-permissions.md) | Threat model, handles and grants, revocation semantics, Workbench diagnostics, per-milestone properties |
| 05 | [Interface design](05-ui-design.md) | Visual identity, tokens, the three screens, flow, rendering stages, reduced motion |
| 06 | [Milestones](06-milestones.md) | Dependencies, deliverables, pass criteria, failure points for all five |
| 07 | [Milestone 1 specification](07-milestone-1-spec.md) | Exact files, build, VM command, expected serial output, debugging |
| 08 | [Hardware and dual boot](08-hardware-and-dual-boot.md) | Read-only checks, removable-media test, trust routes, dual-boot design, rollback |
| 09 | [References](09-references.md) | Specifications and documentation cited |
