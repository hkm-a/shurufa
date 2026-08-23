#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""候选窗 S4 验收环境预检 + 自动项执行。

用法：
    python scripts/test-cand-acceptance.py [--exe target/debug/shurufa-ui.exe]

功能：
1. 检查 Windows 环境（显示器数量、系统 DPI、pywinauto、构建产物）。
2. 自动执行 `test-cand-ui.py` 与 `test-cand-faults.py`。
3. 输出 S4 手动验收清单提醒。
"""
import argparse
import ctypes
import os
import subprocess
import sys
import time


def check_environment(exe: str):
    print("== 环境预检 ==")
    print(f"OS: {os.environ.get('OS', 'Windows')}")
    monitor_count = -1
    try:
        user32 = ctypes.windll.user32
        monitors = []

        @ctypes.WINFUNCTYPE(
            ctypes.c_int, ctypes.c_ulong, ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong), ctypes.c_double,
        )
        def callback(hmonitor, hdc, lprc, dwdata):
            monitors.append(hmonitor)
            return 1

        user32.EnumDisplayMonitors(0, 0, callback, 0)
        monitor_count = len(monitors)
        print(f"显示器数量: {monitor_count}")
        if monitor_count < 2:
            print("  [提醒] 当前只有 1 个显示器；多显示器用例需在扩展模式下补测")
    except Exception as exc:
        print(f"显示器检测失败：{exc}")
    try:
        dpi = ctypes.windll.user32.GetDpiForSystem()
        print(f"系统 DPI: {dpi}（{dpi * 100 // 96}% 缩放）")
    except Exception:
        print("系统 DPI: 无法读取")
    try:
        import pywinauto  # noqa: F401
        print("pywinauto: 已安装")
    except Exception:
        print("pywinauto: 未安装（请先 python -m pip install pywinauto）")
        return False, monitor_count
    if not os.path.isfile(exe):
        print(f"shurufa-ui.exe: 不存在 {exe}")
        return False, monitor_count
    print(f"shurufa-ui.exe: {exe}")
    return True, monitor_count


def run_script(script: str, exe: str) -> bool:
    print(f"\n== 自动执行 {script} ==")
    proc = subprocess.run([sys.executable, script, "--exe", exe], capture_output=True, text=True)
    print(proc.stdout, end="")
    if proc.stderr:
        print(proc.stderr, end="")
    print(f"exit={proc.returncode}")
    return proc.returncode == 0


def print_manual_checklist(monitor_count: int):
    print("\n== S4 手动验收清单 ==")
    if monitor_count <= 1:
        print("当前为单屏环境，先跑本机可测项：")
        checklist = [
            "A1-A5 hosted 基本链路（显示/点击/数字/翻页/Esc）",
            "B1-B3 杀 shurufa-ui 回退内置、重启恢复",
            "C1-C2 两个编辑器多客户端不串台",
            "E1 管理员应用 hosted 可用",
            "E2 全屏应用自动回退内置（可开一个最大化窗口近似验证）",
            "E4 锁屏/安全桌面",
            "E5 NVDA/讲述人读取候选文本（UIA Name）",
            "F1-F3 长时间稳定性/快速连击/反复重启",
        ]
        for item in checklist:
            print(f"  [ ] {item}")
        print("\n以下项当前单屏无法完整验证，后续有多屏/远程环境再补：")
        deferred = [
            "D1-D4 多显示器 + 不同 DPI 拖屏",
            "E3 RDP 远程桌面",
            "多屏下的候选窗跟随/坐标",
        ]
        for item in deferred:
            print(f"  [ ] {item}")
    else:
        checklist = [
            "A1-A5 hosted 基本链路（显示/点击/数字/翻页/Esc）",
            "B1-B3 杀 shurufa-ui 回退内置、重启恢复",
            "C1-C2 两个编辑器多客户端不串台",
            "D1-D4 多显示器 + 125%/150%/200% + 不同 DPI 拖屏",
            "E1 管理员应用 hosted 可用",
            "E2 全屏应用自动回退内置",
            "E3 RDP 远程桌面",
            "E4 锁屏/安全桌面",
            "E5 NVDA/讲述人读取候选文本（UIA Name）",
            "F1-F3 长时间稳定性/快速连击/反复重启",
        ]
        for item in checklist:
            print(f"  [ ] {item}")
    print("\n详细步骤见 docs/候选窗迁出宿主进程-验收矩阵.md")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--exe",
        default=r"target\debug\shurufa-ui.exe",
        help="shurufa-ui.exe 路径（默认 target\\debug\\shurufa-ui.exe）",
    )
    args = parser.parse_args()

    ok_env, monitor_count = check_environment(args.exe)
    if not ok_env:
        print("\n环境预检未通过，先补齐依赖。")
        return 1

    ok = True
    ok &= run_script("scripts/test-cand-ui.py", args.exe)
    ok &= run_script("scripts/test-cand-faults.py", args.exe)

    print_manual_checklist(monitor_count)
    print("\n自动项结果:", "全部通过" if ok else "存在失败")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
