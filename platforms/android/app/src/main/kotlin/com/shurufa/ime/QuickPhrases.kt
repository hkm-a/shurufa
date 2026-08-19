package com.shurufa.ime

/**
 * M-A2-1 常用语 / 快捷短语（搜狗安卓 8.0 快捷短语 / 11.46 常用语）。
 *
 * 持久化采用行格式 category\ttext（含自定义分类），保证顺序；
 * 纯函数（encode/decode/filter）与 UI 分离，便于 JVM 单测。
 * "输入前 3 个字快速发送"（11.46）需要候选栏引擎集成，列为 M-A2 后续切片。
 */
data class QuickPhrase(val text: String, val category: String = "自定义")

object QuickPhrases {
    val DEFAULT_CATEGORIES = listOf("常用回复", "问候", "祝福", "表情文字")
    const val CUSTOM_CATEGORY = "自定义"

    val defaults: List<QuickPhrase> = listOf(
        QuickPhrase("好的", "常用回复"),
        QuickPhrase("收到", "常用回复"),
        QuickPhrase("没问题", "常用回复"),
        QuickPhrase("稍等，我看一下", "常用回复"),
        QuickPhrase("谢谢", "常用回复"),
        QuickPhrase("不好意思，刚才在忙", "常用回复"),
        QuickPhrase("在吗？", "问候"),
        QuickPhrase("早上好", "问候"),
        QuickPhrase("中午好", "问候"),
        QuickPhrase("晚上好", "问候"),
        QuickPhrase("晚安", "问候"),
        QuickPhrase("新年快乐", "祝福"),
        QuickPhrase("生日快乐", "祝福"),
        QuickPhrase("恭喜发财", "祝福"),
        QuickPhrase("一切顺利", "祝福"),
        QuickPhrase("哈哈", "表情文字"),
        QuickPhrase("😂😂😂", "表情文字"),
        QuickPhrase("👍", "表情文字"),
        QuickPhrase("🤔", "表情文字"),
        QuickPhrase("😭", "表情文字"),
    )

    /** 行格式：category\ttext，逐行一条；顺序即展示顺序。 */
    fun encode(phrases: List<QuickPhrase>): String =
        phrases.joinToString("\n") { it.category + "\t" + it.text }

    /** 解析行格式；脏数据（缺列 / 空文本 / 空分类）丢弃或回退自定义。 */
    fun decode(raw: String): List<QuickPhrase> =
        raw.lineSequence()
            .map { it.split("\t", limit = 2) }
            .filter { it.size == 2 && it[1].isNotBlank() }
            .map { QuickPhrase(it[1].trim(), it[0].trim().ifBlank { CUSTOM_CATEGORY }) }
            .toList()

    /** 全部分类（默认分类在前，随后是自定义出现的分类）。 */
    fun categories(phrases: List<QuickPhrase>): List<String> =
        (DEFAULT_CATEGORIES + phrases.map { it.category })
            .distinct()
            .toList()

    /** 按分类过滤 + 文本包含查询；category 为 null 表示全部。 */
    fun filter(phrases: List<QuickPhrase>, category: String?, query: String): List<QuickPhrase> {
        val q = query.trim()
        return phrases.filter { p ->
            (category == null || p.category == category) &&
                (q.isEmpty() || p.text.contains(q))
        }
    }
}
