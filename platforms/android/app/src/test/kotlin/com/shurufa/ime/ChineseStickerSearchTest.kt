package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Test

class ChineseStickerSearchTest {
    @Test
    fun 用中文候选词从表情包名称和分类中检索() {
        val entries = listOf(
            ChineseStickerSearch.Entry("笑死我了.jpg", "滑稽表情包", "https://example.com/laugh.jpg"),
            ChineseStickerSearch.Entry("晚安.jpg", "问候", "https://example.com/night.jpg"),
        )
        assertEquals("笑死我了.jpg", ChineseStickerSearch.search(listOf("笑死"), entries).first().name)
        assertEquals("晚安.jpg", ChineseStickerSearch.search(listOf("晚安"), entries).first().name)
    }

    @Test
    fun 高频拼音能直接召回中文表情包() {
        val entries = listOf(
            ChineseStickerSearch.Entry("哈哈哈.jpg", "滑稽表情包", "https://example.com/laugh.jpg"),
        )
        assertEquals("哈哈哈.jpg", ChineseStickerSearch.search(listOf("haha"), entries).first().name)
    }

    @Test
    fun 支持约定的中文高频拼音意图() {
        val intents = mapOf(
            "wuyu" to "无语",
            "wanan" to "晚安",
            "moyu" to "摸鱼",
            "jiayou" to "加油",
        )
        intents.forEach { (pinyin, keyword) ->
            val entries = listOf(ChineseStickerSearch.Entry("$keyword.jpg", "常用", "https://example.com/$pinyin.jpg"))
            assertEquals("$keyword.jpg", ChineseStickerSearch.search(listOf(pinyin), entries).single().name)
        }
    }
}
