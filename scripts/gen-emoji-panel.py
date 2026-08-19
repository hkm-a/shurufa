#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""从 PC 设置中心 main.js 提取 emoji 分类与搜索索引，生成安卓 EmojiPanel.kt。
数据同源（搜狗 6.24.1 方向：分类/颜文字/搜索），保证双端面板一致。
注意：本文件刻意不写反斜杠字面量（统一用 chr(92)），避免各层转义。
"""
import io

REPO = 'C:/Users/hkm/Documents/shurufa'
SRC = REPO + '/platforms/windows-settings/ui/src/main.js'
OUT = REPO + '/platforms/android/app/src/main/kotlin/com/shurufa/ime/EmojiPanel.kt'
BS = chr(92)

text = io.open(SRC, encoding='utf-8').read()

def grab(name):
    start = text.index('const ' + name + ' = [')
    i = text.index('[', start)
    depth = 0
    j = i
    while True:
        c = text[j]
        if c == '[':
            depth += 1
        elif c == ']':
            depth -= 1
            if depth == 0:
                break
        j += 1
    return text[i:j + 1]

def parse_strings(src):
    out = []
    i = 0
    while i < len(src):
        if src[i] == '"':
            j = i + 1
            buf = []
            while j < len(src):
                c = src[j]
                if c == BS and j + 1 < len(src):
                    nxt = src[j + 1]
                    if nxt == BS:
                        buf.append(BS)
                    elif nxt == 'n':
                        buf.append(chr(10))
                    elif nxt == 't':
                        buf.append(chr(9))
                    else:
                        buf.append(nxt)
                    j += 2
                elif c == '"':
                    break
                else:
                    buf.append(c)
                    j += 1
            out.append(''.join(buf))
            i = j + 1
        else:
            i += 1
    return out

def parse_entries(src):
    out = []
    for m in src.split('{')[1:]:
        sid = m[m.index('id: "') + 5:]
        sid = sid[:sid.index('"')]
        label = m[m.index('label: "') + 8:]
        label = label[:label.index('"')]
        syms_start = m.index('symbols: [') + 10
        syms_end = m.index(']', syms_start)
        symbols = parse_strings(m[syms_start:syms_end])
        out.append((sid, label, symbols))
    return out

cats = parse_entries(grab('SYMBOL_CATEGORIES'))
emoji_cats = [c for c in cats if c[0] in ('face', 'hand', 'animal', 'life', 'heart', 'kaomoji')]

index = []
idx_src = grab('EMOJI_SEARCH_INDEX')
i = 0
while True:
    s = idx_src.find('[', i)
    if s < 0:
        break
    e = idx_src.find(']', s)
    if e < 0:
        break
    parts = parse_strings(idx_src[s:e + 1])
    if len(parts) >= 2:
        index.append(parts)
    i = e + 1

def kstr(s):
    return '"' + s.replace(BS, BS + BS).replace('"', BS + '"') + '"'

L = []
L.append('package com.shurufa.ime')
L.append('')
L.append('/**')
L.append(' * M-A2-2 表情面板数据（搜狗安卓 8.0 表情面板 / 4.8 表情搜索）。')
L.append(' * 数据与 PC 设置中心符号面板同源（windows-settings/ui/src/main.js），')
L.append(' * 由 scripts/gen-emoji-panel.py 生成，保证双端分类/搜索一致。')
L.append(' */')
L.append('data class EmojiCategory(val id: String, val label: String, val symbols: List<String>)')
L.append('')
L.append('object EmojiPanel {')
L.append('    val CATEGORIES: List<EmojiCategory> = listOf(')
for sid, label, syms in emoji_cats:
    L.append('        EmojiCategory(' + kstr(sid) + ', ' + kstr(label) + ', listOf(')
    for s in syms:
        L.append('            ' + kstr(s) + ',')
    L.append('        )),')
L.append('    )')
L.append('')
L.append('    /** [中文名, 拼音, 英文名, emoji]；与 PC 端搜索索引同源。 */')
L.append('    val SEARCH_INDEX: List<List<String>> = listOf(')
for entry in index:
    L.append('        listOf(' + ', '.join(kstr(p) for p in entry) + '),')
L.append('    )')
L.append('')
L.append('    /** 面板内搜索：关键词索引命中优先，其次符号字符/分类名包含，去重保序。 */')
L.append('    fun search(query: String): List<String> {')
L.append('        val q = query.trim().lowercase()')
L.append('        if (q.isEmpty()) return emptyList()')
L.append('        val hits = mutableListOf<String>()')
L.append('        val seen = mutableSetOf<String>()')
L.append('        fun push(s: String) { if (seen.add(s)) hits.add(s) }')
L.append('        for (entry in SEARCH_INDEX) {')
L.append('            if (entry.dropLast(1).any { it.lowercase().contains(q) }) push(entry.last())')
L.append('        }')
L.append('        for (cat in CATEGORIES) {')
L.append('            val labelHit = cat.label.lowercase().contains(q)')
L.append('            for (s in cat.symbols) {')
L.append('                if (labelHit || s.lowercase().contains(q)) push(s)')
L.append('            }')
L.append('        }')
L.append('        return hits')
L.append('    }')
L.append('')
L.append('    // 最近使用：分隔符串持久化，最多保留 limit 条，重复上移。')
L.append('    const val RECENT_KEY = "emoji_recent"')
L.append('    fun decodeRecent(raw: String): List<String> =')
L.append('        raw.split("' + BS + 'u0001").filter { it.isNotBlank() }')
L.append('    fun encodeRecent(list: List<String>): String = list.joinToString("' + BS + 'u0001")')
L.append('    fun pushRecent(current: List<String>, emoji: String, limit: Int = 30): List<String> =')
L.append('        (listOf(emoji) + current.filter { it != emoji }).take(limit)')
L.append('}')
L.append('')

io.open(OUT, 'w', encoding='utf-8').write(chr(10).join(L))
print('generated %d emoji cats, %d index entries -> %s' % (len(emoji_cats), len(index), OUT))
