package com.shurufa.ime

import android.graphics.Color
import android.graphics.Typeface
import android.inputmethodservice.InputMethodService
import android.util.TypedValue
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import android.widget.TextView
import java.io.File
import kotlin.concurrent.thread

/**
 * 最小可用的拼音键盘：QWERTY 三排字母 + 功能行 + 候选栏。
 *
 * 引擎在后台线程初始化（首次含词典部署，数秒到数十秒），
 * 就绪前按键直接以字母上屏（英文直通），与桌面端策略一致。
 */
class ShurufaImeService : InputMethodService() {

    companion object {
        private const val XK_BACKSPACE = 0xff08
        private const val XK_RETURN = 0xff0d
        private const val COLOR_BG = 0xFFF2F3F5.toInt()
        private const val COLOR_KEY = 0xFFFFFFFF.toInt()
        private const val COLOR_KEY_FUNC = 0xFFDCE0E6.toInt()
        private const val COLOR_TEXT = 0xFF202020.toInt()
        private const val COLOR_CAND = 0xFF202020.toInt()
        private const val COLOR_CAND_HL = 0xFFB06030.toInt()
        private const val COLOR_PREEDIT = 0xFF888888.toInt()

        @Volatile
        private var engineReady = false

        @Volatile
        private var engineStarting = false
    }

    private lateinit var candidateBar: LinearLayout
    private lateinit var preeditView: TextView

    override fun onCreate() {
        super.onCreate()
        ensureEngine()
    }

    /** 后台准备引擎：解包方案资产 → 初始化 librime（幂等）。 */
    private fun ensureEngine() {
        if (engineReady || engineStarting) return
        engineStarting = true
        thread(name = "rime-init") {
            try {
                val schemas = unpackSchemas()
                val userDir = File(filesDir, "rime").apply { mkdirs() }
                engineReady = RimeBridge.nativeInit(schemas.absolutePath, userDir.absolutePath)
            } catch (e: Exception) {
                android.util.Log.e("shurufa", "引擎初始化失败", e)
            } finally {
                engineStarting = false
            }
        }
    }

    /** 把 assets/schemas 解包到私有目录；按 versionCode 判断是否需要刷新。 */
    private fun unpackSchemas(): File {
        val dest = File(filesDir, "schemas")
        val marker = File(dest, ".version")
        val version = packageManager.getPackageInfo(packageName, 0).longVersionCode.toString()
        if (marker.takeIf { it.exists() }?.readText() == version) {
            return dest
        }
        dest.deleteRecursively()
        dest.mkdirs()
        assets.list("schemas")?.forEach { name ->
            assets.open("schemas/$name").use { input ->
                File(dest, name).outputStream().use { input.copyTo(it) }
            }
        }
        marker.writeText(version)
        return dest
    }

    // ---------- 键盘视图 ----------

