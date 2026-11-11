//! `test`: the acceptance tests of docs/07 section 9. `m1-unit`, `m1-repro`
//! and `m1-screen` run once; `m1-boot`, `m1-panic`, `m1-pge`, `m1-reject`,
//! `m1-serial` and `m1-stalekey` are the boot tests that `--repeat` repeats;
//! `m1` is all of them.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::build::{self, Artifacts};
use crate::inspect::{self, Corruption};
use crate::qemu::{self, Accel, Serial, Vm};
use crate::util::{cache_dir, sha256_hex, workspace_root};
use crate::{bootfs, check, image, m2, screen};

pub const NAMES: &str =
    "m1-unit, m1-repro, m1-screen, m1-boot, m1-panic, m1-pge, m1-reject, m1-serial, m1-stalekey, m1, \
     m2-unit, m2-repro, m2-screen, m2-hello, m2-crash, m2-preempt, m2-prober, m2-kwrite, m2-kexec, m2-badfs, \
     m2 or all";

pub struct TestOpts {
    pub release: bool,
    pub accel: Accel,
    pub timeout: u64,
    pub repeat: u32,
}

impl TestOpts {
    pub fn profile(&self) -> &'static str {
        if self.release { "release" } else { "dev" }
    }
}

pub fn run(name: &str, o: &TestOpts) -> Result<(), String> {
    let boots = |tests: &[BootTest]| {
        check::require_present()
            .and_then(|_| build::build(o.release))
            .and_then(|art| repeated(o, &art, tests))
    };
    let result = match name {
        "m1-unit" => {
            host_tests("m1-unit", &M1_CRATES).and_then(|_| build::build(o.release)).and_then(|art| artifacts(&art))
        }
        "m1-repro" => repro("m1-repro", &M1_PARTS),
        "m2-unit" => host_tests("m2-unit", &M2_CRATES)
            .and_then(|_| build::build(o.release))
            .and_then(|art| bootfs_artifact(&art)),
        "m2-repro" => repro("m2-repro", &M2_PARTS),
        "m1-screen" => check::require_present()
            .and_then(|_| build::build(o.release))
            .and_then(|art| screen_boot(o, &art)),
        "m1-boot" => boots(&[boot]),
        "m1-panic" => boots(&[panic_boot]),
        "m1-pge" => boots(&[pge_boot]),
        "m1-reject" => boots(&[reject_boot]),
        "m1-serial" => boots(&[serial_boot]),
        "m1-stalekey" => boots(&[stalekey_boot]),
        "m2-hello" => boots(&[hello_boot]),
        "m2-screen" => check::require_present()
            .and_then(|_| build::build(o.release))
            .and_then(|art| m2::screen_boot(o, &art)),
        "m2-crash" => boots(&[m2::crash_boot]),
        "m2-preempt" => boots(&[m2::preempt_boot]),
        "m2-prober" => boots(&[m2::prober_boot]),
        "m2-kwrite" => boots(&[kwrite_boot]),
        "m2-kexec" => boots(&[kexec_boot]),
        "m2-badfs" => boots(&[badfs_boot]),
        "m1" => check::require_present().and_then(|_| m1_suite(o)),
        "m2" => check::require_present().and_then(|_| m2_suite(o)),
        "all" => check::require_present().and_then(|_| m1_suite(o)).and_then(|_| m2_suite(o)),
        other => Err(format!("unknown test `{other}`: expected {NAMES}")),
    };
    // `m1-repro` always builds the release profile; neither it nor the host
    // tests start a VM.
    let profile = if matches!(name, "m1-repro" | "m2-repro") { "release" } else { o.profile() };
    let accel = match name {
        "m1-unit" | "m1-repro" | "m2-unit" | "m2-repro" => "none",
        // Software emulation is the only accelerator this user can use.
        _ if o.accel == Accel::Tcg && !check::kvm_usable() => "tcg only",
        _ => o.accel.as_str(),
    };
    println!(
        "SUMMARY test={name} profile={profile} repeat={} result={} accel={accel}",
        o.repeat,
        if result.is_ok() { "pass" } else { "fail" },
    );
    result
}

type BootTest = fn(&TestOpts, &Artifacts, u32) -> Result<(), String>;

fn repeated(o: &TestOpts, art: &Artifacts, tests: &[BootTest]) -> Result<(), String> {
    for round in 1..=o.repeat {
        for test in tests {
            test(o, art, round)?;
        }
    }
    Ok(())
}

// ---- m1-unit ---------------------------------------------------------------

/// The crates whose host tests `m1-unit` runs, and those of `m2-unit`: the
/// new library crates and the ones of Milestone 1 (docs/10 section 10).
const M1_CRATES: [&str; 3] = ["sk-elf", "stillkey-bootinfo", "xtask"];
const M2_CRATES: [&str; 6] = ["stillkey-abi", "sk-bootfs", "sk-kcore", "sk-elf", "stillkey-bootinfo", "xtask"];

