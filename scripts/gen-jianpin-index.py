#!/usr/bin/env python3
# 生成简拼索引：从词库提取词条的简拼编码，输出 jianpin_index.txt。
# 用法：python scripts/gen-jianpin-index.py <schemas目录>
import io, os, re, sys, collections

def load_dict(path):
    rows = []
    in_data = False
    with io.open(path, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if line.startswith("---"):
                in_data = True
                continue
            if not in_data or not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) < 2:
                continue
            word = parts[0]
            pinyin = parts[1].strip()
            weight = 0
            if len(parts) > 2:
                try:
                    weight = int(parts[2])
                except ValueError:
                    weight = 0
            rows.append((word, pinyin, weight))
    return rows

def code_of(pinyin):
    # 拼音序列 -> 简拼编码（模拟 rime-ice abbrev：zh/ch/sh 双字母）
    sylls = re.split(r"[ \\'\t]+", pinyin.strip())
    code = []
    for s in sylls:
        if not s:
            continue
        s = s.lower()
        if s.startswith(("zh", "ch", "sh")):
            code.append(s[:2])
            continue
        c = s[0]
        if c == "\u00fc":
            c = "v"
        if c < "a" or c > "z":
            return None
        code.append(c)
    return "".join(code)

def main():
    schemas_dir = sys.argv[1] if len(sys.argv) > 1 else "schemas"
    cn_dir = os.path.join(schemas_dir, "cn_dicts")
    sources = ["base.dict.yaml", "ext.dict.yaml", "others.dict.yaml"]
    idx = collections.defaultdict(list)
    for name in sources:
        p = os.path.join(cn_dir, name)
        if not os.path.exists(p):
            print("skip", p, file=sys.stderr)
            continue
        for word, pinyin, weight in load_dict(p):
            if not 2 <= len(word) <= 8:
                continue
            if re.search(r"[A-Za-z0-9\u00fc]", word):
                continue
            code = code_of(pinyin)
            if code is None or not 2 <= len(code) <= 8:
                continue
            if re.search(r"[aeiouv]", code):
                continue
            idx[code].append((word, weight))
    out = os.path.join(schemas_dir, "jianpin_index.txt")
    with io.open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("# 简拼索引（自动生成，改词库后重跑 scripts/gen-jianpin-index.py）\n")
        f.write("# 格式：编码\t词\t权重\n")
        for code in sorted(idx):
            items = sorted(idx[code], key=lambda x: -x[1])
            for word, weight in items[:20]:
                f.write(f"{code}\t{word}\t{weight}\n")
    print(f"共 {len(idx)} 个简拼编码，写入 {out}")

if __name__ == "__main__":
    main()