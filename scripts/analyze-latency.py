#!python3
"""分析 shurufa-tsf.log 里的按键→上屏延迟（LAT 行），输出分布统计。

2026-08-16 起 LAT 打点默认开启（写 %TEMP%\\shurufa-tsf.log，走内存缓冲
批量落盘，零热路径 I/O）。卡顿排查时用本脚本看延迟分布：

    python scripts\\analyze-latency.py                # 全部 LAT 行
    python scripts\\analyze-latency.py --last 500     # 只看最近 500 次
    python scripts\\analyze-latency.py --min-ms 50    # 只列超过 50ms 的尖峰

输出：p50/p90/p95/p99/max + 尖峰明细（时间戳、按键、耗时）。
依赖：python stdlib。
"""
import argparse
import os
import re
import statistics
import sys
from datetime import datetime, timezone

LAT_RE = re.compile(
    r"\[(\d+)\] .*LAT commit keysym=0x([0-9A-Fa-f]+) chars=(\d+) "
    r"elapsed_us=(\d+)"
)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--log", default=os.path.join(
        os.environ.get("TEMP", "."), "shurufa-tsf.log"))
    ap.add_argument("--last", type=int, default=None,
                    help="只统计最近 N 次上屏")
    ap.add_argument("--min-ms", type=float, default=None,
                    help="只列出超过该毫秒数的尖峰")
    args = ap.parse_args()

    if not os.path.isfile(args.log):
        print(f"日志不存在：{args.log}", file=sys.stderr)
        return 2

    rows = []  # (ts_sec, keysym, chars, elapsed_us)
    with open(args.log, encoding="utf-8", errors="replace") as f:
        for line in f:
            m = LAT_RE.search(line)
            if m:
                rows.append((
                    int(m.group(1)),
                    int(m.group(2), 16),
                    int(m.group(3)),
                    int(m.group(4)),
                ))

    if not rows:
        print("日志中没有 LAT 行（可能尚未启用打点，或该版本未写）")
        return 0

    if args.last:
        rows = rows[-args.last:]

    us = [r[3] for r in rows]
    us_sorted = sorted(us)
    n = len(us)
    p = lambda q: us_sorted[min(n - 1, int(n * q))]

    print(f"共 {n} 次上屏延迟统计（单位 ms）：")
    print(f"  min   = {us_sorted[0] / 1000:.2f}")
    print(f"  p50   = {p(0.50) / 1000:.2f}")
    print(f"  p90   = {p(0.90) / 1000:.2f}")
    print(f"  p95   = {p(0.95) / 1000:.2f}")
    print(f"  p99   = {p(0.99) / 1000:.2f}")
    print(f"  max   = {us_sorted[-1] / 1000:.2f}")
    print(f"  mean  = {statistics.fmean(us) / 1000:.2f}")

    if args.min_ms is not None:
        spikes = [r for r in rows if r[3] / 1000 >= args.min_ms]
        if spikes:
            print(f"\n超过 {args.min_ms}ms 的尖峰（{len(spikes)} 次）：")
            for ts, keysym, chars, us in spikes[-20:]:
                when = datetime.fromtimestamp(ts, tz=timezone.utc).strftime("%m-%d %H:%M:%S")
                ch = chr(keysym) if 32 <= keysym < 127 else f"0x{keysym:X}"
                print(f"  {when} UTC 键={ch!r} 上屏{chars}字 耗时={us / 1000:.1f}ms")
    return 0


if __name__ == "__main__":
    sys.exit(main())
