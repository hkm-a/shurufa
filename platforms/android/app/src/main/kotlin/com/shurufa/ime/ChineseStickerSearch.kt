package com.shurufa.ime

/** ChineseBQB 元数据的本地检索规则；数据下载与界面展示留在服务层。 */
internal object ChineseStickerSearch {
    data class Entry(val name: String, val category: String, val url: String)

    fun search(queryTerms: List<String>, entries: List<Entry>, limit: Int = 3): List<Entry> {
        val terms = queryTerms.flatMap(::expandTerms).filter { it.length >= 2 }.distinct()
        if (terms.isEmpty()) return emptyList()
        return entries.asSequence()
            .map { entry -> entry to score(entry, terms) }
            .filter { it.second > 0 }
            .sortedByDescending { it.second }
            .map { it.first }
            .take(limit)
            .toList()
    }

    private fun score(entry: Entry, terms: List<String>): Int {
        val text = normalize("${entry.name} ${entry.category}")
        return terms.maxOf { term -> if (text.contains(term)) term.length else 0 }
    }

    private fun normalize(value: String): String =
        value.lowercase().replace(Regex("[^\\p{IsHan}a-z0-9]+"), "")

    /** 高频拼音先映射为中文意图；长尾语义由后续本地向量索引补齐。 */
    private fun expandTerms(value: String): List<String> {
        val normalized = normalize(value)
        return listOfNotNull(normalized, PINYIN_INTENTS[normalized]).distinct()
    }

    private val PINYIN_INTENTS = mapOf(
        "haha" to "哈哈",
        "xiao" to "笑",
        "wuyu" to "无语",
        "baibai" to "拜拜",
        "wanan" to "晚安",
        "xiexie" to "谢谢",
        "jiayou" to "加油",
        "moyu" to "摸鱼",
        "kaixin" to "开心",
        "shengqi" to "生气",
        "nanguo" to "难过",
    )
}
