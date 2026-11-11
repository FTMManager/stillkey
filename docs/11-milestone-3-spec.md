# 11 · Milestone 3 specification: grant and revoke

Status: **specified; not started.** Written on 2026-11-11 from [03](03-kernel-and-interfaces.md), [04](04-security-and-permissions.md) and the Milestone 3 section of [06](06-milestones.md). It has had the author's own check against those documents (section 13) and no independent review; that review is step 0 of section 11. Choices this document adds are marked **D-K-20 … D-K-28**, **D-S-6 … D-S-8**, **D-W-9** and **D-W-10** and are listed in [01 §3](01-overview-and-architecture.md#3-design-decisions). Section 16 lists the five most open to question.

> **For whoever implements this:** work the steps of section 11 in order, test first. Each step names its files, the interfaces it consumes and produces, its tests and the command that shows it done. Never run two `cargo xtask test` commands at once. Everything stays in the VM: no physical disk, firmware setting, Secure Boot, BitLocker or Windows boot configuration is touched.

## 1. Outcome

One command passes: `cargo xtask test m3`. It boots the Milestone 3 image and shows that

- the kernel starts one program, `init`, and every other program starts with one handle and no access to any file (`m3`);
- a program gets to a file only through a grant that the broker `permd` creates after a decision, and a grant can be neither forged, widened, copied nor passed on (`m3`, `m3-dup`);
- revoking a grant stops every handle to it at once, and a broker that dies takes its grants with it and comes back usable (`m3`, `m3-dup`, `m3-brokerkill`);
- a program that floods channels, handles or requests runs into its own limits and harms nobody else (`m3-prober`);
- every request, grant, refusal, revocation and release is in an audit log that programs cannot write to or push entries out of (`m3`, `m3-prober`);
- an operator can decide and revoke from a serial shell (`m3-shell`, and by a person with `cargo xtask run`).

`cargo xtask test all` runs `m1`, `m2` and `m3`. The Milestone 2 scenarios keep passing, now started by `init` (D-K-20).

**Goal in one sentence:** file access exists only as a grant that a person, or a named test policy, gave and can take back.

**Architecture in three sentences.** The rules that need no privileged instruction stay pure and host-tested (D-K-12): the object store, channel queues, grant states, audit rings and wait bookkeeping go into `sk-kcore`, and the broker's request handling and test policies go into a new crate `sk-broker`. The kernel gains the modules `object`, `ipc`, `grant`, `audit` and `syscall`, starts only `init`, and gives it the root directory, the diagnostics object and the console. `init`, `permd` and `shell` are ordinary ring-3 programs of class System that talk over channels; the kernel does not interpret their messages.

## 2. Decisions this specification adds

