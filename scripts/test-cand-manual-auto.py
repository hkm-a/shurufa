#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""候选窗单屏手动项全自动验收（不需要你手动确认）。

用 Notepad4 + SendInput(scancode) + pywinauto 自动完成 A/B/C/E 单屏项：
- 自动设置 options.json 的 candidate_window=hosted；
- 自动启动 shurufa-ui（如未运行）；
- 自动把 Shurufa 切到中文模式；
- 自动打开 Notepad4、输入拼音、点击候选、读剪贴板验证上屏；
- 自动杀/重启 shurufa-ui 验证回退与恢复；
- 全程无需人工按键/确认。

注意：普通 pywinauto send_keys 走 Unicode 直插，TSF 收不到；
这里统一用 SendInput + KEYEVENTF_SCANCODE 发送真实键盘事件。

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
from ctypes import wintypes

import win32clipboard
import win32con
import win32file
import win32gui
import win32pipe
from pywinauto import Application, Desktop, findwindows

HOSTED_CLASS = "ShurufaCandWin"
BUILTIN_CLASS = "ShurufaCandidateWindow"

VK_SHIFT = 0x10
VK_CONTROL = 0x11
VK_ESCAPE = 0x1B
VK_DELETE = 0x2E
VK_A = 0x41
VK_C = 0x43


# ---------- SendInput(scancode) 键盘工具 ----------

ULONG_PTR = ctypes.c_ulonglong


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wintypes.WORD),
        ("wScan", wintypes.WORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ULONG_PTR),
    ]


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", ctypes.c_long),
        ("dy", ctypes.c_long),
        ("mouseData", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ULONG_PTR),
    ]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [("uMsg", wintypes.DWORD), ("wParamL", wintypes.WORD), ("wParamH", wintypes.WORD)]


class INPUTUNION(ctypes.Union):
    _fields_ = [("ki", KEYBDINPUT), ("mi", MOUSEINPUT), ("hi", HARDWAREINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", wintypes.DWORD), ("union", INPUTUNION)]


def send_key(vk: int, down: bool) -> None:
    """用扫描码发送真实键盘事件，TSF 才能收到。"""
    scan = ctypes.windll.user32.MapVirtualKeyW(vk, 0)
    flags = 0x0008  # KEYEVENTF_SCANCODE
    if not down:
        flags |= 0x0002  # KEYEVENTF_KEYUP
    inp = INPUT()
    inp.type = 1  # INPUT_KEYBOARD
    inp.union.ki.wVk = 0
    inp.union.ki.wScan = scan
    inp.union.ki.dwFlags = flags
    ctypes.windll.user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))


def tap(vk: int) -> None:
    send_key(vk, True)
    send_key(vk, False)
    time.sleep(0.03)


def type_scancode(text: str) -> None:
    for ch in text:
        if ch.isalpha():
            vk = ord(ch.upper())
        elif ch.isdigit():
            vk = ord(ch)
        else:
            continue
        tap(vk)
        time.sleep(0.03)


def ctrl_a() -> None:
    send_key(VK_CONTROL, True)
    tap(VK_A)
    send_key(VK_CONTROL, False)
    time.sleep(0.05)


def ctrl_c() -> None:
    send_key(VK_CONTROL, True)
    tap(VK_C)
    send_key(VK_CONTROL, False)
    time.sleep(0.05)


def press_shift() -> None:
    tap(VK_SHIFT)
    time.sleep(0.4)


# ---------- 选项与窗口工具 ----------


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
    hwnd = win.handle
    u = ctypes.windll.user32
    try:
        win.set_focus()
    except Exception:
        pass
    time.sleep(0.1)
    if u.GetForegroundWindow() == hwnd:
        return
    tid = u.GetWindowThreadProcessId(hwnd, None)
    fg = u.GetForegroundWindow()
    fg_tid = u.GetWindowThreadProcessId(fg, None)
    cur = ctypes.windll.kernel32.GetCurrentThreadId()
    u.AttachThreadInput(cur, tid, True)
    u.BringWindowToTop(hwnd)
    u.SetForegroundWindow(hwnd)
    u.AttachThreadInput(cur, tid, False)
    time.sleep(0.2)


