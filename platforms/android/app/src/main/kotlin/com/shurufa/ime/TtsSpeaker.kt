package com.shurufa.ime

import android.content.Context
import android.speech.tts.TextToSpeech

/**
 * M-A3-4 文字转语音（搜狗安卓 11.4 无障碍「输入文字转语音」半边）。
 * 系统 TTS 懒初始化，失败静默；speak 用 QUEUE_FLUSH 打断上一句。
 */
class TtsSpeaker(context: Context) {
    private val tts: TextToSpeech = TextToSpeech(context.applicationContext) { status ->
        ready = status == TextToSpeech.SUCCESS
    }

    @Volatile
    private var ready = false

    fun speak(text: String) {
        if (!ready || text.isBlank()) return
        try {
            tts.speak(text, TextToSpeech.QUEUE_FLUSH, null, "shurufa-tts")
        } catch (_: Throwable) {
        }
    }

    fun shutdown() {
        try {
            tts.stop()
            tts.shutdown()
        } catch (_: Throwable) {
        }
    }
}
