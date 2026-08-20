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

for prefix in ['Lcom/sogou/imskit/core/ui/keyboard/view/',
               'Lcom/sogou/imskit/core/ui/keyboard/candidate/',
               'Lcom/sogou/imskit/core/ui/keyboard/key/',
               'Lcom/sogou/imskit/core/ui/keyboard/widget/']:
    hits = sorted(set(n for n in classes if n.startswith(prefix)))
    print('===', prefix, '(', len(hits), ') ===')
    for h in hits[:40]:
        print(' ', h)
    print()
