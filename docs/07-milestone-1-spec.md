# 07 · Milestone 1 specification: Boot

This is the implementation contract for the next step. It fixes the files, the configuration, the build, the VM command, the serial output, the tests and the debugging procedure. The behaviour of the loader and the handoff is specified in [02](02-boot-and-handoff.md); this document does not repeat it, it says how to build and prove it.

Nothing in this milestone touches a physical disk, a firmware setting, Secure Boot, BitLocker or the Windows boot configuration. Everything runs inside the WSL2 distribution and inside QEMU.

## 0. Status (2026-11-10): implemented

The milestone was built in two parts, and both are done. The **first boot checkpoint** proved the complete handoff from our loader to our kernel under software emulation. The **second part** added what the checkpoint deferred: the framebuffer fill, the remaining tests, path remapping, KVM, the release profile and the repeat matrix. This document describes the milestone as it now stands.

| | First boot checkpoint | Second part |
|---|---|---|
| Loader | All twelve steps of [02 §1](02-boot-and-handoff.md#1-the-path-from-power-on-to-the-kernel), with the stale-key retry and with `CR4.PGE` cleared in step 11 | The graphics-mode check became the shared rule `Framebuffer::is_well_formed` |
| Kernel | Check of the pointer and mapped length, `BootInfo` validation, command line, image record, memory summary, lower-half removal, the deliberate panic | The framebuffer record is validated (reason `framebuffer`), the screen is filled (`fb.rs`, kernel step 5), `usable_after_reserve` subtracts the framebuffer, and the success marker is `SK:KERNEL:M1_OK attempts=<n>` |
| Tests | `m1-unit` (host tests), `m1-boot`, `m1-panic`, `m1-pge`, `m1-stalekey` | The artifact checks of `m1-unit` (`inspect.rs`), `m1-reject`, `m1-repro`, `m1-screen`, `m1-serial`, path remapping |
| Runs | Software emulation, dev profile, one run of each test | KVM and software emulation, dev and release, 20 repeats (section 9) |

The command, from PowerShell in `D:\OS`:

```powershell
.\tools\sk.ps1 test m1
```

Observed on the development machine on 2026-11-10, for each of `--accel kvm` and `--accel tcg`, with and without `--release`: host tests `sk-elf` 17, `stillkey-bootinfo` 77, `xtask` 32; both artifact checks pass; `m1-repro` passes; `m1-screen` passes on a 1280x800 screen; `m1-boot` status 33, `m1-panic` 37, `m1-pge` 33, the five `m1-reject` images 35 each, `m1-serial` 33, `m1-stalekey` 33. The serial logs are in `build\<test>\serial.log`. The acceptance matrix of section 9 passed on the same day on the final code: four commands, each with 208 result lines that pass and none that fails (section 9 has the counts and the artifact hashes).

Three things the first checkpoint changed in the design, each specified in [02](02-boot-and-handoff.md):

- **The kernel's entry takes two arguments.** `RDI` is the address of the handoff block and `RSI` the number of bytes mapped for it. The kernel validates against that length, not against the `total_size` the block reports about itself ([02 §6](02-boot-and-handoff.md#6-bootinfo-version-1), [02 §9](02-boot-and-handoff.md#9-the-jump-and-the-kernels-entry-contract)). `BootInfo` itself is unchanged: version 1.0, 256 bytes.
- **The loader clears `CR4.PGE`** after `ExitBootServices` and before it loads our page tables, and logs the value before and after ([02 §9](02-boot-and-handoff.md#9-the-jump-and-the-kernels-entry-contract)). This VM's firmware hands over with the bit already clear (`before=0x668`), so the test option `test=pge` sets it first and `m1-pge` proves the clearing.
- **Validation lives in `stillkey-bootinfo`**, which forbids `unsafe` code and is tested on the host. The kernel makes one slice of the mapped length and passes it in. The regression test `a_table_reaching_into_the_unmapped_page_is_rejected_without_a_fault` (`boot/bootinfo/tests/guard_page.rs`) gives the validator a block with a valid header and a memory-map table that ends beyond the mapped length. Every entry inside the block is valid, up to its last byte, and the next page has no access rights, so a reader that walked the entries would fault; the refusal can only come from the table's bound. `tests/bounds.rs` repeats this without the guard page for each table, and for a table that lies past `total_size` in memory that is mapped. Both files were checked against two deliberately weakened validators, which they fail.

Four things the second part added that the earlier text of this document did not have:

- **The framebuffer record has an acceptance rule** ([02 §6](02-boot-and-handoff.md#6-bootinfo-version-1)). The loader applies it before it hands a record over and the kernel applies it again, with one more condition: the pages mapped for the buffer are no page of a reserved range. A record that fails it makes the kernel refuse the handoff with `reason=framebuffer`.
- **Byte order and row layout are pure code** in `stillkey-bootinfo` (`boot/bootinfo/src/fb.rs`, tested by `tests/framebuffer.rs`). `kernel/src/fb.rs` only turns the record's address into a byte slice.
- **`m1-screen`** compares every pixel of QEMU's picture of the screen with what kernel step 5 paints. The `FB_FILLED` line alone would prove a log line, not pixels.
- **`m1-repro` also looks for host paths.** The two builds were byte-identical before path remapping existed. What the remapping changes is that neither binary names the workspace, the home directory or Cargo's home, so the test checks that too.

Each new test that passed the first time it ran was then run against a deliberately broken build, which it fails: `m1-reject` with a corruption that changes nothing; `m1-screen` with a band of 7 rows, with red and blue swapped, and with the last row unpainted.

**One defect was found by the repeat matrix and by nothing else.** In the first 20-repeat run of the release build under KVM, one `m1-reject` boot (run 8) ended with status 35 and without a single `SK:LOADER:` line. The cause: the loader's check that a serial port answers sends one byte in loopback mode and reads it back, and OVMF's console driver polls the same port for input from a timer event every 20 ms while boot services are active. When that event fell between the two steps, firmware took the byte, the loader concluded that there was no port, and it wrote nothing for the rest of the boot. This was confirmed before anything was changed: with a delay of 40,000 port reads inserted between the two steps, every loader line disappeared on every boot, while the kernel, whose check runs after `ExitBootServices`, kept all of its lines. The check now runs at `TPL_HIGH_LEVEL`, where no event is dispatched (section 7, loader step 1). The test `m1-serial` holds the window open for 50 ms: before the change it reported `recheck=lost` under both accelerators, and after it `recheck=ok`.

The review of the second part found one more thing that was fixed: `qemu.log` did not start with the tool versions and firmware hashes that section 5 promises. It does now, and `m1-boot` checks it.

Assumptions settled: A-03 (this user can open `/dev/kvm`, so all four matrix commands apply), A-05 (the loader's SHA-256 equals the host's), A-06 (the raw boot-services table is reachable through `uefi::table::system_table_raw()`, so the exit loop is our own), A-07 (`GOP mode=1280x800 … format=bgrx`), A-08 (two release builds in different directories give the same bytes), A-14 (the image boots), A-15 (`firmware_lines=2`, `direct_map_gib=4`) and A-16 (`nx_compat=yes section_alignment=4096 wx_sections=0`).

## 1. Outcome

One command, `cargo xtask test m1`, that:

1. builds the loader (`BOOTX64.EFI`) and the kernel (`KERNEL.ELF`) from a pinned toolchain;
2. creates a throw-away GPT disk image with a FAT32 EFI System Partition;
3. boots it in QEMU with OVMF, with no display and no network;
4. checks the serial log and QEMU's exit status against the expectations in section 9.

The same command runs the host-side unit tests, checks the headers of the two binaries, and proves that a second build produces the same bytes.

At the end of the milestone the kernel has been started by our own loader on our own page tables and stack, has checked the boot information, has summarised the memory it may use, has written to the framebuffer, and has stopped on purpose.

## 2. One-time setup

**State of the development machine before setup, checked read-only on 2026-11-09** (nothing was installed or changed):

| Item | Found |
|---|---|
| WSL2 distribution | `Ubuntu-26.04` (Ubuntu 26.04.1 LTS, systemd on) |
| `/dev/kvm` | Present, owner `root:kvm`, mode 660. The user is **not** in the `kvm` group yet |
| Already installed | `sgdisk`, `git`, `curl`, `python3`, `timeout` |
| Missing | Rust (`rustup`, `cargo`), a C linker (`cc`), QEMU, OVMF firmware files, `mtools`, GDB |
| Package versions offered by `apt` | QEMU 10.2.1, OVMF 2025.11, mtools 4.0.49, GDB 17.1 |

**Setup script** `tools/setup-wsl.sh` (run it yourself: it asks for the `sudo` password). From PowerShell:

```powershell
wsl.exe -d Ubuntu-26.04 --cd /mnt/d/OS -- bash tools/setup-wsl.sh
```

What it does, and nothing else. It starts with `set -euo pipefail`, so it stops at the first step that fails. A step that is already done is skipped, so a second run changes nothing and asks for no password. The two steps that need `sudo` come first, so the password is asked for once, at the start.

| Step | Command | Why |
|---|---|---|
| 1 | `sudo apt-get update && sudo apt-get install -y build-essential qemu-system-x86 ovmf mtools gdisk gdb curl ca-certificates`, only if `dpkg-query` reports one of them missing | Virtual machine, firmware, image tools (`mtools`, and `sgdisk` from `gdisk`), debugger, `curl` for step 3, and the C linker Cargo needs for host-side code (`xtask`, build scripts, procedural macros). `build-essential` also brings `readelf` |
| 2 | `sudo usermod -aG kvm "$USER"`, only if the user is not in the group | Lets QEMU use hardware virtualisation where `/dev/kvm` exists. The `kvm` group is part of the base system, so the command is harmless when the device is absent. Takes effect after `wsl.exe --terminate Ubuntu-26.04` |
| 3 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh -s -- -y --profile minimal --default-toolchain none`, only if `~/.cargo/bin/rustup` is absent | Installs `rustup` into `~/.cargo` and `~/.rustup`. No toolchain yet |
| 4 | `. "$HOME/.cargo/env"` | Puts `rustup` and `cargo` on `PATH` for the rest of the script. The installer only edits the shell start-up files, which a running script does not re-read |
| 5 | `rustup toolchain install` (no arguments, in the project directory) | Installs the toolchain, components and targets that `rust-toolchain.toml` pins |
| 6 | `[ -f Cargo.lock ] \|\| cargo generate-lockfile` | Creates the lock file once. Every later Cargo call uses `--locked` |
| 7 | `bash tools/sk.sh setup-check` | Prints every tool version and firmware hash, and fails if anything is missing. Going through `sk.sh` puts this first build on the Linux filesystem |

It changes nothing on the Windows side. The script was run on 2026-11-10 and `setup-check` ended with `setup-check: ok`.

After the script has finished, run `wsl.exe --terminate Ubuntu-26.04` once from PowerShell (it closes every open terminal of that distribution). The `kvm` group membership applies from the next start; `setup-check` then reports KVM as usable. Until then the default accelerator is software emulation.

**Wrappers.** `tools/sk.sh`, for use inside WSL:

```bash
#!/usr/bin/env bash
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/stillkey/target}"
exec cargo xtask "$@"
```

`tools/sk.ps1`, for PowerShell:

```powershell
wsl.exe -d Ubuntu-26.04 --cd /mnt/d/OS --exec bash -l tools/sk.sh @args
```

`@args` hands every argument on as it was typed and `--exec` starts `bash` directly, so an argument that contains spaces arrives intact. `bash -l` is a login shell, which is what puts `cargo` on `PATH`.

This document writes commands as `cargo xtask …`. Run them as `.\tools\sk.ps1 …` from PowerShell or `bash tools/sk.sh …` inside WSL; both keep Cargo's build directory on the Linux filesystem.

## 3. Files this milestone creates

```
D:\OS
├─ rust-toolchain.toml
├─ Cargo.toml                      workspace
├─ Cargo.lock                      committed; created once by the setup script
├─ .cargo/config.toml
├─ .gitignore                      build/, target/
├─ .gitattributes                  "* text=auto eol=lf" and "*.ps1 text eol=crlf"
├─ boot/bootinfo/                  crate stillkey-bootinfo   (no_std library, forbids unsafe code, tested on the host)
│  ├─ src/
│  │  ├─ lib.rs                    BootInfo v1 exactly as in 02 §6: constants, byte-exact encode and decode,
│  │  │                            block_layout (where each table lies in the handoff block)
│  │  ├─ convert.rs                UEFI descriptor bytes -> MemoryRegion, stepping by the descriptor size;
│  │  │                            in-place sort by base; overlap check; direct_map_size
│  │  ├─ validate.rs               check_block (pointer and mapped length), validate (the acceptance
│  │  │                            rules of 02 §6 over a byte slice of the mapped length) and
│  │  │                            usable_after_reserve (the subtraction of 02 §7)
│  │  └─ fb.rs                     what a Framebuffer record means: is_well_formed, the byte order of
│  │                               the two formats (pixel), and fill_rows
│  └─ tests/                       layout.rs, validate.rs, bounds.rs, guard_page.rs, framebuffer.rs,
│                                  common/mod.rs
├─ boot/loader/                    crate stillkey-loader     -> BOOTX64.EFI   (binary only, no library target)
│  └─ src/
│     ├─ main.rs                   entry point; the twelve steps of 02 §1 in order
│     ├─ cpu.rs                    the CPUID and CR4 checks of step 1; one-line wrappers that set EFER.NXE and
│     │                            CR0.WP and clear CR4.PGE
│     ├─ mem.rs                    page allocation; GetMemoryMap and ExitBootServices on the raw table
│     ├─ files.rs                  open the boot volume, read a whole file into allocated pages
│     ├─ load.rs                   allocate the image, copy segments
│     ├─ paging.rs                 page-table pool and the mappings of 02 §5
│     ├─ gop.rs  acpi.rs           capture graphics mode; find the ACPI table pointer
│     ├─ exit.rs                   the GetMemoryMap / ExitBootServices loop of 02 §8
│     ├─ jump.rs                   the final assembly block: mov cr3, mov rsp, xor ebp, jmp
│     └─ log.rs  qemu.rs  panic.rs
├─ kernel/                         crate stillkey-kernel     -> KERNEL.ELF
│  ├─ linker.ld
│  ├─ build.rs                     passes the linker script to the linker
│  └─ src/
│     ├─ main.rs                   _start(block: *const u8, block_len: u64), kmain
│     ├─ bootcheck.rs              check_block, the one slice over the handoff block, then validate
│     ├─ memsum.rs                 memory summary
│     ├─ lowmap.rs                 remove the lower-half mappings
│     ├─ fb.rs                     the one slice over the framebuffer, then fill_rows for band and ground
│     ├─ qemu.rs                   debug-exit port, only when asked by a boot option
│     └─ log.rs  panic.rs
├─ lib/sk-elf/                     strict ELF64 validator, rules V1–V10 of 02 §4, taking a policy
│                                  {window, max_file, max_mem}; tested on the host
├─ lib/sk-serial/                  16550 driver: init, presence check, blocking write
├─ xtask/                          host tool: standard library plus sha2; it reads the few ELF and PE
│  │                               header fields it needs with its own code
│  └─ src/                         main.rs, build.rs, image.rs, qemu.rs, tests.rs, check.rs, util.rs,
│                                  inspect.rs (PE and ELF headers, the five corrupted kernels) and
│                                  screen.rs (QEMU's picture of the screen)
├─ tools/                          setup-wsl.sh, sk.sh, sk.ps1, gdb/kernel.gdb (the GDB commands of section 8)
├─ tests/                          m1-boot.expect, m1-panic.expect, m1-pge.expect, m1-reject.expect,
│                                  m1-serial.expect, m1-stalekey.expect
└─ build/                          generated, ignored by git
```

The conversion from UEFI descriptors to `MemoryRegion` lives in `stillkey-bootinfo`, not in the loader, because the loader is a `#![no_std]` `#![no_main]` binary that cannot be built or tested for the host. `direct_map_size` takes a slice of `MemoryRegion`. The loader calls it in step 7 on the preliminary map, which it converts into a scratch buffer with the same code as the final map, and again on the final regions after the exit. The kernel calls it on `BootInfo.memory_map`.

`.gitattributes` matters on the development machine: Git for Windows is set to convert line endings, and a shell script or an expectation file checked out with CR LF does not work inside WSL.

## 4. Pinned configuration

`rust-toolchain.toml`

```toml
[toolchain]
channel = "1.99.1"
targets = ["x86_64-unknown-uefi", "x86_64-unknown-none"]
components = ["rustfmt", "clippy", "llvm-tools"]
profile = "minimal"
```

`Cargo.toml` (workspace root)

```toml
[workspace]
resolver = "3"
members = ["xtask", "boot/bootinfo", "boot/loader", "kernel", "lib/sk-elf", "lib/sk-serial"]
default-members = ["xtask"]          # bare `cargo build` builds only the host tool

[workspace.package]
version = "0.1.0"                    # every crate sets `version.workspace = true`
edition = "2024"
rust-version = "1.99"

[workspace.dependencies]
uefi = { version = "=0.41.0", default-features = false }   # no allocator, no logger: decision D-B-3
sha2 = { version = "=0.11.1", default-features = false }
stillkey-bootinfo = { path = "boot/bootinfo" }
sk-elf = { path = "lib/sk-elf" }
sk-serial = { path = "lib/sk-serial" }

[profile.dev]
opt-level = 1                        # keeps emulated boots quick; still debuggable
[profile.release]
opt-level = 2
debug = "line-tables-only"           # the kernel ELF keeps DWARF for GDB
codegen-units = 1
overflow-checks = true
```

`.cargo/config.toml`

```toml
[alias]
xtask = "run --locked --package xtask --"

[target.x86_64-unknown-none]
rustflags = [
  "-C", "relocation-model=static",       # the target defaults to position-independent; we want ET_EXEC
  "-C", "force-frame-pointers=yes",      # reliable backtraces in GDB
  "-C", "link-arg=-zmax-page-size=0x1000",
]

[target.x86_64-unknown-uefi]
rustflags = [
  "-C", "link-arg=/Brepro",              # deterministic PE timestamp (assumption A-08)
  "-C", "link-arg=/DEBUG:NONE",          # no PDB and no debug record in the .efi; see the table below
  "--cfg", 'sha2_backend="soft"',        # portable SHA-256 code only (assumption A-05)
]
```

`xtask` passes path remapping on the Cargo command line, for the triple being built:

```
--config 'target.<triple>.rustflags=["--remap-path-prefix=<workspace>=.", "--remap-path-prefix=<cargo home>=/cargo"]'
```

It must be this key. Cargo appends it to the array above, whereas `build.rustflags` would be ignored because a `target.<triple>.rustflags` table exists. Absolute paths therefore never reach the binaries. `xtask` also removes `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` from the environment of every Cargo child, because either one would replace all of these flags, `relocation-model=static` included.

`kernel/linker.ld`

```
ENTRY(_start)
PHDRS {
  text   PT_LOAD FLAGS(5);   /* read + execute */
  rodata PT_LOAD FLAGS(4);   /* read */
  data   PT_LOAD FLAGS(6);   /* read + write */
}
SECTIONS {
  . = 0xFFFFFFFF80000000;
  .text   : { *(.text .text.*) }                                                    :text
  . = ALIGN(4K);
  .rodata : { *(.rodata .rodata.*) *(.data.rel.ro .data.rel.ro.*) *(.got .got.*) }  :rodata
  . = ALIGN(4K);
  .data   : { *(.data .data.*) }                                                    :data
  .bss    : { *(.bss .bss.*) *(COMMON) }                                            :data
  /DISCARD/ : { *(.eh_frame*) *(.comment) *(.note*) }
}
```

Alignment is a location-counter assignment, not a section attribute, so it survives an empty section. The pointer-bearing constants of the precompiled `core` (`.data.rel.ro*`) and `.got*` are named in the read-only segment so that they do not fall into the writable one. The linker emits only the listed program headers and leaves out one whose sections are all empty. The Milestone 1 kernel has a writable static (the flag that tells its panic handler whether `qemu_exit=on` was read), so three segments are expected; two would also be valid, and no test pins the count.

`kernel/build.rs` prints two lines: `cargo::rustc-link-arg-bins=-T<manifest directory>/linker.ld` and `cargo::rerun-if-changed=linker.ld`.

Crate facts that the code must respect (all from the tools' documentation, [09](09-references.md#rust-toolchain-and-crates)):

| Fact | Consequence |
|---|---|
| `x86_64-unknown-none` produces a position-independent executable unless told otherwise | The `relocation-model=static` flag above. If it is forgotten, the loader refuses the kernel with `reason=type`, which is the intended safety net |
| The same target disables SSE, MMX and the red zone and uses the `kernel` code model | Nothing to configure. No floating point in the kernel |
| `x86_64-unknown-uefi` links with the bundled `rust-lld` and emits PE32+. For this target `rustc` always asks the linker for debug information (CodeView in a separate `.pdb`, plus a record inside the `.efi` that identifies it) and ignores `strip` | That record changes from build to build. `/DEBUG:NONE` comes later on the linker command line and wins, so no PDB and no record are written. GDB symbols exist for the kernel only. The loader is debugged by its log |
| Cargo uses exactly one source of extra compiler flags: `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS` in the environment replaces every `rustflags` entry in the configuration | `xtask` clears both variables and adds flags only through `target.<triple>.rustflags`, as described above |
| `sha2` 0.11 chooses its implementation with the configuration flag `sha2_backend`, not with a Cargo feature. Without it the crate also compiles an SSE-based variant on x86-64 that can never run in the loader | The `soft` value above |
| Cargo gives a crate no profile name at compile time | The `profile=` value in the log is derived from `debug_assertions` (section 7) |
| `uefi` 0.41: entry is `#[entry] fn main() -> Status`; boot services are free functions in `uefi::boot`; its `exit_boot_services` retries once and resets on failure | We use the crate for protocols and types, and call `GetMemoryMap`/`ExitBootServices` through the raw table ourselves (D-B-2, assumption A-06) |
| Naked functions and `asm!` are stable | The jump is `asm!` with `options(noreturn)` |
| The UEFI target uses the Microsoft calling convention and the kernel target System V | The kernel's entry is `extern "sysv64" fn _start(block: *const u8, block_len: u64) -> !`, with the address of the handoff block in `RDI` and its mapped length in `RSI`. The loader does not make a Rust call: its final `asm!` block takes the two values as `in("rdi")` and `in("rsi")` operands and jumps |

## 5. Build and image

| Command | Effect |
|---|---|
| `cargo xtask setup-check` | Tool versions: `rustc`, `cargo`, `qemu-system-x86_64`, `gdb`, `mtools` and `sgdisk`, each from its `--version` output, and the version of the `ovmf` package from `dpkg-query`; SHA-256 of the two firmware files; whether this user can open `/dev/kvm`, and so which accelerator is the default; whether Cargo's build directory is on the Linux filesystem (warning if not); the dependency tree of the loader and of the kernel (`cargo tree --locked -p stillkey-loader --target x86_64-unknown-uefi` and `cargo tree --locked -p stillkey-kernel --target x86_64-unknown-none`), so the full set of crates inside each binary is on record. Exit status 1 if a required tool or firmware file is absent. `run` and `test` make the same presence checks before they start and stop in the same way |
| `cargo xtask build [--release]` | `cargo build --locked -p stillkey-loader --target x86_64-unknown-uefi` and `cargo build --locked -p stillkey-kernel --target x86_64-unknown-none`, each with the `--config` of section 4; copies `x86_64-unknown-uefi/<debug\|release>/stillkey-loader.efi` and `x86_64-unknown-none/<debug\|release>/stillkey-kernel` from Cargo's build directory to `build/artifacts/<dev\|release>/BOOTX64.EFI` and `KERNEL.ELF`; prints their SHA-256 |
| `cargo xtask image [--release] [--cmdline "<boot options>"]` | Builds, then makes the disk image described below from that profile's binaries, with the boot options of `--cmdline` or with none, at `~/.cache/stillkey/image/disk.img`. Prints that path |
| `cargo xtask run [--release] [--accel kvm\|tcg] [--cmdline "<boot options>"] [--gui] [--gdb] [--monitor] [--trace]` | Builds, makes a fresh image and boots it interactively: serial port on the terminal, no timeout. `--cmdline` gives the boot options, which become that run's `CMDLINE.TXT`; without it the image has none. `--gui` opens a QEMU window. The flags are described in sections 6 and 8 |
| `cargo xtask test <name\|m1> [--release] [--repeat N] [--accel kvm\|tcg] [--timeout SECONDS]` | Runs one acceptance test (section 9), or all of them with `m1`. Each test builds what is out of date and makes its own image with its own boot options. `--timeout` is the time limit of each VM run in seconds; the default is 60 |
| `cargo xtask clean` | Removes `build/`, the Cargo build directory and the working directories under `~/.cache/stillkey/` |

**Accelerator.** `--accel` selects `kvm` or `tcg` (software emulation). Without it the default is `kvm` if this user can open `/dev/kvm` for reading and writing, otherwise `tcg`. QEMU always gets exactly one `-accel` option, so nothing falls back silently: `--accel kvm` without access to `/dev/kvm` ends with status 1. For every VM run the harness prints one result line that includes `accel=<kvm|tcg>` and QEMU's exit status.

**Disk image** (no root privileges; every run builds a new one and deletes the old):

```bash
truncate -s 68M disk.img
sgdisk --clear --new=1:2048:+64M --typecode=1:ef00 --change-name=1:STILLKEY-ESP disk.img
mformat -i disk.img@@1M -F -c 1 -T 131072 -v STILLKEY ::
mmd     -i disk.img@@1M ::/EFI ::/EFI/BOOT ::/EFI/STILLKEY
mcopy   -i disk.img@@1M BOOTX64.EFI  ::/EFI/BOOT/BOOTX64.EFI
mcopy   -i disk.img@@1M KERNEL.ELF   ::/EFI/STILLKEY/KERNEL.ELF
mcopy   -i disk.img@@1M CMDLINE.TXT  ::/EFI/STILLKEY/CMDLINE.TXT
```

The partition starts at sector 2048 (1 MiB), which is what `@@1M` addresses. 64 MiB with one-sector clusters gives about 129,000 clusters, safely above the FAT32 minimum of 65,525. `CMDLINE.TXT` holds the boot options with no trailing newline; when a run has no boot options the file is not written and the last command is skipped.

**Where things live.** Sources stay in `D:\OS`. Cargo's build directory (`~/.cache/stillkey/target`, set by the wrappers of section 2), every disk image and every firmware-variable copy are on the Linux filesystem under `~/.cache/stillkey/`, because QEMU and Cargo are slow and occasionally unreliable on the Windows-mounted drive. Each VM run has its own working directory there, called `$RUN` below: `~/.cache/stillkey/<test>/`, or `~/.cache/stillkey/run/` for `cargo xtask run`. After each run `xtask` copies `serial.log`, `qemu.log` and the two binaries to `D:\OS\build\<test>\`, where Windows tools can read them. `qemu.log` starts with a header written by the harness (the `setup-check` versions and firmware hashes, `accel=`, the full QEMU command line) and then receives everything QEMU writes to its standard error.

**Reproducibility.** `Cargo.lock` is committed; every Cargo call uses `--locked`; the toolchain is pinned; paths are remapped; the PE timestamp is deterministic and the `.efi` carries no debug record. Changing a dependency is the one occasion for a Cargo call without `--locked` (`cargo update -p <crate>`, or `cargo fetch` after adding one); the changed lock file is reviewed and committed. The disk image itself is not required to be byte-identical (its GPT identifiers and FAT timestamps vary) and is never kept.

## 6. VM launch command and exit codes

This is the exact command `xtask` runs for a test. `$RUN` is the test's working directory, `$ACCEL` is `kvm` or `tcg` (section 5), and `$TIMEOUT` is the time limit in seconds: 60, or the value given with `--timeout`.

```bash
cp /usr/share/OVMF/OVMF_VARS_4M.fd "$RUN/vars.fd"        # fresh firmware variables for every run

timeout -k 5 "$TIMEOUT" \
qemu-system-x86_64 \
  -machine q35 -accel "$ACCEL" \
  -cpu max -smp 1 -m 256M \
  -nodefaults -no-user-config \
  -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive if=pflash,format=raw,unit=1,file="$RUN/vars.fd" \
  -drive if=none,id=disk0,format=raw,file="$RUN/disk.img" \
  -device virtio-blk-pci,drive=disk0 \
  -device VGA \
  -display none \
  -serial file:"$RUN/serial.log" \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -nic none \
  -no-reboot \
  2>>"$RUN/qemu.log"
```

| Option | Reason |
|---|---|
| `-machine q35` | A modern chipset model; the one the firmware package documents for both firmware variants |
| `-accel "$ACCEL"` | Exactly one accelerator per run, hardware virtualisation or software emulation, with no fallback from one to the other. Every usable accelerator must pass (section 9) |
| `-cpu max` | Every feature the accelerator offers, so the CPU differs between KVM and software emulation and between hosts. Changeable (decision D-W-6). The loader checks the two CPU properties this milestone depends on in its step 1 (section 7). Under software emulation `max` offers 5-level paging; Ubuntu's OVMF does not switch it on |
| `-nodefaults`, `-nic none`, `-no-user-config` | Only the devices listed exist. No network at all |
| Two `pflash` drives | The documented way to run OVMF: firmware code read-only, variables in a per-run copy. A fresh copy has no stored boot entries, so firmware takes the default path `\EFI\BOOT\BOOTX64.EFI` (assumption A-14) |
| `virtio-blk-pci` | A disk OVMF can boot from. Stillkey itself never reads the disk; the loader uses firmware's file protocol |
| `-device VGA`, `-display none` | A framebuffer exists (firmware's default mode is expected to be 1280×800, 32-bit, blue-green-red order) but no window opens |
| `-serial file:` | Deterministic capture of COM1 |
| `isa-debug-exit` | Lets the guest end the VM with a status. A write of value `v` makes QEMU exit with status `(v << 1) \| 1`. Never combined with `-no-shutdown`, which would turn the exit into a pause on this QEMU version |
| `-no-reboot` | A CPU reset (the result of an unhandled fault at this stage) ends QEMU instead of looping silently |
| `timeout -k 5 "$TIMEOUT"` | A hang becomes a failure after the time limit, 60 seconds unless `--timeout` gives another value: `timeout` ends QEMU and returns status 124, or 137 if QEMU had to be killed five seconds later |
| `2>>"$RUN/qemu.log"` | QEMU's own messages are kept next to the serial log, below the harness header |

**Interactive runs.** `cargo xtask run` uses the same command with these differences:

| Flag | Difference |
|---|---|
| none | No `timeout`. `-serial stdio` in place of `-serial file:`, so the serial port is on the terminal, firmware's own console included. Without `--cmdline` the image has no boot options, so the kernel stops at its success marker and the VM stays up. Ctrl-C ends QEMU. A run with the serial port on the terminal leaves no `serial.log`; `xtask` then copies `qemu.log` and the two binaries to `build\run\` |
| `--cmdline "<boot options>"` | `xtask` writes the text, exactly as given, to `CMDLINE.TXT` in that run's image (section 5), so `test=panic`, `test=stalekey` and the scenarios of later milestones can be run by hand. Nothing is added: the guest ends QEMU by itself only if the text contains `qemu_exit=on`. `xtask` refuses text longer than 1024 bytes or with a byte outside 0x20–0x7E ([02 §3](02-boot-and-handoff.md#3-files-on-the-efi-system-partition)) |
| `--gui` | `-display gtk` in place of `-display none`: QEMU opens a window that shows the framebuffer. The serial port stays where the other flags put it. The window needs WSLg (assumption A-10) and QEMU's GTK module, which the setup script installs as `qemu-system-gui`, a recommended package of `qemu-system-x86` |
| `--monitor` | Adds `-monitor stdio`; the serial port goes back to `-serial file:"$RUN/serial.log"` |
| `--trace` | `-accel tcg`, and adds `-d int,cpu_reset,guest_errors`. The exception log arrives in `qemu.log` |
| `--gdb` | `-accel tcg`, and adds `-S -gdb tcp:127.0.0.1:1234`. The VM starts stopped, and the debugger stub listens on the loopback address only |

`--trace` and `--gdb` always use software emulation; `--accel kvm` together with either is refused. `--cmdline` and `--gui` combine with every other flag.

**The guest writes to the debug-exit port only when the boot option `qemu_exit=on` is present.** Every test adds it except `m1-screen`, which needs the VM to stay up after the boot (section 9). An image for real hardware never has it, and from Milestone 5 the loader of such an image is built without the write (`cargo xtask image --hardware`), so Stillkey never writes to an unknown I/O port on a real machine. The write is one 32-bit `out` to port `0xf4`.

| QEMU exit status | Meaning | Written by |
|---|---|---|
| 33 | Pass (value `0x10`) | Kernel, after its success marker `SK:KERNEL:M1_OK` |
| 35 | Refused or failed (value `0x11`) | Loader only, after any `SK:LOADER:` failure line that follows the reading of the command line |
| 37 | Kernel panic (value `0x12`) | Kernel panic handler, once `qemu_exit=on` has been read from an accepted `BootInfo` |
| 0 | The VM reset or shut down by itself: a failure in every test but one. In `m1-screen` the harness ends QEMU through the monitor after it has taken the picture, and 0 is the status it expects | – |
| 124, 137 | Timeout. Always a failure | `timeout` |
| 1 or anything else | QEMU could not start (for example `--accel kvm` without access to `/dev/kvm`), or an unexpected value | – |

Two kinds of failure cannot write the port and therefore show as a timeout. A loader failure that comes before `CMDLINE.TXT` has been read (the CPU check of step 1, or an image record or boot volume that cannot be opened) returns to firmware without writing. A kernel that rejects `BootInfo` prints `SK:KERNEL:BOOTINFO_REJECTED` and halts without writing, because the command line inside a rejected `BootInfo` is not trusted. The log line identifies both.

## 7. Boot behaviour and expected serial output

**Rules for log lines.** Lines that tests read have the form `SK:<COMPONENT>:<EVENT>` followed by `key=value` fields (a few lines carry a bare value, as the tables show). They are ASCII, start at column 0, end in CR LF, and are written to COM1 (`0x3F8`, 115200 baud, 8 data bits, no parity, 1 stop bit) by our own port I/O in both loader and kernel, never through a firmware protocol. A loader line is therefore legal at any point of the boot, including after a failed `ExitBootServices` call. The loader sends one CR LF before its first line, so that the line starts at column 0 whatever firmware left on the current one. In this milestone the loader does not print through the firmware console, because OVMF mirrors that console to the same serial port (assumption A-15) and every line would appear twice. Firmware prints its own lines (with terminal escape sequences) before ours; the harness strips escape sequences and carriage returns and considers only lines beginning with `SK:` (and, from Milestone 3, `AUDIT `).

`version` is `env!("CARGO_PKG_VERSION")`. `profile` is `if cfg!(debug_assertions) { "dev" } else { "release" }`, which is exact because neither profile in section 4 overrides `debug-assertions`. Numbers are decimal. Addresses, `flags` and the sizes written `0x…` are lower-case hexadecimal. An `<efi status>` is the 64-bit `EFI_STATUS` value in the same form, for example `status=0x800000000000000e` for `EFI_NOT_FOUND`. Digests are 64 lower-case hexadecimal digits.

**Boot options** (`CMDLINE.TXT`, space-separated `key=value`): `qemu_exit=on` (loader and kernel), `test=panic` (kernel), `test=stalekey` (loader), `test=pge` (loader), `test=serial` (loader). An option that a component does not know is ignored; the whole text is visible in the kernel's `CMDLINE` line. Each test writes its own options (section 9). `cargo xtask run --cmdline "<boot options>"` writes them for one interactive run, for example `cargo xtask run --cmdline "qemu_exit=on test=panic"`.

**What each component does**, with the marker it prints on success:

| # | Loader step ([02 §1](02-boot-and-handoff.md#1-the-path-from-power-on-to-the-kernel)) | Marker |
|---|---|---|
| 1 | Initialise COM1 and check that it answers: a byte sent in loopback mode must come back, or the loader writes nothing from then on. The check is made at `TPL_HIGH_LEVEL`, because firmware's console driver polls the same port for input from a timer event while boot services are active, and a poll between the two steps would take the byte ([02 §10](02-boot-and-handoff.md#10-assembly-and-unsafe-in-the-loader)). Then disable the watchdog; check the CPU before anything is allocated; read `ImageBase` and `ImageSize`. The CPU check: CPUID leaf `0x80000001`, EDX bit 20 (no-execute) must be set, and CR4 bit 12 (5-level paging active) must be clear. Otherwise the loader prints `SK:LOADER:CPU_UNSUPPORTED feature=nx` or `feature=la57_active` and returns an error to firmware | `SK:LOADER:START version=0.1.0 profile=<dev\|release>` then `SK:LOADER:IMAGE base=0x… size=0x…` |
| 2–3 | Open the boot volume; read `CMDLINE.TXT` if present, then `KERNEL.ELF`. The kernel's size, taken from the file information, is checked against rule V1 before the file is read | `SK:LOADER:CMDLINE len=<n>` (0 when the file is absent) and `SK:LOADER:KERNEL_FILE size=<bytes>`. With `test=serial`, between the two: the port check of step 1 again, with a pause of 50 ms before the read-back, and `SK:LOADER:SERIAL recheck=<ok\|lost>` |
| 4 | SHA-256 of the whole file, then rules V2–V10 with the kernel policy ([02 §4](02-boot-and-handoff.md#4-validating-the-kernel-elf)) | `SK:LOADER:KERNEL_SHA256 <64 hex digits>` and `SK:LOADER:KERNEL_VALID entry=0x… segments=<n>` |
| 5 | Allocate, zero, copy segments; allocate the 64 KiB stack | `SK:LOADER:KERNEL_LOADED phys=0x… pages=<n>` |
| 6 | Read the current graphics mode; find the ACPI table pointer | `SK:LOADER:GOP mode=<w>x<h> stride=<n> format=<rgbx\|bgrx> fb=0x… size=0x…` or `SK:LOADER:GOP none`; `SK:LOADER:ACPI rsdp=0x…` or `ACPI none` |
| 7 | Read the memory map once (the preliminary map); compute the direct-map size with `stillkey_bootinfo::direct_map_size` ([02 §5](02-boot-and-handoff.md#5-loading-segments-mappings-and-the-stack)), where a size above 64 TiB is `SK:LOADER:NO_MEMORY what=tables`; size and allocate the page-table pool; build the mappings | `SK:LOADER:PAGING root=0x… tables=<n> direct_map_gib=<n>` |
| 8 | Allocate one block, in whole pages and below 4 GiB, for `BootInfo`, the command line, the reserved table and the region array, whose capacity is the descriptor count of the preliminary map plus 64; fill everything except the memory map. The block's size in bytes is the mapped length the kernel receives in `RSI`. The buffer for the raw UEFI map is a separate allocation that is not in the reserved table and that the kernel never reads | `SK:LOADER:BOOTINFO phys=0x… size=<n>` |
| 9 | The loop of [02 §8](02-boot-and-handoff.md#8-the-final-getmemorymap--exitbootservices-sequence). After each failed `ExitBootServices` call the loader prints a retry line and goes round again. A map with more descriptors than the region array holds is `SK:LOADER:NO_MEMORY what=regions`. With `test=stalekey`, the loader allocates one page between `GetMemoryMap` and the first `ExitBootServices` call, which makes the key stale on purpose | `SK:LOADER:EBS_BEGIN`, then `SK:LOADER:EBS_RETRY n=<k>` once per failed call |
| 10 | Interrupts off; convert the map into the region array, sort it by base, check for overlap, and check that the direct map built in step 7 covers it | `SK:LOADER:EBS_OK attempts=<n> regions=<n>` |
| 11–12 | Set `EFER.NXE` and `CR0.WP` through the ordinary wrappers. Read `CR4` and, if bit 7 (`PGE`) is set, write the value back with that bit alone cleared, so that no global translation of firmware's survives the `CR3` load ([02 §9](02-boot-and-handoff.md#9-the-jump-and-the-kernels-entry-contract)); with `test=pge` the loader first sets the bit, as firmware on other machines leaves it. Print the markers; then the final assembly block, with `RDI` already holding the block's address and `RSI` its mapped length: `mov cr3`, `mov rsp`, `xor ebp, ebp`, `jmp` | `SK:LOADER:CR4 pge_was=<0\|1> before=0x… after=0x…`, then `SK:LOADER:JUMP entry=0x…` |

A Rust panic in the loader is a bug, not a refusal. The loader prints `SK:LOADER:PANIC at <file>:<line>: <message>`, writes the failure value if `qemu_exit=on` has been read, and halts, like the failures in [02 §11](02-boot-and-handoff.md#11-failure-behaviour) that cannot return to firmware.

| # | Kernel step | Marker |
|---|---|---|
| 1 | Initialise COM1 again (own driver) | `SK:KERNEL:START version=0.1.0 profile=<dev\|release>` |
| 2 | Validate the handoff as [02 §6](02-boot-and-handoff.md#6-bootinfo-version-1) defines, in this order. First the two entry values, before any byte of the block is read (`stillkey_bootinfo::check_block`): the address is in the direct map and page-aligned, the length is a non-zero multiple of 4096 of at most 16 MiB, and the block ends below 4 GiB. Then one byte slice of exactly that length is made, and `stillkey_bootinfo::validate` checks the header (`magic`; `version_major == 1`; `header_size` at least 256, a multiple of 8 and not above `total_size`; `total_size` not above the mapped length; `BOOT_SERVICES_EXITED` set; the direct-map base), then the memory map, the reserved table, the command line and, if `FRAMEBUFFER_PRESENT` is set, the framebuffer record. Each table's count, element size, address and alignment are checked with overflow-checked arithmetic and the table must lie inside the block before one byte of it is read. On failure the kernel prints `SK:KERNEL:BOOTINFO_REJECTED reason=<block\|magic\|version\|size\|flags\|layout\|memory_map\|reserved\|cmdline\|framebuffer>` and halts for ever, without writing the debug-exit port | `SK:KERNEL:BOOTINFO_OK version=1.0 size=<n> flags=0x…` |
| 3 | Log the command line, the state of `CR4.PGE` (a set bit is a kernel panic) and the image record. In the command line, bytes outside 0x20–0x7E, the double quote and the backslash are written as `\xNN` | `SK:KERNEL:CMDLINE "<text>"`, `SK:KERNEL:CR4 pge=0`, `SK:KERNEL:IMAGE sha256=<64 hex digits> phys=0x… pages=<n>` |
| 4 | Summarise memory: pages per kind, the reserved ranges, and what an allocator may use ([02 §7](02-boot-and-handoff.md#7-memory-the-kernel-must-reserve-before-starting-its-allocator)). Compute the direct-map size from the region array with the same `direct_map_size` function the loader used | One `SK:KERNEL:MEM kind=<name> regions=<n> pages=<n>` per kind present, `SK:KERNEL:MEM reserved ranges=<n> pages=<n>`, `SK:KERNEL:MEM usable_after_reserve pages=<n>`, `SK:KERNEL:MEM direct_map_gib=<n>` |
| 5 | If a framebuffer is present, write every visible pixel once: the `void` colour `#06080C`, with an 8-pixel band of `cyan` `#3DDCFF` at the top ([05 §2](05-ui-design.md#2-design-tokens)), in the byte order the format names. A row begins every `stride_pixels × 4` bytes; the bytes between the width and the stride are not written. The record was validated in step 2. `kernel/src/fb.rs` makes one byte slice of `stride_pixels × 4 × height` bytes at the framebuffer window, and `Framebuffer::fill_rows` of `stillkey-bootinfo` writes the band and the ground | `SK:KERNEL:FB_FILLED <w>x<h> format=<rgbx\|bgrx>` or `SK:KERNEL:FB none` |
| 6 | Clear the lower half of the top-level page table and reload `CR3`. From here only higher-half addresses work | `SK:KERNEL:LOWMAP_DROPPED` |
| 7 | With `test=panic`: panic deliberately | See below |
| 8 | Finish | `SK:KERNEL:M1_OK attempts=<n>`, then exit QEMU if asked, then `cli; hlt` forever |

Fields whose name does not say it all: `attempts` is the number of `ExitBootServices` calls (`BootInfo.exit_attempts`), and `n` in `EBS_RETRY` is the number of the call that failed. `regions` is the number of entries in the region array. `pages` are 4 KiB pages. In the `MEM reserved` line, `ranges` is the number of entries in `BootInfo.reserved` and `pages` is the sum of their pages. `usable_after_reserve` is the number of pages of kinds 1 and 2 left after the ranges of [02 §7](02-boot-and-handoff.md#7-memory-the-kernel-must-reserve-before-starting-its-allocator) are subtracted: the first 1 MiB, every `reserved` entry, and the framebuffer where it lies inside such a region. `size` in the loader's `BOOTINFO` line is the size of the block in bytes; in `BOOTINFO_OK` it is `total_size`, and `version` is major.minor. `kind=<name>` is one of `usable`, `loader`, `boot_services`, `acpi_reclaim`, `acpi_nvs`, `runtime`, `reserved`, `device`, `unusable`, `persistent`, `unaccepted` (kinds 1 to 11 of 02 §7), one line for each kind that occurs, in that order.

The kernel runs with interrupts disabled for the whole milestone. From the loader's `mov cr3` onward firmware's descriptor tables are no longer mapped, and the kernel has none of its own yet, so any CPU exception resets the VM; section 8 shows how that is diagnosed.

**Expected serial log, `m1-boot`** (firmware lines omitted; `…` are values that vary).

```
SK:LOADER:START version=0.1.0 profile=dev
SK:LOADER:IMAGE base=0x… size=0x…
SK:LOADER:CMDLINE len=12
SK:LOADER:KERNEL_FILE size=…
SK:LOADER:KERNEL_SHA256 ……
SK:LOADER:KERNEL_VALID entry=0xffffffff80…… segments=3
SK:LOADER:KERNEL_LOADED phys=0x… pages=…
SK:LOADER:GOP mode=1280x800 stride=1280 format=bgrx fb=0x… size=0x…
SK:LOADER:ACPI rsdp=0x…
SK:LOADER:PAGING root=0x… tables=… direct_map_gib=4
SK:LOADER:BOOTINFO phys=0x… size=…
SK:LOADER:EBS_BEGIN
SK:LOADER:EBS_OK attempts=1 regions=…
SK:LOADER:CR4 pge_was=0 before=0x668 after=0x668
SK:LOADER:JUMP entry=0xffffffff80……
SK:KERNEL:START version=0.1.0 profile=dev
SK:KERNEL:BOOTINFO_OK version=1.0 size=… flags=0x17
SK:KERNEL:CMDLINE "qemu_exit=on"
SK:KERNEL:CR4 pge=0
SK:KERNEL:IMAGE sha256=…… phys=0x… pages=…
SK:KERNEL:MEM kind=usable regions=… pages=…
SK:KERNEL:MEM kind=loader regions=… pages=…
SK:KERNEL:MEM kind=boot_services regions=… pages=…
SK:KERNEL:MEM kind=…
SK:KERNEL:MEM reserved ranges=5 pages=…
SK:KERNEL:MEM usable_after_reserve pages=…
SK:KERNEL:MEM direct_map_gib=4
SK:KERNEL:FB_FILLED 1280x800 format=bgrx
SK:KERNEL:LOWMAP_DROPPED
SK:KERNEL:M1_OK attempts=1
```

`pge_was=0` with `before` equal to `after` is what this VM's firmware gives: it hands over with `CR4.PGE` clear, so the loader has nothing to change; the `CR4` values are not pinned. `flags=0x17` is boot services exited, framebuffer present, ACPI pointer present, command line present. `reserved ranges=5` is the kernel image, the boot stack, the page tables, the boot info block and the loader image. `direct_map_gib=4` is the lower bound of the direct-map rule: this VM has no memory of kinds 1–6 or 10 above 4 GiB (assumption A-15). `segments=3` is an example, explained in section 4. `attempts=1` is what this VM is expected to produce, but a stale key on the first call is normal on some firmware, in which case `EBS_RETRY` lines appear and the count is higher. The values shown for `profile`, `mode`, `stride`, `format`, `flags`, `segments` and `attempts` are those of a dev build on this VM and are not pinned; the test checks values only where section 9 says so.

**`m1-panic`** has the command line `qemu_exit=on test=panic` (so `SK:LOADER:CMDLINE len=23`) and ends with:

```
SK:KERNEL:LOWMAP_DROPPED
SK:KERNEL:PANIC_BEGIN
SK:KERNEL:PANIC at kernel/src/main.rs:<line>: test=panic requested
SK:KERNEL:PANIC_END
```

**`m1-reject`** (one run per corrupted image) ends with, for example:

```
SK:LOADER:KERNEL_SHA256 ……
SK:LOADER:KERNEL_REJECTED reason=ident
```

**`m1-stalekey`** has the command line `qemu_exit=on test=stalekey` (so `SK:LOADER:CMDLINE len=26`) and differs from `m1-boot` around the exit:

```
SK:LOADER:EBS_BEGIN
SK:LOADER:EBS_RETRY n=1
SK:LOADER:EBS_OK attempts=2 regions=…
```

and then continues as `m1-boot`, except that the kernel's `CMDLINE` line shows both options and its last line is `SK:KERNEL:M1_OK attempts=2`. One retry is what this VM shows; more are accepted.

**`m1-pge`** has the command line `qemu_exit=on test=pge` (so `SK:LOADER:CMDLINE len=21`) and differs from `m1-boot` in one line:

```
SK:LOADER:CR4 pge_was=1 before=0x6e8 after=0x668
```

The kernel's `SK:KERNEL:CR4 pge=0` follows as in `m1-boot`. The two values are those of this VM; the test compares them with each other, not with these numbers.

## 8. Debugging

Start with the serial log: `build\<test>\serial.log`. The last `SK:` line says how far the boot got.

| Last line seen | Most likely cause | Look at |
|---|---|---|
| No `SK:` line at all | Firmware did not start the loader: image layout, partition type, file path, or the firmware files (assumption A-14). Or the loader ran and its serial driver wrote nothing | `cargo xtask run`: when no loader starts, OVMF ends in its built-in shell, which is on the serial terminal; there, `map -r` then `ls fs0:\EFI\BOOT`. On the host, `sgdisk -p` and `mdir` as below |
| `SK:KERNEL:` lines but no `SK:LOADER:` line | The loader's port check failed although the port works, so the loader wrote nothing: something read the port's receive buffer during the check | `probe` in the loader's `log.rs`; `m1-serial` |
| `LOADER:START`, then `CPU_UNSUPPORTED` and status 124 | The CPU model lacks no-execute, or this firmware build runs with 5-level paging, which the loader does not support | The `feature=` field; the `-cpu` option; the OVMF version in `setup-check` |
| `LOADER:IMAGE` or `LOADER:CMDLINE`, then nothing more or `KERNEL_MISSING` | File protocol calls; a path with the wrong separators. `KERNEL_MISSING` with no `CMDLINE` line before it, and status 124, means the loader's image record or the boot volume itself could not be opened | The `status=` field of `KERNEL_MISSING` |
| `KERNEL_REJECTED reason=type` | Kernel was built position-independent (the static flag was not applied) | `.cargo/config.toml`; `readelf -h KERNEL.ELF` should say `EXEC` |
| `KERNEL_REJECTED reason=segment` or `range` | Linker script or page-size flag | `readelf -lW KERNEL.ELF` |
| `EBS_BEGIN`, then `EBS_RETRY` lines up to `EBS_FAILED` | Something changes the memory map between the two calls on every pass: an allocation, or a firmware console or protocol call | The loop in `exit.rs`; nothing may be between the two calls |
| `EBS_BEGIN`, nothing more | A fault or endless loop in the map conversion or the sort (wrong descriptor stride), a fault while the last `BootInfo` fields are written, or a boot service or firmware console call made after `ExitBootServices` succeeded | `uefi_descriptor_size`, `convert.rs`, the exception log below |
| `EBS_OK`, no `JUMP` | A fault in the `EFER`, `CR0` or `CR4` wrapper, or in the serial write | The exception log below; the `SK:LOADER:CR4` line, if it was printed |
| `JUMP`, no `KERNEL:START` | The page tables: the loader's own code not identity-mapped; the no-execute bit used in a table while `EFER.NXE` is clear; a wrong entry address; a misaligned stack | The exception log below |
| `KERNEL:START`, then `BOOTINFO_REJECTED` and status 124 | Loader and kernel were built from different versions of `stillkey-bootinfo`. With `reason=block`: `RDI` did not hold the block's address or `RSI` its mapped length at the jump | The `reason=` field ([02 §6](02-boot-and-handoff.md#6-bootinfo-version-1) lists what each one covers); rebuild both; `jump.rs` |
| `KERNEL:CMDLINE`, then a panic naming `CR4.PGE` | The loader jumped with global pages still on | The `SK:LOADER:CR4` line; `cpu.rs` |
| `KERNEL:START`, then a reset | A kernel access to an address that is not mapped (direct map too short, framebuffer mapping, low addresses after `LOWMAP_DROPPED`) | The exception log below; `CR2` is the address |
| Any other `SK:LOADER:` failure line | The condition listed for that line in [02 §11](02-boot-and-handoff.md#11-failure-behaviour) | The fields of the line |
| Status 124 with a complete log | The exit device was not written: missing `qemu_exit=on` | `CMDLINE.TXT` in the image; the `SK:KERNEL:CMDLINE` line |
| Status 1 and an empty serial log | QEMU did not start: `--accel kvm` before the `kvm` group membership applies, or a missing firmware file | `qemu.log`; `cargo xtask setup-check` |

**Exception log.** A CPU exception at this stage has no handler, becomes a double fault, then a reset. To see the first exception:

```bash
cargo xtask run --trace          # software emulation, adds: -d int,cpu_reset,guest_errors
```

The log is `build\run\qemu.log`. Find the first `check_exception` entry. `v=0e` is a page fault: `CR2` is the address touched, `RIP` the instruction, and the error code `e=` says whether it was a write (bit 1), an instruction fetch (bit 4) or a reserved-bit violation (bit 3, typical of using the no-execute bit before enabling it). `v=0d` is a general-protection fault (often a non-canonical address). `v=06` is an invalid opcode (often a jump into data, or an SSE instruction). Software emulation is used for this because the exception log is not available with hardware virtualisation (assumption A-11).

**GDB on the kernel.**

```bash
cargo xtask run --gdb
# QEMU starts with: -accel tcg -S -gdb tcp:127.0.0.1:1234
# then, in a second WSL terminal in /mnt/d/OS:
gdb build/artifacts/dev/KERNEL.ELF -x tools/gdb/kernel.gdb
```

`tools/gdb/kernel.gdb` turns output paging off, selects Intel syntax, loads the symbols of the dev kernel from Cargo's build directory (its `file` line, which replaces a file named on the command line; change that line to debug a release build), and then runs `target remote localhost:1234`, `hbreak _start` and `continue`. `hbreak` (a hardware breakpoint) is used because the kernel's address is not mapped when the breakpoint is set. After it stops: `info registers`, `x/16i $rip`, `p/x $rdi` and `p/x $rsi` (the block's address and mapped length), `x/32xg $rdi` (the header, 256 bytes), `bt`. The header is read as raw words because the block holds the byte layout of [02 §6](02-boot-and-handoff.md#6-bootinfo-version-1), which the kernel decodes field by field; it is not a Rust struct in memory. The kernel is a fixed-address executable, so symbol addresses match without any offset.

**Debugging the loader.** The loader is linked without debug information, and GDB is not expected to read the PDB form this target would otherwise produce (assumption A-12), so the loader is kept small and is debugged through its markers. Add temporary `SK:LOADER:DBG` lines; if single-stepping is unavoidable, use `SK:LOADER:IMAGE base=` plus the offsets from `"$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-objdump" -d BOOTX64.EFI` to set address breakpoints. The `llvm-tools` component installs its tools in that directory; they are not on `PATH`.

**QEMU monitor.** `cargo xtask run --monitor` puts the monitor on the terminal and the serial port in `serial.log`: `info registers`, `info mem` and `info tlb` (the page tables as the CPU sees them), `xp /16xg <physical address>`, `info mtree`.

**Looking at the framebuffer.** `cargo xtask run --gui` opens a QEMU window. A correct Milestone 1 boot shows the screen filled with `#06080C` and an 8-pixel band of `#3DDCFF` along the top edge. A broken or slanted band, or red and blue swapped, points at the stride or the pixel format in `boot/bootinfo/src/fb.rs`. `m1-screen` makes this comparison for every pixel; the window is for looking.

**Inspecting artifacts on the host.** `readelf -hlW KERNEL.ELF` (type `EXEC`, entry, the segments and their flags). `"$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-readobj" --file-headers --sections BOOTX64.EFI` (machine `IMAGE_FILE_MACHINE_AMD64 (0x8664)`, subsystem `IMAGE_SUBSYSTEM_EFI_APPLICATION (0xA)`, the NX-compatible flag among the DLL characteristics, the section alignment, each section's characteristics). `mdir -i disk.img@@1M -/ ::` (files in the image). `sgdisk -p disk.img` (partition type `EF00`). The image of a test is `~/.cache/stillkey/<test>/disk.img`. `m1-unit` checks the header properties automatically; these commands are for looking.

## 9. Acceptance tests

`m1-reject` is the first of the loader failure-path tests that must pass on disposable VM disks before any hardware boot ([08 §1](08-hardware-and-dual-boot.md#1-safety-rules), rule 6).

The boot tests use the command in section 6. Each expectation file in `tests/` holds one required prefix per line, in order: a line of the section 7 transcript, cut before its first value that is shown as `…` or that section 7 names as not pinned (for example `SK:LOADER:GOP mode=`, `SK:LOADER:KERNEL_VALID entry=0xffffffff80`, `SK:KERNEL:MEM kind=usable regions=`, `SK:KERNEL:FB_FILLED `). The placeholder line `MEM kind=…` is left out. A log matches if it contains, in that order, a line that starts with each prefix. Other `SK:` lines may come in between, which is how `EBS_RETRY` and further `MEM kind=` lines fit. A line starting with `!` gives text that no `SK:` line may contain. Values are compared by the harness in code, and only where the table says so.

The six files follow from that rule, with one exception: the success marker is held as `SK:KERNEL:M1_OK`, without ` attempts=`, which is the text `m1-panic.expect` forbids. `m1-boot.expect` holds every line of the `m1-boot` transcript, then `!PANIC`, `!REJECTED` and `!FAILED`. `m1-stalekey.expect` holds the same lines with `SK:LOADER:CMDLINE len=26`, with `SK:LOADER:EBS_RETRY n=` added after `SK:LOADER:EBS_BEGIN`, and with `SK:KERNEL:CMDLINE "qemu_exit=on test=stalekey"`. `m1-panic.expect` holds the `m1-boot` lines up to `SK:KERNEL:LOWMAP_DROPPED`, with `SK:LOADER:CMDLINE len=23` and `SK:KERNEL:CMDLINE "qemu_exit=on test=panic"`, then `SK:KERNEL:PANIC_BEGIN`, `SK:KERNEL:PANIC at ` and `SK:KERNEL:PANIC_END`, then `!SK:KERNEL:M1_OK`, `!REJECTED` and `!FAILED`. `m1-pge.expect` is shorter, because `m1-boot` already pins the rest: the two `START` lines, `SK:LOADER:CMDLINE len=21`, `EBS_BEGIN`, `EBS_OK attempts=`, `SK:LOADER:CR4 pge_was=1`, `JUMP`, `BOOTINFO_OK`, `SK:KERNEL:CMDLINE "qemu_exit=on test=pge"`, `SK:KERNEL:CR4 pge=0`, `LOWMAP_DROPPED` and `SK:KERNEL:M1_OK`, then `!PANIC`, `!REJECTED` and `!FAILED`. `m1-serial.expect` is short in the same way: the two `START` lines, `SK:LOADER:CMDLINE len=24`, `SK:LOADER:SERIAL recheck=`, `KERNEL_FILE`, `EBS_BEGIN`, `EBS_OK attempts=`, `JUMP`, `BOOTINFO_OK`, `SK:KERNEL:CMDLINE "qemu_exit=on test=serial"`, `LOWMAP_DROPPED` and `SK:KERNEL:M1_OK`, then the same three forbidden texts. `m1-reject.expect` serves all five images: the `m1-boot` lines up to `SK:LOADER:KERNEL_SHA256 `, then `SK:LOADER:KERNEL_REJECTED reason=`, then `!KERNEL_VALID` and `!SK:KERNEL:`. The reason is not in the file: the harness compares it in code with the reason expected for each image.

| Test | Image | Passes when |
|---|---|---|
| `m1-unit` | – | Two parts, both on the host. (1) `cargo test --locked` for each of `sk-elf`, `stillkey-bootinfo` and `xtask`: every rule V1–V10 has an accepting and a rejecting case; the encoded `BootInfo` is 256 bytes, `MemoryRegion` 32, `ReservedRange` 24, with every field at its offset of 02 §6; map conversion handles a descriptor size of 48, sorts by base and reports an overlap; `direct_map_size` returns 4 GiB for a map that ends below 4 GiB, rounds up to 1 GiB above that, ignores kinds 7, 8, 9 and 11, and reports a size above 64 TiB as an error; `check_block` and `validate` accept the block the loader's layout produces and reject each violation of the acceptance rules of 02 §6 with its reason, including counts and addresses chosen to overflow; and the regression tests in `tests/guard_page.rs` and `tests/bounds.rs`, where the header is valid, every table entry inside the block is valid, and a table extends beyond the mapped block or beyond `total_size`, are rejected on the bound with no read past the slice; the framebuffer rule of 02 §6 accepts the mode this VM reports and rejects each violation with `framebuffer`, a record without `FRAMEBUFFER_PRESENT` is ignored, `usable_after_reserve` subtracts each range of 02 §7 once, a pixel is stored red first for `rgbx` and blue first for `bgrx`, and `fill_rows` writes the visible pixels of the rows asked for and nothing else (`tests/framebuffer.rs`); and `xtask`'s own readers: the PE and ELF header checks accept a good file and reject each defect, each of the five corruptions changes what its name says, the picture check accepts the expected screen and names the first wrong pixel, and the check of the `qemu.log` header refuses a log without the QEMU version or without a firmware hash. The harness fails if any of the three packages ran zero tests. (2) Artifact checks done by `xtask` itself, with no external tool, on the binaries of the selected profile: `BOOTX64.EFI` has `IMAGE_DLLCHARACTERISTICS_NX_COMPAT` (`0x0100`) set, a section alignment of at least 4096, and no section that is both writable and executable; `KERNEL.ELF` is `ET_EXEC` for x86-64, and every `PT_LOAD` has flags 4, 5 or 6 |
| `m1-boot` | `qemu_exit=on` | Status 33. All markers of the section 7 transcript in order, with `GOP mode=`, `ACPI rsdp=` and `FB_FILLED` (not their `none` forms). The mode and the format in `FB_FILLED` are those of the loader's `GOP` line. The run's `qemu.log` starts with the QEMU version and the hashes of both firmware files, before its `accel=` line (section 5). `profile=` in both `START` lines is the profile under test. `KERNEL_SHA256` equals the host's SHA-256 of the file placed in the image, and equals the kernel's `IMAGE sha256=`. The kernel's `CMDLINE` text is `qemu_exit=on`. `reserved ranges=5`. `direct_map_gib=4` in the loader's line and in the kernel's (assumption A-15). `usable_after_reserve` is greater than 0 and less than 65,536 pages (256 MiB). The attempt count is not pinned, and `EBS_RETRY` lines are allowed. No line contains `PANIC`, `REJECTED` or `FAILED` |
| `m1-panic` | `qemu_exit=on test=panic` | Status 37. `PANIC_BEGIN`, one `PANIC at` line naming a file and line with the message `test=panic requested`, `PANIC_END`, in that order. No success marker |
| `m1-pge` | `qemu_exit=on test=pge` | Status 33. `SK:LOADER:CR4 pge_was=1`; `before` has bit 7 set; `after` equals `before` with bit 7 cleared and every other bit unchanged; `SK:KERNEL:CR4 pge=0`; then the success marker |
| `m1-reject` | Five images with `qemu_exit=on`, each with one corruption of the real kernel: first byte changed (`ident`); `e_type` set to 3 (`type`); `p_flags` of the last `PT_LOAD` set to 7 (`wx`); file truncated to `p_offset + p_filesz − 1` of the last `PT_LOAD` with `p_filesz > 0` (`segment`); file truncated to 100 bytes, inside the program-header table (`phdr`) | Each: status 35; `KERNEL_REJECTED` with the expected reason; `KERNEL_FILE size=` and `KERNEL_SHA256` are the size and the SHA-256 of the corrupted file as the host computes them, so the loader judged that file and no other; no `KERNEL_VALID` and no `SK:KERNEL:` line |
| `m1-serial` | `qemu_exit=on test=serial` | Status 33; `SK:LOADER:SERIAL recheck=ok`; the loader's later lines and the kernel's success marker follow. `recheck=lost` means that firmware could read the port during the loader's check |
| `m1-stalekey` | `qemu_exit=on test=stalekey` | Status 33; at least one `EBS_RETRY` line, then `EBS_OK` with `attempts` of 2 or more, then the success marker; the kernel's `attempts` equals the loader's |
| `m1-repro` | – | Two release builds of the loader and the kernel into two fresh, different build directories, `$RUN/repro-a` and `$RUN/repro-b`, where `$RUN` is `~/.cache/stillkey/m1-repro/`. Both directories are deleted first, and each build gets its directory through Cargo's `--target-dir` option, which takes precedence over `CARGO_TARGET_DIR`. The SHA-256 of `BOOTX64.EFI` is the same in both, and so is that of `KERNEL.ELF`. No binary of either build contains the path of the workspace, of the home directory or of Cargo's home (the remapping of section 4). The four files are kept in `build\m1-repro\`. It never calls `xtask clean`, and it does not depend on `--release` or `--accel` |
| `m1-screen` | No boot options, so the kernel halts and the VM stays up | The harness waits for `SK:KERNEL:M1_OK` in the serial log, then tells QEMU's monitor to write a picture of the screen (`screendump`) and to quit. Status 0. The picture has the width and height of the mode in `FB_FILLED`. Rows 0 to 7 are `#3DDCFF` and every other pixel is `#06080C`, compared exactly; a failure names the number of wrong pixels and the first of them. The picture is kept as `build\m1-screen\screen.ppm` |

Each corruption in `m1-reject` breaks exactly one rule, or, for the 100-byte file, makes V4 the first rule that fails; the validator reports the first failing rule in the order V1 to V10. Each of the five runs has its own working directory and its own folder under `build\m1-reject\`, named after the reason.

For `m1-boot` the harness also counts the non-empty serial lines that precede the first `SK:` line and prints `firmware_lines=<n>` on its result line. Zero means OVMF did not mirror its console to the serial port (assumption A-15); that is reported, not failed, because no marker depends on it.

**Order and repeats.** `cargo xtask test m1` runs `m1-unit`, `m1-repro` and `m1-screen` once each, then the six boot tests (`m1-boot`, `m1-panic`, `m1-pge`, `m1-reject`, `m1-serial`, `m1-stalekey`) `--repeat` times. `--repeat` applies to the boot tests only. The run stops at the first failure and leaves that run's files in place. Every `cargo xtask test` invocation ends with one summary line, `SUMMARY test=<name> profile=<dev|release> repeat=<n> result=<pass|fail> accel=<kvm|tcg>`; for `m1-unit` or `m1-repro` alone, which start no VM, it says `accel=none`, and for `m1-repro` alone it says `profile=release`, the profile that test always builds. `repeat=` is the value of `--repeat` as given, also for a test that runs once. When this user cannot open `/dev/kvm`, so that `tcg` is the only usable accelerator, the line ends `accel=tcg only`.

**Acceptance:** 20 consecutive passes in both build profiles on every usable accelerator (D-W-5). When this user cannot open `/dev/kvm` (assumption A-03), that is the two `--accel tcg` commands alone, and their summary line says `accel=tcg only`. Otherwise it is all four of these, with no failure:

```bash
cargo xtask test m1 --repeat 20 --accel kvm
cargo xtask test m1 --repeat 20 --accel tcg
cargo xtask test m1 --release --repeat 20 --accel kvm
cargo xtask test m1 --release --repeat 20 --accel tcg
```

**Result, 2026-11-10.** All four commands passed on the development machine, one after the other, on the code as it stands. Each command printed 208 `RESULT` lines that end `result=pass` (5 from `m1-unit`, 2 from `m1-repro`, 1 from `m1-screen`, and 10 for each of the 20 rounds of boot tests, of which `m1-reject` gives 5), no line with `result=fail`, and `SUMMARY test=m1 profile=<dev|release> repeat=20 result=pass accel=<kvm|tcg>`. That is 200 boots per command and 800 in all, plus one `m1-screen` boot per command. Each command took about eight minutes. The logs are `build\matrix\final\<dev|release>-<kvm|tcg>.log`, with one line per command in `summary.txt`.

| Artifact | Profile | SHA-256 |
|---|---|---|
| `BOOTX64.EFI` | dev | `2678051daf2d489abece29390e432dd4a4209f898d5c9c94588c057dec0dd01d` |
| `KERNEL.ELF` | dev | `65a8447ab3890abd696998bd70f99329e36e73569531762ff89e6ffd80724809` |
| `BOOTX64.EFI` | release | `d11faf9bd7d7fbefb3314dbf0fef20c8426dcfc3008ada422db659385e268c03` |
| `KERNEL.ELF` | release | `4835a4e1100a280a3ac345384da3e47b9668220ad4e4994f87d66f38d7843bea` |

The release hashes are also the two that `m1-repro` printed for both of its builds in all four commands. The hashes are a record of this toolchain on the development machine, not a value any test compares against; `m1-repro` varies the build directory only, so whether another workspace location gives the same bytes is not tested.

An earlier matrix on the code before the serial fix failed in its third command (section 0). Its logs are in `build\matrix\probe\`.

What this demonstrates, and what it does not, is stated in [04 §9](04-security-and-permissions.md#9-what-each-milestone-demonstrates).

## 10. Implementation order

Each step ends with something observable, so a failure is caught where it was introduced.

| Step | Work | Check |
|---|---|---|
| A | `tools/setup-wsl.sh`, `tools/sk.sh`, `tools/sk.ps1`, `.gitattributes`, `rust-toolchain.toml`, the workspace skeleton with every crate's manifest and all Milestone 1 dependencies declared (so that the lock file generated by the setup script is complete), `xtask setup-check` | The setup script runs to its end; `setup-check` prints all versions and exits 0 |
| B | `sk-serial`; a loader that prints `SK:LOADER:START` and returns; a stub kernel that only halts, linked with the real linker script, so that `build` and `image` have both binaries; `xtask build`, `image`, `run` | The marker appears in the serial log. This proves toolchain, image commands, firmware files and the QEMU options (assumption A-14) before any real logic exists |
| C | `sk-elf` with host tests; loader reads files, hashes, validates; `xtask test` with the expectation files | `cargo test --locked -p sk-elf` and `m1-reject` pass |
| D | `stillkey-bootinfo` with host tests (layout, conversion, `direct_map_size`); the artifact checks in `xtask`; loader checks the CPU, allocates, copies, captures graphics and ACPI, builds page tables | `m1-unit` passes; markers up to `BOOTINFO` |
| E | Exit loop, the `EFER` and `CR0` wrappers, jump; kernel prints `START` | `SK:KERNEL:START` appears. This is the hardest step; the exception log is the tool |
| F | Kernel validation, memory summary, framebuffer fill, lower-half removal, exit | `m1-boot` passes |
| G | Panic path, stale-key option, reproducibility check, repeat runs in both profiles on every usable accelerator | Section 9 acceptance |

A git repository is created at step A, with `.gitattributes` in its first commit.

## 11. Assumptions this milestone verifies

A-02 to A-09, A-11, A-12, A-14, A-15 and A-16 in [01 §2](01-overview-and-architecture.md#2-assumptions-to-verify) are checked here; A-17 is checked at Milestone 2, A-10 at Milestone 4, A-01 and A-13 at Milestone 5. The ones most likely to need an adjustment, each with its fallback:

| Assumption | Fallback |
|---|---|
| A-06: the `uefi` crate exposes the raw system table. **Holds** (`uefi::table::system_table_raw()`, crate version 0.41.0); the fallback was not needed | Call the crate's `exit_boot_services` and accept one retry and reset-on-failure; the `m1-stalekey` test is then dropped or reworked |
| A-08: `/Brepro` plus `/DEBUG:NONE` are accepted and make the `.efi` byte-identical across builds (checked by `m1-repro`). **Holds**; the fallback was not needed | `/timestamp:0` in place of `/Brepro`; or `xtask` clears the timestamp and debug-directory fields of the PE header after the build |
| A-05: `sha2` builds for the soft-float UEFI target. **Holds**: the loader's hash equals the host's in every `m1-boot` and `m1-reject` run | It is already forced to its portable backend with `--cfg sha2_backend="soft"`. The fallback is a 100-line SHA-256 of our own with the standard test vectors |
| A-03: KVM works in WSL2. **Holds**: this user can open `/dev/kvm`, and all four commands of section 9 apply | Software emulation only: boots take a few seconds longer, acceptance is the two `--accel tcg` commands of section 9, and their summary line says `accel=tcg only` |
| A-14: QEMU 10.2.1 accepts `-device VGA` and `virtio-blk-pci` under `-nodefaults`, and OVMF with a fresh variables file starts `\EFI\BOOT\BOOTX64.EFI` from the disk. **Holds** | Step B exists to catch this before anything depends on it. The repair is the option spelling or another disk model that OVMF can boot from |
| A-15: OVMF mirrors its text console to COM1, and this VM's memory map tops out below 4 GiB, so the direct map is 4 GiB. **Holds** (`firmware_lines=2`, `direct_map_gib=4`) | Without mirroring, no test changes; only the firmware shell of section 8 is no longer reachable on the serial terminal. With memory above 4 GiB, the value `m1-boot` expects becomes the one both components log |
| A-16: `rust-lld`'s defaults for `x86_64-unknown-uefi` give the NX-compatible flag, 4 KiB section alignment and no section that is both writable and executable (checked by `m1-unit`). **Holds** in both profiles (`nx_compat=yes section_alignment=4096 wx_sections=0`); the fallback was not needed | Add `-C link-arg=/NXCOMPAT` and `-C link-arg=/ALIGN:4096` to the UEFI target's flags in section 4. A writable and executable section would have to be found in the code that produced it |

## 12. Not in this milestone

No interrupts, no descriptor tables, no allocator, no processes, no keyboard, no drawing beyond a filled screen, no boot filesystem, no signature check, no graphics mode selection. Each has a milestone in [06](06-milestones.md).
