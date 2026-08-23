#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""候选窗 UI 冒烟测试（pywinauto）。

用法：
    python scripts/test-cand-ui.py [--exe target/debug/shurufa-ui.exe]

流程：
1. 启动 `shurufa-ui --cand-selftest`（该模式会创建 shurufa-cand 管道、
   模拟 TSF 客户端推送一条 Show，并把候选窗保持可见约 2 秒）。
2. 用 pywinauto 按窗口类名 `ShurufaCandWin` 找到候选窗。
3. 断言窗口可见，打印窗口矩形。
4. 等待 selftest 进程正常退出（退出码 0）。
"""
import argparse
import subprocess
import sys
import time


def find_candidate_hwnd(timeout: float = 10.0):
    from pywinauto import findwindows

    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            handles = findwindows.find_windows(class_name="ShurufaCandWin")
        except Exception:
            handles = []
        if handles:
            return handles[0]
        time.sleep(0.05)
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--exe",
        default=r"target\debug\shurufa-ui.exe",
        help="shurufa-ui.exe 路径（默认 target\\debug\\shurufa-ui.exe）",
    )
    args = parser.parse_args()

    proc = subprocess.Popen([args.exe, "--cand-selftest"])
    try:
        hwnd = find_candidate_hwnd()
        if hwnd is None:
            print("FAIL: 未找到候选窗 ShurufaCandWin")
            proc.kill()
            return 1

        import ctypes

        from pywinauto import Desktop

        # 可见性检查用 win32 后端；候选文本用 GetWindowTextW 直接读窗口标题
        # （cand_host 会把候选文本写入标题，pywinauto window_text 对这类
        # 无边框工具窗偶发返回空，ctypes 更稳）。
        win = Desktop(backend="win32").window(handle=hwnd)
        if not win.is_visible():
            print("FAIL: 候选窗存在但不可见")
            proc.kill()
            return 1

        buf = ctypes.create_unicode_buffer(512)
        n = ctypes.windll.user32.GetWindowTextW(hwnd, buf, 512)
        title = buf.value[:n] if n else ""
        if "你好" not in title:
            print(f"FAIL: 候选窗标题未包含预期候选文本，实际={title!r}")
            proc.kill()
            return 1

        rect = win.rectangle()
        print(
            f"OK: 候选窗 hwnd={hwnd} rect=({rect.left},{rect.top},{rect.right},{rect.bottom}) "
            f"title={title!r}"
        )

        code = proc.wait(timeout=10)
        print(f"OK: shurufa-ui --cand-selftest 退出码={code}")
        return 0 if code == 0 else 1
    except Exception as exc:  # noqa: BLE001 - 测试脚本直接暴露错误
        print(f"FAIL: pywinauto 检查异常：{exc}")
        proc.kill()
        return 1


if __name__ == "__main__":
    sys.exit(main())
