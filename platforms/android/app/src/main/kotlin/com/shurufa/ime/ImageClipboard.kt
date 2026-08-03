package com.shurufa.ime

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import androidx.core.content.FileProvider
import java.io.File

/**
 * 把同步图片写入系统剪贴板的共享工具（接收链路与输入法发送兜底共用）。
 *
 * Android 10 及以上优先使用 MediaStore URI，确保系统图片剪贴板和相册应用都能读取；
 * 旧系统使用 FileProvider 临时文件（filesDir/sync/received）。发送失败需要用户从相册发图时，
 * 用 [saveToGallery] 显式写入 Pictures/Shurufa。
 *
 * 剪贴板标签统一以「Shurufa 同步·」开头，供剪贴板监听器识别为本机回填、
 * 避免回环再同步。写入方需自行处理 FileProvider URI 对目标包的显式授权。
 */
internal object ImageClipboard {

    /** 与 ClipboardTypePolicy.REMOTE_LABEL_PREFIX 保持一致的本机回填前缀。 */
    const val LABEL_PREFIX = "Shurufa 同步·"
    const val IMAGE_MIME_TYPE = "image/png"

    /** 把 PNG 字节落为系统可读取的 content URI。 */
    fun writeUri(context: Context, png: ByteArray): Uri {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            return writeMediaStore(ContentResolverImageStore(context.contentResolver), png)
        }
        return writeFileProvider(context, png)
    }

    private fun writeFileProvider(context: Context, png: ByteArray): Uri {
        val dir = File(context.filesDir, "sync/received").apply { mkdirs() }
        val file = dir.resolve("image-${System.currentTimeMillis()}.png").apply { writeBytes(png) }
        return FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", file)
    }

    /** 以 URI 条目 + image/png 写入系统剪贴板，返回 URI；标签带本机回填前缀。 */
    fun setClipboard(context: Context, png: ByteArray, label: String): Uri {
        val uri = writeUri(context, png)
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val description = clipboardDescription(label)
        clipboard.setPrimaryClip(ClipData(description, ClipData.Item(uri)))
        return uri
    }

    internal fun clipboardLabel(label: String): String = LABEL_PREFIX + label

    internal fun clipboardDescription(label: String): ClipDescription =
        ClipDescription(clipboardLabel(label), arrayOf(IMAGE_MIME_TYPE))

    /** 使用 MediaStore 的原子写入；发生异常时删除已插入的半成品并抛出明确错误。 */
    internal fun writeMediaStore(store: ImageStore, png: ByteArray, timestamp: Long = System.currentTimeMillis()): Uri {
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, "shurufa-$timestamp.png")
            put(MediaStore.Images.Media.MIME_TYPE, IMAGE_MIME_TYPE)
            put(MediaStore.Images.Media.RELATIVE_PATH, "Pictures/Shurufa")
            put(MediaStore.Images.Media.IS_PENDING, 1)
        }
        return writeImageTransaction(
            create = { store.insert(values) },
            openOutput = store::openOutputStream,
            publish = { uri ->
                store.update(uri, ContentValues().apply {
                    put(MediaStore.Images.Media.IS_PENDING, 0)
                }) > 0
            },
            cleanup = store::delete,
            png = png,
        )
    }

    /**
     * 保存到系统相册 Pictures/Shurufa（Q+ MediaStore），返回 URI；失败返回 null。
     * 用于输入框既拒绝 commitContent 又不支持粘贴时的兜底：用户从相册「+」发送。
     */
    fun saveToGallery(context: Context, png: ByteArray): Uri? {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            try {
                val values = android.content.ContentValues().apply {
                    put(MediaStore.Images.Media.DISPLAY_NAME, "shurufa-${System.currentTimeMillis()}.png")
                    put(MediaStore.Images.Media.MIME_TYPE, "image/png")
                    put(MediaStore.Images.Media.RELATIVE_PATH, "Pictures/Shurufa")
                    put(MediaStore.Images.Media.IS_PENDING, 1)
                }
                val pendingUri = context.contentResolver.insert(
                    MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                    values,
                ) ?: return null
                return try {
                    context.contentResolver.openOutputStream(pendingUri)?.use { it.write(png) }
                        ?: return null
                    context.contentResolver.update(
                        pendingUri,
                        android.content.ContentValues().apply {
                            put(MediaStore.Images.Media.IS_PENDING, 0)
                        },
                        null,
                        null,
                    )
                    pendingUri
                } catch (e: Throwable) {
                    context.contentResolver.delete(pendingUri, null, null)
                    null
                }
            } catch (e: Throwable) {
                android.util.Log.w("shurufa", "保存到相册失败", e)
            }
        }
        return null
    }

    /** 对当前编辑器包显式授权 FileProvider URI（系统剪贴板自动授权之外的兜底）。 */
    fun grantTo(context: Context, uri: Uri, targetPackage: String?) {
        if (targetPackage.isNullOrBlank()) return
        if (uri.authority != "${context.packageName}.fileprovider") return
        try {
            context.grantUriPermission(targetPackage, uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        } catch (e: Throwable) {
            android.util.Log.w("shurufa", "剪贴板 URI 显式授权失败 目标=$targetPackage URI=$uri", e)
        }
    }
}

/** 将媒体记录的创建、写入、发布和失败清理固定为同一事务语义。 */
internal fun <T> writeImageTransaction(
    create: () -> T?,
    openOutput: (T) -> java.io.OutputStream?,
    publish: (T) -> Boolean,
    cleanup: (T) -> Int,
    png: ByteArray,
): T {
    val item = create() ?: throw IllegalStateException("写入系统图片库失败：无法创建图片条目")
    try {
        openOutput(item)?.use { it.write(png) }
            ?: throw IllegalStateException("写入系统图片库失败：无法打开输出流")
        if (!publish(item)) throw IllegalStateException("写入系统图片库失败：无法发布图片条目")
        return item
    } catch (e: Throwable) {
        try {
            cleanup(item)
        } catch (cleanupError: Throwable) {
            e.addSuppressed(cleanupError)
        }
        if (e is IllegalStateException && e.message?.startsWith("写入系统图片库失败") == true) {
            throw e
        }
        throw IllegalStateException("写入系统图片库失败：PNG 数据写入异常", e)
    }
}

/** MediaStore 最小操作面，便于验证失败回滚而无需启动 Android 设备。 */
internal interface ImageStore {
    fun insert(values: ContentValues): Uri?
    fun openOutputStream(uri: Uri): java.io.OutputStream?
    fun update(uri: Uri, values: ContentValues): Int
    fun delete(uri: Uri): Int
}

private class ContentResolverImageStore(private val resolver: ContentResolver) : ImageStore {
    override fun insert(values: ContentValues): Uri? =
        resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)

    override fun openOutputStream(uri: Uri): java.io.OutputStream? = resolver.openOutputStream(uri)

    override fun update(uri: Uri, values: ContentValues): Int = resolver.update(uri, values, null, null)

    override fun delete(uri: Uri): Int = resolver.delete(uri, null, null)
}
