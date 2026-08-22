#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""行剖面：扫描指定 y 带内颜色偏暗的 x 簇（找深色圆形 chips）。"""
import importlib.util
import sys

spec = importlib.util.spec_from_file_location('png_scan', 'scripts/png-scan.py')
png_scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(png_scan)

path = sys.argv[1]
y0, y1 = int(sys.argv[2]), int(sys.argv[3])
w, h, ch, px = png_scan.decode_png(path)
dark = {}
for y in range(y0, y1 + 1):
    for x in range(0, w, 2):
        i = y * w * ch + x * ch
        r, g, b = px[i], px[i + 1], px[i + 2]
        if r < 90 and g < 110 and b < 110:
            dark[x] = dark.get(x, 0) + 1
xs = sorted(dark)
clusters = []
s = None
last = None
for x in xs:
    if s is None:
        s = x
    elif x - last > 30:
        clusters.append((s + last) // 2)
        s = x
    last = x
if s is not None:
    clusters.append((s + last) // 2)
print('dark clusters x-centers in y', y0, '-', y1, ':', clusters)
