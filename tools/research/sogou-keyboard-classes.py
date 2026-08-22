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
names = []
for dex_name in dexs:
    d = DEX(z.read(dex_name))
    names += [c.get_name() for c in d.get_classes()]

# 键盘视图层与输入引擎层
def show(prefixes, label, limit=45):
    hits = sorted(set(n for n in names if any(n.startswith(p) for p in prefixes)))
    print('===', label, '(', len(hits), ') ===')
    for h in hits[:limit]:
        print(' ', h)
    print()

show(['Lcom/sogou/bu/keyboard/'], 'keyboard')
show(['Lcom/sogou/bu/input/'], 'input', 30)
show(['Lcom/sogou/bu/theme/'], 'theme', 15)
