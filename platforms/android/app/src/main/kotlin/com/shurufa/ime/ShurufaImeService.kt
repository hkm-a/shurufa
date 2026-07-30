package com.shurufa.ime

import android.content.ClipboardManager
import android.graphics.Color
import android.graphics.Typeface
import android.inputmethodservice.InputMethodService
import android.os.Handler
import android.os.Looper
import android.util.TypedValue
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import android.widget.ScrollView
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
    /// 键盘按键区容器；切换字母页/符号页时只重建这一部分
    private lateinit var keyArea: LinearLayout
    /// 中英文标记键，切换后更新键帽
    private var langKey: TextView? = null
    /// 当前是否显示符号页
    private var symbolMode = false
    /// 顶部同步提示条：显示电脑同步来的剪贴板，点击一键上屏
    private var syncBar: TextView? = null
    private var pendingSyncText: String? = null
    private val syncPoll = Handler(Looper.getMainLooper())
    /// 剪贴板历史面板（覆盖按键区）与去重用的最近系统剪贴板文本
    private var historyPanel: LinearLayout? = null
    private var lastClipboardText: String? = null

    override fun onCreate() {
        super.onCreate()
        ensureEngine()
        // 同步与历史库初始化含 I/O，放后台线程避免阻塞主线程
        thread(name = "sync-start") {
            try {
                ClipStore.ensureInit(applicationContext)
                SyncBridge.ensureStarted(applicationContext)
            } catch (e: Throwable) {
                android.util.Log.e("shurufa", "同步/历史初始化失败", e)
            }
        }
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

        // 顶部同步提示条：电脑复制的内容同步到手机后在此一键上屏
        syncBar = TextView(this).apply {
            setBackgroundColor(0xFFFFF3E0.toInt())
            setTextColor(COLOR_TEXT)
            textSize = 14f
            setPadding(dp(12f), dp(9f), dp(12f), dp(9f))
            visibility = View.GONE
            setOnClickListener {
                pendingSyncText?.let { currentInputConnection?.commitText(it, 1) }
                pendingSyncText = null
                visibility = View.GONE
            }
        }
        root.addView(syncBar)

        // 候选栏行：左侧剪贴板按钮 + 可横向滚动的候选列表
        val topRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, dp(44f)
            )
        }
        val clipButton = TextView(this).apply {
            text = "⊞"
            gravity = Gravity.CENTER
            textSize = 20f
            setTextColor(COLOR_CAND_HL)
            setBackgroundColor(COLOR_KEY_FUNC)
            setPadding(dp(14f), 0, dp(14f), 0)
            setOnClickListener { toggleHistory() }
        }
        topRow.addView(
            clipButton,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.MATCH_PARENT
            )
        )
        candidateBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
        }
        val scroll = HorizontalScrollView(this).apply {
            addView(candidateBar)
            isHorizontalScrollBarEnabled = false
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, 1f)
        }
        topRow.addView(scroll)
        root.addView(topRow)

        keyArea = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        root.addView(keyArea)
        rebuildKeys()

        // 历史面板：默认隐藏，展开时盖住按键区
        historyPanel = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
        }
        root.addView(historyPanel)
        return root
    }

    // ---------- 剪贴板历史面板 ----------

    private fun toggleHistory() {
        val panel = historyPanel ?: return
        if (panel.visibility == View.VISIBLE) {
            panel.visibility = View.GONE
            keyArea.visibility = View.VISIBLE
        } else {
            populateHistory(panel)
            panel.visibility = View.VISIBLE
            keyArea.visibility = View.GONE
        }
    }

    private fun populateHistory(panel: LinearLayout) {
        panel.removeAllViews()
        panel.addView(TextView(this).apply {
            text = "剪贴板历史（点击上屏，再点 ⊞ 返回）"
            textSize = 12f
            setTextColor(COLOR_PREEDIT)
            setPadding(dp(12f), dp(6f), dp(12f), dp(6f))
        })
        val entries = try {
            ClipStore.list(30)
        } catch (e: Throwable) {
            emptyList()
        }
        if (entries.isEmpty()) {
            panel.addView(TextView(this).apply {
                text = "（暂无历史）"
                setTextColor(COLOR_PREEDIT)
                setPadding(dp(12f), dp(16f), dp(12f), dp(16f))
            })
            return
        }
        val list = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        for (entry in entries) {
            list.addView(TextView(this).apply {
                text = entry.text.replace('\n', ' ').take(48)
                textSize = 16f
                setTextColor(COLOR_TEXT)
                setPadding(dp(14f), dp(12f), dp(14f), dp(12f))
                setOnClickListener {
                    currentInputConnection?.commitText(entry.text, 1)
                    toggleHistory()
                }
            })
        }
        panel.addView(ScrollView(this).apply {
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, dp(210f)
            )
            addView(list)
        })
    }

    /// 键盘弹出时读系统剪贴板：新内容入本地历史并推送电脑（手机→电脑）。
    /// 输入法在前台，读取合法；安卓 12+ 会有一次系统提示，属正常。
    private fun captureSystemClipboard() {
        val cm = getSystemService(CLIPBOARD_SERVICE) as? ClipboardManager ?: return
        val clip = cm.primaryClip ?: return
        if (clip.itemCount == 0) return
        val text = clip.getItemAt(0).coerceToText(this)?.toString()
        if (text.isNullOrBlank() || text == lastClipboardText) return
        lastClipboardText = text
        try {
            ClipStore.insert(text, "本机")
            SyncBridge.nativeSendClip(text)
        } catch (e: Throwable) {
            android.util.Log.e("shurufa", "剪贴板同步失败", e)
        }
    }

    /// 依据 symbolMode 重建按键区：字母页含常驻数字行，符号页含标点表。
    private fun rebuildKeys() {
        if (!::keyArea.isInitialized) return
        keyArea.removeAllViews()
        langKey = null
        if (symbolMode) {
            buildSymbolPage()
        } else {
            buildLetterPage()
        }
    }

    private fun buildLetterPage() {
        // 常驻数字行：有组合时选词，无组合时上屏数字（复用 onLetter 的直通逻辑）
        val numberRow = rowLayout()
        "1234567890".forEach { c ->
            numberRow.addView(key(c.toString(), 1f) { onLetter(c) })
        }
        keyArea.addView(numberRow)

        listOf("qwertyuiop", "asdfghjkl", "zxcvbnm").forEachIndexed { index, row ->
            keyArea.addView(buildLetterRow(row, withBackspace = index == 2))
        }
        keyArea.addView(buildBottomRow())
    }

    private fun buildSymbolPage() {
        // 常用中文标点与符号，点击直接上屏（组合中先确认首选）
        listOf(
            "，。？！；：",
            "“”‘’（）",
            "、~·@#￥",
            "%&*—…/",
        ).forEach { line ->
            val row = rowLayout()
            line.forEach { c ->
                row.addView(key(c.toString(), 1f) { onPunct(c.toString()) })
            }
            keyArea.addView(row)
        }
        keyArea.addView(buildBottomRow())
    }

    private fun rowLayout(): LinearLayout = LinearLayout(this).apply {
        orientation = LinearLayout.HORIZONTAL
        layoutParams = LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT, dp(50f)
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

    /// 底部功能行：符号页切换、中英切换、逗号、空格、句号、回车。
    private fun buildBottomRow(): LinearLayout {
        val row = rowLayout()
        val symbolLabel = if (symbolMode) "返回" else "符"
        row.addView(key(symbolLabel, 1.4f, functional = true) {
            symbolMode = !symbolMode
            rebuildKeys()
        })
        langKey = key(langLabel(), 1.4f, functional = true) { onToggleLang() }
        row.addView(langKey)
        row.addView(key("，", 1f, functional = true) { onPunct("，") })
        row.addView(key("空格", 3.4f) { onSpace() })
        row.addView(key("。", 1f, functional = true) { onPunct("。") })
        row.addView(key("回车", 1.8f, functional = true) { onEnter() })
        return row
    }

    private fun langLabel(): String =
        if (engineReady && RimeBridge.nativeIsAscii()) "英" else "中"

    private fun onToggleLang() {
        if (!engineReady) return
        RimeBridge.nativeToggleAscii()
        // 切换会清掉未完成组合，刷新界面
        RimeBridge.nativeReset()
        langKey?.text = langLabel()
        sync()
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
        langKey?.text = langLabel()
        updateCandidates("", emptyList(), 0)
        // 历史面板复位到键盘态
        historyPanel?.visibility = View.GONE
        keyArea.visibility = View.VISIBLE
        captureSystemClipboard()
        startSyncPolling()
    }

    override fun onFinishInput() {
        super.onFinishInput()
        if (engineReady) {
            RimeBridge.nativeReset()
        }
        syncPoll.removeCallbacksAndMessages(null)
    }

    /// 键盘活跃期间轮询同步入站队列（安卓不用反向 JNI 回调）。
    private fun startSyncPolling() {
        syncPoll.removeCallbacksAndMessages(null)
        val tick = object : Runnable {
            override fun run() {
                val raw = try {
                    SyncBridge.nativePoll()
                } catch (e: Throwable) {
                    ""
                }
                if (raw.isNotEmpty()) {
                    val parts = raw.split('\u0001')
                    val from = parts.getOrNull(0).orEmpty()
                    val text = parts.drop(1).joinToString("\u0001")
                    if (text.isNotEmpty()) {
                        ClipStore.insert(text, "同步·$from")
                        pendingSyncText = text
                        val preview = text.replace('\n', ' ').take(30)
                        syncBar?.text = "来自 $from：$preview（点此上屏）"
                        syncBar?.visibility = View.VISIBLE
                    }
                }
                syncPoll.postDelayed(this, 1500)
            }
        }
        syncPoll.postDelayed(tick, 800)
    }
}
