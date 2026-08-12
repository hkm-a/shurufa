#!/usr/bin/env python3
"""R2.1 键序到上屏真实延迟自检：ctypes SendInput → **Notepad4**，shurufa TSF 打点
`SHURUFA_LATENCY_LOG=1` 时 host 侧会在 %TEMP%\\shurufa-tsf.log 写
`LAT commit keysym=0x.. chars=N elapsed_us=.. q0=.. q1=..`。本脚本在 SendInput
同时也用 ctypes QPC 记时间戳（同 epoch），配对求差以 p95 门槛。

用法（依赖用户已安装 Notepad4；Windows 系统上不再保证 `notepad.exe` 存在）：
  set SHURUFA_LATENCY_LOG=1  # 在拉起 host 的 shell 里先设再启 host
  python tools/bench_keystroke_e2e.py --warmup 10 --iters 150 --p95-ms 50
"""
from __future__ import annotations

import argparse
import ctypes
import json
import os
import random
import statistics
import subprocess
import sys
import time
from ctypes import wintypes
from datetime import datetime
from pathlib import Path

kernel32 = ctypes.windll.kernel32
user32 = ctypes.windll.user32

_qpc = kernel32.QueryPerformanceCounter
_qpc.argtypes = [ctypes.POINTER(ctypes.c_int64)]
_qpc.restype = wintypes.BOOL
_qpcf = kernel32.QueryPerformanceFrequency
_qpcf.argtypes = [ctypes.POINTER(ctypes.c_int64)]
_qpcf.restype = wintypes.BOOL


def qpc_now() -> int:
    v = ctypes.c_int64()
    _qpc(ctypes.byref(v))
    return v.value


def qpc_freq() -> int:
    v = ctypes.c_int64()
    _qpcf(ctypes.byref(v))
    return v.value


# ---- 键盘事件（keybd_event；SendInput 被 Notepad4 UIPI 隔离阻断时用回退）----
# SendInput 在某些桌面应用（比如 arm64 UWP、提权应用）会被完整性级别拦截，
# 报 sent=0。ctypes keybd_event 是遗留 API，目标同进程组，避免 UIPI 卡死。
keybd_event = user32.keybd_event
keybd_event.argtypes = [wintypes.BYTE, wintypes.BYTE, wintypes.DWORD, ctypes.c_void_p]
keybd_event.restype = None
KEYEVENTF_KEYUP = 0x0002

