package com.shurufa.ime

import android.content.Context

/** 输入法界面读取的最近一条入站内容；网络队列由后台服务独占消费。 */
internal object SyncInbox {
    data class Event(
        val kind: String,
        val from: String,
        val payload: String,
        val token: Long,
    )

    private const val PREFS = "sync_inbox"
    private const val KEY_KIND = "kind"
    private const val KEY_FROM = "from"
    private const val KEY_PAYLOAD = "payload"
    private const val KEY_TOKEN = "token"

    @Synchronized
    fun save(context: Context, kind: String, from: String, payload: String): Event {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val token = maxOf(System.currentTimeMillis(), prefs.getLong(KEY_TOKEN, 0L) + 1)
        val event = Event(kind, from, payload, token)
        prefs.edit()
            .putString(KEY_KIND, event.kind)
            .putString(KEY_FROM, event.from)
            .putString(KEY_PAYLOAD, event.payload)
            .putLong(KEY_TOKEN, event.token)
            .apply()
        return event
    }

    fun load(context: Context): Event? {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val kind = prefs.getString(KEY_KIND, null) ?: return null
        val from = prefs.getString(KEY_FROM, "").orEmpty()
        val payload = prefs.getString(KEY_PAYLOAD, "").orEmpty()
        val token = prefs.getLong(KEY_TOKEN, 0L)
        return Event(kind, from, payload, token)
    }

    fun clear(context: Context, token: Long) {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (prefs.getLong(KEY_TOKEN, Long.MIN_VALUE) == token) prefs.edit().clear().apply()
    }
}

/** JNI 入站字段协议的集中解析器。 */
internal object SyncIncomingProtocol {
    data class Incoming(val kind: String, val from: String, val payload: String)

    fun parse(raw: String): Incoming? {
        if (raw.isEmpty()) return null
        val parts = raw.split('\u0001')
        if (parts.size < 3 || parts[0] !in setOf("text", "image", "file")) return null
        return Incoming(parts[0], parts[1], parts.drop(2).joinToString("\u0001"))
    }
}
