package com.shurufa.ime

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
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
import android.view.MotionEvent
import android.view.View
import android.view.inputmethod.EditorInfo
import android.widget.HorizontalScrollView
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.PopupWindow
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
    private var emojiMode = false
    /// 大写锁定（微信输入法同款 capslock 键：行首图标键）
    private var shiftMode = false
    private var syncBar: TextView? = null
    private var pendingSyncText: String? = null
    /// 同步收到的图片历史 id，syncBar 点击时上屏
    private var pendingSyncImageId: Int? = null
    /// 同步收到的文件历史 id，syncBar 点击时作为附件提交。
    private var pendingSyncFileId: Int? = null
    private val syncPoll = Handler(Looper.getMainLooper())
    private var pendingSyncToken: Long? = null
    private var historyPanel: LinearLayout? = null
    /// 微信输入法 S33 同款图片预览键盘（点图后先预览再保存/发送）
    private var previewKeyboard: LinearLayout? = null
    /// 预览键盘当前展示的图片历史 id
    private var previewImageId: Int? = null

    /// 当前主题；随系统深色设置在重建输入视图时更新
    private var palette: Palette = LIGHT
    /// 字符气泡（按键放大预览）
    private var bubble: PopupWindow? = null
    private var bubbleText: TextView? = null
    /// 退格长按连删定时器
    private val repeatHandler = Handler(Looper.getMainLooper())

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
        val version = appVersionCode().toString()
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
                .apply { setMargins(dp(1.5f), dp(2f), dp(1.5f), dp(2f)) }
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
                .apply { setMargins(dp(1.5f), dp(2f), dp(1.5f), dp(2f)) }
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
            setPadding(dp(1f), dp(1f), dp(1f), dp(1f))
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

        // 候选栏行（微信输入法布局：顶部整条白底候选栏 + 左侧工具按钮）
        val topRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            // 微信候选栏：白底与键盘区分
            setBackgroundColor(palette.key)
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, dp(46f)
            )
        }
        // 候选栏与键盘之间的细分隔线（微信同款层次感）
        root.addView(View(this).apply {
            setBackgroundColor(if (isDark()) 0xFF3A3F47.toInt() else 0xFFD8DCE3.toInt())
        }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(1f)))
        val clipButton = TextView(this).apply {
            text = "⊞"
            contentDescription = "剪贴板历史"
            gravity = Gravity.CENTER
            textSize = 21f
            setTextColor(palette.accent)
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
        // 斗图键（微信输入法同款入口）：打开图片历史，点图预览后发送
        val memeButton = TextView(this).apply {
            text = "🖼"
            contentDescription = "斗图（图片历史）"
            gravity = Gravity.CENTER
            textSize = 19f
            setPadding(dp(14f), 0, dp(14f), 0)
            setOnClickListener { toggleImageHistory() }
        }
        topRow.addView(
            memeButton,
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
        // 键盘高度 = 屏幕 39%（微信输入法同款比例）：固定高度让 IME 窗口跟随，任何分辨率都满屏
        val metrics = resources.displayMetrics
        val kbHeight = (metrics.heightPixels * 0.39f).toInt()
        root.addView(
            keyArea,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                kbHeight - dp(47f), // 候选栏 46dp + 分隔线 1dp
            ),
        )
        rebuildKeys()

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
        val bmp = BitmapFactory.decodeByteArray(png, 0, png.size)
        if (bmp != null) image.setImageBitmap(bmp)
        previewImageId = id
        // 隐藏一切可能挤压预览区的元素（同步条/拼音行/历史面板）
        syncBar?.visibility = View.GONE
        preeditView.visibility = View.GONE
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
                    ClipStore.imageData(entry.id)?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }
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
        langKey = null
        when {
            emojiMode -> buildEmojiPage()
            symbolMode -> buildSymbolPage()
            else -> buildLetterPage()
        }
    }

    private fun buildLetterPage() {
        // 微信输入法 S2 布局：候选栏下直接 3 行字母 + 底部功能行（无独立数字行，数字在符号页）
        listOf("qwertyuiop", "asdfghjkl", "zxcvbnm").forEachIndexed { index, row ->
            keyArea.addView(buildLetterRow(row, withBackspace = index == 2))
        }
        keyArea.addView(buildBottomRow())
    }

    private fun buildSymbolPage() {
        // 符号页第一行放数字（微信输入法同款：数字在符号页）
        listOf(
            "1234567890",
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
        // 行高自适应：4 行均分键盘高度
        layoutParams = LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f
        )
    }

    private fun buildLetterRow(letters: String, withBackspace: Boolean): LinearLayout {
        val row = rowLayout()
        // 微信输入法 S2 行首是 capslock 图标键（不是空白占位）
        if (letters.length == 9) row.addView(shiftKey(1.2f))
        letters.forEach { c -> row.addView(charKey(c.toString(), 1f) { onLetter(c) }) }
        if (withBackspace) {
            row.addView(backspaceKey(1.2f))
        } else if (letters.length == 9) {
            row.addView(spacer(0.5f))
        }
        return row
    }

    /// 大写键（微信输入法 icon_keys_outlinedcapslock 同款）：点一次切大写，再点切回。
    private fun shiftKey(weight: Float): TextView = funcKey("⇧", weight) {
        shiftMode = !shiftMode
        rebuildKeys()
    }.apply {
        // 大写态高亮（微信输入法 capslock 按下变色）
        if (shiftMode) {
            setTextColor(palette.accent)
            background = keyBackground(palette.keyFunc, palette.funcPressed)
        }
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

    /** 表情页（微信输入法同款：表情网格 + 底部功能行，点表情直接上屏） */
    private fun buildEmojiPage() {
        listOf(
            "😀😁😂🤣😊😍😘😜🤪🤔",
            "👍👎🙏👏🤝💪🔥❤️💔😭",
            "🎉🎂🎁⭐🌟☀️🌙🌈🍀🎯",
            "🐶🐱🐼🐯🦊🐸🐵🐷🐮🐔",
            "🚗🚕🚀✈️🏠💰📱💻🎧🎮",
            "✅❌❗❓➕➖➗✖️💯🔔",
        ).forEach { line ->
            val row = rowLayout()
            line.forEach { e ->
                val emoji = e.toString()
                row.addView(charKey(emoji, 1f) {
                    currentInputConnection?.commitText(emoji, 1)
                }.apply {
                    textSize = 22f
                    // 表情不需要放大气泡：覆盖触摸为纯点击
                    setOnTouchListener(null)
                    setOnClickListener { currentInputConnection?.commitText(emoji, 1) }
                })
            }
            keyArea.addView(row)
        }
        keyArea.addView(buildBottomRow())
    }

    private fun buildBottomRow(): LinearLayout {
        val row = rowLayout()
        // 微信输入法 S2 真实底部功能行（反编译 JSON 实证）：
        //   123 | 表情 | ， | 换行 | 空格(最长) | 中/英
        // 宽度比 92:92:92:92:103:92
        row.addView(funcKey(if (symbolMode || emojiMode) "返回" else "123", 1f) {
            if (symbolMode || emojiMode) {
                symbolMode = false
                emojiMode = false
            } else {
                // 微信输入法 icon_keys_123 同款：主键盘点 123 切数字/符号页
                symbolMode = true
            }
            shiftMode = false
            rebuildKeys()
        })
        row.addView(funcKey("☺", 1f) {
            symbolMode = false
            emojiMode = !emojiMode
            shiftMode = false
            rebuildKeys()
        })
        row.addView(funcKey("，", 1f) { onPunct("，") })
        row.addView(funcKey("换行", 1f) { onEnter() })
        row.addView(charKey("空格", 1.12f) { onSpace() }.apply {
            // 空格用字符键的底色更醒目，但不需要气泡：覆盖触摸为纯点击
            setOnTouchListener(null)
            setOnClickListener { onSpace() }
        })
        langKey = funcKey(langLabel(), 1f) { onToggleLang() }
        row.addView(langKey)
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
        refreshSyncInbox()
        startInboxPolling()
    }

    override fun onFinishInput() {
        super.onFinishInput()
        if (engineReady) RimeBridge.nativeReset()
        syncPoll.removeCallbacksAndMessages(null)
        hideBubble()
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
