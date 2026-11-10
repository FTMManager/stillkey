//! The tests `m2` and `m2-preempt` (docs/10 section 10): several programs
//! run until the harness has seen enough. The inspector's tables are
//! checked against the kernel's own lines and against each other.

use std::fs;

use crate::build::Artifacts;
use crate::follow::{self, Verdict};
use crate::image;
use crate::qemu::{Serial, Vm};
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

/// Boots `name` as its own scenario and follows it until `enough` holds or
/// the kernel stops; then runs `check` on all lines.
fn scenario(
    o: &TestOpts,
    art: &Artifacts,
    round: u32,
    name: &str,
    enough: &dyn Fn(&[String]) -> bool,
    check: &dyn Fn(&[String]) -> Result<(), String>,
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
    let raw = fs::read(dir.join("serial.log")).unwrap_or_default();
    let (lines, firmware_lines) = sk_lines(&raw);
    let kept = keep_evidence(&dir, name, art, &lines)?;
    let outcome = Outcome { status: *followed.as_ref().unwrap_or(&-1), lines, firmware_lines };
    let lines = &outcome.lines;

    let verdict = followed.and_then(|_| {
        check_expect(lines, name)?;
        // The harness ended the VM: the kernel neither panicked nor stopped.
        expect_status(&outcome, 0)?;
        let digest = format!("SK:LOADER:BOOTFS_SHA256 {}", art.bootfs_sha256);
        if !lines.contains(&digest) {
            return Err(format!("no line `{digest}`"));
        }
        frames_add_up(lines)?;
        check(lines)
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
    scenario(o, art, round, "m2", &enough, &|lines| check_m2(lines, &|program| image_pages(art, program)))
}

/// `m2-preempt`: a program that never makes a system call does not stop
/// the others.
pub fn preempt_boot(o: &TestOpts, art: &Artifacts, round: u32) -> Result<(), String> {
    scenario(o, art, round, "m2-preempt", &|lines| check_preempt(lines).is_ok(), &check_preempt)
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
}
