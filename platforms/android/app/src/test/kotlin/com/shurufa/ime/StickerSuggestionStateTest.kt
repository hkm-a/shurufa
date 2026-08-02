package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class StickerSuggestionStateTest {
    @Test
    fun 单个字符不触发表情包延迟检索() {
        val state = StickerSuggestionState()

        assertNull(state.replace("h", listOf("好")))
        assertNull(state.replace("你", listOf("你好")))
    }

    @Test
    fun 有效预编辑保留拼音和前三个候选词() {
        val request = StickerSuggestionState().replace("haha", listOf("哈哈", "呵呵", "笑", "多余"))

        requireNotNull(request)
        assertEquals(listOf("haha", "哈哈", "呵呵", "笑"), request.terms)
    }

    @Test
    fun 连续输入后旧查询不能展示() {
        val state = StickerSuggestionState()
        val oldRequest = requireNotNull(state.replace("haha", listOf("哈哈")))
        val currentRequest = requireNotNull(state.replace("wuyu", listOf("无语")))

        assertFalse(state.isCurrent(oldRequest))
        assertTrue(state.isCurrent(currentRequest))
    }

    @Test
    fun 清空或提交输入后在途结果失效() {
        val state = StickerSuggestionState()
        val request = requireNotNull(state.replace("wanan", listOf("晚安")))

        state.invalidate()

        assertFalse(state.isCurrent(request))
    }

    @Test
    fun 横幅最多展示两张且停顿时长固定() {
        assertEquals(2, StickerSuggestionPolicy.visibleLimit)
        assertEquals(420L, StickerSuggestionPolicy.delayMillis)
    }
}
