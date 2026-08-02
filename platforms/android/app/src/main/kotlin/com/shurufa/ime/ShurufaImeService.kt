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
import android.util.TypedValue
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
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
import java.net.URL
import java.net.HttpURLConnection
import java.net.URLConnection
import java.util.concurrent.Executors
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
        // 方案资源变化时递增，确保同一应用版本也会重新解包词典。
        private const val SCHEMA_BUNDLE_VERSION = "rime-ice-20260801"
        /// 缩略图采样目标边长（px），仅为展示用，不必保留原图尺寸
        private const val THUMBNAIL_TARGET = 260
        /// 预览图采样目标边长（px），兼顾清晰度与内存
        private const val PREVIEW_TARGET = 2048
        /// 单张贴纸下载大小上限（字节），防恶意/异常源耗尽存储与内存
        private const val MAX_STICKER_BYTES = 8L * 1024 * 1024

        private val LIGHT = Palette(
            bg = 0xFFF0F1F5.toInt(),
            key = 0xFFFFFFFF.toInt(),
            keyPressed = 0xFFC7CED9.toInt(),
            keyFunc = 0xFFD5DAE3.toInt(),
            funcPressed = 0xFFBFC6D2.toInt(),
            keyText = 0xFF1A1A1A.toInt(),
            funcText = 0xFF33383F.toInt(),
            candidate = 0xFF1A1A1A.toInt(),
            candidateHl = 0xFF35B982.toInt(),
            preedit = 0xFF8A9099.toInt(),
            syncBg = 0xFFFFF1DE.toInt(),
            syncText = 0xFF6A4A20.toInt(),
            accent = 0xFF35B982.toInt(),
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
            candidateHl = 0xFF68D3A0.toInt(),
            preedit = 0xFF8B9199.toInt(),
            syncBg = 0xFF3A331F.toInt(),
            syncText = 0xFFEAD6B0.toInt(),
            accent = 0xFF68D3A0.toInt(),
        )

        @Volatile
        private var engineReady = false

        @Volatile
        private var engineStarting = false
    }

    private lateinit var candidateBar: LinearLayout
    private lateinit var keyArea: LinearLayout
    private var inputRoot: LinearLayout? = null
    private var symbolMode = false
    /// 大写锁定（微信输入法同款 capslock 键：行首图标键）
    private var shiftMode = false
    private var syncBar: TextView? = null
    private var pendingSyncText: String? = null
    /// 同步收到的图片历史 id，syncBar 点击时上屏
    private var pendingSyncImageId: Int? = null
    /// 同步收到的文件历史 id，syncBar 点击时作为附件提交。
    private var pendingSyncFileId: Int? = null
    private val chineseStickerCatalog by lazy { ChineseStickerCatalog(this) }
    private val syncPoll = Handler(Looper.getMainLooper())
    private val stickerSearchHandler = Handler(Looper.getMainLooper())
    /// 受限并发 + 超时的贴纸下载线程池（避免无界线程/挂死）
    private val stickerIo = Executors.newFixedThreadPool(2) { r ->
        Thread(r, "sticker-io").apply { isDaemon = true }
    }
    private val stickerSuggestionState = StickerSuggestionState()
    private var pendingStickerSearch: Runnable? = null
    private var stickerBannerViews: List<View> = emptyList()
    private var stickerInsertIndex = 0
    private var pendingSyncToken: Long? = null
    private var historyPanel: LinearLayout? = null
    /// 微信输入法 S33 同款图片预览键盘（点图后先预览再保存/发送）
    private var previewKeyboard: LinearLayout? = null
    /// 预览键盘当前展示的图片历史 id
    private var previewImageId: Int? = null

    /// 当前主题；随系统深色设置在重建输入视图时更新
    private var palette: Palette = LIGHT

    /// 附件发送结果：SENT=已投递，COPIED=已复制到剪贴板（需长按粘贴），FAILED=彻底失败
    private enum class SendResult { SENT, COPIED, FAILED }

    /// 收起 syncBar 并清空待发送状态
    private fun dismissSyncBar() {
        pendingSyncToken?.let { SyncInbox.clear(this, it) }
        pendingSyncText = null
        pendingSyncImageId = null
        pendingSyncFileId = null
        pendingSyncToken = null
        syncBar?.visibility = View.GONE
    }

    override fun onCreate() {
        super.onCreate()
        ClipboardSyncService.start(applicationContext)
        ensureEngine()
        thread(name = "sync-start") {
            try {
                ClipStore.ensureInit(applicationContext)
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
        val version = "${appVersionCode()}-$SCHEMA_BUNDLE_VERSION"
        if (marker.takeIf { it.exists() }?.readText() == version) {
            return dest
        }
        dest.deleteRecursively()
        dest.mkdirs()
        copySchemaAssets("schemas", dest)
        marker.writeText(version)
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

    private fun isLandscape(): Boolean =
        resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE

    private fun candidateBarHeight(): Int = dp(if (isLandscape()) 36f else 42f)

    private fun candidateItemSize(): Int = dp(if (isLandscape()) 36f else 42f)

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
        palette = if (isDark()) DARK else LIGHT

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(palette.bg)
            setPadding(dp(1f), dp(1f), dp(1f), dp(1f))
        }
        inputRoot = root

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
                val fileId = pendingSyncFileId
                when {
                    // 微信输入法同款：点图先进预览键盘，再保存/发送
                    imgId != null -> {
                        dismissSyncBar()
                        openImagePreview(imgId)
                    }
                    fileId != null -> when (commitFile(fileId)) {
                        SendResult.SENT -> dismissSyncBar()
                        SendResult.COPIED -> {
                            text = "文件已复制到剪贴板，长按输入框粘贴即可发送"
                        }
                        SendResult.FAILED -> showAttachmentError("文件发送失败")
                    }
                    else -> {
                        val sent = pendingSyncText?.let {
                            currentInputConnection?.commitText(it, 1)
                            true
                        } ?: false
                        if (sent) dismissSyncBar() else showAttachmentError("无可发送内容")
                    }
                }
            }
        }
        root.addView(syncBar, LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT
        ).apply { setMargins(dp(3f), dp(2f), dp(3f), dp(2f)) })

        // 横屏进一步压缩候选栏，把垂直空间优先让给四行可触达的按键。
        val compactCandidateBar = isLandscape()
        val clipButtonSize = dp(if (compactCandidateBar) 28f else 32f)
        val clipVerticalMargin = dp(if (compactCandidateBar) 4f else 5f)
        val topRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setBackgroundColor(palette.key)
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, candidateBarHeight()
            )
        }
        val clipButton = TextView(this).apply {
            text = "▦"
            contentDescription = "剪贴板历史"
            gravity = Gravity.CENTER
            textSize = 16f
            setTextColor(0xFFFFFFFF.toInt())
            background = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(palette.accent)
            }
            setOnClickListener { toggleHistory() }
        }
        topRow.addView(
            clipButton,
            LinearLayout.LayoutParams(
                clipButtonSize,
                LinearLayout.LayoutParams.MATCH_PARENT
            ).apply { setMargins(dp(8f), clipVerticalMargin, dp(6f), clipVerticalMargin) }
        )
        candidateBar = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        val scroll = HorizontalScrollView(this).apply {
            addView(candidateBar)
            isHorizontalScrollBarEnabled = false
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, 1f)
        }
        topRow.addView(scroll)
        root.addView(topRow)
        // 候选栏与键区之间的细分隔线，避免白底候选和浅灰键区粘连。
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
            populateHistory(panel, onlyImages = false)
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
            populateHistory(panel, onlyImages = true)
            panel.visibility = View.VISIBLE
            keyArea.visibility = View.GONE
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
                val png = try {
                    ClipStore.imageData(id)
                } catch (e: Throwable) {
                    null
                }
                if (png == null) {
                    showAttachmentError("图片数据缺失")
                    return@setOnClickListener
                }
                val uri = ImageClipboard.saveToGallery(this@ShurufaImeService, png)
                if (uri != null) {
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
                val result = commitImage(id)
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
        // 隐藏一切可能挤压预览区的元素（同步条/历史面板）。
        syncBar?.visibility = View.GONE
        historyPanel?.visibility = View.GONE
        keyArea.visibility = View.GONE
        panel.visibility = View.VISIBLE
        android.util.Log.i("shurufa", "打开图片预览 历史ID=$id 字节=${png.size}")
    }

    private fun closeImagePreview() {
        // 同步条会挤压预览区，关闭时一并收起（发送反馈已用 toast）
        dismissSyncBar()
        previewKeyboard?.visibility = View.GONE
        previewImageId = null
        keyArea.visibility = View.VISIBLE
    }

    private fun populateHistory(panel: LinearLayout, onlyImages: Boolean = false) {
        panel.removeAllViews()
        panel.addView(TextView(this).apply {
            text = if (onlyImages) "斗图 · 点图片预览 · 再点 ⊞ 返回" else "剪贴板历史 · 点击上屏 · 再点 ⊞ 返回"
            textSize = 12f
            setTextColor(palette.preedit)
            setPadding(dp(14f), dp(8f), dp(14f), dp(8f))
        })
        val entries = try {
            ClipStore.list(30).filter { !onlyImages || it.kind == "image" }
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
                val supportedImageMime = currentSupportedImageMimeType()
                android.util.Log.i(
                    "shurufa",
                    "图片历史条目状态 历史ID=${entry.id} 声明MIME=$supportedImageMime",
                )
                val bmp = try {
                    ClipStore.imageData(entry.id)?.let { decodeSampledBitmap(it, THUMBNAIL_TARGET) }
                } catch (e: Throwable) {
                    null
                }
                val thumb = ImageView(this).apply {
                    contentDescription = "图片附件：${entry.source}，点击预览后发送到当前输入框"
                    if (bmp != null) setImageBitmap(bmp)
                    adjustViewBounds = true
                    maxHeight = dp(130f)
                    scaleType = ImageView.ScaleType.FIT_START
                    background = keyBackground(palette.key, palette.keyPressed)
                    alpha = 1f
                    setPadding(dp(10f), dp(8f), dp(10f), dp(8f))
                    // 微信输入法同款：点图先进预览键盘，再保存/发送
                    setOnClickListener { openImagePreview(entry.id) }
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
                    val fileMimeType = URLConnection.guessContentTypeFromName(fileName)
                        ?: "application/octet-stream"
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
        syncBar?.text = message
        syncBar?.visibility = View.VISIBLE
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
        keyboard.setAvailableKeyboardHeight(root.height - chromeHeight)
    }

    private fun buildLetterPage() {
        // 微信输入法 S2 键盘整块布局（反编译 JSON 直接渲染）
        val asciiMode = engineReady && RimeBridge.nativeIsAscii()
        keyArea.addView(
            WetypeKeyboardView(
                this,
                KeyboardLayoutSpec.Page.LETTERS,
                isDark(),
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
            WetypeKeyboardView.WetypeAction.Lang -> onToggleLang()
            WetypeKeyboardView.WetypeAction.Clear -> {
                if (engineReady) RimeBridge.nativeReset()
                currentInputConnection?.finishComposingText()
                updateCandidates("", emptyList(), 0)
            }
        }
    }
    private fun langLabel(): String =
        if (engineReady && RimeBridge.nativeIsAscii()) "英" else "中"

    private fun onToggleLang() {
        if (!engineReady) return
        RimeBridge.nativeToggleAscii()
        RimeBridge.nativeReset()
        if (::keyArea.isInitialized && !symbolMode) rebuildKeys() // 刷新中英键帽
        sync()
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
        clearStickerSuggestions()
        candidateBar.removeAllViews()
        // 拼音预编辑与候选共用一行，输入时不会额外抬高整个键盘。
        if (preedit.isNotEmpty()) {
            candidateBar.addView(
                TextView(this).apply {
                    text = preedit.take(12).let { if (preedit.length > it.length) "$it…" else it }
                    textSize = 13f
                    gravity = Gravity.CENTER
                    setTextColor(palette.preedit)
                    setPadding(dp(10f), 0, dp(8f), 0)
                },
                LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.MATCH_PARENT,
                ),
            )
        }
        stickerInsertIndex = if (preedit.isEmpty()) 0 else 1
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
        scheduleStickerSuggestions(preedit, candidates)
    }

    /** 立即撤掉旧横幅，并让已排队或后台运行的旧查询失效。 */
    private fun clearStickerSuggestions() {
        pendingStickerSearch?.let(stickerSearchHandler::removeCallbacks)
        pendingStickerSearch = null
        stickerSuggestionState.invalidate()
        if (::candidateBar.isInitialized) {
            stickerBannerViews.forEach(candidateBar::removeView)
        }
        stickerBannerViews = emptyList()
    }

    /** 输入停顿后才在线程中检索；主线程只负责确认请求仍有效并插入已得到的结果。 */
    private fun scheduleStickerSuggestions(preedit: String, candidates: List<String>) {
        val request = stickerSuggestionState.replace(preedit, candidates) ?: return
        if (!chineseStickerCatalog.isLoaded) return
        val runnable = Runnable {
            pendingStickerSearch = null
            thread {
                val stickers = chineseStickerCatalog.search(request.terms, StickerSuggestionPolicy.visibleLimit)
                stickerSearchHandler.post {
                    if (!::candidateBar.isInitialized || !stickerSuggestionState.isCurrent(request)) return@post
                    val views = stickers.map(::buildStickerCandidate)
                    stickerBannerViews = views
                    views.forEachIndexed { offset, view ->
                        candidateBar.addView(
                            view,
                            (stickerInsertIndex + offset).coerceAtMost(candidateBar.childCount),
                            LinearLayout.LayoutParams(candidateItemSize(), LinearLayout.LayoutParams.MATCH_PARENT),
                        )
                    }
                }
            }
        }
        pendingStickerSearch = runnable
        stickerSearchHandler.postDelayed(runnable, StickerSuggestionPolicy.delayMillis)
    }

    private fun buildStickerCandidate(sticker: ChineseStickerSearch.Entry): ImageView = ImageView(this).apply {
        contentDescription = "表情包：${sticker.name}"
        scaleType = ImageView.ScaleType.CENTER_CROP
        background = keyBackground(palette.key, palette.keyPressed)
        setPadding(dp(3f), dp(3f), dp(3f), dp(3f))
        setOnClickListener {
            clearStickerSuggestions()
            downloadAndSendSticker(sticker)
        }
        stickerIo.execute {
            val file = downloadStickerToCache(sticker)
            val bitmap = runCatching { file?.let { BitmapFactory.decodeFile(it.absolutePath) } }.getOrNull()
            if (bitmap != null) syncPoll.post { setImageBitmap(bitmap) }
        }
    }

    private fun downloadAndSendSticker(sticker: ChineseStickerSearch.Entry) {
        Toast.makeText(this, "正在发送表情包…", Toast.LENGTH_SHORT).show()
        stickerIo.execute {
            val file = downloadStickerToCache(sticker)
            syncPoll.post {
                val f = file
                if (f == null || !f.isFile || !commitAttachment(f, imageMimeType(f.name))) {
                    showAttachmentError("表情包发送失败")
                }
            }
        }
    }

    /**
     * 下载贴纸到缓存：带连接/读超时与字节上限，经临时文件原子改名落盘。
     * 失败或超限返回 null。可并发调用（由受限线程池调度）。
     */
    private fun downloadStickerToCache(sticker: ChineseStickerSearch.Entry): File? {
        val file = stickerCacheFile(sticker)
        if (file.isFile && file.length() in 1..MAX_STICKER_BYTES) return file
        return try {
            file.parentFile?.mkdirs()
            val conn = URL(sticker.url).openConnection() as HttpURLConnection
            try {
                conn.connectTimeout = 10_000
                conn.readTimeout = 15_000
                conn.setRequestProperty("Accept", "image/*")
                conn.instanceFollowRedirects = true
                val input = conn.inputStream
                val tmp = File(file.parentFile, "${file.name}.part")
                var total = 0L
                try {
                    // 输出流只打开一次；多轮复用，避免每轮重开截断文件
                    tmp.outputStream().use { out ->
                        val buf = ByteArray(16 * 1024)
                        while (true) {
                            val n = input.read(buf)
                            if (n < 0) break
                            total += n
                            if (total > MAX_STICKER_BYTES) return null
                            out.write(buf, 0, n)
                        }
                    }
                    if (total in 1..MAX_STICKER_BYTES && tmp.renameTo(file)) {
                        file
                    } else {
                        tmp.delete()
                        null
                    }
                } finally {
                    input.close()
                }
            } finally {
                conn.disconnect()
            }
        } catch (e: Throwable) {
            android.util.Log.w("shurufa", "贴纸下载失败：${sticker.name}", e)
            file.delete()
            null
        }
    }

    /** 按目标边长下采样解码，避免全分辨率大图在主线程 OOM/卡顿。 */
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

    private fun stickerCacheFile(sticker: ChineseStickerSearch.Entry): File {
        val extension = sticker.url.substringAfterLast('.', "jpg").substringBefore('?').take(5)
        return File(cacheDir, "stickers/${sticker.url.hashCode().toUInt().toString(16)}.$extension")
    }

    private fun imageMimeType(name: String): String = when (name.substringAfterLast('.', "").lowercase()) {
        "png" -> "image/png"
        "webp" -> "image/webp"
        "gif" -> "image/gif"
        else -> "image/jpeg"
    }

    override fun onStartInputView(info: android.view.inputmethod.EditorInfo?, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        // 系统深色设置变化后重建输入视图，刷新主题
        if ((palette === DARK) != isDark()) {
            setInputView(onCreateInputView())
        }
        ensureEngine()
        chineseStickerCatalog.load { syncPoll.post { sync() } }
        if (engineReady) RimeBridge.nativeReset()
        updateCandidates("", emptyList(), 0)
        historyPanel?.visibility = View.GONE
        keyArea.visibility = View.VISIBLE
        refreshSyncInbox()
        startInboxPolling()
    }

    override fun onFinishInput() {
        super.onFinishInput()
        if (engineReady) RimeBridge.nativeReset()
        clearStickerSuggestions()
        syncPoll.removeCallbacksAndMessages(null)
    }

    override fun onDestroy() {
        clearStickerSuggestions()
        stickerSearchHandler.removeCallbacksAndMessages(null)
        stickerIo.shutdownNow()
        super.onDestroy()
    }

    private fun startInboxPolling() {
        syncPoll.removeCallbacksAndMessages(null)
        val tick = object : Runnable {
            override fun run() {
                refreshSyncInbox()
                syncPoll.postDelayed(this, 500)
            }
        }
        syncPoll.postDelayed(tick, 500)
    }

    private fun refreshSyncInbox() {
        val event = SyncInbox.load(this) ?: return
        if (event.token == pendingSyncToken) return
        pendingSyncToken = event.token
        when (event.kind) {
            "text" -> if (event.payload.isNotEmpty()) {
                pendingSyncText = event.payload
                pendingSyncImageId = null
                pendingSyncFileId = null
                val preview = event.payload.replace('\n', ' ').take(30)
                syncBar?.text = "来自 ${event.from}：$preview（点此上屏）"
                syncBar?.visibility = View.VISIBLE
            }
            "image" -> event.payload.toIntOrNull()?.let { id ->
                pendingSyncImageId = id
                pendingSyncText = null
                pendingSyncFileId = null
                syncBar?.text = "来自 ${event.from}：收到图片（点此发送）"
                syncBar?.visibility = View.VISIBLE
            }
            "file" -> event.payload.toIntOrNull()?.let { id ->
                pendingSyncFileId = id
                pendingSyncText = null
                pendingSyncImageId = null
                syncBar?.text = "来自 ${event.from}：收到文件（点此发送）"
                syncBar?.visibility = View.VISIBLE
            }
        }
    }
}
