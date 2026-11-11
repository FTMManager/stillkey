//! Packing an image on the host. The output for the same input is the same
//! bytes: entries are sorted by path and nothing else goes in.

use sha2::{Digest, Sha256};

use crate::{ENTRY_SIZE, HEADER_SIZE, MAGIC, PAGE, VERSION};

/// One file to pack.
#[derive(Clone, Debug)]
pub struct Source {
    pub path: String,
    pub name: String,
    pub display_name: String,
    pub class: u32,
    pub mem_limit_mib: u32,
    pub flags: u32,
    pub kind: u32,
    pub data: Vec<u8>,
}

fn page_up(value: usize) -> usize {
    value.next_multiple_of(PAGE as usize)
}

fn put(field: &mut [u8], text: &str, what: &str) -> Result<(), String> {
    // One byte stays zero: the reader finds the end of the text by it.
    if text.len() >= field.len() {
        return Err(format!("{what} `{text}` is longer than {} bytes", field.len() - 1));
    }
    field[..text.len()].copy_from_slice(text.as_bytes());
    Ok(())
}

/// Builds the image and checks it with [`crate::parse`], so that nothing is
/// packed that the kernel would refuse.
pub fn pack(files: &[Source]) -> Result<Vec<u8>, String> {
    let mut files: Vec<&Source> = files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let data_offset = page_up(HEADER_SIZE + files.len() * ENTRY_SIZE);
    let mut image = vec![0u8; data_offset];
    for (index, file) in files.iter().enumerate() {
        let offset = image.len();
        image.extend_from_slice(&file.data);
        image.resize(page_up(image.len()), 0);

        let mut entry = [0u8; ENTRY_SIZE];
        put(&mut entry[0..64], &file.path, "path")?;
        entry[64..72].copy_from_slice(&(offset as u64).to_le_bytes());
        entry[72..80].copy_from_slice(&(file.data.len() as u64).to_le_bytes());
        entry[80..112].copy_from_slice(&Sha256::digest(&file.data));
        put(&mut entry[112..128], &file.name, "name")?;
        put(&mut entry[128..160], &file.display_name, "display name")?;
        entry[160..164].copy_from_slice(&file.class.to_le_bytes());
        entry[164..168].copy_from_slice(&file.mem_limit_mib.to_le_bytes());
        entry[168..172].copy_from_slice(&file.flags.to_le_bytes());
        entry[172..176].copy_from_slice(&file.kind.to_le_bytes());
        let at = HEADER_SIZE + index * ENTRY_SIZE;
        image[at..at + ENTRY_SIZE].copy_from_slice(&entry);
    }

    let total = image.len() as u64;
    image[0..8].copy_from_slice(&MAGIC);
    image[8..12].copy_from_slice(&VERSION.to_le_bytes());
    image[12..16].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
    image[16..20].copy_from_slice(&(files.len() as u32).to_le_bytes());
    image[20..24].copy_from_slice(&(ENTRY_SIZE as u32).to_le_bytes());
    image[24..32].copy_from_slice(&total.to_le_bytes());
    image[32..40].copy_from_slice(&(HEADER_SIZE as u64).to_le_bytes());
    image[40..48].copy_from_slice(&(data_offset as u64).to_le_bytes());

    crate::parse(&image).map_err(|reason| format!("the packed image is invalid: {}", reason.as_str()))?;
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CLASS_APP, CLASS_SYSTEM, KIND_FILE, KIND_PROGRAM, Reject, parse};

    fn source(path: &str, name: &str, data: &[u8]) -> Source {
        Source {
            path: path.to_string(),
            name: name.to_string(),
            display_name: format!("The {name}"),
            class: CLASS_APP,
            mem_limit_mib: 0,
            flags: 0,
            kind: KIND_PROGRAM,
            data: data.to_vec(),
        }
    }

    fn three() -> Vec<Source> {
        let mut licence = source("licenses/spleen.txt", "spleen", &[7u8; 5000]);
        licence.kind = KIND_FILE;
        let mut inspector = source("bin/inspector", "inspector", b"second");
        inspector.class = CLASS_SYSTEM;
        vec![licence, source("bin/hello", "hello", b"first"), inspector]
    }

    /// Byte offset of a field of entry `index`.
    fn at(index: usize, field: usize) -> usize {
        HEADER_SIZE + index * ENTRY_SIZE + field
    }

    #[test]
    fn a_packed_image_parses_back() {
        let image = pack(&three()).unwrap();
        let parsed = parse(&image).unwrap();
        let entries: Vec<_> = parsed.entries().collect();
        assert_eq!(entries.len(), 3);
        // Sorted by path, whatever the input order.
        assert_eq!(entries[0].path, "bin/hello");
        assert_eq!(entries[1].path, "bin/inspector");
        assert_eq!(entries[2].path, "licenses/spleen.txt");
        assert_eq!(parsed.data(&entries[0]), b"first");
        assert_eq!(parsed.data(&entries[2]), &[7u8; 5000][..]);
        assert_eq!(entries[0].offset, 4096);
        assert_eq!(entries[1].offset, 8192);
        assert_eq!(entries[2].offset, 12288);
        assert_eq!(image.len(), 12288 + 8192);
        assert_eq!(entries[1].class, CLASS_SYSTEM);
        assert_eq!(entries[0].display_name, "The hello");
        assert_eq!(entries[0].sha256[..], Sha256::digest(b"first")[..]);
        assert_eq!(parsed.find("bin/inspector").unwrap().name, "inspector");
        assert!(parsed.find("bin/absent").is_none());
    }

    #[test]
    fn the_same_files_in_another_order_give_the_same_bytes() {
        let mut shuffled = three();
        shuffled.reverse();
        assert_eq!(pack(&three()).unwrap(), pack(&shuffled).unwrap());
    }

    #[test]
    fn a_name_that_does_not_fit_is_an_error_not_a_cut() {
        assert!(pack(&[source("bin/x", "sixteen-letters-", b"x")]).is_err());
        assert!(pack(&[source("bin/x", "fifteen-letters", b"x")]).is_ok());
    }

    #[test]
    fn each_rule_names_its_reason() {
        let good = pack(&three()).unwrap();
        let broken = |edit: &dyn Fn(&mut Vec<u8>)| {
            let mut image = good.clone();
            edit(&mut image);
            parse(&image).map(|_| ()).unwrap_err()
        };
        assert_eq!(broken(&|i| i[0] = b'X'), Reject::Magic);
        assert_eq!(parse(&good[..32]).unwrap_err(), Reject::Magic);
        assert_eq!(broken(&|i| i[8] = 2), Reject::Version);
        assert_eq!(broken(&|i| i[20] = 191), Reject::Version);
        // The stated size is not the size of the memory that was handed over.
        assert_eq!(broken(&|i| i.truncate(i.len() - 4096)), Reject::Size);
        assert_eq!(broken(&|i| i[16] = 0), Reject::Table);
        assert_eq!(broken(&|i| i[16] = 65), Reject::Table);
        assert_eq!(broken(&|i| i[40] = 1), Reject::Table);
        assert_eq!(broken(&|i| i[at(0, 0)] = b'B'), Reject::Name);
        assert_eq!(broken(&|i| i[at(0, 4)] = b'/'), Reject::Name); // "bin//ello"
        assert_eq!(broken(&|i| i[at(0, 63)] = b'x'), Reject::Name); // text after the zero
        assert_eq!(broken(&|i| i[at(0, 112)] = b'_'), Reject::Name);
        assert_eq!(broken(&|i| i[at(0, 160)] = 3), Reject::Field);
        assert_eq!(broken(&|i| i[at(0, 168)] = 4), Reject::Field);
        assert_eq!(broken(&|i| i[at(0, 172)] = 0), Reject::Field);
        assert_eq!(broken(&|i| i[at(0, 191)] = 1), Reject::Field);
        assert_eq!(broken(&|i| i[at(0, 64)] = 8), Reject::Range); // offset 4104
        assert_eq!(broken(&|i| i[at(0, 72)] = 0), Reject::Range); // length 0
        assert_eq!(broken(&|i| i[at(2, 74)] = 0xFF), Reject::Range); // past the end
    }

    #[test]
    fn entries_must_be_sorted_apart_and_uniquely_named() {
        let good = pack(&three()).unwrap();
        // Swap the first two entries: valid each, but out of order.
        let mut swapped = good.clone();
        let (a, b) = (at(0, 0), at(1, 0));
        let first = swapped[a..a + ENTRY_SIZE].to_vec();
        swapped.copy_within(b..b + ENTRY_SIZE, a);
        swapped[b..b + ENTRY_SIZE].copy_from_slice(&first);
        assert_eq!(parse(&swapped).unwrap_err(), Reject::Order);

        // Two entries that claim the same bytes.
        let mut shared = good.clone();
        shared[at(1, 64)..at(1, 72)].copy_from_slice(&4096u64.to_le_bytes());
        assert_eq!(parse(&shared).unwrap_err(), Reject::Order);

        // Two programs with one name.
        let twins = [source("bin/a", "same", b"a"), source("bin/b", "same", b"b")];
        assert!(pack(&twins).is_err());
    }

    fn many(count: usize) -> Vec<Source> {
        (0..count).map(|i| source(&format!("bin/p{i}"), &format!("p{i}"), b"data")).collect()
    }

    #[test]
    fn the_table_may_be_full_and_no_fuller() {
        let image = pack(&many(crate::MAX_ENTRIES)).unwrap();
        assert_eq!(parse(&image).unwrap().entries().count(), crate::MAX_ENTRIES);
        assert!(pack(&many(crate::MAX_ENTRIES + 1)).is_err());
    }

    #[test]
    fn a_table_that_ends_at_a_page_end_is_accepted() {
        // 64 + 21 * 192 = 4096: the data starts where the table ends.
        assert_eq!(HEADER_SIZE + 21 * ENTRY_SIZE, PAGE as usize);
        let image = pack(&many(21)).unwrap();
        assert_eq!(parse(&image).unwrap().entries().count(), 21);
    }

    #[test]
    fn a_name_must_leave_room_for_its_end() {
        assert!(pack(&[source("bin/a", &"n".repeat(15), b"data")]).is_ok());
        let error = pack(&[source("bin/a", &"n".repeat(16), b"data")]).unwrap_err();
        assert!(error.contains("longer than 15 bytes"), "{error}");
    }
}
