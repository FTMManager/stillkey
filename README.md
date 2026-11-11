# Stillkey

Stillkey is an experimental desktop operating system for x86-64 UEFI machines, written from scratch in Rust. It has its own UEFI loader and its own kernel.

The aim is a private workspace: an app starts with access to nothing, gets access only when the person at the machine allows it, and the system can always show what each app is doing and what it can reach.

**Stillkey is at an early stage and runs only in a virtual machine.** Do not boot it on a real computer.

## Status

| Milestone | State |
|---|---|
| 1, boot | Done. The loader checks and loads the kernel and hands over; the kernel checks what it was given and reports on the serial port and the screen. |
| 2, run and observe | Done. Programs run in ring 3 in separate address spaces under a pre-emptive scheduler. A program that faults is ended and the others keep running. |
| 3, grant and revoke | Specified, not started. File access through grants that can be revoked. |
| 4, desktop demo | Planned. A desktop with a permission prompt that apps cannot forge, then an assistant that works only in what it has been handed. |
| 5, hardware and dual boot | Planned. |

`main` holds the milestones that are done.

## Build and test

You need Linux, or WSL2 on Windows, with:

- Rust, in the version that `rust-toolchain.toml` pins (`rustup` installs it by itself);
- QEMU for x86-64 and the OVMF firmware;
- `mtools` and `gdisk`, to build the disk image.

On Debian or Ubuntu: `sudo apt-get install build-essential qemu-system-x86 ovmf mtools gdisk`.

```sh
cargo xtask setup-check   # says what is missing
cargo xtask build         # the loader, the kernel and a disk image
cargo xtask run           # boot it in QEMU, serial port on the terminal
cargo xtask test all      # every test; `m1` or `m2` runs one milestone's
```

The tests boot the real image in QEMU and judge the serial log. They use KVM when `/dev/kvm` is usable, and software emulation otherwise.

## Layout

| Path | What |
|---|---|
| `boot/` | The UEFI loader and the block of data it hands to the kernel |
| `kernel/` | The kernel |
| `lib/` | `no_std` crates shared by the loader, the kernel and the tests |
| `user/` | Programs that run on Stillkey |
| `xtask/` | The build and test tool |
| `tests/` | The serial output each test expects |
| `docs/` | The design and the build plan, and the record of each milestone's tests |

## Design documents

[docs/](docs/README.md) has the design: what the system is for, how it boots, the kernel's interfaces, the permission model, the interface, and the plan milestone by milestone. Each milestone that is built has a specification there and the records of its test runs.

## License

MIT, see [LICENSE](LICENSE). The bundled font, Spleen, has its own BSD 2-Clause license in `assets/spleen-LICENSE`.
