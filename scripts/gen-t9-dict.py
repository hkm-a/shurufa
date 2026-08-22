#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""9 键 T9 拼音词典生成器（M-A1-2，搜狗安卓 1.40 九宫格 / 8.13 大九键）。

数据源：schemas/cn_dicts/base.dict.yaml（雾凇拼音基础词库，含字/词 + 拼音 + 权重）
        + others.dict.yaml（杂项，量小）＋ shurufa_ext.dict.yaml（本地扩展词条）。
用法：python scripts/gen-t9-dict.py [仓库根目录]
输出：schemas/shurufa_t9.dict.yaml —— 词条按"整词 T9 数字串"作为单码索引
      （2abc 3def 4ghi 5jkl 6mno 7pqrs 8tuv 9wxyz，ü/v 并入 u 组）。

生成后必须运行引擎集成测试（core/ime-bridge/tests/t9_dict.rs）验证
shurufa→7487832→输入法、nihao→64426→你好 可打。
"""
import collections
import io
import os
import re
import sys

# 仓库根目录可由命令行参数指定，避免硬编码本机路径。
REPO = sys.argv[1] if len(sys.argv) > 1 else os.getcwd()
OUT = REPO + '/schemas/shurufa_t9.dict.yaml'
SOURCE = [REPO + '/schemas/cn_dicts/base.dict.yaml', REPO + '/schemas/cn_dicts/others.dict.yaml', REPO + '/schemas/shurufa_ext.dict.yaml']

T9 = {}
for group, letters in enumerate(['abc', 'def', 'ghi', 'jkl', 'mno', 'pqrs', 'tuv', 'wxyz'], start=2):
    for ch in letters:
        T9[ch] = str(group)
T9['v'] = '8'  # ü 在 rime 拼音中常写作 v，与 u 同组
T9['ü'] = '8'


def code_of(syllable):
    """单个拼音音节 → T9 数字串；无法映射的字符跳过。"""
    return ''.join(T9.get(ch, '') for ch in syllable.lower())


def word_code(pinyin_syllables):
    return ''.join(code_of(s) for s in pinyin_syllables.split())


def load_source(path):
    entries = []
    with io.open(path, encoding='utf-8') as f:
        for line in f:
            p = line.rstrip().split('\t')
            if len(p) < 2:
                continue
            word, pinyin = p[0], p[1]
            if not word or not re.match(r'^[\u4e00-\u9fff\u3400-\u4dbf]+$', word):
                continue
            code = word_code(pinyin)
            if not code:
                continue
            try:
                weight = float(p[2])
            except (IndexError, ValueError):
                weight = 1.0
            entries.append((word, code, weight))
    return entries


def main():
    entries = []
    for path in SOURCE:
        entries.extend(load_source(path))
    # 同码内按权重降序，与 rime sort: by_weight 一致；整体按码排序便于二分。
    entries.sort(key=lambda e: (e[1], -e[2]))
    with io.open(OUT, 'w', encoding='utf-8', newline='\n') as f:
        f.write('# Rime dictionary\n')
        f.write('# encoding: utf-8\n')
        f.write('---\n')
        f.write('name: shurufa_t9\n')
        f.write('version: "2026-08-19"\n')
        f.write('sort: by_weight\n')
        f.write('use_preset_vocabulary: false\n')
        f.write('...\n')
        for word, code, weight in entries:
            f.write('%s\t%s\t%s\n' % (word, code, int(weight) if weight == int(weight) else weight))
    print('generated %s entries -> %s' % (len(entries), OUT))


if __name__ == '__main__':
    main()