/// Part 1 of `m1-unit` and `m2-unit`: the host tests of the library crates
/// and of this tool. Fails if a crate ran zero tests.
fn host_tests(test: &str, crates: &[&str]) -> Result<(), String> {
    for krate in crates {
        let out = build::cargo()
            .args(["test", "--locked", "-p", krate])
            .output()
            .map_err(|e| format!("cargo test: {e}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        let passed: u64 = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("test result: ok. "))
            .filter_map(|rest| rest.split(' ').next()?.parse::<u64>().ok())
            .sum();
        let ok = out.status.success() && passed > 0;
        if !ok {
            print!("{text}");
        }
        println!(
            "RESULT test={test} crate={krate} passed={passed} result={}",
            if ok { "pass" } else { "fail" }
        );
        if !out.status.success() {
            return Err(format!("host tests of {krate} failed"));
        }
        if passed == 0 {
            return Err(format!("{krate} ran zero tests"));
        }
    }
    Ok(())
}

/// Part 2: the header properties of the two binaries of the selected profile.
fn artifacts(art: &Artifacts) -> Result<(), String> {
    type Check = fn(&[u8]) -> Result<String, String>;
    let checks: [(&str, &Path, Check); 2] = [
        ("BOOTX64.EFI", &art.loader, inspect::check_loader),
        ("KERNEL.ELF", &art.kernel, inspect::check_kernel),
    ];
    for (name, path, check) in checks {
        let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        match check(&bytes) {
            Ok(fields) => println!(
                "RESULT test=m1-unit artifact={name} profile={} {fields} result=pass",
                art.profile
            ),
            Err(why) => {
                println!(
                    "RESULT test=m1-unit artifact={name} profile={} result=fail",
                    art.profile
                );
                println!("  FAILED: {why}");
                return Err(format!("{name}: {why}"));
            }
        }
    }
    Ok(())
}

// ---- m2-unit ---------------------------------------------------------------

/// The entry of `BOOTFS.IMG` that the kernel must refuse to start.
const BAD_ELF: &str = "bin/bad-elf";

/// Part 2 of `m2-unit`: what `BOOTFS.IMG` holds. The image parses; every
/// entry's digest is that of its bytes; every program but `bin/bad-elf` is
/// one the kernel would start; `bin/bad-elf` is refused for its writable
/// and executable segment; no program names a directory of this machine.
fn check_bootfs(image: &[u8], host_paths: &[(&'static str, PathBuf)]) -> Result<String, String> {
    let parsed = sk_bootfs::parse(image).map_err(|reason| format!("the image is refused: {}", reason.as_str()))?;
    let (mut entries, mut programs, mut refused) = (0, 0, false);
    for entry in parsed.entries() {
        entries += 1;
        let data = parsed.data(&entry);
        let recorded: String = entry.sha256.iter().map(|byte| format!("{byte:02x}")).collect();
        if sha256_hex(data) != recorded {
            return Err(format!("{}: the digest in the image is not that of its bytes", entry.path));
        }
        if entry.kind != sk_bootfs::KIND_PROGRAM {
            continue;
        }
        programs += 1;
        let limit = match entry.mem_limit_mib {
            0 => stillkey_abi::DEFAULT_MEM_LIMIT,
            mib => u64::from(mib) << 20,
        };
        let verdict = sk_elf::validate(data, &sk_elf::user_policy(limit));
        if entry.path == BAD_ELF {
            match verdict {
                Err(reason) if reason.as_str() == "wx" => refused = true,
                Err(reason) => return Err(format!("{BAD_ELF} is refused with {}, expected wx", reason.as_str())),
                Ok(_) => return Err(format!("{BAD_ELF} is a loadable program")),
            }
        } else if let Err(reason) = verdict {
            return Err(format!("{}: not a loadable program: {}", entry.path, reason.as_str()));
        }
        let named = host_paths.iter().find(|(_, path)| contains(data, path.as_os_str().as_encoded_bytes()));
        if let Some((what, _)) = named {
            return Err(format!("{} contains the path of: {what}", entry.path));
        }
    }
    if !refused {
        return Err(format!("the image has no {BAD_ELF}"));
    }
    Ok(format!("entries={entries} programs={programs} refused=bad-elf:wx host_paths=none"))
}

fn bootfs_artifact(art: &Artifacts) -> Result<(), String> {
    let image = fs::read(&art.bootfs).map_err(|e| format!("{}: {e}", art.bootfs.display()))?;
    match check_bootfs(&image, &host_paths()?) {
        Ok(fields) => {
            println!("RESULT test=m2-unit artifact=BOOTFS.IMG profile={} {fields} result=pass", art.profile);
            Ok(())
        }
        Err(why) => {
            println!("RESULT test=m2-unit artifact=BOOTFS.IMG profile={} result=fail", art.profile);
            println!("  FAILED: {why}");
            Err(format!("BOOTFS.IMG: {why}"))
        }
    }
}

// ---- The suites ------------------------------------------------------------

/// `m1`: every test of Milestone 1 (docs/08 section 9).
fn m1_suite(o: &TestOpts) -> Result<(), String> {
    host_tests("m1-unit", &M1_CRATES)?;
    let art = build::build(o.release)?;
    artifacts(&art)?;
    repro("m1-repro", &M1_PARTS)?;
    screen_boot(o, &art)?;
    repeated(o, &art, &[boot, panic_boot, pge_boot, reject_boot, serial_boot, stalekey_boot])
}

/// `m2`: every test of Milestone 2 in the order of docs/10 section 10: the
/// three that run once, then the boot tests `--repeat` times.
fn m2_suite(o: &TestOpts) -> Result<(), String> {
    host_tests("m2-unit", &M2_CRATES)?;
    let art = build::build(o.release)?;
    bootfs_artifact(&art)?;
    repro("m2-repro", &M2_PARTS)?;
    m2::screen_boot(o, &art)?;
    repeated(
        o,
        &art,
        &[hello_boot, m2::crash_boot, m2::preempt_boot, m2::prober_boot, kwrite_boot, kexec_boot, badfs_boot],
    )
}

// ---- m1-repro, m2-repro ----------------------------------------------------

/// True if `needle` occurs in `haystack`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|window| window == needle)
}

/// The directories of this machine that must not be recorded in a binary.
fn host_paths() -> Result<Vec<(&'static str, PathBuf)>, String> {
    let mut paths = vec![("workspace", workspace_root())];
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(("home", PathBuf::from(home)));
    }
    paths.push(("cargo home", build::cargo_home()?));
    Ok(paths)
}

/// What one release build gives.
struct Rebuilt {
    loader: Vec<u8>,
    kernel: Vec<u8>,
    bootfs: Vec<u8>,
}

/// A file of a build, by its name on the boot volume.
type Part = (&'static str, fn(&Rebuilt) -> &[u8]);

fn loader_of(built: &Rebuilt) -> &[u8] {
    &built.loader
}

fn kernel_of(built: &Rebuilt) -> &[u8] {
    &built.kernel
}

fn bootfs_of(built: &Rebuilt) -> &[u8] {
    &built.bootfs
}

const M1_PARTS: [Part; 2] = [("BOOTX64.EFI", loader_of), ("KERNEL.ELF", kernel_of)];
const M2_PARTS: [Part; 1] = [("BOOTFS.IMG", bootfs_of)];

/// Where the files of the two builds are kept for a look after a failure.
fn repro_keep() -> PathBuf {
    workspace_root().join("build").join("m1-repro")
}

/// Two release builds into two fresh, different build directories. They are
/// made once per run of this tool: `m1-repro` and `m2-repro` compare
/// different files of the same two builds.
fn rebuilt() -> Result<&'static [Rebuilt], String> {
    static BUILDS: OnceLock<Result<Vec<Rebuilt>, String>> = OnceLock::new();
    BUILDS
        .get_or_init(|| -> Result<Vec<Rebuilt>, String> {
            let run = cache_dir().join("m1-repro");
            let rows = bootfs::read_manifest()?;
            let mut builds = Vec::new();
            for name in ["repro-a", "repro-b"] {
                let dir = run.join(name);
                if dir.exists() {
                    fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                }
                let built = build::compile(true, Some(&dir))?;
                let loader = fs::read(&built.loader).map_err(|e| format!("{}: {e}", built.loader.display()))?;
                let kernel = fs::read(&built.kernel).map_err(|e| format!("{}: {e}", built.kernel.display()))?;
                let bootfs = bootfs::pack(&rows, &built.programs)?;
                let kept = repro_keep().join(name);
                fs::create_dir_all(&kept).map_err(|e| e.to_string())?;
                let _ = fs::write(kept.join("BOOTX64.EFI"), &loader);
                let _ = fs::write(kept.join("KERNEL.ELF"), &kernel);
                let _ = fs::write(kept.join("BOOTFS.IMG"), &bootfs);
                builds.push(Rebuilt { loader, kernel, bootfs });
            }
            Ok(builds)
        })
        .as_deref()
        .map_err(String::clone)
}

/// `m1-repro` and `m2-repro`: each named file is the same bytes in both
/// builds, and none names a directory of this machine.
fn repro(test: &str, parts: &[Part]) -> Result<(), String> {
    let builds = rebuilt()?;
    let keep = repro_keep();
    let host_paths = host_paths()?;
    let mut verdict = Ok(());
    for (name, part) in parts {
        let (first, second) = (part(&builds[0]), part(&builds[1]));
        let same = first == second;
        let named: Vec<&str> = host_paths
            .iter()
            .filter(|(_, path)| {
                let needle = path.as_os_str().as_encoded_bytes();
                contains(first, needle) || contains(second, needle)
            })
            .map(|(what, _)| *what)
            .collect();
        let ok = same && named.is_empty();
        println!(
            "RESULT test={test} artifact={name} sha256_a={} sha256_b={} host_paths={} result={}",
            sha256_hex(first),
            sha256_hex(second),
            if named.is_empty() { "none".to_string() } else { named.join("+").replace(' ', "_") },
            if ok { "pass" } else { "fail" },
        );
        if !same {
            println!("  FAILED: the two builds of {name} differ");
            println!("  files: {}", keep.display());
            verdict = Err(format!("the two builds of {name} differ"));
        } else if !named.is_empty() {
            println!("  FAILED: {name} contains the path of: {}", named.join(", "));
            println!("  files: {}", keep.display());
            verdict = Err(format!("{name} contains a path of this machine"));
        }
    }
    verdict
}

// ---- Boot tests ------------------------------------------------------------

pub struct Outcome {
    pub status: i32,
    pub lines: Vec<String>,
    pub firmware_lines: usize,
}

/// The start of a run's `qemu.log` (docs/07 section 5): the `setup-check`
/// tool versions and the hashes of the two firmware files, before the
/// `accel=` line and the command. A log without them cannot say which QEMU
/// and which firmware produced it.
fn check_log_header(log: &str) -> Result<(), String> {
    if !log.lines().any(|l| l.starts_with("accel=")) {
        return Err("qemu.log has no `accel=` line".to_string());
    }
    let head: Vec<&str> = log.lines().take_while(|l| !l.starts_with("accel=")).collect();
    let qemu = "tool qemu-system-x86_64: ";
    if !head.iter().any(|l| l.starts_with(qemu) && !l.ends_with("MISSING")) {
        return Err("qemu.log does not start with the version of qemu-system-x86_64".to_string());
    }
    for file in [check::OVMF_CODE, check::OVMF_VARS] {
        let prefix = format!("firmware {file}: sha256=");
        if !head.iter().any(|l| l.starts_with(&prefix)) {
            return Err(format!("qemu.log does not start with the hash of firmware {file}"));
        }
    }
    Ok(())
}

/// Removes terminal escape sequences and carriage returns, then keeps the
/// lines that start with `SK:`. Also counts the non-empty lines firmware
/// printed before the first of them.
pub fn sk_lines(raw: &[u8]) -> (Vec<String>, usize) {
    let mut clean = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            0x1B => {
                i += 1;
                if raw.get(i) == Some(&b'[') {
                    i += 1;
                    while i < raw.len() && !(0x40..=0x7E).contains(&raw[i]) {
                        i += 1;
                    }
                }
                i += 1; // the final byte of the sequence
            }
            b'\r' => i += 1,
            byte => {
                clean.push(byte);
                i += 1;
            }
        }
    }
    let text = String::from_utf8_lossy(&clean);
    let mut lines = Vec::new();
    let mut firmware_lines = 0;
    for line in text.split('\n') {
        if line.starts_with("SK:") {
            lines.push(line.to_string());
        } else if lines.is_empty() && !line.trim().is_empty() {
            firmware_lines += 1;
        }
    }
    (lines, firmware_lines)
}

