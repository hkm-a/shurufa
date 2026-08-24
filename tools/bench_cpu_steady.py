#!/usr/bin/env python3
"""R2.2 整词库候选窗稳态 CPU 自检。

测量对象：常驻 `shurufa-algo.exe` 服务（空闲/低负载）的 CPU 占用。
通过标准（docs/quality/输入法验收规范.md R2.2）：
  - 平均 % Processor Time < 1.5%
  - 无 5 秒以上持续 100% 毛刺

用法：
  python tools/bench_cpu_steady.py --minutes 2        # 快速冒烟（默认）
  python tools/bench_cpu_steady.py --minutes 30       # 完整验收
"""
from __future__ import annotations

import argparse
import json
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
PROC_NAME = "shurufa-algo.exe"


def find_existing() -> object | None:
    try:
        import psutil
    except ModuleNotFoundError:
        return None
    for p in psutil.process_iter(["name", "pid"]):
        try:
            if (p.info.get("name") or "").lower() == PROC_NAME:
                return p
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue
    return None


def main() -> int:
    ap = argparse.ArgumentParser(prog="shurufa-bench-cpu")
    ap.add_argument("--minutes", type=float, default=2.0,
                    help="采样分钟数（默认 2 分钟快速冒烟；完整验收用 30）")
    args = ap.parse_args()

    if not ALGO_EXE.exists() and find_existing() is None:
        print(f"❌ 未找到 {ALGO_EXE}，请先 cargo build -p shurufa-algo", file=sys.stderr)
        return 2

    spawned = None
    proc = find_existing()
    if proc is None:
        print(f"启动 {ALGO_EXE.name} serve 模式…")
        spawned = subprocess.Popen([str(ALGO_EXE)], cwd=str(REPO_ROOT))
        # 等管道/引擎初始化
        time.sleep(3)
        proc = find_existing()
        if proc is None:
            print("❌ algo 进程启动后未找到，可能被全局互斥挡掉", file=sys.stderr)
            return 2
        print(f"已启动 PID={proc.pid}")
    else:
        print(f"使用已运行的 algo PID={proc.pid}")

    try:
        import psutil
        p = psutil.Process(proc.pid)
    except Exception as e:
        print(f"❌ 无法附加进程：{e}", file=sys.stderr)
        if spawned is not None:
            spawned.kill()
        return 2

    # 预热 2 秒（librime 启动/部署完成后 CPU 才会回稳）
    time.sleep(2)

    duration = max(1, int(args.minutes * 60))
    samples: list[float] = []
    start = time.perf_counter()
    print(f"采样 {duration}s（每 1s 一次）…")
    for _ in range(duration):
        try:
            cpu = p.cpu_percent(interval=1)
            samples.append(cpu)
        except Exception:
            break

    if len(samples) < max(10, duration // 2):
        print("❌ 采样样本不足", file=sys.stderr)
        if spawned is not None:
            spawned.kill()
        return 2

    mean = statistics.fmean(samples)
    high = max(samples, default=0.0)
    spikes = sum(1 for s in samples if s >= 100.0)
    ok = mean < 1.5 and spikes == 0
    elapsed_s = time.perf_counter() - start

    result = {
        "kind": "R2.2 稳态 CPU",
        "pid": p.pid,
        "samples": len(samples),
        "elapsed_s": round(elapsed_s, 1),
        "cpu_mean_pct": round(mean, 2),
        "cpu_max_pct": round(high, 2),
        "spikes_100_pct": spikes,
        "ok": ok,
        "ts": datetime.now().isoformat(),
        "note": "平均 <1.5% 且无 100% 毛刺为通过；30 分钟为完整验收时长",
    }

    ts = datetime.now().strftime("%Y-%m-%d_%H-%M-%S")
    out = TARGET_DIR / f"cpu-steady-{ts}.json"
    out.write_text(json.dumps(result, indent=2, ensure_ascii=False), encoding="utf-8")
    print(json.dumps(result, indent=2, ensure_ascii=False))
    print(f"→ {out.relative_to(REPO_ROOT)}")

    if spawned is not None:
        spawned.kill()
        try:
            spawned.wait(timeout=5)
        except subprocess.TimeoutExpired:
            spawned.kill()

    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
