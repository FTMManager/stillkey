//! Step 5: the kernel image and its boot stack in physical memory.

use core::ptr;

use sk_elf::Image;
use stillkey_bootinfo::BOOT_STACK_SIZE;

use crate::mem::{self, PAGE};

pub struct Loaded {
    pub image_phys: u64,
    pub image_pages: usize,
    pub stack_phys: u64,
    pub stack_pages: usize,
}

pub enum LoadError {
    Image,
    Stack,
}

/// Allocates and fills the image, then allocates the stack. `image` is the
/// result of validating `file`, so every offset below is inside `file`.
pub fn load(file: &[u8], image: &Image) -> Result<Loaded, LoadError> {
    // One zeroed allocation: the zeroing provides .bss.
    let image_pages = mem::pages_for(image.span).ok_or(LoadError::Image)?;
    let image_phys = mem::alloc_zeroed(image_pages).ok_or(LoadError::Image)?;
    for segment in image.segments() {
        let bytes = &file[segment.offset as usize..][..segment.filesz as usize];
        let to = image_phys + (segment.vaddr - image.base);
        // SAFETY: rule V10 bounds every segment inside `image.span` bytes from
        // `image.base`, and `filesz <= memsz`, so the destination lies inside
        // the allocation made above. Source and destination do not overlap:
        // they are separate allocations.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), to as *mut u8, bytes.len()) };
    }

    // Zeroed, so the "return address" the kernel finds at its first RSP is 0.
    let stack_pages = (BOOT_STACK_SIZE / PAGE) as usize;
    let stack_phys = mem::alloc_zeroed(stack_pages).ok_or(LoadError::Stack)?;

    Ok(Loaded { image_phys, image_pages, stack_phys, stack_pages })
}
