#!/usr/bin/env python3
# -*- coding: utf-8 -*-
from androguard.core.apk import APK

a = APK('dist/staging/sogou.apk')
print('package:', a.get_package())
print('version:', a.get_androidversion_name(), a.get_androidversion_code())
print('--- services (ime/input) ---')
for s in a.get_services():
    if 'ime' in s.lower() or 'input' in s.lower():
        print(' ', s)
print('--- main activity ---')
print(a.get_main_activity())
print('--- input-relevant permissions ---')
perms = [p for p in a.get_permissions() if any(k in p.lower() for k in ['record','storage','phone','contacts','network','vibrate','bluetooth'])]
print(perms)
