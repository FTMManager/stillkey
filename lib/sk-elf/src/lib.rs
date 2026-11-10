//! Strict ELF64 validator: rules V1-V10 of docs/02 section 4.
//!
//! Pure code over a byte slice. It accepts a deliberately narrow subset of
//! ELF64 and names the first rule that fails.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests;

pub const MAX_SEGMENTS: usize = 16;
pub const PAGE: u64 = 4096;

/// What an image may look like. `window_last` is inclusive so that a window
/// reaching the top of the address space can be expressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub window_first: u64,
    pub window_last: u64,
    pub max_file: u64,
    pub max_mem: u64,
}

/// The policy the loader applies to `KERNEL.ELF`.
pub const KERNEL_POLICY: Policy = Policy {
    window_first: 0xFFFF_FFFF_8000_0000,
    window_last: u64::MAX,
    max_file: 64 << 20,
    max_mem: 64 << 20,
};

/// The policy the kernel applies to a ring-3 program whose memory limit is
/// `max_mem` (docs/10 section 8.3): the image lies in 4 MiB..2 GiB.
pub const fn user_policy(max_mem: u64) -> Policy {
    Policy { window_first: 0x40_0000, window_last: 0x7FFF_FFFF, max_file: 16 << 20, max_mem }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    Size,
    Ident,
    Type,
    Phdr,
    Segment,
    Range,
    Wx,
    Entry,
    Dynamic,
}

