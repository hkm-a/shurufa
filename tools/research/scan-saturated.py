#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""通用饱和色行扫描：定位彩色按钮（功能行 chips 等）。"""
import importlib.util
import sys

spec = importlib.util.spec_from_file_location('png_scan', 'scripts/png-scan.py')
png_scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(png_scan)

path = sys.argv[1]
w, h, ch, px = png_scan.decode_png(path)
rows = {}
for y in range(h):
    c = 0
    for x in range(0, w, 3):
        i = y * w * ch + x * ch
        r, g, b = px[i], px[i + 1], px[i + 2]
        if max(r, g, b) - min(r, g, b) > 90 and max(r, g, b) > 90:
            c += 1
    if c > 8:
        rows[y] = c
bands = []
s = None
last = None
for y in sorted(rows):
    if s is None:
        s = y
    elif y - last > 14:
        bands.append((s, last))
        s = y
    last = y
if s is not None:
    bands.append((s, last))
print('size', w, h, 'saturated bands:', bands)
