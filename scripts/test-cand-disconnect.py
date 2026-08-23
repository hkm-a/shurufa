#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""候选窗管道断开清理自动化：客户端断开后旧窗隐藏，新客户端可恢复。

覆盖 B3：
1. 两个伪 TSF 客户端分别推不同候选文本；
2. 关闭其中一个客户端连接；
3. 断言对应候选窗被隐藏（不残留可见窗口）；
4. 新客户端连接推新 Show，断言候选窗恢复。

用法：
    python scripts/test-cand-disconnect.py [--exe target/debug/shurufa-ui.exe]
"""
import argparse
import json
import struct
import subprocess
import sys
import time

import win32con
import win32file
import win32pipe


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
            win32pipe.SetNamedPipeHandleState(
                handle, win32pipe.PIPE_READMODE_MESSAGE, None, None
            )
            return handle
        except Exception:
            time.sleep(0.05)
    raise RuntimeError("无法连接 shurufa-cand 管道")


def send_show(handle, client_id: int, text: str):
    event = {
        "Show": {
            "client_id": client_id,
            "context": {
                "preedit": "",
                "candidates": [{"text": text, "comment": ""}],
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
    win32file.WriteFile(handle, struct.pack("<I", len(body)) + body)


def find_by_title(fragment: str, timeout: float = 5.0):
    import ctypes

    from pywinauto import findwindows

    deadline = time.time() + timeout
    while time.time() < deadline:
        for hwnd in findwindows.find_windows(class_name="ShurufaCandWin"):
            buf = ctypes.create_unicode_buffer(512)
            ctypes.windll.user32.GetWindowTextW(hwnd, buf, 512)
            if fragment in buf.value:
                return hwnd
        time.sleep(0.05)
    return None


def is_visible(hwnd) -> bool:
    from pywinauto import Desktop

    try:
        return Desktop(backend="win32").window(handle=hwnd).is_visible()
    except Exception:
        return False


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--exe",
        default=r"target\debug\shurufa-ui.exe",
        help="shurufa-ui.exe 路径（默认 target\\debug\\shurufa-ui.exe）",
    )
    args = parser.parse_args()

    proc = subprocess.Popen([args.exe], stderr=subprocess.PIPE, text=True)
    try:
        a = connect_pipe()
        b = connect_pipe()
        send_show(a, 201, "甲")
        send_show(b, 202, "乙")

        hwnd_a = find_by_title("甲")
        hwnd_b = find_by_title("乙")
        if hwnd_a is None or hwnd_b is None:
            print("FAIL: 两个客户端候选窗未同时出现")
            return 1
        print(f"OK: 客户端 A/B 候选窗同时存在 hwnd=({hwnd_a},{hwnd_b})")

        # 关闭 A 连接，cand_host 应清理并隐藏 A 的窗口
        a.close()
        time.sleep(1.0)
        if is_visible(hwnd_a):
            print("FAIL: A 断开后 A 的候选窗仍可见")
            return 1
        print("OK: A 断开后 A 候选窗已隐藏")

        # 新客户端 C 推新 Show，候选窗恢复
        c = connect_pipe()
        send_show(c, 203, "丙")
        hwnd_c = find_by_title("丙")
        if hwnd_c is None or not is_visible(hwnd_c):
            print("FAIL: C 连接后候选窗未恢复")
            return 1
        print("OK: C 连接后候选窗恢复")

        return 0
    except Exception as exc:  # noqa: BLE001
        print(f"FAIL: {exc}")
        return 1
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=5)


if __name__ == "__main__":
    sys.exit(main())
