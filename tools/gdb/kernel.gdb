# Attach to a VM started with:  cargo xtask run --gdb
# Use from WSL:  gdb -x tools/gdb/kernel.gdb
set pagination off
set disassembly-flavor intel
file ~/.cache/stillkey/target/x86_64-unknown-none/debug/stillkey-kernel
target remote localhost:1234
# A hardware breakpoint: the kernel's addresses are not mapped until the
# loader has switched page tables, so a software breakpoint cannot be written.
hbreak _start
continue
