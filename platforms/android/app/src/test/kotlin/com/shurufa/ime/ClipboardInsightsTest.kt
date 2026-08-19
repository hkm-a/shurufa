package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** M-A2-5 剪贴板关键内容提取验证（网址 / 手机号 / 邮箱）。 */
class ClipboardInsightsTest {

    @Test
    fun 提取网址手机号邮箱() {
        val text = "验证码 1234 链接 https://shurufa.example/a?b=1 电话 13800138000 邮箱 hi@shurufa.example"
        val items = ClipboardInsights.extract(text)
        assertTrue(items.contains("https://shurufa.example/a?b=1"))
        assertTrue(items.contains("13800138000"))
        assertTrue(items.contains("hi@shurufa.example"))
    }

    @Test
    fun 普通文本无提取() {
        assertEquals(emptyList<String>(), ClipboardInsights.extract("今天天气不错"))
        assertEquals(emptyList<String>(), ClipboardInsights.extract(""))
    }

    @Test
    fun 提取去重() {
        val items = ClipboardInsights.extract("https://a.b 与 https://a.b")
        assertEquals(1, items.count { it == "https://a.b" })
    }

    @Test
    fun 标签匹配() {
        assertEquals("🔗 网址", ClipboardInsights.labelOf("https://a.b"))
        assertEquals("📞 电话", ClipboardInsights.labelOf("13800138000"))
        assertEquals("✉ 邮箱", ClipboardInsights.labelOf("a@b.com"))
    }
}
