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
print('dex entries:', dexs)
total = 0
names = []
for dex_name in dexs:
    d = DEX(z.read(dex_name))
    cs = list(d.get_classes())
    total += len(cs)
    names += [c.get_name() for c in cs]
print('total classes:', total)
kws = ['Keyboard', 'Candidate', 'SogouIME', 'InputView', 'Skin', 'SoftKey', 'KeyView', 'Composing', 'Preedit']
hits = sorted(set(n for n in names if any(k in n for k in kws)))
print('hits:', len(hits))
for h in hits[:80]:
    print(' ', h)
