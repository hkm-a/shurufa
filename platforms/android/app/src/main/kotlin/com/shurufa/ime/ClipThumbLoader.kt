package com.shurufa.ime

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * 剪贴板历史图片缩略图加载辅助（RecyclerView+Coil 接入）。
 *
 * 图片字节在 IO 线程从 ClipStore 读出，交给 Coil `load(ByteArray)` 按
 * THUMBNAIL_TARGET 采样解码；`memoryCacheKey` 用条目 id 保持跨滚动命中。
 * 替代旧版打开面板时一次性预解码 100 张 Bitmap 常驻内存的 PreparedHistory。
 */
object ClipThumbLoader {
    const val KEY_PREFIX = "clip-thumb://"

    /** IO 线程读原图字节；条目已删/读库失败返回 null（行内图片留空）。 */
    suspend fun bytes(entryId: Int): ByteArray? = withContext(Dispatchers.IO) {
        try {
            ClipStore.imageData(entryId)
        } catch (e: Throwable) {
            null
        }
    }
}
