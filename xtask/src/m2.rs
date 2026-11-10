//! The tests `m2`, `m2-preempt` and `m2-prober` (docs/10 section 10): several programs
//! run until the harness has seen enough. The inspector's tables are
//! checked against the kernel's own lines and against each other.

use std::fs;

use crate::build::Artifacts;
use crate::follow::{self, Verdict};
use crate::{image, screen};
use crate::qemu::{self, Serial, Vm};
use crate::tests::{Outcome, TestOpts, check_expect, expect_status, frames_add_up, keep_evidence, report, sk_lines};
use crate::util::cache_dir;

/// Where a program's image starts (D-K-14): a crash record's `offset` is
/// counted from here.
const USER_IMAGE_FIRST: u64 = 0x40_0000;
/// Milliseconds per timer tick.
const TICK_MS: u64 = 10;

/// One line a program printed: `SK:APP:OUT pid=<n> name=<s> | <text>`.
#[derive(Debug, PartialEq)]
struct AppLine<'a> {
    /// The line's place among all `SK:` lines.
    index: usize,
    pid: u64,
    name: &'a str,
    text: &'a str,
}

fn app_lines(lines: &[String]) -> Vec<AppLine<'_>> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let rest = line.strip_prefix("SK:APP:OUT pid=")?;
            let (head, text) = rest.split_once(" | ")?;
            let (pid, name) = head.split_once(" name=")?;
            Some(AppLine { index, pid: pid.parse().ok()?, name, text })
        })
        .collect()
}

/// The value of `key=` among the words of `text`.
fn word<'a>(text: &'a str, key: &str) -> Result<&'a str, String> {
    text.split(' ')
        .find_map(|w| w.strip_prefix(key)?.strip_prefix('='))
        .ok_or_else(|| format!("no `{key}=` in `{text}`"))
}

fn num(text: &str, key: &str) -> Result<u64, String> {
    let value = word(text, key)?;
    value.parse().map_err(|_| format!("`{key}={value}` is not a number in `{text}`"))
}

fn hex(text: &str, key: &str) -> Result<u64, String> {
    let value = word(text, key)?;
    value
        .strip_prefix("0x")
        .and_then(|digits| u64::from_str_radix(digits, 16).ok())
        .ok_or_else(|| format!("`{key}={value}` is not a hex number in `{text}`"))
}

/// A `ps` row of a table.
#[derive(Debug, Clone, PartialEq)]
struct Ps {
    pid: u64,
    name: String,
    state: String,
    image: u64,
    heap: u64,
    stack: u64,
    tables: u64,
    cpu: u64,
}

impl Ps {
    fn pages(&self) -> u64 {
        self.image + self.heap + self.stack + self.tables
    }
}

/// A `crash` row of a table.
#[derive(Debug, Clone, PartialEq)]
struct CrashRow {
    pid: u64,
    vector: u64,
    addr: u64,
    rip: u64,
    offset: u64,
}

/// A table the inspector printed from its first line to an `end` line that
/// carries the same change counter: nothing changed while it was read.
#[derive(Debug, Clone, PartialEq)]
struct Table {
    /// The places of its first and last line among all `SK:` lines.
    first: usize,
    last: usize,
    change: u64,
    free: u64,
    kheap: u64,
    uptime_ms: u64,
    ps: Vec<Ps>,
    crashes: Vec<CrashRow>,
}

impl Table {
    fn ps(&self, name: &str) -> Result<&Ps, String> {
        self.ps
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("no `{name}` in the table of change {} at {} ms", self.change, self.uptime_ms))
    }

    /// Frames that are free, the kernel's, or a running program's.
    fn accounted(&self) -> u64 {
        self.free + self.kheap + self.ps.iter().filter(|p| p.state == "Running").map(Ps::pages).sum::<u64>()
    }
}