INPUT_KEYBOARD = 1
KEYEVENTF_SCANCODE = 0x0008
KEYEVENTF_KEYUP_UP = 0x0002
KEYEVENTF_SCANCODE_BELOW = 0x0008
KEYEVENTF_UNICODE = 0x0004


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wintypes.WORD),
        ("wScan", wintypes.WORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class INPUT_UNION(ctypes.Union):
    _fields_ = [("ki", KEYBDINPUT)]


class INPUT(ctypes.Structure):
    _anonymous_ = ("u",)
    _fields_ = [("type", wintypes.DWORD), ("u", INPUT_UNION)]


SendInput = user32.SendInput
SendInput.argtypes = [wintypes.UINT, ctypes.c_void_p, ctypes.c_int]
SendInput.restype = wintypes.UINT


def send_vk(vk: int, up: bool) -> None:
    flags = KEYEVENTF_KEYUP if up else 0
    ki = KEYBDINPUT(wVk=vk, dwFlags=flags)
    inp = INPUT(type=INPUT_KEYBOARD, u=INPUT_UNION(ki=ki))
    arr = (INPUT * 1)(inp)
    sent = SendInput(1, arr, ctypes.sizeof(INPUT))
    if sent != 1:
        # UIPI 被拦：回退 keybd_event
        keybd_event(vk, 0, KEYEVENTF_KEYUP if up else 0, None)


def press_char(vk: int) -> None:
    send_vk(vk, up=False)
    time.sleep(0.001)
    send_vk(vk, up=True)


# ---- Notepad4 ----

NOTEPAD_CANDIDATES = [
    Path(r"C:\Users\hkm\AppData\Local\Microsoft\WinGet\Links\Notepad4.exe"),
    Path(r"C:\Program Files\Notepad4\Notepad4.exe"),
    Path(r"C:\Windows\System32\notepad.exe"),
    Path(r"C:\Windows\notepad.exe"),
]


def find_editor_exe() -> Path | None:
    for p in NOTEPAD_CANDIDATES:
        if p.exists():
            return p
    # 让 PATH 一阵（含 winget links）
    try:
        out = subprocess.check_output(
            ["where", "Notepad4.exe"], text=True, encoding="utf-8", errors="replace"
        ).strip()
        if out:
            return Path(out.splitlines()[0].strip())
    except (OSError, subprocess.CalledProcessError):
        pass
    try:
        out = subprocess.check_output(
            ["where", "notepad.exe"], text=True, encoding="utf-8", errors="replace"
        ).strip()
        if out:
            return Path(out.splitlines()[0].strip())
    except (OSError, subprocess.CalledProcessError):
        pass
    return None


def focus_editor_by_pid(pid: int, match_titles: list[str]) -> bool:
    EnumWindows = user32.EnumWindows
    GetWindowThreadProcessId = user32.GetWindowThreadProcessId
    IsWindowVisible = user32.IsWindowVisible
    SetForegroundWindow = user32.SetForegroundWindow
    GetWindowTextW = user32.GetWindowTextW
    GetWindowTextLengthW = user32.GetWindowTextLengthW

    found = []
    CBF = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    @CBF
    def each(hwnd, _):
        if not IsWindowVisible(hwnd):
            return True
        wpid = wintypes.DWORD()
        GetWindowThreadProcessId(hwnd, ctypes.byref(wpid))
        if wpid.value == pid:
            length = GetWindowTextLengthW(hwnd)
            if length > 0:
                buf = ctypes.create_unicode_buffer(length + 1)
                GetWindowTextW(hwnd, buf, length + 1)
                title = buf.value
                if any(t in title for t in match_titles):
                    found.append(hwnd)
        return True

    EnumWindows(each, 0)
    if not found:
        return False
    return bool(SetForegroundWindow(found[0]))


def tsf_latency_log_path() -> Path:
    # 与 TSF debug_log 同一解析路径：%TEMP%\shurufa-tsf.log
    tmp = os.environ.get("TEMP") or os.environ.get("TMP")
    if tmp:
        return Path(tmp).resolve() / "shurufa-tsf.log"
    return Path(os.path.expandvars(r"%LOCALAPPDATA%\Temp")).resolve() / "shurufa-tsf.log"


def lookup_lat_entries(log_path: Path) -> list[dict]:
    if not log_path.exists():
        return []
    out = []
    for line in log_path.read_text(encoding="utf-8", errors="replace").splitlines():
        if "LAT commit" not in line:
            continue
        try:
            rest = line.split("LAT commit", 1)[1]
            kv = dict(tok.split("=", 1) for tok in rest.split() if "=" in tok)
            out.append(
                {
                    "chars": int(kv.get("chars", "0")),
                    "elapsed_us": float(kv.get("elapsed_us", "0")),
                    "q0": int(kv.get("q0", "0")),
                    "q1": int(kv.get("q1", "0")),
                }
            )
        except (ValueError, IndexError, KeyError):
            continue
    return out


INITIALS = list("bpmfdtnlgkhjqxzcsr") + ["zh", "ch", "sh", "y", "w"]
FINALS = [
    "a", "o", "e", "ai", "ei", "ao", "ou", "an", "en", "ang", "eng", "ong",
    "i", "ia", "ie", "iao", "iu", "ian", "in", "iang", "ing", "iong",
    "u", "ua", "uo", "uai", "ui", "uan", "un", "uang", "ueng",
    "v", "ve",
]
VK_LETTER_BASE = 0x41  # A..Z


def pinyin_to_vks(text: str) -> list[int]:
    vks = []
    for ch in text:
        if ch.isalpha():
            vks.append(VK_LETTER_BASE + ord(ch.upper()) - ord("A"))
        elif ch == "'":
            vks.append(0xDE)  # VK_OEM_7
        elif ch == " ":
            vks.append(0x20)  # VK_SPACE
        else:
            raise ValueError(f"不支持字符：{ch}")
    return vks


def rand_pinyin(rng: random.Random) -> str:
    return rng.choice(INITIALS) + rng.choice(FINALS)


def main() -> int:
    ap = argparse.ArgumentParser(prog="bench_keystroke_e2e")
    ap.add_argument("--warmup", type=int, default=10)
    ap.add_argument("--iters", type=int, default=150)
    ap.add_argument("--p95-ms", type=float, default=50.0)
    ap.add_argument("--seed", type=int, default=20260811)
    args = ap.parse_args()

    notepad = find_editor_exe()
    if not notepad:
        print("❌ 未找到 Notepad4 / notepad.exe", file=sys.stderr)
        return 2

    print(f"[setup] 编辑器：{notepad}")
    print(f"[setup] host 侧 SHURUFA_LATENCY_LOG=1 需要在拉起 host 的 shell 里先设")

    log_path = tsf_latency_log_path()
    baseline = lookup_lat_entries(log_path)
    n0 = len(baseline)
    print(f"[setup] 现有 LAT 条目：{n0}")

    proc = subprocess.Popen([str(notepad)])
    time.sleep(1.8)

    if not focus_editor_by_pid(proc.pid, ["Notepad4", "notepad", "无标题", "记事本"]):
        print("⚠️  编辑器未聚焦", file=sys.stderr)

    freq = qpc_freq()
    rng = random.Random(args.seed)

    timings_ms: list[float] = []
    for i in range(args.warmup + args.iters):
        is_warm = i < args.warmup
        # 键序：拼音 + 空格（拼音→空格触发 commit）
        keys = rand_pinyin(rng) + " "
        t_send_start = qpc_now()
        max_vk_ts = t_send_start
        for ch in keys:
            for vk in pinyin_to_vks(ch):
                press_char(vk)
                max_vk_ts = qpc_now()
                time.sleep(0.028)
        t_send_done = qpc_now()

        # 等 LAT 行出现且落在本次发送窗内
        deadline = time.time() + 0.6
        matched = None
        while time.time() < deadline:
            cur = lookup_lat_entries(log_path)
            if len(cur) > n0:
                cand = [
                    c for c in cur[n0:]
                    if c["q0"] >= t_send_start
                    and c["q1"] <= t_send_done + int(0.6 * freq)
                ]
                if cand:
                    matched = cand[-1]
                    break
            time.sleep(0.03)
        n0 = len(lookup_lat_entries(log_path))
        if matched is None:
            if not is_warm:
                print(f"⚠️  第 {i} 次未等到 LAT（可能该键序未产生 commit）")
            time.sleep(0.05)
            continue

        # 键到上屏延迟 = host q1 - python 发送起点 QPC（同 epoch）
        delta_us = (matched["q1"] - t_send_start) * 1_000_000.0 / freq
        if not is_warm:
            timings_ms.append(delta_us / 1000.0)
        time.sleep(0.05)

    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    summary = {
        "iters": args.iters,
        "warmup": args.warmup,
        "measured": len(timings_ms),
        "seed": hex(args.seed),
        "editor": str(notepad),
        "ts": datetime.now().isoformat(),
    }
    if timings_ms:
        timings_sorted = sorted(timings_ms)
        summary.update(
            {
                "p50_ms": round(statistics.median(timings_sorted), 3),
                "p95_ms": round(
                    timings_sorted[max(0, int(len(timings_sorted) * 0.95) - 1)], 3
                ),
                "p99_ms": round(
                    timings_sorted[max(0, int(len(timings_sorted) * 0.99) - 1)], 3
                ),
                "mean_ms": round(statistics.mean(timings_sorted), 3),
                "min_ms": round(timings_sorted[0], 3),
                "max_ms": round(timings_sorted[-1], 3),
                "threshold_ms": args.p95_ms,
                "ok": summary.get("p95_ms", 1e9) <= args.p95_ms
                and len(timings_ms) >= args.iters // 2,
            }
        )
    else:
        summary["ok"] = False
        summary["reason"] = "未等到任何 LAT 条目；请确认 SHURUFA_LATENCY_LOG=1 已生效"

    out_dir = Path(__file__).resolve().parent.parent / "target" / "bench"
    out_dir.mkdir(parents=True, exist_ok=True)
    stem = f"keystroke-{datetime.now().strftime('%Y-%m-%d_%H-%M-%S')}"
    out_json = out_dir / f"{stem}.json"
    out_md = out_dir / "last-keystroke.md"
    out_json.write_text(
        json.dumps({"summary": summary, "timings_ms": timings_ms}, indent=2, ensure_ascii=False),
        encoding="utf-8",
    )
    ok = "✅" if summary.get("ok") else "❌"
    out_md.write_text(
        f"# R2.1 键序到上屏延迟\n"
        f"- {summary['ts']}\n"
        f"- 编辑器: {summary['editor']}\n"
        f"- 样本 {len(timings_ms)}/{args.iters}\n"
        f"- p50={summary.get('p50_ms', 'n/a')}ms p95={summary.get('p95_ms', 'n/a')}ms p99={summary.get('p99_ms', 'n/a')}ms\n"
        f"- {ok}\n",
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    print(f"→ {out_json.name}")
    return 0 if summary.get("ok") else 1


if __name__ == "__main__":
    sys.exit(main())
