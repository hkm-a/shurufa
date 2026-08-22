#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""模拟器 IME 测试助手（实测坐标版）：chip y≈1724，x 按像素实测。
chip 序号：0=history 1=images 2=ai 3=scheme 4=settings 5=phrases 6=quick 7=emoji 8=calc
用法：python scripts/emu-tap.py <chip_idx>
"""
import subprocess
import sys

ADB = r'C:/Users/hkm/AppData/Local/Packages/Claude_pzs8sxrjxfjjc/LocalCache/Local/Android/Sdk/platform-tools/adb.exe'
SERIAL = 'emulator-5554'

CHIP_X = [69, 177, 285, 393, 501, 609, 717, 825, 933]
CHIP_Y = 1724


def main():
    idx = int(sys.argv[1])
    x, y = CHIP_X[idx], CHIP_Y
    print('tap', x, y)
    subprocess.run([ADB, '-s', SERIAL, 'shell', 'input', 'tap', str(x), str(y)])


if __name__ == '__main__':
    main()
