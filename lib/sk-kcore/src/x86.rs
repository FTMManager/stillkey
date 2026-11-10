//! Encodings the processor defines: descriptors, gates, the task state
//! segment and the frame an interrupt leaves on the stack.

/// Selectors of the descriptor table built by [`gdt`]. User data comes
/// before user code, the order `SYSRET` would need, although Milestone 2
/// enters and leaves the kernel by interrupt only.
pub const KERNEL_CODE: u16 = 0x08;
pub const KERNEL_DATA: u16 = 0x10;
pub const USER_DATA: u16 = 0x18 | 3;
pub const USER_CODE: u16 = 0x20 | 3;
pub const TSS_SELECTOR: u16 = 0x28;

/// Null, four segments, and a task-state descriptor that takes two slots.
pub const GDT_SLOTS: usize = 7;
pub const IDT_GATES: usize = 256;
/// Stacks the processor switches to by itself: double fault, NMI, machine check.
pub const IST_STACKS: usize = 3;
pub const IST_DOUBLE_FAULT: u8 = 1;
pub const IST_NMI: u8 = 2;
pub const IST_MACHINE_CHECK: u8 = 3;

pub const VECTOR_NMI: u8 = 2;
pub const VECTOR_DOUBLE_FAULT: u8 = 8;
pub const VECTOR_PAGE_FAULT: u8 = 14;
pub const VECTOR_MACHINE_CHECK: u8 = 18;
/// The 8259's lines 0-15 are moved to vectors 0x20-0x2F.
pub const VECTOR_TIMER: u8 = 0x20;
pub const VECTOR_PIC_SPURIOUS_MASTER: u8 = 0x27;
pub const VECTOR_PIC_SPURIOUS_SLAVE: u8 = 0x2F;
pub const VECTOR_SYSCALL: u8 = 0x80;
pub const VECTOR_APIC_SPURIOUS: u8 = 0xFF;

/// The task state segment of long mode: stack pointers, nothing else.
#[repr(C, packed)]
pub struct Tss {
    reserved0: u32,
    /// `rsp[0]` is loaded when an interrupt arrives in ring 3.
    pub rsp: [u64; 3],
    reserved1: u64,
    /// `ist[n - 1]` is loaded for a gate whose stack index is `n`.
    pub ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    /// Beyond the segment limit: there is no I/O permission map, so every
    /// port instruction in ring 3 faults.
    pub iomap_base: u16,
}

pub const TSS_SIZE: usize = core::mem::size_of::<Tss>();

impl Tss {
    pub const fn new() -> Tss {
        Tss {
            reserved0: 0,
            rsp: [0; 3],
            reserved1: 0,
            ist: [0; 7],
            reserved2: 0,
            reserved3: 0,
            iomap_base: TSS_SIZE as u16,
        }
    }
}

impl Tss {
    /// `rsp0` is the stack for an interrupt that arrives in ring 3;
    /// `ist[n]` is the stack of index `n + 1`.
    pub const fn with_stacks(rsp0: u64, ist: [u64; IST_STACKS]) -> Tss {
        let mut tss = Tss::new();
        tss.rsp = [rsp0, 0, 0];
        tss.ist = [ist[0], ist[1], ist[2], 0, 0, 0, 0];
        tss
    }
}

impl Default for Tss {
    fn default() -> Tss {
        Tss::new()
    }
}

/// The descriptor table for a task state segment at `tss_base`.
pub const fn gdt(tss_base: u64) -> [u64; GDT_SLOTS] {
    let limit = (TSS_SIZE - 1) as u64;
    // Type 0x9: an available 64-bit task state segment, present, ring 0.
    let tss_low = (limit & 0xFFFF)
        | (tss_base & 0x00FF_FFFF) << 16
        | 0x89 << 40
        | (limit >> 16 & 0xF) << 48
        | (tss_base >> 24 & 0xFF) << 56;
    [
        0,
        0x00AF_9A00_0000_FFFF, // ring 0 code, 64-bit
        0x00CF_9200_0000_FFFF, // ring 0 data
        0x00CF_F200_0000_FFFF, // ring 3 data
        0x00AF_FA00_0000_FFFF, // ring 3 code, 64-bit
        tss_low,
        tss_base >> 32,
    ]
}

