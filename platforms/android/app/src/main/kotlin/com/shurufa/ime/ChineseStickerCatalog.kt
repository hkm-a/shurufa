package com.shurufa.ime

import android.content.Context
import org.json.JSONObject
import java.io.File
import java.net.URL

/** ChineseBQB 中文表情包索引的按需下载与本地缓存。 */
internal class ChineseStickerCatalog(context: Context) {
    private val cache = File(context.filesDir, "stickers/chinesebqb-index.json")

    @Volatile
    private var entries: List<ChineseStickerSearch.Entry> = emptyList()

    val isLoaded: Boolean
        get() = entries.isNotEmpty()

    fun load(onReady: () -> Unit) {
        if (entries.isNotEmpty()) return
        Thread {
            val raw = runCatching {
                cache.takeIf(File::isFile)?.readText()
                    ?: URL(INDEX_URL).readText().also {
                        cache.parentFile?.mkdirs()
                        cache.writeText(it)
                    }
            }.getOrNull() ?: return@Thread
            entries = parse(raw)
            if (entries.isNotEmpty()) onReady()
        }.start()
    }

    fun search(terms: List<String>, limit: Int = 3): List<ChineseStickerSearch.Entry> =
        ChineseStickerSearch.search(terms, entries, limit)

    private fun parse(raw: String): List<ChineseStickerSearch.Entry> {
        val data = JSONObject(raw).optJSONArray("data") ?: return emptyList()
        return buildList {
            for (index in 0 until data.length()) {
                val item = data.optJSONObject(index) ?: continue
                val name = item.optString("name")
                val category = item.optString("category")
                val url = cdnUrl(item.optString("url"))
                if (name.isNotBlank() && category.isNotBlank() && url.startsWith("https://")) {
                    add(ChineseStickerSearch.Entry(name, category, url))
                }
            }
        }
    }

    private companion object {
        /**
         * GitHub Raw 在部分中文网络下不可达；jsDelivr 同步同一仓库，并可直接提供索引与图片。
         */
        const val INDEX_URL = "https://cdn.jsdelivr.net/gh/zhaoolee/ChineseBQB@master/chinesebqb_github.json"
        const val GITHUB_RAW_PREFIX = "https://raw.githubusercontent.com/zhaoolee/ChineseBQB/master/"
        const val CDN_PREFIX = "https://cdn.jsdelivr.net/gh/zhaoolee/ChineseBQB@master/"
    }

    private fun cdnUrl(url: String): String =
        url.replace(GITHUB_RAW_PREFIX, CDN_PREFIX)
}
