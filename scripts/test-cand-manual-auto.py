#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""候选窗单屏手动项全自动验收（不需要你手动确认）。

用 Notepad4/记事本 + SendInput + pywinauto 自动完成 A/B/C/E 单屏项：
- 自动设置 options.json 的 candidate_window=hosted；
- 自动启动 shurufa-ui（如未运行）；
- 自动打开编辑器、输入拼音、点击候选、读剪贴板验证上屏；
- 自动杀/重启 shurufa-ui 验证回退与恢复；
- 全程无需人工按键/确认。

用法：
    python scripts/test-cand-manual-auto.py [--exe target/debug/shurufa-ui.exe]
"""
import argparse
import ctypes
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

import win32clipboard
import win32con
import win32gui
from pywinauto import Application, Desktop, findwindows, keyboard

HOSTED_CLASS = "ShurufaCandWin"
BUILTIN_CLASS = "ShurufaCandidateWindow"


def app_options_path() -> str:
    return os.path.join(os.environ.get("APPDATA", ""), "shurufa", "options.json")


def read_options() -> dict:
    path = app_options_path()
    try:
        with open(path, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {}


def write_options(opts: dict) -> None:
    path = app_options_path()
    os.makedirs(os.path.dirname(path), exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(opts, f, ensure_ascii=False, indent=2)
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            try:
                os.remove(tmp)
            except OSError:
                pass


def set_candidate_window(value: str, opts: dict) -> None:
    opts["candidate_window"] = value
    write_options(opts)


def find_editor_path():
    candidates = [
        r"C:\Program Files\Notepad4\Notepad4.exe",
        r"C:\Program Files (x86)\Notepad4\Notepad4.exe",
        r"C:\Windows\Notepad4.exe",
    ]
    for c in candidates:
        if os.path.isfile(c):
            return c
    for name in ("notepad4", "Notepad4.exe", "notepad.exe", "notepad"):
        p = shutil.which(name)
        if p:
            return p
    raise RuntimeError("找不到 Notepad4 / 记事本")


def set_focus(win):
    try:
        win.set_focus()
    except Exception:
        hwnd = win.handle
        ctypes.windll.user32.ShowWindow(hwnd, 5)  # SW_SHOW
        ctypes.windll.user32.SetForegroundWindow(hwnd)
    time.sleep(0.2)


def launch_editor(path):
    proc = subprocess.Popen([path])
    app = Application(backend="win32").connect(process=proc.pid, timeout=15)
    win = app.top_window()
    set_focus(win)
    time.sleep(0.5)
    return proc, win


def type_text(win, text):
    set_focus(win)
    keyboard.send_keys(text)
    time.sleep(0.8)


def read_editor_text(win):
    set_focus(win)
    keyboard.send_keys("^a")
    keyboard.send_keys("^c")
    time.sleep(0.3)
    try:
        win32clipboard.OpenClipboard()
        data = win32clipboard.GetClipboardData(win32con.CF_UNICODETEXT)
        win32clipboard.CloseClipboard()
        return data or ""
    except Exception:
        try:
            win32clipboard.CloseClipboard()
        except Exception:
            pass
        return ""


def wait_text(win, fragment, timeout=5.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if fragment in read_editor_text(win):
            return True
        time.sleep(0.2)
    return False


def find_class(class_name, visible_only=True):
    return findwindows.find_windows(class_name=class_name, visible_only=visible_only)


def window_title(hwnd):
    buf = ctypes.create_unicode_buffer(512)
    ctypes.windll.user32.GetWindowTextW(hwnd, buf, 512)
    return buf.value


def click_first_candidate():
    hosted = find_class(HOSTED_CLASS)
    if not hosted:
        return False
    hwnd = hosted[0]
    rect = win32gui.GetWindowRect(hwnd)
    x = 30
    y = (rect[3] - rect[1]) // 2
    lparam = (y << 16) | x
    ctypes.windll.user32.PostMessageW(hwnd, 0x0201, 1, lparam)
    ctypes.windll.user32.PostMessageW(hwnd, 0x0202, 0, lparam)
    time.sleep(1.0)
    return True


def is_ui_running():
    return len(find_class("ShurufaUiHost", visible_only=False)) > 0


def start_ui(exe):
    return subprocess.Popen([exe], stderr=subprocess.PIPE, text=True)


def ensure_ui_running(exe) -> bool:
    """确保 shurufa-ui 在运行；返回是否由本脚本启动。"""
    if is_ui_running():
        print("shurufa-ui 已在运行")
        return False
    print(f"启动 shurufa-ui: {exe}")
    proc = start_ui(exe)
    deadline = time.time() + 8
    while time.time() < deadline:
        if is_ui_running():
            print("shurufa-ui 已就绪")
            return True
        if proc.poll() is not None:
            break
        time.sleep(0.1)
    if proc.poll() is None:
        proc.kill()
        proc.wait(timeout=5)
    print("shurufa-ui 启动失败/超时，后续 hosted 用例可能失败")
    return True  # 仍按启动过处理，由 finally 清理


def stop_ui():
    subprocess.run(["taskkill", "/F", "/IM", "shurufa-ui.exe"], capture_output=True)
    time.sleep(0.5)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--exe",
        default=r"target\debug\shurufa-ui.exe",
        help="shurufa-ui.exe 路径（默认 target\\debug\\shurufa-ui.exe）",
    )
    args = parser.parse_args()

    editor = find_editor_path()
    print(f"使用编辑器: {editor}")

    # 切换 hosted 前保存原值，验收结束恢复
    orig_opts = read_options()
    orig_candidate_window = orig_opts.get("candidate_window")
    had_candidate_window_key = "candidate_window" in orig_opts
    print("设置 candidate_window=hosted")
    set_candidate_window("hosted", dict(orig_opts))
    print("提示：请确保 Notepad4 当前激活的是 Shurufa 输入法；否则真实 TSF 用例会失败。")

    results = []

    def record(name: str, ok: bool):
        results.append((name, ok))
        print(("PASS" if ok else "FAIL") + f" | {name}")

    proc = None
    proc2 = None
    ui_proc = None
    try:
        # 确保 shurufa-ui 常驻，TSF 才能连到 shurufa-cand 管道
        if not is_ui_running():
            print(f"启动 shurufa-ui: {args.exe}")
            ui_proc = start_ui(args.exe)
            deadline = time.time() + 8
            while time.time() < deadline:
                if is_ui_running():
                    print("shurufa-ui 已就绪")
                    break
                if ui_proc.poll() is not None:
                    print("shurufa-ui 启动失败/超时，后续 hosted 用例可能失败")
                    break
                time.sleep(0.1)
        else:
            print("shurufa-ui 已在运行")

        # A1 + A3 + A4 + A5 在一个编辑器会话里完成
        proc, win = launch_editor(editor)
        type_text(win, "nihao")
        hosted = find_class(HOSTED_CLASS)
        record("A1 hosted 候选窗出现且含「你好」", len(hosted) > 0 and any("你好" in window_title(h) for h in hosted))

        click_first_candidate()
        record("A3 点击候选后上屏「你好」", wait_text(win, "你好"))

        # A4 数字选词
        type_text(win, "{ESC}nihao")
        keyboard.send_keys("1")
        record("A4 数字选词后上屏正常", wait_text(win, "你好"))

        # A5 Esc 取消
        type_text(win, "nihao")
        keyboard.send_keys("{ESC}")
        time.sleep(0.5)
        visible = [h for h in find_class(HOSTED_CLASS) if Desktop(backend="win32").window(handle=h).is_visible()]
        record("A5 Esc 后 hosted 候选窗隐藏", len(visible) == 0)

        # B1 杀 ui 回退内置
        stop_ui()
        type_text(win, "nihao")
        record("B1 杀 ui 后回退内置候选窗", len(find_class(BUILTIN_CLASS)) > 0)

        # B2 重启 ui 恢复 hosted
        ui_proc = start_ui(args.exe)
        time.sleep(1.0)
        type_text(win, "nihao")
        record("B2 重启 ui 后恢复 hosted", len(find_class(HOSTED_CLASS)) > 0)

        # C1 两个编辑器多客户端
        proc2, win2 = launch_editor(editor)
        type_text(win, "nihao")
        type_text(win2, "nihao")
        time.sleep(0.5)
        record("C1 两个编辑器出现两个 hosted 候选窗", len(find_class(HOSTED_CLASS)) >= 2)

        # E2 最大化近似全屏回退内置
        win.maximize()
        time.sleep(0.3)
        type_text(win, "nihao")
        time.sleep(0.5)
        builtin = find_class(BUILTIN_CLASS)
        hosted = find_class(HOSTED_CLASS)
        record("E2 最大化时回退内置候选窗", len(builtin) > 0 and len(hosted) == 0)

        # E5 候选文本可读
        win.restore()
        time.sleep(0.3)
        type_text(win, "nihao")
        hosted = find_class(HOSTED_CLASS)
        record("E5 候选文本可被自动化读取", any("你好" in window_title(h) for h in hosted))

        print("\n===== 单屏手动项自动验收汇总 =====")
        failed = [name for name, ok in results if not ok]
        for name, ok in results:
            print(("PASS" if ok else "FAIL") + f" | {name}")
        if failed:
            print(f"未通过 {len(failed)} 项：{', '.join(failed)}")
            return 1
        print("全部通过")
        return 0
    except Exception as exc:  # noqa: BLE001
        print(f"FAIL: {exc}")
        return 1
    finally:
        if ui_proc is not None and ui_proc.poll() is None:
            ui_proc.kill()
            ui_proc.wait(timeout=5)
        for p in (proc, proc2):
            if p is not None and p.poll() is None:
                p.kill()
                p.wait(timeout=5)
        # 恢复原始选项；原来没有 candidate_window 则删除该键
        restore_opts = read_options()
        if had_candidate_window_key:
            restore_opts["candidate_window"] = orig_candidate_window
        else:
            restore_opts.pop("candidate_window", None)
        write_options(restore_opts)
        print("已恢复 options.json 原始 candidate_window 设置")


if __name__ == "__main__":
    sys.exit(main())