/// The whole tables among the inspector's lines. A table that was cut off,
/// or whose change counter moved while it was read, is left out; a row that
/// cannot be read is an error.
fn tables(apps: &[AppLine<'_>]) -> Result<Vec<Table>, String> {
    let mut whole = Vec::new();
    let mut open: Option<Table> = None;
    for line in apps.iter().filter(|a| a.name == "inspector") {
        let text = line.text;
        if text.starts_with("table ") {
            open = Some(Table {
                first: line.index,
                last: line.index,
                change: num(text, "change")?,
                free: num(text, "free")?,
                kheap: num(text, "kheap")?,
                uptime_ms: num(text, "uptime_ms")?,
                ps: Vec::new(),
                crashes: Vec::new(),
            });
        } else if text.starts_with("ps ") {
            if let Some(table) = &mut open {
                table.ps.push(Ps {
                    pid: num(text, "pid")?,
                    name: word(text, "name")?.to_string(),
                    state: word(text, "state")?.to_string(),
                    image: num(text, "image")?,
                    heap: num(text, "heap")?,
                    stack: num(text, "stack")?,
                    tables: num(text, "tables")?,
                    cpu: num(text, "cpu")?,
                });
            }
        } else if text.starts_with("crash ") {
            if let Some(table) = &mut open {
                table.crashes.push(CrashRow {
                    pid: num(text, "pid")?,
                    vector: num(text, "vector")?,
                    addr: hex(text, "addr")?,
                    rip: hex(text, "rip")?,
                    offset: hex(text, "offset")?,
                });
            }
        } else if text.starts_with("end ") {
            if let Some(mut table) = open.take() {
                if num(text, "change").ok() == Some(table.change) {
                    table.last = line.index;
                    whole.push(table);
                }
            }
        }
    }
    Ok(whole)
}

/// The ticker's lines: one PID, numbers that count up from 1 without a gap.
/// Returns the place of each line among all `SK:` lines.
fn ticks(apps: &[AppLine<'_>]) -> Result<Vec<usize>, String> {
    let mut places = Vec::new();
    let mut pid = None;
    for line in apps.iter().filter(|a| a.name == "ticker") {
        let want = format!("tick {}", places.len() + 1);
        if line.text != want {
            return Err(format!("the ticker printed `{}`, expected `{want}`", line.text));
        }
        if *pid.get_or_insert(line.pid) != line.pid {
            return Err(format!("`{want}` came from pid {}, earlier ticks from pid {}", line.pid, pid.unwrap_or(0)));
        }
        places.push(line.index);
    }
    Ok(places)
}

fn ticks_after(apps: &[AppLine<'_>], place: usize) -> Result<usize, String> {
    Ok(ticks(apps)?.iter().filter(|&&at| at > place).count())
}

/// The pages a program's image takes, counted from its ELF file.
fn image_pages(art: &Artifacts, program: &str) -> Result<u64, String> {
    let path = art.bootfs.with_file_name("bin").join(program);
    let file = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let image = sk_elf::validate(&file, &sk_elf::user_policy(stillkey_abi::DEFAULT_MEM_LIMIT))
        .map_err(|reason| format!("bin/{program}: {}", reason.as_str()))?;
    Ok(image.segments().iter().map(|s| s.memsz.div_ceil(4096)).sum())
}

fn crash_line(lines: &[String]) -> Option<usize> {
    lines.iter().position(|l| l.starts_with("SK:KERNEL:CRASH "))
}

/// Everything `m2` asks of the lines of one run, given the image pages of
/// the three programs.
fn check_m2(lines: &[String], image_of: &dyn Fn(&str) -> Result<u64, String>) -> Result<(), String> {
    let apps = app_lines(lines);
    let tables = tables(&apps)?;

    // The two calls the kernel must refuse.
    let want = "inspector abi=2 registers=ACCESS_DENIED rotext=INVALID_ARGS";
    let said = apps
        .iter()
        .find(|a| a.name == "inspector" && a.text.starts_with("inspector "))
        .ok_or("the inspector did not print its first line")?;
    if said.text != want {
        return Err(format!("the inspector printed `{}`, expected `{want}`", said.text));
    }

    // One crash: the faulter's write to address 0.
    let crash_at = crash_line(lines).ok_or("no CRASH line")?;
    let crash = &lines[crash_at];
    if lines.iter().filter(|l| l.starts_with("SK:KERNEL:CRASH ")).count() != 1 {
        return Err("more than one CRASH line".to_string());
    }
    let (pid, rip) = (num(crash, "pid")?, hex(crash, "rip")?);
    if word(crash, "name")? != "faulter" || num(crash, "vector")? != 14 || hex(crash, "addr")? != 0 {
        return Err(format!("not the faulter's page fault at address 0: {crash}"));
    }

    // Before it: every program runs, and its image is as large as its file says.
    let before = tables.iter().rfind(|t| t.last < crash_at).ok_or("no whole table before the crash")?;
    for name in ["ticker", "faulter", "inspector"] {
        let ps = before.ps(name)?;
        let image = image_of(name)?;
        if ps.state != "Running" || ps.image != image || ps.stack == 0 || ps.tables == 0 {
            return Err(format!("before the crash, expected {name} Running with image={image}: {ps:?}"));
        }
    }

    // After it: the faulter is marked, with a record of the same fault.
    let after = tables.iter().find(|t| t.first > crash_at).ok_or("no whole table after the crash")?;
    let faulter = after.ps("faulter")?;
    if faulter.pid != pid || faulter.state != "Crashed" {
        return Err(format!("after the crash, expected faulter pid={pid} Crashed: {faulter:?}"));
    }
    let record = CrashRow { pid, vector: 14, addr: 0, rip, offset: rip.wrapping_sub(USER_IMAGE_FIRST) };
    if after.crashes != [record.clone()] {
        return Err(format!("crash rows {:?}, expected {record:?}", after.crashes));
    }

    // Every frame the faulter had came back.
    let prefix = format!("SK:KERNEL:REAPED pid={pid} ");
    let reaped = lines.iter().find(|l| l.starts_with(&prefix)).ok_or("no REAPED line for the faulter")?;
    let had = before.ps("faulter")?.pages();
    if num(reaped, "frames")? != had {
        return Err(format!("the faulter had {had} pages before the crash: {reaped}"));
    }

    // No frame is lost or counted twice, at any time.
    let accounted = before.accounted();
    if let Some(odd) = tables.iter().find(|t| t.accounted() != accounted) {
        return Err(format!(
            "free + kheap + running pages is {} in the table at {} ms, {accounted} at {} ms",
            odd.accounted(),
            odd.uptime_ms,
            before.uptime_ms
        ));
    }

    // The ticker never noticed.
    let more = ticks_after(&apps, crash_at)?;
    if more < 5 {
        return Err(format!("{more} tick lines after the crash, expected 5"));
    }
    Ok(())
}

/// Everything `m2-preempt` asks of the lines of one run.
fn check_preempt(lines: &[String]) -> Result<(), String> {
    let apps = app_lines(lines);
    let start = apps
        .iter()
        .find(|a| a.name == "spinner" && a.text == "spinner start")
        .ok_or("the spinner did not start")?
        .index;
    let more = ticks_after(&apps, start)?;
    if more < 5 {
        return Err(format!("{more} tick lines after `spinner start`, expected 5"));
    }
    if let Some(line) = lines.iter().find(|l| l.starts_with("SK:KERNEL:EXIT") || l.starts_with("SK:KERNEL:CRASH")) {
        return Err(format!("a program ended: {line}"));
    }

    let tables: Vec<Table> = tables(&apps)?.into_iter().filter(|t| t.first > start).collect();
    for table in &tables {
        let spinner = table.ps("spinner")?;
        if spinner.state != "Running" {
            return Err(format!("the spinner is {} at {} ms", spinner.state, table.uptime_ms));
        }
    }
    let (Some(first), Some(last)) = (tables.first(), tables.last()) else {
        return Err("no whole table after `spinner start`".to_string());
    };
    let span = last.uptime_ms - first.uptime_ms;
    if span < 1000 {
        return Err(format!("the tables span {span} ms, expected a second or more"));
    }
    // The spinner never gives the processor up, yet it has it most of the
    // time and the ticker, which sleeps, almost never.
    let passed = span / TICK_MS;
    let spinner = last.ps("spinner")?.cpu - first.ps("spinner")?.cpu;
    let ticker = last.ps("ticker")?.cpu - first.ps("ticker")?.cpu;
    if 2 * spinner < passed || 10 * ticker >= passed {
        return Err(format!("in {passed} ticks the spinner ran {spinner} and the ticker {ticker}"));
    }
    Ok(())
}

/// The probes of docs/10 section 9 in the order of the start table, which
/// gives them the PIDs from 2 on. With each the vector that must end it;
/// `None`: the probe must exit with code 0.
const PROBES: [(&str, Option<u64>); 17] = [
    ("kread", Some(14)),
    ("hlt", Some(13)),
    ("in", Some(13)),
    ("out", Some(13)),
    ("cli", Some(13)),
    ("sse", Some(6)),
    ("x87", Some(7)),
    ("twin-a", None),
    ("twin-b", Some(14)),
    ("memcap", None),
    ("badptr", None),
    ("handles", None),
    ("forge", None),
    ("edge", Some(13)),
    ("wtext", Some(14)),
    ("xstack", Some(14)),
    ("xheap", Some(14)),
];
/// Where the kernel's image starts: what `kread` reads.
const KERNEL_IMAGE: u64 = 0xFFFF_FFFF_8000_0000;
/// The first text of `forge` as the log must show it: one line, with the
/// two line ends written out.
const FORGED: &str = r"a\x0D\x0ASK:KERNEL:CRASH pid=1 name=ticker vector=14\x0D\x0AAUDIT seq=1";
/// What `forge` writes after that: 256 lines of 256 bytes.
const FLOOD_BYTES: u64 = 256 * 256;

/// How a probe ended.
struct Ended {
    pid: u64,
    /// The place of its `CRASH` or `EXIT` line among the `SK:` lines.
    at: usize,
    crashed: bool,
}

/// Where each probe ended, in the order of `PROBES`. The error names the
/// first probe without an end line.
fn probe_ends(lines: &[String]) -> Result<Vec<Ended>, String> {
    let mut ends = Vec::new();
    for (i, (probe, _)) in PROBES.iter().enumerate() {
        let pid = i as u64 + 2;
        let crash = format!("SK:KERNEL:CRASH pid={pid} name=prober-{probe} ");
        let exit = format!("SK:KERNEL:EXIT pid={pid} name=prober-{probe} ");
        let at = lines
            .iter()
            .position(|l| l.starts_with(&crash) || l.starts_with(&exit))
            .ok_or_else(|| format!("the probe `{probe}` (pid {pid}) has no end line"))?;
        ends.push(Ended { pid, at, crashed: lines[at].starts_with(&crash) });
    }
    Ok(ends)
}

fn expect_hex(line: &str, key: &str, want: u64) -> Result<(), String> {
    if hex(line, key)? == want { Ok(()) } else { Err(format!("expected {key}={want:#x} in `{line}`")) }
}

fn expect_lines(printed: &[&str], want: &[&str]) -> Result<(), String> {
    if printed == want { Ok(()) } else { Err(format!("it printed {printed:?}, expected {want:?}")) }
}

/// The address a probe printed in its first line, `<word> addr=<hex>`,
/// which must be its only line.
fn announced(printed: &[&str], word: &str) -> Result<u64, String> {
    match printed {
        [line] if line.starts_with(&format!("{word} addr=")) => hex(line, "addr"),
        _ => Err(format!("it printed {printed:?}, expected one line `{word} addr=<hex>`")),
    }
}

/// What one probe must have printed, and where it must have been ended.
/// `end` is its `CRASH` or `EXIT` line. Returns the address a twin got.
fn check_probe(probe: &str, pid: u64, printed: &[&str], end: &str, lines: &[String]) -> Result<Option<u64>, String> {
    match probe {
        "kread" => {
            expect_lines(printed, &[])?;
            expect_hex(end, "addr", KERNEL_IMAGE)?;
        }
        "hlt" | "in" | "out" | "cli" => {
            expect_lines(printed, &[])?;
            expect_hex(end, "addr", 0)?;
        }
        "sse" | "x87" => expect_lines(printed, &[])?,
        "twin-a" | "twin-b" => {
            let last = if probe == "twin-a" { "twin final=own" } else { "twin freed" };
            let [first, readback, third] = printed else {
                return Err(format!("it printed {printed:?}, expected three lines"));
            };
            let addr = hex(first, "addr")?;
            if !first.starts_with("twin addr=") || *readback != "twin readback=own" || *third != last {
                return Err(format!("it printed {printed:?}, expected an address, `twin readback=own` and `{last}`"));
            }
            if probe == "twin-b" {
                expect_hex(end, "addr", addr)?;
            }
            return Ok(Some(addr));
        }
        "memcap" => {
            let [first, second] = printed else {
                return Err(format!("it printed {printed:?}, expected two lines"));
            };
            let obtained = num(first, "obtained_mib")?;
            if !first.starts_with("memcap obtained_mib=")
                || word(first, "status")? != "NO_MEMORY"
                || !(1..=15).contains(&obtained)
                || *second != "memcap alive"
            {
                return Err(format!("it printed {printed:?}, expected 1 to 15 MiB, `status=NO_MEMORY` and `memcap alive`"));
            }
        }
        "badptr" => expect_lines(
            printed,
            &[
                "badptr case=kernel status=INVALID_ARGS",
                "badptr case=straddle status=INVALID_ARGS",
                "badptr case=long status=INVALID_ARGS",
                "badptr case=ac status=INVALID_ARGS",
            ],
        )?,
        "handles" => expect_lines(printed, &["handles calls=16384 bad_handle=16384 other=0"])?,
        "forge" => {
            let flood = printed.iter().filter(|text| text.len() == 256 && text.bytes().all(|b| b == b'x')).count();
            if printed.first() != Some(&FORGED)
                || printed.last() != Some(&"forge done")
                || printed.len() != flood + 2
            {
                return Err(format!(
                    "expected `{FORGED}`, lines of 256 `x` and `forge done`; it printed {} line(s), the first `{}`",
                    printed.len(),
                    printed.first().unwrap_or(&"")
                ));
            }
            if flood > 32 {
                return Err(format!("{flood} of its 256-byte lines are in the log, more than 32"));
            }
            let dropped_line = format!("SK:APP:DROPPED pid={pid} ");
            let mut dropped = 0;
            for line in lines.iter().filter(|l| l.starts_with(&dropped_line)) {
                dropped += num(line, "bytes")?;
            }
            if 256 * flood as u64 + dropped != FLOOD_BYTES {
                return Err(format!(
                    "{flood} lines of 256 bytes shown and {dropped} bytes dropped do not add up to {FLOOD_BYTES}"
                ));
            }
        }
        "edge" => {
            // A write of no bytes may show as an empty line.
            let cases: Vec<&str> = printed.iter().copied().filter(|text| !text.is_empty()).collect();
            expect_lines(
                &cases,
                &[
                    "edge case=unknown status=INVALID_ARGS",
                    "edge case=write0 status=OK",
                    "edge case=alloc0 status=INVALID_ARGS",
                    "edge case=allocbig status=INVALID_ARGS",
                    "edge case=freebad status=INVALID_ARGS",
                    "edge case=close0 status=BAD_HANDLE",
                ],
            )?;
        }
        "wtext" => {
            expect_hex(end, "addr", announced(printed, "wtext")?)?;
            // Present, write, user.
            expect_hex(end, "error", 0x7)?;
        }
        "xstack" | "xheap" => {
            let addr = announced(printed, probe)?;
            expect_hex(end, "addr", addr)?;
            expect_hex(end, "rip", addr)?;
            // Present, user, instruction fetch.
            expect_hex(end, "error", 0x15)?;
        }
        other => return Err(format!("no rule for the probe `{other}`")),
    }
    Ok(None)
}

/// Everything `m2-prober` asks of one run. `strays` are the lines after
/// `SK:KERNEL:M2_RUNNING` that do not start with `SK:`.
fn check_prober(lines: &[String], strays: &[String]) -> Result<(), String> {
    let ends = probe_ends(lines)?;
    let apps = app_lines(lines);
    let mut twins = Vec::new();
    for (&(probe, vector), end) in PROBES.iter().zip(&ends) {
        let line = &lines[end.at];
        let named = |why: String| format!("probe `{probe}`: {why}");
        match vector {
            Some(vector) if !end.crashed || num(line, "vector").map_err(named)? != vector => {
                return Err(named(format!("the kernel must end it with vector={vector}: `{line}`")));
            }
            None if end.crashed || num(line, "code").map_err(named)? != 0 => {
                return Err(named(format!("it must exit with code 0: `{line}`")));
            }
            _ => {}
        }
        let printed: Vec<&str> = apps.iter().filter(|a| a.pid == end.pid).map(|a| a.text).collect();
        twins.extend(check_probe(probe, end.pid, &printed, line, lines).map_err(named)?);
    }

    // The twins: the same address in two address spaces, and the one that
    // stays is not disturbed when the other is ended.
    if twins.len() != 2 || twins[0] != twins[1] {
        return Err(format!("the twins did not get the same address: {twins:x?}"));
    }
    let twin = |probe: &str| PROBES.iter().position(|(name, _)| *name == probe).map(|i| &ends[i]);
    let (Some(a), Some(b)) = (twin("twin-a"), twin("twin-b")) else {
        return Err("the twins are not in the table".to_string());
    };
    let last_of_a = apps.iter().find(|line| line.pid == a.pid && line.text == "twin final=own");
    if !last_of_a.is_some_and(|line| line.index > b.at) {
        return Err("`twin-a` must print `twin final=own` after `twin-b` was ended".to_string());
    }

    if !lines.iter().any(|l| l == "SK:KERNEL:SPAWN_REFUSED name=bad-elf reason=wx") {
        return Err("no line `SK:KERNEL:SPAWN_REFUSED name=bad-elf reason=wx`".to_string());
    }
    // Nothing else crashed, and no program put a line of its own into the log.
    for (at, line) in lines.iter().enumerate().filter(|(_, l)| l.starts_with("SK:KERNEL:CRASH ")) {
        if !ends.iter().any(|end| end.at == at) {
            return Err(format!("a crash that no probe accounts for: `{line}`"));
        }
    }
    if let Some(line) = strays.first() {
        return Err(format!("a line of the log does not start with `SK:`: `{line}`"));
    }

    let last_end = ends.iter().map(|end| end.at).max().unwrap_or(0);
    let more = ticks_after(&apps, last_end)?;
    if more < 2 {
        return Err(format!("{more} tick line(s) after the last probe ended, expected at least 2"));
    }
    Ok(())
}

/// The lines after `SK:KERNEL:M2_RUNNING` that do not start with `SK:`.
/// From that line on only the kernel's log is on the serial line.
fn strays(raw: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(raw)
        .lines()
        .skip_while(|line| *line != "SK:KERNEL:M2_RUNNING")
        .filter(|line| !line.is_empty() && !line.starts_with("SK:"))
        .map(str::to_string)
        .collect()
}

/// Boots `name` as its own scenario and follows it until `enough` holds or
/// the kernel stops; then runs `check` on all `SK:` lines and on the lines
/// that `strays` finds.
fn scenario(
    o: &TestOpts,
    art: &Artifacts,
    round: u32,
    name: &str,
    enough: &dyn Fn(&[String]) -> bool,
    check: &dyn Fn(&[String], &[String]) -> Result<(), String>,
) -> Result<(), String> {
    let dir = cache_dir().join(name);
    image::make(&dir, art, Some(&format!("qemu_exit=on test={name}")), true)?;
    let vm = Vm {
        dir: &dir,
        accel: o.accel,
        timeout: Some(o.timeout),
        serial: Serial::File,
        gui: false,
        monitor: true,
        trace: false,
        gdb: false,
    };
    let followed = follow::follow(
        &vm,
        &mut |lines| {
            if lines.iter().any(|l| l.starts_with("SK:KERNEL:PANIC")) {
                Verdict::Failed("the kernel stopped".to_string())
            } else if enough(lines) {
                Verdict::Done
            } else {
                Verdict::Waiting
            }
        },
        "quit\n",
    );
    // The harness ends the VM at any moment: the kernel may have been in
    // the middle of a line, and that line is not part of the run.
    let raw = fs::read(dir.join("serial.log")).unwrap_or_default();
    let (lines, firmware_lines) = sk_lines(follow::whole(&raw));
    let kept = keep_evidence(&dir, name, art, &lines)?;
    let outcome = Outcome { status: *followed.as_ref().unwrap_or(&-1), lines, firmware_lines };
    let lines = &outcome.lines;

    let verdict = followed.and_then(|_| {
        check_expect(lines, name)?;
        // The harness ended the VM: the kernel neither panicked nor stopped.
        // If the time limit ended it instead, say what was still missing.
        let strays = strays(follow::whole(&raw));
        expect_status(&outcome, 0).map_err(|status| match check(lines, &strays) {
            Err(missing) => format!("{status}; {missing}"),
            Ok(()) => status,
        })?;
        let digest = format!("SK:LOADER:BOOTFS_SHA256 {}", art.bootfs_sha256);
        if !lines.contains(&digest) {
            return Err(format!("no line `{digest}`"));
        }
        frames_add_up(lines)?;
        check(lines, &strays)
    });
    report(o, name, round, &outcome, &kept, &verdict);
    verdict
}

/// `m2`: a program that crashes is ended, recorded and cleaned up, and the
/// other programs carry on.
pub fn crash_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    let enough = |lines: &[String]| {
        let apps = app_lines(lines);
        crash_line(lines).is_some_and(|crash_at| {
            ticks_after(&apps, crash_at).is_ok_and(|more| more >= 5)
                && tables(&apps).is_ok_and(|tables| tables.iter().any(|t| t.first > crash_at))
        })
    };
    scenario(o, art, round, "m2", &enough, &|lines, _| check_m2(lines, &|program| image_pages(art, program)))
}

/// `m2-preempt`: a program that never makes a system call does not stop
/// the others.
pub fn preempt_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    scenario(o, art, round, "m2-preempt", &|lines| check_preempt(lines).is_ok(), &|lines, _| check_preempt(lines))
}

/// `m2-prober`: seventeen programs that each try something a program may
/// not do, or an edge of a system call (docs/10 section 9), and an image
/// the kernel refuses to start.
pub fn prober_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    let enough = |lines: &[String]| {
        probe_ends(lines).is_ok_and(|ends| {
            let last_end = ends.iter().map(|end| end.at).max().unwrap_or(0);
            ticks_after(&app_lines(lines), last_end).is_ok_and(|more| more >= 2)
        })
    };
    scenario(o, art, round, "m2-prober", &enough, &check_prober)
}

