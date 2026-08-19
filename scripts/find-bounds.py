#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""在 uiautomator dump 中按 text/content-desc 查找节点 bounds。"""
import re
import sys

path = sys.argv[1]
needle = sys.argv[2]
s = open(path, encoding='utf-8', errors='replace').read()
found = False
for m in re.finditer(r'<node[^>]*/?>', s):
    node = m.group(0)
    t = re.search(r'text="([^"]*)"', node)
    d = re.search(r'content-desc="([^"]*)"', node)
    b = re.search(r'bounds="([[0-9,]+][[0-9,]+])"', node)
    label = (t.group(1) if t else '') or (d.group(1) if d else '')
    if needle in label and b:
        print(label, '->', b.group(1))
        found = True
if not found:
    print('NOT FOUND:', needle)
