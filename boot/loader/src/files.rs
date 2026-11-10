//! Reading the loader's two files from its own volume (docs/02 section 3).
//! Read-only: no write, delete, flush or set-info call exists here.

use uefi::boot::{self, ScopedProtocol};
use uefi::proto::media::file::{Directory, File, FileAttribute, FileInfo, FileMode, RegularFile};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::{CStr16, Status, cstr16};

const CMDLINE_PATH: &CStr16 = cstr16!("\\EFI\\STILLKEY\\CMDLINE.TXT");
const KERNEL_PATH: &CStr16 = cstr16!("\\EFI\\STILLKEY\\KERNEL.ELF");

/// The volume the loader was started from. Dropping it closes the root
/// directory and then the protocol, which must happen before step 9.
pub struct Volume {
    root: Directory,
    _fs: ScopedProtocol<SimpleFileSystem>,
}

/// `FileInfo` must be read into storage aligned for its 64-bit fields.
#[repr(C, align(8))]
struct InfoBuf([u8; 512]);

impl Volume {
    pub fn open() -> Result<Volume, Status> {
        let mut fs = boot::get_image_file_system(boot::image_handle()).map_err(|e| e.status())?;
        let root = fs.open_volume().map_err(|e| e.status())?;
        Ok(Volume { root, _fs: fs })
    }

    fn open_file(&mut self, path: &CStr16) -> Result<RegularFile, Status> {
        let handle = self
            .root
            .open(path, FileMode::Read, FileAttribute::empty())
            .map_err(|e| e.status())?;
        // A directory of that name is not the file.
        handle.into_regular_file().ok_or(Status::NOT_FOUND)
    }

    /// Reads at most the first `buf.len()` bytes of `CMDLINE.TXT`. A file
    /// that cannot be opened or read counts as absent: the result is 0.
    pub fn read_cmdline(&mut self, buf: &mut [u8]) -> usize {
        let Ok(mut file) = self.open_file(CMDLINE_PATH) else {
            return 0;
        };
        let mut len = 0;
        while len < buf.len() {
            match file.read(&mut buf[len..]) {
                Ok(0) => break,
                Ok(n) => len += n,
                Err(_) => return 0,
            }
        }
        len
    }

    /// Opens `KERNEL.ELF` and reports the size `GetInfo` gives for it.
    pub fn open_kernel(&mut self) -> Result<(RegularFile, u64), Status> {
        let mut file = self.open_file(KERNEL_PATH)?;
        let mut buf = InfoBuf([0; 512]);
        let info: &mut FileInfo = file.get_info(&mut buf.0).map_err(|e| e.status())?;
        let size = info.file_size();
        Ok((file, size))
    }
}

/// Fills `buf` from the file's current position.
pub fn read_exact(file: &mut RegularFile, buf: &mut [u8]) -> Result<(), Status> {
    let mut done = 0;
    while done < buf.len() {
        match file.read(&mut buf[done..]) {
            Ok(0) => return Err(Status::END_OF_FILE),
            Ok(n) => done += n,
            Err(e) => return Err(e.status()),
        }
    }
    Ok(())
}
