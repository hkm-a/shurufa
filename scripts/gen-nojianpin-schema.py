#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成「无简拼」变体方案（M10 困难项简拼开关的部署期替代）。

librime 1.17 speller/algebra 不支持条件规则（option@jianpin: 实测
Error loading formula #13），无法热开关简拼；改为生成去掉 abbrev 规则的
rime_ice_nojianpin.schema.yaml，由设置中心方案页切换 + 重新部署生效。

阶段3曾评估 *.custom.yaml patch 覆盖层：Rime 的 __include 可跨文件引用
节点，但无法在列表中精准删除 abbrev 条目；在未验证 schema 级继承前，
继续保留“生成完整副本”策略，避免破坏 20 个引擎集成测试的安全网。

用法：python scripts/gen-nojianpin-schema.py [仓库根目录]
输出：schemas/rime_ice_nojianpin.schema.yaml + schemas/rime_ice_nojianpin.dict.yaml

词典名必须独立（dict 壳 import_tables 引用 rime_ice）：librime 编译产物以
translator/dictionary 命名（rime_ice.prism.bin 等），两方案共用词典名时
后编译者会反噬覆盖对方的 prism（algebra 不同则互踩；2026-08-23 实测：编译
变体后 rime_ice.prism 被无 abbrev 版覆盖，正常方案的引擎简拼被静默关闭）。
独立词典名使产物隔离，代价是一份 table.bin 磁盘副本。
"""
import io
import os
import sys

REPO = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else os.getcwd()
SRC = os.path.join(REPO, 'schemas', 'rime_ice.schema.yaml')
DST = os.path.join(REPO, 'schemas', 'rime_ice_nojianpin.schema.yaml')


def main():
    lines = []
    removed = 0
    with io.open(SRC, encoding='utf-8') as f:
        for raw in f:
            line = raw.rstrip('\n')
            if line.lstrip().startswith('- abbrev/'):
                removed += 1
                continue
            if line == '  schema_id: rime_ice':
                line = '  schema_id: rime_ice_nojianpin'
            elif line == '  name: 雾凇拼音':
                line = '  name: 雾凇拼音（无简拼）'
            elif line == '  dictionary: rime_ice':
                # 词典名独立，避免与 rime_ice 共用编译产物（见模块注释）
                line = '  dictionary: rime_ice_nojianpin'
            lines.append(line)
    with io.open(DST, 'w', encoding='utf-8', newline='\n') as f:
        f.write('\n'.join(lines) + '\n')
    print(f'generated {DST} (removed {removed} abbrev rules)')

    # 词典壳：镜像 rime_ice.dict.yaml 的 import_tables 叶子表（librime 的
    # import_tables 不传递——import rime_ice 壳只会得到壳自身的零词条 +
    # 大写字母/数字注音，实测 table.bin 仅 5KB；必须平铺引用同样的叶子表），
    # 自身零词条，与 rime_ice 词库保持同源。
    dict_dst = os.path.join(REPO, 'schemas', 'rime_ice_nojianpin.dict.yaml')
    shell = (
        '# Rime dictionary\n'
        '# encoding: utf-8\n'
        '\n'
        '# 无简拼变体的词典壳（gen-nojianpin-schema.py 生成，勿手改）：\n'
        '# import_tables 与 rime_ice.dict.yaml 平铺同源（不传递，不能只 import\n'
        '# rime_ice 壳）；独立 name 使编译产物与 rime_ice 隔离。\n'
        '---\n'
        'name: rime_ice_nojianpin\n'
        'version: "generated"\n'
        'sort: original\n'
        'import_tables:\n'
        '  - cn_dicts/8105\n'
        '  - cn_dicts/base\n'
        '  - cn_dicts/ext\n'
        '  - cn_dicts/others\n'
        '  - shurufa_ext\n'
        '  - rime_ice\n'
        '...\n'
    )
    with io.open(dict_dst, 'w', encoding='utf-8', newline='\n') as f:
        f.write(shell)
    print(f'generated {dict_dst}')


if __name__ == '__main__':
    main()