impl Reject {
    pub const fn as_str(self) -> &'static str {
        match self {
            Reject::Size => "size",
            Reject::Ident => "ident",
            Reject::Type => "type",
            Reject::Phdr => "phdr",
            Reject::Segment => "segment",
            Reject::Range => "range",
            Reject::Wx => "wx",
            Reject::Entry => "entry",
            Reject::Dynamic => "dynamic",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Segment {
    pub vaddr: u64,
    pub offset: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub flags: u32,
}

/// A validated image: every `PT_LOAD` segment, in ascending address order.
#[derive(Clone, Copy, Debug)]
pub struct Image {
    pub entry: u64,
    /// Lowest segment address.
    pub base: u64,
    /// Bytes from `base` to the page-rounded end of the last segment.
    pub span: u64,
    segments: [Segment; MAX_SEGMENTS],
    count: usize,
}

impl Image {
    pub fn segments(&self) -> &[Segment] {
        &self.segments[..self.count]
    }
}

const EHDR_SIZE: u64 = 64;
const PHDR_SIZE: u64 = 56;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_TLS: u32 = 7;
const FLAGS_R: u32 = 4;
const FLAGS_RX: u32 = 5;
const FLAGS_RW: u32 = 6;

/// Rule V1 alone, for a caller that knows the size before it has the bytes.
pub fn check_file_size(len: u64, policy: &Policy) -> Result<(), Reject> {
    if len < EHDR_SIZE || len > policy.max_file {
        return Err(Reject::Size);
    }
    Ok(())
}

/// Little-endian fields at byte offsets the caller has already bounds-checked.
/// A read that is out of range all the same gives `None`, never a panic.
fn bytes<const N: usize>(file: &[u8], at: u64) -> Option<[u8; N]> {
    let at = usize::try_from(at).ok()?;
    file.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn u16_at(file: &[u8], at: u64) -> Option<u16> {
    bytes(file, at).map(u16::from_le_bytes)
}

fn u32_at(file: &[u8], at: u64) -> Option<u32> {
    bytes(file, at).map(u32::from_le_bytes)
}

fn u64_at(file: &[u8], at: u64) -> Option<u64> {
    bytes(file, at).map(u64::from_le_bytes)
}

fn page_up(addr: u64) -> Option<u64> {
    Some(addr.checked_add(PAGE - 1)? & !(PAGE - 1))
}

/// Applies V1-V10 in order and returns the first failure.
pub fn validate(file: &[u8], policy: &Policy) -> Result<Image, Reject> {
    let len = file.len() as u64;

    // V1
    check_file_size(len, policy)?;

    // V2
    let ident: [u8; 7] = bytes(file, 0).ok_or(Reject::Ident)?;
    if ident != [0x7F, b'E', b'L', b'F', 2, 1, 1] {
        return Err(Reject::Ident);
    }

    // V3
    let e_type = u16_at(file, 16).ok_or(Reject::Type)?;
    let e_machine = u16_at(file, 18).ok_or(Reject::Type)?;
    let e_version = u32_at(file, 20).ok_or(Reject::Type)?;
    if e_type != 2 || e_machine != 62 || e_version != 1 {
        return Err(Reject::Type);
    }

    // V4
    let entry = u64_at(file, 24).ok_or(Reject::Phdr)?;
    let phoff = u64_at(file, 32).ok_or(Reject::Phdr)?;
    let ehsize = u16_at(file, 52).ok_or(Reject::Phdr)?;
    let phentsize = u16_at(file, 54).ok_or(Reject::Phdr)?;
    let phnum = u16_at(file, 56).ok_or(Reject::Phdr)?;
    if u64::from(ehsize) != EHDR_SIZE
        || u64::from(phentsize) != PHDR_SIZE
        || phnum == 0
        || usize::from(phnum) > MAX_SEGMENTS
    {
        return Err(Reject::Phdr);
    }
    let table_end = phoff.checked_add(u64::from(phnum) * PHDR_SIZE).ok_or(Reject::Phdr)?;
    if table_end > len {
        return Err(Reject::Phdr);
    }

    // Read the table once. V5-V8 apply to the PT_LOAD headers, V9 to the rest.
    let mut segments = [Segment::default(); MAX_SEGMENTS];
    let mut count = 0;
    let mut forbidden = false;
    for i in 0..u64::from(phnum) {
        let at = phoff + i * PHDR_SIZE;
        match u32_at(file, at).ok_or(Reject::Phdr)? {
            PT_LOAD => {
                segments[count] = Segment {
                    flags: u32_at(file, at + 4).ok_or(Reject::Phdr)?,
                    offset: u64_at(file, at + 8).ok_or(Reject::Phdr)?,
                    vaddr: u64_at(file, at + 16).ok_or(Reject::Phdr)?,
                    filesz: u64_at(file, at + 32).ok_or(Reject::Phdr)?,
                    memsz: u64_at(file, at + 40).ok_or(Reject::Phdr)?,
                };
                count += 1;
            }
            PT_DYNAMIC | PT_INTERP | PT_TLS => forbidden = true,
            _ => {}
        }
    }
    let loads = &segments[..count];

    // V5
    for s in loads {
        let file_end = s.offset.checked_add(s.filesz).ok_or(Reject::Segment)?;
        if s.filesz > s.memsz
            || s.memsz == 0
            || file_end > len
            || s.vaddr % PAGE != 0
            || s.offset % PAGE != s.vaddr % PAGE
        {
            return Err(Reject::Segment);
        }
    }

    // V6
    let mut prev_end = None;
    for s in loads {
        if s.vaddr < policy.window_first {
            return Err(Reject::Range);
        }
        let end = s.vaddr.checked_add(s.memsz).ok_or(Reject::Range)?;
        if end - 1 > policy.window_last {
            return Err(Reject::Range);
        }
        if prev_end.is_some_and(|prev| prev > s.vaddr) {
            return Err(Reject::Range);
        }
        prev_end = Some(page_up(end).ok_or(Reject::Range)?);
    }

    // V7
    if loads.iter().any(|s| !matches!(s.flags, FLAGS_R | FLAGS_RX | FLAGS_RW)) {
        return Err(Reject::Wx);
    }

    // V8
    let entry_ok = loads
        .iter()
        .any(|s| s.flags == FLAGS_RX && entry >= s.vaddr && entry - s.vaddr < s.filesz);
    if !entry_ok {
        return Err(Reject::Entry);
    }

    // V9
    if forbidden {
        return Err(Reject::Dynamic);
    }

    // V10. V8 passed, so there is at least one segment.
    let base = loads[0].vaddr;
    let span = prev_end.ok_or(Reject::Size)? - base;
    if span > policy.max_mem {
        return Err(Reject::Size);
    }

    Ok(Image { entry, base, span, segments, count })
}
