package com.shurufa.attachmenttest

import org.junit.Assert.assertTrue
import org.junit.Test

class AttachmentReadResultTest {
    @Test
    fun 成功结果包含来源类型和字节数() {
        val text = AttachmentReadResult(
            source = "commitContent",
            label = "clip.png",
            uri = "content://example/clip.png",
            declaredMimeTypes = listOf("image/png"),
            resolvedMimeType = "image/png",
            byteCount = 1024,
            error = null,
        ).asDisplayText()

        assertTrue(text.contains("来源：commitContent"))
        assertTrue(text.contains("声明 MIME：image/png"))
        assertTrue(text.contains("读取大小：1024 字节"))
    }

    @Test
    fun 失败结果保留错误信息() {
        val text = AttachmentReadResult(
            source = "剪贴板粘贴",
            label = "document.pdf",
            uri = "content://example/document.pdf",
            declaredMimeTypes = emptyList(),
            resolvedMimeType = null,
            byteCount = null,
            error = "Permission Denial",
        ).asDisplayText()

        assertTrue(text.contains("声明 MIME：无"))
        assertTrue(text.contains("读取大小：未读取"))
        assertTrue(text.contains("错误：Permission Denial"))
    }
}