/// Each line of the file is a required prefix, in order; other `SK:` lines may
/// come in between. A line starting with `!` is text no `SK:` line may contain.
pub fn check_expect(lines: &[String], name: &str) -> Result<(), String> {
    let path = workspace_root().join("tests").join(format!("{name}.expect"));
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    expect_text(lines, &text)
}

/// `check_expect` on the text of an expect file.
fn expect_text(lines: &[String], text: &str) -> Result<(), String> {
    let mut next = 0;
    for want in text.lines().filter(|l| !l.is_empty()) {
        if let Some(forbidden) = want.strip_prefix('!') {
            if let Some(line) = lines.iter().find(|l| l.contains(forbidden)) {
                return Err(format!("forbidden text `{forbidden}` in line: {line}"));
            }
            continue;
        }
        match lines[next..].iter().position(|l| l.starts_with(want)) {
            Some(offset) => next += offset + 1,
            None => return Err(format!("no line starting with `{want}` after line {next}")),
        }
    }
    Ok(())
}

/// The value of `key=` in the first line that starts with `prefix`.
fn field<'a>(lines: &'a [String], prefix: &str, key: &str) -> Result<&'a str, String> {
    let line = lines
        .iter()
        .find(|l| l.starts_with(prefix))
        .ok_or_else(|| format!("no line starting with `{prefix}`"))?;
    let key_eq = format!("{key}=");
    line.split(' ')
        .find_map(|word| word.strip_prefix(key_eq.as_str()))
        .ok_or_else(|| format!("no `{key}=` in line: {line}"))
}

