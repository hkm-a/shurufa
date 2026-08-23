#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""候选窗交互自动化：点击选词 + 滚轮翻页（hosted UI 层）。

不依赖完整 TSF/输入法环境：
1. 启动常驻 shurufa-ui（cand_host）；
2. 用伪 TSF 客户端推送 Show（空 preedit，候选从左侧开始）；
3. pywinauto 找到候选窗并点击第 1 项；
4. 从管道读取 CandCommand，断言收到 Select index=0；
5. 向候选窗 PostMessage WM_MOUSEWHEEL，断言收到 PageNext/PagePrev。

用法：
    python scripts/test-cand-interact.py [--exe target/debug/shurufa-ui.exe]
"""
import argparse
import ctypes
import json
import struct
import subprocess
import sys
import threading
import time

import win32con
import win32file
import win32pipe

WM_LBUTTONDOWN = 0x0201
WM_LBUTTONUP = 0x0202
MK_LBUTTON = 0x0001
WM_MOUSEWHEEL = 0x020A


def connect_pipe(timeout: float = 10.0):
    import win32file as _wf

    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            handle = _wf.CreateFile(
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


def send_show(handle, client_id: int, preedit: str = ""):
    event = {
        "Show": {
            "client_id": client_id,
            "context": {
                "preedit": preedit,
                "candidates": [
                    {"text": "你好", "comment": ""},
                    {"text": "拟好", "comment": ""},
                    {"text": "泥嚎", "comment": ""},
                ],
                "highlighted": 0,
                "cursor_pos": 0,
                "page_no": 0,
                "page_size": 9,
                "is_last_page": False,
                "is_ascii": False,
                "is_full_shape": False,
            },
            "caret_rect": [200, 300, 8, 16],
            "dpi": 96,
        }
    }
    body = json.dumps(event, ensure_ascii=False).encode("utf-8")
    win32file.WriteFile(handle, struct.pack("<I", len(body)) + body)


def decode_command(frame: bytes):
    body = json.loads(frame[4:].decode("utf-8"))
    return body


def find_cand_window(timeout: float = 5.0):
    from pywinauto import findwindows

    deadline = time.time() + timeout
    while time.time() < deadline:
        hs = findwindows.find_windows(class_name="ShurufaCandWin")
        if hs:
            return hs[0]
        time.sleep(0.05)
    raise RuntimeError("未找到候选窗 ShurufaCandWin")


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
        handle = connect_pipe()
        send_show(handle, 101, preedit="")

        hwnd = find_cand_window()
        from pywinauto import Desktop

        win = Desktop(backend="win32").window(handle=hwnd)
        rect = win.rectangle()
        print(f"候选窗 rect=({rect.left},{rect.top},{rect.right},{rect.bottom})")

        commands = []
        reader_done = threading.Event()

        def reader():
            try:
                while True:
                    res = win32file.ReadFile(handle, 65536)
                    if isinstance(res, tuple) and len(res) >= 2:
                        commands.append(decode_command(res[1]))
                    else:
                        break
            except Exception:
                pass
            finally:
                reader_done.set()

        threading.Thread(target=reader, daemon=True).start()
        time.sleep(0.2)

        # 点击第 1 项（空 preedit 时第一项从 padding=8 开始）
        # 直接用 PostMessageW 投递 WM_LBUTTONDOWN/UP，避免无激活窗口的鼠标路由差异。
        client_x = 30
        client_y = (rect.bottom - rect.top) // 2
        lparam = (client_y << 16) | client_x
        ctypes.windll.user32.PostMessageW(hwnd, WM_LBUTTONDOWN, MK_LBUTTON, lparam)
        ctypes.windll.user32.PostMessageW(hwnd, WM_LBUTTONUP, 0, lparam)
        reader_done.wait(3.0)

        if not commands:
            print("FAIL: 点击后未收到 CandCommand")
            return 1
        cmd = commands[0]
        print(f"点击命令: {cmd}")
        if cmd.get("Select", {}).get("index") != 0:
            print(f"FAIL: 期望 Select index=0，实际 {cmd}")
            return 1

        # 滚轮下翻 -> PageNext
        before = len(commands)
        ctypes.windll.user32.PostMessageW(hwnd, WM_MOUSEWHEEL, 0xFF880000, 0)
        deadline = time.time() + 3.0
        while len(commands) == before and time.time() < deadline:
            time.sleep(0.05)
        if len(commands) == before:
            print("FAIL: 滚轮下翻未收到 PageNext")
            return 1
        cmd = commands[before]
        print(f"下翻命令: {cmd}")
        if "PageNext" not in cmd:
            print(f"FAIL: 期望 PageNext，实际 {cmd}")
            return 1

        # 滚轮上翻 -> PagePrev
        before = len(commands)
        ctypes.windll.user32.PostMessageW(hwnd, WM_MOUSEWHEEL, 0x00880000, 0)
        deadline = time.time() + 3.0
        while len(commands) == before and time.time() < deadline:
            time.sleep(0.05)
        if len(commands) == before:
            print("FAIL: 滚轮上翻未收到 PagePrev")
            return 1
        cmd = commands[before]
        print(f"上翻命令: {cmd}")
        if "PagePrev" not in cmd:
            print(f"FAIL: 期望 PagePrev，实际 {cmd}")
            return 1

        print("OK: 点击选词 + 滚轮翻页命令全部正确")
        return 0
    except Exception as exc:  # noqa: BLE001
        print(f"FAIL: {exc}")
        return 1
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=5)
        err = proc.stderr.read() if proc.stderr else ""
        if err:
            print("--- shurufa-ui stderr ---")
            print(err)


if __name__ == "__main__":
    sys.exit(main())
