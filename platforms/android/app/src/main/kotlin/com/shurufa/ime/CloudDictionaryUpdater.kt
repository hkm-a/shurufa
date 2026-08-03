package com.shurufa.ime

import android.content.Context
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * 自托管云词库更新器。
 *
 * 更新源是 HTTPS JSON 清单，清单中的每个 Rime YAML 都带 SHA-256 和精确
 * 字节数。所有内容先写入应用私有暂存目录，全部校验通过后才替换完整覆盖
 * 包；下次初始化引擎时覆盖到解包方案并重新部署。
 */
internal object CloudDictionaryUpdater {
    private const val MAX_MANIFEST_BYTES = 1024 * 1024
    private const val MAX_FILE_BYTES = 32 * 1024 * 1024
    private const val MAX_FILES = 32

    data class Manifest(val revision: String, val files: List<Entry>)
    data class Entry(
        val path: String,
        val url: String,
        val fallbackUrls: List<String>,
        val sha256: String,
        val size: Int,
    )

    fun update(context: Context, manifestUrl: String): Result<String> = runCatching {
        val source = manifestUrl.trim()
        val manifest = if (source.equals("rime-ice", ignoreCase = true)) {
            context.assets.open("schemas/rime-ice-2026.06.30.json").use { parseManifest(it.readBytes()) }
        } else {
            requireHttps(source)
            parseManifest(download(source, MAX_MANIFEST_BYTES))
        }
        val filesRoot = File(context.filesDir, "cloud-dicts")
        val staging = File(context.filesDir, "cloud-dicts.staging")
        staging.deleteRecursively()
        check(staging.mkdirs()) { "无法创建词库暂存目录" }
        try {
            manifest.files.forEach { entry ->
                val content = downloadVerified(entry)
                val target = safeChild(staging, entry.path)
                target.parentFile?.mkdirs()
                target.writeBytes(content)
            }
            File(staging, "manifest.json").writeText(
                JSONObject().put("revision", manifest.revision).toString(),
            )
            replaceDirectory(staging, filesRoot)
            File(context.filesDir, "dict-update").apply { mkdirs() }
                .resolve("source.url")
                .writeText(source)
            manifest.revision
        } catch (error: Throwable) {
            staging.deleteRecursively()
            throw error
        }
    }

    /** 将已验证的远端词典覆盖到 Rime 共享目录；返回本次是否改变了方案。 */
    fun applyOverlay(context: Context, schemas: File): Boolean {
        val overlay = File(context.filesDir, "cloud-dicts")
        val revision = runCatching {
            JSONObject(File(overlay, "manifest.json").readText()).getString("revision")
        }.getOrNull() ?: return false
        val marker = File(schemas, ".cloud-dict-revision")
        if (marker.takeIf { it.isFile }?.readText() == revision) return false
        copyTree(overlay, schemas, skipName = "manifest.json")
        File(schemas, "build").deleteRecursively()
        marker.writeText(revision)
        return true
    }

    fun source(context: Context): String = runCatching {
        File(context.filesDir, "dict-update/source.url").readText().trim()
    }.getOrDefault("rime-ice").ifEmpty { "rime-ice" }

    internal fun parseManifest(bytes: ByteArray): Manifest {
        check(bytes.size <= MAX_MANIFEST_BYTES) { "词库清单超过大小上限" }
        val objectValue = JSONObject(bytes.toString(Charsets.UTF_8))
        check(objectValue.optInt("version", 0) == 1) { "词库清单版本不受支持" }
        val revision = objectValue.optString("revision").trim()
        check(revision.isNotEmpty()) { "词库清单缺少版本号" }
        val array = objectValue.optJSONArray("files") ?: error("词库清单缺少文件列表")
        check(array.length() in 1..MAX_FILES) { "词库清单文件数无效" }
        val entries = buildList {
            for (index in 0 until array.length()) {
                val item = array.getJSONObject(index)
                val entry = Entry(
                    path = item.getString("path"),
                    url = item.getString("url"),
                    fallbackUrls = item.optJSONArray("fallback_urls")?.let { fallbackUrls ->
                        List(fallbackUrls.length()) { fallbackUrls.getString(it) }
                    }.orEmpty(),
                    sha256 = item.getString("sha256"),
                    size = item.getInt("size"),
                )
                validateEntry(entry)
                add(entry)
            }
        }
        return Manifest(revision, entries)
    }