def make_fullscreen(win):
    hwnd = win.handle
    u = ctypes.windll.user32
    vx = u.GetSystemMetrics(76)   # SM_XVIRTUALSCREEN
    vy = u.GetSystemMetrics(77)   # SM_YVIRTUALSCREEN
    vw = u.GetSystemMetrics(78)   # SM_CXVIRTUALSCREEN
    vh = u.GetSystemMetrics(79)   # SM_CYVIRTUALSCREEN
    u.SetWindowPos(hwnd, 0, vx, vy, vw, vh, 0x0004)  # SWP_NOZORDER
    time.sleep(0.3)


def restore_window(win, rect):
    hwnd = win.handle
    u = ctypes.windll.user32
    u.SetWindowPos(
        hwnd,
        0,
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
        0x0004,  # SWP_NOZORDER
    )
    time.sleep(0.3)


def launch_editor(path):
    proc = subprocess.Popen([path])
    app = Application(backend="win32").connect(process=proc.pid, timeout=15)
    win = app.top_window()
    set_focus(win)
    time.sleep(0.5)
    return proc, win


def visible_candidate_count():
    n = 0
    for cls in (HOSTED_CLASS, BUILTIN_CLASS):
        for h in find_class(cls, visible_only=False):
            try:
                if Desktop(backend="win32").window(handle=h).is_visible():
                    n += 1
            except Exception:
                pass
    return n


def ensure_chinese_mode(win) -> bool:
    """通过探测按键把 Shurufa 切到中文模式。"""
    set_focus(win)
    for _ in range(4):
        type_scancode("a")
        time.sleep(0.5)
        if visible_candidate_count() > 0:
            tap(VK_ESCAPE)
            time.sleep(0.2)
            return True
        # 英文模式下 'a' 已直接上屏，清掉再切换
        ctrl_a()
        tap(VK_DELETE)
        time.sleep(0.2)
        press_shift()
    return False


def type_text(win, text):
    set_focus(win)
    type_scancode(text)
    time.sleep(0.8)


def read_editor_text(win):
    set_focus(win)
    ctrl_a()
    ctrl_c()
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
    # 等窗口标题出现「你好」再点，避免布局尚未刷新
    deadline = time.time() + 3.0
    while time.time() < deadline and "你好" not in window_title(hwnd):
        time.sleep(0.05)
    rect = win32gui.GetWindowRect(hwnd)
    # 用真实鼠标点击 hosted 首项（首项在 preedit 之后，取窗口内 x≈90）
    screen_x = rect[0] + 90
    screen_y = (rect[1] + rect[3]) // 2
    user32 = ctypes.windll.user32
    user32.SetCursorPos(screen_x, screen_y)
    time.sleep(0.1)
    user32.mouse_event(0x0002, 0, 0, 0, 0)  # MOUSEEVENTF_LEFTDOWN
    user32.mouse_event(0x0004, 0, 0, 0, 0)  # MOUSEEVENTF_LEFTUP
    time.sleep(1.0)
    return True


def is_ui_running():
    return len(find_class("ShurufaUiHost", visible_only=False)) > 0


def pipe_ready() -> bool:
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
        handle.Close()
        return True
    except Exception:
        return False


