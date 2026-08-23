#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""候选窗单屏手动项“半自动引导”验收。

自动项跑完后，用本脚本引导你逐项完成单屏可测的手动验收：
- 它会告诉你“打开 Notepad4、输入 nihao、按回车”，然后自动检查候选窗。
- 有些步骤需要你肉眼确认（如是否上屏正确），输入 y/n。

用法：
    python scripts/test-cand-manual-guided.py [--exe target/debug/shurufa-ui.exe]
"""
import argparse
import subprocess
import sys
import time

import ctypes
from pywinauto import findwindows

HOSTED_CLASS = "ShurufaCandWin"
BUILTIN_CLASS = "ShurufaCandidateWindow"


def ask(question: str) -> bool:
    while True:
        answer = input(f"{question} (y/n): ").strip().lower()
        if answer in ("y", "yes"):
            return True
        if answer in ("n", "no"):
            return False


def wait_enter(hint: str):
    input(f"\n{hint}\n完成后按 Enter 继续...")


def find_class(class_name: str):
    return findwindows.find_windows(class_name=class_name)


def window_title(hwnd) -> str:
    buf = ctypes.create_unicode_buffer(512)
    ctypes.windll.user32.GetWindowTextW(hwnd, buf, 512)
    return buf.value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--exe",
        default=r"target\debug\shurufa-ui.exe",
        help="shurufa-ui.exe 路径（默认 target\\debug\\shurufa-ui.exe）",
    )
    args = parser.parse_args()

    results = []

    def record(name: str, ok: bool):
        results.append((name, ok))
        print(("PASS" if ok else "FAIL") + f" | {name}")

    print("========== 候选窗单屏手动验收（引导模式） ==========")
    print("请按提示操作；每步完成后按 Enter。")

    # A1 hosted 显示
    wait_enter("1. 请打开 Notepad4，输入 nihao，应出现 hosted 候选窗")
    hosted = find_class(HOSTED_CLASS)
    ok = len(hosted) > 0 and any("你好" in window_title(h) for h in hosted)
    record("A1 hosted 候选窗显示且含「你好」", ok)

    # A3 点击选词
    if ok:
        wait_enter("2. 请用鼠标点击候选窗里的「你好」")
        record("A3 点击选词（请自行确认已上屏）", ask("是否已上屏为「你好」？"))
    else:
        record("A3 点击选词", False)

    # A4 数字选词/翻页
    wait_enter("3. 请用数字键 1 或 2 选词，再试试 PageDown/PageUp 翻页")
    record("A4 数字选词/翻页（请自行确认行为正常）", ask("数字选词和翻页是否正常？"))

    # A5 Esc
    wait_enter("4. 请按 Esc 取消组合")
    time.sleep(0.5)
    # hosted 窗口可能隐藏而非销毁；这里只做提示确认
    record("A5 Esc 取消后候选窗隐藏", ask("按 Esc 后候选窗是否隐藏/消失？"))

    # B1 杀 ui 回退内置
    print("\n下一步会自动杀掉 shurufa-ui.exe，然后你在 Notepad4 继续输入 nihao。")
    input("准备好了按 Enter...")
    subprocess.run(["taskkill", "/F", "/IM", "shurufa-ui.exe"], capture_output=True)
    time.sleep(0.5)
    wait_enter("5. 现在请在 Notepad4 再输入 nihao，应回退为内置候选窗")
    builtin = find_class(BUILTIN_CLASS)
    record("B1 杀掉 ui 后回退内置候选窗", len(builtin) > 0)

    # B2 重启 ui 恢复 hosted
    subprocess.Popen([args.exe])
    time.sleep(1.0)
    wait_enter("6. 请再输入 nihao，应恢复为 hosted 候选窗")
    hosted = find_class(HOSTED_CLASS)
    record("B2 重启 ui 后恢复 hosted 候选窗", len(hosted) > 0)

    # C1 两个编辑器
    wait_enter("7. 请打开两个 Notepad4，分别输入拼音，应出现两个 hosted 候选窗")
    hosted = find_class(HOSTED_CLASS)
    record("C1 两个编辑器多客户端不串台", len(hosted) >= 2)

    # E1 管理员 Notepad4
    wait_enter("8. 请用管理员身份打开 Notepad4，输入 nihao")
    hosted = find_class(HOSTED_CLASS)
    record("E1 管理员应用 hosted 可用", len(hosted) > 0)

    # E2 全屏回退
    wait_enter("9. 请把窗口最大化/全屏（或开一个全屏视频），输入 nihao")
    builtin = find_class(BUILTIN_CLASS)
    hosted = find_class(HOSTED_CLASS)
    record("E2 全屏/最大化时回退内置", len(builtin) > 0 and len(hosted) == 0)

    # E5 UIA/标题可读
    hosted = find_class(HOSTED_CLASS)
    titles_ok = any("你好" in window_title(h) for h in hosted)
    record("E5 候选文本可被自动化读取（标题/UIA）", titles_ok)

    print("\n========== 手动验收汇总 ==========")
    failed = [name for name, ok in results if not ok]
    for name, ok in results:
        print(("PASS" if ok else "FAIL") + f" | {name}")
    if failed:
        print(f"未通过 {len(failed)} 项：{', '.join(failed)}")
        return 1
    print("全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
