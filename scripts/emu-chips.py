#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""定位截图中的绿色圆钮（工具栏 chips）：emu-chips.py <png>"""
import importlib.util
import sys

spec = importlib.util.spec_from_file_location("png_scan", "scripts/png-scan.py")
png_scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(png_scan)

w, h, ch, px = png_scan.decode_png(sys.argv[1])
acc = set()
for y in range(h):
    for x in range(w):
        i = y * w * ch + x * ch
        r, g, b = px[i], px[i + 1], px[i + 2]
        if g > 100 and g - r > 60 and g - b > 15 and r < 90:
            acc.add((x, y))
rows = {}
for (x, y) in acc:
    rows.setdefault(y, []).append(x)
ys = sorted(rows)
bands = []
s = None
last = None
for y in ys:
    if s is None:
        s = y
    elif y - last > 12:
        bands.append((s, last))
        s = y
    last = y
if s is not None:
    bands.append((s, last))
for (y0, y1) in bands:
    colc = {}
    for (x, y) in acc:
        if y0 <= y <= y1:
            colc[x] = colc.get(x, 0) + 1
    xb = []
    xs = sorted(colc)
    s = None
    last = None
    for x in xs:
        if colc[x] < 30:
            continue
        if s is None:
            s = x
        elif x - last > 8:
            xb.append((s, last))
            s = x
        last = x
    if s is not None:
        xb.append((s, last))
    wide = [(a, b) for a, b in xb if b - a >= 40]
    if wide:
        cy = (y0 + y1) // 2
        print("band", (y0, y1), "chips:", [( (a + b) // 2, cy) for a, b in wide], "count", len(wide))
