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

targets = {
  'KeyboardPopupView': 'Lcom/sogou/bu/keyboard/popup/KeyboardPopupView;',
  'SingleHandKeyboard': 'Lcom/sogou/imskit/core/ui/keyboard/resize/singlehand/SingleHandKeyboard;',
  'BaseKeyboardRootComponentView': 'Lcom/sogou/imskit/core/ui/keyboard/view/BaseKeyboardRootComponentView;',
  'KeyPopupViewLayout': 'Lcom/sogou/bu/keyboard/popup/style/KeyPopupViewLayout;',
}
for label, full in targets.items():
    c = classes.get(full)
    if not c:
        print('===', label, 'NOT FOUND ==='); continue
    print('===', label, '->', c.get_superclassname(), '===')
    ms = list(c.get_methods())
    print('  methods:', len(ms))
    for m in ms[:40]:
        print('   ', m.get_name(), m.get_descriptor()[:70])
    print()
