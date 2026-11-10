//! `build`: compile the loader and the kernel and collect the two binaries.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{sha256_hex, workspace_root};

pub struct Artifacts {
    pub loader: PathBuf,
    pub kernel: PathBuf,
    pub profile: &'static str,
    pub kernel_sha256: String,
}

/// The two binaries where Cargo left them in its build directory.
pub struct Built {
    pub loader: PathBuf,
    pub kernel: PathBuf,
}

fn default_target_dir() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => workspace_root().join("target"),
    }
}

/// A Cargo child with the flag variables removed: either one would replace
/// every `rustflags` entry of `.cargo/config.toml`, `relocation-model=static`
/// included.
pub fn cargo() -> Command {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(workspace_root())
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
    cmd
}

/// Cargo's home directory: where the sources of the dependencies are.
pub fn cargo_home() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("CARGO_HOME") {
        return Ok(PathBuf::from(dir));
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".cargo"))
        .ok_or_else(|| "neither CARGO_HOME nor HOME is set".to_string())
}

/// The `--config` value that keeps the directories of this machine out of the
/// binaries (docs/07 section 4): the workspace becomes `.` and Cargo's home
/// `/cargo`. It must be this key: Cargo appends the array to the `rustflags`
/// of `.cargo/config.toml` for the same target, where a flag variable would
/// replace them.
fn remap_config(target: &str) -> Result<String, String> {
    let quoted = |path: PathBuf, to: &str| -> Result<String, String> {
        let text = path.to_str().ok_or("a build path is not UTF-8")?;
        // The value is a TOML string; these paths need no escaping, and a
        // path that would is refused rather than escaped wrongly.
        if text.contains(['"', '\\']) || text.chars().any(char::is_control) {
            return Err(format!("the path {text:?} cannot be written into a Cargo setting"));
        }
        Ok(format!("\"--remap-path-prefix={text}={to}\""))
    };
    Ok(format!(
        "target.{target}.rustflags=[{}, {}]",
        quoted(workspace_root(), ".")?,
        quoted(cargo_home()?, "/cargo")?
    ))
}

fn cargo_build(
    package: &str,
    target: &str,
    release: bool,
    target_dir: Option<&Path>,
) -> Result<(), String> {
    let mut cmd = cargo();
    cmd.args(["build", "--locked", "-p", package, "--target", target]);
    cmd.arg("--config").arg(remap_config(target)?);
    if release {
        cmd.arg("--release");
    }
    if let Some(dir) = target_dir {
        // Takes precedence over CARGO_TARGET_DIR.
        cmd.arg("--target-dir").arg(dir);
    }
    let status = cmd.status().map_err(|e| format!("cargo build: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("cargo build -p {package} failed")) }
}

/// Compiles both binaries, into `target_dir` if one is given and otherwise
/// into Cargo's usual build directory.
pub fn compile(release: bool, target_dir: Option<&Path>) -> Result<Built, String> {
    cargo_build("stillkey-loader", "x86_64-unknown-uefi", release, target_dir)?;
    cargo_build("stillkey-kernel", "x86_64-unknown-none", release, target_dir)?;
    let cargo_profile = if release { "release" } else { "debug" };
    let target = target_dir.map(Path::to_path_buf).unwrap_or_else(default_target_dir);
    Ok(Built {
        loader: target.join("x86_64-unknown-uefi").join(cargo_profile).join("stillkey-loader.efi"),
        kernel: target.join("x86_64-unknown-none").join(cargo_profile).join("stillkey-kernel"),
    })
}

pub fn build(release: bool) -> Result<Artifacts, String> {
    let built = compile(release, None)?;
    let profile = if release { "release" } else { "dev" };

    let out = workspace_root().join("build").join("artifacts").join(profile);
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let loader = out.join("BOOTX64.EFI");
    let kernel = out.join("KERNEL.ELF");
    fs::copy(&built.loader, &loader).map_err(|e| format!("{}: {e}", built.loader.display()))?;
    fs::copy(&built.kernel, &kernel).map_err(|e| format!("{}: {e}", built.kernel.display()))?;

    let loader_sha = sha256_hex(&fs::read(&loader).map_err(|e| e.to_string())?);
    let kernel_sha256 = sha256_hex(&fs::read(&kernel).map_err(|e| e.to_string())?);
    println!("built profile={profile}");
    println!("  BOOTX64.EFI sha256={loader_sha}");
    println!("  KERNEL.ELF  sha256={kernel_sha256}");
    Ok(Artifacts { loader, kernel, profile, kernel_sha256 })
}
