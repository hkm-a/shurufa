package com.shurufa.ime

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipboardTypePolicyTest {
    @Test
    fun 解析类型优先识别图片() {
        assertTrue(ClipboardTypePolicy.isImage("image/png", emptyList(), "无扩展名"))
        assertTrue(ClipboardTypePolicy.isImage(null, listOf("image/jpeg"), "attachment"))
    }

    @Test
    fun 类型缺失时按常见图片扩展名识别() {
        assertTrue(ClipboardTypePolicy.isImage(null, emptyList(), "照片.HEIC"))
        assertFalse(ClipboardTypePolicy.isImage(null, emptyList(), "报告.pdf"))
    }

    @Test
    fun 同步来源标签阻止文本回环() {
        assertTrue(ClipboardTypePolicy.isRemoteLabel("Shurufa 同步·电脑"))
        assertFalse(ClipboardTypePolicy.isRemoteLabel("普通复制"))
    }
}
