package com.shurufa.ime

import android.content.Context
import android.content.res.ColorStateList
import android.content.res.Configuration
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.StateListDrawable
import android.text.SpannableString
import android.text.Spanned
import android.text.style.ForegroundColorSpan
import android.util.TypedValue
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import kotlin.math.max
import kotlin.math.min

/**
 * 按微信输入法四行 QWERTY、五行数字符号的实际视觉顺序渲染键盘。
 *
 * 原始键盘导出数据含有「位于某键左侧/右侧」等相对定位约束，不能直接当作
 * LinearLayout 的顺序和权重使用；这里将其归一为明确的行规格，避免底栏错位。
 */
internal class WetypeKeyboardView(
    context: Context,
    page: KeyboardLayoutSpec.Page,
    private val dark: Boolean,
    private val asciiMode: Boolean,
    private val uppercaseLetters: Boolean,
    private val onAction: (WetypeAction) -> Unit,
) : LinearLayout(context) {

    sealed class WetypeAction {
        data class Char(val value: String) : WetypeAction()
        object Backspace : WetypeAction()
        object Shift : WetypeAction()
        object NumberPage : WetypeAction()
        object BackPage : WetypeAction()
        object Enter : WetypeAction()
        object Space : WetypeAction()
        object Lang : WetypeAction()
        object Clear : WetypeAction()
    }

    private val keyColor = if (dark) 0xFF2B2F36.toInt() else 0xFFFFFFFF.toInt()
    private val keyPressedColor = if (dark) 0xFF474D57.toInt() else 0xFFC7CED9.toInt()
    private val funcColor = if (dark) 0xFF373C44.toInt() else 0xFFD5DAE3.toInt()
    private val funcPressedColor = if (dark) 0xFF4A5059.toInt() else 0xFFBFC6D2.toInt()
    private val textColor = if (dark) 0xFFE6E8EB.toInt() else 0xFF1A1A1A.toInt()
    private val funcTextColor = if (dark) 0xFFE6E8EB.toInt() else 0xFF33383F.toInt()
    private val languageActiveColor = if (dark) 0xFF68D3A0.toInt() else 0xFF35B982.toInt()
    private val languageInactiveColor = if (dark) 0xFF98A1AD.toInt() else 0xFF8E97A3.toInt()
    private val rows: List<KeyboardLayoutSpec.Row> = when (page) {
        KeyboardLayoutSpec.Page.LETTERS -> KeyboardLayoutSpec.letterRows(
            uppercaseLetters,
            KeyboardLayoutSpec.languageLabel(asciiMode),
        )
        KeyboardLayoutSpec.Page.SYMBOLS -> KeyboardLayoutSpec.symbolRows()
    }
    // 横屏不能继续按短边比例压缩：四行按键至少44dp，才有稳定的触控面积和视觉节奏。
    private val normalKeyboardHeight = KeyboardHeightSpec.normalHeight(resources.displayMetrics.heightPixels)
    private val preferredKeyboardHeight = KeyboardHeightSpec.preferredHeight(
        normalHeightPx = normalKeyboardHeight,
        rowCount = rows.size,
        minimumRowHeightPx = if (resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE) {
            dp(44f)
        } else {
            0
        },
    )
    private var renderedKeyboardHeight = preferredKeyboardHeight

    init {
        orientation = VERTICAL
        renderRows(renderedKeyboardHeight)
    }

    /**
     * 以 IME 输入视图的实际余量限制键区高度。
     *
     * 不能直接依赖完整显示屏高度：分屏、自由窗口和系统受限输入区都会比它小。
     */
    fun setAvailableKeyboardHeight(availableHeightPx: Int) {
        val targetHeight = KeyboardHeightSpec.resolve(preferredKeyboardHeight, availableHeightPx)
        if (targetHeight == renderedKeyboardHeight) return
        renderedKeyboardHeight = targetHeight
        renderRows(targetHeight)
        requestLayout()
    }

    private fun renderRows(keyboardHeight: Int) {
        removeAllViews()
        val rowUnits = rows.sumOf { it.height.toDouble() }.toFloat()
        var allocatedHeight = 0
        rows.forEachIndexed { index, row ->
            val rowHeight = if (index == rows.lastIndex) {
                keyboardHeight - allocatedHeight
            } else {
                (keyboardHeight * row.height / rowUnits).toInt().also { allocatedHeight += it }
            }
            addView(buildRow(row, rowHeight), LayoutParams(LayoutParams.MATCH_PARENT, rowHeight))
        }
    }

    private fun buildRow(row: KeyboardLayoutSpec.Row, rowHeight: Int): LinearLayout =
        LinearLayout(context).apply {
            gravity = Gravity.CENTER_VERTICAL
            orientation = HORIZONTAL
            if (row.inset > 0f) {
                addView(View(context), LayoutParams(0, 1, row.inset))
            }
            val contentWeight = 1f - row.inset * 2f
            val content = LinearLayout(context).apply {
                gravity = Gravity.CENTER_VERTICAL
                orientation = HORIZONTAL
            }
            row.keys.forEach { key ->
                content.addView(
                    buildKey(key),
                    LayoutParams(0, (rowHeight * 0.9f).toInt().coerceAtLeast(1), key.weight).apply {
                        setMargins(dp(2.5f), 0, dp(2.5f), 0)
                    },
                )
            }
            addView(content, LayoutParams(0, LayoutParams.MATCH_PARENT, contentWeight))
            if (row.inset > 0f) {
                addView(View(context), LayoutParams(0, 1, row.inset))
            }
        }

    private fun buildKey(key: KeyboardLayoutSpec.Key): View {
        val isBack = key.kind == KeyboardLayoutSpec.Kind.BACK
        val normal = when {
            isBack -> if (dark) 0xFF45B985.toInt() else 0xFF5BCB99.toInt()
            key.functional -> funcColor
            else -> keyColor
        }
        val pressed = when {
            isBack -> 0xFF46B886.toInt()
            key.functional -> funcPressedColor
            else -> keyPressedColor
        }
        val background = StateListDrawable().apply {
            addState(intArrayOf(android.R.attr.state_pressed), round(pressed))
            addState(intArrayOf(), round(normal))
        }
        val label = displayLabel(key)
        val action = actionFor(key, label)

        return if (key.icon != null) {
            ImageView(context).apply {
                setImageResource(iconResource(key.icon))
                imageTintList = ColorStateList.valueOf(if (isBack) 0xFFFFFFFF.toInt() else funcTextColor)
                this.background = background
                contentDescription = key.description
                val repeatAction = key.longKind?.let { longKind -> actionFor(key.copy(kind = longKind), label) }
                if (repeatAction == null && !key.swipeUpClears) {
                    setOnClickListener { onAction(action) }
                } else {
                    var downY = 0f
                    var cleared = false
                    var repeated = false
                    val repeatDelete = object : Runnable {
                        override fun run() {
                            onAction(repeatAction ?: action)
                            repeated = true
                            postDelayed(this, BackspaceGestureSpec.REPEAT_INTERVAL_MILLIS)
                        }
                    }
                    setOnTouchListener { view, event ->
                        when (event.actionMasked) {
                            MotionEvent.ACTION_DOWN -> {
                                downY = event.y
                                cleared = false
                                repeated = false
                                if (repeatAction != null) {
                                    view.postDelayed(repeatDelete, BackspaceGestureSpec.REPEAT_DELAY_MILLIS)
                                }
                                true
                            }
                            MotionEvent.ACTION_MOVE -> {
                                if (!cleared && key.swipeUpClears && BackspaceGestureSpec.shouldClear(
                                        downY,
                                        event.y,
                                        dp(BackspaceGestureSpec.CLEAR_SWIPE_DP).toFloat(),
                                    )
                                ) {
                                    cleared = true
                                    view.removeCallbacks(repeatDelete)
                                    onAction(WetypeAction.Clear)
                                }
                                true
                            }
                            MotionEvent.ACTION_UP -> {
                                view.removeCallbacks(repeatDelete)
                                view.performClick()
                                if (BackspaceGestureSpec.shouldDeleteOnRelease(cleared, repeated)) {
                                    onAction(action)
                                }
                                true
                            }
                            MotionEvent.ACTION_CANCEL -> {
                                view.removeCallbacks(repeatDelete)
                                true
                            }
                            else -> true
                        }
                    }
                }
            }
        } else {
            FrameLayout(context).apply {
                this.background = background
                contentDescription = key.description
                addView(TextView(context).apply {
                    // 当前语言始终位于首行并用绿色强调，次行只表达切换目标。
                    text = if (key.kind == KeyboardLayoutSpec.Kind.LANG) languageLabel(label) else label
                    gravity = Gravity.CENTER
                    setTextColor(
                        if (isBack) 0xFFFFFFFF.toInt()
                        else if (key.kind == KeyboardLayoutSpec.Kind.LANG) languageInactiveColor
                        else if (key.functional) funcTextColor
                        else textColor,
                    )
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, key.textSize)
                    if (key.bold) typeface = Typeface.DEFAULT_BOLD
                }, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
                key.secondary?.let { secondary ->
                    addView(TextView(context).apply {
                        text = displaySecondary(secondary)
                        gravity = Gravity.CENTER
                        setTextColor(if (dark) 0xFF9BA2AC.toInt() else 0xFF9AA0AA.toInt())
                        setTextSize(TypedValue.COMPLEX_UNIT_SP, 9f)
                    }, FrameLayout.LayoutParams(dp(22f), dp(18f), Gravity.TOP or Gravity.END).apply {
                        topMargin = dp(2f)
                        rightMargin = dp(1f)
                    })
                }
                setOnClickListener { onAction(action) }
                (key.longKind?.let { longKind -> actionFor(key.copy(kind = longKind), label) }
                    ?: key.secondary?.let { secondary -> WetypeAction.Char(displaySecondary(secondary)) })?.let { longAction ->
                    setOnLongClickListener {
                        onAction(longAction)
                        true
                    }
                }
                if (key.swipeUpClears) {
                    var downY = 0f
                    setOnTouchListener { view, event ->
                        when (event.actionMasked) {
                            MotionEvent.ACTION_DOWN -> downY = event.y
                            MotionEvent.ACTION_UP -> {
                                if (event.y < downY - dp(28f)) {
                                    onAction(WetypeAction.Clear)
                                    return@setOnTouchListener true
                                }
                            }
                        }
                        false
                    }
                }
            }
        }
    }

    private fun actionFor(key: KeyboardLayoutSpec.Key, label: String): WetypeAction = when (key.kind) {
        KeyboardLayoutSpec.Kind.CHAR -> WetypeAction.Char(label)
        KeyboardLayoutSpec.Kind.BACKSPACE -> WetypeAction.Backspace
        KeyboardLayoutSpec.Kind.SHIFT -> WetypeAction.Shift
        KeyboardLayoutSpec.Kind.NUMBER -> WetypeAction.NumberPage
        KeyboardLayoutSpec.Kind.BACK -> WetypeAction.BackPage
        KeyboardLayoutSpec.Kind.ENTER -> WetypeAction.Enter
        KeyboardLayoutSpec.Kind.SPACE -> WetypeAction.Space
        KeyboardLayoutSpec.Kind.LANG -> WetypeAction.Lang
    }

    private fun displayLabel(key: KeyboardLayoutSpec.Key): String =
        KeyboardLayoutSpec.displayLabel(if (asciiMode) key.asciiLabel ?: key.label else key.label, uppercaseLetters)

    private fun displaySecondary(value: String): String =
        if (!asciiMode) value else value
            .replace('：', ':')
            .replace('；', ';')
            .replace('（', '(')
            .replace('）', ')')
            .replace('～', '~')
            .replace('“', '"')
            .replace('”', '"')
            .replace('、', '\\')
            .replace('？', '?')
            .replace('！', '!')
            .replace('…', '.')

    private fun languageLabel(label: String): CharSequence = SpannableString(label).apply {
        setSpan(ForegroundColorSpan(languageActiveColor), 0, 1, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
    }

    private fun iconResource(icon: KeyboardLayoutSpec.Icon): Int = when (icon) {
        KeyboardLayoutSpec.Icon.BACKSPACE -> R.drawable.icon_keys_delete
        KeyboardLayoutSpec.Icon.SHIFT -> R.drawable.icon_keys_outlinedcapslock
        KeyboardLayoutSpec.Icon.BACK -> R.drawable.icon_keys_back
    }

    private fun round(color: Int) = GradientDrawable().apply {
        setColor(color)
        cornerRadius = dp(6f).toFloat()
    }

    // 用视图所在 Context 的密度而非 Resources.getSystem()，多窗口/外接屏下更一致
    private fun dp(value: Float): Int = (value * resources.displayMetrics.density).toInt()
}

/** 小窗布局的纯高度规则，便于在 JVM 测试中验证输入视图不会被键区撑破。 */
internal object KeyboardHeightSpec {
    private const val NORMAL_SCREEN_HEIGHT_RATIO = 0.255f

    fun normalHeight(screenHeightPx: Int): Int =
        (screenHeightPx.coerceAtLeast(0) * NORMAL_SCREEN_HEIGHT_RATIO).toInt()

    fun preferredHeight(normalHeightPx: Int, rowCount: Int, minimumRowHeightPx: Int): Int =
        max(normalHeightPx.coerceAtLeast(0), rowCount.coerceAtLeast(0) * minimumRowHeightPx.coerceAtLeast(0))

    fun resolve(normalHeightPx: Int, availableHeightPx: Int): Int =
        min(normalHeightPx.coerceAtLeast(0), availableHeightPx.coerceAtLeast(0))
}

/** 键盘页面与键位语义的纯规格，便于单元测试验证视觉顺序不会影响输入功能。 */
internal object KeyboardLayoutSpec {
    enum class Page { LETTERS, SYMBOLS }
    enum class Icon { BACKSPACE, SHIFT, BACK }
    enum class Kind { CHAR, BACKSPACE, SHIFT, NUMBER, BACK, ENTER, SPACE, LANG }

    data class Key(
        val label: String = "",
        val secondary: String? = null,
        val asciiLabel: String? = null,
        val icon: Icon? = null,
        val kind: Kind = Kind.CHAR,
        val weight: Float = 1f,
        val functional: Boolean = false,
        val textSize: Float = 20f,
        val bold: Boolean = false,
        val description: String = label,
        val longKind: Kind? = null,
        val swipeUpClears: Boolean = false,
    )

    data class Row(val keys: List<Key>, val height: Float = 1f, val inset: Float = 0f)

    fun letterRows(uppercase: Boolean, languageLabel: String = languageLabel(false)): List<Row> = listOf(
        letterRow("QWERTYUIOP", "1234567890"),
        letterRow("ASDFGHJKL", "-/：；（）～“”", inset = 0.055f),
        Row(
            listOf(
                Key(icon = Icon.SHIFT, kind = Kind.SHIFT, weight = 1.35f, functional = true, description = "切换大小写"),
                *letterKeys("ZXCVBNM", "@.#、？！…").toTypedArray(),
                Key(
                    icon = Icon.BACKSPACE,
                    kind = Kind.BACKSPACE,
                    weight = 1.35f,
                    functional = true,
                    description = "删除；长按连续删除，上滑清空拼音",
                    longKind = Kind.BACKSPACE,
                    swipeUpClears = true,
                ),
            ),
        ),
        Row(
            listOf(
                Key("123", kind = Kind.NUMBER, weight = 1.16f, functional = true, textSize = 16f, bold = true, description = "九宫格数字键盘"),
                Key("，", secondary = "。", asciiLabel = ",", kind = Kind.CHAR, weight = 0.9f, description = "逗号"),
                Key("", kind = Kind.SPACE, weight = 4.0f, description = "空格"),
                Key(languageLabel, kind = Kind.LANG, weight = 1.1f, textSize = 14f, description = "切换中英文"),
                Key("换行", kind = Kind.ENTER, weight = 1.55f, functional = true, textSize = 15f, description = "换行"),
            ),
        ),
    )

    fun symbolRows(): List<Row> = listOf(
        numberRow("123", "！@#"),
        numberRow("456", "￥%^"),
        numberRow("789", "&*？"),
        Row(
            listOf(
                Key("ABC", kind = Kind.BACK, weight = 1.12f, functional = true, textSize = 14f, bold = true, description = "返回字母键盘"),
                Key("0", secondary = "。", kind = Kind.CHAR, weight = 0.9f, textSize = 21f, description = "数字 0"),
                Key("", kind = Kind.SPACE, weight = 3.7f, description = "空格"),
                Key(
                    icon = Icon.BACKSPACE,
                    kind = Kind.BACKSPACE,
                    weight = 1.12f,
                    functional = true,
                    description = "删除；长按连续删除，上滑清空拼音",
                    longKind = Kind.BACKSPACE,
                    swipeUpClears = true,
                ),
                Key("换行", kind = Kind.ENTER, weight = 1.4f, functional = true, textSize = 15f, description = "换行"),
            ),
        ),
    )

    fun displayLabel(label: String, uppercaseLetters: Boolean): String =
        if (label.length == 1 && label[0].isLetter()) {
            if (uppercaseLetters) label.uppercase() else label.lowercase()
        } else {
            label
        }

    /** 当前语言置于首行，切换方向置于次行，避免“中英”状态不明。 */
    fun languageLabel(asciiMode: Boolean): String = if (asciiMode) "英\n中" else "中\n英"

    private fun letterRow(letters: String, secondary: String, inset: Float = 0f): Row =
        Row(letterKeys(letters, secondary), inset = inset)

    private fun letterKeys(letters: String, secondary: String): List<Key> =
        letters.mapIndexed { index, letter ->
            Key(letter.toString(), secondary.getOrNull(index)?.toString(), textSize = 21f, description = "字母 $letter")
        }

    private fun numberRow(numbers: String, secondary: String): Row =
        Row(numbers.mapIndexed { index, number ->
            Key(
                label = number.toString(),
                secondary = secondary.getOrNull(index)?.toString(),
                textSize = 24f,
                description = "数字 $number",
            )
        })
}

/** 删除键触摸规则独立于视图，确保长按删除和上滑清空始终作用于退格键。 */
internal object BackspaceGestureSpec {
    const val REPEAT_DELAY_MILLIS = 400L
    // 每次触发都同步调用引擎（退格→processKey+commit+context），
    // 收紧到 80ms 兼顾删除手感与主线程 JNI 负载。
    const val REPEAT_INTERVAL_MILLIS = 80L
    const val CLEAR_SWIPE_DP = 36f

    fun shouldClear(downY: Float, currentY: Float, thresholdPx: Float): Boolean =
        downY - currentY > thresholdPx

    fun shouldDeleteOnRelease(cleared: Boolean, repeated: Boolean): Boolean = !cleared && !repeated
}
