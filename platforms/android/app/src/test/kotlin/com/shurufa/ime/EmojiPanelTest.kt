package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** M-A2-2 表情数据的纯逻辑验证（分类 / 搜索 / 最近使用）。 */
class EmojiPanelTest {

    @Test
    fun 分类包含六大类且数据非空() {
        assertEquals(
            listOf("face", "hand", "animal", "life", "heart", "kaomoji"),
            EmojiPanel.CATEGORIES.map { it.id },
        )
        assertTrue(EmojiPanel.CATEGORIES.all { it.symbols.isNotEmpty() })
        assertTrue(EmojiPanel.CATEGORIES.first { it.id == "face" }.symbols.contains("😀"))
    }

    @Test
    fun 关键词搜索命中中文拼音英文() {
        assertTrue(EmojiPanel.search("weixiao").contains("😊"))
        assertTrue(EmojiPanel.search("微笑").contains("😊"))
        assertTrue(EmojiPanel.search("smile").contains("😊"))
        assertTrue(EmojiPanel.search("猫").contains("🐱"))
        assertTrue(EmojiPanel.search("").isEmpty())
    }

    @Test
    fun 搜索结果去重保序() {
        // 赞 / zan / thumbsup 都指向 👍，只出现一次
        val hits = EmojiPanel.search("zan")
        assertEquals(1, hits.count { it == "👍" })
    }

    @Test
    fun 最近使用上限且重复上移() {
        val base = listOf("😀", "😂", "😍")
        val pushed = EmojiPanel.pushRecent(base, "😂")
        assertEquals(listOf("😂", "😀", "😍"), pushed)
        val many = (1..40).map { it.toString() }
        assertEquals(30, EmojiPanel.pushRecent(many, "x").size)
    }

    @Test
    fun 最近使用编解码往返() {
        val list = listOf("😀", "😁", "😂")
        assertEquals(list, EmojiPanel.decodeRecent(EmojiPanel.encodeRecent(list)))
    }
}
