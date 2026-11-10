//! Reads the headers of the two binaries, with no external tool: the artifact
//! checks of `m1-unit` and the corrupted kernels of `m1-reject` (docs/07
//! section 9).

/// `IMAGE_DLLCHARACTERISTICS_NX_COMPAT`.
pub const NX_COMPAT: u16 = 0x0100;
/// `IMAGE_SCN_MEM_EXECUTE` and `IMAGE_SCN_MEM_WRITE`.
pub const SCN_EXECUTE: u32 = 0x2000_0000;
pub const SCN_WRITE: u32 = 0x8000_0000;

#[derive(Debug, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub characteristics: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Pe {
    pub dll_characteristics: u16,
    pub section_alignment: u32,
    pub sections: Vec<Section>,
}

/// One `PT_LOAD` program header.
#[derive(Debug, PartialEq, Eq)]
pub struct Load {
    /// Where the header itself begins in the file.
    pub header_at: usize,
    pub flags: u32,
    pub offset: u64,
    pub filesz: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Elf {
    pub e_type: u16,
    pub machine: u16,
    pub loads: Vec<Load>,
}

/// The five corrupted kernels of `m1-reject`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corruption {
    Ident,
    Type,
    Wx,
    Segment,
    Phdr,
}

impl Corruption {
    pub const ALL: [Corruption; 5] =
        [Corruption::Ident, Corruption::Type, Corruption::Wx, Corruption::Segment, Corruption::Phdr];

    /// The `reason=` the loader must give for this image.
    pub fn reason(self) -> &'static str {
        match self {
            Corruption::Ident => "ident",
            Corruption::Type => "type",
            Corruption::Wx => "wx",
            Corruption::Segment => "segment",
            Corruption::Phdr => "phdr",
        }
    }
}

/// `N` bytes at `at`, or an error that names what was being read.
fn bytes<const N: usize>(file: &[u8], at: usize, what: &str) -> Result<[u8; N], String> {
    at.checked_add(N)
        .and_then(|end| file.get(at..end))
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("the file ends before {what} (offset {at})"))
}

fn u16_at(file: &[u8], at: usize, what: &str) -> Result<u16, String> {
    bytes(file, at, what).map(u16::from_le_bytes)
}

fn u32_at(file: &[u8], at: usize, what: &str) -> Result<u32, String> {
    bytes(file, at, what).map(u32::from_le_bytes)
}

fn u64_at(file: &[u8], at: usize, what: &str) -> Result<u64, String> {
    bytes(file, at, what).map(u64::from_le_bytes)
}

/// Reads the fields of a PE32+ file that the loader check needs.
pub fn parse_pe(file: &[u8]) -> Result<Pe, String> {
    if bytes::<2>(file, 0, "the DOS header")? != *b"MZ" {
        return Err("not a PE file: no MZ signature".to_string());
    }
    let pe = u32_at(file, 0x3C, "e_lfanew")? as usize;
    if bytes::<4>(file, pe, "the PE signature")? != *b"PE\0\0" {
        return Err("not a PE file: no PE signature".to_string());
    }
    // The COFF header is 20 bytes; the optional header follows it.
    let coff = pe + 4;
    let section_count = u16_at(file, coff + 2, "the section count")?;
    let optional_size = u16_at(file, coff + 16, "the optional-header size")? as usize;
    let optional = coff + 20;
    if u16_at(file, optional, "the optional-header magic")? != 0x020B {
        return Err("not a PE32+ file".to_string());
    }
    let section_alignment = u32_at(file, optional + 32, "SectionAlignment")?;
    let dll_characteristics = u16_at(file, optional + 70, "DllCharacteristics")?;

    let table = optional
        .checked_add(optional_size)
        .ok_or("the optional-header size overflows")?;
    let mut sections = Vec::new();
    for index in 0..usize::from(section_count) {
        let at = table + index * 40;
        let name: [u8; 8] = bytes(file, at, "a section header")?;
        let characteristics = u32_at(file, at + 36, "a section's characteristics")?;
        let name = String::from_utf8_lossy(&name).trim_end_matches('\0').to_string();
        sections.push(Section { name, characteristics });
    }
    Ok(Pe { dll_characteristics, section_alignment, sections })
}

