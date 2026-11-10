//! Stillkey host tool. Run it as `cargo xtask <command>` through
//! `tools/sk.sh` (WSL) or `tools/sk.ps1` (PowerShell).

mod bootfs;
mod build;
mod check;
mod image;
mod inspect;
mod qemu;
mod screen;
mod tests;
mod util;

use std::fs;
use std::process::ExitCode;

use qemu::{Accel, Serial, Vm};

const USAGE: &str = "\
usage: cargo xtask <command>
  setup-check
  build  [--release]
  image  [--release] [--cmdline \"<boot options>\"]
  run    [--release] [--accel kvm|tcg] [--cmdline \"<boot options>\"] [--gui] [--gdb] [--monitor] [--trace]
  test   <m1-unit|m1-repro|m1-screen|m1-boot|m1-panic|m1-pge|m1-reject|m1-serial|m1-stalekey|m1|m2-hello|m2-kwrite|m2-kexec>
         [--release] [--repeat N] [--accel kvm|tcg] [--timeout SECONDS]
  clean";

#[derive(Default)]
struct Args {
    positional: Vec<String>,
    release: bool,
    accel: Option<Accel>,
    cmdline: Option<String>,
    timeout: Option<u64>,
    repeat: Option<u32>,
    gui: bool,
    gdb: bool,
    monitor: bool,
    trace: bool,
}

fn parse(raw: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut it = raw.iter();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| it.next().cloned().ok_or(format!("{flag} needs a value"));
        match arg.as_str() {
            "--release" => args.release = true,
            "--gui" => args.gui = true,
            "--gdb" => args.gdb = true,
            "--monitor" => args.monitor = true,
            "--trace" => args.trace = true,
            "--accel" => args.accel = Some(Accel::parse(&value("--accel")?)?),
            "--cmdline" => args.cmdline = Some(value("--cmdline")?),
            "--timeout" => {
                args.timeout =
                    Some(value("--timeout")?.parse().map_err(|_| "--timeout needs seconds")?)
            }
            "--repeat" => {
                args.repeat = Some(value("--repeat")?.parse().map_err(|_| "--repeat needs a count")?)
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}\n{USAGE}")),
            _ => args.positional.push(arg.clone()),
        }
    }
    Ok(args)
}

fn run_interactive(args: &Args) -> Result<(), String> {
    check::require_present()?;
    let needs_tcg = args.trace || args.gdb;
    if needs_tcg && args.accel == Some(Accel::Kvm) {
        return Err("--trace and --gdb use software emulation; drop --accel kvm".to_string());
    }
    let accel = if needs_tcg { Accel::Tcg } else { args.accel.unwrap_or_else(Accel::default_for_host) };
    if let Some(text) = &args.cmdline {
        image::check_cmdline(text)?;
    }
    let art = build::build(args.release)?;
    let dir = util::cache_dir().join("run");
    image::make(&dir, &art, args.cmdline.as_deref(), true)?;
    let serial = if args.monitor { Serial::File } else { Serial::Stdio };
    if args.gdb {
        println!("QEMU is stopped and waits for: gdb build/artifacts/{}/KERNEL.ELF -x tools/gdb/kernel.gdb", art.profile);
    }
    let status = qemu::run(&Vm {
        dir: &dir,
        accel,
        timeout: None,
        serial,
        gui: args.gui,
        monitor: args.monitor,
        trace: args.trace,
        gdb: args.gdb,
    })?;
    let keep = util::workspace_root().join("build").join("run");
    fs::create_dir_all(&keep).map_err(|e| e.to_string())?;
    for file in ["qemu.log", "serial.log"] {
        let _ = fs::copy(dir.join(file), keep.join(file));
    }
    let _ = fs::copy(&art.loader, keep.join("BOOTX64.EFI"));
    let _ = fs::copy(&art.kernel, keep.join("KERNEL.ELF"));
    println!("\nRESULT run accel={} status={status} logs={}", accel.as_str(), keep.display());
    Ok(())
}

fn clean() -> Result<(), String> {
    let root = util::workspace_root();
    for dir in [root.join("build"), root.join("target"), util::cache_dir()] {
        if dir.exists() {
            // The cache directory holds Cargo's build directory, which contains
            // the running xtask binary; on Linux removing it while it runs is fine.
            fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            println!("removed {}", dir.display());
        }
    }
    Ok(())
}

fn dispatch(raw: &[String]) -> Result<(), String> {
    let args = parse(raw)?;
    let command = args.positional.first().map(String::as_str).ok_or(USAGE)?;
    match command {
        "setup-check" => check::setup_check(),
        "build" => build::build(args.release).map(|_| ()),
        "image" => {
            let art = build::build(args.release)?;
            let img = image::make(&util::cache_dir().join("image"), &art, args.cmdline.as_deref(), true)?;
            println!("{}", img.display());
            Ok(())
        }
        "run" => run_interactive(&args),
        "test" => {
            let name = args
                .positional
                .get(1)
                .ok_or_else(|| format!("test needs a name: {}", tests::NAMES))?;
            tests::run(
                name,
                &tests::TestOpts {
                    release: args.release,
                    accel: args.accel.unwrap_or_else(Accel::default_for_host),
                    timeout: args.timeout.unwrap_or(60),
                    repeat: args.repeat.unwrap_or(1).max(1),
                },
            )
        }
        "clean" => clean(),
        other => Err(format!("unknown command {other}\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&raw) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("xtask: {msg}");
            ExitCode::FAILURE
        }
    }
}
