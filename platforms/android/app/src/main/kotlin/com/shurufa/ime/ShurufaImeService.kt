package com.shurufa.ime

import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.res.Configuration
import android.graphics.BitmapFactory
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.StateListDrawable
import android.inputmethodservice.InputMethodService
import android.os.Handler
import android.os.Looper
import android.util.TypedValue
import android.view.Gravity
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.widget.HorizontalScrollView
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.PopupWindow
import android.widget.ScrollView
import android.widget.TextView
import androidx.core.content.FileProvider
import androidx.core.view.inputmethod.EditorInfoCompat
import androidx.core.view.inputmethod.InputConnectionCompat
import androidx.core.view.inputmethod.InputContentInfoCompat
import java.io.File
import kotlin.concurrent.thread

/**
 * 拼音键盘：QWERTY + 常驻数字行 + 符号页 + 中英切换，候选栏与
 * 剪贴板历史面板，双向同步接收。视觉支持浅色/深色主题、圆角按键
 * 与按下态、字符气泡、退格长按连删。
 *
 * 引擎在后台线程初始化（首次含词典部署），就绪前按键英文直通，
 * 与桌面端策略一致。
 */
class ShurufaImeService : InputMethodService() {

    /** 一套主题配色（ARGB）。 */
    private data class Palette(
        val bg: Int,
        val key: Int,
        val keyPressed: Int,
        val keyFunc: Int,
        val funcPressed: Int,
        val keyText: Int,
        val funcText: Int,
        val candidate: Int,
        val candidateHl: Int,
        val preedit: Int,
        val syncBg: Int,
        val syncText: Int,
        val accent: Int,
    )

    companion object {
        private const val XK_BACKSPACE = 0xff08
        private const val XK_RETURN = 0xff0d

        private val LIGHT = Palette(
            bg = 0xFFE8EAEE.toInt(),
            key = 0xFFFFFFFF.toInt(),
            keyPressed = 0xFFC7CED9.toInt(),
            keyFunc = 0xFFD3D8E1.toInt(),
            funcPressed = 0xFFBFC6D2.toInt(),
            keyText = 0xFF1A1A1A.toInt(),
            funcText = 0xFF33383F.toInt(),
            candidate = 0xFF1A1A1A.toInt(),
            candidateHl = 0xFFB86A34.toInt(),
            preedit = 0xFF8A9099.toInt(),
            syncBg = 0xFFFFF1DE.toInt(),
            syncText = 0xFF6A4A20.toInt(),
            accent = 0xFFB86A34.toInt(),
        )
        private val DARK = Palette(
            bg = 0xFF15171B.toInt(),
            key = 0xFF2B2F36.toInt(),
            keyPressed = 0xFF474D57.toInt(),
            keyFunc = 0xFF373C44.toInt(),
            funcPressed = 0xFF4A5059.toInt(),
            keyText = 0xFFECECEC.toInt(),
            funcText = 0xFFCFD3D9.toInt(),
            candidate = 0xFFECECEC.toInt(),
            candidateHl = 0xFFE6A867.toInt(),
            preedit = 0xFF8B9199.toInt(),
            syncBg = 0xFF3A331F.toInt(),
            syncText = 0xFFEAD6B0.toInt(),
            accent = 0xFFE6A867.toInt(),
        )

        @Volatile
        private var engineReady = false

        @Volatile
        private var engineStarting = false
    }

    private lateinit var candidateBar: LinearLayout
    private lateinit var preeditView: TextView
    private lateinit var keyArea: LinearLayout
    private var langKey: TextView? = null
    private var symbolMode = false
    private var syncBar: TextView? = null
    private var pendingSyncText: String? = null
    /// 同步收到的图片历史 id，syncBar 点击时上屏
    private var pendingSyncImageId: Int? = null
    private val syncPoll = Handler(Looper.getMainLooper())
    private var historyPanel: LinearLayout? = null
    private var lastClipboardText: String? = null

    /// 当前主题；随系统深色设置在重建输入视图时更新
    private var palette: Palette = LIGHT
    /// 字符气泡（按键放大预览）
    private var bubble: PopupWindow? = null
    private var bubbleText: TextView? = null
    /// 退格长按连删定时器
    private val repeatHandler = Handler(Looper.getMainLooper())