/// The loader's header properties (docs/07 section 9, `m1-unit` part 2):
/// NX-compatible, sections aligned to at least a page, and no section both
/// writable and executable. Returns the fields of the result line.
pub fn check_loader(file: &[u8]) -> Result<String, String> {
    let pe = parse_pe(file)?;
    if pe.dll_characteristics & NX_COMPAT == 0 {
        return Err(format!(
            "DllCharacteristics {:#06x} lacks IMAGE_DLLCHARACTERISTICS_NX_COMPAT (0x0100)",
            pe.dll_characteristics
        ));
    }
    if pe.section_alignment < 4096 {
        return Err(format!("section alignment {} is below 4096", pe.section_alignment));
    }
    let wx = SCN_WRITE | SCN_EXECUTE;
    if let Some(section) = pe.sections.iter().find(|s| s.characteristics & wx == wx) {
        return Err(format!(
            "section {} is writable and executable (characteristics {:#010x})",
            section.name, section.characteristics
        ));
    }
    Ok(format!(
        "nx_compat=yes section_alignment={} sections={} wx_sections=0",
        pe.section_alignment,
        pe.sections.len()
    ))
}

const PT_LOAD: u32 = 1;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 62;

/// Reads the fields of a little-endian ELF64 file that the kernel check and
/// the corruptions need.
pub fn parse_elf(file: &[u8]) -> Result<Elf, String> {
    if bytes::<6>(file, 0, "the ELF identification")? != [0x7F, b'E', b'L', b'F', 2, 1] {
        return Err("not a little-endian ELF64 file".to_string());
    }
    let e_type = u16_at(file, 16, "e_type")?;
    let machine = u16_at(file, 18, "e_machine")?;
    let table = u64_at(file, 32, "e_phoff")?;
    let entry_size = usize::from(u16_at(file, 54, "e_phentsize")?);
    let count = usize::from(u16_at(file, 56, "e_phnum")?);
    let table = usize::try_from(table).map_err(|_| "e_phoff does not fit".to_string())?;
    if entry_size < 56 {
        return Err(format!("e_phentsize {entry_size} is below 56"));
    }
    let mut loads = Vec::new();
    for index in 0..count {
        let at = index
            .checked_mul(entry_size)
            .and_then(|offset| table.checked_add(offset))
            .ok_or("the program-header table overflows")?;
        let kind = u32_at(file, at, "a program header")?;
        let flags = u32_at(file, at + 4, "p_flags")?;
        let offset = u64_at(file, at + 8, "p_offset")?;
        let filesz = u64_at(file, at + 32, "p_filesz")?;
        // The whole header must be in the file, not only the fields read.
        bytes::<8>(file, at + 48, "a program header")?;
        if kind == PT_LOAD {
            loads.push(Load { header_at: at, flags, offset, filesz });
        }
    }
    Ok(Elf { e_type, machine, loads })
}

/// The kernel's header properties (docs/07 section 9, `m1-unit` part 2):
/// `ET_EXEC` for x86-64, and every `PT_LOAD` readable, readable and
/// executable, or readable and writable. Returns the fields of the result line.
pub fn check_kernel(file: &[u8]) -> Result<String, String> {
    let elf = parse_elf(file)?;
    if elf.e_type != ET_EXEC {
        return Err(format!("e_type is {}, not ET_EXEC (2)", elf.e_type));
    }
    if elf.machine != EM_X86_64 {
        return Err(format!("e_machine is {}, not x86-64 (62)", elf.machine));
    }
    if elf.loads.is_empty() {
        return Err("no PT_LOAD segment".to_string());
    }
    if let Some(load) = elf.loads.iter().find(|l| !matches!(l.flags, 4 | 5 | 6)) {
        return Err(format!("a PT_LOAD segment has flags {}, not 4, 5 or 6", load.flags));
    }
    let flags: Vec<String> = elf.loads.iter().map(|l| l.flags.to_string()).collect();
    Ok(format!("type=exec machine=x86-64 loads={} flags={}", elf.loads.len(), flags.join(",")))
}

