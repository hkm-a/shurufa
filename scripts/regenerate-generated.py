#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""重新生成三份构建期 schemas 产物（阶段 3 出库后统一入口）。

生成：
  schemas/shurufa_t9.dict.yaml
  schemas/jianpin_index.txt
  schemas/rime_ice_nojianpin.schema.yaml

并更新 schemas/generated-files.sha256（小清单入库，CI 用它校验生成物一致）。
用法：python scripts/regenerate-generated.py [仓库根目录]
"""
import hashlib
import io
import os
import subprocess
import sys

ROOT = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
SCHEMAS = os.path.join(ROOT, 'schemas')
SCRIPTS = os.path.join(ROOT, 'scripts')
MANIFEST = os.path.join(SCHEMAS, 'generated-files.sha256')
GENERATED = ['shurufa_t9.dict.yaml', 'jianpin_index.txt', 'rime_ice_nojianpin.schema.yaml', 'rime_ice_nojianpin.dict.yaml']


def run(args):
    subprocess.check_call(args)


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def main():
    run([sys.executable, os.path.join(SCRIPTS, 'gen-t9-dict.py'), ROOT])
    run([sys.executable, os.path.join(SCRIPTS, 'gen-jianpin-index.py'), SCHEMAS])
    run([sys.executable, os.path.join(SCRIPTS, 'gen-nojianpin-schema.py'), ROOT])
    with io.open(MANIFEST, 'w', encoding='ascii', newline='\n') as f:
        for name in GENERATED:
            path = os.path.join(SCHEMAS, name)
            if not os.path.isfile(path):
                raise SystemExit(f'missing generated file: {path}')
            f.write(f'{sha256(path)}  {name}\n')
    print(f'regenerated {len(GENERATED)} artifacts and wrote {MANIFEST}')


if __name__ == '__main__':
    main()
