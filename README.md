# Stillkey

Stillkey is an experimental desktop operating system for x86-64 UEFI machines, written from scratch in Rust. It has its own UEFI loader and its own kernel.

The aim is a private workspace: an app starts with access to nothing, gets access only when the person at the machine allows it, and the system can always show what each app is doing and what it can reach.

**Stillkey is at an early stage and runs only in a virtual machine.** Do not boot it on a real computer.

## Status

| Branch | What it does |
|---|---|
| `main` | Milestone 1, boot. The loader checks and loads the kernel and hands over; the kernel checks what it was given and reports on the serial port and the screen. |
| `milestone-2` | Milestone 2, run and observe. Programs run in ring 3 in separate address spaces under a pre-emptive scheduler. A program that faults is ended and the others keep running. |

Next is file access through grants that can be revoked.

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
cargo xtask test m1       # the Milestone 1 tests; on milestone-2 also `m2` and `all`
```

The tests boot the real image in QEMU and judge the serial log. They use KVM when `/dev/kvm` is usable, and software emulation otherwise.

## Layout

| Path | What |
|---|---|
| `boot/` | The UEFI loader and the block of data it hands to the kernel |
| `kernel/` | The kernel |
| `lib/` | `no_std` crates shared by the loader, the kernel and the tests |
| `user/` | Programs that run on Stillkey (branch `milestone-2`) |
| `xtask/` | The build and test tool |
| `tests/` | The serial output each test expects |

## License

MIT, see [LICENSE](LICENSE). The bundled font, Spleen, has its own BSD 2-Clause license in `assets/spleen-LICENSE` (branch `milestone-2`).
