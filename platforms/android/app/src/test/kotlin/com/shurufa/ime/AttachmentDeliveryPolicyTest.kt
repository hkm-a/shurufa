package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AttachmentDeliveryPolicyTest {
    @Test
    fun 图片通配声明优先使用无损Png() {
        assertEquals(
            "image/png",
            AttachmentDeliveryPolicy.selectImageMimeType(arrayOf("image/*")),
        )
    }

    @Test
    fun 目标仅接收Jpeg时选择Jpeg() {
        assertEquals(
            "image/jpeg",
            AttachmentDeliveryPolicy.selectImageMimeType(arrayOf("text/plain", "image/jpeg")),
        )
    }

    @Test
    fun 目标仅接收Webp时选择Webp() {
        assertEquals(
            "image/webp",
            AttachmentDeliveryPolicy.selectImageMimeType(arrayOf("image/webp")),
        )
    }

    @Test
    fun 未声明图片类型时不支持提交() {
        assertNull(AttachmentDeliveryPolicy.selectImageMimeType(emptyArray()))
        assertNull(AttachmentDeliveryPolicy.selectImageMimeType(arrayOf("text/plain", "application/pdf")))
    }

    @Test
    fun 文件类型支持精确与通配匹配() {
        assertTrue(AttachmentDeliveryPolicy.supportsMimeType("application/pdf", arrayOf("application/*")))
        assertTrue(AttachmentDeliveryPolicy.supportsMimeType("application/pdf", arrayOf("application/pdf")))
        assertFalse(AttachmentDeliveryPolicy.supportsMimeType("application/pdf", arrayOf("image/*")))
    }

    @Test
    fun 进程重启后仍能识别自有附件Uri() {
        assertTrue(
            AttachmentDeliveryPolicy.isOwnProvider(
                authority = "com.shurufa.ime.fileprovider",
                packageName = "com.shurufa.ime",
            ),
        )
        assertFalse(
            AttachmentDeliveryPolicy.isOwnProvider(
                authority = "com.example.files",
                packageName = "com.shurufa.ime",
            ),
        )
        assertFalse(AttachmentDeliveryPolicy.isOwnProvider(authority = null, packageName = "com.shurufa.ime"))
    }
}
