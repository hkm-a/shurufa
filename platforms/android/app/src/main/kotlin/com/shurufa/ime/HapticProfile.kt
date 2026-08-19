package com.shurufa.ime

/**
 * M-A3-1 触觉输入（搜狗安卓 11.13.1「开启 talkback 振动反馈层次分明；
 * 长按连续删除，末位强振提示」）。
 *
 * 层次映射（对应 android.view.HapticFeedbackConstants）：
 * - TAP        → KEYBOARD_TAP（常规按键）
 * - LONG_PRESS → LONG_PRESS（长按删除启动 / 末位强振）
 * - REPEAT     → CONTEXT_CLICK（连续删除每 tick 的中等反馈）
 *
 * 纯函数便于 JVM 单测；talkback 开启时层次更分明（REPEAT 也保持可感知）。
 */
object HapticProfile {
    const val TAP = 0
    const val LONG_PRESS = 1
    const val REPEAT = 2

    /** 长按删除反馈：tick=0 启动用 LONG_PRESS，之后每 tick 用 REPEAT。 */
    fun deleteFeedback(tick: Int): Int = if (tick <= 0) LONG_PRESS else REPEAT

    /** 末位强振：组合已被删空时的提示（与启动同为 LONG_PRESS 级别）。 */
    const val FINAL_FEEDBACK: Int = LONG_PRESS

    /** TalkBack 手势模式下常规按键反馈是否升级（KEYBOARD_TAP 即可感知，不升级）。 */
    fun tapFeedback(_talkback: Boolean): Int = TAP
}