/// `m2-screen`: the kernel writes to its own code, without `qemu_exit`, so
/// it stops with its banner on the screen. Every pixel must be the one the
/// console code draws, on the host, from the serial lines after `FBCON on`.
pub fn screen_boot(o: &TestOpts, art: &Artifacts) -> Result<(), String> {
    let name = "m2-screen";
    let dir = cache_dir().join(name);
    image::make(&dir, art, Some("test=kwrite_text"), true)?;
    let ppm = dir.join("screen.ppm");
    // The kernel ends a line on the serial port after it drew the line.
    let status = qemu::capture_screen(
        &Vm {
            dir: &dir,
            accel: o.accel,
            timeout: Some(o.timeout),
            serial: Serial::File,
            gui: false,
            monitor: true,
            trace: false,
            gdb: false,
        },
        "SK:KERNEL:PANIC_END\r\n",
        &ppm,
    )?;
    let raw = fs::read(dir.join("serial.log")).unwrap_or_default();
    let (lines, firmware_lines) = sk_lines(&raw);
    let kept = keep_evidence(&dir, name, art, &lines)?;
    let outcome = Outcome { status, lines, firmware_lines };
    let lines = &outcome.lines;

    let (mut size, mut drawn_lines) = (String::from("none"), 0);
    let verdict = (|| {
        expect_status(&outcome, 0)?;
        let on = lines
            .iter()
            .position(|l| l.starts_with("SK:KERNEL:FBCON on"))
            .ok_or("no line `SK:KERNEL:FBCON on`")?;
        let drawn = &lines[on + 1..];
        if !drawn.iter().any(|l| l.starts_with("SK:KERNEL:PANIC_END")) {
            return Err("the kernel did not print its banner".to_string());
        }
        drawn_lines = drawn.len();
        let file = fs::read(&ppm).map_err(|e| format!("{}: {e}", ppm.display()))?;
        let picture = screen::parse_ppm(&file)?;
        size = format!("{}x{}", picture.width, picture.height);
        let grid = (picture.width as u64 / 8, (picture.height as u64).saturating_sub(16) / 16);
        if (num(&lines[on], "cols")?, num(&lines[on], "rows")?) != grid {
            return Err(format!("a {size} screen has {} columns and {} rows: {}", grid.0, grid.1, lines[on]));
        }
        screen::check_same(&picture, &screen::render_console(picture.width, picture.height, drawn)?)
    })();
    println!(
        "RESULT test={name} profile={} accel={} status={} screen={size} lines={drawn_lines} result={}",
        o.profile(),
        o.accel.as_str(),
        outcome.status,
        if verdict.is_ok() { "pass" } else { "fail" },
    );
    if let Err(why) = &verdict {
        for line in lines {
            println!("  {line}");
        }
        println!("  FAILED: {why}");
        println!("  logs: {}", kept.display());
    }
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(pid: u64, name: &str, text: &str) -> String {
        format!("SK:APP:OUT pid={pid} name={name} | {text}")
    }

    fn ps(pid: u64, name: &str, state: &str, cpu: u64) -> String {
        out(3, "inspector", &format!("ps pid={pid} name={name} class=App state={state} image=3 heap=0 stack=16 tables=7 cpu={cpu}"))
    }

    fn table(change: u64, free: u64, uptime_ms: u64, rows: &[String], end: u64) -> Vec<String> {
        let mut lines = vec![out(
            3,
            "inspector",
            &format!("table change={change} processes=3 crashes=0 free={free} kheap=40 total=9000 uptime_ms={uptime_ms}"),
        )];
        lines.extend_from_slice(rows);
        lines.push(out(3, "inspector", &format!("end change={end}")));
        lines
    }

    const RIP: u64 = 0x40_1234;

    fn crash_row() -> String {
        out(3, "inspector", &format!("crash seq=1 pid=2 name=faulter vector=14 addr=0x0 rip={RIP:#x} offset=0x1234"))
    }

    /// A run of `m2` that is as it should be.
    fn good_m2() -> Vec<String> {
        let running =
            [ps(1, "ticker", "Running", 0), ps(2, "faulter", "Running", 0), ps(3, "inspector", "Running", 1)];
        let mut lines = vec![
            out(2, "faulter", "faulter start"),
            out(3, "inspector", "inspector abi=2 registers=ACCESS_DENIED rotext=INVALID_ARGS"),
        ];
        lines.extend(table(3, 1000, 10, &running, 3));
        lines.push(out(1, "ticker", "tick 1"));
        lines.push(format!("SK:KERNEL:CRASH pid=2 name=faulter vector=14 addr=0x0 rip={RIP:#x} error=0x6"));
        lines.push("SK:KERNEL:REAPED pid=2 frames=26".to_string());
        let crashed =
            [ps(1, "ticker", "Running", 0), ps(2, "faulter", "Crashed", 0), ps(3, "inspector", "Running", 2), crash_row()];
        lines.extend(table(5, 1026, 3010, &crashed, 5));
        lines.extend((2..=6).map(|n| out(1, "ticker", &format!("tick {n}"))));
        lines
    }

    fn m2(lines: &[String]) -> Result<(), String> {
        check_m2(lines, &|_| Ok(3))
    }

    fn replaced(lines: &[String], from: &str, to: &str) -> Vec<String> {
        assert!(lines.iter().any(|l| l.contains(from)), "`{from}` is not in the lines");
        lines.iter().map(|l| l.replace(from, to)).collect()
    }

    #[test]
    fn a_good_run_passes() {
        assert_eq!(m2(&good_m2()), Ok(()));
    }

    #[test]
    fn app_lines_are_taken_apart() {
        let lines = [out(7, "ticker", "tick 1 | more"), "SK:KERNEL:M2_RUNNING".to_string()];
        assert_eq!(app_lines(&lines), [AppLine { index: 0, pid: 7, name: "ticker", text: "tick 1 | more" }]);
    }

    #[test]
    fn a_table_whose_change_counter_moved_is_left_out() {
        let rows = [ps(1, "ticker", "Running", 0)];
        let mut lines = table(3, 1000, 10, &rows, 4);
        lines.extend(table(4, 1000, 20, &rows, 4));
        let found = tables(&app_lines(&lines)).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].change, found[0].first, found[0].last), (4, 3, 5));
    }

    #[test]
    fn a_table_that_was_cut_off_is_left_out() {
        let mut lines = table(3, 1000, 10, &[ps(1, "ticker", "Running", 0)], 3);
        lines.pop();
        assert_eq!(tables(&app_lines(&lines)), Ok(Vec::new()));
    }

    #[test]
    fn a_row_that_cannot_be_read_is_an_error() {
        let lines = table(3, 1000, 10, &[ps(1, "ticker", "Running", 0).replace("cpu=0", "cpu=x")], 3);
        assert!(tables(&app_lines(&lines)).is_err());
    }

    #[test]
    fn the_inspector_must_be_refused_twice() {
        let lines = replaced(&good_m2(), "rotext=INVALID_ARGS", "rotext=OK");
        assert!(m2(&lines).unwrap_err().contains("rotext=OK"));
        let lines = replaced(&good_m2(), "registers=ACCESS_DENIED", "registers=OK");
        assert!(m2(&lines).is_err());
    }

    #[test]
    fn the_crash_must_be_the_page_fault_at_zero() {
        // Only the kernel's line changes; the inspector's row keeps `addr=0x0`.
        let lines = replaced(&good_m2(), "vector=14 addr=0x0 rip=0x401234 error", "vector=14 addr=0x8 rip=0x401234 error");
        assert!(m2(&lines).unwrap_err().contains("address 0"));
        let lines = replaced(&good_m2(), "vector=14 addr=0x0 rip=0x401234 error", "vector=13 addr=0x0 rip=0x401234 error");
        assert!(m2(&lines).is_err());
    }

    #[test]
    fn the_image_must_be_as_large_as_the_file_says() {
        assert!(check_m2(&good_m2(), &|_| Ok(4)).unwrap_err().contains("image=4"));
    }

    #[test]
    fn the_faulter_must_be_crashed_afterwards() {
        let mut lines = good_m2();
        let at = lines.iter().position(|l| l.contains("state=Crashed")).unwrap();
        lines[at] = ps(2, "faulter", "Running", 0);
        assert!(m2(&lines).is_err());
    }

    #[test]
    fn the_record_must_match_the_kernel_line() {
        for (from, to) in [("rip=0x401234 offset", "rip=0x401238 offset"), ("offset=0x1234", "offset=0x1238")] {
            let lines = replaced(&good_m2(), from, to);
            assert!(m2(&lines).unwrap_err().contains("crash rows"), "{from}");
        }
        let lines: Vec<String> = good_m2().into_iter().filter(|l| !l.contains("| crash ")).collect();
        assert!(m2(&lines).unwrap_err().contains("crash rows"));
    }

    #[test]
    fn every_frame_of_the_faulter_must_come_back() {
        let lines = replaced(&good_m2(), "REAPED pid=2 frames=26", "REAPED pid=2 frames=25");
        assert!(m2(&lines).unwrap_err().contains("26 pages"));
    }

    #[test]
    fn a_lost_frame_is_noticed() {
        let lines = replaced(&good_m2(), "free=1026", "free=1025");
        assert!(m2(&lines).unwrap_err().contains("free + kheap"));
    }

    #[test]
    fn the_ticker_must_keep_counting() {
        let mut lines = good_m2();
        lines.pop();
        assert!(m2(&lines).unwrap_err().contains("4 tick lines"));
        let lines = replaced(&good_m2(), "| tick 4", "| tick 5");
        assert!(m2(&lines).unwrap_err().contains("expected `tick 4`"));
        let mut lines = good_m2();
        let last = lines.len() - 1;
        lines[last] = out(4, "ticker", "tick 6");
        assert!(m2(&lines).unwrap_err().contains("pid 4"));
    }

    /// A run of `m2-preempt` in which the spinner got `spinner` of 100 ticks
    /// and the ticker `ticker`.
    fn preempt(spinner: u64, ticker: u64, state: &str) -> Vec<String> {
        let rows = |s, t| [ps(1, "ticker", "Running", t), ps(2, "spinner", state, s), ps(3, "inspector", "Running", 1)];
        let mut lines = vec![out(2, "spinner", "spinner start")];
        lines.extend(table(3, 1000, 10, &rows(1, 0), 3));
        lines.extend((1..=5).map(|n| out(1, "ticker", &format!("tick {n}"))));
        lines.extend(table(3, 1000, 1010, &rows(1 + spinner, ticker), 3));
        lines
    }

    #[test]
    fn a_spinner_that_does_not_starve_the_ticker_passes() {
        assert_eq!(check_preempt(&preempt(95, 1, "Running")), Ok(()));
        assert_eq!(check_preempt(&preempt(50, 9, "Running")), Ok(()));
    }

    #[test]
    fn the_spinner_must_have_the_processor_half_the_time() {
        assert!(check_preempt(&preempt(49, 1, "Running")).unwrap_err().contains("spinner ran 49"));
    }

    #[test]
    fn the_ticker_must_stay_under_a_tenth() {
        assert!(check_preempt(&preempt(60, 10, "Running")).unwrap_err().contains("ticker 10"));
    }

    #[test]
    fn the_spinner_must_stay_running() {
        assert!(check_preempt(&preempt(95, 1, "Crashed")).unwrap_err().contains("Crashed"));
        let mut lines = preempt(95, 1, "Running");
        lines.push("SK:KERNEL:CRASH pid=2 name=spinner vector=13 addr=0x0 rip=0x401000 error=0x0".to_string());
        assert!(check_preempt(&lines).unwrap_err().contains("a program ended"));
    }

    #[test]
    fn the_tables_must_span_a_second() {
        let lines = replaced(&preempt(95, 1, "Running"), "uptime_ms=1010", "uptime_ms=1000");
        assert!(check_preempt(&lines).unwrap_err().contains("span 990 ms"));
    }

    #[test]
    fn five_ticks_must_follow_the_start() {
        let lines: Vec<String> = preempt(95, 1, "Running").into_iter().filter(|l| !l.contains("| tick 5")).collect();
        assert!(check_preempt(&lines).unwrap_err().contains("4 tick lines"));
    }

    // ---- m2-prober ----

    fn pout(pid: u64, probe: &str, text: &str) -> String {
        out(pid, &format!("prober-{probe}"), text)
    }

    fn crash(pid: u64, probe: &str, vector: u64, addr: u64, rip: u64, error: u64) -> String {
        format!("SK:KERNEL:CRASH pid={pid} name=prober-{probe} vector={vector} addr={addr:#x} rip={rip:#x} error={error:#x}")
    }

    fn exit(pid: u64, probe: &str) -> String {
        format!("SK:KERNEL:EXIT pid={pid} name=prober-{probe} code=0")
    }

    const HEAP: u64 = 0x1_0000_0000;
    const STACK: u64 = 0x7FFF_FFFF_EF00;

    fn good_prober() -> Vec<String> {
        let mut l = vec![
            "SK:KERNEL:SPAWN_REFUSED name=bad-elf reason=wx".to_string(),
            "SK:KERNEL:M2_RUNNING".to_string(),
            crash(2, "kread", 14, KERNEL_IMAGE, 0x40_0010, 0x5),
            crash(3, "hlt", 13, 0, 0x40_0010, 0),
            crash(4, "in", 13, 0, 0x40_0010, 0),
            crash(5, "out", 13, 0, 0x40_0010, 0),
            crash(6, "cli", 13, 0, 0x40_0010, 0),
            crash(7, "sse", 6, 0, 0x40_0010, 0),
            crash(8, "x87", 7, 0, 0x40_0010, 0),
            pout(9, "twin-a", "twin addr=0x100000000"),
            pout(10, "twin-b", "twin addr=0x100000000"),
            pout(11, "memcap", "memcap obtained_mib=15 status=NO_MEMORY"),
            pout(11, "memcap", "memcap alive"),
            exit(11, "memcap"),
            pout(12, "badptr", "badptr case=kernel status=INVALID_ARGS"),
            pout(12, "badptr", "badptr case=straddle status=INVALID_ARGS"),
            pout(12, "badptr", "badptr case=long status=INVALID_ARGS"),
            pout(12, "badptr", "badptr case=ac status=INVALID_ARGS"),
            exit(12, "badptr"),
            pout(13, "handles", "handles calls=16384 bad_handle=16384 other=0"),
            exit(13, "handles"),
            pout(14, "forge", FORGED),
        ];
        l.extend((0..15).map(|_| pout(14, "forge", &"x".repeat(256))));
        l.extend([
            pout(15, "edge", "edge case=unknown status=INVALID_ARGS"),
            pout(15, "edge", ""),
            pout(15, "edge", "edge case=write0 status=OK"),
            pout(15, "edge", "edge case=alloc0 status=INVALID_ARGS"),
            pout(15, "edge", "edge case=allocbig status=INVALID_ARGS"),
            pout(15, "edge", "edge case=freebad status=INVALID_ARGS"),
            pout(15, "edge", "edge case=close0 status=BAD_HANDLE"),
            crash(15, "edge", 13, 0, 0x40_0200, 0x102),
            pout(16, "wtext", "wtext addr=0x400000"),
            crash(16, "wtext", 14, 0x40_0000, 0x40_0040, 0x7),
            pout(17, "xstack", &format!("xstack addr={STACK:#x}")),
            crash(17, "xstack", 14, STACK, STACK, 0x15),
            pout(18, "xheap", "xheap addr=0x100000000"),
            crash(18, "xheap", 14, HEAP, HEAP, 0x15),
            out(1, "ticker", "tick 1"),
            out(1, "ticker", "tick 2"),
            pout(9, "twin-a", "twin readback=own"),
            pout(10, "twin-b", "twin readback=own"),
            pout(10, "twin-b", "twin freed"),
            crash(10, "twin-b", 14, HEAP, 0x40_0100, 0x4),
            "SK:APP:DROPPED pid=14 bytes=61696".to_string(),
            pout(14, "forge", "forge done"),
            exit(14, "forge"),
            out(1, "ticker", "tick 3"),
            pout(9, "twin-a", "twin final=own"),
            exit(9, "twin-a"),
            out(1, "ticker", "tick 4"),
            out(1, "ticker", "tick 5"),
        ]);
        l
    }

    fn prober(lines: &[String]) -> Result<(), String> {
        check_prober(lines, &[])
    }

    fn without(lines: &[String], part: &str) -> Vec<String> {
        assert!(lines.iter().any(|l| l.contains(part)), "`{part}` is not in the lines");
        lines.iter().filter(|l| !l.contains(part)).cloned().collect()
    }

    /// The error of a run in which `from` was replaced by `to`.
    fn prober_with(from: &str, to: &str) -> String {
        prober(&replaced(&good_prober(), from, to)).unwrap_err()
    }

    #[test]
    fn a_good_prober_run_passes() {
        assert_eq!(prober(&good_prober()), Ok(()));
    }

    #[test]
    fn the_first_probe_without_an_end_line_is_named() {
        let lines = without(&without(&good_prober(), "CRASH pid=3 "), "CRASH pid=6 ");
        assert_eq!(prober(&lines).unwrap_err(), "the probe `hlt` (pid 3) has no end line");
        assert!(probe_ends(&good_prober()).is_ok());
    }

    #[test]
    fn a_probe_that_was_not_stopped_fails() {
        let error = prober_with(&crash(7, "sse", 6, 0, 0x40_0010, 0), "SK:KERNEL:EXIT pid=7 name=prober-sse code=1");
        assert!(error.contains("probe `sse`") && error.contains("vector=6"), "{error}");
    }

    #[test]
    fn a_probe_must_be_ended_by_its_own_vector() {
        assert!(prober_with("name=prober-x87 vector=7", "name=prober-x87 vector=6").contains("probe `x87`"));
        assert!(prober_with("name=prober-cli vector=13", "name=prober-cli vector=14").contains("probe `cli`"));
    }

    #[test]
    fn a_probe_that_should_exit_must_exit_with_zero() {
        assert!(prober_with("name=prober-handles code=0", "name=prober-handles code=1").contains("code 0"));
        let crashed = prober_with(&exit(12, "badptr"), &crash(12, "badptr", 14, 0, 0, 0));
        assert!(crashed.contains("probe `badptr`"), "{crashed}");
    }

    #[test]
    fn kread_must_fault_at_the_kernel_image() {
        assert!(prober_with("vector=14 addr=0xffffffff80000000", "vector=14 addr=0xffffffff80001000").contains("probe `kread`"));
        assert!(prober_with("name=prober-hlt vector=13 addr=0x0", "name=prober-hlt vector=13 addr=0x8").contains("probe `hlt`"));
    }

    #[test]
    fn the_twins_must_see_only_their_own_bytes() {
        let other = prober(&replaced(&good_prober(), "pid=10 name=prober-twin-b | twin readback=own", "pid=10 name=prober-twin-b | twin readback=other"));
        assert!(other.unwrap_err().contains("probe `twin-b`"));
        assert!(prober_with("twin final=own", "twin final=mixed").contains("probe `twin-a`"));
        assert!(prober_with("pid=10 name=prober-twin-b | twin addr=0x100000000", "pid=10 name=prober-twin-b | twin addr=0x100001000").contains("twin-b"));
        assert!(prober_with("twin freed", "twin free=INVALID_ARGS").contains("probe `twin-b`"));
    }

    #[test]
    fn both_twins_must_have_the_same_address() {
        let lines = replaced(&good_prober(), "pid=9 name=prober-twin-a | twin addr=0x100000000", "pid=9 name=prober-twin-a | twin addr=0x100001000");
        assert!(prober(&lines).unwrap_err().contains("same address"));
    }

    #[test]
    fn the_twin_that_stays_must_finish_after_the_other_was_ended() {
        let mut lines = good_prober();
        let last = lines.iter().position(|l| l.ends_with("twin final=own")).unwrap();
        let crash_at = lines.iter().position(|l| l.starts_with("SK:KERNEL:CRASH pid=10 ")).unwrap();
        let line = lines.remove(last);
        lines.insert(crash_at, line);
        assert!(prober(&lines).unwrap_err().contains("after `twin-b` was ended"));
    }

    #[test]
    fn memcap_must_be_stopped_by_its_limit() {
        assert!(prober_with("obtained_mib=15", "obtained_mib=16").contains("probe `memcap`"));
        assert!(prober_with("obtained_mib=15", "obtained_mib=0").contains("probe `memcap`"));
        assert!(prober_with("status=NO_MEMORY", "status=OK").contains("probe `memcap`"));
        assert!(prober(&without(&good_prober(), "memcap alive")).unwrap_err().contains("probe `memcap`"));
        assert_eq!(prober(&replaced(&good_prober(), "obtained_mib=15", "obtained_mib=1")), Ok(()));
    }

    #[test]
    fn badptr_must_be_refused_four_times_and_print_nothing_else() {
        assert!(prober_with("case=ac status=INVALID_ARGS", "case=ac status=OK").contains("probe `badptr`"));
        assert!(prober_with("case=straddle status=INVALID_ARGS", "case=straddle status=OK").contains("probe `badptr`"));
        let mut lines = good_prober();
        lines.insert(20, pout(12, "badptr", "panic"));
        assert!(prober(&lines).unwrap_err().contains("probe `badptr`"));
    }

    #[test]
    fn every_handle_value_must_be_refused() {
        assert!(prober_with("bad_handle=16384 other=0", "bad_handle=16383 other=1").contains("probe `handles`"));
    }

    #[test]
    fn a_forged_line_is_noticed() {
        // The text unescaped: its first part stays in the program's line.
        let raw = prober_with(FORGED, "a");
        assert!(raw.contains("probe `forge`"), "{raw}");
        let stray = check_prober(&good_prober(), &["AUDIT seq=1".to_string()]).unwrap_err();
        assert!(stray.contains("does not start with `SK:`"), "{stray}");
        let mut lines = good_prober();
        lines.insert(30, "SK:KERNEL:CRASH pid=1 name=ticker vector=14".to_string());
        assert!(prober(&lines).unwrap_err().contains("no probe accounts for"));
    }

    #[test]
    fn the_flood_must_be_cut_and_counted() {
        assert!(prober_with("pid=14 bytes=61696", "pid=14 bytes=61440").contains("do not add up"));
        // The lines of another program do not count.
        assert!(prober_with("DROPPED pid=14 bytes=61696", "DROPPED pid=13 bytes=61696").contains("do not add up"));
        let mut lines = replaced(&good_prober(), "pid=14 bytes=61696", "pid=14 bytes=57088");
        let at = lines.iter().position(|l| l.ends_with("forge done")).unwrap();
        for _ in 0..18 {
            lines.insert(at, pout(14, "forge", &"x".repeat(256)));
        }
        assert!(prober(&lines).unwrap_err().contains("more than 32"));
        // 32 lines in two seconds are allowed.
        lines.remove(at);
        let lines = replaced(&lines, "pid=14 bytes=57088", "pid=14 bytes=57344");
        assert_eq!(prober(&lines), Ok(()));
    }

    #[test]
    fn the_edges_must_answer_as_the_table_says() {
        assert!(prober_with("case=write0 status=OK", "case=write0 status=INVALID_ARGS").contains("probe `edge`"));
        assert!(prober_with("case=close0 status=BAD_HANDLE", "case=close0 status=OK").contains("probe `edge`"));
        assert!(prober_with("case=unknown status=INVALID_ARGS", "case=unknown status=OK").contains("probe `edge`"));
        assert!(prober(&without(&good_prober(), "case=allocbig")).unwrap_err().contains("probe `edge`"));
    }

    #[test]
    fn the_permission_probes_must_fault_where_they_said() {
        assert!(prober_with("addr=0x400000 rip=0x400040 error=0x7", "addr=0x400000 rip=0x400040 error=0x6").contains("probe `wtext`"));
        assert!(prober_with("wtext addr=0x400000", "wtext addr=0x401000").contains("probe `wtext`"));
        let rip = prober_with(&crash(17, "xstack", 14, STACK, STACK, 0x15), &crash(17, "xstack", 14, STACK, 0x40_0050, 0x15));
        assert!(rip.contains("probe `xstack`") && rip.contains("rip"), "{rip}");
        let error = prober_with(&crash(18, "xheap", 14, HEAP, HEAP, 0x15), &crash(18, "xheap", 14, HEAP, HEAP, 0x14));
        assert!(error.contains("probe `xheap`") && error.contains("error"), "{error}");
    }

    #[test]
    fn the_bad_image_must_be_refused_for_its_segment() {
        assert!(prober(&without(&good_prober(), "SPAWN_REFUSED")).unwrap_err().contains("bad-elf"));
        assert!(prober_with("name=bad-elf reason=wx", "name=bad-elf reason=segment").contains("bad-elf"));
    }

    #[test]
    fn the_ticker_must_count_on_after_the_last_probe() {
        assert!(prober(&without(&good_prober(), "| tick 5")).unwrap_err().contains("1 tick line(s)"));
        assert!(prober(&without(&good_prober(), "| tick 2")).unwrap_err().contains("expected `tick 2`"));
    }

    #[test]
    fn strays_are_the_lines_after_the_start_that_are_not_the_kernels() {
        let raw = b"BdsDxe: loading\r\nSK:KERNEL:M2_RUNNING\r\nSK:APP:OUT pid=2 name=a | a\\x0D\\x0Ab\r\n\r\nAUDIT seq=1\r\nSK:B\r\n";
        assert_eq!(strays(raw), ["AUDIT seq=1"]);
        assert!(strays(b"firmware\r\nSK:KERNEL:M2_RUNNING\r\nSK:A\r\n").is_empty());
    }
}
