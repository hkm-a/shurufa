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

base = 'Lcom/sohu/inputmethod/settings/preference/BaseSettingActivity;'
c = classes.get(base)
print('=== BaseSettingActivity ->', c.get_superclassname(), '===')
ms = list(c.get_methods())
print('methods:', len(ms))
for m in ms[:50]:
    print(' ', m.get_name(), m.get_descriptor()[:70])
