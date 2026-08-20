import os, re, urllib.request, time

SRC = 'dist/staging/sogou-help-pages'
OUT = 'dist/staging/sogou-help-imgs'
os.makedirs(OUT, exist_ok=True)

headers = {'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36'}
pattern = re.compile(r'<img[^>]+src="([^"]+)"', re.I)

def absolutize(src, base_url):
    if src.startswith('http'):
        return src
    if src.startswith('//'):
        return 'https:' + src
    if src.startswith('../'):
        return 'http://pinyin.sogou.com/help/' + src.lstrip('../').lstrip('/')
    if src.startswith('images/'):
        return 'http://pinyin.sogou.com/help/' + src
    return 'http://pinyin.sogou.com/' + src.lstrip('/')

total = 0
for fn in sorted(os.listdir(SRC)):
    if not fn.endswith('.html'):
        continue
    name = fn[:-5]
    raw = open(os.path.join(SRC, fn), encoding='utf-8', errors='replace').read()
    m = re.search(r'(<h1[^>]*>.*?</h1>)(.*?)(<h3[^>]*>.*?帮助分类|$)', raw, flags=re.S | re.I)
    seg = m.group(2) if m else raw
    srcs = pattern.findall(seg)
    for i, src in enumerate(srcs):
        url = absolutize(src, '')
        ext = os.path.splitext(url.split('?')[0])[1] or '.png'
        outfn = os.path.join(OUT, f"{name}_{i}{ext}")
        if os.path.exists(outfn):
            total += 1
            continue
        try:
            req = urllib.request.Request(url, headers=headers)
            data = urllib.request.urlopen(req, timeout=25).read()
            open(outfn, 'wb').write(data)
            total += 1
        except Exception as e:
            print(name, i, url, 'ERR', e)
        time.sleep(0.2)
print('total imgs:', total)
