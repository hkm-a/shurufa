#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""常用生僻字词库包生成器（v1.2）。

数据源：
  1) 词库驱动：schemas/cn_dicts/{base,ext,others}.dict.yaml 中出现但不在
     8105.dict.yaml 的单字，按词条权重累计排序（437 字）；
  2) 知名扩展 B 补充字（龘靐齉爩…，12 字，权重 1000）。
拼音来源：rime-ice 41448 大字表（https://cdn.jsdelivr.net/gh/iDvel/rime-ice@2026.06.30/cn_dicts/41448.dict.yaml）

输出：把词条**内联追加到 schemas/rime_ice.dict.yaml**（实测 librime import_tables
对这些补充词库不生效，内联才是可靠路径，2026-08-19）。
"""
import collections
import io
import sys
import urllib.request

REPO = 'C:/Users/hkm/Documents/shurufa'
DICT41448_URL = 'https://cdn.jsdelivr.net/gh/iDvel/rime-ice@2026.06.30/cn_dicts/41448.dict.yaml'
BONUS = ['龘','靐','齉','爩','灪','麤','鱻','飝','龖','龗','䶮','㵘','㬢','爨','齑','齺','䲜','䲘','䲟','䲰','䲳','䴙','䴘']


def load_entries(path):
    out = {}
    with io.open(path, encoding='utf-8') as f:
        for line in f:
            p = line.rstrip().split('\t')
            if len(p) >= 2:
                out[p[0]] = p
    return out


def main():
    base = set()
    for line in io.open(REPO + '/schemas/cn_dicts/8105.dict.yaml', encoding='utf-8'):
        p = line.rstrip().split('\t')
        if len(p) >= 2 and len(p[0]) == 1:
            base.add(p[0])
    weight = collections.defaultdict(float)
    for fn in ('base', 'ext', 'others'):
        for line in io.open(REPO + '/schemas/cn_dicts/%s.dict.yaml' % fn, encoding='utf-8'):
            p = line.rstrip().split('\t')
            if len(p) >= 3:
                try:
                    w = float(p[2])
                except ValueError:
                    continue
                for ch in p[0]:
                    if len(ch) == 1:
                        weight[ch] += w
    rare = [(c, w) for c, w in weight.items() if c not in base]
    rare.sort(key=lambda x: -x[1])
    print('downloading 41448 …')
    urllib.request.urlretrieve(DICT41448_URL, REPO + '/dist/staging/41448.dict.yaml')
    py = {}
    for line in io.open(REPO + '/dist/staging/41448.dict.yaml', encoding='utf-8'):
        p = line.rstrip().split('\t')
        if len(p) >= 2 and len(p[0]) == 1:
            py.setdefault(p[0], p[1])
    rows, seen = [], set()
    for c, w in rare:
        if c in py:
            rows.append((c, py[c], int(w)))
            seen.add(c)
    for c in BONUS:
        if c not in seen and c in py:
            rows.append((c, py[c], 1000))
            seen.add(c)
    block = ['# ===== v1.2 常用生僻字词库包（%d 字）=====' % len(rows),
             '# 来源：base/ext/others 词库中非 8105 规范字（按 25 亿字语料字频权重）+ 知名扩展 B 补充；',
             '# 拼音来自 rime-ice 41448（Unihan kMandarin + 汉典 zdic）。内联进主词典才能被编译',
             '# （实测 librime import_tables 对部分单字不生效，2026-08-19）。']
    for c, p, w in rows:
        block.append('%s\t%s\t%d' % (c, p, w))
    path = REPO + '/schemas/rime_ice.dict.yaml'
    content = io.open(path, encoding='utf-8').read()
    start = content.find('# ===== v1.2 常用生僻字词库包')
    if start != -1:
        content = content[:start].rstrip() + '\n'
    content += '\n'.join(block) + '\n'
    io.open(path, 'w', encoding='utf-8').write(content)
    print('inlined %d entries into rime_ice.dict.yaml' % len(rows))


if __name__ == '__main__':
    main()