fn expect_field(lines: &[String], prefix: &str, key: &str, want: &str) -> Result<(), String> {
    let got = field(lines, prefix, key)?;
    if got == want { Ok(()) } else { Err(format!("{prefix} {key}={got}, expected {want}")) }
}

pub fn expect_status(outcome: &Outcome, want: i32) -> Result<(), String> {
    if outcome.status == want {
        Ok(())
    } else {
        Err(format!("QEMU exit status {}, expected {want}", outcome.status))
    }
}

/// Copies the files of a run to `build/<name>`, where Windows tools can read
/// them, and returns that folder.
pub fn keep_evidence(dir: &Path, name: &str, art: &Artifacts, lines: &[String]) -> Result<PathBuf, String> {
    let keep = workspace_root().join("build").join(name);
    fs::create_dir_all(&keep).map_err(|e| e.to_string())?;
    for file in ["serial.log", "qemu.log", "screen.ppm"] {
        let _ = fs::copy(dir.join(file), keep.join(file));
    }
    let _ = fs::copy(&art.loader, keep.join("BOOTX64.EFI"));
    let _ = fs::copy(&art.kernel, keep.join("KERNEL.ELF"));
    let _ = fs::copy(&art.bootfs, keep.join("BOOTFS.IMG"));
    let sk_text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let _ = fs::write(keep.join("sk-lines.txt"), sk_text);
    Ok(keep)
}

/// Boots an image made from `art` and the boot options. `name` names the run
/// directory under the cache and the evidence folder under `build`; it may
/// have two parts, as in `m1-reject/ident`.
fn boot_vm(
    o: &TestOpts,
    art: &Artifacts,
    name: &str,
    cmdline: &str,
) -> Result<(Outcome, PathBuf), String> {
    boot_image(o, art, name, cmdline, false)
}

/// Boots an image, with or without `BOOTFS.IMG`, until the guest ends the VM.
fn boot_image(
    o: &TestOpts,
    art: &Artifacts,
    name: &str,
    cmdline: &str,
    with_bootfs: bool,
) -> Result<(Outcome, PathBuf), String> {
    let dir = cache_dir().join(name);
    image::make(&dir, art, Some(cmdline), with_bootfs)?;
    let status = qemu::run(&Vm {
        dir: &dir,
        accel: o.accel,
        timeout: Some(o.timeout),
        serial: Serial::File,
        gui: false,
        monitor: false,
        trace: false,
        gdb: false,
    })?;
    let raw = fs::read(dir.join("serial.log")).unwrap_or_default();
    let (lines, firmware_lines) = sk_lines(&raw);
    let keep = keep_evidence(&dir, name, art, &lines)?;
    Ok((Outcome { status, lines, firmware_lines }, keep))
}

/// Prints the result line of one run; `name` is the test, with ` image=<x>`
/// after it where a test boots several images.
pub fn report(
    o: &TestOpts,
    name: &str,
    round: u32,
    outcome: &Outcome,
    kept: &Path,
    verdict: &Result<(), String>,
) {
    if verdict.is_err() || round == 1 {
        for line in &outcome.lines {
            println!("  {line}");
        }
    }
    println!(
        "RESULT test={name} run={round}/{} profile={} accel={} status={} firmware_lines={} result={}",
        o.repeat,
        o.profile(),
        o.accel.as_str(),
        outcome.status,
        outcome.firmware_lines,
        if verdict.is_ok() { "pass" } else { "fail" },
    );
    if let Err(why) = verdict {
        println!("  FAILED: {why}");
        println!("  logs: {}", kept.display());
    }
}

/// A number field of the first line that starts with `prefix`.
fn number(lines: &[String], prefix: &str, key: &str) -> Result<u64, String> {
    let text = field(lines, prefix, key)?;
    text.parse().map_err(|_| format!("{prefix}: {key}={text} is not a number"))
}

/// `m2-hello`, checkpoint 1 of Milestone 2 (docs/10 section 11.1): the timer
/// fires, and one ring-3 program prints through the kernel and exits. The
/// kernel does not end the VM; the harness does, after the hundredth tick.
fn hello_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    let name = "m2-hello";
    let dir = cache_dir().join(name);
    image::make(&dir, art, Some("qemu_exit=on test=m2-hello"), true)?;
    let status = qemu::quit_after(
        &Vm {
            dir: &dir,
            accel: o.accel,
            timeout: Some(o.timeout),
            serial: Serial::File,
            gui: false,
            monitor: true,
            trace: false,
            gdb: false,
        },
        "SK:KERNEL:TIMER_OK",
    )?;
    let raw = fs::read(dir.join("serial.log")).unwrap_or_default();
    let (lines, firmware_lines) = sk_lines(crate::follow::whole(&raw));
    let kept = keep_evidence(&dir, name, art, &lines)?;
    let outcome = Outcome { status, lines, firmware_lines };
    let lines = &outcome.lines;

    let verdict = (|| {
        check_expect(lines, name)?;
        // The harness ended the VM: the kernel neither panicked nor stopped.
        expect_status(&outcome, 0)?;

        // The loader read the image this build packed.
        let digest = format!("SK:LOADER:BOOTFS_SHA256 {}", art.bootfs_sha256);
        if !lines.contains(&digest) {
            return Err(format!("no line `{digest}`"));
        }

        frames_add_up(lines)?;

        // Everything the program had came back: its image pages, sixteen
        // stack pages, and seven page tables (the top level, and three
        // levels each for the image and the stack).
        let hello = art.bootfs.with_file_name("bin").join("hello");
        let file = fs::read(&hello).map_err(|e| format!("{}: {e}", hello.display()))?;
        let image = sk_elf::validate(&file, &sk_elf::user_policy(stillkey_abi::DEFAULT_MEM_LIMIT))
            .map_err(|reason| format!("bin/hello: {}", reason.as_str()))?;
        let image_pages: u64 = image.segments().iter().map(|s| s.memsz.div_ceil(4096)).sum();
        let want = image_pages + stillkey_abi::USER_STACK_SIZE / 4096 + 7;
        let reaped = number(lines, "SK:KERNEL:REAPED", "frames")?;
        if reaped != want {
            return Err(format!("REAPED frames={reaped}, expected {want} ({image_pages} image pages)"));
        }
        Ok(())
    })();
    report(o, name, round, &outcome, &kept, &verdict);
    verdict
}

