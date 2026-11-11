# Milestone 2: independent review and fix pass (2026-11-11)

Three reviewers, each started with a fresh context, the branch and the documents, and none of the author's notes: (1) memory and the CPU tables; (2) processes, system calls and the scheduler; (3) the tests, the harness and the documents against the code. Verdict of each: with fixes. One fix pass followed, in this order. Commit IDs are those of the branch `milestone-2`.

Fixed: `mem_free` stops the kernel for an address near the top of the address space (Critical, found by two reviewers; `kernel/src/proc.rs`, the end of the range was added without a check, and the kernel is built with overflow checks) — new case `freewrap` of the probe `edge`, test `m2-prober` RED→GREEN: RED `SK:KERNEL:PANIC at kernel/src/proc.rs:506: attempt to add with overflow`, GREEN after `checked_add` (`6c87e5e`).

Fixed: a long line begins the next screen row in its first column, so a program can make a row begin with text of its choice (`lib/sk-kcore/src/console.rs`) — host tests `a_continuation_row_leaves_its_first_two_cells_empty`, `a_continuation_row_holds_two_bytes_fewer` and two changed tests RED→GREEN, `sk-kcore` 93/93; `m2-screen` passes, its expected picture is drawn with the same code (`c1b8dd0`).

Fixed: the probe `handles` could not fail (values 0–4095 only, all below the first handle value, and no process of the scenario held a handle) — the probe tries 0–8191 and 4096 others, and `ticker` holds the `Diag` handle in `m2-prober`; test `m2-prober` RED→GREEN: RED with a seeded defect (`diag_read` looking in the table of process 1) `handles calls=24576 bad_handle=24575 other=1`, GREEN without it (`bd0265b`).

Fixed: nothing noticed a missing `clac` on interrupt entry — with `CR4.SMAP` set the dispatcher stops the kernel if `RFLAGS.AC` is set; test `m2-prober` RED→GREEN: RED with `clac` replaced by a no-op, `SK:KERNEL:PANIC at kernel/src/tables.rs:185: RFLAGS.AC is set in the kernel` at the `ac` case of `badptr`; GREEN with `clac` (`3050d1f`).

Fixed: no check measured time — the follower notes when it first saw each line, and a scenario with a `ticker` fails unless its `tick` lines came 600 to 1600 ms apart on average; host tests `ticks_a_second_apart_keep_the_pace`, `a_timer_at_half_or_twice_the_rate_is_seen`, `one_late_look_does_not_change_the_pace`, `fewer_than_two_timed_ticks_are_no_measurement`, `a_run_without_a_ticker_has_no_pace_to_keep`, `each_line_keeps_the_time_it_was_first_seen` RED→GREEN, `xtask` 101/101; in the VM RED with the 8254 at 200 Hz (`PIT_DIVISOR` 5966): `the ticker's lines came 501 ms apart on average, expected 600 to 1600`; GREEN unchanged (`168153b`).

Fixed: the harness's own line checks had no host test (`sk_lines`, `check_expect`, `frames_add_up`, the fault-code check of `m2-kwrite` and `m2-kexec`) — `check_expect` and the fault-code check are now the pure functions `expect_text` and `refused_access`; 10 host tests in `xtask/src/tests.rs` (`line_checks`), RED (the two functions did not exist) →GREEN, `xtask` 111/111.

Fixed: the acceptance record did not name the commit, and a row that ran fewer tests would pass — `tools/acceptance.sh` prints `COMMIT <id> changed_files=<n>` first and fails if a row's count of results differs from the first row's (`7b43768`). No test; the script is checked by its next run.

Decision: the two `asm!` options that were untrue are removed without a failing test (`preserves_flags` on every move to or from a control register, kernel and loader; `nomem` on `sti; hlt; cli`) — an option is a promise to the compiler, and no test can show a miscompilation that did not happen — if wrong: nothing; the compiler only assumes less (`04689e0`).

Decision: `ticker` gets the `Diag` handle in the scenario `m2-prober`, rather than adding the `inspector` — `ticker` never ends, so the value `0x1000` is held by another process for the whole of the guessing; the `inspector` could end first — if wrong: an `App` program holds a system right in one test scenario, which goes away with the start table in Milestone 3 (D-K-20).

Decision: the pace of the ticks is judged on the average and widely (600 to 1600 ms) — the harness looks every 50 ms and the host can hold it up; the limits still catch a timer at half or twice the rate — if wrong: a timer that is 40 % off passes.

Decision: the check of `RFLAGS.AC` runs on every interrupt and system call, in both profiles — it is the only thing that makes a missing `clac` visible — costs a few instructions per entry.

