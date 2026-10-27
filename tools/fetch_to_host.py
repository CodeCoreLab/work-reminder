#!/usr/bin/env python3
# Copyright (c) 2026 Ma Xin <maxin@example.com>
# SPDX-License-Identifier: MIT
"""Download a resource locally, then push it to a remote build host.

Why this exists: some build hosts reach `index.crates.io` and the USTC mirrors but
are blocked from `static.rust-lang.org` and unreliable for `github`. The local machine has
the opposite problem. Rather than fighting either network, fetch where it works and
transfer the bytes over the SSH channel that is already trusted.

Usage:
    python tools/fetch_to_host.py <url> <remote-path> [--sha256 <hex>]
    python tools/fetch_to_host.py --from-file <local-file> <remote-path>

Environment:
    WM_BUILD_HOST   remote host (required)
    WM_BUILD_USER   SSH user (default: root)
    WM_BUILD_KEY    SSH private key path (default: ~/.ssh/id_workbuddy)

The SHA-256 check is optional but recommended for release archives: a truncated
download that still extracts is the classic way a toolchain install goes subtly wrong.
"""
import argparse
import hashlib
import os
import sys
import urllib.request
from pathlib import Path

import paramiko

HOST = os.environ.get("WM_BUILD_HOST", "")
USER = os.environ.get("WM_BUILD_USER", "root")
KEY = os.environ.get("WM_BUILD_KEY", str(Path.home() / ".ssh" / "id_workbuddy"))


def connect():
    if not HOST:
        raise SystemExit(
            "WM_BUILD_HOST is not set.\n"
            "Set it to the machine that should receive the file, e.g.\n"
            "  set WM_BUILD_HOST=your-build-host"
        )
    c = paramiko.SSHClient()
    c.set_missing_host_key_policy(paramiko.WarningPolicy())
    c.load_system_host_keys()
    c.connect(HOST, 22, USER, key_filename=KEY,
              timeout=20, banner_timeout=20, auth_timeout=20)
    return c


def download(url: str, dest: Path) -> str:
    """Fetch `url` to `dest`, retrying once through the USTC mirror if it is a
    rust-lang.org asset — the direct host is the one most often blocked."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    urls = [url]
    if "static.rust-lang.org" in url:
        urls.append(url.replace("https://static.rust-lang.org",
                                "https://mirrors.ustc.edu.cn/rust-static"))

    last = None
    for candidate in urls:
        try:
            print(f"  GET {candidate}")
            with urllib.request.urlopen(candidate, timeout=120) as r, dest.open("wb") as f:
                while chunk := r.read(1 << 20):
                    f.write(chunk)
            h = hashlib.sha256(dest.read_bytes()).hexdigest()
            print(f"  {dest.stat().st_size:,} bytes  sha256 {h}")
            return h
        except Exception as e:  # noqa: BLE001 - report the real error, try the next URL
            last = e
            print(f"  failed: {e}")
    raise SystemExit(f"all sources failed for {url}: {last}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("url", nargs="?")
    ap.add_argument("remote_path")
    ap.add_argument("--from-file", help="skip the download and push this local file")
    ap.add_argument("--sha256", help="expected digest; abort on mismatch")
    args = ap.parse_args()

    if args.from_file:
        local = Path(args.from_file)
        if not local.exists():
            raise SystemExit(f"no such file: {local}")
        print(f"local file {local} ({local.stat().st_size:,} bytes)")
    else:
        if not args.url:
            raise SystemExit("need a URL or --from-file")
        local = Path("build_cache") / Path(args.url).name
        h = download(args.url, local)
        if args.sha256 and h != args.sha256:
            raise SystemExit(f"sha256 mismatch: got {h}, expected {args.sha256}")

    c = connect()
    try:
        parent = str(Path(args.remote_path).parent).replace("\\", "/")
        c.exec_command(f"mkdir -p {parent}")[1].read()
        sftp = c.open_sftp()
        try:
            # Upload under a temporary name, then rename: a dropped transfer must not
            # leave a half-written archive where a valid one is expected.
            tmp = args.remote_path + ".part"
            sftp.put(str(local), tmp)
            c.exec_command(f"mv -f {tmp} {args.remote_path}")[1].read()
        finally:
            sftp.close()

        _, out, _ = c.exec_command(
            f"ls -la {args.remote_path}; sha256sum {args.remote_path}")
        print(out.read().decode(errors="replace").strip())
        print("uploaded")
        return 0
    finally:
        c.close()


if __name__ == "__main__":
    sys.exit(main())
