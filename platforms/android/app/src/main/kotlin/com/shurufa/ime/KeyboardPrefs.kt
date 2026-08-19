package com.shurufa.ime

import android.content.Context

/** 单手模式三态：关闭 / 键盘靠左 / 键盘靠右。 */
enum class SingleHandMode { OFF, LEFT, RIGHT }

/**
 * 键盘偏好（M-A1-1，搜狗 3.7 快捷设置 / 5.1 键盘调节 / 5.4 按键反馈）。
 * 纯函数（clamp/parse）与持久化分离，便于 JVM 单测。
 */
data class KeyboardPrefs(
    val heightPercent: Int = DEFAULT_HEIGHT_PERCENT,
    val keySound: Boolean = true,
    val haptic: Boolean = true,
    val singleHand: SingleHandMode = SingleHandMode.OFF,
) {
    companion object {
        const val MIN_HEIGHT_PERCENT = 40
        const val MAX_HEIGHT_PERCENT = 120
        const val DEFAULT_HEIGHT_PERCENT = 100

        /** 高度百分比夹取到 [MIN_HEIGHT_PERCENT, MAX_HEIGHT_PERCENT]。 */
        fun clampHeight(percent: Int): Int = percent.coerceIn(MIN_HEIGHT_PERCENT, MAX_HEIGHT_PERCENT)

        /** 单手模式字符串解析，未知/空值回退到关闭。 */
        fun parseSingleHand(value: String?): SingleHandMode = when (value) {
            SingleHandMode.LEFT.name -> SingleHandMode.LEFT
            SingleHandMode.RIGHT.name -> SingleHandMode.RIGHT
            else -> SingleHandMode.OFF
        }

        fun load(context: Context): KeyboardPrefs {
            val sp = context.getSharedPreferences("shurufa", Context.MODE_PRIVATE)
            return KeyboardPrefs(
                heightPercent = clampHeight(sp.getInt("kb_height_percent", DEFAULT_HEIGHT_PERCENT)),
                keySound = sp.getBoolean("kb_key_sound", true),
                haptic = sp.getBoolean("kb_haptic", true),
                singleHand = parseSingleHand(sp.getString("kb_single_hand", null)),
            )
        }

        fun save(context: Context, prefs: KeyboardPrefs) {
            context.getSharedPreferences("shurufa", Context.MODE_PRIVATE).edit()
                .putInt("kb_height_percent", clampHeight(prefs.heightPercent))
                .putBoolean("kb_key_sound", prefs.keySound)
                .putBoolean("kb_haptic", prefs.haptic)
                .putString("kb_single_hand", prefs.singleHand.name)
                .apply()
        }
    }
}
