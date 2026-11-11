"""A sampled mutation pass: one small change to the code at a time, then the
host tests of its crate. A change no test notices is printed as SURVIVED.
Run inside WSL from the workspace root: python3 tools/mutate.py"""
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"
# file, crate, how many mutants, where the code under test ends
TARGETS = [
    ("lib/abi/src/lib.rs", "stillkey-abi", 4, None),
    ("lib/abi/src/diag.rs", "stillkey-abi", 4, None),
    ("lib/sk-bootfs/src/lib.rs", "sk-bootfs", 7, None),
    ("lib/sk-bootfs/src/build.rs", "sk-bootfs", 3, "cfg(test)"),
    ("lib/sk-kcore/src/paging.rs", "sk-kcore", 8, "cfg(test)"),
    ("lib/sk-kcore/src/sched.rs", "sk-kcore", 6, "cfg(test)"),
    ("lib/sk-kcore/src/heap.rs", "sk-kcore", 6, "cfg(test)"),
    ("lib/sk-kcore/src/handles.rs", "sk-kcore", 4, "cfg(test)"),
    ("lib/sk-kcore/src/crash.rs", "sk-kcore", 3, "cfg(test)"),
    ("lib/sk-kcore/src/account.rs", "sk-kcore", 3, "cfg(test)"),
    ("lib/sk-kcore/src/usercopy.rs", "sk-kcore", 4, "cfg(test)"),
    ("lib/sk-kcore/src/logfmt.rs", "sk-kcore", 2, "cfg(test)"),
    ("lib/sk-kcore/src/console.rs", "sk-kcore", 6, "cfg(test)"),
    ("xtask/src/m2.rs", "xtask", 16, "fn scenario("),
    ("xtask/src/follow.rs", "xtask", 2, "/// Starts the VM"),
    ("xtask/src/font.rs", "xtask", 5, "cfg(test)"),
    ("xtask/src/screen.rs", "xtask", 5, "cfg(test)"),
    ("xtask/src/bootfs.rs", "xtask", 4, "cfg(test)"),
]
OPS = [
    (" == ", " != "), (" != ", " == "), (" >= ", " > "), (" <= ", " < "),
    (" < ", " <= "), (" > ", " >= "), (" && ", " || "), (" || ", " && "),
    (" + 1", " + 2"), (" - 1", " + 1"),
]
WORDS = [(r"\btrue\b", "false"), (r"\bfalse\b", "true")]


def candidates(lines, end):
    found = []
    for at, line in enumerate(lines[:end]):
        code = line.strip()
        if not code or code.startswith("//") or '"' in line or "'" in line or code.startswith("#["):
            continue
        for old, new in OPS:
            if old in line:
                found.append((at, old.strip(), line.replace(old, new, 1)))
                break
        else:
            for pattern, new in WORDS:
                if re.search(pattern, line):
                    found.append((at, pattern[2:-2], re.sub(pattern, new, line, count=1)))
                    break
    return found


def run(crate):
    try:
        done = subprocess.run(
            ["timeout", "240", "cargo", "test", "--locked", "-q", "-p", crate],
            cwd=ROOT, capture_output=True, text=True,
        )
    except Exception as error:  # noqa: BLE001
        return "ERROR " + str(error)
    text = done.stdout + done.stderr
    if done.returncode == 0:
        return "SURVIVED"
    if done.returncode == 124:
        return "KILLED(timeout)"
    if "error[" in text or "error:" in text and "test result" not in text:
        return "BUILD"
    return "KILLED"


totals = {}
for path, crate, want, marker in TARGETS:
    original = open(ROOT + path, encoding="utf-8", newline="").read()
    lines = original.split("\n")
    end = len(lines)
    if marker:
        end = next(i for i, line in enumerate(lines) if marker in line)
    found = candidates(lines, end)
    step = max(1, len(found) // want)
    picked = found[step // 2::step][:want]
    try:
        for at, op, changed in picked:
            mutated = lines[:at] + [changed] + lines[at + 1:]
            open(ROOT + path, "w", encoding="utf-8", newline="").write("\n".join(mutated))
            verdict = run(crate)
            totals[verdict] = totals.get(verdict, 0) + 1
            print(f"{verdict} {path}:{at + 1} `{op}` | {lines[at].strip()[:110]}", flush=True)
    finally:
        open(ROOT + path, "w", encoding="utf-8", newline="").write(original)
print("TOTAL", " ".join(f"{k}={v}" for k, v in sorted(totals.items())), flush=True)
clean = subprocess.run(["git", "diff", "--quiet"], cwd=ROOT).returncode == 0
print("TREE", "clean" if clean else "CHANGED", flush=True)
sys.exit(0)
