//! `BOOTFS.IMG`, version 1 (docs/10 section 5.1): a header, a table of
//! entries, then the files, each at a page-aligned offset.
//!
//! The parser is pure code over a byte slice: every field is decoded by
//! value with a bounds check, and the first rule that fails is named.

#![cfg_attr(not(any(test, feature = "std")), no_std)]
#![forbid(unsafe_code)]

#[cfg(any(test, feature = "std"))]
pub mod build;

pub const MAGIC: [u8; 8] = *b"SKBOOTFS";
pub const VERSION: u32 = 1;
pub const HEADER_SIZE: usize = 64;
pub const ENTRY_SIZE: usize = 192;
pub const MAX_ENTRIES: usize = 64;
pub const MAX_IMAGE: u64 = 32 << 20;
pub const PAGE: u64 = 4096;

pub const CLASS_APP: u32 = 1;
pub const CLASS_SYSTEM: u32 = 2;
pub const KIND_PROGRAM: u32 = 1;
pub const KIND_FILE: u32 = 2;
pub const FLAG_CRITICAL: u32 = 1 << 0;
pub const FLAG_DISPLAY: u32 = 1 << 1;

/// Offsets inside an entry.
const PATH: usize = 0;
const PATH_LEN: usize = 64;
const OFFSET: usize = 64;
const LENGTH: usize = 72;
const SHA256: usize = 80;
const NAME: usize = 112;
const NAME_LEN: usize = 16;
const DISPLAY: usize = 128;
const DISPLAY_LEN: usize = 32;
const CLASS: usize = 160;
const MEM_LIMIT: usize = 164;
const FLAGS: usize = 168;
const KIND: usize = 172;
const RESERVED: usize = 176;

/// Why an image was refused: the `reason=` word of
/// `SK:KERNEL:BOOTFS_REJECTED`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    Magic,
    Version,
    Size,
    Table,
    Name,
    Field,
    Range,
    Order,
}

impl Reject {
    pub const fn as_str(self) -> &'static str {
        match self {
            Reject::Magic => "magic",
            Reject::Version => "version",
            Reject::Size => "size",
            Reject::Table => "table",
            Reject::Name => "name",
            Reject::Field => "field",
            Reject::Range => "range",
            Reject::Order => "order",
        }
    }
}

/// One file of the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    pub path: &'a str,
    pub name: &'a str,
    pub display_name: &'a str,
    pub offset: u64,
    pub length: u64,
    pub sha256: [u8; 32],
    pub class: u32,
    /// 0 stands for the default limit.
    pub mem_limit_mib: u32,
    pub flags: u32,
    pub kind: u32,
}

/// An image that passed [`parse`].
#[derive(Clone, Copy, Debug)]
pub struct Image<'a> {
    bytes: &'a [u8],
    count: usize,
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at.checked_add(8)?)?.try_into().ok()?))
}

/// The text before the first zero byte of a fixed-size field: at least one
/// byte, every byte accepted by `allowed`, and nothing but zeros after it.
fn text(field: &[u8], allowed: fn(u8) -> bool) -> Option<&str> {
    let len = field.iter().position(|&b| b == 0)?;
    let (head, tail) = field.split_at(len);
    if len == 0 || !head.iter().all(|&b| allowed(b)) || tail.iter().any(|&b| b != 0) {
        return None;
    }
    core::str::from_utf8(head).ok()
}

fn path_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'/' | b'-')
}

fn name_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
}

fn printable(b: u8) -> bool {
    (0x20..=0x7E).contains(&b)
}

