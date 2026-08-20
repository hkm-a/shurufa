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
    /** 候选字大小百分比（UI-2 借鉴搜狗候选字大小设置）。 */
    val candidateSizePercent: Int = DEFAULT_CANDIDATE_SIZE_PERCENT,
    /** P4-5 输入风格预设：主候选行显示条数（5=经典 / 9=高效，搜狗风格差异核心）。 */
    val candidateCount: Int = DEFAULT_CANDIDATE_COUNT,
    /** P4-6 自定义标点：符号页中文标点行（，。、；：？！……——《》）显隐。 */
    val showPunctRow: Boolean = true,
    /** AI 候选预测开关（默认关：云端消耗需用户主动开启）。 */
    val aiCandidates: Boolean = false,
) {
    companion object {
        const val MIN_HEIGHT_PERCENT = 40
        const val MAX_HEIGHT_PERCENT = 120
        const val DEFAULT_HEIGHT_PERCENT = 100
        const val MIN_CANDIDATE_SIZE_PERCENT = 80
        const val MAX_CANDIDATE_SIZE_PERCENT = 140
        const val DEFAULT_CANDIDATE_SIZE_PERCENT = 100
        const val CANDIDATE_COUNT_CLASSIC = 5
        const val CANDIDATE_COUNT_EFFICIENT = 9
        const val DEFAULT_CANDIDATE_COUNT = CANDIDATE_COUNT_CLASSIC

        /** 候选条数夹取到 {5, 9}。 */
        fun clampCandidateCount(count: Int): Int = when (count) {
            CANDIDATE_COUNT_EFFICIENT -> CANDIDATE_COUNT_EFFICIENT
            else -> CANDIDATE_COUNT_CLASSIC
        }

        /** 候选字大小百分比夹取。 */
        fun clampCandidateSize(percent: Int): Int =
            percent.coerceIn(MIN_CANDIDATE_SIZE_PERCENT, MAX_CANDIDATE_SIZE_PERCENT)

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
                candidateSizePercent = clampCandidateSize(sp.getInt("kb_candidate_size", DEFAULT_CANDIDATE_SIZE_PERCENT)),
                candidateCount = clampCandidateCount(sp.getInt("kb_candidate_count", DEFAULT_CANDIDATE_COUNT)),
                showPunctRow = sp.getBoolean("kb_show_punct_row", true),
                aiCandidates = sp.getBoolean("kb_ai_candidates", false),
            )
        }

        fun save(context: Context, prefs: KeyboardPrefs) {
            context.getSharedPreferences("shurufa", Context.MODE_PRIVATE).edit()
                .putInt("kb_height_percent", clampHeight(prefs.heightPercent))
                .putBoolean("kb_key_sound", prefs.keySound)
                .putBoolean("kb_haptic", prefs.haptic)
                .putString("kb_single_hand", prefs.singleHand.name)
                .putInt("kb_candidate_size", clampCandidateSize(prefs.candidateSizePercent))
                .putInt("kb_candidate_count", clampCandidateCount(prefs.candidateCount))
                .putBoolean("kb_show_punct_row", prefs.showPunctRow)
                .putBoolean("kb_ai_candidates", prefs.aiCandidates)
                .apply()
        }
    }
}
