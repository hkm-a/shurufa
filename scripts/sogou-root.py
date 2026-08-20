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

# 全名定位
for name, c in classes.items():
    if name.endswith('KeyboardRootComponentView;') or name.endswith('BaseKeyboardRootComponentView;'):
        print('FULL:', name, '->', c.get_superclassname())

print()
print('=== 谁调用 setInputView ===')
cnt = 0
for name, c in classes.items():
    for m in c.get_methods():
        if m.get_name() == 'setInputView':
            print(' ', name, '::', m.get_descriptor()[:60])
            cnt += 1
            if cnt > 10: break
    if cnt > 10: break