    private fun dp(v: Float): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, resources.displayMetrics).toInt()

    override fun onCreateInputView(): View {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(COLOR_BG)
            setPadding(dp(2f), dp(2f), dp(2f), dp(4f))
        }

        preeditView = TextView(this).apply {
            setTextColor(COLOR_PREEDIT)
            textSize = 13f
            setPadding(dp(10f), dp(2f), dp(10f), 0)
            visibility = View.GONE
        }
        root.addView(preeditView)

        candidateBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
        }
        val scroll = HorizontalScrollView(this).apply {
            addView(candidateBar)
            isHorizontalScrollBarEnabled = false
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, dp(44f)
            )
        }
        root.addView(scroll)

        listOf("qwertyuiop", "asdfghjkl", "zxcvbnm").forEachIndexed { index, row ->
            root.addView(buildLetterRow(row, withBackspace = index == 2))
        }
        root.addView(buildBottomRow())
        return root
    }

    private fun rowLayout(): LinearLayout = LinearLayout(this).apply {
        orientation = LinearLayout.HORIZONTAL
        layoutParams = LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT, dp(52f)
        )
    }

    private fun key(
        label: String,
        weight: Float,
        functional: Boolean = false,
        onClick: () -> Unit,
    ): TextView = TextView(this).apply {
        text = label
        gravity = Gravity.CENTER
        textSize = 19f
        setTextColor(COLOR_TEXT)
        typeface = Typeface.DEFAULT
        setBackgroundColor(if (functional) COLOR_KEY_FUNC else COLOR_KEY)
        layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, weight)
            .apply { setMargins(dp(2f), dp(3f), dp(2f), dp(3f)) }
        setOnClickListener { onClick() }
    }

    private fun buildLetterRow(letters: String, withBackspace: Boolean): LinearLayout {
        val row = rowLayout()
        if (letters.length == 9) {
            row.addView(spacer(0.5f))
        }
        letters.forEach { c ->
            row.addView(key(c.toString(), 1f) { onLetter(c) })
        }
        if (withBackspace) {
            row.addView(key("⌫", 1.5f, functional = true) { onBackspace() })
        } else if (letters.length == 9) {
            row.addView(spacer(0.5f))
        }
        return row
    }

    private fun spacer(weight: Float): View = View(this).apply {
        layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, weight)
    }

    private fun buildBottomRow(): LinearLayout {
        val row = rowLayout()
        row.addView(key("，", 1.2f, functional = true) { onPunct("，") })
        row.addView(key("空格", 4f) { onSpace() })
        row.addView(key("。", 1.2f, functional = true) { onPunct("。") })
        row.addView(key("回车", 1.6f, functional = true) { onEnter() })
        return row
    }

    // ---------- 输入处理 ----------

    private fun onLetter(c: Char) {
        if (!engineReady) {
            ensureEngine()
            currentInputConnection?.commitText(c.toString(), 1)
            return
        }
        val eaten = RimeBridge.nativeProcessKey(c.code, 0)
        if (!eaten) {
            currentInputConnection?.commitText(c.toString(), 1)
        }
        sync()
    }

    private fun onBackspace() {
        val eaten = engineReady && RimeBridge.nativeProcessKey(XK_BACKSPACE, 0)
        if (!eaten) {
            currentInputConnection?.deleteSurroundingText(1, 0)
        }
        sync()
    }

    private fun onSpace() {
        val eaten = engineReady && RimeBridge.nativeProcessKey(0x20, 0)
        if (!eaten) {
            currentInputConnection?.commitText(" ", 1)
        }
        sync()
    }

    private fun onEnter() {
        val eaten = engineReady && RimeBridge.nativeProcessKey(XK_RETURN, 0)
        if (!eaten) {
            currentInputConnection?.apply {
                sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER))
                sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_ENTER))
            }
        }
        sync()
    }

    private fun onPunct(s: String) {
        // 组合中先确认当前首选，再上屏标点（主流输入法行为）
        if (engineReady && RimeBridge.nativeContext().isNotEmpty()) {
            RimeBridge.nativeProcessKey(0x20, 0)
            sync()
        }
        currentInputConnection?.commitText(s, 1)
    }

    private fun onCandidate(index: Int) {
        if (!engineReady) return
        RimeBridge.nativeProcessKey('1'.code + index, 0)
        sync()
    }

    /** 把引擎状态同步到编辑框与候选栏。 */
    private fun sync() {
        val ic = currentInputConnection ?: return
        if (!engineReady) {
            updateCandidates("", emptyList(), 0)
            return
        }
        val commit = RimeBridge.nativeCommit()
        if (commit.isNotEmpty()) {
            ic.commitText(commit, 1)
        }
        val raw = RimeBridge.nativeContext()
        if (raw.isEmpty()) {
            ic.finishComposingText()
            updateCandidates("", emptyList(), 0)
            return
        }
        val parts = raw.split('\u0001')
        val preedit = parts[0]
        val highlighted = parts.getOrNull(1)?.toIntOrNull() ?: 0
        val candidates = if (parts.size > 2) parts.subList(2, parts.size) else emptyList()
        ic.setComposingText(preedit, 1)
        updateCandidates(preedit, candidates, highlighted)
    }

    private fun updateCandidates(preedit: String, candidates: List<String>, highlighted: Int) {
        if (!::candidateBar.isInitialized) return
        preeditView.text = preedit
        preeditView.visibility = if (preedit.isEmpty()) View.GONE else View.VISIBLE
        candidateBar.removeAllViews()
        candidates.forEachIndexed { i, text ->
            val item = TextView(this).apply {
                this.text = text
                textSize = 20f
                gravity = Gravity.CENTER
                setTextColor(if (i == highlighted) COLOR_CAND_HL else COLOR_CAND)
                if (i == highlighted) typeface = Typeface.DEFAULT_BOLD
                setPadding(dp(14f), 0, dp(14f), 0)
                setOnClickListener { onCandidate(i) }
            }
            candidateBar.addView(
                item,
                LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.MATCH_PARENT
                )
            )
        }
    }

    override fun onStartInputView(info: android.view.inputmethod.EditorInfo?, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        ensureEngine()
        if (engineReady) {
            RimeBridge.nativeReset()
        }
        updateCandidates("", emptyList(), 0)
    }

    override fun onFinishInput() {
        super.onFinishInput()
        if (engineReady) {
            RimeBridge.nativeReset()
        }
    }
}
