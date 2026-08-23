#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""候选窗故障注入冒烟：多客户端 + 杀进程 + 重启恢复。

只测 `shurufa-ui` 的 `cand_host` 层面：
1. 启动常驻 `shurufa-ui`；
2. 用两个伪 TSF 客户端分别推送 Show → 断言出现两个独立候选窗；
3. 杀掉 `shurufa-ui` → 断言候选窗消失；
4. 重启 `shurufa-ui` → 再推 Show → 断言候选窗自动恢复。

用法：
    python scripts/test-cand-faults.py [--exe target/debug/shurufa-ui.exe]
"""
import argparse
import json
import struct
import subprocess
import sys
import time

import win32con
import win32file


def connect_pipe(timeout: float = 10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            handle = win32file.CreateFile(
                r"\\.\pipe\shurufa-cand",
                win32con.GENERIC_READ | win32con.GENERIC_WRITE,
                0,
                None,
                win32con.OPEN_EXISTING,
                0,
                None,
            )
            return handle
        except Exception:
            time.sleep(0.05)
    raise RuntimeError("无法连接 shurufa-cand 管道")


def send_show(handle, client_id: int):
    event = {
        "Show": {
            "client_id": client_id,
            "context": {
                "preedit": "nihao",
                "candidates": [
                    {"text": "你好", "comment": ""},
                    {"text": "拟好", "comment": ""},
                ],
                "highlighted": 0,
                "cursor_pos": 0,
                "page_no": 0,
                "page_size": 9,
                "is_last_page": False,
                "is_ascii": False,
                "is_full_shape": False,
            },
            "caret_rect": [200 + client_id, 300, 8, 16],
            "dpi": 96,
        }
    }
    body = json.dumps(event, ensure_ascii=False).encode("utf-8")
    frame = struct.pack("<I", len(body)) + body
    win32file.WriteFile(handle, frame)


def cand_windows():
    from pywinauto import findwindows

    return findwindows.find_windows(class_name="ShurufaCandWin")


def wait_cand_count(count: int, timeout: float = 5.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if len(cand_windows()) == count:
            return
        time.sleep(0.05)
    raise RuntimeError(f"候选窗数量未达到 {count}，实际 {len(cand_windows())}")


def start_ui(exe: str) -> subprocess.Popen:
    return subprocess.Popen([exe])


def stop_ui(proc: subprocess.Popen):
    if proc.poll() is None:
        proc.kill()
        proc.wait(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--exe",
        default=r"target\debug\shurufa-ui.exe",
        help="shurufa-ui.exe 路径（默认 target\\debug\\shurufa-ui.exe）",
    )
    args = parser.parse_args()

    # 清掉可能残留的候选窗
    for hwnd in cand_windows():
        import ctypes

        ctypes.windll.user32.DestroyWindow(hwnd)

    proc = start_ui(args.exe)
    try:
        # 场景 1：两个客户端并发候选窗
        h1 = connect_pipe()
        h2 = connect_pipe()
        send_show(h1, 1)
        send_show(h2, 2)
        wait_cand_count(2)
        print("OK: 两个客户端候选窗同时存在")

        # 场景 2：杀进程 → 候选窗消失
        stop_ui(proc)
        time.sleep(0.5)
        if cand_windows():
            print("FAIL: 杀掉 shurufa-ui 后候选窗仍存在")
            return 1
        print("OK: 杀掉 shurufa-ui 后候选窗消失")

        # 场景 3：重启 → 候选窗自动恢复
        proc = start_ui(args.exe)
        h3 = connect_pipe()
        send_show(h3, 3)
        wait_cand_count(1)
        print("OK: 重启后候选窗自动恢复")
        return 0
    except Exception as exc:  # noqa: BLE001
        print(f"FAIL: {exc}")
        return 1
    finally:
        stop_ui(proc)


if __name__ == "__main__":
    sys.exit(main())
