package com.shurufa.ime

import android.content.ContentResolver
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import java.io.ByteArrayOutputStream
import kotlin.math.max
import kotlin.math.roundToInt
import kotlin.math.sqrt

/** 将剪贴板图片归一化为满足同步协议字节上限的 PNG。 */
internal object ImageSyncEncoder {
    private const val MAX_EDGE = 2000
    private const val MIN_EDGE = 128
    private const val MAX_ENCODE_ATTEMPTS = 8
    private const val SIZE_HEADROOM = 0.9

    internal data class Dimensions(val width: Int, val height: Int)

    fun encode(resolver: ContentResolver, uri: Uri, maxBytes: Int): ByteArray? {
        if (maxBytes <= 0) return null
        val original = decodeSampled(resolver, uri) ?: return null
        var current = original
        try {
            repeat(MAX_ENCODE_ATTEMPTS) {
                val png = encodePng(current) ?: return null
                if (png.size <= maxBytes) return png

                val next = nextDimensions(current.width, current.height, png.size, maxBytes)
                    ?: return null
                val resized = Bitmap.createScaledBitmap(current, next.width, next.height, true)
                if (current !== original) current.recycle()
                current = resized
            }
            return null
        } finally {
            if (current !== original) current.recycle()
            original.recycle()
        }
    }

    private fun decodeSampled(resolver: ContentResolver, uri: Uri): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, bounds) }
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null

        val options = BitmapFactory.Options().apply {
            inSampleSize = sampleSize(bounds.outWidth, bounds.outHeight)
            inPreferredConfig = Bitmap.Config.ARGB_8888
        }
        return resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, options) }
    }

    private fun encodePng(bitmap: Bitmap): ByteArray? {
        val output = ByteArrayOutputStream()
        if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)) return null
        return output.toByteArray()
    }

    internal fun sampleSize(width: Int, height: Int): Int {
        var sample = 1
        while (max(width, height) / sample > MAX_EDGE) sample *= 2
        return sample
    }

    internal fun nextDimensions(
        width: Int,
        height: Int,
        encodedBytes: Int,
        maxBytes: Int,
    ): Dimensions? {
        if (width <= MIN_EDGE || height <= MIN_EDGE || encodedBytes <= maxBytes || maxBytes <= 0) {
            return null
        }
        val scale = sqrt(maxBytes.toDouble() / encodedBytes) * SIZE_HEADROOM
        if (scale >= 1.0) return null
        val nextWidth = max(MIN_EDGE, (width * scale).roundToInt())
        val nextHeight = max(MIN_EDGE, (height * scale).roundToInt())
        if (nextWidth >= width && nextHeight >= height) return null
        return Dimensions(nextWidth, nextHeight)
    }
}
