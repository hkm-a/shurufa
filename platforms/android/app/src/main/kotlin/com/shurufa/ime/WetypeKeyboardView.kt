package com.shurufa.ime

import android.content.Context
import android.content.res.ColorStateList
import android.content.res.Configuration
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.StateListDrawable
import android.media.AudioManager
import android.text.SpannableString
import android.text.Spanned
import android.text.style.ForegroundColorSpan
import android.util.TypedValue
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.MotionEvent
import android.view.SoundEffectConstants
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
    private val palette: ShurufaImeService.Palette,
    private val asciiMode: Boolean,
    private val uppercaseLetters: Boolean,
    private val onAction: (WetypeAction) -> Unit,
    private val heightPercent: Int = 100,
    private val keySoundEnabled: Boolean = true,
    private val hapticEnabled: Boolean = true,
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
        /** 空格键长按语音：发出 / 取消 / 结束（松手上屏）。 */
        object VoiceStart : WetypeAction()
        object VoiceCancel : WetypeAction()
        object VoiceEnd : WetypeAction()
        /** 删除键长按开始连删；上滑清空。二者提示由 Service 状态条统一显示。 */
        object BackspaceStart : WetypeAction()
        object BackspaceClear : WetypeAction()
        object BackspaceEnd : WetypeAction()
    }

    private val keyColor = palette.key
    private val keyPressedColor = palette.keyPressed
    private val funcColor = palette.keyFunc
    private val funcPressedColor = palette.funcPressed
    private val textColor = palette.keyText
    private val funcTextColor = palette.funcText
    private val languageActiveColor = palette.accent
    private val languageInactiveColor = palette.preedit
    private val rows: List<KeyboardLayoutSpec.Row> = when (page) {
        KeyboardLayoutSpec.Page.LETTERS -> KeyboardLayoutSpec.letterRows(
            uppercaseLetters,
            KeyboardLayoutSpec.languageLabel(asciiMode),
        )
        KeyboardLayoutSpec.Page.SYMBOLS -> KeyboardLayoutSpec.symbolRows()
    }
    // 横屏不能继续按短边比例压缩：四行按键至少44dp，才有稳定的触控面积和视觉节奏。
    // 高度百分比（M-A1-1，搜狗 5.1 键盘调节）缩放自然高度与可用余量两个输入。
    private val heightScale = KeyboardPrefs.clampHeight(heightPercent) / 100f
    private val normalKeyboardHeight = (KeyboardHeightSpec.normalHeight(resources.displayMetrics.heightPixels) * heightScale).toInt()
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
     * 以 IME 输入实际的实际余量限制键区高度。
     *
     * 不能直接依赖完整显示屏高度：分屏、自由窗口和系统受限输入区都会比它小。
     */
    fun setAvailableKeyboardHeight(availableHeightPx: Int) {
        val scaledAvailable = (availableHeightPx * heightScale).toInt()
        val targetHeight = KeyboardHeightSpec.resolve(preferredKeyboardHeight, scaledAvailable)
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

    /** M-A1-1 按键反馈：振动（系统触觉设置内）＋ 按键音（系统音效音量内），均受偏好门控。 */
    private fun playKeyFeedback() {
        if (hapticEnabled) {
            performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP)
        }
        if (keySoundEnabled) {
            (context.getSystemService(Context.AUDIO_SERVICE) as? AudioManager)
                ?.playSoundEffect(SoundEffectConstants.CLICK)
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
                    setOnClickListener {
                        playKeyFeedback()
                        onAction(action)
                    }
                } else {
                    var downY = 0f
                    var cleared = false
                    var repeated = false
                    // 长按重复触发后：连续删除；提示由 Service 顶部状态条统一显示。
                    val startRepeat = object : Runnable {
                        override fun run() {
                            if (!repeated) onAction(WetypeAction.BackspaceStart)
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
                                playKeyFeedback()
                                if (repeatAction != null) {
                                    view.postDelayed(startRepeat, BackspaceGestureSpec.REPEAT_DELAY_MILLIS)
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
                                    view.removeCallbacks(startRepeat)
                                    onAction(WetypeAction.Clear)
                                    onAction(WetypeAction.BackspaceClear)
                                }
                                true
                            }
                            MotionEvent.ACTION_UP -> {
                                view.removeCallbacks(startRepeat)
                                onAction(WetypeAction.BackspaceEnd)
                                view.performClick()
                                if (BackspaceGestureSpec.shouldDeleteOnRelease(cleared, repeated)) {
                                    onAction(action)
                                }
                                true
                            }
                            MotionEvent.ACTION_CANCEL -> {
                                view.removeCallbacks(startRepeat)
                                onAction(WetypeAction.BackspaceEnd)
                                true
                            }
                            else -> true
                        }
                    }
                }
            }
        } else if (key.kind == KeyboardLayoutSpec.Kind.SPACE) {
            // 空格键：单击输出空格；长按启动语音输入（提示走 Service 顶部状态条）。
            // 上滑取消；松手上屏识别结果。键面常驻一个小麦克风图标。
            FrameLayout(context).apply {
                this.background = background
                contentDescription = key.description
                // 键面中央：常驻麦克风图标；语音聆听中切换为状态文（最可靠的视觉反馈）。
                val micIcon = TextView(context).apply {
                    text = "🎤"
                    textSize = 15f
                    gravity = Gravity.CENTER
                    setTextColor(if (dark) 0xFF8E949D.toInt() else 0xFF6B7280.toInt())
                }
                addView(micIcon, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
                val subHint = TextView(context).apply {
                    text = "按住说话"
                    textSize = 9f
                    gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
                    setTextColor(if (dark) 0x669BA2AC.toInt() else 0x669AA0AA.toInt())
                    setPadding(0, 0, 0, dp(4f))
                }
                addView(subHint, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
                var downY = 0f
                var voiceActive = false
                var cancelled = false
                val startVoice = object : Runnable {
                    override fun run() {
                        if (voiceActive) return
                        voiceActive = true
                        cancelled = false
                        micIcon.text = "🎤 聆听中…"
                        micIcon.setTextColor(if (dark) 0xFF78D5A6.toInt() else 0xFF2FA360.toInt())
                        subHint.visibility = View.GONE
                        onAction(WetypeAction.VoiceStart)
                    }
                }
                fun restoreMicFace() {
                    micIcon.text = "🎤"
                    micIcon.setTextColor(if (dark) 0xFF8E949D.toInt() else 0xFF6B7280.toInt())
                    subHint.visibility = View.VISIBLE
                }
                setOnTouchListener { view, event ->
                    when (event.actionMasked) {
                        MotionEvent.ACTION_DOWN -> {
                            downY = event.y
                            voiceActive = false
                            cancelled = false
                            view.postDelayed(startVoice, VoiceGestureSpec.LONG_PRESS_MILLIS)
                            true
                        }
                        MotionEvent.ACTION_MOVE -> {
                            if (voiceActive && !cancelled && VoiceGestureSpec.shouldCancel(
                                    downY,
                                    event.y,
                                    dp(VoiceGestureSpec.CANCEL_SWIPE_DP).toFloat(),
                                )
                            ) {
                                cancelled = true
                                restoreMicFace()
                                onAction(WetypeAction.VoiceCancel)
                            }
                            true
                        }
                        MotionEvent.ACTION_UP -> {
                            view.removeCallbacks(startVoice)
                            restoreMicFace()
                            if (voiceActive && !cancelled) {
                                onAction(WetypeAction.VoiceEnd)
                            } else if (!voiceActive) {
                                // 短按：普通空格。
                                playKeyFeedback()
                                onAction(WetypeAction.Space)
                            }
                            true
                        }
                        MotionEvent.ACTION_CANCEL -> {
                            view.removeCallbacks(startVoice)
                            restoreMicFace()
                            if (voiceActive && !cancelled) {
                                onAction(WetypeAction.VoiceCancel)
                            }
                            true
                        }
                        else -> true
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
                setOnClickListener {
                    playKeyFeedback()
                    onAction(action)
                }
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
        // 可用余量不足时回退到自然高度，避免被一次性压成 0：键盘需要刷新到满尺寸。
        if (availableHeightPx <= 0) normalHeightPx.coerceAtLeast(0)
        else min(normalHeightPx.coerceAtLeast(0), availableHeightPx.coerceAtLeast(0))
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

/** 空格键语音手势；长按阈值与上滑取消距离。 */
internal object VoiceGestureSpec {
    const val LONG_PRESS_MILLIS = 380L
    const val CANCEL_SWIPE_DP = 40f

    fun shouldCancel(downY: Float, currentY: Float, thresholdPx: Float): Boolean =
        downY - currentY > thresholdPx
}
