package com.shurufa.ime

import android.content.ClipData
import android.content.ClipDescription
import android.content.ContentResolver
import android.content.Context
import android.database.Cursor
import android.net.Uri
import android.provider.OpenableColumns
import java.io.ByteArrayOutputStream

/** 已从系统剪贴板物化、可安全跨线程发送的内容。 */
internal sealed interface CapturedClipboard {
    val signature: Int

    data class Text(val value: String) : CapturedClipboard {
        override val signature: Int = 31 * value.hashCode() + 1
    }

    data class Image(val png: ByteArray) : CapturedClipboard {
        override val signature: Int = 31 * png.contentHashCode() + 2
    }

    data class FileContent(
        val name: String,
        val mimeType: String,
        val data: ByteArray,
    ) : CapturedClipboard {
        override val signature: Int = 31 * (31 * name.hashCode() + data.contentHashCode()) + 3
    }
}

/**
 * 将 `ClipData` 中可能短期有效的 URI 立即读取为字节，避免来源应用收回
 * URI 权限后再发送失败。图片 URI 优先于同条目携带的文件名文本。
 */
internal class ClipboardCapture(private val context: Context) {
    private val resolver: ContentResolver = context.contentResolver

    fun capture(clip: ClipData): CapturedClipboard? {
        if (clip.itemCount == 0 || ClipboardTypePolicy.isRemoteLabel(clip.description.label)) {
            return null
        }
        val item = clip.getItemAt(0)
        val uri = item.uri
        if (AttachmentDeliveryPolicy.isOwnProvider(uri?.authority, context.packageName)) {
            return null
        }
        if (uri != null) return captureUri(clip.description, uri)

        val text = item.coerceToText(context)?.toString()?.takeIf { it.isNotBlank() } ?: return null
        return CapturedClipboard.Text(text)
    }

    private fun captureUri(description: ClipDescription, uri: Uri): CapturedClipboard? {
        val resolvedMime = try {
            resolver.getType(uri)
        } catch (e: Throwable) {
            android.util.Log.w(TAG, "无法解析剪贴板 URI 类型：$uri", e)
            null
        }
        val name = queryDisplayName(uri) ?: uri.lastPathSegment?.substringAfterLast('/') ?: "attachment"
        val imageCandidate = ClipboardTypePolicy.isImage(
            resolvedMime = resolvedMime,
            declaredMimes = List(description.mimeTypeCount) { description.getMimeType(it) },
            displayName = name,
        )
        if (imageCandidate) {
            val png = try {
                ImageSyncEncoder.encode(resolver, uri, SyncBridge.maxImageBytes())
            } catch (e: Throwable) {
                android.util.Log.w(TAG, "剪贴板图片编码失败：$uri", e)
                null
            }
            return png?.let(CapturedClipboard::Image)
        }

        val data = readBounded(uri, SyncBridge.maxFileBytes()) ?: return null
        return CapturedClipboard.FileContent(
            name = name,
            mimeType = resolvedMime ?: "application/octet-stream",
            data = data,
        )
    }

    private fun readBounded(uri: Uri, maxBytes: Int): ByteArray? {
        if (maxBytes <= 0) return null
        return try {
            resolver.openInputStream(uri)?.use { input ->
                val output = ByteArrayOutputStream()
                val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    if (output.size() + count > maxBytes) return@use null
                    output.write(buffer, 0, count)
                }
                output.toByteArray()
            }
        } catch (e: Throwable) {
            android.util.Log.w(TAG, "无法读取剪贴板文件：$uri", e)
            null
        }
    }

    private fun queryDisplayName(uri: Uri): String? {
        return try {
            val cursor: Cursor = resolver.query(uri, null, null, null, null) ?: return null
            cursor.use {
                val index = it.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                if (index >= 0 && it.moveToFirst()) it.getString(index) else null
            }
        } catch (e: Throwable) {
            android.util.Log.w(TAG, "无法读取剪贴板文件名：$uri", e)
            null
        }
    }

    companion object {
        private const val TAG = "shurufa-sync"
    }
}

/** 不依赖 Android 运行时的类型判定策略，便于 JVM 单元测试覆盖。 */
internal object ClipboardTypePolicy {
    const val REMOTE_LABEL_PREFIX = "Shurufa 同步·"

    private val IMAGE_EXTENSIONS = setOf(
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "heic", "heif", "avif",
    )

    fun isRemoteLabel(label: CharSequence?): Boolean =
        label?.toString()?.startsWith(REMOTE_LABEL_PREFIX) == true

    fun isImage(resolvedMime: String?, declaredMimes: List<String>, displayName: String?): Boolean {
        if (resolvedMime?.startsWith("image/", ignoreCase = true) == true) return true
        if (declaredMimes.any { it.startsWith("image/", ignoreCase = true) }) return true
        val extension = displayName?.substringAfterLast('.', "")?.lowercase().orEmpty()
        return extension in IMAGE_EXTENSIONS
    }
}
