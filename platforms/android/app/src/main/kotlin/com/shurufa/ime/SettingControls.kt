package com.shurufa.ime

import android.content.Context
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import android.widget.Switch
import android.widget.TextView

/**
 * P3 借鉴搜狗 Preference 控件库（com.sogou.lib.preference 共 38 类）：
 * 设置面板可复用控件的最小集，全部由同一 palette 驱动，样式全局统一；
 * 后续新增设置页（剪贴板同步 / 隐私 / 数据同步等）直接复用，避免样式漂移。
 */
internal class SettingControls(
    private val ctx: Context,
    private val palette: ShurufaImeService.Palette,
    private val dark: Boolean,
) {

    private fun dp(value: Float): Int = (value * ctx.resources.displayMetrics.density).toInt()

    /** SogouCategory 分组标题：小号灰色加粗。 */
    fun category(title: String): TextView = TextView(ctx).apply {
        text = title
        textSize = 12f
        typeface = Typeface.DEFAULT_BOLD
        setTextColor(palette.panelMuted)
        setPadding(0, dp(14f), 0, dp(3f))
    }

    /** 面板主标题（加粗）。 */
    fun panelTitle(title: String): TextView = TextView(ctx).apply {
        text = title
        textSize = 16f
        typeface = Typeface.DEFAULT_BOLD
        setTextColor(palette.panelText)
    }

    /** 灰色说明副文本（开关项用途/状态说明）。 */
    fun subtext(text: String): TextView = TextView(ctx).apply {
        this.text = text
        textSize = 11f
        setTextColor(if (dark) 0xFF8E949D.toInt() else 0xFF9AA0AA.toInt())
        setPadding(0, dp(1f), 0, dp(2f))
    }

    /** 标题 + 开关行（SogouSwitchPreference）。 */
    fun switchRow(title: String, initial: Boolean, onChange: (Boolean) -> Unit): LinearLayout =
        LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(0, dp(8f), 0, 0)
            addView(
                TextView(ctx).apply {
                    text = title
                    textSize = 14f
                    setTextColor(palette.panelText)
                },
                LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f),
            )
            addView(
                Switch(ctx).apply {
                    isChecked = initial
                    setOnCheckedChangeListener { _, checked -> onChange(checked) }
                },
            )
        }

    /**
     * 单选组 chips（SogouRadioGroupPreference 简化版）：返回容器与刷新函数，
     * 选中项实心 accent，其余浅底。
     */
    fun <T> radioGroup(
        options: List<Pair<T, String>>,
        initial: T,
        onSelect: (T) -> Unit,
    ): Pair<LinearLayout, (T) -> Unit> {
        val row = LinearLayout(ctx).apply { orientation = LinearLayout.HORIZONTAL }
        val chips = mutableListOf<TextView>()
        fun render(selected: T) {
            options.forEachIndexed { i, (value, _) ->
                val chip = chips[i]
                val active = value == selected
                chip.setTextColor(if (active) 0xFFFFFFFF.toInt() else palette.panelText)
                (chip.background as? GradientDrawable)?.setColor(
                    if (active) palette.accent else palette.keyPressed
                )
            }
        }
        options.forEach { (value, label) ->
            val chip = TextView(ctx).apply {
                text = label
                textSize = 13f
                gravity = Gravity.CENTER
                setPadding(dp(18f), dp(8f), dp(18f), dp(8f))
                background = GradientDrawable().apply { cornerRadius = dp(8f).toFloat() }
                setOnClickListener { render(value); onSelect(value) }
            }
            chips.add(chip)
            row.addView(
                chip,
                LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                ).apply { setMargins(0, 0, dp(10f), 0) },
            )
        }
        render(initial)
        return row to ::render
    }

    /** 可点击设置行（SogouLinkPreference）：标题 + 可选右侧当前值，点击回调。 */
    fun linkRow(title: String, value: String? = null, onClick: () -> Unit): TextView =
        TextView(ctx).apply {
            text = if (value == null) title else title + "    " + value
            textSize = 14f
            setTextColor(palette.panelText)
            setPadding(0, dp(10f), 0, dp(2f))
            setOnClickListener { onClick() }
        }

    /** 分隔线（SogouDividerPreference）。 */
    fun divider(): View = View(ctx).apply {
        setBackgroundColor(if (dark) 0xFF3A3F47.toInt() else 0xFFD8DCE3.toInt())
    }
}
