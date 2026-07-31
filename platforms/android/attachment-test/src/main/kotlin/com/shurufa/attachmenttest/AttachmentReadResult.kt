package com.shurufa.attachmenttest

internal data class AttachmentReadResult(
    val source: String,
    val label: String,
    val uri: String,
    val declaredMimeTypes: List<String>,
    val resolvedMimeType: String?,
    val byteCount: Long?,
    val error: String?,
) {
    fun asDisplayText(): String {
        val size = byteCount?.let { "$it 字节" } ?: "未读取"
        val failure = error?.let { "\n错误：$it" }.orEmpty()
        return "来源：$source\n" +
            "标签：$label\n" +
            "URI：$uri\n" +
            "声明 MIME：${declaredMimeTypes.joinToString().ifEmpty { "无" }}\n" +
            "解析 MIME：${resolvedMimeType ?: "未知"}\n" +
            "读取大小：$size$failure"
    }
}
