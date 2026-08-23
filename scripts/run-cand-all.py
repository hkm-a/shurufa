#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""候选窗全量自动化一键跑。

用法：
    python scripts/run-cand-all.py [--exe target/debug/shurufa-ui.exe]

依次执行：
1. cargo test -p ime-ipc
2. cargo test -p windows-ipc --test cand_e2e
3. scripts/test-cand-ui.py
4. scripts/test-cand-interact.py
5. scripts/test-cand-faults.py
6. scripts/test-cand-stability.py
7. scripts/test-cand-disconnect.py

任一失败立即停止并返回非 0。
"""
import argparse
import subprocess
import sys


def run(cmd, title):
    print(f"\n===== {title} =====")
    proc = subprocess.run(cmd, capture_output=True, text=True)
    print(proc.stdout, end="")
    if proc.stderr:
        print(proc.stderr, end="")
    ok = proc.returncode == 0
    print(f"--- {title}: {'PASS' if ok else 'FAIL'} (exit={proc.returncode}) ---")
    return ok


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--exe",
        default=r"target\debug\shurufa-ui.exe",
        help="shurufa-ui.exe 路径（默认 target\\debug\\shurufa-ui.exe）",
    )
    args = parser.parse_args()

    steps = [
        (["cargo", "test", "-p", "ime-ipc"], "Rust ime-ipc 单测"),
        (["cargo", "test", "-p", "windows-ipc", "--test", "cand_e2e"], "Rust cand 管道 e2e"),
        ([sys.executable, "scripts/test-cand-ui.py", "--exe", args.exe], "UI 语义冒烟"),
        ([sys.executable, "scripts/test-cand-interact.py", "--exe", args.exe], "交互（点击/翻页）"),
        ([sys.executable, "scripts/test-cand-faults.py", "--exe", args.exe], "故障注入"),
        ([sys.executable, "scripts/test-cand-stability.py", "--exe", args.exe], "稳定性"),
        ([sys.executable, "scripts/test-cand-disconnect.py", "--exe", args.exe], "断开清理"),
    ]

    failed = []
    for cmd, title in steps:
        if not run(cmd, title):
            failed.append(title)

    print("\n===== 汇总 =====")
    if failed:
        for title in failed:
            print(f"FAIL: {title}")
        return 1
    print("全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
