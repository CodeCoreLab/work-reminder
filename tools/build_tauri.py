#!/usr/bin/env python3
# Copyright (c) 2026 Ma Xin <maxin@example.com>
# SPDX-License-Identifier: MIT
"""Cross-build the Tauri front end on a remote Linux host and fetch the artifact.

Why a remote host: the local Rust toolchain ships a broken mingw — `as.exe` and
`cc1.exe` are missing from `self-contained/`, so `dlltool` cannot create import
libraries and any crate that needs one fails to link. A Linux host with the whole
mingw-w64 suite plus a working rustup can cross-compile to Windows.

Usage:
    python tools/build_tauri.py            # build and download
    python tools/build_tauri.py --check    # cargo check only (fast feedback)

Environment:
    WM_BUILD_HOST       remote host (required for a remote build)
    WM_BUILD_USER       SSH user (default: root)
    WM_BUILD_KEY        SSH private key path
                        (default: ~/.ssh/id_workbuddy)
    WM_BUILD_REMOTE     remote work directory
                        (default: /opt/workbuddy/build/work-reminder-tauri)
"""
import argparse
import hashlib
import os
import posixpath
import sys
from pathlib import Path

import paramiko

HOST = os.environ.get("WM_BUILD_HOST", "")
REMOTE = os.environ.get("WM_BUILD_REMOTE", "/opt/workbuddy/build/work-reminder-tauri")
USER = os.environ.get("WM_BUILD_USER", "root")
KEY = os.environ.get("WM_BUILD_KEY", str(Path.home() / ".ssh" / "id_workbuddy"))
TARGET = "x86_64-pc-windows-gnu"
LOCAL_ROOT = Path(__file__).resolve().parent.parent / "tauri"
LOCAL_OUT = Path(__file__).resolve().parent.parent / "dist" / "tauri"


def connect():
    if not HOST:
        raise SystemExit(
            "WM_BUILD_HOST is not set.\n"
            "Set it to a Linux host with mingw-w64 and rustup installed, e.g.\n"
            "  set WM_BUILD_HOST=your-build-host\n"
            "A local build works too if your Rust toolchain has a working mingw:\n"
            "  cd tauri && cargo build --release --target x86_64-pc-windows-gnu"
        )
    c = paramiko.SSHClient()
    c.set_missing_host_key_policy(paramiko.WarningPolicy())
    c.load_system_host_keys()
    c.connect(
        HOST,
        22,
        USER,
        key_filename=KEY,
        timeout=20,
        banner_timeout=20,
        auth_timeout=20,
    )
    return c


def run(c, cmd, timeout=300):
    _, out, err = c.exec_command(cmd, timeout=timeout)
    o = out.read().decode(errors="replace")
    e = err.read().decode(errors="replace")
    return o + e


def upload(c, root: Path, remote: str):
    """Push the source tree, skipping build output.

    The skip test is per-path, not per-file: `core/target` is nested, so a
    top-level check would ship megabytes of stale artifacts and mask real errors.
    """
    sftp = c.open_sftp()
    try:
        run(c, f"rm -rf {remote} && mkdir -p {remote}")
        dirs = set()
        sent = 0
        for path in root.rglob("*"):
            rel = path.relative_to(root).as_posix()
            if path.is_dir():
                continue
            if "/target/" in f"/{rel}" or rel.startswith("target/"):
                continue
            if rel.endswith(".exe") or rel.endswith(".lock"):
                continue
            parent = posixpath.dirname(f"{remote}/{rel}")
            if parent and parent not in dirs:
                run(c, f"mkdir -p {parent}")
                dirs.add(parent)
            sftp.put(str(path), f"{remote}/{rel}")
            sent += 1
        return sent
    finally:
        sftp.close()


def sha256_local(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true",
                    help="run cargo check instead of a full release build")
    args = ap.parse_args()

    c = connect()
    try:
        # The jump host's own crates.io index host is blocked; the mirror is set up
        # in /root/.cargo/config.toml. Verify it before a long build fails obscurely.
        cfg = run(c, "cat /root/.cargo/config.toml 2>/dev/null || echo MISSING")
        if "mirrors.ustc.edu.cn" not in cfg:
            print("ERROR: jump host missing the crates.io mirror config", file=sys.stderr)
            print("       expected /root/.cargo/config.toml to replace crates-io", file=sys.stderr)
            return 2

        print(f"uploading {LOCAL_ROOT} -> {HOST}:{REMOTE}")
        sent = upload(c, LOCAL_ROOT, REMOTE)
        print(f"  {sent} files")

        # Stale locks from a previous upload can pin an old dependency graph.
        run(c, f"rm -f {REMOTE}/Cargo.lock {REMOTE}/core/Cargo.lock")

        if args.check:
            cmd = f"cd {REMOTE} && cargo check --target {TARGET} 2>&1 | tail -35"
        else:
            cmd = (f"cd {REMOTE} && cargo build --release --target {TARGET} "
                   f"2>&1 | tail -35")
        print(f"building ({'check' if args.check else 'release'}) ...")
        log = run(c, cmd, timeout=1800)
        print(log)

        if args.check:
            return 0

        exe_remote = f"{REMOTE}/target/{TARGET}/release/work-reminder.exe"
        probe = run(c, f"ls -la {exe_remote} 2>&1; sha256sum {exe_remote} 2>&1")
        print(probe)
        if "No such file" in probe or "sha256sum: " in probe and "cannot" in probe:
            print("ERROR: build produced no artifact", file=sys.stderr)
            return 1

        LOCAL_OUT.mkdir(parents=True, exist_ok=True)
        local_exe = LOCAL_OUT / "work-reminder.exe"
        sftp = c.open_sftp()
        try:
            # Fetch to a temporary name so a dropped connection cannot leave a
            # truncated binary looking like a valid build.
            tmp = local_exe.with_suffix(".part")
            sftp.get(exe_remote, str(tmp))
        finally:
            sftp.close()
        tmp.replace(local_exe)

        # The mingw build imports WebView2Loader.dll at run time (the MSVC toolchain
        # links it statically instead). Ship it beside the exe or the app dies during
        # load with STATUS_DLL_NOT_FOUND and no visible error.
        dll_remote = f"{REMOTE}/target/{TARGET}/release/WebView2Loader.dll"
        dll_local = LOCAL_OUT / "WebView2Loader.dll"
        have_dll = run(c, f"ls -la {dll_remote} 2>&1")
        if "No such file" not in have_dll:
            sftp = c.open_sftp()
            try:
                tmp_dll = dll_local.with_suffix(".part")
                sftp.get(dll_remote, str(tmp_dll))
            finally:
                sftp.close()
            tmp_dll.replace(dll_local)
            print(f"  + WebView2Loader.dll ({dll_local.stat().st_size:,} bytes)")
        else:
            print("  ! WebView2Loader.dll was not staged by the build; the exe will fail to start")

        remote_hash = ""
        for line in probe.splitlines():
            if len(line.split()) == 2 and len(line.split()[0]) == 64:
                remote_hash = line.split()[0]
        local_hash = sha256_local(local_exe)
        size = local_exe.stat().st_size
        print(f"artifact: {local_exe}")
        print(f"  {size:,} bytes")
        print(f"  sha256 {local_hash}")
        if remote_hash and remote_hash != local_hash:
            print("ERROR: transfer corrupted (hash mismatch)", file=sys.stderr)
            return 1
        print("  transfer verified")
        return 0
    finally:
        c.close()


if __name__ == "__main__":
    sys.exit(main())
