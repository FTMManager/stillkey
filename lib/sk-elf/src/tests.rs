//! Host tests for rules V1-V10 (docs/02 section 4): an accepting and a
//! rejecting case for every rule.

use super::*;

const BASE: u64 = 0xFFFF_FFFF_8000_0000;

#[derive(Clone)]
struct Ph {
    p_type: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
}

#[derive(Clone)]
struct Elf {
    ident: [u8; 16],
    e_type: u16,
    machine: u16,
    version: u32,
    entry: u64,
    phoff: u64,
    ehsize: u16,
    phentsize: u16,
    phnum: Option<u16>,
    phs: Vec<Ph>,
    len: usize,
}

const PT_LOAD: u32 = 1;

fn load(flags: u32, offset: u64, vaddr: u64, filesz: u64, memsz: u64) -> Ph {
    Ph { p_type: PT_LOAD, flags, offset, vaddr, filesz, memsz }
}

/// A kernel-shaped image: text, read-only data, data with bss.
fn good() -> Elf {
    let mut ident = [0u8; 16];
    ident[..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    ident[4] = 2; // 64-bit
    ident[5] = 1; // little-endian
    ident[6] = 1; // version
    Elf {
        ident,
        e_type: 2,
        machine: 62,
        version: 1,
        entry: BASE + 0x10,
        phoff: 64,
        ehsize: 64,
        phentsize: 56,
        phnum: None,
        phs: vec![
            load(5, 0x1000, BASE, 0x200, 0x200),
            load(4, 0x2000, BASE + 0x1000, 0x100, 0x100),
            load(6, 0x3000, BASE + 0x2000, 0x80, 0x1080),
        ],
        len: 0x3080,
    }
}

impl Elf {
    fn bytes(&self) -> Vec<u8> {
        let mut b = vec![0u8; self.len.max(64)];
        b[..16].copy_from_slice(&self.ident);
        b[16..18].copy_from_slice(&self.e_type.to_le_bytes());
        b[18..20].copy_from_slice(&self.machine.to_le_bytes());
        b[20..24].copy_from_slice(&self.version.to_le_bytes());
        b[24..32].copy_from_slice(&self.entry.to_le_bytes());
        b[32..40].copy_from_slice(&self.phoff.to_le_bytes());
        b[52..54].copy_from_slice(&self.ehsize.to_le_bytes());
        b[54..56].copy_from_slice(&self.phentsize.to_le_bytes());
        let phnum = self.phnum.unwrap_or(self.phs.len() as u16);
        b[56..58].copy_from_slice(&phnum.to_le_bytes());
        for (i, ph) in self.phs.iter().enumerate() {
            let Some(at) = (self.phoff as usize).checked_add(i * 56) else { break };
            if at.checked_add(56).is_none_or(|end| end > b.len()) {
                break;
            }
            b[at..at + 4].copy_from_slice(&ph.p_type.to_le_bytes());
            b[at + 4..at + 8].copy_from_slice(&ph.flags.to_le_bytes());
            b[at + 8..at + 16].copy_from_slice(&ph.offset.to_le_bytes());
            b[at + 16..at + 24].copy_from_slice(&ph.vaddr.to_le_bytes());
            b[at + 32..at + 40].copy_from_slice(&ph.filesz.to_le_bytes());
            b[at + 40..at + 48].copy_from_slice(&ph.memsz.to_le_bytes());
        }
        b.truncate(self.len);
        b
    }

    fn check(&self) -> Result<Image, Reject> {
        validate(&self.bytes(), &KERNEL_POLICY)
    }
}

fn rejects(change: impl FnOnce(&mut Elf)) -> Reject {
    let mut e = good();
    change(&mut e);
    e.check().expect_err("the changed image must be rejected")
}

#[test]
fn accepts_a_well_formed_kernel_image() {
    let img = good().check().expect("valid image");
    assert_eq!(img.entry, BASE + 0x10);
    assert_eq!(img.base, BASE);
    assert_eq!(img.span, 0x4000); // three segments, the last ends at +0x3080
    let segs = img.segments();
    assert_eq!(segs.len(), 3);
    assert_eq!(
        segs[2],
        Segment { vaddr: BASE + 0x2000, offset: 0x3000, filesz: 0x80, memsz: 0x1080, flags: 6 }
    );
}

// V1
#[test]
fn v1_rejects_a_file_shorter_than_an_elf_header() {
    assert_eq!(validate(&[0u8; 63], &KERNEL_POLICY).unwrap_err(), Reject::Size);
}

#[test]
fn v1_rejects_a_file_larger_than_the_policy_allows() {
    let small = Policy { max_file: 0x3000, ..KERNEL_POLICY };
    assert_eq!(validate(&good().bytes(), &small).unwrap_err(), Reject::Size);
    assert_eq!(check_file_size(0x3001, &small), Err(Reject::Size));
    assert_eq!(check_file_size(0x3000, &small), Ok(()));
    assert_eq!(check_file_size(63, &small), Err(Reject::Size));
}

// V2
#[test]
fn v2_rejects_bad_identification_bytes() {
    assert_eq!(rejects(|e| e.ident[0] = 0x7E), Reject::Ident);
    assert_eq!(rejects(|e| e.ident[4] = 1), Reject::Ident); // 32-bit
    assert_eq!(rejects(|e| e.ident[5] = 2), Reject::Ident); // big-endian
    assert_eq!(rejects(|e| e.ident[6] = 0), Reject::Ident); // version
}

// V3
#[test]
fn v3_rejects_anything_but_a_fixed_address_x86_64_executable() {
    assert_eq!(rejects(|e| e.e_type = 3), Reject::Type); // ET_DYN: position-independent
    assert_eq!(rejects(|e| e.machine = 183), Reject::Type);
    assert_eq!(rejects(|e| e.version = 2), Reject::Type);
}

// V4
#[test]
fn v4_rejects_a_bad_program_header_table() {
    assert_eq!(rejects(|e| e.ehsize = 52), Reject::Phdr);
    assert_eq!(rejects(|e| e.phentsize = 32), Reject::Phdr);
    assert_eq!(rejects(|e| e.phnum = Some(0)), Reject::Phdr);
    assert_eq!(rejects(|e| e.phnum = Some(17)), Reject::Phdr);
    assert_eq!(rejects(|e| e.phoff = 0x3080 - 56), Reject::Phdr); // table runs off the end
    assert_eq!(rejects(|e| e.phoff = u64::MAX - 8), Reject::Phdr); // sum overflows
    assert_eq!(rejects(|e| e.len = 100), Reject::Phdr); // truncated inside the table
}

// V5
#[test]
fn v5_rejects_malformed_load_segments() {
    assert_eq!(rejects(|e| e.phs[0].filesz = 0x201), Reject::Segment); // filesz > memsz
    assert_eq!(rejects(|e| { e.phs[1].filesz = 0; e.phs[1].memsz = 0 }), Reject::Segment);
    assert_eq!(rejects(|e| e.len = 0x3000 + 0x80 - 1), Reject::Segment); // last segment cut short
    assert_eq!(rejects(|e| e.phs[2].offset = u64::MAX - 0x7F), Reject::Segment); // overflow
    assert_eq!(rejects(|e| e.phs[1].vaddr = BASE + 0x1008), Reject::Segment); // not page-aligned
    assert_eq!(rejects(|e| e.phs[1].offset = 0x2008), Reject::Segment); // offset not congruent
}

// V6
#[test]
fn v6_rejects_segments_outside_the_window_or_out_of_order() {
    assert_eq!(rejects(|e| e.phs[0].vaddr = BASE - 0x1000), Reject::Range); // below the window
    assert_eq!(rejects(|e| e.phs[0].vaddr = 0x40_0000), Reject::Range); // a user address
    assert_eq!(rejects(|e| e.phs[2].memsz = u64::MAX), Reject::Range); // end overflows
    assert_eq!(rejects(|e| e.phs.swap(1, 2)), Reject::Range); // descending
    assert_eq!(rejects(|e| e.phs[1].vaddr = BASE), Reject::Range); // shares a page with text
    assert_eq!(rejects(|e| e.phs[0].memsz = 0x1001), Reject::Range); // text runs into the next page
}

#[test]
fn v6_accepts_a_user_image_only_inside_the_user_window() {
    let user = Policy {
        window_first: 0x40_0000,
        window_last: 0x8000_0000 - 1,
        max_file: 16 << 20,
        max_mem: 16 << 20,
    };
    let mut e = good();
    for (i, ph) in e.phs.iter_mut().enumerate() {
        ph.vaddr = 0x40_0000 + (i as u64) * 0x1000;
    }
    e.entry = 0x40_0010;
    assert!(validate(&e.bytes(), &user).is_ok());
    e.phs[2].vaddr = 0x8000_0000 - 0x1000; // bss would cross the end of the window
    assert_eq!(validate(&e.bytes(), &user).unwrap_err(), Reject::Range);
}

// V7
#[test]
fn v7_rejects_any_flags_but_r_rx_rw() {
    assert_eq!(rejects(|e| e.phs[2].flags = 7), Reject::Wx); // writable and executable
    assert_eq!(rejects(|e| e.phs[0].flags = 1), Reject::Wx); // execute-only
    assert_eq!(rejects(|e| e.phs[1].flags = 2), Reject::Wx); // write-only
    assert_eq!(rejects(|e| e.phs[1].flags = 0x8000_0004), Reject::Wx); // unknown bit
}

// V8
#[test]
fn v8_rejects_an_entry_point_outside_executable_file_bytes() {
    assert_eq!(rejects(|e| e.entry = BASE + 0x1010), Reject::Entry); // in read-only data
    assert_eq!(rejects(|e| e.entry = BASE + 0x200), Reject::Entry); // one past the text bytes
    assert_eq!(rejects(|e| e.entry = 0), Reject::Entry);
    assert_eq!(rejects(|e| e.entry = BASE - 1), Reject::Entry);
}

#[test]
fn v8_accepts_the_last_text_byte_as_entry() {
    let mut e = good();
    e.entry = BASE + 0x1FF;
    assert!(e.check().is_ok());
}

// V9
#[test]
fn v9_rejects_dynamic_interp_and_tls_headers() {
    for bad in [2u32, 3, 7] {
        let got = rejects(|e| {
            e.phs.push(Ph { p_type: bad, flags: 4, offset: 0, vaddr: 0, filesz: 0, memsz: 0 })
        });
        assert_eq!(got, Reject::Dynamic, "p_type {bad}");
    }
}

#[test]
fn v9_ignores_other_non_load_headers() {
    let mut e = good();
    // PT_NOTE and PT_GNU_STACK carry no rule; their fields are not examined.
    e.phs.push(Ph { p_type: 4, flags: 7, offset: u64::MAX, vaddr: 1, filesz: u64::MAX, memsz: 0 });
    e.phs.push(Ph { p_type: 0x6474_E551, flags: 6, offset: 0, vaddr: 0, filesz: 0, memsz: 0 });
    assert_eq!(e.check().expect("still valid").segments().len(), 3);
}

// V10
#[test]
fn v10_rejects_an_image_that_needs_more_memory_than_allowed() {
    let tight = Policy { max_mem: 0x3000, ..KERNEL_POLICY };
    assert_eq!(validate(&good().bytes(), &tight).unwrap_err(), Reject::Size);
    let exact = Policy { max_mem: 0x4000, ..KERNEL_POLICY };
    assert!(validate(&good().bytes(), &exact).is_ok());
}

#[test]
fn the_first_failing_rule_gives_the_reason() {
    // V2 and V3 both fail: V2 is reported.
    assert_eq!(rejects(|e| { e.ident[0] = 0; e.e_type = 3 }), Reject::Ident);
    // V5 (segment) and V7 (flags) both fail: V5 is reported.
    assert_eq!(rejects(|e| { e.phs[0].filesz = 0x999; e.phs[2].flags = 7 }), Reject::Segment);
    // V7 and V8 both fail: V7 is reported.
    assert_eq!(rejects(|e| { e.phs[2].flags = 7; e.entry = 0 }), Reject::Wx);
}

#[test]
fn reason_words_match_the_loader_log() {
    let words: Vec<&str> = [
        Reject::Size, Reject::Ident, Reject::Type, Reject::Phdr, Reject::Segment,
        Reject::Range, Reject::Wx, Reject::Entry, Reject::Dynamic,
    ]
    .iter()
    .map(|r| r.as_str())
    .collect();
    assert_eq!(
        words,
        ["size", "ident", "type", "phdr", "segment", "range", "wx", "entry", "dynamic"]
    );
}