/// The operand of `lgdt` and `lidt`.
#[repr(C, packed)]
pub struct TablePointer {
    pub limit: u16,
    pub base: u64,
}

/// One entry of the interrupt descriptor table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Gate {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl Gate {
    pub const EMPTY: Gate = Gate {
        offset_low: 0,
        selector: 0,
        ist: 0,
        type_attr: 0,
        offset_mid: 0,
        offset_high: 0,
        reserved: 0,
    };

    /// An interrupt gate: the processor clears the interrupt flag on entry.
    /// `ist` is 0 for the current kernel stack or 1-7 for a dedicated one.
    /// `ring` is the least privileged ring that may reach the gate with an
    /// `int` instruction; hardware interrupts and exceptions ignore it.
    pub const fn interrupt(handler: u64, ist: u8, ring: u8) -> Gate {
        Gate {
            offset_low: handler as u16,
            selector: KERNEL_CODE,
            ist: ist & 7,
            type_attr: 0x8E | (ring & 3) << 5,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }
}

/// Whether the processor pushes an error code for this exception.
pub const fn has_error_code(vector: u8) -> bool {
    matches!(vector, 8 | 10..=14 | 17 | 21 | 29 | 30)
}

/// The stack index of a vector's gate (0: no switch).
pub const fn ist_of(vector: u8) -> u8 {
    match vector {
        VECTOR_DOUBLE_FAULT => IST_DOUBLE_FAULT,
        VECTOR_NMI => IST_NMI,
        VECTOR_MACHINE_CHECK => IST_MACHINE_CHECK,
        _ => 0,
    }
}

/// What the entry code leaves on the kernel stack: the registers it pushed,
/// the vector and error code, then the frame the processor pushed. Lowest
/// address first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

pub const TRAP_FRAME_SIZE: usize = core::mem::size_of::<TrapFrame>();

/// `RFLAGS` of a program that starts: interrupts on, nothing else.
pub const USER_RFLAGS: u64 = 0x202;

impl TrapFrame {
    /// Whether the interrupted code ran in ring 3.
    pub const fn from_user(&self) -> bool {
        self.cs & 3 == 3
    }

