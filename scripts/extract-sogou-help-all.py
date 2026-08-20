import os, re, html

SRC = 'dist/staging/sogou-help-pages'
OUT = 'dist/staging/sogou-help-texts'
os.makedirs(OUT, exist_ok=True)

def extract(fn):
    raw = open(os.path.join(SRC, fn), encoding='utf-8', errors='replace').read()
    raw = re.sub(r'<script.*?</script>', '', raw, flags=re.S | re.I)
    raw = re.sub(r'<style.*?</style>', '', raw, flags=re.S | re.I)
    raw = re.sub(r'<h([1-6])[^>]*>', '\n### ', raw, flags=re.I)
    raw = re.sub(r'</h[1-6]>', '', raw, flags=re.I)
    raw = re.sub(r'<br\s*/?>', '\n', raw, flags=re.I)
    raw = re.sub(r'<li[^>]*>', '\n- ', raw, flags=re.I)
    raw = re.sub(r'<p[^>]*>', '\n', raw, flags=re.I)
    raw = re.sub(r'<[^>]+>', ' ', raw)
    text = html.unescape(raw)
    text = re.sub(r'[ \t]+', ' ', text)
    text = re.sub(r'\n\s*\n+', '\n', text)
    return text.strip()

for fn in sorted(os.listdir(SRC)):
    if not fn.endswith('.html'):
        continue
    text = extract(fn)
    name = fn[:-5]
    with open(os.path.join(OUT, name + '.txt'), 'w', encoding='utf-8') as f:
        f.write(text)
    # 找正文起点（跳过导航）：取首个 ### 之后
    print(name, len(text))