/// Decodes entry `index` of a table that lies inside `bytes`, checking the
/// rules that concern one entry alone.
fn entry_at(bytes: &[u8], index: usize, data_offset: u64) -> Result<Entry<'_>, Reject> {
    let start = HEADER_SIZE + index * ENTRY_SIZE;
    let raw = bytes.get(start..start + ENTRY_SIZE).ok_or(Reject::Table)?;

    let path = text(&raw[PATH..PATH + PATH_LEN], path_byte).ok_or(Reject::Name)?;
    if path.split('/').any(str::is_empty) {
        return Err(Reject::Name);
    }
    let name = text(&raw[NAME..NAME + NAME_LEN], name_byte).ok_or(Reject::Name)?;
    let display_name = text(&raw[DISPLAY..DISPLAY + DISPLAY_LEN], printable).ok_or(Reject::Name)?;

    let class = u32_at(raw, CLASS).ok_or(Reject::Field)?;
    let mem_limit_mib = u32_at(raw, MEM_LIMIT).ok_or(Reject::Field)?;
    let flags = u32_at(raw, FLAGS).ok_or(Reject::Field)?;
    let kind = u32_at(raw, KIND).ok_or(Reject::Field)?;
    if !matches!(class, CLASS_APP | CLASS_SYSTEM)
        || !matches!(kind, KIND_PROGRAM | KIND_FILE)
        || flags & !(FLAG_CRITICAL | FLAG_DISPLAY) != 0
        || raw[RESERVED..].iter().any(|&b| b != 0)
    {
        return Err(Reject::Field);
    }

    let offset = u64_at(raw, OFFSET).ok_or(Reject::Range)?;
    let length = u64_at(raw, LENGTH).ok_or(Reject::Range)?;
    let end = offset.checked_add(length).ok_or(Reject::Range)?;
    if offset % PAGE != 0 || offset < data_offset || length == 0 || end > bytes.len() as u64 {
        return Err(Reject::Range);
    }

    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(&raw[SHA256..SHA256 + 32]);
    Ok(Entry { path, name, display_name, offset, length, sha256, class, mem_limit_mib, flags, kind })
}

/// Accepts `image` only if every rule of docs/10 section 5.1 holds, checked
/// in the order of that section.
pub fn parse(image: &[u8]) -> Result<Image<'_>, Reject> {
    if image.len() < HEADER_SIZE || image[..8] != MAGIC {
        return Err(Reject::Magic);
    }
    let field = |at| u32_at(image, at).ok_or(Reject::Version);
    if field(8)? != VERSION || field(12)? != HEADER_SIZE as u32 || field(20)? != ENTRY_SIZE as u32 {
        return Err(Reject::Version);
    }
    let total_size = u64_at(image, 24).ok_or(Reject::Size)?;
    if total_size != image.len() as u64 || total_size > MAX_IMAGE {
        return Err(Reject::Size);
    }

    let count = u32_at(image, 16).ok_or(Reject::Table)? as usize;
    let table_offset = u64_at(image, 32).ok_or(Reject::Table)?;
    let data_offset = u64_at(image, 40).ok_or(Reject::Table)?;
    let table_end = (HEADER_SIZE + count.min(MAX_ENTRIES) * ENTRY_SIZE) as u64;
    if count == 0
        || count > MAX_ENTRIES
        || table_offset != HEADER_SIZE as u64
        || data_offset % PAGE != 0
        || table_end > data_offset
        || data_offset > total_size
        || image[48..HEADER_SIZE].iter().any(|&b| b != 0)
    {
        return Err(Reject::Table);
    }

    // Each entry alone, then the entries against each other.
    for index in 0..count {
        entry_at(image, index, data_offset)?;
    }
    let parsed = Image { bytes: image, count };
    for (i, a) in parsed.entries().enumerate() {
        for (j, b) in parsed.entries().enumerate().skip(i + 1) {
            let sorted = j != i + 1 || a.path < b.path;
            let apart = a.offset + a.length <= b.offset || b.offset + b.length <= a.offset;
            let same_program =
                a.kind == KIND_PROGRAM && b.kind == KIND_PROGRAM && a.name == b.name;
            if !sorted || !apart || same_program {
                return Err(Reject::Order);
            }
        }
    }
    Ok(parsed)
}

impl<'a> Image<'a> {
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Bytes of the whole image.
    pub fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// The entries, in path order.
    pub fn entries(&self) -> impl Iterator<Item = Entry<'a>> + use<'a> {
        let bytes = self.bytes;
        let data_offset = u64_at(bytes, 40).unwrap_or(0);
        // `parse` decoded every entry once already, so none fails here.
        (0..self.count).filter_map(move |index| entry_at(bytes, index, data_offset).ok())
    }

    pub fn find(&self, path: &str) -> Option<Entry<'a>> {
        self.entries().find(|entry| entry.path == path)
    }

    /// The file an entry of this image describes.
    pub fn data(&self, entry: &Entry<'_>) -> &'a [u8] {
        // `parse` showed that the range lies inside the image.
        &self.bytes[entry.offset as usize..(entry.offset + entry.length) as usize]
    }
}