| ID | Decision | Alternative | Cost if changed later |
|---|---|---|---|
| D-K-20 | **The Milestone 2 scenarios move to `init`.** The kernel's start table is removed, as [03 §7.1](03-kernel-and-interfaces.md#71-launch-topology) says. Under `test=m2`, `m2-hello`, `m2-preempt` and `m2-prober` `init` starts the same programs in the same order and starts neither `permd` nor `shell`. What changes in those tests: every PID is one higher (`init` is PID 1), `SPAWN` lines say `handles=1`, the `inspector` gets its `Diag` handle in its launch message, and the probe `handles` finds `WRONG_TYPE` on its bootstrap handle and `BAD_HANDLE` on every other value. The isolation tests then run on the kernel that ships | Keep a frozen Milestone 2 mode in the kernel behind a boot option | The expectations of four tests |
| D-K-21 | **Kernel objects live in typed slabs** (`sk_kcore::objects`), named by index and generation, with explicit counts of the handles and queued messages that refer to each. A slab grows with `try_reserve`; when that fails the system call returns `NO_MEMORY`. No `Arc`, `Rc` or `Box::new` on a system-call path | `Arc` with the unstable fallible constructors | The object store |
| D-K-22 | **What kernel memory costs a process**, counted in bytes against its memory limit: 4096 for each page of its handle table (256 slots); 512 for a channel, to its creator; 64 + the length + 24 per handle for a message, to the sender, from `channel_write` until the message is read or discarded; 256 for a grant, to its creator. `ProcessRecord` shows the sum as `kernel_kib` | Other figures | Constants in `stillkey-abi` |
| D-K-23 | **Two kinds of handle cannot be sent in a message:** an end of the channel being written to, and a channel end that has messages waiting. Both are refused with `INVALID_ARGS`. A channel end can then never lie in a queue that only it leads to, so closing handles always frees everything (section 8.3) | Find and free unreachable channel ends when a process ends | The check in `ipc`, and a collector |
| D-K-24 | **`channel_read` into a buffer that is too small** returns `INVALID_ARGS` and puts the sizes it needs in `RDX` (bytes in the low 32 bits, handles in the high 32). The message stays queued | A separate call that asks for the size | One branch |
| D-K-25 | **A process's bootstrap handle is always `0x1000`** (`FIRST_HANDLE`). D-K-18 is narrowed to this one handle; everything else arrives in messages | Pass the value in a register at entry | One constant |
| D-K-26 | **A blocked call starts again.** A thread that blocks records at most 16 objects with the signals it waits for, and a deadline. When one of those signals changes, or the deadline passes, the thread becomes Ready and the system call runs again from its first check ([03 §5.3](03-kernel-and-interfaces.md#53-scheduling)). A program never sees a wake-up that had no cause | Wait queues per object | `sched` and the two wait calls |
| D-K-27 | **Test events.** A program may send *test-event* {code} on its bootstrap channel. Under a `test=` option `init` acts on it: it ends `permd` (`m3-brokerkill`), starts the next program (`m3-prober`), or passes it to `permd` as a cue for the test policy. With no `test=` option `init` ignores it, and no program is started with the self-test flag | Scripts driven by time | The scenarios' scripts |
| D-K-28 | **The kernel keeps a record of the last 256 grants.** A new grant replaces the oldest record that is not Active. With 256 Active grants `grant_create` returns `NO_MEMORY`. Grant ids count from 1 and are never reused | Keep every record | A constant |
| D-S-6 | **How refusals are limited** ([04 §8](04-security-and-permissions.md#8-audit-log) left two readings open). In each second, counted per process from its first refusal, the first 8 `denied` notes each make an entry and a line. A further note in that second is added to the newest `denied` entry of that process with the same reason, or else to its newest `denied` entry: the count rises, the time of the last is set, nothing is printed. A person's refusals, which come seconds apart, therefore each keep their own entry and path | One entry per process and reason, whatever the path | The `audit` module |
| D-S-7 | **`permd` takes at most 8 requests per second from one app for a decision.** More are refused at once with `ACCESS_DENIED` and `reason=busy`. Without this a test policy, which answers at once, would let an app write `asked` entries as fast as it can ask and push other entries out of the main ring; [04 §8](04-security-and-permissions.md#8-audit-log) assumed that decisions are slow | Make test policies slow | One counter in `sk-broker` |
| D-S-8 | **A new reason, `control-closed`.** When a process that is still running closes a `GrantControl` handle, the grant is revoked with `actor=kernel reason=control-closed`. `broker-exit` is used only when the process is ending | One reason for both | A constant |
| D-W-9 | **`m3-shell`: the operator shell is tested by the tool as well as by a person.** For this one test the VM's serial port is a socket that the harness writes to, with QEMU's own log of the port as `serial.log`. It types `grant`, `revoke`, `deny`, `grants`, `audit` and `ps` and checks the answers and the `AUDIT` lines. This also shows that IRQ 4 arrives, which [06](06-milestones.md) says no automated test can see. D-K-10 stands for every other test | Leave the shell to the person | One variant in `xtask/src/qemu.rs` |
| D-W-10 | **Added tests:** `m3-unit` (host tests of every crate), `m3-initexit` (when `init` ends, the kernel stops with "system service lost") and `m3-shell`. `abi_version()` returns 3 | Only the four scenarios of 06 | – |

These go beyond [06](06-milestones.md) and are named there now: the three added tests; the kernel lines `SK:KERNEL:INIT` and `SK:KERNEL:KILLED`; the lines `init`, `permd`, `shell` and `notes` print (section 7); the third phase of `chan-flood` (section 9).

## 3. Files this milestone creates or changes

| Path | What |
|---|---|
| `lib/abi/src/lib.rs` | Calls 10–22, 26, 27; `ABI_VERSION` 3; signals; actor, action and reason codes; the limits and charges of section 4 |
| `lib/abi/src/msg.rs` (new) | Wire structures of section 8.11, with `encode` and `decode` |
| `lib/abi/src/path.rs` (new) | `check(path) -> Result<(), BadPath>` (section 8.5) |
| `lib/abi/src/diag.rs` | `GrantRecord`, `AuditRecord`, `HandleCounts`, `FileStat`, `AuditNote`; `kernel_kib` in `ProcessRecord` |
| `lib/sk-kcore/src/{objects,ipc,grants,audit,waits,rxring}.rs` (new) | Section 8.12 |
| `lib/sk-kcore/src/{handles,account}.rs` | Tables that grow by pages to the class limit; the byte counter for kernel memory |
| `lib/sk-broker/` (new crate, `no_std`, no `unsafe`) | `clients`, `policy`, `admin` (section 8.12) |
| `lib/sk-serial/src/lib.rs` | `enable_receive()`, `received() -> Option<u8>` |
| `kernel/src/{object,ipc,fsobj,grant,audit,syscall,console_in}.rs` (new) | The object layer over `sk-kcore`; calls 10–22, 26, 27; system-call dispatch moves out of `proc.rs` into `syscall.rs` |
| `kernel/src/{proc,sched,arch,main}.rs` | Start from a `File`; kill; the `critical` flag; the Waiting state; vector `0x24`; the start of `init` |
| `kernel/src/m2.rs` → `kernel/src/selftest.rs` | Only `test=kwrite_text` and `test=kexec_data` remain |
| `user/libstill/src/` | Wrappers for the new calls; `Handle` (closes when dropped); message helpers; output that keeps to the budget (section 9) |
| `user/init/`, `user/permd/`, `user/shell/`, `user/samples/notes/` (new) | Section 9 |
| `user/samples/prober/src/bin/{copyout,chan-flood,ask-flood}.rs` (new) | Section 9 |
| `user/samples/inspector/`, `user/samples/prober/src/bin/handles.rs` | D-K-20 |
| `bootfs/MANIFEST`, `bootfs/docs/welcome.txt`, `bootfs/docs/secret.txt` | Section 5 |
| `xtask/src/m3.rs` (new), `xtask/src/{m2,tests,qemu,bootfs,main}.rs`, `tests/m3*.expect`, `tests/m2*.expect` | Sections 5, 6 and 10 |
| `docs/01`, `03`, `04`, `06`, `README.md` | The decision rows; the wording D-K-23, D-K-24, D-S-6, D-S-7 and D-S-8 change; status lines |

## 4. Pinned configuration

The VM is that of Milestone 2: `q35`, `-cpu max`, one CPU, 256 MiB, the same firmware. New constants, all in `stillkey-abi`:

| Name | Value |
|---|---|
| `ABI_VERSION` | 3 |
| Call numbers | 10 `handle_duplicate`, 11 `channel_create`, 12 `channel_write`, 13 `channel_read`, 14 `wait`, 15 `dir_open`, 16 `file_read`, 17 `file_stat`, 18 `grant_create`, 19 `grant_revoke`, 20 `process_spawn`, 21 `process_kill`, 22 `diag_note`, 26 `wait_many`, 27 `console_read`. 23, 24, 25 and 28 return `INVALID_ARGS` until Milestone 4 (D-K-15) |
| Signals | `READABLE` 1, `PEER_CLOSED` 2, `EXITED` 4 |
| Deadlines | Nanoseconds on the scale of `clock()`. 0 does not block; `u64::MAX` never times out |
| Message | At most 65,536 bytes and 16 handles. A queue holds at most 64 messages and 262,144 bytes |
| `file_read` | At most 65,536 bytes per call |
| `wait_many` | At most 16 items |
| Handles | 256 for class App, 4096 for class System (unchanged); 256 slots per table page |
| Charges (D-K-22) | Table page 4096; channel 512; message 64 + length + 24 per handle; grant 256 |
| Audit | Main ring 256 entries; denied ring 64; 8 new `denied` entries per process per second; path in a label cut to 128 bytes |
| Requests | 8 per app per second taken for a decision (D-S-7); one waiting per app |
| Grant records | 256 (D-K-28) |
| Console | Receive ring of 256 bytes; a shell line is at most 64 bytes |
| Actors | 1 `prompt`, 2 `shell`, 3 `workbench`, 4 `init`, 5 `permd`, 6 `kernel`, 16 `policy:m3`, 17 `policy:m3-dup`, 18 `policy:m3-prober`, 19 `policy:m3-brokerkill` |
| Actions | 1 `read` |
| Reasons | 0 none, 1 `broker-exit`, 2 `requester-exit`, 3 `busy`, 4 `bad-path`, 5 `no-file`, 6 `control-closed` |
| `permd` restarts | At most 5 per boot; at the sixth end of `permd`, `init` exits and the kernel stops (section 8.10) |
| Interrupts | 8259 master mask `0xEE`; COM1 is IRQ 4 at vector `0x24` |

The kernel heap window must hold at least 64 MiB, so that three apps can each queue messages up to their 16 MiB limit. Step D checks the Milestone 2 constant and raises it if it is smaller.

## 5. Build and the boot filesystem

The format of `BOOTFS.IMG` does not change (version 1, [10 §5.1](10-milestone-2-spec.md#51-format-of-bootfsimg-version-1)); a path is at most 64 bytes there. New rows of `bootfs/MANIFEST`:

| Path | Source | Name | Class | Limit | Flags |
|---|---|---|---|---|---|
| `bin/init` | `program:init` | `init` | system | 0 | `critical` |
| `bin/permd` | `program:permd` | `permd` | system | 0 | – |
| `bin/shell` | `program:shell` | `shell` | system | 0 | – |
| `bin/notes` | `program:notes` | `notes` | app | 0 | – |
| `bin/prober-copyout`, `bin/prober-chan-flood`, `bin/prober-ask-flood` | `program:prober-…` | `prober-…` | app | 0 | – |
| `docs/welcome.txt` | `file:bootfs/docs/welcome.txt` | – | – | – | – |
| `docs/secret.txt` | `file:bootfs/docs/secret.txt` | – | – | – | – |
| `docs/large.bin` | `gen:large` | – | – | – | – |

That makes 34 entries of the 64 the format allows. `welcome.txt` begins with the line `Welcome to Stillkey.`; `secret.txt` holds one line of text that no test may see in the log. `gen:large` is a new source that `xtask` makes itself, so that no 200 KiB file sits in the repository: 204,800 bytes, where the 32 bytes of block *i* (from 0) are the SHA-256 of the 18 bytes `stillkey large.bin` followed by *i* as eight little-endian bytes. A host test pins the digest of the whole file.

The path a person sees is `/` followed by the path in the image: `/docs/welcome.txt`.

The loader does not change.

## 6. VM launch and how a run ends

Scenario tests run as in Milestone 2 (D-W-7): the harness follows `serial.log`, ends QEMU through the monitor when every required line has appeared, and fails on a forbidden line, on status other than 0, or at the time limit (60 s under KVM, 240 s under TCG). `m3-initexit` ends by itself with status 37.

For `m3-shell` only, the serial port is `-chardev socket,id=ser0,path=<run directory>/serial.sock,server=on,wait=off,logfile=<run directory>/serial.log,logappend=off -serial chardev:ser0` (D-W-9, assumption A-23). The harness connects to the socket, follows `serial.log` as always, and writes the next line of its script when the line it waits for has appeared.

`cargo xtask run` connects the serial port to the terminal, as before. The shell does not echo single characters; it prints each finished line back (section 9).

## 7. Boot behaviour and serial output

Everything up to the point where the Milestone 2 kernel read its start table is unchanged ([10 §7](10-milestone-2-spec.md#7-boot-behaviour-and-serial-output)), and so are `stop=m1`, `test=panic`, `test=kwrite_text` and `test=kexec_data`. Then:

| Step | What the kernel does | Line |
|---|---|---|
| 1 | Sets up the object store, the audit rings and the grant records | – |
| 2 | Enables COM1 receive: interrupt enable bit 0, modem control `OUT2`, 8259 master mask `0xEE`, handler at vector `0x24` | `SK:KERNEL:CONSOLE irq=4 vector=0x24` |
| 3 | Creates `init`'s bootstrap channel and keeps its far end for the rest of the boot. Queues one message: the boot command line as bytes, with three handles in this order: the root `Dir` (`LOOKUP`, `READ`, `EXECUTE`, `GRANT`, `DUPLICATE`, `TRANSFER`), the `Diag` (`INSPECT`, `WRITE`, `DUPLICATE`, `TRANSFER`), the `Console` (`READ`, `TRANSFER`) | – |
| 4 | Starts `bin/init`. A missing or refused image is a kernel panic | `SK:KERNEL:SPAWN pid=1 name=init class=System handles=1`, then `SK:KERNEL:INIT pid=1` |
| 5 | Schedules. From here the kernel starts nothing by itself | – |

`init` then reads its message and prints, as the text of `SK:APP:OUT pid=1 name=init |` lines: `SK:INIT:SCENARIO name=<s> programs=<n>` (the name is `default` with no `test=` option) and, when everything in its table has been started, `SK:INIT:RUNNING`.

New kernel lines: `SK:KERNEL:KILLED pid=<n> name=<s>` before the `REAPED` line of a process ended by `process_kill`; `AUDIT …` lines in the format of [04 §8](04-security-and-permissions.md#8-audit-log). The `SCENARIO` and `M2_RUNNING` lines of the Milestone 2 kernel are gone; `init`'s two lines take their place in the Milestone 2 expectations.

Lines the services print (each is the text of an `SK:APP:OUT` line, accepted only from the System-class process of that name):

| Program | Lines |
|---|---|
| `permd` | `SK:PERMD:POLICY name=<s>` (only under a test policy); `SK:PERMD:REVOKE grant=<id> first=<status> second=<status>` when a policy revokes |
| `shell` | `SK:SHELL:READY`; `SK:SHELL:CMD text="<line>"`; `SK:SHELL:ASK id=<n> pid=<n> name=<s> path="<path>" action=read`; `SK:SHELL:GRANTS count=<n>` then one `SK:SHELL:GRANT id=<n> pid=<n> path="<path>"` each; `SK:SHELL:PS pid=<n> name=<s> class=<s> state=<s> handles=<n> dir=<n> chan=<n> grant=<n> kernel_kib=<n>`; `SK:SHELL:AUDIT <the entry in the form of an AUDIT line, with count=<n>>`; `SK:SHELL:ERROR text="<why>"` |
| `notes` | Section 9 |

Expected order of a `test=m3` boot, PIDs included: `init` 1, `permd` 2, `shell` 3, `ticker` 4, `notes` 5.

## 8. Interfaces

### 8.1 Objects and handles

A handle is a slot in the caller's table: an object reference and rights. Its value is the slot index in bits 0–11 and a generation in bits 12–31; a slot's generation starts at 1 and rises by one each time the slot is given out again (after `0xFFFFF` the slot is retired). The value 0 is never valid. Slots are given out lowest index first.

The checks on a handle run in this order, and the first failure is the result: the value is in the caller's table (`BAD_HANDLE`); the object has the type the call needs (`WRONG_TYPE`); the handle carries the right the call needs (`ACCESS_DENIED`); for a `Grant`, the grant is Active (`REVOKED`). Only then are the other arguments looked at.

| Object | Rights its handles can carry | Freed when |
|---|---|---|
| `Channel` end | `READ`, `WRITE`, `TRANSFER` | No handle and no queued message refers to the end; the channel when both ends are gone |
| `Dir` | `LOOKUP`, `READ`, `GRANT`, `EXECUTE`, `DUPLICATE`, `TRANSFER` | Never; it is a prefix of the read-only image |
| `File` | `READ`, `GRANT`, `EXECUTE`, `DUPLICATE`, `TRANSFER` | Never; it is an entry of the image |
| `Grant` | `READ`, `DUPLICATE`, `TRANSFER` | Its record stays (D-K-28) |
| `GrantControl` | `REVOKE` | With its last handle, which revokes the grant |
| `Process` | `READ`, `KILL` | Never (D-K-17) |
| `Diag` | `INSPECT`, `INSPECT_REGISTERS`, `WRITE`, `DUPLICATE`, `TRANSFER` | Never |
| `Console` | `READ`, `TRANSFER` | Never |

No handle to a `Grant` ever carries `GRANT` or `EXECUTE`, and nothing in this milestone gives out `INSPECT_REGISTERS`.

### 8.2 System calls 10–22, 26 and 27

The table of [03 §5.5](03-kernel-and-interfaces.md#55-system-calls) holds. What it leaves open:

| Call | Detail fixed here |
|---|---|
| `handle_duplicate(h, rights)` | Any object. Needs `DUPLICATE`; `rights` not a subset of the handle's: `INVALID_ARGS`. No free slot: `NO_MEMORY`. A duplicate of a `Grant` handle refers to the same grant |
| `channel_create()` | Both ends carry `READ`, `WRITE` and `TRANSFER`. First end in `RAX`, second in `RDX`. Two slots and 512 bytes, or `NO_MEMORY` and nothing is made |
| `channel_write(h, bytes, len, items, n)` | Needs `WRITE`. `len` > 65,536 or `n` > 16: `INVALID_ARGS`. An item is 8 bytes: handle `u32`, rights `u32`. Section 8.3 gives the order of the checks. Peer end gone: `PEER_CLOSED` |
| `channel_read(h, buf, cap, handles, hcap)` | Needs `READ`. Bytes read in `RAX`, handles received in `RDX`; the handle values are written to `handles` as `u32`. Nothing queued: blocks, or `PEER_CLOSED` when the peer end is gone. Too small: D-K-24. Section 8.3 |
| `wait(h, signals, deadline)` | Needs `READ`. Section 8.4. Returns the signals that are set, or `TIMED_OUT` |
| `wait_many(items, n, deadline)` | An item is 16 bytes: handle `u32`, signals `u32`, observed `u32` (written by the kernel), 4 zero bytes. `n` 0 or > 16: `INVALID_ARGS`. Returns the index of the first item with a signal set |
| `dir_open(dir, name, len, rights)` | Needs `LOOKUP`. Section 8.5 |
| `file_read(h, offset, buf, len)` | `File` or `Grant`; needs `READ`. Copies min(`len`, 65,536, size − `offset`) bytes and returns the count; `offset` at or past the end returns 0. Only the bytes it copies must be writable in the caller, else `INVALID_ARGS` and nothing is copied |
| `file_stat(h, out)` | `File` or `Grant`; needs `READ`. Writes a `FileStat` of 80 bytes: `size` `u64`, `path_len` `u32`, 4 zero bytes, `path` [u8; 64] (with the leading `/`) |
| `grant_create(target, rights, actor)` | Section 8.6. `Grant` handle in `RAX`, `GrantControl` in `RDX` |
| `grant_revoke(control, actor)` | Needs `REVOKE`. Actor one of `prompt`, `shell`, `workbench`, a policy, else `INVALID_ARGS`. Returns `OK` whether or not the grant was still Active |
| `process_spawn(image, bootstrap)` | Section 8.10. `Process` handle in `RAX`, PID in `RDX` |
| `process_kill(process, actor)` | Needs `KILL`. Actor `init`, `shell` or `workbench`, else `INVALID_ARGS`. A process that has already ended: `OK`, nothing happens |
| `diag_note(diag, kind, text, len)` | Needs `WRITE`. `kind` 1 `asked` or 2 `denied`, else `INVALID_ARGS`. `text` is an `AuditNote` of 152 bytes and `len` must be 152: `pid` `u64`, `action` `u32`, `actor` `u32`, `reason` `u32`, `path_len` `u32` (≤ 128), `path` [u8; 128]. A PID that names no process, or an unknown code: `INVALID_ARGS` |
| `console_read(console, buf, cap)` | Needs `READ`. `cap` 0: `INVALID_ARGS`. Returns 1 to min(`cap`, 256) bytes; blocks while none has arrived |

A call that needs a new slot when the table is at its class limit, or when the process cannot pay for another table page, returns `NO_MEMORY`.

### 8.3 Channels

A channel has two ends; each end has a queue of the messages sent to it. `channel_write`, after the checks on `h`, checks in this order and changes nothing unless all pass:

1. The byte buffer and the item array are readable in the caller (`INVALID_ARGS`).
2. Each item: its handle is in the table (`BAD_HANDLE`), carries `TRANSFER` (`ACCESS_DENIED`), and the rights asked are a subset of its own (`INVALID_ARGS`). No handle appears twice, none is an end of this channel, and none is a channel end with messages waiting (`INVALID_ARGS`, D-K-23).
3. The peer end exists (`PEER_CLOSED`).
4. The queue has room for one more message and these bytes, and the sender can pay for the message (`NO_MEMORY`).

Then the handles leave the sender's table, the message is queued, and readers waiting on the peer end are woken. A sent handle arrives with exactly the rights of its item.

`channel_read`, after the checks on `h`, with a message at the head of the queue: the buffer holds its bytes and `hcap` its handles (D-K-24); both buffers are writable in the caller (`INVALID_ARGS`); the caller's table has room for the handles (`NO_MEMORY`). If any check fails the message stays at the head of the queue. Otherwise the bytes are copied, the handles enter the caller's table, the sender is credited, and for each `Grant` handle that enters the table of a process other than the grant's creator for the first time, the grantee is recorded and the `granted` entry is written (section 8.6).

**Closing.** An end is gone when its last handle is closed and no queued message carries it. The other end then reports `PEER_CLOSED`, and the messages still waiting for the gone end are discarded: their senders are credited and the handles in them are closed, which can free further objects. The kernel works through this with a list, not by calling itself, so the depth of the kernel stack does not depend on what a program built.

**Why D-K-23 is enough.** For a channel end to be lost while still holding memory, a set of ends would each have to lie in a queue of the next. The last send that closed such a ring would have to carry an end whose queue already held a message, and that send is refused.

### 8.4 Waiting

| Object | Signals |
|---|---|
| `Channel` end | `READABLE` (a message is queued), `PEER_CLOSED` |
| `Process` | `EXITED` (it has exited, crashed or been ended) |
| `Console` | `READABLE` |

Any other type: `WRONG_TYPE`. `signals` 0, or a signal the type does not have: `INVALID_ARGS`. If a signal is already set the call returns at once. Otherwise, with deadline 0 it returns `TIMED_OUT`; else the thread waits (D-K-26). Deadlines are rounded up to the next tick. A handle that is closed, by its own process's end, while its thread waits on it simply goes with the thread. Ending a waiting thread removes its wait record and nothing else.

### 8.5 Files and directories

`abi::path::check` accepts a name of 1 to 64 bytes, each in `0x21`–`0x7E` and none a backslash, made of components separated by single `/`, with no empty component, no component `.` or `..`, and no `/` at the start or the end. `permd` applies it to what follows the leading `/` of a request; the kernel applies it in `dir_open`.

`dir_open(dir, name, len, rights)`: a name that fails the check, or `rights` not a subset of the `Dir` handle's rights, is `INVALID_ARGS`. The name is taken below the directory. If it is the path of an entry, the result is a `File`; if it is a prefix of entries followed by `/`, a `Dir`; otherwise `NOT_FOUND`. The image has no links, so no name leads outside the directory.

A `File` knows its entry, and from it its path, size and kind.

### 8.6 Grants

`grant_create(target, rights, actor)`: `target` must be a `File` (`WRONG_TYPE`, which is also the answer for a `Grant` or a `GrantControl`) with `GRANT` (`ACCESS_DENIED`). `rights` must be `READ`, or `READ` and `DUPLICATE` (`INVALID_ARGS`), and the `File` handle must carry `READ` (`INVALID_ARGS`). `actor` must be `prompt`, `shell` or a policy (`INVALID_ARGS`). It needs two slots, 256 bytes and a grant record (`NO_MEMORY`). The `Grant` handle carries `rights` and `TRANSFER`; the `GrantControl` handle carries `REVOKE` and nothing else, so it can be neither copied nor sent.

A grant is Active, Revoked or Released, and records what [04 §4](04-security-and-permissions.md#4-grants) lists. Its label is the action word, a space and the file's path with the leading `/`.

| Event | What the kernel does |
|---|---|
| The `Grant` handle first enters the table of a process that is not its creator | Records the grantee. Writes `granted` with the grantee as subject and the actor given at creation |
| `grant_revoke`, or the `GrantControl` handle closes | If Active: becomes Revoked, `revoked` is written once, with the actor given, or `actor=kernel` and `reason=control-closed` or `broker-exit`. Otherwise nothing |
| The last handle to the grant closes (the control handle does not count) | If it has a grantee: becomes Released (a Revoked grant keeps its revocation time), and `released` is written with `actor=kernel`. If it never had a grantee, its record is dropped and nothing is written |
| `file_read`, `file_stat` through a handle | Allowed only while Active; otherwise `REVOKED`, whether the grant is Revoked or Released |

### 8.7 Audit log

The rings, kinds, actors and line format are those of [04 §8](04-security-and-permissions.md#8-audit-log). Fixed here: one sequence number, from 1, counts the entries of both rings in the order they are made; `t` is milliseconds since the kernel's clock started; the mirror line is written when the entry is made. `crashed` is written with the `CRASH` line and `ended` with the `KILLED` line. The rule for `denied` is D-S-6. An entry that leaves a ring is simply gone.

### 8.8 Console

The interrupt handler at vector `0x24` reads bytes from COM1 while the line-status register reports data, puts them in a ring of 256 bytes (a byte that does not fit is dropped and counted), wakes a reader, and sends the end-of-interrupt. One `Console` object exists. The kernel still writes its log to COM1; nothing a person types is echoed by the kernel.

### 8.9 `Diag` additions

`diag_read` gains kinds, all needing `INSPECT`:

| Kind | Record | Cursor |
|---|---|---|
| 5 grants | `GrantRecord`, 168 bytes: `id`, `grantee_pid`, `creator_pid`, `created_ms`, `ended_ms` (`u64` each), `state` (1 Active, 2 Revoked, 3 Released), `rights`, `actor`, `handles` (`u32` each), `label` [u8; 112] | Lowest id; 0 starts at the oldest kept |
| 6 audit, main ring | `AuditRecord`, 216 bytes: `seq`, `time_ms`, `pid`, `grant` (0: none), `last_ms` (`u64` each), `kind`, `actor`, `reason`, `count` (`u32` each), `name` [u8; 16], `label_len` `u32`, 4 zero bytes, `label` [u8; 136] | Lowest `seq` |
| 7 audit, denied ring | The same record | Lowest `seq` |
| 8 handle counts | `HandleCounts`, 32 bytes: `pid` `u64`, then a `u16` count for each of twelve object types in the order of the table in 8.1, the rest zero | Lowest PID |

`ProcessRecord` keeps its 96 bytes: the four spare bytes become `kernel_kib`, and `state` 4 (`ended`) is now reached, by `process_kill`. `wait_reason` gains 2 (channel), 3 (process) and 4 (console).

### 8.10 Process start, kill and the critical flag

`process_spawn(image, bootstrap)`: `image` must be a `File` (`WRONG_TYPE`) with `EXECUTE` (`ACCESS_DENIED`); `bootstrap` must be a channel end (`WRONG_TYPE`) with `TRANSFER` (`ACCESS_DENIED`). Its queue may already hold the launch message: D-K-23 is a rule of `channel_write` and does not apply here, because the end goes into a table and not into a queue. An entry that is not a program is `INVALID_ARGS`. An image the validator refuses is `INVALID_ARGS`, with the Milestone 2 line `SK:KERNEL:SPAWN_REFUSED name=<s> reason=<s>`. A full process table or too little memory is `NO_MEMORY` (`reason=table` or `reason=memory`). On any failure the bootstrap handle stays where it was. On success it leaves the caller's table and is the new process's handle `0x1000` with `READ` and `WRITE`; the caller gets a `Process` handle with `READ` and `KILL`. Name, class, limit and flags come from the image's entry; the new process pays for its own memory.

When a process ends, for whatever cause, in this order: its threads stop; its handles close in slot order, with all that follows from each (sections 8.3 and 8.6; the reason for a revocation is `broker-exit`); the Milestone 2 teardown runs ([10 §8.3](10-milestone-2-spec.md#83-process-start)); `EXITED` is set. If its entry carries `critical`, the kernel then panics with `system service lost: <name>`.

`process_kill` ends a process in state `ended` and writes `SK:KERNEL:KILLED` and the `ended` entry with the actor given.

### 8.11 Messages between programs

Little-endian; the first `u32` is the type. All fit in 512 bytes. The kernel does not read them.

| Type | From → to | Fields | Handles, in order |
|---|---|---|---|
| 1 *launch* | `init` → every process | `scenario` `u32` (0 none, 1 `m3`, 2 `m3-dup`, 3 `m3-brokerkill`, 4 `m3-prober`), `flags` `u32` (bit 0: self-test), `registry_id` `u32` (the index of the program's entry in the image) | App: its end of a request channel, when `permd` runs. `permd`: root `Dir` (`LOOKUP`, `READ`, `GRANT`), `Diag` (`WRITE`), the asking end of the prompt channel, its end of the grant-admin channel. `shell`: `Console`, `Diag` (`INSPECT`), the deciding end of the prompt channel, its end of the grant-admin channel. `inspector`: `Diag` (`INSPECT`) |
| 2 *new client* | `init` → `permd` | `pid` `u64`, `registry_id` `u32` | The serving end of that app's request channel |
| 3 *new channels* | `init` → an app or `shell`, after `permd` was restarted | – | App: a new request end. `shell`: new prompt and grant-admin ends |
| 4 *test-event* | a process → `init` | `code` `u32`: 1 revoke-welcome, 2 kill-broker, 3 flood-in-place | – |
| 5 *test-cue* | `init` → `permd` | `pid` `u64`, `code` `u32` | – |
| 10 *request* | app → `permd` | `action` `u32`, `path_len` `u32`, `path` (at most 255 bytes) | – |
| 11 *reply* | `permd` → app | `status` `u32` | The `Grant`, with `READ` (and `DUPLICATE` under `m3-dup`), when the status is `OK` |
| 20 *ask* | `permd` → `shell` | `request_id` `u64`, `pid` `u64`, `registry_id` `u32`, `action` `u32`, `path_len` `u32`, `path` | – |
| 21 *answer* | `shell` → `permd` | `request_id` `u64`, `allow` `u32` | – |
| 22 *cancel*, 23 *result* | `permd` → `shell` | `request_id` `u64`; *result* also `outcome` `u32` (1 granted, 2 no such file) | – |
| 30 *list grants*, 32 *list waiting*, 34 *revoke grant* {`grant_id` `u64`} | `shell` → `permd` | – | – |
| 31, 33, 35 | `permd` → `shell` | 31: `count` `u32`, then for each `grant_id` `u64`, `pid` `u64`, `path_len` `u32`, `path`, in as many messages as it takes. 33: the same shape for requests. 35: `status` `u32` | – |

A message of an unknown type, of the wrong length, or with handles it should not carry is dropped by the receiver, which closes the handles.

### 8.12 Pure modules and their tests

| Module | Produces | Tests must show |
|---|---|---|
| `sk_kcore::objects` | `Slab<T>`: `insert(T) -> Result<Ref, NoMemory>`, `get(Ref)`, `get_mut(Ref)`, `remove(Ref) -> Option<T>`; `Ref { index, generation }` | A removed reference is not valid again after its place is reused; `insert` reports failure when told that memory is short, and changes nothing |
| `sk_kcore::handles` | The Milestone 2 table with `grow` by pages of 256 to a limit, and `set_rights` | Values are slot + generation as in 8.1; the 257th handle of an App table is refused; a closed value is `BadHandle` after its slot is used again; every value 0–4095 and 4096 fixed pseudo-random values are `BadHandle` on a table that holds two handles |
| `sk_kcore::ipc` | `Channels`: `create`, `write(end, bytes, items) -> Result<Wake, WriteError>`, `read(end, caps) -> Result<Message, ReadError>`, `close(end) -> Closed` (what was freed and whom to credit), `signals(end)` | Each numbered check of 8.3, broken one at a time, changes nothing; 64 messages fit and the 65th does not; 262,144 bytes fit and one more does not; a too-small read leaves the message and reports the sizes; closing an end discards its queue and closes the handles in it, to any depth, without recursion; the two-channel ring of D-K-23 cannot be built in either order |
| `sk_kcore::grants` | `Grants`: `create`, `entered(grant, pid) -> Option<Granted>`, `revoke(grant, how) -> Option<Revoked>`, `handle_closed(grant) -> Option<Released>`, `usable(grant) -> bool`, `records()` | Revoke twice gives one event; a duplicate refers to the same grant and both stop at revoke; released is reported once, and not for a grant without grantee; the 257th Active grant is refused and an older ended record is replaced first; ids never repeat |
| `sk_kcore::audit` | `Log`: `push(Entry) -> Option<Line>`, `note_denied(pid, reason, entry, tick) -> Option<Line>`, `main()`, `denied()`; `Line` formats as in 04 §8 | 257 entries keep the last 256; 1,000 refusals in one second give 8 lines and one entry whose count holds the rest; refusals never displace a main-ring entry; a quote, a backslash and a byte above `0x7E` in a path come out escaped; a path of 200 bytes is cut to 128 before escaping |
| `sk_kcore::waits` | `Wait { items: [(Ref, signals); 16], deadline }`, `wake_on(object) -> impl Iterator<Item = Tid>` | A waiter is woken by its object and by no other; a deadline wakes it; an ended thread leaves no record |
| `sk_kcore::rxring` | `Ring::push(byte) -> bool`, `pop() -> Option<u8>`, `dropped()` | Order is kept; the 257th byte is dropped and counted |
| `sk_kcore::account` | `charge_bytes`, `credit_bytes` for kernel memory | A charge that would pass the limit changes nothing; pages and bytes add up against one limit |
| `abi::path` | `check` | Every refusal of 8.5 has a test; `docs/welcome.txt` passes |
| `abi::msg` | `encode`, `decode` for each type | A round trip for each; every truncation of a valid message is refused |
| `sk_broker::clients` | Per client: `request(now, path) -> Step` (`Refuse(reason)`, `Ask`), `decided(allow)`, `gone()`; the 8-per-second count | A second request while one waits is `busy`; the ninth in a second is `busy`; a bad path is `bad-path`; nothing in a message's bytes changes which client is charged |
| `sk_broker::policy` | `Policy::named(name)`, `decide(path) -> bool`, `with_duplicate() -> bool`, `on_cue(code) -> Action` | The tables of section 9 |
| `sk_broker::admin` | The answers to *list grants*, *list waiting*, *revoke grant* over a table of issued grants | Revoke of an unknown id is `NOT_FOUND`; lists are in id order |

## 9. Programs

**`libstill`.** Wrappers for the new calls; `Handle`, which closes its value when dropped; `recv`/`send` for the messages of 8.11. `out!` now keeps to the output budget by itself: it counts its bytes in windows of one second from its first line and sleeps to the next window before a line that would take the window past 2,048 bytes, so no kernel window of 4,096 bytes is exceeded and no line is lost.

**`init`.** Reads the kernel's message; keeps the root, the full `Diag` and the `Console` only as long as it needs them to hand on. Its table:

| `test=` | Starts, in this order |
|---|---|
| none | `permd` (no policy), `shell`, `ticker`, `notes` |
| `m3`, `m3-dup`, `m3-brokerkill` | `permd` (that policy), `shell`, `ticker`, `notes` with the self-test flag |
| `m3-prober` | `permd` (policy `m3-prober`), `shell`, `ticker`, `prober-copyout`, `prober-chan-flood`, `prober-ask-flood`; then, when `copyout` and `ask-flood` have exited and `chan-flood` has sent *flood-in-place*, `notes` with the self-test flag |
| `m3-initexit` | Nothing; it prints `SK:INIT:EXITING` and exits with code 0 |
| `m2`, `m2-hello`, `m2-preempt`, `m2-prober` | The programs of that Milestone 2 scenario, in its order; `inspector` receives a `Diag` with `INSPECT`; `bin/bad-elf` is attempted last in `m2-prober` |

For each program it follows the steps of [03 §7.1](03-kernel-and-interfaces.md#71-launch-topology). It makes a duplicate of the root with `LOOKUP`, `READ`, `GRANT` and `TRANSFER` for `permd` and sends it with the first three. It then waits, with `wait_many`, on `permd`'s `Process` handle and on the bootstrap channels. When `permd` has ended it starts a new one with a new root, new prompt and grant-admin channels and the same `test=` option, and sends *new client* for every app still running and *new channels* to each of them and to `shell`.

**`permd`.** For each request, in this order: the type and length of the message; `clients::request` (bad path, busy); the `asked` note; the decision (the test policy at once, or *ask* to `shell` and wait for *answer*); for a refusal the `denied` note and the reply `ACCESS_DENIED`; for an allowance `dir_open(root, path, READ + GRANT)`, and if that is `NOT_FOUND` the `denied` note with `reason=no-file` and the reply `ACCESS_DENIED`; otherwise `grant_create`, close the `File` handle, send the reply with the `Grant`, keep the `GrantControl`. It knows a client only by the channel end `init` sent with *new client*. When a client's channel reports `PEER_CLOSED` it cancels a waiting request with a `denied` note (`actor=permd reason=requester-exit`) and closes that client's control handles.

Test policies (`sk_broker::policy`):

| Policy | Allows | On cue *revoke-welcome* for a PID |
|---|---|---|
| `m3`, `m3-prober`, `m3-brokerkill` | `/docs/welcome.txt`, `/docs/large.bin`, `/docs/absent.txt` (which names no file); refuses everything else | Calls `grant_revoke` twice on the newest grant of that PID for `/docs/welcome.txt` and prints `SK:PERMD:REVOKE grant=<id> first=OK second=OK` |
| `m3-dup` | The same, and creates each grant with `DUPLICATE` | The same |

**`shell`.** Waits on the console, the prompt channel and the grant-admin channel. A line ends at CR or LF; backspace and delete remove a byte; other bytes below `0x20` are ignored; a line longer than 64 bytes is refused. Commands: `ps`; `grants`; `requests`; `grant [id]` and `deny [id]` (the oldest waiting request when no id is given); `revoke <grant id>`; `audit [all]` (the last 20 entries of each ring, or all). `ps` and `audit` read `Diag`; the others are the messages of 8.11, and the audit actor of what they cause is `shell`.

**`notes`.** Without the self-test flag: it asks for `/docs/welcome.txt`; when granted it prints `NOTES line="<first line>"` and reads again every second; when a read returns `REVOKED` it prints `NOTES read=REVOKED` and asks once more; when refused it prints `NOTES denied` and sleeps. With the flag it runs the checks below, prints one line for each, then `NOTES_DONE checks=<n>`, and exits with code 0 while it still holds its Active grants. Each line names a criterion of [06](06-milestones.md).

| Line | What `notes` did | Must print | Criterion |
|---|---|---|---|
| `N01 guess` | `file_read` of one byte on every value 0–8191, on 4096 values from a fixed generator, and on each handle it holds | `held=2 wrong_type=2 bad_handle=12286 ok=0 other=0` | 2 |
| `N02 reuse` | Creates a channel, closes both ends, creates another, tries the two closed values, then closes the second pair | `closed=BAD_HANDLE,BAD_HANDLE new=0x2002,0x2003` | 2 |
| `N03 wrongtype` | `dir_open`, `process_spawn` and `grant_create` with each handle it holds | `dir_open=WRONG_TYPE process_spawn=WRONG_TYPE grant_create=WRONG_TYPE handles=2` | 3 |
| `N04 request` | Asks for `/docs/welcome.txt` | `status=OK` | 4 |
| `N05 read` | Reads the whole file through the grant | `bytes=<n> sha256=<hex>`, equal to the host's | 4 |
| `N06 guess` | N01 again | `held=3 wrong_type=2 bad_handle=12286 ok=1 other=0` | 4 |
| `N07 wrongtype` | N03 again, the grant handle included | The same three statuses, `handles=3` | 3 |
| `N08 delegate` | `handle_duplicate` on the grant; a `channel_write` that carries it; a read | `duplicate=ACCESS_DENIED send=ACCESS_DENIED read=OK` | 6 |
| `N09 request` | Asks for `/docs/secret.txt` | `status=ACCESS_DENIED` | 5 |
| `N10 request` | Asks for `/docs/absent.txt` | `status=ACCESS_DENIED` | 5 |
| `N11 large` | Asks for `/docs/large.bin`; reads into a 200 KiB buffer, then in a loop | `first=65536 bytes=204800 sha256=<hex>` | 7 |
| `N12 revoked` | Sends *revoke-welcome*; calls `file_stat` every tick until it is not `OK` (at most 5 s); then `file_read` | `stat=REVOKED read=REVOKED` | 8 |
| `N13 request` | Asks for `/docs/welcome.txt` again | `status=OK` | 8 |
| `N14 again` | Reads through the old handle and through the new one | `old=REVOKED new=OK` | 8 |

Under `m3-dup`, N08 is `N08 duplicate wide=INVALID_ARGS narrow=OK read=OK send=ACCESS_DENIED` and N12 adds `dup=REVOKED` (criterion 10). Under `m3-brokerkill`, after N05 it sends *kill-broker*, then prints `N20 broker read=REVOKED channel=PEER_CLOSED`, waits for *new channels*, asks again and prints `N21 request status=OK` and `N22 read=OK` (criterion 14), and ends there.

The sweep goes to 8191, not to 4095 as [06](06-milestones.md) first said. No handle value is below 4096, because a generation is never 0, so a sweep of 0–4095 can meet no handle of any process. The values 4096–8191 are every slot with generation 1; at that point of the run they include handles that are live in the tables of `init`, `permd` and `shell`, and `notes` must get `BAD_HANDLE` on each of them (the review of Milestone 2 found this gap in the probe `handles`). The fixed generator and its seed are in `stillkey-abi`; a host test shows that none of its 4096 values is a handle value `notes` can hold.

**Probes** (binaries of the package `prober`):

| Binary | Does | Must print |
|---|---|---|
| `prober-copyout` | Creates a channel, writes 16 bytes, reads into its own text, once plain and once with `EFLAGS.AC` set, then into a buffer on its stack | `PROBE copyout text=INVALID_ARGS ac=INVALID_ARGS data=16`, then exit 0 |
| `prober-chan-flood` | 1: `channel_create` until refused. 2: one-byte messages to a channel it never reads until refused. 3: 65,536-byte messages into its other channels until refused. Then sends *flood-in-place* and sleeps for good, holding everything | `PROBE chan-flood channels=127 create=NO_MEMORY small=64 write=NO_MEMORY big=<n> big_write=NO_MEMORY`, where 200 ≤ *n* ≤ 255, and no `EXIT` line |
| `prober-ask-flood` | 1,000 requests for `/docs/none-<i>.txt`, as fast as they are answered | `PROBE ask-flood sent=1000 denied=1000 other=0`, then exit 0 |

The first two phases of `chan-flood` are criterion 12; the third shows the memory limit itself, which the queue limits would otherwise hide. The figures: 254 free slots give 127 channels; a queue takes 64 messages; a 65,536-byte message costs 65,600 bytes, so a limit of 16 MiB allows at most 255 of them, and fewer by what the program's own pages, table and channels cost.

## 10. Acceptance tests

| Test | Kind | Passes when |
|---|---|---|
| `m3-unit` | Host | `cargo test --locked` passes for `stillkey-abi`, `sk-bootfs`, `sk-kcore`, `sk-broker`, `sk-elf`, `stillkey-bootinfo` and `xtask`. No user program's sources contain a `File` or `Dir` handle type outside `init` and `permd` (a check by text) |
| `m3` | Scenario `test=m3` | Criteria 1–9 and 15 of [06](06-milestones.md): the `SPAWN` lines of section 7 with `handles=1`; `SK:PERMD:POLICY name=m3` from PID 2 before the first `AUDIT` line; lines N01–N14 exactly, with the two digests computed on the host; the `AUDIT` lines in order: asked, granted (welcome), asked, denied (secret), asked, denied `reason=no-file` (absent), asked, granted (large), revoked `actor=policy:m3` once for the first grant id, asked, granted (welcome, a new id), and after `SK:KERNEL:EXIT pid=5` one `released` for each of the three grant ids, the revoked one included; `SK:PERMD:REVOKE … first=OK second=OK`; every `granted` follows an `asked` of the same PID and object; `seq` rises by one from 1 and `t` never falls; the text of `secret.txt` is nowhere in the log; `ticker` counts without a gap to the last line; no `PANIC`, `CRASH` or `FAILED` |
| `m3-dup` | Scenario | As `m3` with the policy name and actor `m3-dup`, the N08 and N12 lines of that variant (criterion 10) |
| `m3-prober` | Scenario | Criteria 11–13 and 15: the three `PROBE` lines of section 9; for `ask-flood`'s PID no second holds more than 8 `denied` lines and no more than 8 `asked` lines; then the whole `m3` sequence for `notes` (PID 8) while `chan-flood` is still alive; no `PANIC` |
| `m3-brokerkill` | Scenario | Criterion 14: `AUDIT … kind=ended pid=2 name=permd … actor=init`; `revoked … actor=kernel reason=broker-exit` for the grant id of N04; N20; a second `SPAWN … name=permd` with PID 6 and `SK:PERMD:POLICY` from PID 6; N21 and N22 with a `granted` under a new id |
| `m3-initexit` | Boot test, ends itself | Status 37; `SK:INIT:EXITING`; a panic that names `system service lost: init` |
| `m3-shell` | Scenario with input, no `test=` | The script: after `SK:SHELL:ASK id=1 pid=5 name=notes path="/docs/welcome.txt" action=read` type `grants` (answer `count=0`), `grant` (then `granted … actor=shell` and `NOTES line="Welcome to Stillkey."`), `grants` (one `GRANT` line with that id), `revoke <id>` (then `revoked … actor=shell` and `NOTES read=REVOKED`), and after the second `ASK` type `deny` (then `denied … actor=shell` and `NOTES denied`), `audit` (the three decisions with `actor=shell`), `ps` (`dir=1` for `init` and `permd`, `dir=0` for every other process). No `SK:PERMD:POLICY` line anywhere |
| `m2`, `m2-hello`, `m2-preempt`, `m2-prober` | Scenarios | As accepted, with the changes of D-K-20 |

Criterion 16 stays a check by a person: `cargo xtask run`, and `cargo xtask run --cmdline "test=m3-prober"` to read `audit` after the flood.

**Order and repeats.** `cargo xtask test m3` runs `m3-unit` once; then `m3`, `m3-dup`, `m3-prober`, `m3-brokerkill`, `m3-initexit` and `m3-shell` `--repeat` times each. `m2-repro` already compares every built file and so covers the new programs. `cargo xtask test all` is `m1`, `m2`, `m3`.

**Acceptance (D-W-5):** `tools/acceptance.sh 20` — `cargo xtask test all --repeat 20` in dev and release, under KVM and TCG, with no failure.

## 11. Implementation order

Each step is committed on a branch `milestone-3` when its check passes. "RED" is the failure to see before the code exists.

| Step | Work | Consumes → produces | Check |
|---|---|---|---|
| 0 | Independent review of this document against 03, 04 and 06; correct it | – | Findings in `docs/records/` |
| A | `stillkey-abi`: the constants of section 4, `msg`, `path`, the new `diag` records, with tests that pin every number and size | → all constants and wire structures | RED: the tests name constants that do not exist. `cargo test --locked -p stillkey-abi` |
| B | `sk_kcore::{objects, handles, account}` with tests | A → `Slab`, `Ref`, growing tables, byte charges | `cargo test --locked -p sk-kcore` |
| C | `sk_kcore::{ipc, waits}` with tests | B → `Channels`, `Wait` | The same |
| D | Kernel: `syscall.rs` split out of `proc.rs` with no change in behaviour; `object`, `ipc`; calls 10–14 and 26; the Waiting state and wake-ups; kernel memory charged; heap window checked | A–C → channels in the kernel | RED: a new program `chan-test` (a manifest row for this step only) fails with `INVALID_ARGS` on call 11. Then it prints `CHAN_OK` after sending itself a message with a handle, in both profiles; `cargo xtask test m2` still passes |
| E | `fsobj`, calls 15–17; `process_spawn`, `process_kill`, the critical flag; the kernel starts `init`; `init` with the Milestone 2 rows; `inspector` and `handles` changed; `selftest.rs` | D → `init` starts programs | RED: `m2` fails with `no SPAWN line for init`. Then `cargo xtask test m2` passes in both profiles with the expectations of D-K-20, and `m3-initexit` passes |
| F | `sk_kcore::{grants, audit}` with tests; kernel `grant`, `audit`; calls 10 (grants), 18, 19, 22; `Diag` kinds 5–8 | B, E → grants and the log | Host tests; a boot shows `AUDIT … kind=ended` when `init` kills a test program |
| G | `sk-broker` with tests; `permd` with policies; `init` rows for `m3`, request channels, test events; `gen:large` and the three files; `notes` | F → the `m3` run | RED: `m3` fails with `no SK:PERMD:POLICY line`, then line by line. Then `m3` and `m3-dup` pass under both accelerators |
| H | `permd` restart in `init`; `notes` broker variant | G | RED: `m3-brokerkill` fails with `no second SPAWN of permd`. Then it passes |
| I | The three probes; `init` row `m3-prober` | G | RED: `m3-prober` names the first missing `PROBE` line. Then it passes |
| J | `sk-serial` receive, `rxring`, `console_in`, call 27; `shell`; the prompt and grant-admin messages in `permd`; `Serial::Socket` and the script runner in `xtask` | G → the operator path | RED: `m3-shell` times out waiting for `SK:SHELL:ASK`. Then it passes; then `cargo xtask run` by hand once |
| K | `m3-unit`; `test m3` and `test all`; one mutation pass with `tools/mutate.py` over the new pure modules; the acceptance commands; status paragraphs in this document, 06, 01 and the README | all | Section 10 |

### 11.1 Checkpoints

| Checkpoint | Shows | Contains |
|---|---|---|
| 1 | Channels work, and `init` starts every program: `m2` passes on the new start path, `m3-initexit` passes | Steps A–E |
| 2 | Grant and revoke: `m3`, `m3-dup` | Steps F and G |
| 3 | The broker's end, the floods, the shell, and the acceptance commands | Steps H–K |

## 12. What to review hardest

1. **A duplicate copies the grant instead of referring to it, so a revoke misses it.** Owner: the `grants` host tests (step F) and `m3-dup` (N12 `dup=REVOKED`).
2. **A failed `channel_write` moves some handles and not others.** Owner: the `ipc` tests that break each check of 8.3 in turn (step C); N08 (`read=OK` after the refused send).
3. **An object is freed while something still refers to it, or never.** Owner: the `objects` generation tests and the `ipc` close tests (steps B, C); `m3-brokerkill`, where a whole process's handles close at once; the D-K-23 ring test.
4. **Kernel memory used for a program is not charged to it.** Owner: `account` tests (step B) and all three phases of `chan-flood`, with `notes` served while the flood is in place.
5. **`permd` trusts something a request says.** Owner: the `clients` tests (step G): the client is the channel, never a field. And the path check: every refusal of 8.5 has a test, in `abi::path`, used by both `permd` and the kernel.

Also to be read with care, though a test covers each: the copy-out in `channel_read` to a read-only page (`copyout`); that existence is looked at only after the decision (the `asked` line before both refusals of criterion 5); that `grant_create` and `process_spawn` refuse a `Grant` (N07).

## 13. Coverage check against the design

| Criterion of [06](06-milestones.md) | Where it is met |
|---|---|
| 1 policy line, `handles=1` | Section 7; test `m3` |
| 2 handle guessing, generation | 8.1; N01, N02 |
| 3 wrong type for `dir_open`, `process_spawn`, `grant_create` | 8.2, 8.6, 8.10; N03, N07 |
| 4 asked, granted, exact bytes, one more valid value | 8.6, 8.7; N04–N06; the audit order in `m3` |
| 5 refused and missing both `ACCESS_DENIED`, asked first | Section 9 (`permd`); N09, N10 |
| 6 no duplicate, no send | 8.2, 8.3; N08 |
| 7 at most 64 KiB per read | 8.2; N11 |
| 8 `REVOKED`, second revoke harmless, new id | 8.6; N12–N14; `SK:PERMD:REVOKE` |
| 9 released at exit | 8.6, 8.10; the `released` lines in `m3` |
| 10 duplicate with fewer rights, both revoked | `m3-dup` |
| 11 copy-out | `prober-copyout` |
| 12 channel and handle flood | `prober-chan-flood` phases 1 and 2 |
| 13 request flood | D-S-6, D-S-7; `prober-ask-flood` |
| 14 broker killed and restarted | 8.10; section 9 (`init`); `m3-brokerkill` |
| 15 `ticker` and no panic | Every scenario |
| 16 the operator, by a person | `cargo xtask run`; and by the tool, `m3-shell` (D-W-9) |
| Deliverables: `object`, `ipc`; calls 10–22, 26, 27; `Console`; limits and charging; audit log and mirror; `init`, `permd`, `shell`; `notes`; three probes; test files; policies | Sections 3, 4, 5, 8 and 9 |

## 14. Assumptions this milestone verifies

| # | Assumption | Check, and fallback |
|---|---|---|
| A-23 | A QEMU socket character device with `logfile=` writes everything the guest sends to that file, whether or not a client is connected | Step J: `m3-shell` reads `SK:SHELL:READY` from `serial.log` before it connects. Fallback: serial on QEMU's standard input and output, with the harness as the other end and the monitor on a socket |
| A-24 | The emulated 16550 raises IRQ 4 for a received byte when `OUT2` is set and the 8259 line is unmasked, under KVM and TCG | Step J: the first typed line arrives. Fallback: none; the shell needs it |
| A-25 | `sha2` with its software backend builds for the user target and hashes 200 KiB in well under a second under TCG | Step G: N11 |
| A-26 | Thirty-four entries and about 1 MiB of programs and files fit the image limits (64 entries, 32 MiB) | `cargo xtask build` in step G |

## 15. Not in this milestone

`Memory` objects, mapping, the keyboard, surfaces, the desktop, the Workbench and the prompt (Milestone 4). Calls 23–25 and 28. `INSPECT_REGISTERS` in anyone's hands. Sending a file grant to another process, and so revocation that reaches a second process (D-S-1; [04 §9](04-security-and-permissions.md#9-what-each-milestone-demonstrates) says so). Writable storage; decisions that outlive a restart. More than one thread per process. More than one CPU. An allocator in `libstill`. A file chooser in place of an app-named path (D-S-5). A defence against a `permd` or `init` that misbehaves: both are trusted.

## 16. Open questions

Each has an answer built into this document, and each could reasonably be decided the other way. On 2026-11-11 all five were left as this document has them.

1. **The Milestone 2 tests change (D-K-20).** They keep their properties but run under `init`, with PIDs one higher. The other way is a frozen Milestone 2 mode in the kernel, which keeps the accepted expectations and tests a start path that nothing else uses.
2. **How refusals are counted (D-S-6) and the cap of 8 requests a second (D-S-7).** Both close a gap in [04 §8](04-security-and-permissions.md#8-audit-log): as written there, a person's two refusals for two paths would share one entry, and an app under a test policy could fill the main ring with `asked` entries.
3. **The shell is also tested by the tool (D-W-9).** It needs a serial socket in one test. Without it, serial input is checked only when a person runs the shell.
4. **Two kinds of handle cannot be sent (D-K-23).** No program in Milestones 3 or 4 needs to send either. The other way is a collector for channel ends nothing can reach.
5. **`init` gives up after five restarts of `permd`** and the system stops, where it could also go on without a broker (no new grants, existing ones revoked).
