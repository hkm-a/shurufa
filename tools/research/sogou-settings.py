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

# 设置相关类
hits = sorted(set(n for n in classes if 'Settings' in n and 'sogou' in n.lower()))
print('=== Settings classes ===')
for h in hits[:40]:
    print(' ', h)

# 输入设置/键盘设置等 fragment/activity
for kw in ['InputSettings', 'KeyboardSettings', 'HandWrite', 'VoiceSetting', 'DictSetting', 'Preference']:
    h2 = sorted(set(n for n in classes if kw in n))[:10]
    if h2:
        print('===', kw, '===')
        for h in h2: print(' ', h)
