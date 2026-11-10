//! `BOOTFS.IMG` on the host: reading `bootfs/MANIFEST` and packing what it
//! names (docs/10 section 5.2).

use std::fs;
use std::path::{Path, PathBuf};

use sk_bootfs::build::Source;
use sk_bootfs::{CLASS_APP, CLASS_SYSTEM, FLAG_CRITICAL, FLAG_DISPLAY, KIND_FILE, KIND_PROGRAM};

use crate::inspect::{self, Corruption};
use crate::util::workspace_root;

/// Where the bytes of an entry come from.
#[derive(Debug, PartialEq, Eq)]
pub enum From {
    /// A ring-3 program: the package and binary of this name.
    Program(String),
    /// A file, by its path from the workspace root.
    File(PathBuf),
}

/// One line of the manifest.
#[derive(Debug, PartialEq, Eq)]
pub struct Row {
    pub path: String,
    pub from: From,
    pub name: String,
    pub display_name: String,
    pub class: u32,
    pub mem_limit_mib: u32,
    pub flags: u32,
    /// `corrupt:wx`: the entry is the program with a segment made writable
    /// and executable, which the kernel must refuse to start.
    pub corrupt_wx: bool,
}

fn parse_line(line: &str) -> Result<Row, String> {
    let fields: Vec<&str> = line.split('|').map(str::trim).collect();
    let [path, source, name, display_name, class, mem, flags] = fields[..] else {
        return Err(format!("expected 7 fields separated by `|`, found {}", fields.len()));
    };
    let from = match source.split_once(':') {
        Some(("program", binary)) if !binary.is_empty() => From::Program(binary.to_string()),
        Some(("file", file)) if !file.is_empty() => From::File(PathBuf::from(file)),
        _ => return Err(format!("source `{source}` is neither program:<binary> nor file:<path>")),
    };
    let class = match class {
        "app" => CLASS_APP,
        "system" => CLASS_SYSTEM,
        other => return Err(format!("class `{other}` is neither app nor system")),
    };
    let mem_limit_mib = mem.parse().map_err(|_| format!("mem_mib `{mem}` is not a number"))?;
    let mut bits = 0;
    let mut corrupt_wx = false;
    if flags != "-" {
        for flag in flags.split(',') {
            bits |= match flag.trim() {
                "critical" => FLAG_CRITICAL,
                "display" => FLAG_DISPLAY,
                "corrupt:wx" => {
                    corrupt_wx = true;
                    0
                }
                other => return Err(format!("flag `{other}` is none of critical, display, corrupt:wx")),
            };
        }
    }
    Ok(Row {
        path: path.to_string(),
        from,
        name: name.to_string(),
        display_name: display_name.to_string(),
        class,
        mem_limit_mib,
        flags: bits,
        corrupt_wx,
    })
}

pub fn parse_manifest(text: &str) -> Result<Vec<Row>, String> {
    let mut rows = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        rows.push(parse_line(line).map_err(|e| format!("bootfs/MANIFEST line {}: {e}", number + 1))?);
    }
    if rows.is_empty() {
        return Err("bootfs/MANIFEST names no file".to_string());
    }
    Ok(rows)
}

pub fn read_manifest() -> Result<Vec<Row>, String> {
    let path = workspace_root().join("bootfs").join("MANIFEST");
    parse_manifest(&fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?)
}

