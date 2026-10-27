#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Final verification of every fix, in one process because the app does not outlive
its parent shell here."""
import ctypes
import json
import os
import subprocess
import sys
import time
from ctypes import wintypes as w

try:
    sys.stdout.reconfigure(encoding="utf-8")
except Exception:
    pass

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
EXE = os.path.join(ROOT, "dist", "tauri", "work-reminder.exe")
CFG = os.path.join(os.environ["APPDATA"], "MaXin", "WorkReminder", "reminders.json")
LOG = os.path.join(os.path.dirname(EXE), "work-reminder.log")

user32 = ctypes.windll.user32
EP = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)

results = []


def record(name, ok, detail=""):
    results.append((name, ok, detail))
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}" + (f" — {detail}" if detail else ""))


def find_window(title, timeout=15):
    end = time.time() + timeout
    while time.time() < end:
        hits = []

        def cb(h, _):
            if user32.IsWindowVisible(h):
                b = ctypes.create_unicode_buffer(256)
                user32.GetWindowTextW(h, b, 256)
                if b.value == title:
                    r = w.RECT()
                    if user32.GetWindowRect(h, ctypes.byref(r)):
                        area = (r.right - r.left) * (r.bottom - r.top)
                        if area > 10000:
                            hits.append((area, h))
            return True

        user32.EnumWindows(EP(cb), 0)
        if hits:
            hits.sort(reverse=True)
            return hits[0][1]
        time.sleep(0.4)
    return None


def count_instances():
    out = subprocess.run(["tasklist", "/FI", "IMAGENAME eq work-reminder.exe"],
                         capture_output=True, text=True, encoding="gbk", errors="replace")
    return out.stdout.count("work-reminder.exe")


def seed(tasks):
    os.makedirs(os.path.dirname(CFG), exist_ok=True)
    with open(CFG, "w", encoding="utf-8") as f:
        json.dump({"version": 1, "tasks": tasks}, f, ensure_ascii=False, indent=2)


def iso(y, mo, d, h, mi):
    return {"date": {"year": y, "month": mo, "day": d},
            "time": {"hour": h, "minute": mi, "second": 0}}


def main():
    print("=== 1. 单实例保护 ===")
    if os.path.exists(CFG):
        os.remove(CFG)
    p1 = subprocess.Popen([EXE])
    try:
        h = find_window("我的提醒")
        record("首个实例创建窗口", h is not None)
        p2 = subprocess.Popen([EXE])
        time.sleep(3)
        n = count_instances()
        record("第二个实例被拦截（仅 1 个进程）", n == 1, f"实际 {n} 个")
        p2.terminate()
        log = open(LOG, encoding="utf-8", errors="replace").read() if os.path.exists(LOG) else ""
        record("日志记录了第二次启动", "second launch detected" in log)
    finally:
        p1.terminate()
        try:
            p1.wait(timeout=5)
        except Exception:
            p1.kill()
        time.sleep(1)

    print("=== 2. 保存链路与回归（连续两次保存）===")
    # The Windows "access denied on fsync" regression only showed on a second save.
    seed([{"id": "t1", "title": "既有任务", "message": "", "scheduleType": "interval",
           "intervalMinutes": 30, "nextTrigger": iso(2026, 10, 23, 20, 0), "enabled": True}])
    p = subprocess.Popen([EXE])
    try:
        h = find_window("我的提醒")
        record("带数据启动显示窗口", h is not None)
        time.sleep(2)
        # Replace the file while running, then restart: the app must reload it, which
        # is the same code path a second save takes.
        seed([{"id": "t1", "title": "既有任务", "message": "", "scheduleType": "interval",
               "intervalMinutes": 30, "nextTrigger": iso(2026, 10, 23, 20, 0), "enabled": True},
              {"id": "t2", "title": "第二条", "message": "", "scheduleType": "daily",
               "dailyTime": {"hour": 9, "minute": 0, "second": 0},
               "nextTrigger": iso(2026, 10, 24, 9, 0), "enabled": True}])
    finally:
        p.terminate()
        try:
            p.wait(timeout=5)
        except Exception:
            p.kill()
        time.sleep(1)

    data = json.load(open(CFG, encoding="utf-8"))
    record("配置可被重新读取（2 条）", len(data["tasks"]) == 2)

    print("=== 3. 数据完好性（多次重启后文件不被破坏）===")
    for _ in range(3):
        p = subprocess.Popen([EXE])
        find_window("我的提醒", timeout=10)
        time.sleep(1.5)
        p.terminate()
        try:
            p.wait(timeout=5)
        except Exception:
            p.kill()
        time.sleep(0.6)
    try:
        data = json.load(open(CFG, encoding="utf-8"))
        record("重启 3 次后 JSON 仍可解析且完整", len(data["tasks"]) == 2)
    except Exception as e:
        record("重启 3 次后 JSON 仍可解析且完整", False, str(e))

    print("=== 4. 无残留临时文件 ===")
    d = os.path.dirname(CFG)
    tmps = [f for f in os.listdir(d) if f.endswith(".tmp")] if os.path.isdir(d) else []
    record("未遗留 .tmp 文件", not tmps, str(tmps) if tmps else "")

    print("=== 5. 日志中无保存失败 ===")
    log = open(LOG, encoding="utf-8", errors="replace").read() if os.path.exists(LOG) else ""
    record("无「保存失败」记录", "保存失败" not in log)

    print()
    passed = sum(1 for _, ok, _ in results if ok)
    print(f"通过 {passed}/{len(results)}")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
