#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""从搜狗官网 HTML 提取安卓 APK 下载链接。"""
import re
import sys

s = open(sys.argv[1], encoding="utf-8", errors="replace").read()
# downloadConfig 是 JSON 字符串（内含 \" 转义）
m = re.search(r"downloadConfig\\":\\"(.*?)\\"", s)
found = []
if m:
    raw = m.group(1).encode().decode("unicode_escape", errors="replace")
    # 找所有 url
    urls = re.findall(r"https?://[^" ]+", raw)
    apks = [u for u in urls if ".apk" in u or "android" in u.lower() or "mobile" in u.lower()]
    found = apks[:20]
    print("config apk/android urls:")
    for u in found:
        print(" ", u)
else:
    print("no downloadConfig")
# 全页找 apk
all_apks = re.findall(r"https?://[^"' ]+.apk[^"' ]*", s)
if all_apks:
    print("page apk urls:", all_apks[:10])
