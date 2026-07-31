package com.shurufa.ime

import android.content.Context
import java.io.File

/**
 * 手机端剪贴板历史（clip_jni.rs，复用桌面 clipboard-store）。
 * 列表协议：记录以 RECORD 分隔，字段 `id / 来源 / 文本` 以 FIELD 分隔。
 */
object ClipStore {
    init {
        System.loadLibrary("shurufa_rime")
    }

    private const val FIELD = "\u0001"
    private const val RECORD = "\u0002"

    external fun nativeInit(dbPath: String): Boolean
    external fun nativeInsert(text: String, source: String)
    external fun nativeList(limit: Int): String
    external fun nativeImageData(id: Int): ByteArray
    external fun nativeDelete(id: Int)

    data class Entry(val id: Int, val kind: String, val source: String, val text: String)

    @Volatile
    private var ready = false

    @Synchronized
    fun ensureInit(context: Context): Boolean {
        if (ready) return true
        val db = File(context.filesDir, "clipboard.db")
        ready = nativeInit(db.absolutePath)
        return ready
    }

    fun insert(text: String, source: String) {
        if (ready && text.isNotBlank()) nativeInsert(text, source)
    }

    fun list(limit: Int = 30): List<Entry> {
        if (!ready) return emptyList()
        val raw = nativeList(limit)
        if (raw.isEmpty()) return emptyList()
        return raw.split(RECORD).mapNotNull { rec ->
            val f = rec.split(FIELD)
            if (f.size >= 4) {
                val id = f[0].toIntOrNull() ?: return@mapNotNull null
                Entry(id, f[1], f[2], f.subList(3, f.size).joinToString(FIELD))
            } else null
        }
    }

    fun delete(id: Int) {
        if (ready) nativeDelete(id)
    }

    /** 图片条目 PNG 字节；非图片或不存在返回 null。 */
    fun imageData(id: Int): ByteArray? {
        if (!ready) return null
        val b = try { nativeImageData(id) } catch (e: Throwable) { return null }
        return if (b.isEmpty()) null else b
    }
}