    override fun onCreate() {
        super.onCreate()
        ensureEngine()
        thread(name = "sync-start") {
            try {
                ClipStore.ensureInit(applicationContext)
                SyncBridge.ensureStarted(applicationContext)
            } catch (e: Throwable) {
                android.util.Log.e("shurufa", "同步/历史初始化失败", e)
            }
        }
    }

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

    // ---------- 主题与控件工厂 ----------

    private fun dp(v: Float): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, resources.displayMetrics).toInt()

    private fun isDark(): Boolean =
        (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
            Configuration.UI_MODE_NIGHT_YES

    /// 圆角按键背景：normal / pressed 两态。每次新建（Drawable 状态不可共享）。
    private fun keyBackground(normal: Int, pressed: Int): StateListDrawable {
        fun round(color: Int) = GradientDrawable().apply {
            setColor(color)
            cornerRadius = dp(7f).toFloat()
        }
        return StateListDrawable().apply {
            addState(intArrayOf(android.R.attr.state_pressed), round(pressed))
            addState(intArrayOf(), round(normal))
        }
    }

    /// 字符键：圆角、按下态、按下弹放大气泡。
    private fun charKey(label: String, weight: Float, onTap: () -> Unit): TextView =
        TextView(this).apply {
            text = label
            gravity = Gravity.CENTER
            textSize = 20f
            setTextColor(palette.keyText)
            background = keyBackground(palette.key, palette.keyPressed)
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, weight)
                .apply { setMargins(dp(2.5f), dp(3.5f), dp(2.5f), dp(3.5f)) }
            setOnTouchListener { v, ev ->
                when (ev.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        isPressed = true
                        showBubble(v, label)
                        true
                    }
                    MotionEvent.ACTION_UP -> {
                        isPressed = false
                        hideBubble()
                        onTap()
                        performClick()
                        true
                    }
                    MotionEvent.ACTION_CANCEL -> {
                        isPressed = false
                        hideBubble()
                        true
                    }
                    else -> false
                }
            }
        }

    /// 功能键：圆角、按下态，无气泡。返回 TextView 供调用方持有（如中英键）。
    private fun funcKey(label: String, weight: Float, onTap: () -> Unit): TextView =
        TextView(this).apply {
            text = label
            gravity = Gravity.CENTER
            textSize = 16f
            setTextColor(palette.funcText)
            typeface = Typeface.DEFAULT_BOLD
            background = keyBackground(palette.keyFunc, palette.funcPressed)
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, weight)
                .apply { setMargins(dp(2.5f), dp(3.5f), dp(2.5f), dp(3.5f)) }
            setOnClickListener { onTap() }
        }

    private fun showBubble(anchor: View, label: String) {
        val w = dp(46f)
        val h = dp(54f)
        val tv = bubbleText ?: TextView(this).apply {
            gravity = Gravity.CENTER
            textSize = 26f
            setTextColor(palette.keyText)
            background = GradientDrawable().apply {
                setColor(palette.key)
                cornerRadius = dp(8f).toFloat()
                setStroke(dp(1f), palette.keyPressed)
            }
            bubbleText = this
        }
        tv.text = label
        tv.setTextColor(palette.keyText)
        val pop = bubble ?: PopupWindow(tv, w, h, false).also {
            it.isClippingEnabled = false
            bubble = it
        }
        val loc = IntArray(2)
        anchor.getLocationInWindow(loc)
        val x = loc[0] + anchor.width / 2 - w / 2
        val y = loc[1] - h - dp(2f)
        try {
            if (pop.isShowing) {
                pop.update(x, y, w, h)
            } else {
                pop.showAtLocation(anchor, Gravity.NO_GRAVITY, x, y)
            }
        } catch (e: Throwable) {
            // 个别 ROM 的 IME 窗口不允许弹窗，降级为无气泡
        }
    }

    private fun hideBubble() {
        try {
            bubble?.dismiss()
        } catch (e: Throwable) {
        }
    }

    // ---------- 键盘视图 ----------

    override fun onCreateInputView(): View {
        palette = if (isDark()) DARK else LIGHT

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(palette.bg)
            setPadding(dp(3f), dp(4f), dp(3f), dp(6f))
        }

        preeditView = TextView(this).apply {
            setTextColor(palette.preedit)
            textSize = 13f
            setPadding(dp(12f), dp(3f), dp(12f), 0)
            visibility = View.GONE
        }
        root.addView(preeditView)

        syncBar = TextView(this).apply {
            background = GradientDrawable().apply {
                setColor(palette.syncBg)
                cornerRadius = dp(8f).toFloat()
            }
            setTextColor(palette.syncText)
            textSize = 14f
            setPadding(dp(14f), dp(10f), dp(14f), dp(10f))
            visibility = View.GONE
            setOnClickListener {
                val imgId = pendingSyncImageId
                if (imgId != null) {
                    commitImage(imgId)
                } else {
                    pendingSyncText?.let { currentInputConnection?.commitText(it, 1) }
                }
                pendingSyncText = null
                pendingSyncImageId = null
                visibility = View.GONE
            }
        }
        root.addView(syncBar, LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT
        ).apply { setMargins(dp(3f), dp(2f), dp(3f), dp(2f)) })

        // 候选栏行：左侧剪贴板按钮 + 横向滚动候选
        val topRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, dp(46f)
            )
        }
        val clipButton = TextView(this).apply {
            text = "⊞"
            gravity = Gravity.CENTER
            textSize = 21f
            setTextColor(palette.accent)
            setPadding(dp(16f), 0, dp(16f), 0)
            setOnClickListener { toggleHistory() }
        }
        topRow.addView(
            clipButton,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.MATCH_PARENT
            )
        )
        candidateBar = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
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
            text = "剪贴板历史 · 点击上屏 · 再点 ⊞ 返回"
            textSize = 12f
            setTextColor(palette.preedit)
            setPadding(dp(14f), dp(8f), dp(14f), dp(8f))
        })
        val entries = try {
            ClipStore.list(30)
        } catch (e: Throwable) {
            emptyList()
        }
        if (entries.isEmpty()) {
            panel.addView(TextView(this).apply {
                text = "（暂无历史）"
                setTextColor(palette.preedit)
                setPadding(dp(14f), dp(20f), dp(14f), dp(20f))
            })
            return
        }
        val list = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        for (entry in entries) {
            if (entry.kind == "image") {
                val bmp = try {
                    ClipStore.imageData(entry.id)?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }
                } catch (e: Throwable) {
                    null
                }
                val thumb = ImageView(this).apply {
                    if (bmp != null) setImageBitmap(bmp) else contentDescription = "图片"
                    adjustViewBounds = true
                    maxHeight = dp(130f)
                    scaleType = ImageView.ScaleType.FIT_START
                    background = keyBackground(palette.key, palette.keyPressed)
                    setPadding(dp(10f), dp(8f), dp(10f), dp(8f))
                    setOnClickListener {
                        commitImage(entry.id)
                        toggleHistory()
                    }
                }
                list.addView(thumb, LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
                ).apply { setMargins(dp(4f), dp(3f), dp(4f), dp(3f)) })
                continue
            }
            list.addView(TextView(this).apply {
                text = entry.text.replace('\n', ' ').take(48)
                textSize = 16f
                setTextColor(palette.keyText)
                background = keyBackground(palette.key, palette.keyPressed)
                setPadding(dp(14f), dp(13f), dp(14f), dp(13f))
                setOnClickListener {
                    currentInputConnection?.commitText(entry.text, 1)
                    toggleHistory()
                }
            }, LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT
            ).apply { setMargins(dp(4f), dp(3f), dp(4f), dp(3f)) })
        }
        panel.addView(ScrollView(this).apply {
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, dp(210f)
            )
            addView(list)
        })
    }

    /// 图片上屏：经 FileProvider 暴露 content:// URI，用 commitContent 交给
    /// 目标输入框。仅当输入框声明支持 image/png（微信、邮件等富文本框）时
    /// 生效；不支持的输入框静默（图片仍在历史面板可查看）。
    private fun commitImage(id: Int) {
        val png = try {
            ClipStore.imageData(id)
        } catch (e: Throwable) {
            null
        } ?: return
        val ic = currentInputConnection ?: return
        val editor = currentInputEditorInfo ?: return
        val supported = EditorInfoCompat.getContentMimeTypes(editor)
            .any { ClipDescription.compareMimeTypes(it, "image/png") }
        if (!supported) return
        try {
            val dir = File(cacheDir, "shared").apply { mkdirs() }
            val f = File(dir, "clip_$id.png")
            f.writeBytes(png)
            val uri = FileProvider.getUriForFile(this, "$packageName.fileprovider", f)
            val desc = ClipDescription("图片", arrayOf("image/png"))
            val content = InputContentInfoCompat(uri, desc, null)
            InputConnectionCompat.commitContent(
                ic, editor, content,
                InputConnectionCompat.INPUT_CONTENT_GRANT_READ_URI_PERMISSION, null
            )
        } catch (e: Throwable) {
            android.util.Log.e("shurufa", "图片上屏失败", e)
        }
    }

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

    // ---------- 按键布局 ----------

    private fun rebuildKeys() {
        if (!::keyArea.isInitialized) return
        keyArea.removeAllViews()
        langKey = null
        if (symbolMode) buildSymbolPage() else buildLetterPage()
    }

    private fun buildLetterPage() {
        val numberRow = rowLayout()
        "1234567890".forEach { c ->
            numberRow.addView(charKey(c.toString(), 1f) { onLetter(c) })
        }
        keyArea.addView(numberRow)

        listOf("qwertyuiop", "asdfghjkl", "zxcvbnm").forEachIndexed { index, row ->
            keyArea.addView(buildLetterRow(row, withBackspace = index == 2))
        }
        keyArea.addView(buildBottomRow())
    }

    private fun buildSymbolPage() {
        listOf(
            "，。？！；：",
            "“”‘’（）",
            "、~·@#￥",
            "%&*—…/",
        ).forEach { line ->
            val row = rowLayout()
            line.forEach { c ->
                row.addView(charKey(c.toString(), 1f) { onPunct(c.toString()) })
            }
            keyArea.addView(row)
        }
        keyArea.addView(buildBottomRow())
    }

    private fun rowLayout(): LinearLayout = LinearLayout(this).apply {
        orientation = LinearLayout.HORIZONTAL
        layoutParams = LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT, dp(52f)
        )
    }

    private fun buildLetterRow(letters: String, withBackspace: Boolean): LinearLayout {
        val row = rowLayout()
        if (letters.length == 9) row.addView(spacer(0.5f))
        letters.forEach { c -> row.addView(charKey(c.toString(), 1f) { onLetter(c) }) }
        if (withBackspace) {
            row.addView(backspaceKey(1.5f))
        } else if (letters.length == 9) {
            row.addView(spacer(0.5f))
        }
        return row
    }

    /// 退格键：轻触删一字，长按连续删除，上滑清空整个输入。
    private fun backspaceKey(weight: Float): TextView = TextView(this).apply {
        text = "⌫"
        gravity = Gravity.CENTER
        textSize = 19f
        setTextColor(palette.funcText)
        background = keyBackground(palette.keyFunc, palette.funcPressed)
        layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, weight)
            .apply { setMargins(dp(2.5f), dp(3.5f), dp(2.5f), dp(3.5f)) }
        var downY = 0f
        var cleared = false
        var repeated = false
        val repeat = object : Runnable {
            override fun run() {
                onBackspace()
                repeated = true
                repeatHandler.postDelayed(this, 55)
            }
        }
        setOnTouchListener { _, ev ->
            when (ev.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    isPressed = true
                    downY = ev.y
                    cleared = false
                    repeated = false
                    repeatHandler.postDelayed(repeat, 400)
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    if (!cleared && downY - ev.y > dp(36f)) {
                        cleared = true
                        repeatHandler.removeCallbacks(repeat)
                        onClearAll()
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    isPressed = false
                    repeatHandler.removeCallbacks(repeat)
                    if (!cleared && !repeated) onBackspace()
                    performClick()
                    true
                }
                MotionEvent.ACTION_CANCEL -> {
                    isPressed = false
                    repeatHandler.removeCallbacks(repeat)
                    true
                }
                else -> false
            }
        }
    }

    private fun onClearAll() {
        val ic = currentInputConnection ?: return
        ic.beginBatchEdit()
        if (engineReady) RimeBridge.nativeReset()
        ic.finishComposingText()
        ic.deleteSurroundingText(50000, 50000)
        ic.endBatchEdit()
        updateCandidates("", emptyList(), 0)
        syncBar?.visibility = View.GONE
    }

    private fun spacer(weight: Float): View = View(this).apply {
        layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, weight)
    }

    private fun buildBottomRow(): LinearLayout {
        val row = rowLayout()
        row.addView(funcKey(if (symbolMode) "返回" else "符", 1.4f) {
            symbolMode = !symbolMode
            rebuildKeys()
        })
        langKey = funcKey(langLabel(), 1.4f) { onToggleLang() }
        row.addView(langKey)
        row.addView(funcKey("，", 1f) { onPunct("，") })
        row.addView(charKey("空格", 3.4f) { onSpace() }.apply {
            // 空格用字符键的底色更醒目，但不需要气泡：覆盖触摸为纯点击
            setOnTouchListener(null)
            setOnClickListener { onSpace() }
        })
        row.addView(funcKey("。", 1f) { onPunct("。") })
        row.addView(funcKey("回车", 1.8f) { onEnter() })
        return row
    }

    private fun langLabel(): String =
        if (engineReady && RimeBridge.nativeIsAscii()) "英" else "中"

    private fun onToggleLang() {
        if (!engineReady) return
        RimeBridge.nativeToggleAscii()
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
        if (!eaten) currentInputConnection?.commitText(c.toString(), 1)
        sync()
    }

    private fun onBackspace() {
        val eaten = engineReady && RimeBridge.nativeProcessKey(XK_BACKSPACE, 0)
        if (!eaten) currentInputConnection?.deleteSurroundingText(1, 0)
        sync()
    }

    private fun onSpace() {
        val eaten = engineReady && RimeBridge.nativeProcessKey(0x20, 0)
        if (!eaten) currentInputConnection?.commitText(" ", 1)
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

    private fun sync() {
        val ic = currentInputConnection ?: return
        if (!engineReady) {
            updateCandidates("", emptyList(), 0)
            return
        }
        val commit = RimeBridge.nativeCommit()
        if (commit.isNotEmpty()) ic.commitText(commit, 1)
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
                setTextColor(if (i == highlighted) palette.candidateHl else palette.candidate)
                if (i == highlighted) typeface = Typeface.DEFAULT_BOLD
                setPadding(dp(15f), 0, dp(15f), 0)
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
        // 系统深色设置变化后重建输入视图，刷新主题
        if ((palette === DARK) != isDark()) {
            setInputView(onCreateInputView())
        }
        ensureEngine()
        if (engineReady) RimeBridge.nativeReset()
        langKey?.text = langLabel()
        updateCandidates("", emptyList(), 0)
        historyPanel?.visibility = View.GONE
        keyArea.visibility = View.VISIBLE
        captureSystemClipboard()
        startSyncPolling()
    }

    override fun onFinishInput() {
        super.onFinishInput()
        if (engineReady) RimeBridge.nativeReset()
        syncPoll.removeCallbacksAndMessages(null)
        hideBubble()
    }

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
                    val kind = parts.getOrNull(0).orEmpty()
                    val from = parts.getOrNull(1).orEmpty()
                    val payload = parts.drop(2).joinToString("\u0001")
                    when (kind) {
                        "text" -> if (payload.isNotEmpty()) {
                            ClipStore.insert(payload, "同步·$from")
                            pendingSyncText = payload
                            pendingSyncImageId = null
                            val preview = payload.replace('\n', ' ').take(30)
                            syncBar?.text = "来自 $from：$preview（点此上屏）"
                            syncBar?.visibility = View.VISIBLE
                        }
                        "image" -> payload.toIntOrNull()?.let { id ->
                            pendingSyncImageId = id
                            pendingSyncText = null
                            syncBar?.text = "来自 $from：收到图片（点此上屏）"
                            syncBar?.visibility = View.VISIBLE
                        }
                    }
                }
                syncPoll.postDelayed(this, 1500)
            }
        }
        syncPoll.postDelayed(tick, 800)
    }
}
