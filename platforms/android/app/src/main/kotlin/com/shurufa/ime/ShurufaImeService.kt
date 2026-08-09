package com.shurufa.ime

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.StateListDrawable
import android.inputmethodservice.InputMethodService
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.text.Editable
import android.text.TextWatcher
import android.util.TypedValue
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.widget.EditText
import android.widget.HorizontalScrollView
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import androidx.core.content.FileProvider
import androidx.core.view.inputmethod.EditorInfoCompat
import androidx.core.view.inputmethod.InputConnectionCompat
import androidx.core.view.inputmethod.InputContentInfoCompat
import java.io.File
import java.io.ByteArrayOutputStream
import java.net.URLConnection
import kotlin.concurrent.thread

/**
 * 拼音键盘：QWERTY + 常驻数字行 + 符号页 + 中英切换，候选栏与
 * 剪贴板历史面板，双向同步接收。视觉支持浅色/深色主题、圆角按键
 * 与按下态、字符气泡、退格长按连删。
 *
 * 引擎在后台线程初始化（首次含词典部署）。初始化期间，当前输入框的
 * 按键会短暂缓存并在引擎就绪后自动回放，避免用户首次使用误输入英文。
 *
 * 候选栏左侧有「中/英」模式角标（B9），长按候选可复制或删除用户词条（B8）；
 * 剪贴板历史面板支持文本搜索与条目置顶/删除（A6），标题行展示今日/累计
 * 打字统计（B12，由 Rust 侧埋点）。
 */
class ShurufaImeService : InputMethodService() {

    /** 一套主题配色（ARGB）。 */
    internal data class Palette(
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
        val accent: Int,
    )

    companion object {
        private const val XK_BACKSPACE = 0xff08
        private const val XK_RETURN = 0xff0d
        // 方案资源变化时递增，确保同一应用版本也会重新解包词典。
        private const val SCHEMA_BUNDLE_VERSION = "rime-ice-20260807"
        /// 缩略图采样目标边长（px），仅为展示用，不必保留原图尺寸
        private const val THUMBNAIL_TARGET = 260
        /// 预览图采样目标边长（px），兼顾清晰度与内存
        private const val PREVIEW_TARGET = 2048

        @Volatile
        private var engineReady = false

        @Volatile
        private var engineStarting = false
    }

    private lateinit var candidateBar: LinearLayout
    private lateinit var expandedCandidateBar: LinearLayout
    private lateinit var candidateExpandButton: TextView
    private lateinit var keyArea: LinearLayout
    private var voice: VoiceInputController? = null
    /// 键盘内置的语音状态条（不受系统 Toast 抑制，必现）。
    private var voiceStatusBar: TextView? = null
    private var inputRoot: LinearLayout? = null
    private var symbolMode = false
    /// 大写锁定（微信输入法同款 capslock 键：行首图标键）
    private var shiftMode = false
    private var historyPanel: LinearLayout? = null
    /// 微信输入法 S33 同款图片预览键盘（点图后先预览再保存/发送）
    private var previewKeyboard: LinearLayout? = null
    /// 预览键盘当前展示的图片历史 id
    private var previewImageId: Int? = null
    /// AI 帮写面板（提示输入 + 草稿预览 + 粘贴按钮）
    private var aiPanel: LinearLayout? = null
    private var aiInputBox: EditText? = null
    private var aiStatusLine: TextView? = null
    private var aiDraftView: TextView? = null
    private var aiPasteButton: TextView? = null
    /// 最近一次成功返回的 AI 草稿；点击「粘贴」写入编辑器。
    private var aiLastDraft: String? = null
    private val mainHandler = Handler(Looper.getMainLooper())
    /// 图片解码 / 相册写入 / 剪贴板大查询走 IO 线程：避免阻塞 IME 主线程导致按键抖动。
    private val ioExecutor = java.util.concurrent.Executors.newSingleThreadExecutor { task ->
        Thread(task, "shurufa-io").apply { isDaemon = true }
    }
    private var inputGeneration = 0L
    private var candidatesExpanded = false
    private var compositionCursorOverride: Int? = null
    private val warmupQueue = EngineWarmupQueue<PendingInput>()

    /// 候选行左侧的中/英模式角标（B9），engineReady 时随 sync/切换刷新。
    private var modeBadge: TextView? = null
    /// 历史面板搜索框（A6），只在文本模式下创建并显示。
    private var historySearchBox: EditText? = null
    /// 搜索输入 250ms 防抖任务。
    private var historySearchRunnable: Runnable? = null

    /// 当前主题；随系统深色设置在重建输入视图时更新
    private var palette: Palette = paletteFromSkin(SkinPalette.lightDefault())

    /// 附件发送结果：SENT=已投递，COPIED=已复制到剪贴板（需长按粘贴），FAILED=彻底失败
    private enum class SendResult { SENT, COPIED, FAILED }

    private sealed class PendingInput {
        data class Letter(val value: Char) : PendingInput()
        object Backspace : PendingInput()
        object Space : PendingInput()
        object Enter : PendingInput()
        data class Punct(val value: String) : PendingInput()
    }

    override fun onCreate() {
        super.onCreate()
        // 不在 onCreate 中启动剪贴板同步服务——IME 刚创建时就启动
        // 会触发系统（尤其 MIUI/HyperOS）的剪贴板访问面板，导致每次点开
        // 文本框时弹出剪贴板而非键盘。改为大幅延迟启动，等用户稳定使用后再捕获。
        thread(name = "clipboard-sync-delayed") {
            try {
                Thread.sleep(60_000) // 1 分钟后才启动剪贴板同步
                ClipboardSyncService.start(applicationContext)
            } catch (_: InterruptedException) {
                // 被中断说明 IME 已销毁，无需启动
            }
        }
        voice = VoiceInputController(applicationContext).apply {
            onResult = { text -> commitVoiceResult(text, showBar = true) }
            onCommit = { text -> commitVoiceResult(text, showBar = false) }
            onPartial = { text -> updateVoicePreview(text) }
            onStatus = { s -> updateVoiceStatus(s) }
        }
        ensureEngine()
        thread(name = "sync-start") {
            try {
                ClipStore.ensureInit(applicationContext)
            } catch (e: Throwable) {
                android.util.Log.e("shurufa", "同步/历史初始化失败", e)
            }
        }
    }

    override fun onStartInput(attribute: EditorInfo?, restarting: Boolean) {
        super.onStartInput(attribute, restarting)
        inputGeneration += 1
        warmupQueue.clear()
        // 每次打开新输入框时，强制关闭历史面板和图片预览，只展示纯键盘。
        historyPanel?.visibility = View.GONE
        previewImageId = null
        previewKeyboard?.visibility = View.GONE
        if (::keyArea.isInitialized) keyArea.visibility = View.VISIBLE
    }

    override fun onFinishInput() {
        resetCompositionForInputChange()
        inputGeneration += 1
        warmupQueue.clear()
        historyPanel?.visibility = View.GONE
        previewKeyboard?.visibility = View.GONE
        aiPanel?.visibility = View.GONE
        aiLastDraft = null
        aiDraftView?.text = ""
        aiStatusLine?.text = ""
        previewImageId = null
        if (::keyArea.isInitialized) keyArea.visibility = View.VISIBLE
        super.onFinishInput()
    }