/// In every Milestone 2 run the loader's digest of `BOOTFS.IMG` is the one
/// the host computed (docs/10 section 10).
fn loader_digest(lines: &[String], art: &Artifacts) -> Result<(), String> {
    let digest = format!("SK:LOADER:BOOTFS_SHA256 {}", art.bootfs_sha256);
    if lines.contains(&digest) { Ok(()) } else { Err(format!("no line `{digest}`")) }
}

/// `m2-badfs` (docs/10 section 10): the boot volume holds a `BOOTFS.IMG`
/// whose first byte is wrong. The kernel must refuse it by name, start no
/// program and stop.
fn badfs_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    let name = "m2-badfs";
    let mut image = fs::read(&art.bootfs).map_err(|e| format!("{}: {e}", art.bootfs.display()))?;
    *image.first_mut().ok_or("BOOTFS.IMG is empty")? ^= 0xFF;
    let dir = cache_dir().join("m2-badfs-image");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let bootfs = dir.join("BOOTFS.IMG");
    fs::write(&bootfs, &image).map_err(|e| format!("{}: {e}", bootfs.display()))?;
    let bad = Artifacts {
        loader: art.loader.clone(),
        kernel: art.kernel.clone(),
        profile: art.profile,
        kernel_sha256: art.kernel_sha256.clone(),
        bootfs,
        bootfs_sha256: sha256_hex(&image),
    };
    let (outcome, kept) = boot_image(o, &bad, name, "qemu_exit=on", true)?;
    let lines = &outcome.lines;
    let verdict = (|| {
        check_expect(lines, name)?;
        expect_status(&outcome, 37)?;
        loader_digest(lines, &bad)?;
        frames_add_up(lines)
    })();
    report(o, name, round, &outcome, &kept, &verdict);
    verdict
}

/// `m2-kwrite` and `m2-kexec` (docs/10 sections 8.6 and 10): the kernel
/// writes to its own code, or calls its own data. The processor must refuse
/// with a page fault whose error code says "the page is there, and this
/// access is not allowed", at the address the kernel announced.
fn kernel_fault_boot(o: &TestOpts, art: &Artifacts, round: u32, name: &str, selftest: &str) -> Result<(), String> {
    let cmdline = format!("qemu_exit=on test={selftest}");
    let (outcome, kept) = boot_image(o, art, name, &cmdline, true)?;
    let lines = &outcome.lines;
    let verdict = (|| {
        check_expect(lines, name)?;
        expect_status(&outcome, 37)?;
        loader_digest(lines, art)?;
        frames_add_up(lines)?;
        let announced = hex_field(lines, "SK:KERNEL:SELFTEST", "addr")?;
        let banner = "SK:KERNEL:PANIC exception";
        let (error, addr, rip) =
            (hex_field(lines, banner, "error")?, hex_field(lines, banner, "addr")?, hex_field(lines, banner, "rip")?);
        refused_access(selftest, error, addr, rip, announced)
    })();
    report(o, name, round, &outcome, &kept, &verdict);
    verdict
}

/// Whether a page fault in the kernel is the processor refusing what the
/// self-test `kwrite_text` or `kexec_data` tried at the address it announced.
fn refused_access(selftest: &str, error: u64, addr: u64, rip: u64, announced: u64) -> Result<(), String> {
    if addr != announced {
        return Err(format!("the fault is at {addr:#x}, the self-test announced {announced:#x}"));
    }
    // Bit 0: the page is present. Bit 1: a write. Bit 2: from ring 3.
    // Bit 4: an instruction fetch.
    let (want, mask) = if selftest == "kwrite_text" { (0b00011, 0b00111) } else { (0b10001, 0b10101) };
    if error & mask != want {
        return Err(format!("error code {error:#x} is not that of a refused {selftest}"));
    }
    if selftest == "kexec_data" && rip != announced {
        return Err(format!("the fetch faulted at rip {rip:#x}, not at {announced:#x}"));
    }
    Ok(())
}

fn kwrite_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    kernel_fault_boot(o, art, round, "m2-kwrite", "kwrite_text")
}

fn kexec_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    kernel_fault_boot(o, art, round, "m2-kexec", "kexec_data")
}

/// The allocator accounts for every page the handoff calls usable.
pub fn frames_add_up(lines: &[String]) -> Result<(), String> {
    let usable = number(lines, "SK:KERNEL:MEM usable_after_reserve", "pages")?;
    let free = number(lines, "SK:KERNEL:FRAMES", "free")?;
    let bitmap = number(lines, "SK:KERNEL:FRAMES", "bitmap_pages")?;
    if free + bitmap != usable {
        return Err(format!("free {free} + bitmap {bitmap} is not usable_after_reserve {usable}"));
    }
    Ok(())
}

/// The mode in the kernel's `FB_FILLED <w>x<h> format=<f>` line.
fn filled_mode(lines: &[String]) -> Result<&str, String> {
    lines
        .iter()
        .find_map(|l| l.strip_prefix("SK:KERNEL:FB_FILLED "))
        .and_then(|rest| rest.split(' ').next())
        .ok_or_else(|| "no FB_FILLED line".to_string())
}

/// `m1-boot`: the normal boot ends at the kernel's success marker with status 33.
fn boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    let name = "m1-boot";
    let (outcome, kept) = boot_vm(o, art, name, "qemu_exit=on stop=m1")?;
    let lines = &outcome.lines;
    let verdict = (|| {
        check_expect(lines, name)?;
        expect_status(&outcome, 33)?;
        expect_field(lines, "SK:LOADER:START", "profile", o.profile())?;
        expect_field(lines, "SK:KERNEL:START", "profile", o.profile())?;
        // The loader read the file the build produced, and the kernel was told the same.
        let logged = lines
            .iter()
            .find_map(|l| l.strip_prefix("SK:LOADER:KERNEL_SHA256 "))
            .ok_or("no KERNEL_SHA256 line")?;
        if logged != art.kernel_sha256 {
            return Err(format!("loader hashed {logged}, the host {}", art.kernel_sha256));
        }
        expect_field(lines, "SK:KERNEL:IMAGE", "sha256", &art.kernel_sha256)?;
        expect_field(lines, "SK:KERNEL:MEM reserved", "ranges", "5")?;
        expect_field(lines, "SK:LOADER:PAGING", "direct_map_gib", "4")?;
        expect_field(lines, "SK:KERNEL:MEM direct_map_gib", "direct_map_gib", "4")?;
        let usable: u64 = field(lines, "SK:KERNEL:MEM usable_after_reserve", "pages")?
            .parse()
            .map_err(|_| "usable_after_reserve is not a number")?;
        if usable == 0 || usable >= 65_536 {
            return Err(format!("usable_after_reserve pages={usable} is outside 1..65535"));
        }
        // The kernel painted the mode the loader captured, in its byte order.
        let mode = field(lines, "SK:LOADER:GOP", "mode")?;
        let painted = filled_mode(lines)?;
        if painted != mode {
            return Err(format!("the kernel filled {painted}, the loader captured {mode}"));
        }
        let format = field(lines, "SK:LOADER:GOP", "format")?;
        expect_field(lines, "SK:KERNEL:FB_FILLED", "format", format)?;
        // The log of the run says which QEMU and which firmware produced it.
        let log = cache_dir().join(name).join("qemu.log");
        let log = fs::read_to_string(&log).map_err(|e| format!("{}: {e}", log.display()))?;
        check_log_header(&log)
    })();
    report(o, name, round, &outcome, &kept, &verdict);
    verdict
}

