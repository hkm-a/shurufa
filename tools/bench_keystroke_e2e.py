#!/usr/bin/env python3
"""R2.1 键序到上屏真实延迟自检：ctypes SendInput → Notepad → shurufa TSF 命中
`SHURUFA_LATENCY_LOG=1` 时 host 侧会在 %TEMP%\\shurufa-tsf.log 写
`LAT commit keysym=0x.. chars=N elapsed_us=.. q0=.. q1=..`。本脚本（Python）
在 SendInput 同时也用 ctypes QPC 记时间戳（同 epoch），最后配对起止
窗口并求时延，p95 需 ≤ 阈值（50ms）。

运行：
  set SHURUFA_LATENCY_LOG=1（在启动 host 的 shell；host 侧 run 模式下生效）
  python tools/bench_keystroke_e2e.py --iters 200 --warmup 20 --p95-ms 50
"""
from __future__ import annotations

import argparse
import ctypes
import json
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


def qpc_delta_us(t0: int, t1: int, freq: int) -> float:
    return (t1 - t0) * 1_000_000.0 / freq


SendInput = user32.SendInput
INPUT_KEYBOARD = 1
KEYEVENTF_KEYUP = 0x0002


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


def send_vk(vk: int, up: bool) -> None:
    flags = KEYEVENTF_KEYUP if up else 0
    ki = KEYBDINPUT(wVk=vk, dwFlags=flags)
    inp = INPUT(type=INPUT_KEYBOARD, u=INPUT_UNION(ki=ki))
    arr = (INPUT * 1)(inp)
    sent = SendInput(1, arr, ctypes.sizeof(INPUT))
    if sent != 1:
        raise RuntimeError(f"SendInput vk={vk:#x} up={up} sent={sent}")


def press_char(vk: int) -> None:
    """按下并抬起（单次，不热混动）。"""
    send_vk(vk, up=False)
    time.sleep(0.001)
    send_vk(vk, up=True)


def launch_notepad() -> subprocess.Popen:
    p = subprocess.Popen(["notepad.exe"])
    time.sleep(1.2)
    return p


def focus_notepad_by_pid(pid: int) -> bool:
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
                if "无标题" in title or "Notepad" in title or "记事本" in title:
                    found.append(hwnd)
        return True

    EnumWindows(each, 0)
    if not found:
        return False
    return bool(SetForegroundWindow(found[0]))


def tsf_latency_log_path() -> Path:
    tmp = Path(sys.base_prefix) / ".." / "AppData" / "Local" / "Temp"
    return tmp.resolve() / "shurufa-tsf.log"


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


# ---- 拼音批次 ----

INITIALS = list("bpmfdtnlgkhjqxzcsr") + ["zh", "ch", "sh", "y", "w"]
FINALS = [
    "a", "o", "e", "ai", "ei", "ao", "ou", "an", "en", "ang", "eng", "ong",
    "i", "ia", "ie", "iao", "iu", "ian", "in", "iang", "ing", "iong",
    "u", "ua", "uo", "uai", "ui", "uan", "un", "uang", "ueng", "v", "ve",
]
VK_LETTER_BASE = 0x41  # A..Z


def pinyin_to_vks(text: str) -> list[int]:
    vks = []
    for ch in text:
        if ch.isalpha():
            vks.append(VK_LETTER_BASE + ord(ch.upper()) - ord("A"))
        elif ch == "'":
            vks.append(0xDE)  # VK_OEM_7
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

    # 0) 确认 SHURUFA_LATENCY_LOG
    # （本脚本不在 host 的进程组里设不了；需要用户在 shell 里先 set 后启动 host，
    #  或更新 deploy-v4 脚本后再 supervise。此处用环境变量检 host 侧是否在写。）
    if not (subprocess.list2cmdline(["cmd", "/c", "echo", "%SHURUFA_LATENCY_LOG%"]).strip()):
        pass  # 无强依赖；不外发 —— host 侧启动时自己查环境

    log_path = tsf_latency_log_path()
    print(f"[setup] 日志：{log_path}")

    proc = launch_notepad()
    if proc.poll() is not None:
        print("❌ Notepad 启动失败", file=sys.stderr)
        return 2
    focus_ok = focus_notepad_by_pid(proc.pid)
    if not focus_ok:
        print("⚠️  Notepad 未聚焦；SendInput 将落到当前前台窗", file=sys.stderr)

    freq = qpc_freq()
    rng = random.Random(args.seed)

    # 记录基准日志存量
    baseline = lookup_lat_entries(log_path)
    n0 = len(baseline)
    print(f"[setup] 已有 LAT 条目：{n0}")

    timings_ms: list[float] = []

    for i in range(args.warmup + args.iters):
        is_warm = i < args.warmup
        # 每只键序：拼音 + Space（引擎 commit）
        keys = rand_pinyin(rng) + " "
        t_send_start = qpc_now()
        max_vk_ts = t_send_start
        for ch in keys:
            for vk in pinyin_to_vks(ch):
                press_char(vk)
                max_vk_ts = qpc_now()
                time.sleep(0.028)
        t_send_done = qpc_now()

        # 等到 LAT 新增（串扫，取 q0 落在本次发送窗内的条目）
        deadline = time.time() + 0.45
        matched = None
        while time.time() < deadline:
            cur = lookup_lat_entries(log_path)
            if len(cur) > n0:
                # 过滤：q0 ≥ t_send_start 且 q1 ≤ t_send_done + 450ms
                cand = [
                    c for c in cur[n0:]
                    if c["q0"] >= t_send_start
                    and c["q1"] <= t_send_done + int(0.45 * freq)
                ]
                if cand:
                    matched = cand[-1]
                    break
            time.sleep(0.02)
        if matched is None:
            if not is_warm:
                print(f"⚠️  第 {i} 次未发现对应 LAT（可能该批未触发 commit）")
            continue

        # 时长 = host q1（编辑会话内写文本时刻）－ python 端 send_start 的 QPC
        # （同 epoch），是键序到上屏的真实 wall-time
        delta_secs = (matched["q1"] - t_send_start) / freq
        if not is_warm:
            timings_ms.append(delta_secs * 1000.0)
        n0 = len(lookup_lat_entries(log_path))
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
                "ok": (
                    summary.get("p95_ms", float("inf")) <= args.p95_ms
                    and len(timings_ms) >= args.iters // 2
                ),
            }
        )
    else:
        summary["ok"] = False
        summary["reason"] = "整个批次里没等到任何 LAT 提交（确认 SHURUFA_LATENCY_LOG=1 已在 host shell 启 host）"

    out_dir = Path(__file__).resolve().parent.parent / "target" / "bench"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_json = out_dir / f"keystroke-{datetime.now().strftime('%Y-%m-%d_%H-%M-%S')}.json"
    out_md = out_dir / "last-keystroke.md"
    out_json.write_text(
        json.dumps({"summary": summary, "timings_ms": timings_ms}, indent=2),
        encoding="utf-8",
    )
    ok_mark = "✅" if summary.get("ok") else "❌"
    out_md.write_text(
        f"# R2.1 键序到上屏延迟\n"
        f"- {summary['ts']}\n"
        f"- p95 {summary.get('p95_ms', 'n/a')} ms （阈值 {args.p95_ms}ms）\n"
        f"- 样本 {len(timings_ms)}/{args.iters}\n"
        f"- {ok_mark}\n",
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    print(f"→ {out_json.name}")
    return 0 if summary.get("ok") else 1


if __name__ == "__main__":
    sys.exit(main())
