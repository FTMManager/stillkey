use std::fmt::Write as _;
use std::path::Path;
use std::{env, fs};

use sk_kcore::x86;

/// One entry stub per interrupt vector, as assembly text for `tables.rs`:
/// a zero where the processor pushes no error code, the vector, and a jump
/// to the common part. Then a table of the stubs' addresses.
fn isr_stubs() -> String {
    let mut text = String::from("    .section .text.isr, \"ax\"\n");
    for vector in 0..x86::IDT_GATES {
        writeln!(text, "isr_stub_{vector}:").unwrap();
        if !x86::has_error_code(vector as u8) {
            text.push_str("    push 0\n");
        }
        writeln!(text, "    push {vector}\n    jmp isr_common").unwrap();
    }
    text.push_str("    .section .rodata.isr, \"a\"\n    .balign 8\n    .global isr_stub_table\nisr_stub_table:\n");
    for vector in 0..x86::IDT_GATES {
        writeln!(text, "    .quad isr_stub_{vector}").unwrap();
    }
    text.push_str("    .text\n");
    text
}

fn main() {
    let dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    println!("cargo::rustc-link-arg-bins=-T{dir}/linker.ld");
    println!("cargo::rerun-if-changed=linker.ld");
    println!("cargo::rerun-if-changed=build.rs");

    let out = env::var("OUT_DIR").expect("OUT_DIR");
    fs::write(Path::new(&out).join("isr_stubs.s"), isr_stubs()).expect("write isr_stubs.s");
}