    internal fun verify(entry: Entry, content: ByteArray) {
        check(content.size == entry.size) { "词库大小校验失败：${entry.path}" }
        check(sha256(content).equals(entry.sha256, ignoreCase = true)) {
            "词库 SHA-256 校验失败：${entry.path}"
        }
    }

    internal fun fitsDownloadLimit(advertisedSize: Int, limit: Int): Boolean =
        advertisedSize < 0 || advertisedSize <= limit

    private fun validateEntry(entry: Entry) {
        check(entry.path.endsWith(".yaml") && entry.path.isNotBlank()) { "词库路径非法" }
        check(!entry.path.contains('\\') && entry.path.split('/').none { it.isEmpty() || it == "." || it == ".." }) {
            "词库路径非法"
        }
        (listOf(entry.url) + entry.fallbackUrls).forEach(::requireHttps)
        check(entry.size in 1..MAX_FILE_BYTES) { "词库文件大小无效" }
        check(entry.sha256.length == 64 && entry.sha256.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' }) {
            "词库 SHA-256 格式无效"
        }
    }

    private fun requireHttps(url: String) {
        check(url.trim().startsWith("https://")) { "词库地址必须使用 HTTPS" }
    }

    private fun download(url: String, limit: Int): ByteArray {
        val connection = (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = 15_000
            readTimeout = 30_000
            instanceFollowRedirects = false
            requestMethod = "GET"
        }
        try {
            check(connection.responseCode in 200..299) { "下载失败：HTTP ${connection.responseCode}" }
            check(fitsDownloadLimit(connection.contentLength, limit)) { "下载内容超过大小上限" }
            connection.inputStream.use { input ->
                val output = ByteArrayOutputStream()
                val buffer = ByteArray(8192)
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    output.write(buffer, 0, count)
                    check(output.size() <= limit) { "下载内容超过大小上限" }
                }
                return output.toByteArray()
            }
        } finally {
            connection.disconnect()
        }
    }

    private fun downloadVerified(entry: Entry): ByteArray {
        val errors = mutableListOf<String>()
        (listOf(entry.url) + entry.fallbackUrls).forEach { url ->
            runCatching {
                download(url, entry.size).also { verify(entry, it) }
            }.onSuccess { return it }.onFailure { errors += "$url：${it.message}" }
        }
        error("所有词库下载源均失败：${errors.joinToString("；")}")
    }

    private fun safeChild(root: File, relative: String): File {
        val candidate = File(root, relative)
        check(candidate.canonicalPath.startsWith(root.canonicalPath + File.separator)) { "词库路径越界" }
        return candidate
    }

    private fun replaceDirectory(staging: File, destination: File) {
        val backup = File(destination.parentFile, "cloud-dicts.previous")
        backup.deleteRecursively()
        if (destination.exists()) check(destination.renameTo(backup)) { "备份旧词库失败" }
        if (!staging.renameTo(destination)) {
            if (backup.exists()) backup.renameTo(destination)
            error("启用新词库失败")
        }
        backup.deleteRecursively()
    }

    private fun copyTree(source: File, destination: File, skipName: String? = null) {
        source.listFiles()?.forEach { item ->
            if (item.name == skipName) return@forEach
            val target = File(destination, item.name)
            if (item.isDirectory) {
                target.mkdirs()
                copyTree(item, target, skipName)
            } else {
                target.parentFile?.mkdirs()
                item.inputStream().use { input -> target.outputStream().use { input.copyTo(it) } }
            }
        }
    }

    private fun sha256(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256")
        .digest(bytes)
        .joinToString("") { "%02x".format(it) }
}
