//! The records `diag_read` returns (docs/10 section 8.4). Every record has a
//! fixed size and little-endian fields with no padding other than what is
//! named, so a reader needs no knowledge of the kernel's own structures.

/// `kind` of `diag_read`.
pub const KIND_PROCESSES: u64 = 1;
pub const KIND_TOTALS: u64 = 2;
pub const KIND_CRASHES: u64 = 3;
pub const KIND_CRASH_REGISTERS: u64 = 4;

/// `ProcessRecord::state`.
pub const STATE_ALIVE: u32 = 1;
pub const STATE_EXITED: u32 = 2;
pub const STATE_CRASHED: u32 = 3;
pub const STATE_ENDED: u32 = 4;

/// `ProcessRecord::wait_reason`.
pub const WAIT_NONE: u32 = 0;
pub const WAIT_SLEEP: u32 = 1;

/// `CrashRecord::flags`, bit 0: the kernel kept the registers of the crash.
pub const CRASH_HAS_REGISTERS: u32 = 1;
/// `CrashRecord::image_offset` when the fault was outside the image.
pub const NO_IMAGE_OFFSET: u64 = u64::MAX;

/// Kind 4: eighteen values of eight bytes: `rax`, `rbx`, `rcx`, `rdx`,
/// `rsi`, `rdi`, `rbp`, `r8` to `r15`, `rip`, `rsp`, `rflags`.
pub const REGISTERS_SIZE: usize = 144;

struct Writer<const N: usize> {
    bytes: [u8; N],
    at: usize,
}

impl<const N: usize> Writer<N> {
    const fn new() -> Self {
        Writer { bytes: [0; N], at: 0 }
    }
    fn put(&mut self, field: &[u8]) {
        self.bytes[self.at..self.at + field.len()].copy_from_slice(field);
        self.at += field.len();
    }
    fn u64(&mut self, value: u64) {
        self.put(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.put(&value.to_le_bytes());
    }
    fn done(self) -> [u8; N] {
        assert!(self.at == N, "a record is not full");
        self.bytes
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn take<const K: usize>(&mut self) -> [u8; K] {
        let (field, rest) = self.bytes.split_at(K);
        self.bytes = rest;
        let mut out = [0; K];
        out.copy_from_slice(field);
        out
    }
    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take())
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }
}

/// The text of a name field: the bytes before the first zero.
pub fn name_of(name: &[u8; 16]) -> &str {
    let len = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    core::str::from_utf8(&name[..len]).unwrap_or("?")
}

/// Kind 1: one process, running or ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessRecord {
    pub pid: u64,
    pub parent: u64,
    pub start_ms: u64,
    pub cpu_ticks: u64,
    pub image_pages: u32,
    pub heap_pages: u32,
    pub stack_pages: u32,
    pub table_pages: u32,
    pub shared_pages: u32,
    pub state: u32,
    pub wait_reason: u32,
    pub class: u32,
    pub threads: u32,
    pub exit_code: u32,
    pub handles: u32,
    pub name: [u8; 16],
}

impl ProcessRecord {
    pub const SIZE: usize = 96;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut w = Writer::new();
        for value in [self.pid, self.parent, self.start_ms, self.cpu_ticks] {
            w.u64(value);
        }
        for value in [
            self.image_pages,
            self.heap_pages,
            self.stack_pages,
            self.table_pages,
            self.shared_pages,
            self.state,
            self.wait_reason,
            self.class,
            self.threads,
            self.exit_code,
            self.handles,
            0,
        ] {
            w.u32(value);
        }
        w.put(&self.name);
        w.done()
    }

    pub fn decode(bytes: &[u8]) -> Option<ProcessRecord> {
        let mut r = Reader { bytes: bytes.get(..Self::SIZE)? };
        let (pid, parent, start_ms, cpu_ticks) = (r.u64(), r.u64(), r.u64(), r.u64());
        let (image_pages, heap_pages, stack_pages, table_pages, shared_pages) =
            (r.u32(), r.u32(), r.u32(), r.u32(), r.u32());
        let (state, wait_reason, class, threads, exit_code, handles, _pad) =
            (r.u32(), r.u32(), r.u32(), r.u32(), r.u32(), r.u32(), r.u32());
        Some(ProcessRecord {
            pid,
            parent,
            start_ms,
            cpu_ticks,
            image_pages,
            heap_pages,
            stack_pages,
            table_pages,
            shared_pages,
            state,
            wait_reason,
            class,
            threads,
            exit_code,
            handles,
            name: r.take(),
        })
    }
}

/// Kind 2: the whole system.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    /// Rises by one whenever a process starts or ends or a crash is recorded.
    pub change: u64,
    pub uptime_ms: u64,
    pub total_pages: u64,
    pub free_pages: u64,
    /// Frames held by the kernel heap and by page tables of the kernel half.
    pub kernel_pages: u64,
    /// Processes that are alive.
    pub processes: u32,
    /// Crash records kept.
    pub crashes: u32,
}

impl Totals {
    pub const SIZE: usize = 48;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut w = Writer::new();
        for value in [self.change, self.uptime_ms, self.total_pages, self.free_pages, self.kernel_pages] {
            w.u64(value);
        }
        w.u32(self.processes);
        w.u32(self.crashes);
        w.done()
    }

    pub fn decode(bytes: &[u8]) -> Option<Totals> {
        let mut r = Reader { bytes: bytes.get(..Self::SIZE)? };
        Some(Totals {
            change: r.u64(),
            uptime_ms: r.u64(),
            total_pages: r.u64(),
            free_pages: r.u64(),
            kernel_pages: r.u64(),
            processes: r.u32(),
            crashes: r.u32(),
        })
    }
}

/// Kind 3: one crash, without registers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CrashRecord {
    pub seq: u64,
    pub time_ms: u64,
    pub pid: u64,
    pub rip: u64,
    pub addr: u64,
    /// `rip` less the lowest image address, or [`NO_IMAGE_OFFSET`].
    pub image_offset: u64,
    pub error: u64,
    pub vector: u32,
    pub flags: u32,
    /// The first bytes of the SHA-256 of the program file.
    pub digest: [u8; 8],
    pub name: [u8; 16],
}

impl CrashRecord {
    pub const SIZE: usize = 88;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut w = Writer::new();
        for value in [self.seq, self.time_ms, self.pid, self.rip, self.addr, self.image_offset, self.error] {
            w.u64(value);
        }
        w.u32(self.vector);
        w.u32(self.flags);
        w.put(&self.digest);
        w.put(&self.name);
        w.done()
    }

    pub fn decode(bytes: &[u8]) -> Option<CrashRecord> {
        let mut r = Reader { bytes: bytes.get(..Self::SIZE)? };
        Some(CrashRecord {
            seq: r.u64(),
            time_ms: r.u64(),
            pid: r.u64(),
            rip: r.u64(),
            addr: r.u64(),
            image_offset: r.u64(),
            error: r.u64(),
            vector: r.u32(),
            flags: r.u32(),
            digest: r.take(),
            name: r.take(),
        })
    }
}
