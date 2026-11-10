//! Small helpers shared by the xtask commands.

use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// The directory that holds the workspace `Cargo.toml`.
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent directory")
        .to_path_buf()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// `~/.cache/stillkey`, where every image, firmware-variable copy and log is made.
pub fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    PathBuf::from(home).join(".cache").join("stillkey")
}
