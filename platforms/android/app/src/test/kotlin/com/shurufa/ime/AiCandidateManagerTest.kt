package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** AI 候选预测：提示词构造与输出解析（纯函数）。 */
class AiCandidateManagerTest {

    @Test
    fun testBuildPromptIncludesPreedit() {
        val p = AiCandidateManager.buildPrompt("nihao", "")
        assertTrue(p.contains("nihao"))
        assertTrue(p.contains("3 个词"))
    }

    @Test
    fun testBuildPromptIncludesContext() {
        val p = AiCandidateManager.buildPrompt("hao", "今天天气真")
        assertTrue(p.contains("今天天气真"))
    }

    @Test
    fun testParseCommaSeparated() {
        assertEquals(
            listOf("你好", "您好", "您好吗"),
            AiCandidateManager.parseCandidates("你好,您好,您好吗"),
        )
    }

    @Test
    fun testParseChineseCommaAndQuotes() {
        assertEquals(
            listOf("好的", "可以"),
            AiCandidateManager.parseCandidates("“好的”，可以"),
        )
    }

    @Test
    fun testParseDedupAndTrim() {
        assertEquals(
            listOf("好的", "可以"),
            AiCandidateManager.parseCandidates("好的, 可以, 好的"),
        )
    }

    @Test
    fun testParseFiltersEmptyAndTooLong() {
        val r = AiCandidateManager.parseCandidates(",,一个非常非常非常非常非常非常非常非常非常长的词,可以")
        assertEquals(listOf("可以"), r)
    }

    @Test
    fun testParseMaxThree() {
        assertEquals(3, AiCandidateManager.parseCandidates("一,二,三,四,五").size)
    }
}