fn hex_field(lines: &[String], prefix: &str, key: &str) -> Result<u64, String> {
    let text = field(lines, prefix, key)?;
    u64::from_str_radix(text.trim_start_matches("0x"), 16)
        .map_err(|_| format!("{prefix} {key}={text} is not a hexadecimal number"))
}

/// `m1-pge`: the boot option `test=pge` sets `CR4.PGE` after `ExitBootServices`,
/// as firmware on real machines leaves it. The loader must clear exactly that
/// bit before it switches page tables, and the kernel must find it clear.
fn pge_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    const PGE: u64 = 1 << 7;
    let name = "m1-pge";
    let (outcome, kept) = boot_vm(o, art, name, "qemu_exit=on stop=m1 test=pge")?;
    let lines = &outcome.lines;
    let verdict = (|| {
        check_expect(lines, name)?;
        expect_status(&outcome, 33)?;
        let before = hex_field(lines, "SK:LOADER:CR4", "before")?;
        let after = hex_field(lines, "SK:LOADER:CR4", "after")?;
        if before & PGE == 0 {
            return Err(format!("CR4 before={before:#x}: the test did not set PGE"));
        }
        if after != before & !PGE {
            return Err(format!(
                "CR4 before={before:#x} after={after:#x}: expected only bit 7 to change"
            ));
        }
        Ok(())
    })();
    report(o, name, round, &outcome, &kept, &verdict);
    verdict
}

/// `m1-serial`: the boot option `test=serial` repeats the loader's check of
/// the serial port with the window between the loopback byte and its
/// read-back held open for 50 ms. Firmware polls the same port for console
/// input every 20 ms while boot services are active; if the loader does not
/// hold firmware off, firmware takes the byte and the working port looks
/// absent, which at boot would silence every loader line.
fn serial_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    let name = "m1-serial";
    let (outcome, kept) = boot_vm(o, art, name, "qemu_exit=on stop=m1 test=serial")?;
    let lines = &outcome.lines;
    let verdict = (|| {
        check_expect(lines, name)?;
        expect_status(&outcome, 33)?;
        expect_field(lines, "SK:LOADER:SERIAL", "recheck", "ok")
    })();
    report(o, name, round, &outcome, &kept, &verdict);
    verdict
}

/// `m1-stalekey`: the boot option `test=stalekey` makes the first memory-map
/// key stale. The loader must fetch the map again and still hand over.
fn stalekey_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    let name = "m1-stalekey";
    let (outcome, kept) = boot_vm(o, art, name, "qemu_exit=on stop=m1 test=stalekey")?;
    let lines = &outcome.lines;
    let verdict = (|| {
        check_expect(lines, name)?;
        expect_status(&outcome, 33)?;
        let attempts: u32 = field(lines, "SK:LOADER:EBS_OK", "attempts")?
            .parse()
            .map_err(|_| "EBS_OK attempts is not a number")?;
        if attempts < 2 {
            return Err(format!("EBS_OK attempts={attempts}: the stale key caused no second call"));
        }
        // The kernel was told the same count.
        expect_field(lines, "SK:KERNEL:M1_OK", "attempts", &attempts.to_string())
    })();
    report(o, name, round, &outcome, &kept, &verdict);
    verdict
}

/// `m1-panic`: the deliberate panic prints its banner and ends with status 37.
fn panic_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    let name = "m1-panic";
    let (outcome, kept) = boot_vm(o, art, name, "qemu_exit=on stop=m1 test=panic")?;
    let lines = &outcome.lines;
    let verdict = (|| {
        check_expect(lines, name)?;
        expect_status(&outcome, 37)?;
        let at = lines
            .iter()
            .find_map(|l| l.strip_prefix("SK:KERNEL:PANIC at "))
            .ok_or("no `PANIC at` line")?;
        let mut parts = at.splitn(3, ':');
        let file = parts.next().unwrap_or("");
        let line = parts.next().unwrap_or("");
        let message = parts.next().unwrap_or("");
        if !file.ends_with(".rs") || line.parse::<u32>().is_err() {
            return Err(format!("the panic line names no file and line: {at}"));
        }
        if message.trim() != "test=panic requested" {
            return Err(format!("unexpected panic message: {message}"));
        }
        Ok(())
    })();
    report(o, name, round, &outcome, &kept, &verdict);
    verdict
}

/// `m1-reject`: five images, each with one corruption of the real kernel. The
/// loader must refuse each with the reason of the rule that was broken, end
/// QEMU with the failure value (status 35) and never start the kernel. With
/// `qemu_exit=on` the port write ends the VM before the loader's return to
/// firmware runs, so that return is not exercised here.
fn reject_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    let good = fs::read(&art.kernel).map_err(|e| format!("{}: {e}", art.kernel.display()))?;
    for how in Corruption::ALL {
        let reason = how.reason();
        let name = format!("m1-reject/{reason}");
        let dir = cache_dir().join(&name);
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let bad = inspect::corrupt(&good, how)?;
        let kernel = dir.join("KERNEL.ELF");
        fs::write(&kernel, &bad).map_err(|e| format!("{}: {e}", kernel.display()))?;
        let corrupted = Artifacts {
            loader: art.loader.clone(),
            kernel,
            profile: art.profile,
            kernel_sha256: sha256_hex(&bad),
            bootfs: art.bootfs.clone(),
            bootfs_sha256: art.bootfs_sha256.clone(),
        };

        let (outcome, kept) = boot_vm(o, &corrupted, &name, "qemu_exit=on stop=m1")?;
        let lines = &outcome.lines;
        let verdict = (|| {
            check_expect(lines, "m1-reject")?;
            expect_status(&outcome, 35)?;
            expect_field(lines, "SK:LOADER:KERNEL_REJECTED", "reason", reason)?;
            // The loader judged the corrupted file, not another one.
            expect_field(lines, "SK:LOADER:KERNEL_FILE", "size", &bad.len().to_string())?;
            let logged = lines
                .iter()
                .find_map(|l| l.strip_prefix("SK:LOADER:KERNEL_SHA256 "))
                .ok_or("no KERNEL_SHA256 line")?;
            if logged != corrupted.kernel_sha256 {
                return Err(format!("loader hashed {logged}, the host {}", corrupted.kernel_sha256));
            }
            Ok(())
        })();
        report(o, &format!("m1-reject image={reason}"), round, &outcome, &kept, &verdict);
        verdict?;
    }
    Ok(())
}

