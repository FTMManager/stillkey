//! `setup-check`: tool versions, firmware hashes and the facts a run depends on.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::util::{sha256_hex, workspace_root};

pub const OVMF_CODE: &str = "/usr/share/OVMF/OVMF_CODE_4M.fd";
pub const OVMF_VARS: &str = "/usr/share/OVMF/OVMF_VARS_4M.fd";

const TOOLS: &[(&str, &[&str])] = &[
    ("rustc", &["--version"]),
    ("cargo", &["--version"]),
    ("qemu-system-x86_64", &["--version"]),
    ("gdb", &["--version"]),
    ("mtools", &["--version"]),
    ("sgdisk", &["--version"]),
    ("timeout", &["--version"]),
];

fn first_line(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// True if this user can open `/dev/kvm` for reading and writing.
pub fn kvm_usable() -> bool {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
}

/// The lines of the report, and whether every required item is present.
pub fn report() -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let mut ok = true;
    for (tool, args) in TOOLS {
        match first_line(tool, args) {
            Some(v) => lines.push(format!("tool {tool}: {v}")),
            None => {
                ok = false;
                lines.push(format!("tool {tool}: MISSING"));
            }
        }
    }
    match first_line("dpkg-query", &["-W", "-f=${Version}", "ovmf"]) {
        Some(v) => lines.push(format!("package ovmf: {v}")),
        None => lines.push("package ovmf: version unknown".to_string()),
    }
    for path in [OVMF_CODE, OVMF_VARS] {
        match fs::read(path) {
            Ok(bytes) => lines.push(format!("firmware {path}: sha256={}", sha256_hex(&bytes))),
            Err(_) => {
                ok = false;
                lines.push(format!("firmware {path}: MISSING"));
            }
        }
    }
    lines.push(format!(
        "kvm: {}",
        if kvm_usable() {
            "usable"
        } else {
            "not usable by this user (software emulation only)"
        }
    ));
    match std::env::var("CARGO_TARGET_DIR") {
        Ok(dir) if !dir.starts_with("/mnt/") => lines.push(format!("target dir: {dir}")),
        Ok(dir) => lines.push(format!(
            "target dir: {dir}  WARNING: on the Windows drive; use tools/sk.sh or tools/sk.ps1"
        )),
        Err(_) => lines.push(
            "target dir: CARGO_TARGET_DIR not set  WARNING: use tools/sk.sh or tools/sk.ps1"
                .to_string(),
        ),
    }
    (lines, ok)
}

/// The lines of the report, gathered once per `xtask` process: the header of
/// every run's `qemu.log`.
pub fn report_once() -> &'static [String] {
    static LINES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    LINES.get_or_init(|| report().0)
}

/// Fails with a message if a tool or firmware file that a VM run needs is absent.
pub fn require_present() -> Result<(), String> {
    let (lines, ok) = report();
    if ok {
        return Ok(());
    }
    let missing: Vec<&String> = lines.iter().filter(|l| l.ends_with("MISSING")).collect();
    Err(format!(
        "setup incomplete ({} item(s)); run tools/setup-wsl.sh: {:?}",
        missing.len(),
        missing
    ))
}

pub fn setup_check() -> Result<(), String> {
    let (lines, ok) = report();
    for l in &lines {
        println!("{l}");
    }
    let root = workspace_root();
    for (pkg, target) in [
        ("stillkey-loader", "x86_64-unknown-uefi"),
        ("stillkey-kernel", "x86_64-unknown-none"),
    ] {
        println!("-- crates inside {pkg} ({target})");
        let status = Command::new("cargo")
            .current_dir(&root)
            .args(["tree", "--locked", "-p", pkg, "--target", target])
            .status()
            .map_err(|e| format!("cargo tree: {e}"))?;
        if !status.success() {
            return Err(format!("cargo tree failed for {pkg}"));
        }
    }
    if !Path::new(&root).join("Cargo.lock").exists() {
        return Err("Cargo.lock is missing".to_string());
    }
    if ok {
        println!("setup-check: ok");
        Ok(())
    } else {
        Err("setup-check: a required tool or firmware file is missing".to_string())
    }
}