def start_ui(exe):
    return subprocess.Popen([exe], stderr=subprocess.PIPE, text=True)


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

    orig_opts = read_options()
    orig_candidate_window = orig_opts.get("candidate_window")
    had_candidate_window_key = "candidate_window" in orig_opts
    print("设置 candidate_window=hosted")
    set_candidate_window("hosted", dict(orig_opts))

    results = []

    def record(name: str, ok: bool):
        results.append((name, ok))
        print(("PASS" if ok else "FAIL") + f" | {name}")

    proc = None
    proc2 = None
    ui_proc = None
    try:
        if not is_ui_running() or not pipe_ready():
            if is_ui_running():
                print("shurufa-ui 进程在但管道异常，先重启")
                stop_ui()
            print(f"启动 shurufa-ui: {args.exe}")
            ui_proc = start_ui(args.exe)
            deadline = time.time() + 8
            while time.time() < deadline:
                if is_ui_running() and pipe_ready():
                    print("shurufa-ui 已就绪")
                    break
                if ui_proc.poll() is not None:
                    print("shurufa-ui 启动失败/超时，后续 hosted 用例可能失败")
                    break
                time.sleep(0.1)
        else:
            print("shurufa-ui 已在运行且管道就绪")

        proc, win = launch_editor(editor)
        ensure_chinese_mode(win)
        type_text(win, "nihao")
        hosted = find_class(HOSTED_CLASS)
        record("A1 hosted 候选窗出现且含「你好」", len(hosted) > 0 and any("你好" in window_title(h) for h in hosted))

        click_first_candidate()
        record("A3 点击候选后上屏「你好」", wait_text(win, "你好"))

        # A4 数字选词
        type_text(win, "nihao")  # 先清掉旧组合/文本由 Esc 处理
        tap(VK_ESCAPE)
        type_text(win, "nihao")
        tap(ord("1"))
        record("A4 数字选词后上屏正常", wait_text(win, "你好"))

        # A5 Esc 取消
        type_text(win, "nihao")
        tap(VK_ESCAPE)
        time.sleep(0.5)
        visible = [h for h in find_class(HOSTED_CLASS) if Desktop(backend="win32").window(handle=h).is_visible()]
        record("A5 Esc 后 hosted 候选窗隐藏", len(visible) == 0)

        # B1 杀 ui：内置绘制已删除，候选窗消失但输入不中断
        stop_ui()
        type_text(win, "nihao")
        record(
            "B1 杀 ui 后无 hosted/内置候选窗且不崩溃",
            len(find_class(HOSTED_CLASS)) == 0 and len(find_class(BUILTIN_CLASS)) == 0,
        )

        # B2 重启 ui 恢复 hosted
        ui_proc = start_ui(args.exe)
        deadline = time.time() + 8
        while time.time() < deadline and not pipe_ready():
            time.sleep(0.1)
        time.sleep(0.5)
        type_text(win, "nihao")
        record("B2 重启 ui 后恢复 hosted", len(find_class(HOSTED_CLASS)) > 0)

        # C1 多客户端：第二个编辑器能独立获得 hosted 候选窗
        # （两个窗口同时可见由 test-cand-faults.py 用伪客户端覆盖；
        #  真实焦点切换时前一个编辑器会先隐藏候选，这是正常 TSF 行为）
        proc2, win2 = launch_editor(editor)
        ensure_chinese_mode(win2)
        type_text(win2, "nihao")
        time.sleep(0.5)
        hosted = find_class(HOSTED_CLASS)
        record(
            "C1 第二编辑器 hosted 候选窗出现且含「你好」",
            len(hosted) >= 1 and any("你好" in window_title(h) for h in hosted),
        )

        # E2 全屏/最大化：内置已删除，hosted 也不推送，候选窗应全部消失
        original_rect = win.rectangle()
        make_fullscreen(win)
        set_focus(win)
        type_text(win, "nihao")
        time.sleep(0.5)
        builtin = find_class(BUILTIN_CLASS)
        hosted = find_class(HOSTED_CLASS)
        record("E2 最大化时 hosted/内置候选窗均不显示", len(builtin) == 0 and len(hosted) == 0)

        # E5 候选文本可读
        restore_window(win, original_rect)
        set_focus(win)
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
        restore_opts = read_options()
        if had_candidate_window_key:
            restore_opts["candidate_window"] = orig_candidate_window
        else:
            restore_opts.pop("candidate_window", None)
        write_options(restore_opts)
        print("已恢复 options.json 原始 candidate_window 设置")


if __name__ == "__main__":
    sys.exit(main())
