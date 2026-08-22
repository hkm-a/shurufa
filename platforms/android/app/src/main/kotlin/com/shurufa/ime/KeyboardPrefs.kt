package com.shurufa.ime

import android.content.Context
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

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

        // 阶段4第6项：持久化迁到 DataStore（取代手写 SharedPreferences 读写）。
        // 调用方仍是同步 load/save：load 用 runBlocking 读小文件（微秒级），
        // save 沿用旧 apply() 的 fire-and-forget 语义，投递到自有 IO scope。
        private val Context.keyboardDataStore by preferencesDataStore(name = "keyboard_prefs")
        private val storeScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

        private val KEY_HEIGHT = intPreferencesKey("kb_height_percent")
        private val KEY_KEY_SOUND = booleanPreferencesKey("kb_key_sound")
        private val KEY_HAPTIC = booleanPreferencesKey("kb_haptic")
        private val KEY_SINGLE_HAND = stringPreferencesKey("kb_single_hand")
        private val KEY_CANDIDATE_SIZE = intPreferencesKey("kb_candidate_size")
        private val KEY_CANDIDATE_COUNT = intPreferencesKey("kb_candidate_count")
        private val KEY_SHOW_PUNCT_ROW = booleanPreferencesKey("kb_show_punct_row")
        private val KEY_AI_CANDIDATES = booleanPreferencesKey("kb_ai_candidates")
        private val KEY_MIGRATED = booleanPreferencesKey("migrated_from_sp")

        fun load(context: Context): KeyboardPrefs {
            val values = runBlocking { context.keyboardDataStore.data.first() }
            if (values[KEY_MIGRATED] != true) {
                // 首次读取：把旧 SharedPreferences 里的 kb_* 迁移过来（无值用默认）。
                val sp = context.getSharedPreferences("shurufa", Context.MODE_PRIVATE)
                val migrated = KeyboardPrefs(
                    heightPercent = clampHeight(sp.getInt("kb_height_percent", DEFAULT_HEIGHT_PERCENT)),
                    keySound = sp.getBoolean("kb_key_sound", true),
                    haptic = sp.getBoolean("kb_haptic", true),
                    singleHand = parseSingleHand(sp.getString("kb_single_hand", null)),
                    candidateSizePercent = clampCandidateSize(sp.getInt("kb_candidate_size", DEFAULT_CANDIDATE_SIZE_PERCENT)),
                    candidateCount = clampCandidateCount(sp.getInt("kb_candidate_count", DEFAULT_CANDIDATE_COUNT)),
                    showPunctRow = sp.getBoolean("kb_show_punct_row", true),
                    aiCandidates = sp.getBoolean("kb_ai_candidates", false),
                )
                save(context, migrated)
                storeScope.launch {
                    context.keyboardDataStore.edit { it[KEY_MIGRATED] = true }
                }
                return migrated
            }
            return KeyboardPrefs(
                heightPercent = clampHeight(values[KEY_HEIGHT] ?: DEFAULT_HEIGHT_PERCENT),
                keySound = values[KEY_KEY_SOUND] ?: true,
                haptic = values[KEY_HAPTIC] ?: true,
                singleHand = parseSingleHand(values[KEY_SINGLE_HAND]),
                candidateSizePercent = clampCandidateSize(values[KEY_CANDIDATE_SIZE] ?: DEFAULT_CANDIDATE_SIZE_PERCENT),
                candidateCount = clampCandidateCount(values[KEY_CANDIDATE_COUNT] ?: DEFAULT_CANDIDATE_COUNT),
                showPunctRow = values[KEY_SHOW_PUNCT_ROW] ?: true,
                aiCandidates = values[KEY_AI_CANDIDATES] ?: false,
            )
        }

        fun save(context: Context, prefs: KeyboardPrefs) {
            storeScope.launch {
                context.keyboardDataStore.edit {
                    it[KEY_HEIGHT] = clampHeight(prefs.heightPercent)
                    it[KEY_KEY_SOUND] = prefs.keySound
                    it[KEY_HAPTIC] = prefs.haptic
                    it[KEY_SINGLE_HAND] = prefs.singleHand.name
                    it[KEY_CANDIDATE_SIZE] = clampCandidateSize(prefs.candidateSizePercent)
                    it[KEY_CANDIDATE_COUNT] = clampCandidateCount(prefs.candidateCount)
                    it[KEY_SHOW_PUNCT_ROW] = prefs.showPunctRow
                    it[KEY_AI_CANDIDATES] = prefs.aiCandidates
                }
            }
        }
    }
}
