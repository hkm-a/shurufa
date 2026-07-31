package com.shurufa.ime

import android.content.Context
import android.content.res.ColorStateList
import android.content.res.Resources
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.StateListDrawable
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject

/**
 * 微信输入法键盘布局渲染器。
 *
 * 直接消费反编译得到的微信输入法键盘 JSON（assets/keyboard 目录下的键盘 JSON，1080×628 设计稿）：
 * 布局 100% 照抄微信（行高比例、键宽比例、圆角、图标键、功能键排布），
 * 配色用我们自己的主题（浅色白键 / 深色深灰键）。
 *
 * 微信 S2 主键盘实证布局（S2ChineseQwertyKeyboard.json）：
 *   行1: Q W E R T Y U I O P          (h=156)
 *   行2: A S D F G H J K L            (h=158)
 *   行3: [⇧capslock] Z X C V B N M [⌫] (h=158)
 *   行4: [123] [😊] [，] [换行] [空格] [中/英] (h=156)
 * 微信 S13 数字符号页：数字行 + 符号行 + 标点行 + 底部功能行
 */
class WetypeKeyboardView(
    context: Context,
    private val langLabel: String?, // 中英键动态文字（null = 不显示）
    json: JSONObject,
    private val dark: Boolean,
    private val onAction: (WetypeAction) -> Unit,
) : LinearLayout(context) {

    /** 键盘动作（微信 JSON 键 → 我们的行为） */
    sealed class WetypeAction {
        data class Char(val c: String) : WetypeAction()
        object Backspace : WetypeAction()
        object Shift : WetypeAction()
        object NumberPage : WetypeAction()
        object EmojiPage : WetypeAction()
        object BackPage : WetypeAction()
        object SymbolMore : WetypeAction()
        object Enter : WetypeAction()
        object Space : WetypeAction()
        object Lang : WetypeAction()
    }

    // ---- 主题色（我们自己的配色，只借微信布局） ----
    private val keyColor = if (dark) 0xFF2B2F36.toInt() else 0xFFFFFFFF.toInt()
    private val keyPressedColor = if (dark) 0xFF474D57.toInt() else 0xFFC7CED9.toInt()
    private val funcColor = if (dark) 0xFF373C44.toInt() else 0xFFD3D8E1.toInt()
    private val funcPressedColor = if (dark) 0xFF4A5059.toInt() else 0xFFBFC6D2.toInt()
    private val textColor = if (dark) 0xFFE6E8EB.toInt() else 0xFF1A1A1A.toInt()
    private val funcTextColor = if (dark) 0xFFE6E8EB.toInt() else 0xFF33383F.toInt()

    /** 设计稿缩放：1080 宽设计稿 → 实际屏幕 */
    private val scale = Resources.getSystem().displayMetrics.widthPixels / 1080f
    /** 键盘主体（4 行）高度 = 屏幕 39%（微信输入法同款比例） */
    private val kbHeight = (Resources.getSystem().displayMetrics.heightPixels * 0.39f).toInt()

    init {
        orientation = VERTICAL
        // 行高按微信 JSON lineHeight 比例分配
        val config = json.getJSONObject("config")
        val lineHeights = parseLineHeights(config)
        val sum = lineHeights.sum()
        val rows = json.getJSONArray("keys")
        for (r in 0 until rows.length()) {
            val rowH = (kbHeight * lineHeights[r] / sum).toInt()
            addView(buildRow(rows.getJSONArray(r), rowH), LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, rowH
            ))
        }
    }

    private fun buildRow(row: JSONArray, rowH: Int): LinearLayout =
        LinearLayout(context).apply {
            orientation = VERTICAL
            gravity = Gravity.CENTER_VERTICAL
            val keys = (0 until row.length()).map { parseKey(row.getJSONObject(it)) }
            val totalW = keys.fold(0f) { acc, k -> acc + k.width }
            // 行内容水平居中（微信 lineAlign=3 居中）
            val rowContent = LinearLayout(context).apply {
                orientation = HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
            }
            for (k in keys) {
                val v = buildKey(k, rowH)
                rowContent.addView(
                    v,
                    LinearLayout.LayoutParams(0, (rowH * 0.92f).toInt(), k.width)
                        .apply { setMargins(px(2f), 0, px(2f), 0) },
                )
            }
            addView(
                rowContent,
                LinearLayout.LayoutParams(
                    (totalW * scale).toInt(), LinearLayout.LayoutParams.MATCH_PARENT,
                ).apply { gravity = Gravity.CENTER },
            )
        }

    private class WetypeKey(
        val main: String?,
        val img: String?,
        val width: Float,
    )

    private fun parseKey(o: JSONObject): WetypeKey {
        val main = first(o, "mainText")?.takeIf { it.isNotBlank() }
        val img = first(o, "imgSrc")?.removePrefix("@drawable/")?.takeIf { it.isNotBlank() }
        val w = first(o, "width")?.toFloatOrNull() ?: 92f
        return WetypeKey(main, img, w)
    }

    /** JSON 字段可能是 JSON 数组或「变体1 变体2 变体3」字符串，取第一个（中文变体） */
    private fun first(o: JSONObject, key: String): String? {
        if (!o.has(key)) return null
        return when (val v = o.get(key)) {
            is JSONArray -> v.getString(0).trim().takeIf { it.isNotEmpty() }
            else -> v.toString().trim().split(' ').firstOrNull()?.takeIf { it.isNotEmpty() }
        }
    }

    /** lineHeight：JSON 数组 [156,158,158,156] 或空格分隔字符串 */
    private fun parseLineHeights(config: JSONObject): List<Float> {
        if (!config.has("lineHeight")) return emptyList()
        return when (val v = config.get("lineHeight")) {
            is JSONArray -> (0 until v.length()).map { v.getDouble(it).toFloat() }
            else -> v.toString().trim().split(' ').map { it.toFloat() }
        }
    }

    private fun buildKey(k: WetypeKey, rowH: Int): View {
        val iconRes = when (k.img) {
            "icon_keys_123" -> R.drawable.icon_keys_123
            "icon_keys_emoji" -> R.drawable.icon_keys_emoji
            "icon_keys_delete" -> R.drawable.icon_keys_delete
            "icon_keys_outlinedcapslock" -> R.drawable.icon_keys_outlinedcapslock
            "icon_keys_back" -> R.drawable.icon_keys_back
            "icon_keys_num_t9" -> R.drawable.icon_keys_num_t9
            else -> null
        }
        val isFunc = iconRes != null || k.main == "换行" || k.main == "符号" ||
            (k.main == null && k.img == null) || k.width > 100f
        val normal = if (isFunc) funcColor else keyColor
        val pressed = if (isFunc) funcPressedColor else keyPressedColor
        val corner = px(6f).toFloat() // 微信 bgCorner=18px@1080 ≈ 6dp
        val bg = StateListDrawable().apply {
            addState(intArrayOf(android.R.attr.state_pressed), round(pressed, corner))
            addState(intArrayOf(), round(normal, corner))
        }

        if (iconRes != null) {
            return ImageView(context).apply {
                setImageResource(iconRes)
                // 微信图标 fillColor 黑色，按我们的功能键文字色着色
                imageTintList = ColorStateList.valueOf(funcTextColor)
                background = bg
                contentDescription = k.img
                setOnClickListener {
                    onAction(
                        when (k.img) {
                            "icon_keys_123" -> WetypeAction.NumberPage
                            "icon_keys_emoji" -> WetypeAction.EmojiPage
                            "icon_keys_delete" -> WetypeAction.Backspace
                            "icon_keys_outlinedcapslock" -> WetypeAction.Shift
                            "icon_keys_back" -> WetypeAction.BackPage
                            "icon_keys_num_t9" -> WetypeAction.BackPage // 数字页里的 t9 键：返回主键盘
                            else -> WetypeAction.Char(k.main ?: "")
                        }
                    )
                }
            }
        }

        val label = if (k.main.isNullOrEmpty() && k.img == null && k.width <= 100f) (langLabel ?: "") else (k.main ?: "")
        return TextView(context).apply {
            text = label
            gravity = Gravity.CENTER
            setTextColor(if (isFunc) funcTextColor else textColor)
            // 微信 mainTextSize=64px@1080 ≈ 21dp；换行等文字键略小
            setTextSize(TypedValue.COMPLEX_UNIT_SP, if (label == "换行" || label == "符号") 13f else 20f)
            if (label == "换行" || label == "符号") typeface = Typeface.DEFAULT_BOLD
            background = bg
            // 微信 shadowHeight=2px@1080：elevation 浅阴影
            elevation = px(0.8f).toFloat()
            setOnClickListener {
                onAction(
                    when (label) {
                        "换行" -> WetypeAction.Enter
                        "符号" -> WetypeAction.SymbolMore
                        else -> when {
                            label.isEmpty() && k.width > 100f -> WetypeAction.Space
                            label.isEmpty() -> WetypeAction.Lang
                            else -> WetypeAction.Char(label)
                        }
                    }
                )
            }
        }
    }

    private fun round(color: Int, corner: Float) = GradientDrawable().apply {
        setColor(color)
        cornerRadius = corner
    }

    private fun px(dp: Float): Int = (dp * Resources.getSystem().displayMetrics.density).toInt()
}
