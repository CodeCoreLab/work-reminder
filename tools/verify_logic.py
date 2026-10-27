#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Verify pure logic through the binary's own probes, not through pixel guessing.

UI click coordinates break whenever the layout moves; these checks exercise the same
code paths the UI calls and assert on their output.
"""
import json
import subprocess
import sys
from pathlib import Path

try:
    sys.stdout.reconfigure(encoding="utf-8")
except Exception:
    pass

EXE = Path(__file__).resolve().parent.parent / "dist" / "tauri" / "work-reminder.exe"
KINDS = ["interval", "daily", "once", "weekly", "workdays", "monthly",
         "multiDaily", "timedInterval", "countdown", "pomodoro",
         "afterStartup", "usageBreak"]


def run(*args):
    r = subprocess.run([str(EXE), *args], capture_output=True, timeout=30)
    return r.returncode, r.stdout.decode("utf-8", "replace"), r.stderr.decode("utf-8", "replace")


def check_all_kinds_have_defaults():
    """Every kind must pre-fill a non-empty default: a blank field is a save error."""
    failures = []
    for kind in KINDS:
        code, out, err = run("--params", kind)
        if code != 0:
            failures.append(f"{kind}: exit {code} {err.strip()}")
            continue
        for line in out.strip().splitlines():
            parts = line.split("\t")
            if len(parts) != 3:
                failures.append(f"{kind}: malformed line {line!r}")
                continue
            label, _hint, default = parts
            if not default.strip():
                failures.append(f"{kind}/{label}: empty default")
    return failures


def check_probe_flag():
    """`--probe` must exit 0 and must not start the GUI."""
    code, _out, _err = run("--probe")
    return [] if code == 0 else [f"--probe exit {code}"]


def main():
    failures = []
    failures += check_probe_flag()
    failures += check_all_kinds_have_defaults()

    print(f"kinds checked: {len(KINDS)}")
    if failures:
        print("FAILURES:")
        for f in failures:
            print("  -", f)
        return 1
    print("all defaults non-empty: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
