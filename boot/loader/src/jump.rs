//! Step 12: the jump (docs/02 section 9).

use core::arch::asm;

/// Switches to the loader's page tables and the boot stack and enters the
/// kernel. `RDI` carries the address of the handoff block in the direct map
/// and `RSI` the number of bytes mapped for it.
///
/// # Safety
/// Boot services are exited and interrupts are off. `root` is the top-level
/// table built by `paging::build`, which identity-maps this code, maps
/// `stack_top - BOOT_STACK_SIZE .. stack_top` writable and maps `entry`
/// executable. `EFER.NXE` is set and `CR4.PGE` is clear. The stack is zeroed.
pub unsafe fn enter_kernel(
    root: u64,
    stack_top: u64,
    entry: u64,
    block_virt: u64,
    block_len: u64,
) -> ! {
    // The operands are in fixed registers so that nothing below reads memory:
    // after `mov cr3` the stack this function was called on is not mapped.
    // SAFETY: the caller's contract, above.
    unsafe {
        asm!(
            "mov cr3, rax", // still executing from the identity-mapped loader image
            "mov rsp, rcx", // boot stack top - 8; a zero "return address" sits at [rsp]
            "xor ebp, ebp",
            "jmp rdx",
            in("rax") root,
            in("rcx") stack_top - 8,
            in("rdx") entry,
            in("rdi") block_virt,
            in("rsi") block_len,
            options(noreturn),
        );
    }
}