Decision: a continuation row is marked by two empty cells, not by a glyph — a glyph could itself be printed by a program at the end of a row — if wrong: one constant, `CONTINUATION_INDENT`.

Decision: the TSC clock of docs/03 section 5.3 and docs/10 section 7 step 17 is not implemented; `clock()` counts ticks of 10 ms in every case and the boot line always says `clock=ticks` — the VM reports no invariant TSC, so the path would run in no test — the documents now say so (docs/10 sections 7, 8.1, 11.5, 13, 14; docs/03 section 5.3) — if wrong: `clock()` has a resolution of 10 ms, and a tick lost with interrupts off makes it late. Owed before a hardware boot.

### What the reviewers set aside ("declined to judge")

Decision: D-K-13, no hashing of a program image by the kernel at start — the loader prints the digest of `BOOTFS.IMG` and every Milestone 2 test compares it with the host's; whoever can change the image can change the kernel beside it — stands for the VM milestones — if wrong: a changed program starts and only the harness notices.

Decision: D-K-17, the table slot and the kernel stack of an ended process are kept, so one boot starts at most 64 processes — the largest scenario starts 19 — stands until `init` starts programs (Milestone 3) — if wrong: the 65th start is refused with `reason=table`; still no test starts 65.

Decision: `cpu` in the process table is sampled at the tick, and `sleep` ends at a tick — a program that gives up the processor just before every tick shows less time than it used; a sleep is exact to 10 ms — stands: the figures are for a person reading the Workbench, not for accounting — if wrong: a program can hide its processor use from the table.

Decision: the system keeps no memory in reserve — the limits of the programs of every scenario sum to less than the VM's memory — stands for Milestone 2 — if wrong: with more or larger programs a kernel allocation fails and the kernel stops; becomes a finding when `init` can start programs (Milestone 3, D-K-22 charges kernel memory).

Decision: `FLAG_CRITICAL` is read by nothing until Milestone 3 — docs/11 gives it its meaning (`init`) — stands — if wrong: nothing in Milestone 2.

Decision: no UMIP, no address randomisation, `rdtsc` open to programs — a program can read where the descriptor tables are and has a fine timer; addresses are fixed anyway — stands for the VM milestones; to decide before a hardware boot (Milestone 5) — if wrong: it helps whoever has found another defect.

Decision: the serial port is written without waiting for the transmitter — QEMU takes every byte; a real 16550 would lose some — stands while every run is in a VM; must change before a hardware boot — if wrong: a garbled log on hardware.

Decision: the frames that hold the kernel's code are also writable through the direct map — "nothing is writable and executable" holds for each mapping, not for each frame — stands for Milestone 2 as a known limit, now written in docs/10 section 11.5 — if wrong: a kernel defect that writes through the direct map can change kernel code; to close before a hardware boot.

Decision: NMI, machine check and double fault stop the kernel even if they arrive while a program runs — a program cannot cause any of them by itself; each means the machine or the kernel is in trouble — stands — if wrong: a hardware event ends the session instead of one program.

Decision: no defence against timing side channels, no separate page tables for the kernel — out of scope by docs/04; VM only — stands; before hardware, with the processor's own mitigations — if wrong: a program can learn kernel memory on processors with those defects.

Decision: the IST stacks have no guard pages — they lie side by side in `.bss`, and only the short fatal paths run on them — stands — if wrong: a deep fatal path overwrites the next stack and the last lines of a dying kernel are wrong.

Decision: a program can fragment its own heap — it harms only itself and stays inside its limit — stands.

Decision: the Milestone 1 tests and docs 02, 05, 07 and 08 were outside this review — not reviewed again.

Decision: whether the screen console is readable is for a person to judge; not judged.

### What the reviewers could not verify, and is still not checked

Not checked: whether a triple fault under `-no-reboot` ends QEMU with status 0, which the harness could take for its own `quit` (the checks on the lines would still have to pass). Whether firmware leaves other bits of `CR4` set that the kernel keeps (`FSGSBASE` would let two programs pass data through a segment base). Whether one scroll of the screen takes longer than a tick under software emulation. The state of `LDTR`. A `#DB` in ring 0 after `popfq` or `mov ss` followed by `int 0x80` (a program cannot set a debug register, so no such trap is expected).

### Minor findings, deferred

