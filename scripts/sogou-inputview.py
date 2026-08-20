#!/usr/bin/env python3
# -*- coding: utf-8 -*-
from loguru import logger
logger.remove()
from androguard.core.apk import APK
from androguard.core.dex import DEX
import zipfile

a = APK('dist/staging/sogou.apk')
z = zipfile.ZipFile('dist/staging/sogou.apk')
dexs = sorted(n for n in z.namelist() if n.endswith('.dex'))
classes = {}
for dex_name in dexs:
    d = DEX(z.read(dex_name))
    for c in d.get_classes():
        classes[c.get_name()] = c

# 1. 谁实现 onCreateInputView
print('=== onCreateInputView implementers ===')
cnt = 0
for name, c in classes.items():
    for m in c.get_methods():
        if m.get_name() == 'onCreateInputView':
            print(' ', name)
            cnt += 1
            if cnt > 15: break
    if cnt > 15: break

# 2. 名字含 Keyboard 且是 View 子类的
print('=== Keyboard* View classes ===')
cnt = 0
for name, c in classes.items():
    short = name.split('/')[-1].rstrip(';')
    if 'Keyboard' in short and cnt < 25:
        sup = c.get_superclassname().split('/')[-1].rstrip(';') if c.get_superclassname() else '?'
        print(' ', short, '->', sup)
        cnt += 1