/// `m1-screen`: boots without `qemu_exit=on`, so the kernel halts with the VM
/// up, takes QEMU's picture of the screen and compares every pixel with what
/// kernel step 5 paints.
fn screen_boot(o: &TestOpts, art: &Artifacts) -> Result<(), String> {
    let name = "m1-screen";
    let dir = cache_dir().join(name);
    image::make(&dir, art, Some("stop=m1"), false)?;
    let ppm = dir.join("screen.ppm");
    let status = qemu::capture_screen(
        &Vm {
            dir: &dir,
            accel: o.accel,
            timeout: Some(o.timeout),
            serial: Serial::File,
            gui: false,
            monitor: true,
            trace: false,
            gdb: false,
        },
        "SK:KERNEL:M1_OK",
        &ppm,
    )?;
    let raw = fs::read(dir.join("serial.log")).unwrap_or_default();
    let (lines, firmware_lines) = sk_lines(&raw);
    let kept = keep_evidence(&dir, name, art, &lines)?;
    let outcome = Outcome { status, lines, firmware_lines };
    let lines = &outcome.lines;

    let mut size = String::from("none");
    let verdict = (|| {
        if !lines.iter().any(|l| l.starts_with("SK:KERNEL:M1_OK")) {
            return Err("the kernel did not reach its success marker".to_string());
        }
        expect_status(&outcome, 0)?;
        let file = fs::read(&ppm).map_err(|e| format!("{}: {e}", ppm.display()))?;
        let picture = screen::parse_ppm(&file)?;
        size = format!("{}x{}", picture.width, picture.height);
        let painted = filled_mode(lines)?;
        if painted != size {
            return Err(format!("the kernel filled {painted}, the screen is {size}"));
        }
        screen::check_fill(&picture)
    })();
    println!(
        "RESULT test={name} profile={} accel={} status={} screen={size} band_rows={} result={}",
        o.profile(),
        o.accel.as_str(),
        outcome.status,
        screen::BAND_ROWS,
        if verdict.is_ok() { "pass" } else { "fail" },
    );
    if let Err(why) = &verdict {
        for line in lines {
            println!("  {line}");
        }
        println!("  FAILED: {why}");
        println!("  logs: {}", kept.display());
    }
    verdict
}

#[cfg(test)]
mod tests {
    use super::check_log_header;

    const HEADER: &str = "tool rustc: rustc 1.99.1 (abc 2026-10-01)
tool qemu-system-x86_64: QEMU emulator version 10.2.1
package ovmf: 2025.11-1
firmware /usr/share/OVMF/OVMF_CODE_4M.fd: sha256=aa11
firmware /usr/share/OVMF/OVMF_VARS_4M.fd: sha256=bb22
kvm: usable
accel=kvm
command: timeout -k 5 60 qemu-system-x86_64 -machine q35
";

    #[test]
    fn a_log_that_starts_with_versions_and_firmware_hashes_is_accepted() {
        assert_eq!(check_log_header(HEADER), Ok(()));
    }

    #[test]
    fn a_log_that_starts_at_the_accel_line_is_refused() {
        let bare = "accel=kvm
command: timeout -k 5 60 qemu-system-x86_64
";
        assert!(check_log_header(bare).unwrap_err().contains("qemu-system-x86_64"));
    }

    #[test]
    fn versions_after_the_accel_line_do_not_count() {
        let (versions, rest) = HEADER.split_at(HEADER.find("accel=").unwrap());
        let late = format!("{rest}{versions}");
        assert!(check_log_header(&late).is_err());
    }

