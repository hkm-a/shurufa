package com.shurufa.ime

import android.content.Context
import java.io.File
import org.json.JSONObject

/** 键盘与候选窗共用的跨端皮肤颜色语义（ARGB）。 */
internal data class SkinPalette(
    val bg: Int,
    val key: Int,
    val keyPressed: Int,
    val keyFunc: Int,
    val funcPressed: Int,
    val keyText: Int,
    val funcText: Int,
    val candidate: Int,
    val candidateHl: Int,
    val preedit: Int,
    val accent: Int,
) {
    companion object {
        fun lightDefault() = SkinPalette(
            bg = 0xFFF2F3F5.toInt(),
            key = 0xFFFFFFFF.toInt(),
            keyPressed = 0xFFDEE4EC.toInt(),
            keyFunc = 0xFFD5DAE3.toInt(),
            funcPressed = 0xFFBFC6D2.toInt(),
            keyText = 0xFF111418.toInt(),
            funcText = 0xFF3A4048.toInt(),
            candidate = 0xFF15181C.toInt(),
            candidateHl = 0xFF1B9E77.toInt(),
            preedit = 0xFF9AA2AB.toInt(),
            accent = 0xFF1B9E77.toInt(),
        )

        fun darkDefault() = SkinPalette(
            bg = 0xFF121418.toInt(),
            key = 0xFF282C33.toInt(),
            keyPressed = 0xFF414850.toInt(),
            keyFunc = 0xFF32373E.toInt(),
            funcPressed = 0xFF40464E.toInt(),
            keyText = 0xFFF0F1F3.toInt(),
            funcText = 0xFFC9CED4.toInt(),
            candidate = 0xFFF0F1F3.toInt(),
            candidateHl = 0xFF4ECDA2.toInt(),
            preedit = 0xFF8E9399.toInt(),
            accent = 0xFF4ECDA2.toInt(),
        )

        /** 用共享皮肤 JSON 解析指定变体；任一缺失或非法字段回退到默认颜色。 */
        fun fromJson(text: String, dark: Boolean, fallback: SkinPalette): SkinPalette {
            return try {
                val root = JSONObject(text)
                if (root.optInt("version", 0) != 1) return fallback
                val variant = root.optJSONObject(if (dark) "dark" else "light") ?: return fallback
                val keyboard = variant.optJSONObject("keyboard") ?: return fallback
                val candidate = variant.optJSONObject("candidate") ?: return fallback
                SkinPalette(
                    bg = color(keyboard, "background", fallback.bg),
                    key = color(keyboard, "key", fallback.key),
                    keyPressed = color(keyboard, "key_pressed", fallback.keyPressed),
                    keyFunc = color(keyboard, "function_key", fallback.keyFunc),
                    funcPressed = color(keyboard, "function_key_pressed", fallback.funcPressed),
                    keyText = color(keyboard, "text", fallback.keyText),
                    funcText = color(keyboard, "function_text", fallback.funcText),
                    candidate = color(candidate, "text", fallback.candidate),
                    candidateHl = color(keyboard, "accent", fallback.candidateHl),
                    preedit = color(keyboard, "muted_text", fallback.preedit),
                    accent = color(keyboard, "accent", fallback.accent),
                )
            } catch (_: Exception) {
                fallback
            }
        }

        private fun color(objectValue: JSONObject, key: String, fallback: Int): Int {
            val value = objectValue.optString(key, "")
            if (!value.matches(Regex("^#[0-9A-Fa-f]{6}([0-9A-Fa-f]{2})?$"))) return fallback
            return try {
                val rgb = value.substring(1)
                val argb = if (rgb.length == 6) "FF$rgb" else rgb
                argb.toLong(16).toInt()
            } catch (_: NumberFormatException) {
                fallback
            }
        }
    }
}

/** 皮肤加载顺序：应用私有覆盖文件优先，否则读取随方案一起打包的默认资产。 */
internal object SkinConfig {
    private const val FILE_NAME = "shurufa-skin.json"
    private const val ASSET_NAME = "schemas/shurufa-skin.json"

    fun load(context: Context, dark: Boolean): SkinPalette {
        val fallback = if (dark) SkinPalette.darkDefault() else SkinPalette.lightDefault()
        val custom = File(context.filesDir, FILE_NAME)
        val text = try {
            if (custom.isFile) custom.readText()
            else context.assets.open(ASSET_NAME).bufferedReader().use { it.readText() }
        } catch (_: Exception) {
            return fallback
        }
        return SkinPalette.fromJson(text, dark, fallback)
    }
}
