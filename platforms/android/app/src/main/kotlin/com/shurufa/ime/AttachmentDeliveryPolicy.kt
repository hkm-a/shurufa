package com.shurufa.ime

/** 附件交给目标输入框时选择公开 Android 兼容路径。 */
internal object AttachmentDeliveryPolicy {
    private val imageMimePreference = listOf(
        "image/png",
        "image/jpeg",
        "image/webp",
    )

    /** 返回目标编辑器实际接受的图片 MIME；空值表示该输入框不支持图片附件。 */
    fun selectImageMimeType(declaredMimeTypes: Array<String>): String? {
        return imageMimePreference.firstOrNull { candidate ->
            declaredMimeTypes.any { declared -> mimeTypesMatch(candidate, declared) }
        }
    }

    fun supportsMimeType(mimeType: String, declaredMimeTypes: Array<String>): Boolean {
        return declaredMimeTypes.any { declared -> mimeTypesMatch(mimeType, declared) }
    }

    private fun mimeTypesMatch(actual: String, declared: String): Boolean {
        val actualParts = actual.lowercase().split('/', limit = 2)
        val declaredParts = declared.lowercase().split('/', limit = 2)
        if (actualParts.size != 2 || declaredParts.size != 2) return false
        val typeMatches = declaredParts[0] == "*" || actualParts[0] == declaredParts[0]
        val subtypeMatches = declaredParts[1] == "*" || actualParts[1] == declaredParts[1]
        return typeMatches && subtypeMatches
    }

    /** 识别输入法自己写入剪贴板的 FileProvider URI，进程重启后仍可阻止回环。 */
    fun isOwnProvider(authority: String?, packageName: String): Boolean {
        return authority == "$packageName.fileprovider"
    }
}
