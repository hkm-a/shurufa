#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""裁剪 PNG 区域：crop-png.py src dst x0 y0 x1 y1"""
import importlib.util
import struct
import sys
import zlib

spec = importlib.util.spec_from_file_location("png_scan", "scripts/png-scan.py")
png_scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(png_scan)

src, dst = sys.argv[1], sys.argv[2]
x0, y0, x1, y1 = map(int, sys.argv[3:7])
w, h, ch, px = png_scan.decode_png(src)
cw, chh = x1 - x0, y1 - y0
raw = bytearray()
for y in range(y0, y1):
    raw.append(0)
    for x in range(x0, x1):
        i = (y * w + x) * ch
        raw.extend(px[i:i + ch])

def chunk(t, d):
    return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))

out = bytes([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
out += chunk(b"IHDR", struct.pack(">IIBBBBB", cw, chh, 8, 6, 0, 0, 0))
out += chunk(b"IDAT", zlib.compress(bytes(raw)))
out += chunk(b"IEND", b"")
with open(dst, "wb") as f:
    f.write(out)
print("cropped", dst, cw, "x", chh)
