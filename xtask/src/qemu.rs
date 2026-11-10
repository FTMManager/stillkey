//! The VM launch command of docs/07 section 6.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::check::{OVMF_CODE, OVMF_VARS, kvm_usable, report_once};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Accel {
    Kvm,
    Tcg,
}

impl Accel {
    pub fn as_str(self) -> &'static str {
        match self {
            Accel::Kvm => "kvm",
            Accel::Tcg => "tcg",
        }
    }

    /// `kvm` if this user can open `/dev/kvm`, otherwise `tcg`.
    pub fn default_for_host() -> Accel {
        if kvm_usable() { Accel::Kvm } else { Accel::Tcg }
    }

    pub fn parse(text: &str) -> Result<Accel, String> {
        match text {
            "kvm" => Ok(Accel::Kvm),
            "tcg" => Ok(Accel::Tcg),
            other => Err(format!("--accel {other}: expected kvm or tcg")),
        }
    }
}

/// Where the guest's serial port goes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Serial {
    /// `serial.log` in the run directory.
    File,
    /// The terminal.
    Stdio,
}

pub struct Vm<'a> {
    pub dir: &'a Path,
    pub accel: Accel,
    /// Seconds; `None` runs until QEMU ends by itself or by Ctrl-C.
    pub timeout: Option<u64>,
    pub serial: Serial,
    pub gui: bool,
    pub monitor: bool,
    pub trace: bool,
    pub gdb: bool,
}

/// The program and arguments of docs/07 section 6 for `vm`. Also puts a fresh
/// variable store in the run directory and removes the previous serial log.
fn command(vm: &Vm) -> Result<(String, Vec<String>), String> {
    let dir = vm.dir;
    let vars = dir.join("vars.fd");
    fs::copy(OVMF_VARS, &vars).map_err(|e| format!("{OVMF_VARS}: {e}"))?; // fresh variables every run
    let serial_log = dir.join("serial.log");
    let _ = fs::remove_file(&serial_log);

    let mut args: Vec<String> = Vec::new();
    let mut program = "qemu-system-x86_64".to_string();
    if let Some(seconds) = vm.timeout {
        program = "timeout".to_string();
        args.extend(["-k".into(), "5".into(), seconds.to_string(), "qemu-system-x86_64".into()]);
    }
    let path = |p: &Path| p.to_str().map(str::to_string).ok_or("path is not UTF-8".to_string());
    args.extend(
        [
            "-machine", "q35", "-accel", vm.accel.as_str(), "-cpu", "max", "-smp", "1", "-m", "256M",
            "-nodefaults", "-no-user-config",
        ]
        .map(String::from),
    );
    args.push("-drive".into());
    args.push(format!("if=pflash,format=raw,unit=0,readonly=on,file={OVMF_CODE}"));
    args.push("-drive".into());
    args.push(format!("if=pflash,format=raw,unit=1,file={}", path(&vars)?));
    args.push("-drive".into());
    args.push(format!("if=none,id=disk0,format=raw,file={}", path(&dir.join("disk.img"))?));
    args.extend(["-device", "virtio-blk-pci,drive=disk0", "-device", "VGA"].map(String::from));
    args.extend(["-display", if vm.gui { "gtk" } else { "none" }].map(String::from));
    args.push("-serial".into());
    match vm.serial {
        Serial::File => args.push(format!("file:{}", path(&serial_log)?)),
        Serial::Stdio => args.push("stdio".into()),
    }
    if vm.monitor {
        args.extend(["-monitor", "stdio"].map(String::from));
    }
    args.extend(
        ["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04", "-nic", "none", "-no-reboot"]
            .map(String::from),
    );
    if vm.trace {
        args.extend(["-d", "int,cpu_reset,guest_errors"].map(String::from));
    }
    if vm.gdb {
        args.extend(["-S", "-gdb", "tcp:127.0.0.1:1234"].map(String::from));
    }
    Ok((program, args))
}

/// Starts `qemu.log` with its header (docs/07 section 5): the `setup-check`
/// tool versions and firmware hashes, the accelerator and the command. QEMU's
/// own messages follow it.
fn open_log(vm: &Vm, program: &str, args: &[String]) -> Result<fs::File, String> {
    let mut log = fs::File::create(vm.dir.join("qemu.log")).map_err(|e| e.to_string())?;
    for line in report_once() {
        writeln!(log, "{line}").map_err(|e| e.to_string())?;
    }
    writeln!(log, "accel={}", vm.accel.as_str()).map_err(|e| e.to_string())?;
    writeln!(log, "command: {program} {}", args.join(" ")).map_err(|e| e.to_string())?;
    Ok(log)
}

/// Boots `dir/disk.img` and returns QEMU's exit status (124 or 137 after a
/// timeout, -1 if it was ended by a signal).
pub fn run(vm: &Vm) -> Result<i32, String> {
    let (program, args) = command(vm)?;
    let log = open_log(vm, &program, &args)?;
    let status = Command::new(&program)
        .args(&args)
        .stdin(if vm.serial == Serial::Stdio || vm.monitor { Stdio::inherit() } else { Stdio::null() })
        .stderr(log)
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    Ok(status.code().unwrap_or(-1))
}

/// Boots `dir/disk.img`, waits until the serial log contains `marker`, has
/// QEMU write the screen to `ppm`, and ends the VM. Returns QEMU's exit
/// status: 0 after the capture, 124 or 137 if the marker never came and the
/// time limit ended the VM.
///
/// `vm` must have a time limit, the serial port in a file and the monitor on.
/// The guest is expected to stay up after the marker: it is not booted with
/// `qemu_exit=on`.
pub fn capture_screen(vm: &Vm, marker: &str, ppm: &Path) -> Result<i32, String> {
    let ppm_text = ppm.to_str().ok_or("path is not UTF-8")?;
    let _ = fs::remove_file(ppm);
    after_marker(vm, marker, &format!("screendump {ppm_text}\nquit\n"))
}

/// Starts the VM, waits until `marker` is in the serial log and ends the VM
/// through the monitor (D-W-7). Returns QEMU's exit status: 0 if the harness
/// ended it, the guest's own status if the guest ended first.
pub fn quit_after(vm: &Vm, marker: &str) -> Result<i32, String> {
    after_marker(vm, marker, "quit\n")
}

/// Starts the VM and sends `commands` to the monitor once `marker` is in
/// the serial log.
fn after_marker(vm: &Vm, marker: &str, commands: &str) -> Result<i32, String> {
    if vm.timeout.is_none() || vm.serial != Serial::File || !vm.monitor {
        return Err("this run needs a time limit, a serial file and the monitor".to_string());
    }
    let (program, args) = command(vm)?;
    let log = open_log(vm, &program, &args)?;
    let mut child = Command::new(&program)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;

    let serial_log = vm.dir.join("serial.log");
    let marker = marker.as_bytes();
    loop {
        let seen = fs::read(&serial_log)
            .is_ok_and(|bytes| bytes.windows(marker.len()).any(|window| window == marker));
        if seen {
            // The monitor reads its commands from standard input.
            let mut monitor = child.stdin.take().ok_or("QEMU has no standard input")?;
            monitor.write_all(commands.as_bytes()).map_err(|e| format!("QEMU monitor: {e}"))?;
            break;
        }
        // The time limit, or a guest that ended the VM by itself.
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    Ok(status.code().unwrap_or(-1))
}
