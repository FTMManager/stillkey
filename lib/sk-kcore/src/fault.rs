//! Who pays for an exception (docs/10 section 8.2).

/// Who an exception is charged to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charge {
    /// The program that was running ends; the kernel carries on.
    Process,
    /// The kernel stops.
    Kernel,
}

/// A program pays only for what a program can cause: the exception came
/// from ring 3, it is one of the faults an instruction or an address of the
/// program's own raises, and the processor does not say that an event
/// outside the program was involved (bit 0 of the error code of vectors 11,
/// 12 and 13). A non-maskable interrupt, a double fault or a machine check
/// is the kernel's, whatever was running.
pub const fn classify(vector: u8, cs: u64, error: u64) -> Charge {
    if cs & 3 != 3 {
        return Charge::Kernel;
    }
    match vector {
        0 | 1 | 3 | 4 | 5 | 6 | 7 | 14 | 16 | 17 | 19 => Charge::Process,
        11..=13 if error & 1 == 0 => Charge::Process,
        _ => Charge::Kernel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KERNEL_CS: u64 = 0x08;
    const USER_CS: u64 = 0x23;

    #[test]
    fn nothing_from_ring_0_is_charged_to_a_process() {
        for vector in 0..=255 {
            for error in [0, 1, 2, 0x15] {
                assert_eq!(classify(vector, KERNEL_CS, error), Charge::Kernel, "vector {vector}");
            }
        }
    }

    #[test]
    fn a_program_pays_for_its_own_faults_only() {
        let own = [0, 1, 3, 4, 5, 6, 7, 11, 12, 13, 14, 16, 17, 19];
        for vector in 0..32 {
            let want = if own.contains(&vector) { Charge::Process } else { Charge::Kernel };
            assert_eq!(classify(vector, USER_CS, 0), want, "vector {vector}");
        }
    }

    #[test]
    fn an_external_event_is_not_the_programs_fault() {
        for vector in [11, 12, 13] {
            assert_eq!(classify(vector, USER_CS, 0x18), Charge::Process);
            assert_eq!(classify(vector, USER_CS, 0x19), Charge::Kernel, "vector {vector} with EXT");
        }
        // Bit 0 of a page fault's error code means "present", not "external".
        assert_eq!(classify(14, USER_CS, 0x7), Charge::Process);
    }

    #[test]
    fn nmi_double_fault_and_machine_check_stop_the_kernel() {
        for vector in [2, 8, 18] {
            assert_eq!(classify(vector, USER_CS, 0), Charge::Kernel);
        }
    }

    #[test]
    fn only_ring_3_counts_as_a_program() {
        for cs in [0x08, 0x09, 0x0A] {
            assert_eq!(classify(14, cs, 0x4), Charge::Kernel);
        }
        assert_eq!(classify(14, 0x0B, 0x4), Charge::Process);
    }
}
