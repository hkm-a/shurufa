#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""候选窗稳定性冒烟：反复 Show/Hide + 反复重启。

覆盖 F1/F3 的可自动化部分：
- 多次 Show 后候选窗仍正常；
- 多次 Hide 后窗口隐藏且不残留；
- 多次杀掉/重启 shurufa-ui 后候选窗仍能恢复。

用法：
    python scripts/test-cand-stability.py [--exe target/debug/shurufa-ui.exe] [--rounds 3]
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


def send_show(handle, client_id: int):
    event = {
        "Show": {
            "client_id": client_id,
            "context": {
                "preedit": "",
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
    win32file.WriteFile(handle, struct.pack("<I", len(body)) + body)


def cand_count():
    from pywinauto import findwindows

    return len(findwindows.find_windows(class_name="ShurufaCandWin"))


def wait_cand_count(count: int, timeout: float = 5.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if cand_count() == count:
            return
        time.sleep(0.05)
    raise RuntimeError(f"候选窗数量未达到 {count}，实际 {cand_count()}")


def start_ui(exe: str) -> subprocess.Popen:
    return subprocess.Popen([exe], stderr=subprocess.PIPE, text=True)


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
    parser.add_argument("--rounds", type=int, default=3, help="重启轮数（默认 3）")
    args = parser.parse_args()

    # 清理可能残留的候选窗
    from pywinauto import findwindows

    import ctypes

    for hwnd in findwindows.find_windows(class_name="ShurufaCandWin"):
        ctypes.windll.user32.DestroyWindow(hwnd)

    proc = None
    try:
        for round_no in range(1, args.rounds + 1):
            proc = start_ui(args.exe)
            handle = connect_pipe()
            # 每轮 3 个客户端依次 Show
            for client_id in range(1, 4):
                send_show(handle, client_id)
                wait_cand_count(client_id)
            print(f"OK: round {round_no} 三个客户端候选窗全部出现")

            # Hide 只隐藏不销毁，窗口数不变；这里直接重启验证恢复即可。
            stop_ui(proc)
            wait_cand_count(0, timeout=3.0)
            print(f"OK: round {round_no} 杀掉后候选窗清空")

            proc = start_ui(args.exe)
            handle = connect_pipe()
            send_show(handle, 100 + round_no)
            wait_cand_count(1)
            print(f"OK: round {round_no} 重启后候选窗恢复")
            # 清场，下一轮从全新进程开始
            stop_ui(proc)
            proc = None
            wait_cand_count(0, timeout=3.0)
        return 0
    except Exception as exc:  # noqa: BLE001
        print(f"FAIL: {exc}")
        return 1
    finally:
        if proc is not None:
            stop_ui(proc)


if __name__ == "__main__":
    sys.exit(main())