    #[test]
    fn each_firmware_file_needs_its_hash() {
        let one = HEADER.replace("firmware /usr/share/OVMF/OVMF_VARS_4M.fd: sha256=bb22
", "");
        assert!(check_log_header(&one).unwrap_err().contains("firmware"));
        let missing = HEADER.replace("OVMF_VARS_4M.fd: sha256=bb22", "OVMF_VARS_4M.fd: MISSING");
        assert!(check_log_header(&missing).unwrap_err().contains("firmware"));
    }

    #[test]
    fn a_missing_qemu_version_is_refused() {
        let unknown = HEADER.replace("QEMU emulator version 10.2.1", "MISSING");
        assert!(check_log_header(&unknown).unwrap_err().contains("qemu-system-x86_64"));
    }

    #[test]
    fn a_log_without_the_accel_line_is_refused() {
        let cut = HEADER.replace("accel=kvm
", "");
        assert!(check_log_header(&cut).unwrap_err().contains("accel="));
    }
}

#[cfg(test)]
mod bootfs_tests {
    use sk_bootfs::build::{Source, pack};
    use sk_bootfs::{CLASS_APP, KIND_FILE, KIND_PROGRAM};

    use super::*;

    fn source(path: &str, kind: u32, data: &[u8]) -> Source {
        Source {
            path: path.to_string(),
            name: "x".to_string(),
            display_name: "X".to_string(),
            class: CLASS_APP,
            mem_limit_mib: 0,
            flags: 0,
            kind,
            data: data.to_vec(),
        }
    }

    fn checked(sources: &[Source]) -> Result<String, String> {
        check_bootfs(&pack(sources).unwrap(), &[])
    }

    #[test]
    fn an_image_without_the_refused_program_fails() {
        let error = checked(&[source("licenses/a.txt", KIND_FILE, b"text")]).unwrap_err();
        assert_eq!(error, "the image has no bin/bad-elf");
    }

    #[test]
    fn the_refused_program_must_be_refused_for_its_segment() {
        let error = checked(&[source(BAD_ELF, KIND_PROGRAM, b"not an ELF file")]).unwrap_err();
        assert!(error.starts_with("bin/bad-elf is refused with ") && error.ends_with("expected wx"), "{error}");
    }

    #[test]
    fn every_other_program_must_be_loadable() {
        let error = checked(&[source("bin/a", KIND_PROGRAM, b"not an ELF file")]).unwrap_err();
        assert!(error.starts_with("bin/a: not a loadable program: "), "{error}");
    }

    #[test]
    fn a_wrong_magic_fails() {
        let mut image = pack(&[source("licenses/a.txt", KIND_FILE, b"text")]).unwrap();
        image[0] ^= 0xFF;
        assert_eq!(check_bootfs(&image, &[]).unwrap_err(), "the image is refused: magic");
    }

    #[test]
    fn bytes_that_changed_after_packing_fail() {
        let mut image = pack(&[source("licenses/a.txt", KIND_FILE, b"text")]).unwrap();
        let at = image.windows(4).rposition(|window| window == b"text").unwrap();
        image[at] ^= 1;
        assert!(check_bootfs(&image, &[]).unwrap_err().contains("digest"));
    }
}

#[cfg(test)]
mod line_checks {
    use super::{expect_text, frames_add_up, refused_access, sk_lines};

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn only_sk_lines_are_kept_and_firmware_lines_before_them_counted() {
        let raw = b"BdsDxe: loading\r\n\r\nBdsDxe: starting\r\nSK:LOADER:START\r\nnoise\r\n at SK:X\r\nSK:KERNEL:ENTRY\r\n";
        assert_eq!(sk_lines(raw), (lines(&["SK:LOADER:START", "SK:KERNEL:ENTRY"]), 2));
        assert_eq!(sk_lines(b""), (Vec::new(), 0));
    }

    #[test]
    fn escape_sequences_are_removed_before_lines_are_told_apart() {
        // A sequence without parameters, one with parameters, and one of two bytes.
        let raw = b"\x1b[2J\x1b[01;01HSK:A one\x1b[0m\n\x1bcSK:B two\n";
        assert_eq!(sk_lines(raw).0, lines(&["SK:A one", "SK:B two"]));
    }

    #[test]
    fn an_escape_sequence_cut_at_the_end_is_dropped() {
        assert_eq!(sk_lines(b"SK:A\n\x1b["), (lines(&["SK:A"]), 0));
        assert_eq!(sk_lines(b"SK:A\n\x1b"), (lines(&["SK:A"]), 0));
        assert_eq!(sk_lines(b"SK:A\n\x1b[12"), (lines(&["SK:A"]), 0));
    }

    #[test]
    fn expected_prefixes_must_come_in_order() {
        let log = lines(&["SK:A one", "SK:X", "SK:B two", "SK:C"]);
        assert_eq!(expect_text(&log, "SK:A\n\nSK:B two\nSK:C\n"), Ok(()));
        assert_eq!(
            expect_text(&log, "SK:B\nSK:A\n"),
            Err("no line starting with `SK:A` after line 3".to_string())
        );
        assert_eq!(expect_text(&log, "SK:D\n"), Err("no line starting with `SK:D` after line 0".to_string()));
    }

    #[test]
    fn one_line_does_not_answer_for_two_expected_lines() {
        assert!(expect_text(&lines(&["SK:A one"]), "SK:A\nSK:A\n").is_err());
        assert_eq!(expect_text(&lines(&["SK:A one", "SK:A two"]), "SK:A\nSK:A\n"), Ok(()));
    }

    #[test]
    fn an_expected_line_is_a_prefix_not_a_part() {
        assert!(expect_text(&lines(&["SK:KERNEL:A SK:B"]), "SK:B\n").is_err());
    }

    #[test]
    fn forbidden_text_may_be_nowhere_in_any_line() {
        let log = lines(&["SK:A one", "SK:KERNEL:PANIC at x"]);
        assert_eq!(
            expect_text(&log, "SK:A\n!PANIC\n"),
            Err("forbidden text `PANIC` in line: SK:KERNEL:PANIC at x".to_string())
        );
        // Also in a line before the place the expected lines have reached.
        assert!(expect_text(&log, "SK:KERNEL:PANIC\n!one\n").is_err());
        assert_eq!(expect_text(&log, "SK:A\n!STOPPED\n"), Ok(()));
    }

    #[test]
    fn free_and_bitmap_pages_must_be_the_usable_pages() {
        let log = |free: u64| {
            lines(&[
                "SK:KERNEL:MEM usable_after_reserve pages=1000",
                &format!("SK:KERNEL:FRAMES total=1000 free={free} bitmap_pages=10"),
                "SK:KERNEL:FRAMES released=4",
            ])
        };
        assert_eq!(frames_add_up(&log(990)), Ok(()));
        assert!(frames_add_up(&log(989)).unwrap_err().contains("989"));
        assert!(frames_add_up(&log(991)).is_err());
        assert!(frames_add_up(&lines(&["SK:KERNEL:MEM usable_after_reserve pages=1000"])).is_err());
    }

    const AT: u64 = 0xffff_ffff_8000_0000;

    #[test]
    fn a_refused_write_is_a_write_from_ring_0_to_a_page_that_is_there() {
        assert_eq!(refused_access("kwrite_text", 0x3, AT, 0x1234, AT), Ok(()));
        for error in [0x2, 0x1, 0x7, 0x0] {
            assert!(refused_access("kwrite_text", error, AT, 0x1234, AT).is_err(), "error {error:#x}");
        }
        assert!(refused_access("kwrite_text", 0x3, AT + 8, 0x1234, AT).unwrap_err().contains("announced"));
    }

    #[test]
    fn a_refused_call_is_a_fetch_from_ring_0_of_a_page_that_is_there() {
        assert_eq!(refused_access("kexec_data", 0x11, AT, AT, AT), Ok(()));
        for error in [0x10, 0x01, 0x15, 0x03] {
            assert!(refused_access("kexec_data", error, AT, AT, AT).is_err(), "error {error:#x}");
        }
        assert!(refused_access("kexec_data", 0x11, AT, AT + 2, AT).unwrap_err().contains("rip"));
        assert!(refused_access("kexec_data", 0x11, AT + 8, AT, AT).is_err());
    }
}
