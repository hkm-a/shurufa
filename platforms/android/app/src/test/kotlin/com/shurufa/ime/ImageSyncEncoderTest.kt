package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ImageSyncEncoderTest {
    @Test
    fun sampleSize将超大图片限制在单边上限内() {
        assertEquals(1, ImageSyncEncoder.sampleSize(2000, 1200))
        assertEquals(2, ImageSyncEncoder.sampleSize(2001, 1200))
        assertEquals(4, ImageSyncEncoder.sampleSize(8000, 6000))
    }

    @Test
    fun nextDimensions按实际编码大小缩小图片() {
        val next = ImageSyncEncoder.nextDimensions(
            width = 2000,
            height = 1500,
            encodedBytes = 12 * 1024 * 1024,
            maxBytes = 8 * 1024 * 1024,
        )

        requireNotNull(next)
        assertTrue(next.width < 2000)
        assertTrue(next.height < 1500)
    }

    @Test
    fun nextDimensions拒绝无需缩小和无效上限() {
        assertNull(ImageSyncEncoder.nextDimensions(1000, 800, 1024, 2048))
        assertNull(ImageSyncEncoder.nextDimensions(1000, 800, 2048, 0))
        assertNull(ImageSyncEncoder.nextDimensions(128, 800, 4096, 2048))
    }
}
