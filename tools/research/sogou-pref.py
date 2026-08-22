#!/usr/bin/env python3
# -*- coding: utf-8 -*-
from loguru import logger
logger.remove()
from androguard.core.apk import APK
from androguard.core.dex import DEX
import zipfile

a = APK('dist/staging/sogou.apk')
z = zipfile.ZipFile('dist/staging/sogou.apk')
classes = {}
for dex_name in sorted(n for n in z.namelist() if n.endswith('.dex')):
    d = DEX(z.read(dex_name))
    for c in d.get_classes():
        classes[c.get_name()] = c

prefs = sorted(set(n for n in classes if n.startswith('Lcom/sogou/lib/preference/')))
print('preference framework classes:', len(prefs))
for p in prefs[:40]:
    print(' ', p)

# APK 内 preference XML 资源
xmls = [n for n in z.namelist() if n.endswith('.xml') and 'res' in n]
print('xml resources:', len(xmls))
print(xmls[:10])
