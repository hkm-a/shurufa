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

for target in ['Lcom/sogou/bu/ims/support/BaseInputMethodService;',
               'Lcom/sogou/bu/keyboard/KeyboardRootComponentView;',
               'Lcom/sogou/bu/keyboard/BaseKeyboardRootComponentView;']:
    c = classes.get(target)
    if not c:
        print('NOT FOUND', target); continue
    print('===', target, '->', c.get_superclassname(), '===')
    for m in c.get_methods():
        print('  ', m.get_name(), m.get_descriptor()[:80])
    print()
