package com.shurufa.ime

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.widget.Toast

/**
 * 空格键长按触发的语音转文字。
 *
 * 交互模型保持最简单：
 *  - 长按空格：开始聆听
 *  - 松开：结束聆听，最终识别结果上屏
 *  - 上滑：取消，丢弃本段结果
 *
 * 反编译微信输入法（WeType）得到的核心结论：微信用自研引擎（lvcsr + NNVad）
 * 自己控制录音流，没有系统 SpeechRecognizer 的"会话超时/静音结束"问题，所以
 * 语音永远不会"断"。我们只能用系统 SpeechRecognizer，因此用"无缝轮转"模拟：
 *
 *  - 识别过程中 partial 结果实时上屏（composition，可被后续替换）
 *  - 引擎一旦结束（onResults/onError）且用户仍按住：先固化已上屏文字，
 *    再立即重启识别器续听 —— 已出的字永远不丢、不被打断
 *  - 会话看门狗：每段会话最多 6 秒，到点主动 stopListening 轮转，
 *    即使引擎一直不结束也能持续出字
 *
 * RECORD_AUDIO 是运行时权限：IME 无 Activity 栈，需经 [VoicePermissionActivity] 透明
 * 代理请求。授权后可经系统 SpeechRecognizer 识别。
 */
class VoiceInputController(private val context: Context) {
    private val appContext = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private var recognizer: SpeechRecognizer? = null
    /** 用户仍在等待或执行一次语音输入；松手/取消后，迟到的权限回调不得再启动识别。 */
    private var requested = false
    /** 用户手指是否仍按住空格（按住=聆听中；end/cancel 后为 false）。 */
    private var active = false
    /** 本段会话累积的最新识别文本；固化（onCommit）后清空。 */
    private var latest: String = ""
    /** 本段语音内识别器轮转次数（引擎中途结束会自动续听），防死循环。 */
    private var restartCount = 0
    /** 松手后的兜底上屏任务（保存引用以便清理，防止多次语音堆积）。 */
    private val flushTask = Runnable { flushIfIdle() }
    /** 会话看门狗：每段识别会话最长 6 秒，到点主动轮转，保证持续出字不断。 */
    private val sessionTask = Runnable {
        if (active) {
            android.util.Log.w("shurufa", "Voice watchdog: rotating session after 6s")
            try {
                recognizer?.stopListening()
            } catch (_: Throwable) {
            }
        }
    }

    /** 语音结束（松手）时上屏最终文本（主线程）。 */
    var onResult: (String) -> Unit = {}
    /** 按住期间轮转时固化已上屏文字（静默，主线程）。 */
    var onCommit: (String) -> Unit = {}
    /** 实时识别结果（partial，边说边上屏，主线程）。 */
    var onPartial: (String) -> Unit = {}
    /** 状态回调：0 结束/空闲，1 聆听中。 */
    var onStatus: (Int) -> Unit = {}

    fun isRecording(): Boolean = active

    fun hasPermission(): Boolean =
        context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED

