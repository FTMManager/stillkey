//! Which thread runs next (docs/10 section 8.5): a round-robin queue of
//! ready threads and a list of sleepers. No memory is allocated: `N` is the
//! largest number of threads.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tid(pub u32);

#[derive(Clone, Copy)]
struct Sleeper {
    until: u64,
    /// Rises with every call of `sleep`: the order among equal deadlines.
    order: u64,
    tid: Tid,
}

pub struct Queue<const N: usize> {
    ready: [Tid; N],
    head: usize,
    len: usize,
    sleepers: [Sleeper; N],
    sleeping: usize,
    order: u64,
}

impl<const N: usize> Default for Queue<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Queue<N> {
    pub const fn new() -> Self {
        Queue {
            ready: [Tid(0); N],
            head: 0,
            len: 0,
            sleepers: [Sleeper { until: 0, order: 0, tid: Tid(0) }; N],
            sleeping: 0,
            order: 0,
        }
    }

    fn is_ready(&self, tid: Tid) -> bool {
        (0..self.len).any(|i| self.ready[(self.head + i) % N] == tid)
    }

    pub fn is_sleeping(&self, tid: Tid) -> bool {
        self.sleepers[..self.sleeping].iter().any(|s| s.tid == tid)
    }

    /// Threads that could run now.
    pub fn ready_len(&self) -> usize {
        self.len
    }

    /// Puts a thread at the back of the ready queue.
    ///
    /// # Panics
    /// If the thread is in the queue or asleep already, or the queue is full.
    pub fn add(&mut self, tid: Tid) {
        assert!(!self.is_ready(tid) && !self.is_sleeping(tid), "thread {} is queued twice", tid.0);
        assert!(self.len < N, "more threads than the queue holds");
        self.ready[(self.head + self.len) % N] = tid;
        self.len += 1;
    }

    /// Takes the thread at the front of the ready queue.
    pub fn pick(&mut self) -> Option<Tid> {
        if self.len == 0 {
            return None;
        }
        let tid = self.ready[self.head];
        self.head = (self.head + 1) % N;
        self.len -= 1;
        Some(tid)
    }

    /// Parks a thread until `until_tick`. The thread is not in the ready
    /// queue: it is the one that runs.
    pub fn sleep(&mut self, tid: Tid, until_tick: u64) {
        assert!(!self.is_ready(tid) && !self.is_sleeping(tid), "thread {} is queued twice", tid.0);
        assert!(self.sleeping < N, "more sleepers than the queue holds");
        self.sleepers[self.sleeping] = Sleeper { until: until_tick, order: self.order, tid };
        self.sleeping += 1;
        self.order += 1;
    }

    /// Moves every sleeper whose tick has come to the back of the ready
    /// queue, earliest deadline first and, among equal deadlines, in the
    /// order they went to sleep. Returns how many woke.
    pub fn tick(&mut self, now: u64) -> usize {
        let mut woken = 0;
        loop {
            let due = self.sleepers[..self.sleeping]
                .iter()
                .enumerate()
                .filter(|(_, s)| s.until <= now)
                .min_by_key(|(_, s)| (s.until, s.order))
                .map(|(index, _)| index);
            let Some(index) = due else {
                return woken;
            };
            let tid = self.sleepers[index].tid;
            self.sleepers.copy_within(index + 1..self.sleeping, index);
            self.sleeping -= 1;
            self.add(tid);
            woken += 1;
        }
    }

    /// Forgets a thread, ready or asleep.
    pub fn remove(&mut self, tid: Tid) {
        if let Some(index) = self.sleepers[..self.sleeping].iter().position(|s| s.tid == tid) {
            self.sleepers.copy_within(index + 1..self.sleeping, index);
            self.sleeping -= 1;
        }
        let mut kept = 0;
        for i in 0..self.len {
            let other = self.ready[(self.head + i) % N];
            if other != tid {
                self.ready[(self.head + kept) % N] = other;
                kept += 1;
            }
        }
        self.len = kept;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Tid = Tid(1);
    const B: Tid = Tid(2);
    const C: Tid = Tid(3);

    #[test]
    fn three_ready_threads_rotate_in_order() {
        let mut queue = Queue::<4>::new();
        for tid in [A, B, C] {
            queue.add(tid);
        }
        let mut seen = Vec::new();
        for _ in 0..7 {
            let tid = queue.pick().unwrap();
            seen.push(tid);
            queue.add(tid);
        }
        assert_eq!(seen, [A, B, C, A, B, C, A]);
        assert_eq!(queue.ready_len(), 3);
    }

    #[test]
    fn an_empty_queue_picks_nothing() {
        let mut queue = Queue::<4>::new();
        assert_eq!(queue.pick(), None);
        queue.add(A);
        assert_eq!(queue.pick(), Some(A));
        assert_eq!(queue.pick(), None);
    }

    #[test]
    fn a_sleeper_is_picked_at_its_tick_and_not_before() {
        let mut queue = Queue::<4>::new();
        queue.sleep(A, 5);
        assert!(queue.is_sleeping(A));
        assert_eq!(queue.tick(4), 0);
        assert_eq!(queue.pick(), None);
        assert_eq!(queue.tick(5), 1);
        assert!(!queue.is_sleeping(A));
        assert_eq!(queue.pick(), Some(A));
        assert_eq!(queue.tick(6), 0);
    }

    #[test]
    fn sleepers_wake_by_deadline_then_by_who_slept_first() {
        let mut queue = Queue::<4>::new();
        queue.sleep(C, 9);
        queue.sleep(B, 7);
        queue.sleep(A, 9);
        assert_eq!(queue.tick(8), 1);
        assert_eq!(queue.tick(20), 2);
        assert_eq!([queue.pick(), queue.pick(), queue.pick(), queue.pick()], [Some(B), Some(C), Some(A), None]);
    }

    #[test]
    fn a_woken_thread_waits_behind_the_ready_ones() {
        let mut queue = Queue::<4>::new();
        queue.sleep(A, 1);
        queue.add(B);
        queue.tick(1);
        assert_eq!([queue.pick(), queue.pick()], [Some(B), Some(A)]);
    }

    #[test]
    fn a_removed_thread_is_never_picked() {
        let mut queue = Queue::<4>::new();
        for tid in [A, B, C] {
            queue.add(tid);
        }
        queue.remove(B);
        assert_eq!([queue.pick(), queue.pick(), queue.pick()], [Some(A), Some(C), None]);

        queue.sleep(A, 3);
        queue.sleep(B, 3);
        queue.remove(A);
        assert_eq!(queue.tick(10), 1);
        assert_eq!([queue.pick(), queue.pick()], [Some(B), None]);
    }

    #[test]
    fn the_ring_wraps() {
        let mut queue = Queue::<2>::new();
        for round in 0..10 {
            queue.add(Tid(round));
            queue.add(Tid(round + 100));
            assert_eq!(queue.pick(), Some(Tid(round)));
            queue.remove(Tid(round + 100));
            assert_eq!(queue.pick(), None);
        }
    }

    #[test]
    #[should_panic(expected = "queued twice")]
    fn a_thread_cannot_be_ready_twice() {
        let mut queue = Queue::<4>::new();
        queue.add(A);
        queue.add(A);
    }
}
