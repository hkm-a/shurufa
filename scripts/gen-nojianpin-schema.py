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
输出：schemas/rime_ice_nojianpin.schema.yaml
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
            lines.append(line)
    with io.open(DST, 'w', encoding='utf-8', newline='\n') as f:
        f.write('\n'.join(lines) + '\n')
    print(f'generated {DST} (removed {removed} abbrev rules)')


if __name__ == '__main__':
    main()