/// One corruption of a valid kernel (docs/07 section 9, `m1-reject`). Each
/// breaks exactly one rule of the loader's validator, or, for the 100-byte
/// file, makes the program-header rule the first that fails.
pub fn corrupt(kernel: &[u8], how: Corruption) -> Result<Vec<u8>, String> {
    let mut out = kernel.to_vec();
    match how {
        Corruption::Ident => {
            let first = out.first_mut().ok_or("the kernel file is empty")?;
            *first ^= 0xFF;
        }
        Corruption::Type => {
            parse_elf(kernel)?;
            out[16..18].copy_from_slice(&3u16.to_le_bytes());
        }
        Corruption::Wx => {
            let elf = parse_elf(kernel)?;
            let last = elf.loads.last().ok_or("no PT_LOAD segment to change")?;
            out[last.header_at + 4..last.header_at + 8].copy_from_slice(&7u32.to_le_bytes());
        }
        Corruption::Segment => {
            let elf = parse_elf(kernel)?;
            let last = elf
                .loads
                .iter()
                .rev()
                .find(|l| l.filesz > 0)
                .ok_or("no PT_LOAD segment has bytes in the file")?;
            let end = last
                .offset
                .checked_add(last.filesz)
                .and_then(|end| usize::try_from(end).ok())
                .filter(|&end| end <= kernel.len())
                .ok_or("the last PT_LOAD segment already ends past the file")?;
            out.truncate(end - 1);
        }
        Corruption::Phdr => {
            if kernel.len() <= 100 {
                return Err("the kernel file is too short to cut to 100 bytes".to_string());
            }
            out.truncate(100);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- A PE32+ file with the fields the checks read --------------------------

    const TEXT: u32 = 0x6000_0020; // code, execute, read
    const RDATA: u32 = 0x4000_0040; // initialised data, read
    const DATA: u32 = 0xC000_0040; // initialised data, read, write
    const GOOD_DLL: u16 = 0x0160; // high-entropy VA, dynamic base, NX compatible

    fn pe(dll: u16, alignment: u32, sections: &[(&str, u32)]) -> Vec<u8> {
        let mut f = vec![0u8; 0x40];
        f[..2].copy_from_slice(b"MZ");
        f[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        f.extend_from_slice(b"PE\0\0");
        // COFF header: machine, section count, 12 bytes we do not read,
        // size of the optional header, characteristics.
        f.extend_from_slice(&0x8664u16.to_le_bytes());
        f.extend_from_slice(&(sections.len() as u16).to_le_bytes());
        f.extend_from_slice(&[0; 12]);
        f.extend_from_slice(&240u16.to_le_bytes());
        f.extend_from_slice(&0x0022u16.to_le_bytes());
        let mut optional = [0u8; 240];
        optional[..2].copy_from_slice(&0x020Bu16.to_le_bytes());
        optional[32..36].copy_from_slice(&alignment.to_le_bytes());
        optional[68..70].copy_from_slice(&10u16.to_le_bytes());
        optional[70..72].copy_from_slice(&dll.to_le_bytes());
        f.extend_from_slice(&optional);
        for (name, characteristics) in sections {
            let mut header = [0u8; 40];
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[36..].copy_from_slice(&characteristics.to_le_bytes());
            f.extend_from_slice(&header);
        }
        f
    }

    fn good_pe() -> Vec<u8> {
        pe(GOOD_DLL, 4096, &[(".text", TEXT), (".rdata", RDATA), (".data", DATA)])
    }

    #[test]
    fn reads_the_fields_of_a_pe_file() {
        let parsed = parse_pe(&good_pe()).expect("a PE32+ file");
        assert_eq!(parsed.dll_characteristics, GOOD_DLL);
        assert_eq!(parsed.section_alignment, 4096);
        let names: Vec<&str> = parsed.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, [".text", ".rdata", ".data"]);
        assert_eq!(parsed.sections[2].characteristics, DATA);
    }

    #[test]
    fn a_loader_with_nx_4k_alignment_and_no_wx_section_passes() {
        let line = check_loader(&good_pe()).expect("the loader passes");
        assert!(line.contains("nx_compat=yes"), "{line}");
        assert!(line.contains("section_alignment=4096"), "{line}");
        assert!(line.contains("wx_sections=0"), "{line}");
    }

    #[test]
    fn a_loader_without_the_nx_flag_fails() {
        let file = pe(GOOD_DLL & !NX_COMPAT, 4096, &[(".text", TEXT)]);
        let why = check_loader(&file).expect_err("no NX flag");
        assert!(why.contains("NX_COMPAT"), "{why}");
    }

    #[test]
    fn a_loader_with_sections_aligned_below_a_page_fails() {
        let why = check_loader(&pe(GOOD_DLL, 512, &[(".text", TEXT)])).expect_err("512");
        assert!(why.contains("section alignment"), "{why}");
        assert!(check_loader(&pe(GOOD_DLL, 0x20_0000, &[(".text", TEXT)])).is_ok());
    }

    #[test]
    fn a_loader_with_a_writable_and_executable_section_fails() {
        let file = pe(GOOD_DLL, 4096, &[(".text", TEXT), (".bad", TEXT | SCN_WRITE)]);
        let why = check_loader(&file).expect_err("W+X");
        assert!(why.contains(".bad"), "{why}");
    }

    #[test]
    fn a_file_that_is_not_a_whole_pe_file_is_an_error_not_a_panic() {
        let good = good_pe();
        assert!(parse_pe(&[]).is_err());
        assert!(parse_pe(b"MZ").is_err());
        let mut not_mz = good.clone();
        not_mz[0] = b'X';
        assert!(parse_pe(&not_mz).is_err());
        let mut far = good.clone();
        far[0x3C..0x40].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        assert!(parse_pe(&far).is_err());
        let mut pe32 = good.clone();
        pe32[0x58..0x5A].copy_from_slice(&0x010Bu16.to_le_bytes());
        assert!(parse_pe(&pe32).is_err(), "only PE32+ is expected");
        // Every shorter file: the section table or a header is cut.
        for len in 0..good.len() {
            assert!(parse_pe(&good[..len]).is_err(), "length {len}");
        }
    }

    // ---- An ELF64 file with the fields the checks read ---------------------------

    const PT_LOAD: u32 = 1;
    const PT_NOTE: u32 = 4;

    /// `(type, flags, offset, filesz)` per program header; the file is padded
    /// to the end of the furthest segment.
    fn elf(e_type: u16, machine: u16, headers: &[(u32, u32, u64, u64)]) -> Vec<u8> {
        let mut f = vec![0u8; 64];
        f[..7].copy_from_slice(&[0x7F, b'E', b'L', b'F', 2, 1, 1]);
        f[16..18].copy_from_slice(&e_type.to_le_bytes());
        f[18..20].copy_from_slice(&machine.to_le_bytes());
        f[20..24].copy_from_slice(&1u32.to_le_bytes());
        f[32..40].copy_from_slice(&64u64.to_le_bytes());
        f[52..54].copy_from_slice(&64u16.to_le_bytes());
        f[54..56].copy_from_slice(&56u16.to_le_bytes());
        f[56..58].copy_from_slice(&(headers.len() as u16).to_le_bytes());
        let mut end = 0;
        for (kind, flags, offset, filesz) in headers {
            let mut h = [0u8; 56];
            h[..4].copy_from_slice(&kind.to_le_bytes());
            h[4..8].copy_from_slice(&flags.to_le_bytes());
            h[8..16].copy_from_slice(&offset.to_le_bytes());
            h[32..40].copy_from_slice(&filesz.to_le_bytes());
            f.extend_from_slice(&h);
            end = end.max(offset + filesz);
        }
        // Not zero, so that a test can tell padding from a cut.
        f.resize(f.len().max(end as usize), 0x5A);
        f
    }

    /// Text, read-only data, data, then a segment with nothing in the file.
    fn good_elf() -> Vec<u8> {
        elf(
            2,
            62,
            &[
                (PT_LOAD, 5, 0x1000, 0x800),
                (PT_NOTE, 4, 0x200, 0x20),
                (PT_LOAD, 4, 0x2000, 0x300),
                (PT_LOAD, 6, 0x3000, 0x123),
                (PT_LOAD, 6, 0x4000, 0),
            ],
        )
    }

    #[test]
    fn reads_the_load_headers_of_an_elf_file() {
        let parsed = parse_elf(&good_elf()).expect("an ELF64 file");
        assert_eq!((parsed.e_type, parsed.machine), (2, 62));
        let flags: Vec<u32> = parsed.loads.iter().map(|l| l.flags).collect();
        assert_eq!(flags, [5, 4, 6, 6], "the note is not a load");
        assert_eq!(parsed.loads[1], Load { header_at: 64 + 2 * 56, flags: 4, offset: 0x2000, filesz: 0x300 });
    }

    #[test]
    fn an_exec_kernel_for_x86_64_with_plain_segments_passes() {
        let line = check_kernel(&good_elf()).expect("the kernel passes");
        assert!(line.contains("type=exec"), "{line}");
        assert!(line.contains("loads=4"), "{line}");
    }

    #[test]
    fn a_kernel_that_is_not_et_exec_for_x86_64_fails() {
        let loads = [(PT_LOAD, 5, 0x1000, 0x10)];
        assert!(check_kernel(&elf(3, 62, &loads)).expect_err("ET_DYN").contains("ET_EXEC"));
        assert!(check_kernel(&elf(2, 183, &loads)).expect_err("AArch64").contains("x86-64"));
    }

    #[test]
    fn a_kernel_segment_with_other_flags_fails() {
        for flags in [0, 1, 2, 3, 7] {
            let file = elf(2, 62, &[(PT_LOAD, 5, 0x1000, 0x10), (PT_LOAD, flags, 0x2000, 0x10)]);
            let why = check_kernel(&file).expect_err("flags");
            assert!(why.contains(&format!("flags {flags}")), "{why}");
        }
        assert!(check_kernel(&elf(2, 62, &[(PT_NOTE, 4, 0x200, 0x20)])).is_err(), "no PT_LOAD");
    }

    #[test]
    fn a_file_that_is_not_a_whole_elf_header_is_an_error_not_a_panic() {
        let good = good_elf();
        assert!(parse_elf(&[]).is_err());
        let mut not_elf = good.clone();
        not_elf[1] = b'X';
        assert!(parse_elf(&not_elf).is_err());
        let mut far = good.clone();
        far[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(parse_elf(&far).is_err());
        for len in 0..64 + 5 * 56 {
            assert!(parse_elf(&good[..len]).is_err(), "length {len}");
        }
    }

    // ---- The five corrupted kernels ----------------------------------------------

    /// The positions at which two files of the same length differ.
    fn differing(a: &[u8], b: &[u8]) -> Vec<usize> {
        assert_eq!(a.len(), b.len());
        (0..a.len()).filter(|&i| a[i] != b[i]).collect()
    }

    #[test]
    fn the_ident_corruption_changes_the_first_byte_only() {
        let good = good_elf();
        let bad = corrupt(&good, Corruption::Ident).unwrap();
        assert_eq!(differing(&good, &bad), [0]);
    }

    #[test]
    fn the_type_corruption_sets_e_type_to_3() {
        let good = good_elf();
        let bad = corrupt(&good, Corruption::Type).unwrap();
        assert_eq!(differing(&good, &bad), [16]);
        assert_eq!(parse_elf(&bad).unwrap().e_type, 3);
    }

    #[test]
    fn the_wx_corruption_sets_the_flags_of_the_last_load_to_7() {
        let good = good_elf();
        let bad = corrupt(&good, Corruption::Wx).unwrap();
        // The fifth program header is the last PT_LOAD; its flags were 6.
        assert_eq!(differing(&good, &bad), [64 + 4 * 56 + 4]);
        let flags: Vec<u32> = parse_elf(&bad).unwrap().loads.iter().map(|l| l.flags).collect();
        assert_eq!(flags, [5, 4, 6, 7]);
    }

    #[test]
    fn the_segment_corruption_cuts_one_byte_off_the_last_load_that_has_file_bytes() {
        let good = good_elf();
        let bad = corrupt(&good, Corruption::Segment).unwrap();
        // The last PT_LOAD has no bytes in the file; the one before it ends at 0x3123.
        assert_eq!(bad.len(), 0x3000 + 0x123 - 1);
        assert_eq!(bad[..], good[..bad.len()]);
    }

    #[test]
    fn the_phdr_corruption_cuts_the_file_inside_the_program_header_table() {
        let good = good_elf();
        let bad = corrupt(&good, Corruption::Phdr).unwrap();
        assert_eq!(bad.len(), 100);
        assert_eq!(bad[..], good[..100]);
    }

    #[test]
    fn a_kernel_that_cannot_be_corrupted_as_asked_is_an_error() {
        // No load has bytes in the file: nothing to cut.
        let empty = elf(2, 62, &[(PT_LOAD, 6, 0x1000, 0)]);
        assert!(corrupt(&empty, Corruption::Segment).is_err());
        // No PT_LOAD at all.
        let none = elf(2, 62, &[(PT_NOTE, 4, 0x200, 0x20)]);
        assert!(corrupt(&none, Corruption::Wx).is_err());
        // Too short to be cut to 100 bytes.
        assert!(corrupt(&good_elf()[..90], Corruption::Phdr).is_err());
        assert!(corrupt(&[], Corruption::Ident).is_err());
    }

    #[test]
    fn every_corruption_has_its_own_reason() {
        let reasons: Vec<&str> = Corruption::ALL.iter().map(|c| c.reason()).collect();
        assert_eq!(reasons, ["ident", "type", "wx", "segment", "phdr"]);
    }
}
