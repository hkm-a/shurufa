package com.shurufa.ime

import android.content.ClipData
import android.content.ClipDescription
import android.content.ContentResolver
import android.content.Context
import android.database.Cursor
import android.net.Uri
import android.provider.OpenableColumns
import java.io.ByteArrayOutputStream
import java.security.MessageDigest

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
 *
 * 额外的循环抑制：如果远端刚把内容写进系统剪贴板（label 已标记或 authority 自有），
 * 短时间内同 signature 的读取也会被跳过，防止 ROM 改写 label 后引发同步风暴。
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
        // 文本短路：远端刚塞入 1s 内的相同文本不再回送
        if (ClipboardTypePolicy.isRecentEcho(text)) return null
        // C14 第二道闸门（应用写入回声）：某些 ROM（尤其荣耀）在我们把远端文本写进
        // 系统剪贴板 1~2s 后，再次经 onPrimaryClipChanged 触发 capture()，会把同一条
        // 重新入库并回推对端形成抖动。这里以「最新文本条目签名」判定：
        // 2s 窗口内签名（sha256 前 16 hex）一致即视为自身写入的回声，跳过。
        // 注：latestSignature 是单行 SQLite 读，量级远小于 list(30)，可接受。
        val signature = try {
            ClipStore.latestSignature()
        } catch (e: Throwable) {
            ""
        }
        if (signature.isNotEmpty()) {
            val parts = signature.split("\u0001")
            val updatedMs = parts.getOrNull(0)?.toLongOrNull()
            val hash16 = parts.getOrNull(1)
            if (updatedMs != null && !hash16.isNullOrEmpty() &&
                System.currentTimeMillis() - updatedMs < 2_000L &&
                hash16 == sha256Hex16(text)
            ) {
                return null
            }
        }
        return CapturedClipboard.Text(text)
    }

    /** sha256(text UTF-8) 前 8 字节 → 16 位小写 hex，与 Rust 侧签名协议一致。 */
    private fun sha256Hex16(text: String): String {
        val digest = MessageDigest.getInstance("SHA-256").digest(text.toByteArray(Charsets.UTF_8))
        val out = StringBuilder(16)
        for (i in 0 until 8) {
            out.append(String.format("%02x", digest[i]))
        }
        return out.toString()
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

    // ---------- 回环抑制（独立于 label 前缀，不依赖远端设备名）----------

    /** 远端刚写入剪贴板的内容指纹 → 写入时间戳(ms)。读到此指纹且在窗口内即跳过。 */
    private val recentInboundEchoes = java.util.LinkedHashMap<String, Long>(32)
    private const val INBOUND_ECHO_WINDOW_MS = 1_000L
    private const val INBOUND_ECHO_CAPACITY = 8

    /** 远端写入剪贴板前调用：记下内容指纹，本端随后捕获到此内容时视为回声跳过。 */
    @Synchronized
    fun noteInboundEcho(text: String) {
        if (text.isBlank()) return
        recentInboundEchoes[text] = android.os.SystemClock.elapsedRealtime()
        while (recentInboundEchoes.size > INBOUND_ECHO_CAPACITY) {
            val eldest = recentInboundEchoes.entries.first()
            recentInboundEchoes.remove(eldest.key)
        }
    }

    @Synchronized
    fun isRecentEcho(text: String): Boolean {
        val at = recentInboundEchoes[text] ?: return false
        if (android.os.SystemClock.elapsedRealtime() - at > INBOUND_ECHO_WINDOW_MS) {
            recentInboundEchoes.remove(text)
            return false
        }
        return true
    }
}
