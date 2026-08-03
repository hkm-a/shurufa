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
