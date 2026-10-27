#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Screenshot a Tauri window via PrintWindow with PW_RENDERFULLCONTENT.

Plain PrintWindow returns a white rectangle for WebView2 surfaces; flag 2 asks the
compositor to render the web content too. Without it every screenshot of this app
would look blank and hide real layout bugs.
"""
import ctypes
import sys
import time
from ctypes import wintypes as w

from PIL import Image

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32


class BMIH(ctypes.Structure):
    _fields_ = [
        ("biSize", w.DWORD), ("biWidth", w.LONG), ("biHeight", w.LONG),
        ("biPlanes", w.WORD), ("biBitCount", w.WORD), ("biCompression", w.DWORD),
        ("biSizeImage", w.DWORD), ("biXPelsPerMeter", w.LONG),
        ("biYPelsPerMeter", w.LONG), ("biClrUsed", w.DWORD), ("biClrImportant", w.DWORD),
    ]


def find(title: str):
    EnumProc = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    user32.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
    hits = []

    def cb(hwnd, _):
        if user32.IsWindowVisible(hwnd):
            buf = ctypes.create_unicode_buffer(256)
            user32.GetWindowTextW(hwnd, buf, 256)
            if buf.value == title:
                hits.append(hwnd)
        return True

    user32.EnumWindows(EnumProc(cb), 0)
    return hits[0] if hits else None


def grab(hwnd, out: str) -> bool:
    r = w.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    wd, ht = r.right - r.left, r.bottom - r.top
    if wd <= 0 or ht <= 0:
        print(f"bad rect {wd}x{ht}")
        return False
    print(f"rect {r.left},{r.top} {wd}x{ht}")

    hdc = user32.GetWindowDC(hwnd)
    mdc = gdi32.CreateCompatibleDC(hdc)
    bmp = gdi32.CreateCompatibleBitmap(hdc, wd, ht)
    gdi32.SelectObject(mdc, bmp)
    ok = user32.PrintWindow(hwnd, mdc, 2)
    print("PrintWindow:", ok)

    bi = BMIH()
    bi.biSize = ctypes.sizeof(BMIH)
    bi.biWidth = wd
    bi.biHeight = -ht
    bi.biPlanes = 1
    bi.biBitCount = 32
    buf = ctypes.create_string_buffer(wd * ht * 4)
    gdi32.GetDIBits(mdc, bmp, 0, ht, buf, ctypes.byref(bi), 0)
    img = Image.frombuffer("RGBA", (wd, ht), buf, "raw", "BGRA", 0, 1).convert("RGB")
    img.save(out)
    print("saved", out, img.size)
    return True


if __name__ == "__main__":
    title = sys.argv[1] if len(sys.argv) > 1 else "我的提醒"
    out = sys.argv[2] if len(sys.argv) > 2 else "docs/tauri-ui.png"
    for _ in range(20):
        h = find(title)
        if h:
            sys.exit(0 if grab(h, out) else 1)
        time.sleep(0.5)
    print(f"window '{title}' not found")
    sys.exit(1)
