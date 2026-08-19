package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** M-A2-1 常用语的纯逻辑验证（编码/解析/分类/搜索）。 */
class QuickPhrasesTest {

    @Test
    fun 默认短语非空且分类齐全() {
        assertTrue(QuickPhrases.defaults.isNotEmpty())
        val cats = QuickPhrases.categories(QuickPhrases.defaults)
        assertEquals(QuickPhrases.DEFAULT_CATEGORIES, cats.take(4))
    }

    @Test
    fun 编码解析往返保序() {
        val phrases = listOf(
            QuickPhrase("好的", "常用回复"),
            QuickPhrase("👋", "自定义"),
            QuickPhrase("新年快乐", "祝福"),
        )
        val round = QuickPhrases.decode(QuickPhrases.encode(phrases))
        assertEquals(phrases, round)
    }

    @Test
    fun 脏数据被丢弃或回退自定义分类() {
        // 空分类回退自定义；空文本行与缺列行丢弃
        val parsed = QuickPhrases.decode("常用回复\t收到\n\t空分类行\n坏行\n无文本\t\n自定义\t哈哈")
        assertEquals(
            listOf(
                QuickPhrase("收到", "常用回复"),
                QuickPhrase("空分类行", "自定义"),
                QuickPhrase("哈哈", "自定义"),
            ),
            parsed,
        )
    }

    @Test
    fun 分类过滤与搜索() {
        val list = QuickPhrases.defaults
        val reply = QuickPhrases.filter(list, "常用回复", "")
        assertTrue(reply.all { it.category == "常用回复" })
        assertTrue(reply.isNotEmpty())
        val hit = QuickPhrases.filter(list, null, "早安")
        assertTrue(hit.none { it.text.contains("早上好") } || hit.isEmpty())
        val exact = QuickPhrases.filter(list, "问候", "早上好")
        assertEquals(listOf(QuickPhrase("早上好", "问候")), exact)
    }
}
