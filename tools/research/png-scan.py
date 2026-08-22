#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""极简 PNG 解码 + 颜色扫描（无 PIL）：找截图里接近目标颜色的像素行/列分布。"""
import struct
import sys
import zlib


def decode_png(path):
    data = open(path, 'rb').read()
    assert data[:8] == b'\x89PNG\r\n\x1a\n'
    pos = 8
    w = h = None
    bitdepth = colortype = None
    idat = b''
    while pos < len(data):
        ln, typ = struct.unpack('>I4s', data[pos:pos + 8])
        chunk = data[pos + 8:pos + 8 + ln]
        if typ == b'IHDR':
            w, h, bitdepth, colortype = struct.unpack('>IIBB', chunk[:10])
        elif typ == b'IDAT':
            idat += chunk
        pos += 12 + ln
    raw = zlib.decompress(idat)
    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[colortype]
    stride = w * channels
    # 反滤波
    out = bytearray()
    prev = bytearray(stride)
    p = 0
    for y in range(h):
        f = raw[p]
        line = bytearray(raw[p + 1:p + 1 + stride])
        for i in range(stride):
            a = line[i - channels] if i >= channels else 0
            b = prev[i]
            c = prev[i - channels] if i >= channels else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                pa = abs(b - c)
                pb = abs(a - c)
                pc = abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 255
        out += line
        prev = line
        p += 1 + stride
    return w, h, channels, bytes(out)


def main():
    path, target_hex = sys.argv[1], sys.argv[2]
    tol = int(sys.argv[3]) if len(sys.argv) > 3 else 40
    tr = int(target_hex[0:2], 16)
    tg = int(target_hex[2:4], 16)
    tb = int(target_hex[4:6], 16)
    w, h, ch, px = decode_png(path)
    rows = {}
    for y in range(h):
        count = 0
        for x in range(0, w, 2):
            i = y * w * ch + x * ch
            r, g, b = px[i], px[i + 1], px[i + 2]
            if abs(r - tr) <= tol and abs(g - tg) <= tol and abs(b - tb) <= tol:
                count += 1
        if count > 4:
            rows[y] = count
    # 聚合成连续带
    bands = []
    start = None
    last = None
    for y in sorted(rows):
        if start is None:
            start = y
        elif y - last > 12:
            bands.append((start, last))
            start = y
        last = y
    if start is not None:
        bands.append((start, last))
    print('size', w, h, 'match bands (y0,y1):', bands)
    # 对每个带打印 x 分布（簇中心）
    for y0, y1 in bands:
        xcounts = {}
        for y in range(y0, y1 + 1, 4):
            for x in range(0, w, 2):
                i = y * w * ch + x * ch
                r, g, b = px[i], px[i + 1], px[i + 2]
                if abs(r - tr) <= tol and abs(g - tg) <= tol and abs(b - tb) <= tol:
                    xcounts[x] = xcounts.get(x, 0) + 1
        xs = sorted(xcounts)
        clusters = []
        s = None
        lastx = None
        for x in xs:
            if s is None:
                s = x
            elif x - lastx > 30:
                clusters.append((s + lastx) // 2)
                s = x
            lastx = x
        if s is not None:
            clusters.append((s + lastx) // 2)
        print('band', y0, y1, 'x centers:', clusters)


if __name__ == '__main__':
    main()
