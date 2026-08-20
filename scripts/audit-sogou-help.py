import os, re
SRC = 'dist/staging/sogou-help-pages'
for fn in sorted(os.listdir(SRC)):
    if not fn.endswith('.html'):
        continue
    raw = open(os.path.join(SRC, fn), encoding='utf-8', errors='replace').read()
    h1 = re.search(r'<h1[^>]*>(.*?)</h1>', raw, flags=re.S | re.I)
    title = re.sub(r'<[^>]+>', '', h1.group(1)).strip() if h1 else '?'
    # 正文区（h1 到 帮助分类）
    m = re.search(r'<h1[^>]*>.*?</h1>(.*?)(<h3>帮助分类|<h3 class=.*帮助分类|帮助分类)', raw, flags=re.S | re.I)
    seg = m.group(1) if m else ''
    seg = re.sub(r'<script.*?</script>', '', seg, flags=re.S | re.I)
    seg = re.sub(r'<style.*?</style>', '', seg, flags=re.S | re.I)
    text = re.sub(r'<[^>]+>', '', seg)
    text = re.sub(r'\s+', ' ', text).strip()
    imgs = re.findall(r'<img[^>]+src="([^"]+)"', seg, flags=re.I)
    print(f"{fn:24s} | {title[:24]:26s} | text={len(text):5d} | imgs={len(imgs)}")
