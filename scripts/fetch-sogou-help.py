import urllib.request
import os, time, re, html

BASE = 'https://pinyin.sogou.com/help.php?list={}&q={}'
OUT = 'dist/staging/sogou-help-pages'
os.makedirs(OUT, exist_ok=True)

pages = [
    ("4", "2", "settings-general"),
    ("4", "3", "settings-keys"),
    ("4", "4", "settings-dict"),
    ("4", "5", "settings-skin"),
    ("4", "6", "settings-advanced"),
    ("2", "2", "ui-inputwin"),
    ("2", "3", "ui-settingswin"),
    ("3", "4", "rule-shuangpin"),
    ("3", "5", "rule-mohu"),
    ("3", "7", "rule-url"),
    ("3", "8", "rule-umode"),
    ("3", "9", "rule-bihua"),
    ("3", "10", "rule-vmode"),
    ("3", "11", "rule-date"),
    ("3", "12", "rule-chaizi"),
    ("1", "9", "fun-customphrase"),
    ("1", "10", "fun-shouzi"),
    ("1", "11", "fun-renming"),
    ("1", "12", "fun-keyword"),
    ("1", "13", "fun-shengpizi"),
    ("1", "14", "fun-biaoqing"),
]

headers = {'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36'}
for (lst, q, name) in pages:
    url = BASE.format(lst, q)
    try:
        req = urllib.request.Request(url, headers=headers)
        raw = urllib.request.urlopen(req, timeout=25).read()
        # 检测编码
        try:
            text = raw.decode('gb18030')
        except UnicodeDecodeError:
            text = raw.decode('utf-8', errors='replace')
        with open(os.path.join(OUT, name + '.html'), 'w', encoding='utf-8') as f:
            f.write(text)
        print(name, 'OK', len(text))
    except Exception as e:
        print(name, 'ERR', e)
    time.sleep(0.3)
