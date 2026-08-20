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
target = 'Lcom/sohu/inputmethod/sogou/SogouIME;'
for dex_name in dexs:
    d = DEX(z.read(dex_name))
    for c in d.get_classes():
        if c.get_name() == target:
            print('FOUND in', dex_name)
            print('superclass:', c.get_superclassname())
            print('--- methods ---')
            for m in c.get_methods():
                name = m.get_name()
                if any(k in name.lower() for k in ['inputview', 'candidate', 'keyboard', 'oncreate', 'init', 'skin', 'theme', 'config']):
                    print(' ', name, m.get_descriptor()[:90])
            break
