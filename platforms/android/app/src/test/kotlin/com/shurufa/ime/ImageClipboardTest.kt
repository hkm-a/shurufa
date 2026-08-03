package com.shurufa.ime

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayOutputStream

class ImageClipboardTest {
    @Test
    fun 同步剪贴板使用固定图片类型和本机回填标签() {
        assertEquals("image/png", ImageClipboard.IMAGE_MIME_TYPE)
        assertEquals("Shurufa 同步·来自电脑的图片", ImageClipboard.clipboardLabel("来自电脑的图片"))
    }

    @Test
    fun MediaStore事务成功后返回条目并发布完整PNG() {
        val output = ByteArrayOutputStream()
        val result = writeImageTransaction(
            create = { "content://media/1" },
            openOutput = { output },
            publish = { true },
            cleanup = { 1 },
            png = byteArrayOf(1, 2, 3),
        )

        assertEquals("content://media/1", result)
        assertArrayEquals(byteArrayOf(1, 2, 3), output.toByteArray())
    }

    @Test
    fun MediaStore写入失败会清理半成品并返回中文错误() {
        var deleted: String? = null
        val error = assertThrows(IllegalStateException::class.java) {
            writeImageTransaction<String>(
                create = { "content://media/2" },
                openOutput = { null },
                publish = { false },
                cleanup = {
                    deleted = it
                    1
                },
                png = byteArrayOf(9),
            )
        }

        assertEquals("content://media/2", deleted)
        assertTrue(error.message!!.startsWith("写入系统图片库失败"))
    }
}