Deferred: `fatal` prints the `REGS` line also when the frame is from ring 3 (NMI, machine check, double fault while a program runs), so a program's registers could reach the serial log (`kernel/src/tables.rs`).
Deferred: DS, ES, FS and GS are not set again when another process runs; two programs that work together can pass a few bits through a selector.
Deferred: `image_offset` of a crash record is `rip − 0x40_0000` also when the fault is in the heap or the stack.
Deferred: the `debug_write` budget counts bytes, not lines: 4,096 one-byte lines a second scroll the screen 4,096 times.
Deferred: `spawn` does not look at the entry's kind, and `mem_limit_mib` has no upper bound.
Deferred: handle generations wrap at 2^19; the documents say 20 bits.
Deferred: no host test calls `sk_elf::user_policy()` itself.
Deferred: host tests that would pass with their property broken: the empty-table test of `handles`, several cases of `boot/bootinfo/tests`, the trap-frame test in `x86.rs`, one each in `heap.rs`, `paging.rs` and `frames.rs`; missing cases in `sk-bootfs`; no `usercopy` test over a large page.
Deferred: `Bitmap::free` accepts a frame that was never released, and `virt.rs` has no floor at 1 MiB for it.
Deferred: growing the kernel heap panics if no frame is left for a page table.
Deferred: in `spawn`, a failure after a successful `load` (the handle insert) leaves the address space without `destroy`.
Deferred: items of docs/10 section 8.5 that were not built as written, without a ruling at the time: `Bitmap::alloc_run` and `total`, `usercopy::plan`, `PageFault::decode`, `tss_descriptor`, `idt_gate`; and the host tests are inside the modules, not in `tests/<module>.rs`.
Deferred: `wtext.rs` and `twin.rs` use `write_volatile` and `read_volatile` where the specification says `asm!`.
Deferred: docs/06 wording: criterion 7 and "four scenarios".
Deferred: `m2-screen` checks neither the loader's `BOOTFS_SHA256` line nor an expect file.
Deferred: `memcap` accepts any figure from 1 to 15 MiB.
Deferred: no probe reads a fresh `mem_alloc` page before writing it (that it is zero).
Deferred: docs/10: section 13 says "criteria 1–7"; section 2 says `TIMER_OK` and `SCHED_OK` are named in docs/06; section 6 says scenario runs have no `qemu_exit=on` although every expect file has it; section 3 names files that do not exist (`lib/sk-bootfs/tests/format.rs`, `lib/sk-kcore/tests/*.rs`, `user/inspector/`).
Deferred: `libstill` turns a return code it does not know into `InvalidArgs`.

Fixed in passing, each one line in a file that was open: the count of host tests of `check_prober` in docs/10 section 11.4 (19, not 20); "D-K-11 … D-K-18 and D-W-7" in the status line of docs/10 (D-K-19 and D-W-8 exist); the `m2-hello` row of docs/10 section 10 (the kernel compares the frames given back with the frames charged, not the allocator's free count); the status cell of docs/06, which said both "all three checkpoints implemented" and "the first … is the only part implemented".

### The mutation record, corrected and extended

The record of the first mutation pass named `follow.rs` among the files covered. That was wrong: `follow.rs` was a target of `tools/mutate.py`, but the tool found no line in it that it can change (it skips lines with a quote character), made no mutant there, and did not say so. The tool now prints `NONE` for such a target, runs each crate's tests unchanged first (`BASELINE`), and can take a range inside a file.

Second sampled pass, 2026-11-11, on what this fix pass added or changed (`docs/records/milestone-2-mutation-review.txt`): `xtask/src/tests.rs` (the line filter, the expect check, the fault-code check, the frame sum) 7 of 7 killed; `xtask/src/m2.rs` 14 of 16 killed; `lib/sk-kcore/src/console.rs` 6 of 6 killed, one by the time limit (a grid of two columns with an indent of two never finishes a line); `xtask/src/follow.rs` 0 of 1. The three survivors are equivalent: `lines > seen.len()` in `note_arrivals` (a resize to the same length changes nothing), and the two in `m2.rs` that the first pass already judged (`at > place`, `t.first > start`).

Fixed: `m2-kwrite` and `m2-kexec` had no recorded failing run — seeded defects, one run each: with `TEXT` mapped writable `m2-kwrite` fails, with `DATA` mapped executable `m2-kexec` fails; in both the kernel's own audit of its tables stops the boot (`SK:KERNEL:PANIC at kernel/src/virt.rs:290: kernel page … wanted … the tables give …`) and the test says `no line starting with SK:KERNEL:PAGING_AUDIT pages=`. So the audit notices before the processor is asked; the fault-code check of the two tests is shown by host tests and mutants only (`refused_access`, 2 of 2 mutants killed), not by a VM run in which the processor allowed the access.

Still owed: a test that a 65th start is refused with `reason=table`; the TSC clock.
