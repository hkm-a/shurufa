#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""取样指定像素颜色。"""
import importlib.util
import sys

spec = importlib.util.spec_from_file_location('png_scan', 'scripts/png-scan.py')
png_scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(png_scan)

path = sys.argv[1]
w, h, ch, px = png_scan.decode_png(path)
for arg in sys.argv[2:]:
    x, y = [int(v) for v in arg.split(',')]
    i = y * w * ch + x * ch
    print(f'({x},{y}) = RGB({px[i]},{px[i+1]},{px[i+2]})')
