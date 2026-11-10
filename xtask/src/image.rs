//! `image`: a throw-away GPT disk image with one FAT32 EFI System Partition
//! (docs/07 section 5). No root privileges; a file on the Linux filesystem.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::build::Artifacts;

const IMAGE_SIZE: u64 = 68 << 20;

fn tool(dir: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program)
        .current_dir(dir)
        .env("MTOOLS_SKIP_CHECK", "1")
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} {args:?} failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

/// Boot options are ASCII 0x20-0x7E, at most 1024 bytes (docs/02 section 3).
pub fn check_cmdline(text: &str) -> Result<(), String> {
    if text.len() > 1024 {
        return Err("boot options are longer than 1024 bytes".to_string());
    }
    if text.bytes().any(|b| !(0x20..=0x7E).contains(&b)) {
        return Err("boot options contain a byte outside 0x20-0x7E".to_string());
    }
    Ok(())
}

/// Makes `dir/disk.img` from the two binaries, replacing any earlier image.
pub fn make(
    dir: &Path,
    art: &Artifacts,
    cmdline: Option<&str>,
    with_bootfs: bool,
) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let img = dir.join("disk.img");
    let _ = fs::remove_file(&img);
    let file = fs::File::create(&img).map_err(|e| format!("{}: {e}", img.display()))?;
    file.set_len(IMAGE_SIZE).map_err(|e| e.to_string())?;
    drop(file);

    let esp = "disk.img@@1M"; // the partition starts at sector 2048
    tool(
        dir,
        "sgdisk",
        &["--clear", "--new=1:2048:+64M", "--typecode=1:ef00", "--change-name=1:STILLKEY-ESP", "disk.img"],
    )?;
    tool(dir, "mformat", &["-i", esp, "-F", "-c", "1", "-T", "131072", "-v", "STILLKEY", "::"])?;
    tool(dir, "mmd", &["-i", esp, "::/EFI", "::/EFI/BOOT", "::/EFI/STILLKEY"])?;
    let loader = art.loader.to_str().ok_or("loader path is not UTF-8")?;
    let kernel = art.kernel.to_str().ok_or("kernel path is not UTF-8")?;
    tool(dir, "mcopy", &["-i", esp, loader, "::/EFI/BOOT/BOOTX64.EFI"])?;
    tool(dir, "mcopy", &["-i", esp, kernel, "::/EFI/STILLKEY/KERNEL.ELF"])?;
    if with_bootfs {
        let bootfs = art.bootfs.to_str().ok_or("BOOTFS.IMG path is not UTF-8")?;
        tool(dir, "mcopy", &["-i", esp, bootfs, "::/EFI/STILLKEY/BOOTFS.IMG"])?;
    }
    if let Some(text) = cmdline.filter(|t| !t.is_empty()) {
        check_cmdline(text)?;
        fs::write(dir.join("CMDLINE.TXT"), text).map_err(|e| e.to_string())?;
        tool(dir, "mcopy", &["-i", esp, "CMDLINE.TXT", "::/EFI/STILLKEY/CMDLINE.TXT"])?;
    }
    Ok(img)
}
