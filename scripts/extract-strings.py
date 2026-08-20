#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import re

s = open('dist/staging/sogou-res.txt', encoding='utf-8', errors='replace').read()
vals = re.findall(r'\(\s*\) \"([^\"]*)\"', s)
uniq = sorted(set(vals))
open('dist/staging/sogou-strings-def.txt', 'w', encoding='utf-8').write('\n'.join(uniq))
print('total unique:', len(uniq))
kws = ['滑动','单手','悬浮','剪贴板','语音','手写','皮肤','斗图','翻译','天气','快递','截图','游戏','键盘高度','按键音','振动','主题','工具栏','候选','符号','表情','词库','设置','夜间','护眼','跨屏','贴纸','泡泡','剪藏','AI','九宫格','全键盘','输入方式','笔画','双拼','五笔','手写输入','跨屏输入','拍照输入']
for k in kws:
    hits = [v for v in uniq if k in v]
    if hits:
        print('###', k, '(', len(hits), ')')
        for h in hits[:6]:
            print('   ', h[:80])