    /// The frame that starts a program: every general register zero.
    pub const fn user_entry(rip: u64, rsp: u64) -> TrapFrame {
        TrapFrame {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rbp: 0,
            rdi: 0,
            rsi: 0,
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            vector: 0,
            error: 0,
            rip,
            cs: USER_CODE as u64,
            rflags: USER_RFLAGS,
            rsp,
            ss: USER_DATA as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn the_descriptor_table_has_seven_slots() {
        let table = gdt(0xFFFF_FFFF_8012_3450);
        assert_eq!(table.len(), 7);
        assert_eq!(table[0], 0);
        // Long-mode code segments: L set, D clear; ring in the DPL field.
        for (slot, ring) in [(1, 0), (4, 3)] {
            assert_eq!(table[slot] >> 53 & 1, 1, "L");
            assert_eq!(table[slot] >> 54 & 1, 0, "D");
            assert_eq!(table[slot] >> 43 & 1, 1, "code");
            assert_eq!(table[slot] >> 45 & 3, ring);
        }
        for (slot, ring) in [(2, 0), (3, 3)] {
            assert_eq!(table[slot] >> 43 & 1, 0, "data");
            assert_eq!(table[slot] >> 41 & 1, 1, "writable");
            assert_eq!(table[slot] >> 45 & 3, ring);
        }
        // Every segment is present.
        for slot in 1..=5 {
            assert_eq!(table[slot] >> 47 & 1, 1);
        }
    }

    #[test]
    fn the_task_state_descriptor_spans_slots_five_and_six() {
        let base = 0xFFFF_FFFF_8012_3450u64;
        let table = gdt(base);
        let low = table[5];
        assert_eq!(low & 0xFFFF, 0x67, "limit: 104 bytes");
        assert_eq!(low >> 40 & 0xFF, 0x89, "available 64-bit TSS, present, ring 0");
        let decoded = (low >> 16 & 0xFF_FFFF) | (low >> 56 & 0xFF) << 24 | table[6] << 32;
        assert_eq!(decoded, base);
        assert_eq!(table[6] >> 32, 0, "the upper half of slot six is reserved");
        assert_eq!(TSS_SELECTOR as usize, 5 * 8);
    }

    #[test]
    fn selectors_name_the_slots_and_rings() {
        assert_eq!((KERNEL_CODE, KERNEL_DATA), (0x08, 0x10));
        assert_eq!((USER_DATA, USER_CODE), (0x1B, 0x23));
    }

    #[test]
    fn the_task_state_segment_is_104_bytes_without_an_io_map() {
        assert_eq!(size_of::<Tss>(), 104);
        assert_eq!(offset_of!(Tss, rsp), 4);
        assert_eq!(offset_of!(Tss, ist), 0x24);
        assert_eq!(offset_of!(Tss, iomap_base), 0x66);
        let tss = Tss::new();
        assert_eq!({ tss.iomap_base }, 0x68);
        let tss = Tss::with_stacks(0x1000, [0x2000, 0x3000, 0x4000]);
        assert_eq!({ tss.rsp }, [0x1000, 0, 0]);
        assert_eq!({ tss.ist }, [0x2000, 0x3000, 0x4000, 0, 0, 0, 0]);
        assert_eq!({ tss.iomap_base }, 0x68);
    }

    #[test]
    fn a_gate_holds_the_handler_in_three_pieces() {
        assert_eq!(size_of::<Gate>(), 16);
        let gate = Gate::interrupt(0xFFFF_FFFF_8765_4321, 2, 0);
        assert_eq!(gate.offset_low, 0x4321);
        assert_eq!(gate.offset_mid, 0x8765);
        assert_eq!(gate.offset_high, 0xFFFF_FFFF);
        assert_eq!(gate.selector, 0x08);
        assert_eq!(gate.ist, 2);
        assert_eq!(gate.type_attr, 0x8E);
        // Only the system-call gate is open to ring 3.
        assert_eq!(Gate::interrupt(0, 0, 3).type_attr, 0xEE);
    }

    #[test]
    fn error_codes_and_stacks_by_vector() {
        let with: Vec<u8> = (0..=255).filter(|&v| has_error_code(v)).collect();
        assert_eq!(with, [8, 10, 11, 12, 13, 14, 17, 21, 29, 30]);
        assert_eq!((ist_of(8), ist_of(2), ist_of(18)), (1, 2, 3));
        assert_eq!((0..=255).filter(|&v| ist_of(v) != 0).count(), 3);
    }

    #[test]
    fn the_trap_frame_matches_the_entry_code() {
        assert_eq!(size_of::<TrapFrame>(), 176);
        assert_eq!(offset_of!(TrapFrame, rax), 14 * 8);
        assert_eq!(offset_of!(TrapFrame, vector), 15 * 8);
        assert_eq!(offset_of!(TrapFrame, rip), 17 * 8);
        assert_eq!(offset_of!(TrapFrame, ss), 21 * 8);
        let frame = TrapFrame::user_entry(0x40_00D8, 0x7FFF_FFFF_F000);
        assert!(frame.from_user());
        assert_eq!((frame.cs, frame.ss, frame.rflags), (0x23, 0x1B, 0x202));
        assert!(!TrapFrame { cs: 0x08, ..frame }.from_user());
    }
}
