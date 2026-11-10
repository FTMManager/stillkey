//! The physical frame allocator's bookkeeping: one bit per 4 KiB frame,
//! set while the frame is free.
//!
//! The kernel builds it once from the handoff's memory map (`release` for
//! every region it may use, `reserve` for everything it must not touch) and
//! then only allocates and frees single frames.

/// Why a frame was not freed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeError {
    /// The frame number is beyond the bitmap.
    OutOfRange,
    /// The frame is free already: a second free, or a frame never handed out.
    NotAllocated,
}

pub struct Bitmap<'a> {
    words: &'a mut [u64],
    frames: u64,
    free: u64,
    /// The word the next search starts at.
    hint: usize,
}

impl<'a> Bitmap<'a> {
    /// Words needed for `frames` frames.
    pub const fn words_for(frames: u64) -> usize {
        frames.div_ceil(64) as usize
    }

    /// A bitmap for frames `0..frames` in which no frame is free.
    ///
    /// # Panics
    /// If `words` is too short.
    pub fn new(words: &'a mut [u64], frames: u64) -> Bitmap<'a> {
        assert!(words.len() >= Bitmap::words_for(frames), "the bitmap storage is too short");
        words.fill(0);
        Bitmap { words, frames, free: 0, hint: 0 }
    }

    fn set(&mut self, frame: u64, free: bool) {
        let (word, bit) = ((frame / 64) as usize, 1u64 << (frame % 64));
        let was_free = self.words[word] & bit != 0;
        match (was_free, free) {
            (false, true) => {
                self.words[word] |= bit;
                self.free += 1;
            }
            (true, false) => {
                self.words[word] &= !bit;
                self.free -= 1;
            }
            _ => {}
        }
    }

    /// Makes the frames `first..first + count` free, as far as the bitmap
    /// reaches. A frame that is free already stays free and is counted once.
    pub fn release(&mut self, first: u64, count: u64) {
        for frame in first..first.saturating_add(count).min(self.frames) {
            self.set(frame, true);
        }
    }

    /// Takes the frames `first..first + count` out of the free set, whether
    /// or not they were in it.
    pub fn reserve(&mut self, first: u64, count: u64) {
        for frame in first..first.saturating_add(count).min(self.frames) {
            self.set(frame, false);
        }
    }

    /// The lowest free frame at or after the last allocation's word, else
    /// the lowest free frame.
    pub fn alloc(&mut self) -> Option<u64> {
        if self.free == 0 {
            return None;
        }
        let len = self.words.len();
        for step in 0..len {
            let word = (self.hint + step) % len;
            if self.words[word] != 0 {
                let frame = word as u64 * 64 + u64::from(self.words[word].trailing_zeros());
                self.set(frame, false);
                self.hint = word;
                return Some(frame);
            }
        }
        None
    }

    /// Gives a frame back.
    pub fn free(&mut self, frame: u64) -> Result<(), FreeError> {
        if frame >= self.frames {
            return Err(FreeError::OutOfRange);
        }
        if self.is_free(frame) {
            return Err(FreeError::NotAllocated);
        }
        self.set(frame, true);
        // Low frames are reused first, which keeps runs comparable.
        self.hint = self.hint.min((frame / 64) as usize);
        Ok(())
    }

    pub fn is_free(&self, frame: u64) -> bool {
        frame < self.frames && self.words[(frame / 64) as usize] & 1 << (frame % 64) != 0
    }

    pub fn free_count(&self) -> u64 {
        self.free
    }

    /// Frames the bitmap describes, free or not.
    pub fn frames(&self) -> u64 {
        self.frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_bitmap_has_nothing_to_give() {
        let mut words = [u64::MAX; 4];
        let mut map = Bitmap::new(&mut words, 200);
        assert_eq!(map.free_count(), 0);
        assert_eq!(map.alloc(), None);
        assert_eq!(Bitmap::words_for(200), 4);
        assert_eq!(Bitmap::words_for(192), 3);
    }

    #[test]
    fn release_and_reserve_count_each_frame_once() {
        let mut words = [0; 4];
        let mut map = Bitmap::new(&mut words, 200);
        map.release(60, 10); // crosses a word boundary
        assert_eq!(map.free_count(), 10);
        map.release(65, 10); // five of them are free already
        assert_eq!(map.free_count(), 15);
        map.reserve(58, 4); // two of them were never free
        assert_eq!(map.free_count(), 13);
        assert!(!map.is_free(61) && map.is_free(62) && map.is_free(74) && !map.is_free(75));
        // Beyond the bitmap nothing is released.
        map.release(190, 100);
        assert_eq!(map.free_count(), 23);
        assert!(map.is_free(199) && !map.is_free(200));
    }

    #[test]
    fn every_free_frame_is_handed_out_exactly_once() {
        let mut words = [0; 4];
        let mut map = Bitmap::new(&mut words, 200);
        map.release(10, 3);
        map.release(130, 2);
        let mut got = Vec::new();
        while let Some(frame) = map.alloc() {
            got.push(frame);
        }
        assert_eq!(got, [10, 11, 12, 130, 131]);
        assert_eq!(map.free_count(), 0);
    }

    #[test]
    fn a_freed_frame_comes_back_and_low_frames_first() {
        let mut words = [0; 4];
        let mut map = Bitmap::new(&mut words, 200);
        map.release(0, 200);
        for _ in 0..150 {
            map.alloc().unwrap();
        }
        assert_eq!(map.free(7), Ok(()));
        assert_eq!(map.free_count(), 51);
        assert_eq!(map.alloc(), Some(7));
        assert_eq!(map.alloc(), Some(150));
    }

    #[test]
    fn a_second_free_and_a_wild_free_are_refused() {
        let mut words = [0; 4];
        let mut map = Bitmap::new(&mut words, 200);
        map.release(0, 8);
        let frame = map.alloc().unwrap();
        assert_eq!(map.free(frame), Ok(()));
        assert_eq!(map.free(frame), Err(FreeError::NotAllocated));
        assert_eq!(map.free(200), Err(FreeError::OutOfRange));
        assert_eq!(map.free(u64::MAX), Err(FreeError::OutOfRange));
        assert_eq!(map.free_count(), 8);
    }
}