/// The programs the manifest names: binary names, each once.
pub fn programs(rows: &[Row]) -> Vec<&str> {
    let mut names: Vec<&str> = rows
        .iter()
        .filter_map(|row| match &row.from {
            From::Program(binary) => Some(binary.as_str()),
            From::File(_) => None,
        })
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The package that builds a program: the probes are the binaries of one
/// package, every other program is a package of its own name.
pub fn package(binary: &str) -> &str {
    if binary.starts_with("prober-") { "prober" } else { binary }
}

/// The packages to compile, each once.
pub fn packages(rows: &[Row]) -> Vec<&str> {
    let mut names: Vec<&str> = programs(rows).into_iter().map(package).collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Packs the manifest's files; programs are taken from `bin_dir`. A program
/// the kernel would refuse to load is refused here, at build time; a
/// `corrupt:wx` entry must be refused, and for that reason.
pub fn pack(rows: &[Row], bin_dir: &Path) -> Result<Vec<u8>, String> {
    let mut sources = Vec::new();
    for row in rows {
        let (file, kind) = match &row.from {
            From::Program(binary) => (bin_dir.join(binary), KIND_PROGRAM),
            From::File(path) => (workspace_root().join(path), KIND_FILE),
        };
        let mut data = fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        if row.corrupt_wx {
            if kind != KIND_PROGRAM {
                return Err(format!("{}: corrupt:wx is for programs", row.path));
            }
            data = inspect::corrupt(&data, Corruption::Wx)?;
            match sk_elf::validate(&data, &sk_elf::user_policy(stillkey_abi::DEFAULT_MEM_LIMIT)) {
                Err(reason) if reason.as_str() == "wx" => {}
                Err(reason) => return Err(format!("{}: refused with {}, expected wx", row.path, reason.as_str())),
                Ok(_) => return Err(format!("{}: the corrupted image is still loadable", row.path)),
            }
        } else if kind == KIND_PROGRAM {
            let limit = match row.mem_limit_mib {
                0 => stillkey_abi::DEFAULT_MEM_LIMIT,
                mib => u64::from(mib) << 20,
            };
            sk_elf::validate(&data, &sk_elf::user_policy(limit))
                .map_err(|reason| format!("{}: not a loadable program: {}", row.path, reason.as_str()))?;
        }
        sources.push(Source {
            path: row.path.clone(),
            name: row.name.clone(),
            display_name: row.display_name.clone(),
            class: row.class,
            mem_limit_mib: row.mem_limit_mib,
            flags: row.flags,
            kind,
            data,
        });
    }
    sk_bootfs::build::pack(&sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_line_gives_one_row() {
        let text = "# comment\n\nbin/hello | program:hello | hello | Hello there | app | 0 | -\n\
                    licenses/x.txt | file:third_party/x.txt | x | X | system | 4 | critical, display\n";
        let rows = parse_manifest(text).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "bin/hello");
        assert_eq!(rows[0].from, From::Program("hello".to_string()));
        assert_eq!(rows[0].display_name, "Hello there");
        assert_eq!((rows[0].class, rows[0].mem_limit_mib, rows[0].flags), (CLASS_APP, 0, 0));
        assert_eq!(rows[1].from, From::File(PathBuf::from("third_party/x.txt")));
        assert_eq!(rows[1].class, CLASS_SYSTEM);
        assert_eq!(rows[1].mem_limit_mib, 4);
        assert_eq!(rows[1].flags, FLAG_CRITICAL | FLAG_DISPLAY);
        assert_eq!(programs(&rows), ["hello"]);
        assert!(!rows[0].corrupt_wx && !rows[1].corrupt_wx);
    }

    #[test]
    fn the_probes_are_binaries_of_one_package() {
        let text = "bin/prober-hlt | program:prober-hlt | prober-hlt | P | app | 0 | -\n\
                    bin/prober-in | program:prober-in | prober-in | P | app | 0 | -\n\
                    bin/ticker | program:ticker | ticker | T | app | 0 | -\n\
                    bin/bad-elf | program:ticker | bad-elf | B | app | 0 | corrupt:wx\n";
        let rows = parse_manifest(text).unwrap();
        assert_eq!(programs(&rows), ["prober-hlt", "prober-in", "ticker"]);
        assert_eq!(packages(&rows), ["prober", "ticker"]);
        assert_eq!(rows.iter().map(|row| row.corrupt_wx).collect::<Vec<_>>(), [false, false, false, true]);
        assert_eq!(rows[3].flags, 0);
    }

    #[test]
    fn a_wrong_line_is_refused_with_its_number() {
        let bad = [
            "bin/a | program:a | a | A | app | 0",
            "bin/a | binary:a | a | A | app | 0 | -",
            "bin/a | program:a | a | A | user | 0 | -",
            "bin/a | program:a | a | A | app | lots | -",
            "bin/a | program:a | a | A | app | 0 | fast",
        ];
        for line in bad {
            let error = parse_manifest(&format!("# first\n{line}\n")).unwrap_err();
            assert!(error.contains("line 2"), "{error}");
        }
        assert!(parse_manifest("# nothing\n").is_err());
    }
}
