package com.shurufa.ime

import android.content.Context
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.ConcurrentHashMap

/**
 * AI 候选预测（搜狗 13.0+ AI 化主线，2026-08-20 方案见 docs/AI候选预测方案.md）。
 *
 * 输入拼音暂停约 800ms 后，基于当前拼音与上文调 agnès 预测 1-3 个最可能的词，
 * 注入候选行（🤖 标记，排在引擎候选之后）。失败/无 key/超时一律静默返回 null。
 * 节流：同 preedit 结果 10s 缓存复用；请求期间不重复触发。
 */
class AiCandidateManager(private val context: Context) {

    companion object {
        const val CACHE_TTL_MS = 10_000L
        const val TIMEOUT_MS = 8_000L
        const val MAX_CANDIDATES = 3
        private val cache = ConcurrentHashMap<String, CacheEntry>()

        /** 提示词构造（纯函数，便于 JVM 单测）。 */
        fun buildPrompt(preedit: String, contextText: String): String {
            val ctx = contextText.trim().takeLast(80)
            return buildString {
                append("我正在用拼音输入法打字，当前输入的拼音是「")
                append(preedit)
                append("」，")
                if (ctx.isNotEmpty()) {
                    append("上文是「").append(ctx).append("」，")
                }
                append("请预测我接下来最可能输入的 $MAX_CANDIDATES 个词（单字或词语均可），")
                append("只输出词本身，用英文逗号分隔，不要编号、不要解释、不要引号。")
            }
        }

        /** 解析模型输出：按逗号切分、去空白、去重复、去空项，最多 MAX_CANDIDATES 个。 */
        fun parseCandidates(raw: String): List<String> {
            return raw.split(Regex("[，,]"))
                .map { it.trim().trim('"', '“', '”', '「', '」') }
                .filter { it.isNotEmpty() && it.length <= 20 }
                .distinct()
                .take(MAX_CANDIDATES)
        }
    }

    private data class CacheEntry(val candidates: List<String>, val at: Long)

    private fun apiKey(): String? {
        val prefs = context.getSharedPreferences("shurufa", Context.MODE_PRIVATE)
        return prefs.getString("ai_api_key", null)?.takeIf { it.isNotBlank() }
    }

    /**
     * 预测候选；同步阻塞（后台线程调用）。失败/无 key 返回 null。
     * @param preedit 当前拼音（非空）
     * @param contextText 已上屏上文（可空）
     */
    fun predict(preedit: String, contextText: String): List<String>? {
        val key = preedit
        cache[key]?.let { entry ->
            if (System.currentTimeMillis() - entry.at < CACHE_TTL_MS) return entry.candidates
            cache.remove(key)
        }
        val api = apiKey() ?: return null
        val candidates = try {
            callAgnes(api, buildPrompt(preedit, contextText))
        } catch (_: Throwable) {
            null
        }
        if (candidates != null) {
            cache[key] = CacheEntry(candidates, System.currentTimeMillis())
        }
        return candidates
    }

    private fun callAgnes(apiKey: String, prompt: String): List<String>? {
        val endpoint = URL("https://apihub.agnes-ai.com/v1/chat/completions")
        var conn: HttpURLConnection? = null
        return try {
            conn = (endpoint.openConnection() as HttpURLConnection).apply {
                requestMethod = "POST"
                connectTimeout = TIMEOUT_MS.toInt()
                readTimeout = TIMEOUT_MS.toInt()
                doOutput = true
                setRequestProperty("Content-Type", "application/json")
                setRequestProperty("Authorization", "Bearer $apiKey")
            }
            val body = org.json.JSONObject()
                .put("model", "agnes-2.5-flash")
                .put("stream", false)
                .put("temperature", 0.4)
                .put("messages", org.json.JSONArray()
                    .put(org.json.JSONObject()
                        .put("role", "system")
                        .put("content", "你是输入法的 AI 候选预测器。只输出候选词，用英文逗号分隔，不要解释、不要编号、不要引号、不要 Markdown。"))
                    .put(org.json.JSONObject().put("role", "user").put("content", prompt)))
                .toString()
            conn.outputStream.use { os -> os.write(body.toByteArray(Charsets.UTF_8)) }
            val code = conn.responseCode
            if (code !in 200..299) return null
            val text = conn.inputStream.bufferedReader(Charsets.UTF_8).use { it.readText() }
            val content = org.json.JSONObject(text)
                .optJSONArray("choices")?.optJSONObject(0)
                ?.optJSONObject("message")?.optString("content", "")
                .orEmpty()
            parseCandidates(content).ifEmpty { null }
        } catch (_: Throwable) {
            null
        } finally {
            conn?.disconnect()
        }
    }
}
