#!/usr/bin/env python3
"""Shurufa RSS 漏自检（R2.3）：以 `--once` 模式 1000 次逐键冷起动 algo，
以纯 ctypes `GetProcessMemoryInfo` 读 WorkingSetSize，观察冷起动 RSS
波动是否小于阈值（默认 25%），判定引擎冷起动链是否趋势泄漏。

注：shurufa-algo `--once` 模式每次启动都独立加载 librime + userdb，其 RSS
只反映当次冷起动序列；单进程长期增长的泄漏应交由始同一进程的
heap allocation 检测（下一步加 `tools/bench_algo_heap.py` 接续）。本脚本仅
先守住 **冷起动不趋势涨删** 的底线（不吞错、可读、1000 行循环能 8 分钟内跑完）。

退出码：0 = 通过 / 1 = 越界 / 2 = algo 缺失或不识别 --once。
"""
from __future__ import annotations

import argparse
import json
import random
import statistics
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
TARGET_DIR = REPO_ROOT / "target" / "bench"
TARGET_DIR.mkdir(parents=True, exist_ok=True)
ALGO_EXE = REPO_ROOT / "target" / "debug" / "shurufa-algo.exe"
# sanity-check 输出里必须出现的标志 --once 服务被识别
ONCE_OK_MARKER = "--once"


def rss_bytes(pid: int) -> int:
    import ctypes
    from ctypes import wintypes

    kernel32 = ctypes.windll.kernel32
    psapi = ctypes.windll.psapi

    class PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
        _fields_ = [
            ("cb", wintypes.DWORD),
            ("PageFaultCount", wintypes.DWORD),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
            ("PrivateUsage", ctypes.c_size_t),
        ]

    handle = kernel32.OpenProcess(0x1000, False, pid)
    if not handle:
        raise RuntimeError(f"OpenProcess({pid}) failed")
    try:
        counters = PROCESS_MEMORY_COUNTERS_EX()
        counters.cb = ctypes.sizeof(counters)
        if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
            raise RuntimeError(f"GetProcessMemoryInfo({pid}) failed")
        return int(counters.WorkingSetSize)
    finally:
        kernel32.CloseHandle(handle)


_INITIALS = list("bpmfdtnlgkhjqxzcsr") + ["zh", "ch", "sh", "y", "w"]
_FINALS = [
    "a", "o", "e", "ai", "ei", "ao", "ou", "an", "en", "ang", "eng", "ong",
    "i", "ia", "ie", "iao", "iu", "ian", "in", "iang", "ing", "iong",
    "u", "ua", "uo", "uai", "ui", "uan", "un", "uang", "ueng", "v", "ve",
]


def random_pinyin(rng: random.Random) -> str:
    return rng.choice(_INITIALS) + rng.choice(_FINALS)


def bench(iters: int, seed: int, threshold_pct: float) -> dict:
    rng = random.Random(seed)
    samples: list[int] = []
    start = time.perf_counter()
    fail_samples = 0
    for idx in range(iters):
        keys = random_pinyin(rng)
        pre = time.perf_counter()
        proc = subprocess.Popen(
            [str(ALGO_EXE), "--once", keys],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        rss_now = None
        try:
            # 允许它活一会完成一次驻留，读 RSS
            time.sleep(0.18)
            rss_now = rss_bytes(proc.pid)
            samples.append(rss_now)
            # 主动让进程退出（它本来就 `--once` 后会退；timeout 给硬上限）
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
                fail_samples += 1
        except Exception as e:
            # ctypes 拿不到个子（进程结束）也没事，把这次轮回跳
            pass
        _ = pre
    med = statistics.median(samples) if samples else 0
    p95 = (
        samples[int(len(samples) * 0.95)] if samples else 0
    )
    mn = min(samples) if samples else 0

    rss_range_pct = ((p95 - mn) / mn * 100.0) if mn else 0.0
    ok = bool(samples) and rss_range_pct <= threshold_pct and fail_samples == 0
    elapsed_s = time.perf_counter() - start
    result = {
        "kind": "shurufa-algo --once 冷起动 RSS 范围检",
        "iters_planned": iters,
        "iters_sampled": len(samples),
        "fail_samples": fail_samples,
        "elapsed_s": round(elapsed_s, 1),
        "rss_min_bytes": mn,
        "rss_p50_bytes": int(med),
        "rss_p95_bytes": int(p95),
        "rss_range_pct": round(rss_range_pct, 2),
        "threshold_pct": threshold_pct,
        "ok": ok,
        "seed": hex(seed),
        "ts": datetime.now().isoformat(),
        "out": str((TARGET_DIR / "").relative_to(REPO_ROOT)),
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser(prog="shurufa-bench")
    parser.add_argument("--iters", type=int, default=1000)
    parser.add_argument("--seed", type=int, default=0x5F3759DF)
    parser.add_argument("--threshold-pct", type=float, default=25.0,
                        help="冷起动 RSS p95-min / min 的允许百分比（默认 25%，搜狗级）")
    args = parser.parse_args()

    if not ALGO_EXE.exists():
        print(f"❌ 未找到 {ALGO_EXE}，请 cargo build -p shurufa-algo 后重试", file=sys.stderr)
        return 2

    # 先 sanity：单跑一次 `--once` 必须成功退出
    sanity = subprocess.run(
        [str(ALGO_EXE), "--once", "nihao"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=20,
    )
    if ONCE_OK_MARKER not in (sanity.stderr or "") and sanity.returncode != 0:
        print(
            f"❌ algo --once 自检失败：rc={sanity.returncode} 输出片段："
            f"{(sanity.stderr or sanity.stdout or '')[:400]!r}",
            file=sys.stderr,
        )
        return 2

    result = bench(args.iters, args.seed, args.threshold_pct)
    ts = datetime.now().strftime("%Y-%m-%d_%H-%M-%S")
    out_json = TARGET_DIR / f"leak-report-{ts}.json"
    out_json.write_text(json.dumps(result, indent=2, ensure_ascii=False), encoding="utf-8")
    print(json.dumps(result, indent=2, ensure_ascii=False))
    print(f"→ {out_json.relative_to(REPO_ROOT)}")
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())