    /** 系统深色模式切换时键盘内热刷：重建输入视图并清空组合，避免残留旧配色。 */
    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (!isInputViewShown) return
        val fresh = paletteFromSkin(SkinConfig.load(this, isDark()))
        if (fresh != palette) {
            palette = fresh
            resetCompositionForInputChange()
            setInputView(onCreateInputView())
        }
    }

    override fun onUpdateSelection(
        oldSelStart: Int,
        oldSelEnd: Int,
        newSelStart: Int,
        newSelEnd: Int,
        candidatesStart: Int,
        candidatesEnd: Int,
    ) {
        super.onUpdateSelection(
            oldSelStart,
            oldSelEnd,
            newSelStart,
            newSelEnd,
            candidatesStart,
            candidatesEnd,
        )
        if (oldSelStart == newSelStart && oldSelEnd == newSelEnd) return

        // 用户在组合串内点按时，编辑器和 Rime 各自保存光标；必须先同步
        // 引擎位置，否则下一次输入会错误地追加到拼音尾部。
        if (
            engineReady &&
            newSelStart == newSelEnd &&
            candidatesStart >= 0 &&
            candidatesEnd >= candidatesStart &&
            newSelStart in candidatesStart..candidatesEnd
        ) {
            val offset = newSelStart - candidatesStart
            RimeBridge.nativeSetCursor(offset)
            // 覆盖一次组合光标：很多编辑器在下一次 commit 前不会主动回流选区，
            // 这里主动以点选位置重写 composition 的光标，避免视觉光标跳到尾部。
            currentInputConnection?.setComposingRegion(candidatesStart, candidatesEnd)
            currentInputConnection?.setSelection(newSelStart, newSelEnd)
            compositionCursorOverride = offset
            return
        }

        // 用户把光标移到组合区外或选中其他文本时，Rime 仍保存着旧组合。
        // 先结束该组合，后续输入便从新的编辑器选区开始。
        if (candidatesStart < 0 || newSelStart != newSelEnd) {
            compositionCursorOverride = null
            resetCompositionForInputChange()
        }
    }

    private fun resetCompositionForInputChange() {
        compositionCursorOverride = null
        if (!engineReady || RimeBridge.nativeContext().isEmpty()) return
        RimeBridge.nativeReset()
        currentInputConnection?.finishComposingText()
        candidatesExpanded = false
        updateCandidates("", emptyList(), 0)
    }

    private fun ensureEngine() {
        if (engineReady || engineStarting) return
        engineStarting = true
        thread(name = "rime-init") {
            var initialized = false
            try {
                val schemas = unpackSchemas()
                val userDir = File(filesDir, "rime").apply { mkdirs() }
                initialized = RimeBridge.nativeInit(schemas.absolutePath, userDir.absolutePath)
                engineReady = initialized
            } catch (e: Exception) {
                android.util.Log.e("shurufa", "引擎初始化失败", e)
            } finally {
                engineStarting = false
                mainHandler.post {
                    flushWarmupInput(initialized)
                    if (::keyArea.isInitialized && !symbolMode) rebuildKeys()
                }
            }
        }
    }

    private fun enqueueDuringWarmup(input: PendingInput) {
        warmupQueue.enqueue(inputGeneration, input)
        showStatus("正在准备中文输入…", palette.accent)
        ensureEngine()
    }

    private fun flushWarmupInput(initialized: Boolean) {
        warmupQueue.drain(inputGeneration).forEach { input ->
            if (initialized) {
                when (input) {
                    is PendingInput.Letter -> onLetter(input.value)
                    PendingInput.Backspace -> onBackspace()
                    PendingInput.Space -> onSpace()
                    PendingInput.Enter -> onEnter()
                    is PendingInput.Punct -> onPunct(input.value)
                }
            } else {
                fallbackPendingInput(input)
            }
        }
    }

    private fun fallbackPendingInput(input: PendingInput) {
        when (input) {
            is PendingInput.Letter -> currentInputConnection?.commitText(input.value.toString(), 1)
            PendingInput.Backspace -> currentInputConnection?.deleteSurroundingText(1, 0)
            PendingInput.Space -> currentInputConnection?.commitText(" ", 1)
            PendingInput.Enter -> currentInputConnection?.apply {
                sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER))
                sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_ENTER))
            }
            is PendingInput.Punct -> currentInputConnection?.commitText(input.value, 1)
        }
        showStatus("中文输入初始化失败，已保留原始输入", 0xFF8B3A3A.toInt())
    }

    private fun unpackSchemas(): File {
        val dest = File(filesDir, "schemas")
        val marker = File(dest, ".version")
        val version = "${appVersionCode()}-$SCHEMA_BUNDLE_VERSION"
        if (marker.takeIf { it.exists() }?.readText() == version) {
            CloudDictionaryUpdater.applyOverlay(applicationContext, dest)
            return dest
        }
        dest.deleteRecursively()
        dest.mkdirs()
        copySchemaAssets("schemas", dest)
        marker.writeText(version)
        CloudDictionaryUpdater.applyOverlay(applicationContext, dest)
        return dest
    }

    private fun copySchemaAssets(assetPath: String, destination: File) {
        val children = assets.list(assetPath).orEmpty()
        if (children.isEmpty()) {
            assets.open(assetPath).use { input ->
                destination.parentFile?.mkdirs()
                destination.outputStream().use { input.copyTo(it) }
            }
            return
        }
        destination.mkdirs()
        children.forEach { child ->
            copySchemaAssets("$assetPath/$child", File(destination, child))
        }
    }

    @Suppress("DEPRECATION")
    private fun appVersionCode(): Long {
        val info = packageManager.getPackageInfo(packageName, 0)
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            info.longVersionCode
        } else {
            info.versionCode.toLong()
        }
    }

    // ---------- 主题与控件工厂 ----------

    private fun dp(v: Float): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, resources.displayMetrics).toInt()

    private fun isDark(): Boolean =
        (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
            Configuration.UI_MODE_NIGHT_YES

    /** 将跨端皮肤配置适配为现有输入法视图所用的颜色集。 */
    private fun loadPalette(): Palette {
        return paletteFromSkin(SkinConfig.load(this, isDark()))
    }

    /** 保持既有输入视图 API，同时让默认颜色只有共享皮肤模型一个来源。 */
    private fun paletteFromSkin(skin: SkinPalette): Palette {
        return Palette(
            bg = skin.bg,
            key = skin.key,
            keyPressed = skin.keyPressed,
            keyFunc = skin.keyFunc,
            funcPressed = skin.funcPressed,
            keyText = skin.keyText,
            funcText = skin.funcText,
            candidate = skin.candidate,
            candidateHl = skin.candidateHl,
            preedit = skin.preedit,
            accent = skin.accent,
        )
    }

    private fun isLandscape(): Boolean =
        resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE

    private fun candidateBarHeight(): Int = dp(if (isLandscape()) 36f else 42f)


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

    // ---------- 键盘视图 ----------

    /** 横屏保持应用内容与键盘同屏，避免系统切换到占据大面积空白的提取编辑模式。 */
    override fun onEvaluateFullscreenMode(): Boolean = false

    override fun onCreateInputView(): View {
        palette = loadPalette()

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(palette.bg)
            setPadding(dp(1f), dp(1f), dp(1f), dp(1f))
        }
        inputRoot = root


        // 语音状态条：位于键盘最顶部，录音/识别过程必然可见（不受 Toast 抑制影响）。
        voiceStatusBar = TextView(this).apply {
            text = "🎤 正在聆听…（松开结束 / 上滑取消）"
            textSize = 13f
            gravity = Gravity.CENTER
            setTextColor(0xFFFFFFFF.toInt())
            setBackgroundColor(palette.accent)
            visibility = View.GONE
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                dp(34f),
            )
        }
        root.addView(voiceStatusBar)

        // 候选词区支持横滑浏览、展开九宫格与翻页，候选数量由 Rime 保持唯一来源。
        val candidatePanel = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(palette.key)
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
            )
        }
        candidateBar = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        val scroll = HorizontalScrollView(this).apply {
            addView(candidateBar)
            isHorizontalScrollBarEnabled = false
            layoutParams = LinearLayout.LayoutParams(
                0,
                LinearLayout.LayoutParams.MATCH_PARENT,
                1f,
            )
        }
        candidateExpandButton = TextView(this).apply {
            text = "⌄"
            textSize = 22f
            gravity = Gravity.CENTER
            contentDescription = "展开候选词"
            setTextColor(palette.preedit)
            isEnabled = false
            setOnClickListener {
                candidatesExpanded = !candidatesExpanded
                expandedCandidateBar.visibility = if (candidatesExpanded) View.VISIBLE else View.GONE
                text = if (candidatesExpanded) "⌃" else "⌄"
                contentDescription = if (candidatesExpanded) "收起候选词" else "展开候选词"
            }
        }
        // B9 中/英模式角标：候选行左侧小 chip，accent 色文字 + 圆角浅底描边。
        modeBadge = TextView(this).apply {
            text = "中"
            textSize = 11f
            gravity = Gravity.CENTER
            setTextColor(palette.accent)
            background = GradientDrawable().apply {
                setColor(palette.key)
                cornerRadius = dp(8f).toFloat()
                setStroke(dp(1f), palette.accent)
            }
        }
        candidatePanel.addView(LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            addView(
                modeBadge,
                LinearLayout.LayoutParams(dp(30f), dp(22f)).apply {
                    setMargins(dp(6f), 0, 0, 0)
                },
            )
            addView(scroll)
            addView(
                candidateExpandButton,
                LinearLayout.LayoutParams(dp(46f), candidateBarHeight()),
            )
        }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, candidateBarHeight()))
        expandedCandidateBar = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            setPadding(dp(6f), dp(4f), dp(6f), dp(6f))
        }
        candidatePanel.addView(expandedCandidateBar)
        root.addView(candidatePanel)

        // 功能行：候选不再占用本行，改为承载剪贴板历史 / 图片历史 / 表情入口等功能键。
        val compactFunctionRow = isLandscape()
        val functionRowHeight = dp(if (compactFunctionRow) 30f else 38f)
        val clipButtonSize = dp(if (compactFunctionRow) 24f else 30f)
        val clipVerticalMargin = dp(if (compactFunctionRow) 4f else 5f)
        val functionRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(palette.key)
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, functionRowHeight
            )
        }
        fun functionChip(glyph: String, description: String, onClick: () -> Unit): TextView =
            TextView(this).apply {
                text = glyph
                contentDescription = description
                gravity = Gravity.CENTER
                textSize = 15f
                setTextColor(0xFFFFFFFF.toInt())
                background = GradientDrawable().apply {
                    shape = GradientDrawable.OVAL
                    setColor(palette.accent)
                }
                setOnClickListener { onClick() }
            }
        functionRow.addView(
            functionChip("▾▦", "剪贴板历史") { toggleHistory() },
            LinearLayout.LayoutParams(
                clipButtonSize,
                LinearLayout.LayoutParams.MATCH_PARENT
            ).apply { setMargins(dp(8f), clipVerticalMargin, dp(4f), clipVerticalMargin) }
        )
        functionRow.addView(
            functionChip("🖼", "图片历史（斗图）") { toggleImageHistory() },
            LinearLayout.LayoutParams(
                clipButtonSize,
                LinearLayout.LayoutParams.MATCH_PARENT
            ).apply { setMargins(dp(2f), clipVerticalMargin, dp(6f), clipVerticalMargin) }
        )
        // AI 帮写入口：缺 AGNES_API_KEY 时点击显示「未配置」提示。
        functionRow.addView(
            functionChip("🪄", "AI 帮写") { toggleAiPanel() },
            LinearLayout.LayoutParams(
                clipButtonSize,
                LinearLayout.LayoutParams.MATCH_PARENT
            ).apply { setMargins(dp(2f), clipVerticalMargin, dp(6f), clipVerticalMargin) }
        )
        root.addView(functionRow)
        // 功能行与键区之间的细分隔线，避免浅灰功能行和浅灰键区粘连。
        root.addView(View(this).apply {
            setBackgroundColor(if (isDark()) 0xFF3A3F47.toInt() else 0xFFD8DCE3.toInt())
        }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(1f)))

        keyArea = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        root.addView(
            keyArea,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
            ),
        )
        rebuildKeys()

        // 根视图的实际高度由系统在分屏、自由窗口等场景决定；每次布局后同步键区上限。
        root.viewTreeObserver.addOnGlobalLayoutListener {
            resizeKeyboardForInputView(root)
        }

        historyPanel = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
        }
        root.addView(historyPanel)

        // AI 帮写面板（独立于历史面板）
        aiPanel = buildAiPanel()
        root.addView(
            aiPanel,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.MATCH_PARENT,
            ),
        )

        // 微信输入法 S33 同款：图片预览键盘（点图先进预览，再保存/发送）
        previewKeyboard = buildImagePreviewKeyboard()
        // 必须占满整个输入视图：内部用 weight 布局，wrap_content 会使预览区高度塌缩
        root.addView(
            previewKeyboard,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.MATCH_PARENT,
            ),
        )
        return root
    }

    // ---------- 剪贴板历史面板 ----------

    private fun toggleHistory() {
        val panel = historyPanel ?: return
        if (panel.visibility == View.VISIBLE) {
            panel.visibility = View.GONE
            keyArea.visibility = View.VISIBLE
        } else {
            // 每次打开面板清空搜索框（A6）
            historySearchRunnable?.let { mainHandler.removeCallbacks(it) }
            historySearchBox?.setText("")
            populateHistory(panel, onlyImages = false, query = "")
            panel.visibility = View.VISIBLE
            keyArea.visibility = View.GONE
        }
    }

    /** 斗图面板（微信输入法同款）：只显示图片历史，点图预览后发送。 */
    private fun toggleImageHistory() {
        val panel = historyPanel ?: return
        if (panel.visibility == View.VISIBLE) {
            panel.visibility = View.GONE
            keyArea.visibility = View.VISIBLE
        } else {
            historySearchRunnable?.let { mainHandler.removeCallbacks(it) }
            historySearchBox?.setText("")
            populateHistory(panel, onlyImages = true, query = "")
            panel.visibility = View.VISIBLE
            keyArea.visibility = View.GONE
        }
    }

    // ---------- AI 帮写面板 ----------

    private fun toggleAiPanel() {
        val panel = aiPanel ?: return
        if (panel.visibility == View.VISIBLE) {
            panel.visibility = View.GONE
            keyArea.visibility = View.VISIBLE
        } else {
            // 打开时重置：清空输入框与状态，保留上次草稿预览以便复制
            aiInputBox?.setText("")
            aiStatusLine?.text = if (hasAiApiKey()) "" else "未配置 AGNES_API_KEY；请在 PC 端配置后重试"
            aiDraftView?.text = aiLastDraft.orEmpty()
            aiPasteButton?.isEnabled = !aiLastDraft.isNullOrBlank()
            panel.visibility = View.VISIBLE
            keyArea.visibility = View.GONE
            aiInputBox?.requestFocus()
        }
    }

    private fun hasAiApiKey(): Boolean =
        // Android 的 Process environment 与 Windows 用户环境不共享：
        // 优先 SHURUFA 包内置的 BuildConfig（运行时由 Gradle 属性注入），其次服务端 fallback
        // 由宿主 ShurufaHostApi 暴露；此处只是快速判定，真正的 key 永远只在服务调用时读取。
        try {
            val f = Class.forName("com.shurufa.ime.BuildConfig").getField("AGNES_API_KEY")
            (f.get(null) as? String)?.isNotBlank() == true
        } catch (_: Throwable) {
            false
        }

    private fun buildAiPanel(): LinearLayout {
        val dark = isDark()
        val bg = if (dark) 0xFF23262C.toInt() else 0xFFF7F7F7.toInt()
        val titleColor = if (dark) 0xFFE6E8EB.toInt() else 0xFF333333.toInt()
        val hintColor = if (dark) 0xFF8A8F99.toInt() else 0xFF888888.toInt()
        val inputBg = if (dark) 0xFF2B2F36.toInt() else 0xFFFFFFFF.toInt()
        val inputStroke = if (dark) 0xFF4A5059.toInt() else 0xFFD9D9D9.toInt()

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(bg)
            visibility = View.GONE
            setPadding(dp(8f), dp(8f), dp(8f), dp(8f))
        }

        // 标题行
        val header = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        header.addView(TextView(this).apply {
            text = "🪄 AI 帮写"
            textSize = 16f
            setTypeface(typeface, Typeface.BOLD)
            setTextColor(titleColor)
        }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        header.addView(TextView(this).apply {
            text = "✕"
            contentDescription = "关闭"
            gravity = Gravity.CENTER
            textSize = 18f
            setTextColor(hintColor)
            setPadding(dp(12f), dp(6f), dp(12f), dp(6f))
            setOnClickListener {
                aiPanel?.visibility = View.GONE
                keyArea.visibility = View.VISIBLE
            }
        }, LinearLayout.LayoutParams(dp(44f), dp(40f)))
        root.addView(header)

        // 提示输入框（单行；回车由 onEditorAction 提交）
        val input = EditText(this).apply {
            hint = "想让 AI 写什么？例如「朋友婚礼上台祝福 80 字」"
            textSize = 15f
            setTextColor(titleColor)
            setHintTextColor(hintColor)
            setPadding(dp(10f), 0, dp(10f), 0)
            background = GradientDrawable().apply {
                shape = GradientDrawable.RECTANGLE
                cornerRadius = dp(6f).toFloat()
                setColor(inputBg)
                setStroke(dp(1f), inputStroke)
            }
            imeOptions = EditorInfo.IME_ACTION_SEND
            setOnEditorActionListener { _, actionId, _ ->
                if (actionId == EditorInfo.IME_ACTION_SEND) {
                    requestAiDraft()
                    true
                } else {
                    false
                }
            }
        }
        aiInputBox = input
        root.addView(input, LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT,
            dp(44f),
        ).apply { topMargin = dp(6f) })

        // 状态行：进度 / 错误
        val status = TextView(this).apply {
            textSize = 12f
            setTextColor(hintColor)
            setPadding(0, dp(6f), 0, 0)
        }
        aiStatusLine = status
        root.addView(status)

        // 草稿预览
        val draft = TextView(this).apply {
            textSize = 15f
            setTextColor(titleColor)
            setPadding(dp(10f), dp(8f), dp(10f), dp(8f))
            background = GradientDrawable().apply {
                shape = GradientDrawable.RECTANGLE
                cornerRadius = dp(6f).toFloat()
                setColor(inputBg)
                setStroke(dp(1f), inputStroke)
            }
            setTextIsSelectable(true)
        }
        aiDraftView = draft
        root.addView(
            ScrollView(this).apply { addView(draft) },
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                0,
                1f,
            ).apply { topMargin = dp(6f) },
        )

        // 底部按钮行：粘贴 / 重试
        val actions = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, dp(6f), 0, 0)
        }
        val paste = TextView(this).apply {
            text = "粘贴到输入框"
            textSize = 14f
            gravity = Gravity.CENTER
            setTextColor(0xFFFFFFFF.toInt())
            background = GradientDrawable().apply {
                shape = GradientDrawable.RECTANGLE
                cornerRadius = dp(8f).toFloat()
                setColor(palette.accent)
            }
            isEnabled = false
            setOnClickListener { commitAiDraft() }
        }
        aiPasteButton = paste
        actions.addView(paste, LinearLayout.LayoutParams(0, dp(40f), 1f).apply {
            rightMargin = dp(6f)
        })
        actions.addView(TextView(this).apply {
            text = "重试"
            textSize = 14f
            gravity = Gravity.CENTER
            setTextColor(titleColor)
            background = GradientDrawable().apply {
                shape = GradientDrawable.RECTANGLE
                cornerRadius = dp(8f).toFloat()
                setColor(inputBg)
                setStroke(dp(1f), inputStroke)
            }
            setOnClickListener { requestAiDraft() }
        }, LinearLayout.LayoutParams(0, dp(40f), 1f).apply {
            leftMargin = dp(6f)
        })
        root.addView(actions)

        return root
    }

    /** 真正调用 Agnes API。network 一律走 ioExecutor → 主线程 setText。 */
    private fun requestAiDraft() {
        val prompt = aiInputBox?.text?.toString()?.trim().orEmpty()
        if (prompt.isEmpty()) {
            aiStatusLine?.text = "请先输入提示词"
            return
        }
        // 与 PC 端一致：key 从环境变量读；Android 进程不直读用户环境，
        // 通过 BuildConfig 由打包时 gradle 属性注入（仅 release 本机构建可用）。
        val apiKey = readAiApiKey()
        if (apiKey.isNullOrBlank()) {
            aiStatusLine?.text = "未配置 AGNES_API_KEY"
            return
        }
        aiStatusLine?.text = "思考中…"
        aiDraftView?.text = ""
        aiPasteButton?.isEnabled = false
        ioExecutor.execute {
            val result = callAgnesChat(apiKey, prompt) { partial ->
                // SSE 流式增量：每收到一段就把当前累积的 partial 推到主线程刷新。
                // 不必再渲染节流 —— ioExecutor 单线程执行 + mainHandler.post 不会拥塞。
                mainHandler.post {
                    aiDraftView?.text = partial
                    aiStatusLine?.text = "生成中…（${partial.length} 字）"
                }
            }
            mainHandler.post {
                when (result) {
                    is AiResult.Ok -> {
                        aiLastDraft = result.text
                        aiDraftView?.text = result.text
                        aiStatusLine?.text = "已生成 ${result.text.length} 字；点击「粘贴到输入框」插入"
                        aiPasteButton?.isEnabled = true
                    }
                    is AiResult.Err -> {
                        aiStatusLine?.text = "请求失败：${result.message}"
                        aiPasteButton?.isEnabled = aiLastDraft != null
                    }
                }
            }
        }
    }

    private fun commitAiDraft() {
        val draft = aiLastDraft?.takeIf { it.isNotEmpty() } ?: return
        currentInputConnection?.let { ic ->
            ic.finishComposingText()
            ic.commitText(draft, 1)
            aiPanel?.visibility = View.GONE
            keyArea.visibility = View.VISIBLE
            aiLastDraft = null
            aiDraftView?.text = ""
        }
    }

    sealed class AiResult {
        data class Ok(val text: String) : AiResult()
        data class Err(val message: String) : AiResult()
    }

    private fun readAiApiKey(): String? = try {
        val cls = Class.forName("com.shurufa.ime.BuildConfig")
        (cls.getField("AGNES_API_KEY").get(null) as? String)?.takeIf { it.isNotBlank() }
    } catch (_: Throwable) {
        null
    }

    /**
     * 与 PC 端同一接口：https://apihub.agnes-ai.com/v1/chat/completions
     * `onPartial` 非空则启用 SSE 流式（`"stream": true`），每解析到一段
     * choices[0].delta.content 就回调一次当前累积的部分；否则按整段 JSON 读
     * `choices[0].message.content`。返回的是最终结果；onPartial 只是过程预览。
     */
    private fun callAgnesChat(
        apiKey: String,
        prompt: String,
        onPartial: ((String) -> Unit)? = null,
    ): AiResult {
        val endpoint = java.net.URL("https://apihub.agnes-ai.com/v1/chat/completions")
        var conn: java.net.HttpURLConnection? = null
        return try {
            conn = (endpoint.openConnection() as java.net.HttpURLConnection).apply {
                requestMethod = "POST"
                connectTimeout = 15_000
                readTimeout = 45_000
                doOutput = true
                setRequestProperty("Content-Type", "application/json")
                setRequestProperty("Authorization", "Bearer $apiKey")
            }
            val stream = onPartial != null
            val body = org.json.JSONObject()
                .put("model", "agnes-2.5-flash")
                .put("stream", stream)
                .put("temperature", 0.5)
                .put("messages", org.json.JSONArray()
                    .put(org.json.JSONObject().put("role", "system").put("content",
                        "你是用户输入法里的‘AI 帮写’助手。直接输出可粘贴的中文段落，不要解释、不要 Markdown 代码块；除非用户另有要求，控制在 300 字以内。"))
                    .put(org.json.JSONObject().put("role", "user").put("content", prompt)))
                .toString()
            conn.outputStream.use { os ->
                os.write(body.toByteArray(Charsets.UTF_8))
            }
            val code = conn.responseCode
            if (code !in 200..299) {
                val errText = conn.errorStream
                    ?.bufferedReader(Charsets.UTF_8)?.use { it.readText() }.orEmpty()
                return AiResult.Err("HTTP $code: ${errText.take(120)}")
            }
            if (stream) {
                // SSE：逐行解析 "data: {...}"，[DONE] 终止。
                val acc = StringBuilder()
                conn.inputStream.bufferedReader(Charsets.UTF_8).use { reader ->
                    while (true) {
                        val line = reader.readLine() ?: break
                        if (!line.startsWith("data:")) continue
                        val payload = line.removePrefix("data:").trim()
                        if (payload == "[DONE]") break
                        val delta = try {
                            org.json.JSONObject(payload)
                                .optJSONArray("choices")
                                ?.optJSONObject(0)
                                ?.optJSONObject("delta")
                                ?.optString("content")
                                .orEmpty()
                        } catch (_: Exception) { "" }
                        if (delta.isNotEmpty()) {
                            acc.append(delta)
                            onPartial?.invoke(acc.toString())
                        }
                    }
                }
                val full = acc.toString().trim()
                return if (full.isEmpty()) AiResult.Err("返回内容为空") else AiResult.Ok(full)
            }
            val text = conn.inputStream
                ?.bufferedReader(Charsets.UTF_8)?.use { it.readText() }.orEmpty()
            val root = org.json.JSONObject(text)
            val content = root.optJSONArray("choices")
                ?.optJSONObject(0)
                ?.optJSONObject("message")
                ?.optString("content")
                ?.trim()
                .orEmpty()
            if (content.isEmpty()) AiResult.Err("返回内容为空") else AiResult.Ok(content)
        } catch (e: Exception) {
            AiResult.Err(e.message ?: e.javaClass.simpleName)
        } finally {
            conn?.disconnect()
        }
    }

    // ---------- 图片预览键盘（微信输入法 S33 同款布局） ----------

    /**
     * 微信输入法 S33ImagePreviewKeyboard 的布局范式：
     * 顶部工具栏（关闭 + 标题）→ 中部大图预览（圆角卡片）→ 底部操作区（保存 / 发送）。
     * 样式取自反编译规范：键盘白底浅灰、预览圆角 16dp、主按钮微信绿 #07C160 圆角 22dp。
     */
    private fun buildImagePreviewKeyboard(): LinearLayout {
        val dark = isDark()
        val toolbarBg = if (dark) 0xFF23262C.toInt() else 0xFFF7F7F7.toInt()
        val titleColor = if (dark) 0xFFE6E8EB.toInt() else 0xFF333333.toInt()
        val previewBg = if (dark) 0xFF1A1C20.toInt() else 0xFFF2F3F5.toInt()
        val btnWhite = if (dark) 0xFF2B2F36.toInt() else 0xFFFFFFFF.toInt()
        val btnText = if (dark) 0xFFE6E8EB.toInt() else 0xFF33383F.toInt()
        val btnStroke = if (dark) 0xFF4A5059.toInt() else 0xFFD9D9D9.toInt()
        // 主按钮用我们自己的品牌色（样式自己定，只借微信输入法的布局结构）
        val primaryColor = palette.accent
        val primaryPressed = if (dark) 0xFFC98E4E.toInt() else 0xFF9C5A28.toInt()

        // 顶部工具栏：关闭 ✕ + 居中标题（同 S33：mContentInfoLayout 标题区）
        val toolbar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(toolbarBg)
            setPadding(dp(6f), 0, dp(6f), 0)
        }
        val closeBtn = TextView(this).apply {
            text = "✕"
            contentDescription = "关闭预览"
            gravity = Gravity.CENTER
            textSize = 18f
            setTextColor(titleColor)
            setPadding(dp(12f), dp(8f), dp(12f), dp(8f))
            setOnClickListener { closeImagePreview() }
        }
        toolbar.addView(
            closeBtn,
            LinearLayout.LayoutParams(dp(44f), LinearLayout.LayoutParams.MATCH_PARENT),
        )
        val title = TextView(this).apply {
            text = "图片预览"
            gravity = Gravity.CENTER
            textSize = 15f
            setTextColor(titleColor)
        }
        toolbar.addView(title, LinearLayout.LayoutParams(0, dp(44f), 1f))
        // 右侧占位与关闭对齐（保持标题居中）
        toolbar.addView(View(this), LinearLayout.LayoutParams(dp(44f), LinearLayout.LayoutParams.MATCH_PARENT))

        // 中部大图预览（圆角卡片，同 S33：ImeRadiusConstraintLayout + 图片）
        val previewWrap = LinearLayout(this).apply {
            gravity = Gravity.CENTER
            setBackgroundColor(previewBg)
        }
        val previewImage = ImageView(this).apply {
            tag = "preview_image"
            contentDescription = "图片预览"
            scaleType = ImageView.ScaleType.FIT_CENTER
            background = GradientDrawable().apply {
                setColor(btnWhite)
                cornerRadius = dp(16f).toFloat()
                setStroke(dp(1f), btnStroke)
            }
        }
        previewWrap.addView(
            previewImage,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.MATCH_PARENT,
            ).apply { setMargins(dp(14f), dp(12f), dp(14f), dp(12f)) },
        )

        // 底部操作区：保存到相册（次级）+ 发送（微信绿主按钮）
        val actions = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(previewBg)
            setPadding(dp(16f), dp(12f), dp(16f), dp(16f))
        }
        fun actionBtn(label: String, primary: Boolean): TextView = TextView(this).apply {
            text = label
            gravity = Gravity.CENTER
            textSize = 15f
            if (primary) {
                setTextColor(0xFFFFFFFF.toInt())
                // 按下态反馈：主按钮按下变暗（StateListDrawable）
                background = StateListDrawable().apply {
                    addState(
                        intArrayOf(android.R.attr.state_pressed),
                        GradientDrawable().apply {
                            setColor(primaryPressed)
                            cornerRadius = dp(22f).toFloat()
                        },
                    )
                    addState(
                        intArrayOf(),
                        GradientDrawable().apply {
                            setColor(primaryColor)
                            cornerRadius = dp(22f).toFloat()
                        },
                    )
                }
            } else {
                setTextColor(btnText)
                background = StateListDrawable().apply {
                    addState(
                        intArrayOf(android.R.attr.state_pressed),
                        GradientDrawable().apply {
                            setColor(if (dark) 0xFF3A4048.toInt() else 0xFFEDEFF2.toInt())
                            cornerRadius = dp(22f).toFloat()
                            setStroke(dp(1f), btnStroke)
                        },
                    )
                    addState(
                        intArrayOf(),
                        GradientDrawable().apply {
                            setColor(btnWhite)
                            cornerRadius = dp(22f).toFloat()
                            setStroke(dp(1f), btnStroke)
                        },
                    )
                }
            }
        }
        val saveBtn = actionBtn("保存到相册", false).apply {
            setOnClickListener {
                val id = previewImageId ?: return@setOnClickListener
                // 相册写入是系统 ContentResolver IPC（MediaStore），大 PNG 会阻塞主线程
                // 200ms+，从 IME 点击处切到 IO 线程；结果回主线程弹 Toast。
                isEnabled = false
                text = "保存中…"
                ioExecutor.execute {
                    val png = try {
                        ClipStore.imageData(id)
                    } catch (e: Throwable) {
                        null
                    }
                    val uri = png?.let {
                        try {
                            ImageClipboard.saveToGallery(applicationContext, it)
                        } catch (e: Throwable) {
                            null
                        }
                    }
                    mainHandler.post {
                        isEnabled = true
                        text = "保存到相册"
                        if (png == null) {
                            showAttachmentError("图片数据缺失")
                        } else if (uri != null) {
                            android.util.Log.i("shurufa", "预览保存到相册 历史ID=$id URI=$uri")
                            Toast.makeText(
                                this@ShurufaImeService,
                                "已保存到相册（Shurufa 文件夹）",
                                Toast.LENGTH_SHORT,
                            ).show()
                        } else {
                            showAttachmentError("保存到相册失败")
                        }
                    }
                }
            }
        }
        val sendBtn = actionBtn("发 送", true).apply {
            // 点击反馈：立即变「发送中…」禁用，完成后结果 toast（用户可感知全过程）
            setOnClickListener {
                val id = previewImageId ?: return@setOnClickListener
                isEnabled = false
                text = "发送中…"
                Toast.makeText(
                    this@ShurufaImeService,
                    "正在发送图片…",
                    Toast.LENGTH_SHORT,
                ).show()
                // commitImage 内部会查 ClipStore 与 FileProvider，大文件时阻塞主线程；
                // 切到 IO 线程，结果回主线程恢复按钮与提示。
                ioExecutor.execute {
                    val result = try {
                        commitImage(id)
                    } catch (e: Throwable) {
                        android.util.Log.e("shurufa", "发送图片异常", e)
                        SendResult.FAILED
                    }
                    mainHandler.post {
                        isEnabled = true
                        text = "发 送"
                        when (result) {
                            SendResult.SENT -> {
                                Toast.makeText(
                                    this@ShurufaImeService,
                                    "已发送到输入框，请点 App 的发送键",
                                    Toast.LENGTH_SHORT,
                                ).show()
                                closeImagePreview()
                            }
                            SendResult.COPIED -> {
                                // copyImageToClipboard 内部已 toast 明确结果（粘贴失败→存相册）
                                closeImagePreview()
                            }
                            SendResult.FAILED -> showAttachmentError("图片发送失败")
                        }
                    }
                }
            }
        }
        val gap = View(this)
        actions.addView(saveBtn, LinearLayout.LayoutParams(0, dp(44f), 1f))
        actions.addView(gap, LinearLayout.LayoutParams(dp(12f), LinearLayout.LayoutParams.MATCH_PARENT))
        actions.addView(sendBtn, LinearLayout.LayoutParams(0, dp(44f), 1f))

        return LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            setBackgroundColor(previewBg)
            addView(toolbar, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(44f)))
            addView(previewWrap, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
            addView(actions, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
        }
    }

    /** 打开图片预览键盘（微信输入法：点图先进预览，再点发送/保存）。 */
    private fun openImagePreview(id: Int) {
        val panel = previewKeyboard ?: return
        val image = panel.findViewWithTag<ImageView>("preview_image") ?: return
        val png = try {
            ClipStore.imageData(id)
        } catch (e: Throwable) {
            null
        }
        if (png == null) {
            showAttachmentError("图片数据缺失")
            return
        }
        val bmp = decodeSampledBitmap(png, PREVIEW_TARGET)
        if (bmp != null) image.setImageBitmap(bmp)
        previewImageId = id
        // 隐藏一切可能挤压预览区的元素。
        historyPanel?.visibility = View.GONE
        keyArea.visibility = View.GONE
        panel.visibility = View.VISIBLE
        android.util.Log.i("shurufa", "打开图片预览 历史ID=$id 字节=${png.size}")
    }

    private fun closeImagePreview() {
        previewKeyboard?.visibility = View.GONE
        previewImageId = null
        keyArea.visibility = View.VISIBLE
    }

    private fun populateHistory(panel: LinearLayout, onlyImages: Boolean = false, query: String = "") {
        panel.removeAllViews()
        // 标题行：左侧标题，右侧打字统计小字（B12）
        val titleRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(14f), dp(8f), dp(14f), dp(8f))
        }
        titleRow.addView(TextView(this).apply {
            text = if (onlyImages) "斗图 · 点图片预览 · 再点 ▾▦ 返回" else "剪贴板历史 · 点击上屏 · 再点 ▾▦ 返回"
            textSize = 12f
            setTextColor(palette.preedit)
        }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        val statsView = TextView(this).apply {
            textSize = 11f
            setTextColor(palette.preedit)
        }
        titleRow.addView(statsView, LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.WRAP_CONTENT,
            LinearLayout.LayoutParams.WRAP_CONTENT,
        ))
        panel.addView(titleRow)
        refreshTypingStats(statsView)

        // A6 搜索框：仅文本模式（非斗图）显示；空串走 list，非空走 search。
        historySearchBox = null
        if (!onlyImages) {
            val searchBox = EditText(this).apply {
                hint = "搜索历史…"
                textSize = 14f
                setTextColor(palette.keyText)
                setHintTextColor(palette.preedit)
                setSingleLine(true)
                setPadding(dp(14f), dp(10f), dp(14f), dp(10f))
                setText(query)
                addTextChangedListener(object : TextWatcher {
                    override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
                    override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
                    override fun afterTextChanged(s: Editable?) {
                        val q = s?.toString().orEmpty()
                        historySearchRunnable?.let { mainHandler.removeCallbacks(it) }
                        val task = Runnable {
                            val target = historyPanel ?: return@Runnable
                            populateHistory(target, onlyImages = false, query = q)
                        }
                        historySearchRunnable = task
                        mainHandler.postDelayed(task, 250L)
                    }
                })
            }
            historySearchBox = searchBox
            if (query.isNotEmpty()) {
                // 搜索回填时保持光标在末尾，避免再次触发无意义的文本变化回调
                searchBox.setSelection(query.length)
            }
            panel.addView(searchBox)
        }

        // 占位：SQLite 读取与图片缩略解码都在 IO 线程；期间用户可见"加载中"
        val placeholder = TextView(this).apply {
            text = "加载中…"
            textSize = 13f
            setTextColor(palette.preedit)
            setPadding(dp(14f), dp(20f), dp(14f), dp(20f))
        }
        panel.addView(placeholder)
        // 快照主线程读到的 MIME（currentSupportedImageMimeType 需要 currentInput*）
        val mimeSnapshot = currentSupportedImageMimeType()
        ioExecutor.execute {
            val entries = try {
                if (query.isBlank()) {
                    ClipStore.list(30)
                } else {
                    ClipStore.search(query, 30)
                }.filter { !onlyImages || it.kind == "image" }
            } catch (e: Throwable) {
                emptyList()
            }
            // 图片条目预解码缩略图，主线程只做 setImageBitmap，不再做 PNG 解码
            val prepared = entries.map { entry ->
                val thumb = if (entry.kind == "image") {
                    try {
                        ClipStore.imageData(entry.id)?.let { bytes ->
                            decodeSampledBitmap(bytes, THUMBNAIL_TARGET)
                        }
                    } catch (e: Throwable) {
                        null
                    }
                } else {
                    null
                }
                PreparedHistory(entry, thumb)
            }
            mainHandler.post {
                // 列表已更新（populateHistory 又被触发）则放弃本次结果
                if (placeholder.parent !== panel) return@post
                panel.removeView(placeholder)
                if (prepared.isEmpty()) {
                    panel.addView(TextView(this).apply {
                        text = if (query.isBlank()) "（暂无历史）" else "（没有匹配「$query」的历史）"
                        setTextColor(palette.preedit)
                        setPadding(dp(14f), dp(20f), dp(14f), dp(20f))
                    })
                    return@post
                }
                renderHistoryList(panel, prepared, onlyImages, mimeSnapshot)
            }
        }
    }

    /**
     * B12 打字统计展示：解析 nativeStatsTotals（"totalChars FIELD todayChars FIELD …"），
     * 空串/解析失败则置空不占位。
     */
    private fun refreshTypingStats(target: TextView) {
        if (!engineReady) {
            target.text = ""
            return
        }
        val raw = try {
            RimeBridge.nativeStatsTotals()
        } catch (e: Throwable) {
            ""
        }
        if (raw.isEmpty()) {
            target.text = ""
            return
        }
        val parts = raw.split("\u0001")
        val totalChars = parts.getOrNull(0)?.toLongOrNull()
        val todayChars = parts.getOrNull(1)?.toLongOrNull()
        target.text = if (totalChars == null || todayChars == null) {
            ""
        } else {
            "今日 $todayChars 字 · 累计 $totalChars 字"
        }
    }

    /** IO 线程预取后回主线程渲染：entry + 已解码缩略图。 */
    private data class PreparedHistory(
        val entry: ClipStore.Entry,
        val thumb: android.graphics.Bitmap?,
    )

    /** A6 长按菜单：置顶/取消置顶、删除。置顶靠 SQL 层排序体现，不做角标。 */
    private fun showHistoryEntryActions(entry: ClipStore.Entry, onlyImages: Boolean) {
        // 无 pin 字段回传：当前置顶状态不明，菜单同时给出「置顶」与「取消置顶」。
        val options = arrayOf("置顶", "取消置顶", "删除")
        android.app.AlertDialog.Builder(this)
            .setTitle(entry.text.replace('\n', ' ').take(24))
            .setItems(options) { dialog, which ->
                when (which) {
                    0 -> ClipStore.setPinned(entry.id, true)
                    1 -> ClipStore.setPinned(entry.id, false)
                    2 -> ClipStore.delete(entry.id)
                }
                dialog.dismiss()
                historyPanel?.let { populateHistory(it, onlyImages) }
            }
            .show()
    }

    private fun renderHistoryList(
        panel: LinearLayout,
        prepared: List<PreparedHistory>,
        onlyImages: Boolean,
        mimeSnapshot: String?,
    ) {
        val list = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        for (item in prepared) {
            val entry = item.entry
            if (entry.kind == "image") {
                android.util.Log.i(
                    "shurufa",
                    "图片历史条目状态 历史ID=${entry.id} 声明MIME=$mimeSnapshot",
                )
                val thumb = ImageView(this).apply {
                    contentDescription = "图片附件：${entry.source}，点击预览后发送到当前输入框"
                    if (item.thumb != null) setImageBitmap(item.thumb)
                    adjustViewBounds = true
                    maxHeight = dp(130f)
                    scaleType = ImageView.ScaleType.FIT_START
                    background = keyBackground(palette.key, palette.keyPressed)
                    alpha = 1f
                    setPadding(dp(10f), dp(8f), dp(10f), dp(8f))
                    // 微信输入法同款：点图先进预览键盘，再保存/发送
                    setOnClickListener { openImagePreview(entry.id) }
                    setOnLongClickListener {
                        showHistoryEntryActions(entry, onlyImages)
                        true
                    }
                }
                list.addView(thumb, LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
                ).apply { setMargins(dp(4f), dp(3f), dp(4f), dp(3f)) })
                continue
            }
            if (entry.kind == "files") {
                list.addView(TextView(this).apply {
                    val fileName = entry.text.lineSequence().firstOrNull()?.let { File(it).name } ?: "文件"
                    text = fileName
                    contentDescription = "文件附件：$fileName"
                    textSize = 16f
                    setTextColor(palette.keyText)
                    background = keyBackground(palette.key, palette.keyPressed)
                    alpha = 1f
                    setPadding(dp(14f), dp(13f), dp(14f), dp(13f))
                    setOnClickListener {
                        when (commitFile(entry.id)) {
                            SendResult.SENT -> toggleHistory()
                            SendResult.COPIED -> {
                                Toast.makeText(
                                    this@ShurufaImeService,
                                    "文件已复制，长按输入框粘贴",
                                    Toast.LENGTH_SHORT,
                                ).show()
                                toggleHistory()
                            }
                            SendResult.FAILED -> showAttachmentError("文件发送失败")
                        }
                    }
                    setOnLongClickListener {
                        showHistoryEntryActions(entry, onlyImages)
                        true
                    }
                }, LinearLayout.LayoutParams(
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
                setOnLongClickListener {
                    showHistoryEntryActions(entry, onlyImages)
                    true
                }
            }, LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT
            ).apply { setMargins(dp(4f), dp(3f), dp(4f), dp(3f)) })
        }
        panel.addView(ScrollView(this).apply {
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f
            )
            addView(list)
        })
    }

    /// 图片以附件形式提交给当前输入框：无条件尝试 Commit Content 原位插入，失败回退剪贴板。
    /// 不做 ACTION_SEND 分享：分享语义是“分享到应用”而非“插入输入框”，
    /// 对抖音/B 站等会误触打开发布/分享界面（实机验证结论）。
    /// 不检查 MIME 声明：抖音/B 站等评论框声明为空但实现了 OnReceiveContentListener，
    /// 直接 commitContent 依然能成功插入（微信输入法的通用做法）。
    private fun commitImage(id: Int): SendResult {
        val png = try {
            ClipStore.imageData(id)
        } catch (e: Throwable) {
            null
        } ?: return SendResult.FAILED
        // 第 1 级：Commit Content 原位插入（按目标声明转码；无声明用原图 PNG）
        val targetMimeType = currentSupportedImageMimeType() ?: "image/png"
        try {
            val encoded = encodeImageForTarget(png, targetMimeType)
            android.util.Log.i(
                "shurufa",
                "准备发送图片 历史ID=$id 原始字节=${png.size} 类型=$targetMimeType 字节=${encoded.size}",
            )
            val dir = File(cacheDir, "shared").apply { mkdirs() }
            val f = File(dir, "clip_$id.${imageExtension(targetMimeType)}")
            f.writeBytes(encoded)
            if (commitAttachment(f, targetMimeType)) return SendResult.SENT
        } catch (e: Throwable) {
            android.util.Log.e("shurufa", "图片准备上屏失败", e)
        }
        // 第 2 级：原位提交被拒 → 复制到系统剪贴板，长按粘贴或从相册发送
        return copyImageToClipboard(png, id)
    }

    /** 文件以附件形式提交给当前输入框：无条件尝试 commitContent，失败回退剪贴板。 */
    private fun commitFile(id: Int): SendResult {
        val entry = ClipStore.list(100).firstOrNull { it.id == id && it.kind == "files" } ?: return SendResult.FAILED
        val path = entry.text.lineSequence().firstOrNull()?.takeIf { it.isNotBlank() } ?: return SendResult.FAILED
        val file = File(path)
        if (!file.isFile) {
            showAttachmentError("文件不存在")
            return SendResult.FAILED
        }
        val mimeType = URLConnection.guessContentTypeFromName(file.name) ?: "application/octet-stream"
        if (commitAttachment(file, mimeType)) {
            return SendResult.SENT
        }
        return copyFileToClipboard(file, mimeType, id)
    }

    /**
     * 通过 Android Commit Content 标准协议提交，不依赖目标 MIME 声明：
     * 声明只是“接受哪些类型”的广告，真正决定成败的是目标是否实现
     * OnReceiveContentListener / onCommitContent（微信输入法的通用做法）。
     */
    private fun commitAttachment(file: File, mimeType: String): Boolean {
        val ic = currentInputConnection ?: return false
        val editor = currentInputEditorInfo ?: return false
        return try {
            val uri = FileProvider.getUriForFile(this, "$packageName.fileprovider", file)
            val declaredMimeTypes = EditorInfoCompat.getContentMimeTypes(editor)
            val supported = AttachmentDeliveryPolicy.supportsMimeType(mimeType, declaredMimeTypes)
            val targetPackage = editor.packageName
            val desc = ClipDescription(file.name, arrayOf(mimeType))
            val content = InputContentInfoCompat(uri, desc, null)
            val flags = if (Build.VERSION.SDK_INT >= 25) {
                InputConnectionCompat.INPUT_CONTENT_GRANT_READ_URI_PERMISSION
            } else {
                grantAttachmentUri(targetPackage, uri)
                0
            }
            val commitAccepted = InputConnectionCompat.commitContent(
                ic, editor, content, flags, null,
            )
            android.util.Log.i(
                "shurufa",
                "附件富内容投递 名称=${file.name} 字节=${file.length()} 类型=$mimeType " +
                    "目标=$targetPackage 声明=${declaredMimeTypes.joinToString()} 匹配=$supported " +
                    "授权标志=$flags URI=$uri 接收=$commitAccepted",
            )
            if (!commitAccepted) {
                android.util.Log.i("shurufa", "目标拒绝原位提交，准备回退到剪贴板 名称=${file.name}")
            } else {
                // 微信输入法同款行为：commitContent 成功后不自动触发发送，
                // 图片已在输入框内，由用户在 App 的发送键发出。
            }
            commitAccepted
        } catch (e: Throwable) {
            android.util.Log.e("shurufa", "附件上屏失败", e)
            showAttachmentError("附件发送失败")
            false
        }
    }

    /**
     * 微信输入法同款“点击图片直接进输入框”机制（反编译 com.tencent.wetype 实锤）：
     * 图片已写入系统剪贴板后，向目标输入框触发「粘贴」：
     * InputConnection.performContextMenuAction(android.R.id.paste=16908322)
     * —— 微信输入法 S33ImagePreviewKeyboard.h() 对非微信生态 App 的唯一动作
     * （WxHldService.performContextMenuAction → q0 协程 → 标准 InputConnection 调用，
     * 无 performEditorAction / 无回车 / 无键事件）。
     *
     * 支持粘贴图片的输入框（豆包、微信、抖音私信等）会直接从剪贴板取图插入，
     * 之后由用户在 App 的发送键发出；不支持的输入框（抖音/B站评论框）返回 false，
     * 提示保存后从相册发送（微信输入法在 B≥2 时提示同样文案）。
     */
    private fun triggerPasteInTarget(): Boolean {
        return try {
            val ic = currentInputConnection ?: return false
            val accepted = ic.performContextMenuAction(android.R.id.paste)
            android.util.Log.i("shurufa", "触发自动粘贴（微信输入法同款）performContextMenuAction(PASTE)=$accepted")
            accepted
        } catch (e: Throwable) {
            android.util.Log.e("shurufa", "触发自动粘贴失败", e)
            false
        }
    }

    /** Android 7.0 及以下没有提交授权标志，需要显式授权目标包。 */
    private fun grantAttachmentUri(targetPackage: String?, uri: Uri): Boolean {
        if (targetPackage.isNullOrBlank()) return false
        return try {
            grantUriPermission(targetPackage, uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            true
        } catch (e: Throwable) {
            android.util.Log.w("shurufa", "附件 URI 显式授权失败 目标=$targetPackage URI=$uri", e)
            false
        }
    }

    private fun currentSupportedImageMimeType(): String? {
        val editor = currentInputEditorInfo ?: return null
        if (currentInputConnection == null || editor.packageName.isNullOrBlank()) return null
        return AttachmentDeliveryPolicy.selectImageMimeType(EditorInfoCompat.getContentMimeTypes(editor))
    }

    private fun currentSupportsMimeType(mimeType: String): Boolean {
        val editor = currentInputEditorInfo ?: return false
        if (currentInputConnection == null || editor.packageName.isNullOrBlank()) return false
        return AttachmentDeliveryPolicy.supportsMimeType(
            mimeType,
            EditorInfoCompat.getContentMimeTypes(editor),
        )
    }

    /// 第 2 级兜底：图片复制到系统剪贴板（URI + image/png），随后自动触发粘贴+发送。
    private fun copyImageToClipboard(png: ByteArray, id: Int): SendResult {
        return try {
            val uri = ImageClipboard.setClipboard(this, png, "图片")
            ImageClipboard.grantTo(this, uri, currentInputEditorInfo?.packageName)
            android.util.Log.i(
                "shurufa",
                "图片已复制到剪贴板 历史ID=$id 字节=${png.size} URI=$uri 目标=${currentInputEditorInfo?.packageName}",
            )
            // 微信输入法同款：自动触发粘贴（performContextMenuAction PASTE），
            // 支持粘贴图片的输入框会直接从剪贴板取图插入，无需用户长按。
            if (!triggerPasteInTarget()) {
                // 输入框不吃粘贴：保存到相册，用户从「+」→ 相册 → Shurufa 文件夹发送
                val galleryUri = ImageClipboard.saveToGallery(this, png)
                if (galleryUri != null) {
                    android.util.Log.i("shurufa", "已保存到相册 Pictures/Shurufa URI=$galleryUri")
                    showAttachmentError("已保存到相册（Shurufa 文件夹），请从「+」发送")
                } else {
                    showAttachmentError("该输入框不支持发送图片，已复制到剪贴板，请长按粘贴")
                }
            }
            SendResult.COPIED
        } catch (e: Throwable) {
            android.util.Log.e("shurufa", "复制图片到剪贴板失败", e)
            SendResult.FAILED
        }
    }

    /// 第 2 级兜底：文件复制到系统剪贴板（URI + MIME），随后自动触发粘贴+发送。
    private fun copyFileToClipboard(file: File, mimeType: String, id: Int): SendResult {
        return try {
            val uri = FileProvider.getUriForFile(this, "$packageName.fileprovider", file)
            val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            val description = ClipDescription(ImageClipboard.LABEL_PREFIX + "文件", arrayOf(mimeType))
            clipboard.setPrimaryClip(ClipData(description, ClipData.Item(uri)))
            ImageClipboard.grantTo(this, uri, currentInputEditorInfo?.packageName)
            android.util.Log.i(
                "shurufa",
                "文件已复制到剪贴板 历史ID=$id 名称=${file.name} 类型=$mimeType URI=$uri",
            )
            triggerPasteInTarget()
            SendResult.COPIED
        } catch (e: Throwable) {
            android.util.Log.e("shurufa", "复制文件到剪贴板失败", e)
            SendResult.FAILED
        }
    }

    private fun encodeImageForTarget(png: ByteArray, mimeType: String): ByteArray {
        if (mimeType == "image/png") return png
        val bitmap = BitmapFactory.decodeByteArray(png, 0, png.size)
            ?: error("同步图片无法解码")
        return try {
            ByteArrayOutputStream().use { output ->
                val format = when (mimeType) {
                    "image/jpeg" -> android.graphics.Bitmap.CompressFormat.JPEG
                    "image/webp" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                        android.graphics.Bitmap.CompressFormat.WEBP_LOSSLESS
                    } else {
                        @Suppress("DEPRECATION")
                        android.graphics.Bitmap.CompressFormat.WEBP
                    }
                    else -> error("不支持的目标图片类型：$mimeType")
                }
                check(bitmap.compress(format, 95, output)) { "图片转码失败：$mimeType" }
                output.toByteArray()
            }
        } finally {
            bitmap.recycle()
        }
    }

    private fun imageExtension(mimeType: String): String = when (mimeType) {
        "image/jpeg" -> "jpg"
        "image/webp" -> "webp"
        else -> "png"
    }

    private fun logUnsupportedAttachment(name: String) {
        val editor = currentInputEditorInfo
        val declared = editor?.let { EditorInfoCompat.getContentMimeTypes(it).joinToString() }.orEmpty()
        android.util.Log.i(
            "shurufa",
            "目标不支持附件 名称=$name 目标=${editor?.packageName} 声明=$declared",
        )
    }

    private fun showAttachmentError(message: String) {
        Toast.makeText(this, message, Toast.LENGTH_SHORT).show()
    }

    // ---------- 按键布局 ----------

    private fun rebuildKeys() {
        if (!::keyArea.isInitialized) return
        keyArea.removeAllViews()
        if (symbolMode) buildSymbolPage() else buildLetterPage()
        inputRoot?.let { root -> root.post { resizeKeyboardForInputView(root) } }
    }

    private fun resizeKeyboardForInputView(root: LinearLayout) {
        if (!::keyArea.isInitialized || root.height <= 0) return
        val keyboard = keyArea.getChildAt(0) as? WetypeKeyboardView ?: return
        val keyIndex = root.indexOfChild(keyArea)
        if (keyIndex < 0) return
        val chromeHeight = root.paddingTop + root.paddingBottom + (0 until keyIndex).sumOf { index ->
            val child = root.getChildAt(index)
            if (child.visibility == View.GONE) {
                0
            } else {
                val params = child.layoutParams as? ViewGroup.MarginLayoutParams
                child.measuredHeight + (params?.topMargin ?: 0) + (params?.bottomMargin ?: 0)
            }
        }
        val keyspace = root.height - chromeHeight

        // 仅当确有余量时才压缩键区；root 尚未撑开（keyspace<=0）时保留键盘默认高度，
        // 由屏幕比例撑起键盘，避免一次性把可用高度压成 0 死锁。
        if (keyspace > 0) keyboard.setAvailableKeyboardHeight(keyspace)
    }

    private fun buildLetterPage() {
        // 微信输入法 S2 键盘整块布局（反编译 JSON 直接渲染）
        val asciiMode = engineReady && RimeBridge.nativeIsAscii()
        keyArea.addView(
            WetypeKeyboardView(
                this,
                KeyboardLayoutSpec.Page.LETTERS,
                isDark(),
                palette,
                asciiMode,
                asciiMode || shiftMode,
                onAction = { onWetypeAction(it) },
            ),
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT),
        )
    }

    private fun buildSymbolPage() {
        // 微信输入法 S13 数字符号页整块布局（反编译 JSON 直接渲染）
        keyArea.addView(
            WetypeKeyboardView(
                this,
                KeyboardLayoutSpec.Page.SYMBOLS,
                isDark(),
                palette,
                false,
                false,
                onAction = { onWetypeAction(it) },
            ),
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT),
        )
    }

    private fun onWetypeAction(a: WetypeKeyboardView.WetypeAction) {
        when (a) {
            is WetypeKeyboardView.WetypeAction.Char -> {
                val c = a.value
                if (c.length == 1 && c[0].isLetter()) {
                    onLetter(c[0].lowercaseChar())
                } else {
                    currentInputConnection?.commitText(c, 1)
                }
            }
            WetypeKeyboardView.WetypeAction.Backspace -> onBackspace()
            WetypeKeyboardView.WetypeAction.Shift -> {
                shiftMode = !shiftMode
                rebuildKeys()
            }
            WetypeKeyboardView.WetypeAction.NumberPage -> {
                symbolMode = true
                rebuildKeys()
            }
            WetypeKeyboardView.WetypeAction.BackPage -> {
                symbolMode = false
                rebuildKeys()
            }
            WetypeKeyboardView.WetypeAction.Enter -> onEnter()
            WetypeKeyboardView.WetypeAction.Space -> onSpace()
            WetypeKeyboardView.WetypeAction.VoiceStart -> onVoiceStart()
            WetypeKeyboardView.WetypeAction.VoiceCancel -> {
                voice?.cancel()
                // 上滑取消：删掉已实时上屏的预览文字（微信式撤销）。
                currentInputConnection?.setComposingText("", 1)
                voiceStatusBar?.visibility = View.GONE
            }
            WetypeKeyboardView.WetypeAction.VoiceEnd -> {
                voice?.end()
                voiceStatusBar?.visibility = View.GONE
            }
            WetypeKeyboardView.WetypeAction.Lang -> onToggleLang()
            WetypeKeyboardView.WetypeAction.Clear -> {
                if (engineReady) RimeBridge.nativeReset()
                currentInputConnection?.apply {
                    // 真正清空输入框：删除光标前后全部内容（上滑清空不能只提示不干活）。
                    val before = getTextBeforeCursor(Int.MAX_VALUE, 0)?.length ?: 0
                    val after = getTextAfterCursor(Int.MAX_VALUE, 0)?.length ?: 0
                    deleteSurroundingText(before, after)
                    finishComposingText()
                }
                updateCandidates("", emptyList(), 0)
            }
            WetypeKeyboardView.WetypeAction.BackspaceStart -> showStatus("🔍 长按删除中 — 上滑清空", 0xFF4A5059.toInt())
            WetypeKeyboardView.WetypeAction.BackspaceClear -> showStatus("✓ 已清空", 0xFF2F6B4B.toInt())
            WetypeKeyboardView.WetypeAction.BackspaceEnd -> voiceStatusBar?.visibility = View.GONE
        }
    }

    /** 统一的顶部状态条（语音 / 删除反馈共用），短提示自动消失。 */
    private fun showStatus(text: String, color: Int) {
        val bar = voiceStatusBar ?: return
        bar.removeCallbacks(barRunnable)
        bar.text = text
        bar.setBackgroundColor(color)
        bar.visibility = View.VISIBLE
        bar.postDelayed(barRunnable, 1400L)
    }

    private val barRunnable = Runnable {
        voiceStatusBar?.visibility = View.GONE
    }
    private fun langLabel(): String =
        if (engineReady && RimeBridge.nativeIsAscii()) "英" else "中"

    private fun onToggleLang() {
        if (!engineReady) return
        RimeBridge.nativeToggleAscii()
        RimeBridge.nativeReset()
        if (::keyArea.isInitialized && !symbolMode) rebuildKeys() // 刷新中英键帽
        sync()
        updateModeBadge()
    }

    // ---------- 输入处理 ----------

    private fun onLetter(c: Char) {
        // 大写态：直接输出大写字母（微信输入法 capslock 行为）
        if (shiftMode && c.isLetter()) {
            currentInputConnection?.commitText(c.uppercaseChar().toString(), 1)
            shiftMode = false
            rebuildKeys()
            return
        }
        if (!engineReady) {
            enqueueDuringWarmup(PendingInput.Letter(c))
            return
        }
        val eaten = RimeBridge.nativeProcessKey(c.code, 0)
        if (!eaten) currentInputConnection?.commitText(c.toString(), 1)
        sync()
    }

    private fun onBackspace() {
        if (!engineReady) {
            enqueueDuringWarmup(PendingInput.Backspace)
            return
        }
        val eaten = engineReady && RimeBridge.nativeProcessKey(XK_BACKSPACE, 0)
        if (!eaten) currentInputConnection?.deleteSurroundingText(1, 0)
        sync()
    }

    private fun onSpace() {
        if (!engineReady) {
            enqueueDuringWarmup(PendingInput.Space)
            return
        }
        val eaten = engineReady && RimeBridge.nativeProcessKey(0x20, 0)
        if (!eaten) currentInputConnection?.commitText(" ", 1)
        sync()
    }

    private fun onEnter() {
        if (!engineReady) {
            enqueueDuringWarmup(PendingInput.Enter)
            return
        }
        val eaten = engineReady && RimeBridge.nativeProcessKey(XK_RETURN, 0)
        if (!eaten) {
            currentInputConnection?.apply {
                sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER))
                sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_ENTER))
            }
        }
        sync()
    }

    private fun onVoiceStart() {
        // IME 中无法直接弹系统权限框，VoiceInputController 借透明 Activity 请求。
        voiceStatusBar?.apply {
            text = "🎤 麦克风…请允许录音"
            visibility = View.VISIBLE
        }
        voice?.startOrRequestPermission()
    }

    /** 语音状态条反馈：1=聆听中，0=结束/空闲，2=已取消。 */
    private fun updateVoiceStatus(status: Int) {
        val bar = voiceStatusBar ?: return
        when (status) {
            1 -> {
                bar.text = "🎤 麦克风聆听中…松开结束 / 上滑取消"
                bar.visibility = View.VISIBLE
            }
            0 -> bar.postDelayed({ bar.visibility = View.GONE }, if (bar.text.contains("聆")) 0L else 1200L)
            else -> {
                bar.text = "已取消"
                bar.setBackgroundColor(0xFF888888.toInt())
                bar.postDelayed({ bar.visibility = View.GONE }, 800L)
            }
        }
    }

    /** 微信式实时预览：识别过程中的 partial 结果用 composition 上屏，可被后续结果替换。 */
    private fun updateVoicePreview(text: String) {
        if (text.isEmpty()) {
            currentInputConnection?.finishComposingText()
            return
        }
        currentInputConnection?.setComposingText(text, 1)
    }

    /**
     * 语音结果上屏。
     * @param showBar 是否在顶部状态条短暂展示结果反馈（轮转固化时静默，松手时显示）。
     */
    private fun commitVoiceResult(text: String, showBar: Boolean) {
        if (text.isEmpty()) return
        // composition 已实时显示本段文字，这里固化并清空引擎现场避免混排。
        if (engineReady) RimeBridge.nativeReset()
        currentInputConnection?.commitText(text, 1)
        sync()
        if (!showBar) return
        voiceStatusBar?.visibility = View.GONE
        // 短暂展示识别结果作为最终反馈。
        val bar = voiceStatusBar
        if (bar != null) {
            bar.text = "🎤 $text"
            bar.visibility = View.VISIBLE
            bar.postDelayed({ bar.visibility = View.GONE }, 1500L)
        }
    }

    private fun onPunct(s: String) {
        if (!engineReady) {
            enqueueDuringWarmup(PendingInput.Punct(s))
            return
        }
        if (engineReady && RimeBridge.nativeContext().isNotEmpty()) {
            RimeBridge.nativeProcessKey(0x20, 0)
            sync()
        }
        currentInputConnection?.commitText(s, 1)
    }

    /** 直接选中当前页第 index 个候选（Rime select API），不走数字键路径。 */
    private fun onCandidateSelect(index: Int) {
        if (!engineReady) return
        val committed = RimeBridge.nativeSelectCandidate(index)
        val ic = currentInputConnection ?: return
        if (committed.isNotEmpty()) {
            ic.commitText(committed, 1)
        }
        sync()
    }

    private fun onCandidatePage(previous: Boolean) {
        if (!engineReady) return
        RimeBridge.nativeChangePage(previous)
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
        val parts = raw.split("\u0001")
        val preedit = parts[0]
        val highlighted = parts.getOrNull(1)?.toIntOrNull() ?: 0
        val engineCursor = parts.getOrNull(2)?.toIntOrNull() ?: preedit.length
        val cursorPos = compositionCursorOverride
            ?.takeIf { it in 0..preedit.length }
            ?: engineCursor.also { compositionCursorOverride = null }
        val candidates = if (parts.size > 3) parts.subList(3, parts.size) else emptyList()
        ic.setComposingText(preedit, cursorPos)
        updateCandidates(preedit, candidates, highlighted)
        updateModeBadge()
    }

    private fun updateCandidates(preedit: String, candidates: List<String>, highlighted: Int) {
        if (!::candidateBar.isInitialized) return
        candidateBar.removeAllViews()
        expandedCandidateBar.removeAllViews()
        candidateExpandButton.isEnabled = candidates.isNotEmpty()
        if (candidates.isEmpty()) {
            candidatesExpanded = false
        }
        candidateExpandButton.text = if (candidatesExpanded) "⌃" else "⌄"
        candidateExpandButton.contentDescription = if (candidatesExpanded) "收起候选词" else "展开候选词"
        expandedCandidateBar.visibility = if (candidatesExpanded) View.VISIBLE else View.GONE
        // 拼音预编辑由系统输入框承载（setComposingText），候选行只渲染候选词，
        // 不再把拼音当作候选显示；仅在“有输入但无匹配”时给出一行轻提示。
        if (candidates.isEmpty() && preedit.isNotEmpty()) {
            candidateBar.addView(
                TextView(this).apply {
                    text = "未匹配到候选"
                    textSize = 13f
                    gravity = Gravity.CENTER
                    setTextColor(palette.preedit)
                    setPadding(dp(12f), 0, dp(12f), 0)
                },
                LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.MATCH_PARENT,
                ),
            )
            return
        }
        candidates.forEachIndexed { i, text ->
            val item = candidateItem(text, i, highlighted, compact = true)
            candidateBar.addView(
                item,
                LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.MATCH_PARENT
                )
            )
        }
        if (candidates.isNotEmpty()) {
            addExpandedCandidates(candidates, highlighted)
        }
    }

    private fun candidateItem(text: String, index: Int, highlighted: Int, compact: Boolean): TextView =
        TextView(this).apply {
            this.text = text
            textSize = if (compact) 20f else 18f
            gravity = Gravity.CENTER
            setTextColor(if (index == highlighted) palette.candidateHl else palette.candidate)
            if (index == highlighted) typeface = Typeface.DEFAULT_BOLD
            setPadding(dp(if (compact) 15f else 8f), 0, dp(if (compact) 15f else 8f), 0)
            contentDescription = "第 ${index + 1} 候选词：$text"
            setOnClickListener { onCandidateSelect(index) }
            // B8 长按菜单：复制 / 删除该词（用户词典词条）
            setOnLongClickListener {
                showCandidateActions(text, index)
                true
            }
        }

    /** B8 候选长按菜单：复制 / 删除该词。 */
    private fun showCandidateActions(text: String, index: Int) {
        val options = arrayOf("复制", "删除该词")
        android.app.AlertDialog.Builder(this)
            .setTitle(text)
            .setItems(options) { dialog, which ->
                when (which) {
                    0 -> {
                        val cm = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        cm.setPrimaryClip(ClipData.newPlainText("候选词", text))
                        Toast.makeText(this, "已复制", Toast.LENGTH_SHORT).show()
                    }
                    1 -> {
                        val ok = try {
                            engineReady && RimeBridge.nativeForgetOnCurrentPage(index)
                        } catch (e: Throwable) {
                            false
                        }
                        if (ok) {
                            sync()
                        } else {
                            Toast.makeText(this, "该词不可删除", Toast.LENGTH_SHORT).show()
                        }
                    }
                }
                dialog.dismiss()
            }
            .show()
    }

    /**
     * B9 中/英模式角标：解析 nativeStatus（空串=中文），ascii=1 显示「英」否则「中」。
     * 只在候选面板可见时被调用（sync / onToggleLang 之后）。
     */
    private fun updateModeBadge() {
        val badge = modeBadge ?: return
        if (!engineReady) {
            badge.text = "中"
            return
        }
        val raw = try {
            RimeBridge.nativeStatus()
        } catch (e: Throwable) {
            ""
        }
        val ascii = raw.split("\u0001").getOrNull(0) == "1"
        badge.text = if (ascii) "英" else "中"
    }

    private fun addExpandedCandidates(candidates: List<String>, highlighted: Int) {
        candidates.chunked(CandidatePageSpec.GRID_COLUMNS).forEachIndexed { rowIndex, row ->
            val rowView = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
            row.forEachIndexed { columnIndex, text ->
                val index = rowIndex * CandidatePageSpec.GRID_COLUMNS + columnIndex
                rowView.addView(
                    candidateItem(text, index, highlighted, compact = false),
                    LinearLayout.LayoutParams(0, dp(38f), 1f),
                )
            }
            repeat(CandidatePageSpec.GRID_COLUMNS - row.size) {
                rowView.addView(View(this), LinearLayout.LayoutParams(0, dp(38f), 1f))
            }
            expandedCandidateBar.addView(rowView)
        }
        expandedCandidateBar.addView(LinearLayout(this).apply {
            gravity = Gravity.CENTER
            addView(candidatePageButton("‹", "上一页候选词") { onCandidatePage(previous = true) })
            addView(TextView(this@ShurufaImeService).apply {
                text = "滑动候选栏或翻页查看更多"
                gravity = Gravity.CENTER
                textSize = 12f
                setTextColor(palette.preedit)
            }, LinearLayout.LayoutParams(0, dp(34f), 1f))
            addView(candidatePageButton("›", "下一页候选词") { onCandidatePage(previous = false) })
        }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(34f)))
    }

    private fun candidatePageButton(label: String, description: String, onClick: () -> Unit): TextView =
        TextView(this).apply {
            text = label
            textSize = 26f
            gravity = Gravity.CENTER
            contentDescription = description
            setTextColor(palette.candidate)
            setOnClickListener { onClick() }
        }.also {
            it.layoutParams = LinearLayout.LayoutParams(dp(44f), dp(34f))
        }

    /** 按目标边长下采样解码，避免全分辨率大图在主线程 OOM/卡顿。 */
    override fun onDestroy() {
        super.onDestroy()
        voice?.cancel()
        voice = null
        ioExecutor.shutdownNow()
    }

    private fun decodeSampledBitmap(data: ByteArray, target: Int): Bitmap? {
        if (data.isEmpty()) return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(data, 0, data.size, bounds)
        var sample = 1
        val w = bounds.outWidth
        val h = bounds.outHeight
        while ((w / (sample * 2)) >= target || (h / (sample * 2)) >= target) {
            sample *= 2
        }
        val opts = BitmapFactory.Options().apply {
            inJustDecodeBounds = false
            inSampleSize = sample
        }
        return try {
            BitmapFactory.decodeByteArray(data, 0, data.size, opts)
        } catch (e: OutOfMemoryError) {
            // 极少见：极端内存下进一步放大采样
            opts.inSampleSize = sample * 2
            BitmapFactory.decodeByteArray(data, 0, data.size, opts)
        }
    }

}