    /** 若已授权则开始识别；否则交给透明 VoicePermissionActivity 请求权限。 */
    fun startOrRequestPermission() {
        requested = true
        if (hasPermission()) {
            beginListening()
            return
        }
        VoicePermissionActivity.pending = { granted ->
            val shouldStart = requested && granted
            requested = false
            if (shouldStart) beginListening()
        }
        val startIntent = Intent(appContext, VoicePermissionActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        appContext.startActivity(startIntent)
    }

    /** 一段新语音：清空累积结果并启动识别。 */
    private fun beginListening() {
        if (!requested) return
        requested = false
        stopQuietly()
        latest = ""
        restartCount = 0
        main.post { onPartial("") }  // 清掉上一段的残留预览
        startRecognizer()
    }

    /** 启动/重启识别器。注意：不清空 latest，由调用方决定（轮转时必须保留已固化文本）。 */
    private fun startRecognizer() {
        stopQuietly()
        val sr = SpeechRecognizer.createSpeechRecognizer(appContext)
        recognizer = sr
        active = true
        main.removeCallbacks(sessionTask)
        main.postDelayed(sessionTask, 6000L)
        main.post { onStatus(1) }
        sr.setRecognitionListener(object : RecognitionListener {
            override fun onReadyForSpeech(params: Bundle?) {}
            override fun onBeginningOfSpeech() {}
            override fun onRmsChanged(rmsdB: Float) {}
            override fun onBufferReceived(buffer: ByteArray?) {}
            override fun onEndOfSpeech() {}
            override fun onError(error: Int) {
                android.util.Log.w("shurufa", "Voice recognizer error code=$error active=$active restarts=$restartCount")
                if (!active) {
                    // 用户已松手/取消：正常收尾。
                    active = false
                    main.removeCallbacks(sessionTask)
                    main.post { onStatus(0) }
                    return
                }
                // 手指仍按住但引擎报错（静音检测/网络/引擎忙等）：
                // 先固化已识别的字（如有），再重启续听 —— 已出的字不丢、不断。
                if (restartCount >= 60) {
                    active = false
                    main.removeCallbacks(sessionTask)
                    main.post { onStatus(0) }
                    return
                }
                restartCount++
                val t = latest
                latest = ""
                if (t.isNotEmpty()) main.post { onCommit(t) }
                main.postDelayed({ if (active) startRecognizer() }, 120L)
            }
            override fun onResults(results: Bundle?) {
                val text =
                    results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()
                if (!text.isNullOrEmpty()) latest = text
                if (!active) {
                    // 用户已松手（end 已调用 stopListening 触发此回调）：立即上屏。
                    val finalText = latest
                    latest = ""
                    main.removeCallbacks(sessionTask)
                    main.post {
                        onStatus(0)
                        if (finalText.isNotEmpty()) onResult(finalText)
                    }
                    return
                }
                // 用户仍按住：本段结束。先固化本段文字（防止下段 partial 覆盖），
                // 再重启识别器继续听下一段 —— 微信式"边说边上屏、永不断"。
                restartCount++
                val t = latest
                latest = ""
                if (t.isNotEmpty()) main.post { onCommit(t) }
                main.postDelayed({ if (active) startRecognizer() }, 120L)
            }
            override fun onPartialResults(results: Bundle?) {
                val text =
                    results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()
                if (!text.isNullOrEmpty()) {
                    latest = text
                    // 微信式：边说边上屏，输入框实时显示识别文本（可被后续 partial 替换）。
                    main.post { onPartial(text) }
                }
            }
            override fun onEvent(eventType: Int, params: Bundle?) {}
        })
        try {
            sr.startListening(
                Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
                    putExtra(
                        RecognizerIntent.EXTRA_LANGUAGE_MODEL,
                        RecognizerIntent.LANGUAGE_MODEL_FREE_FORM,
                    )
                    putExtra(RecognizerIntent.EXTRA_LANGUAGE, "zh-CN")
                    putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
                    // 加长静音容忍：默认 2s 停顿就结束识别（用户会觉得"莫名其妙断了"）。
                    // 这里允许最长 4s 停顿仍继续聆听。
                    putExtra(
                        RecognizerIntent.EXTRA_SPEECH_INPUT_COMPLETE_SILENCE_LENGTH_MILLIS,
                        4000L,
                    )
                    putExtra(
                        RecognizerIntent.EXTRA_SPEECH_INPUT_POSSIBLY_COMPLETE_SILENCE_LENGTH_MILLIS,
                        2000L,
                    )
                },
            )
        } catch (e: Exception) {
            active = false
            main.removeCallbacks(sessionTask)
            main.post { onStatus(0) }
            Toast.makeText(appContext, "语音识别不可用", Toast.LENGTH_SHORT).show()
        }
    }

    /** 松开手指：结束聆听，最终结果上屏（等 onResults 或 3.5s 兜底）。 */
    fun end() {
        requested = false
        active = false
        main.removeCallbacks(sessionTask)
        val sr = recognizer
        try {
            sr?.stopListening()
        } catch (_: Throwable) {
        }
        // 若松开前已实时上屏过 partial（latest 有值），直接固化，不用等。
        val t = latest
        if (t.isNotEmpty()) {
            latest = ""
            main.removeCallbacks(flushTask)
            stopQuietly()
            main.post {
                onStatus(0)
                onResult(t)
            }
        } else {
            // 还没识别到任何字：清掉残留 preview 并等 onResults / 3.5s 兜底。
            main.post { onPartial("") }
            main.removeCallbacks(flushTask)
            main.postDelayed(flushTask, 3500L)
        }
    }

    /** 上滑取消：丢弃识别结果并立即释放。 */
    fun cancel() {
        requested = false
        active = false
        main.removeCallbacks(flushTask)
        main.removeCallbacks(sessionTask)
        stopQuietly()
        latest = ""
        main.post { onStatus(0) }
    }

    private fun flushIfIdle() {
        if (active) return  // 用户还按着，不应该兜底上屏
        val t = latest
        latest = ""
        stopQuietly()
        main.post {
            onStatus(0)
            if (t.isNotEmpty()) onResult(t)
        }
    }

    private fun stopQuietly() {
        try {
            recognizer?.cancel()
        } catch (_: Throwable) {
        }
        try {
            recognizer?.destroy()
        } catch (_: Throwable) {
        }
        recognizer = null
    }
}

/** 透明权限代理 Activity：仅用于请求 RECORD_AUDIO，结果回填给语音控制器。 */
class VoicePermissionActivity : Activity() {
    companion object {
        const val REQ = 4001
        var pending: ((Boolean) -> Unit)? = null
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            finishWith(true)
            return
        }
        requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), REQ)
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQ) {
            finishWith(grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED)
        }
    }

    private fun finishWith(granted: Boolean) {
        pending?.invoke(granted)
        pending = null
        finish()
    }
}
