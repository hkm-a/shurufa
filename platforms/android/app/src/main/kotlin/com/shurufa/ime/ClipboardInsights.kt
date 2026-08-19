package com.shurufa.ime

/**
 * M-A2-5 剪贴板关键内容提取（搜狗安卓 11.46「自动提取复制内容关键信息，
 * 一键发送」）。从剪贴板文本中提取网址 / 手机号 / 邮箱，供历史面板一键上屏。
 * 纯正则，便于 JVM 单测；提取顺序：网址 > 手机号（首个） > 邮箱（首个），去重。
 */
object ClipboardInsights {

    private val URL = Regex("https?://[A-Za-z0-9\\-._~:/?#\\[\\]@!$&'()*+,;=%]+")
    private val PHONE = Regex("(?<![0-9])1[3-9][0-9]{9}(?![0-9])")
    private val EMAIL = Regex("[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\\.[A-Za-z]{2,}")

    /** 提取关键内容（去重保序）；无则空列表。 */
    fun extract(text: String): List<String> {
        if (text.isBlank()) return emptyList()
        val out = linkedSetOf<String>()
        URL.find(text)?.let { out.add(it.value) }
        PHONE.findAll(text).forEach { out.add(it.value) }
        EMAIL.find(text)?.let { out.add(it.value) }
        return out.toList()
    }

    /** 展示前缀：网址 🔗 / 电话 📞 / 邮箱 ✉（按内容匹配）。 */
    fun labelOf(item: String): String = when {
        URL.matches(item) -> "🔗 网址"
        PHONE.matches(item) -> "📞 电话"
        EMAIL.matches(item) -> "✉ 邮箱"
        else -> "📋 内容"
    }
}
