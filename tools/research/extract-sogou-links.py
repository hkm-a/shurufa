import re
raw = open('dist/staging/sogou-help-utf8.html', encoding='utf-8', errors='replace').read()
links = re.findall("<a[^>]+href=[\"']([^\"']+)[\"'][^>]*>([^<]{0,40})</a>", raw, flags=re.I)
seen = set()
for href, text in links:
    text = text.strip()
    if not text or href in seen:
        continue
    seen.add(href)
    print(text, '=>', href)
