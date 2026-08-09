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
    external fun nativeInsertImage(png: ByteArray, source: String)
    external fun nativeInsertFiles(paths: String, source: String)
    external fun nativeList(limit: Int): String
    external fun nativeImageData(id: Int): ByteArray
    external fun nativeDelete(id: Int)
    external fun nativeSetPinned(id: Int, pinned: Boolean)

    /** 子串搜索文本/文件历史，返回与 nativeList 相同的字段协议串。 */
    external fun nativeSearch(query: String, limit: Int): String

    /** 最新一条文本类条目的签名："updatedMs FIELD sha256(text)前16位hex"；无则空串（图片/文件返回空）。 */
    external fun nativeLatestSignature(): String

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

    fun insertImage(png: ByteArray, source: String) {
        if (ready && png.isNotEmpty()) nativeInsertImage(png, source)
    }

    fun insertFiles(paths: List<String>, source: String) {
        val value = paths.filter { it.isNotBlank() }.joinToString("\n")
        if (ready && value.isNotEmpty()) nativeInsertFiles(value, source)
    }

    fun list(limit: Int = 30): List<Entry> {
        if (!ready) return emptyList()
        val raw = try {
            nativeList(limit)
        } catch (e: Throwable) {
            return emptyList()
        }
        if (raw.isEmpty()) return emptyList()
        return parseEntries(raw)
    }

    /** 子串搜索历史（文本/文件），协议与 list 相同；失败/未就绪返回空。 */
    fun search(query: String, limit: Int = 30): List<Entry> {
        if (!ready || query.isBlank()) return emptyList()
        val raw = try {
            nativeSearch(query, limit)
        } catch (e: Throwable) {
            return emptyList()
        }
        if (raw.isEmpty()) return emptyList()
        return parseEntries(raw)
    }

    private fun parseEntries(raw: String): List<Entry> {
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

    /** 置顶/取消置顶；未就绪或异常时静默忽略。 */
    fun setPinned(id: Int, pinned: Boolean) {
        if (!ready) return
        try {
            nativeSetPinned(id, pinned)
        } catch (_: Throwable) {
        }
    }

    /** 最新文本条目签名 "updatedMs FIELD hash16"；未就绪/异常/无数据返回空串。 */
    fun latestSignature(): String {
        if (!ready) return ""
        return try {
            nativeLatestSignature() ?: ""
        } catch (e: Throwable) {
            ""
        }
    }

    /** 图片条目 PNG 字节；非图片或不存在返回 null。 */
    fun imageData(id: Int): ByteArray? {
        if (!ready) return null
        val b = try { nativeImageData(id) } catch (e: Throwable) { return null }
        return if (b.isEmpty()) null else b
    }
}
