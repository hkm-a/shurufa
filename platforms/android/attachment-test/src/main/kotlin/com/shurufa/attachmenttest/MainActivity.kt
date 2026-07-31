package com.shurufa.attachmenttest

import android.app.Activity
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Gravity
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.core.view.ContentInfoCompat
import androidx.core.view.ViewCompat
import androidx.core.view.inputmethod.EditorInfoCompat
import androidx.core.view.inputmethod.InputConnectionCompat
import androidx.core.content.FileProvider
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.InputStream
import java.util.concurrent.Executors

class MainActivity : Activity() {
    private val mainHandler = Handler(Looper.getMainLooper())
    private val ioExecutor = Executors.newSingleThreadExecutor()
    private lateinit var resultView: TextView
    private lateinit var previewView: ImageView
    private var automationAction: String? = null
    private var automationHandled = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (handleAutomation(intent)) return

        val content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(18), dp(24), dp(18), dp(24))
        }
        content.addView(label("输入法附件测试器", 23f, Color.rgb(30, 30, 30)))
        content.addView(label("依次点击下面四个输入框，再从输入法点击同一张同步图片或文件。", 14f, Color.DKGRAY))

        content.addView(sectionTitle("1. 标准富内容输入框"))
        content.addView(label("发布 image/* 和 application/*，用于验证 commitContent 与 URI 临时授权。", 13f, Color.DKGRAY))
        content.addView(RichContentEditText())

        content.addView(sectionTitle("2. 仅接收 JPEG 的富内容输入框"))
        content.addView(label("只发布 image/jpeg，用于验证输入法按目标能力转码。", 13f, Color.DKGRAY))
        content.addView(RichContentEditText(
            acceptedMimeTypes = arrayOf("image/jpeg"),
            hintText = "点这里测试 JPEG 协商",
            accessibilityName = "仅接收 JPEG 的富内容输入框",
        ))

        content.addView(sectionTitle("3. 未声明但支持接收的输入框（模拟抖音评论框）"))
        content.addView(label("不发布 MIME 但实现 OnReceiveContentListener，验证无条件 commitContent 仍能插入。", 13f, Color.DKGRAY))
        content.addView(RichContentEditText(
            acceptedMimeTypes = arrayOf("image/*", "application/*"),
            declareMimeTypes = false,
            hintText = "点这里测试未声明附件能力",
            accessibilityName = "未声明附件能力的输入框",
        ))

        content.addView(sectionTitle("4. 普通文本输入框"))
        content.addView(label("不支持附件，作为预期失败的负对照。", 13f, Color.DKGRAY))
        content.addView(EditText(this).apply {
            hint = "普通文本输入框"
            contentDescription = "普通文本输入框"
            minHeight = dp(56)
        })

        content.addView(sectionTitle("接收结果"))
        resultView = label("尚未收到附件", 14f, Color.rgb(40, 40, 40)).apply {
            setTextIsSelectable(true)
            setPadding(dp(12), dp(12), dp(12), dp(12))
            setBackgroundColor(Color.rgb(242, 243, 245))
        }
        content.addView(resultView)
        previewView = ImageView(this).apply {
            adjustViewBounds = true
            maxHeight = dp(280)
            scaleType = ImageView.ScaleType.FIT_CENTER
            setBackgroundColor(Color.rgb(242, 243, 245))
        }
        content.addView(previewView, LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT,
            LinearLayout.LayoutParams.WRAP_CONTENT,
        ).apply { topMargin = dp(8) })
        content.addView(Button(this).apply {
            text = "清空结果"
            setOnClickListener {
                resultView.text = "尚未收到附件"
                previewView.setImageDrawable(null)
            }
        })

        setContentView(ScrollView(this).apply { addView(content) })
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || automationHandled) return
        when (automationAction) {
            ACTION_PUBLISH_IMAGE -> publishImageClipboard()
            ACTION_INSPECT_CLIPBOARD -> inspectClipboard()
            else -> return
        }
        automationHandled = true
        finish()
    }

    private fun handleAutomation(intent: Intent): Boolean {
        val action = intent.action
        if (action != ACTION_PUBLISH_IMAGE && action != ACTION_INSPECT_CLIPBOARD) return false
        automationAction = action
        setContentView(TextView(this).apply { text = "正在执行剪贴板自动化检查" })
        return true
    }

    private fun publishImageClipboard() {
        try {
            val file = File(cacheDir, "automation/android-source.png").apply {
                parentFile?.mkdirs()
                writeBytes(createAutomationPng())
            }
            val uri = FileProvider.getUriForFile(this, "$packageName.fileprovider", file)
            val description = ClipDescription("android-source.png", arrayOf("image/png"))
            val clipboard = getSystemService(CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData(description, ClipData.Item(uri)))
            Log.i(AUTOMATION_TAG, "已发布图片剪贴板 类型=image/png 宽=37 高=23")
        } catch (e: Throwable) {
            Log.e(AUTOMATION_TAG, "发布图片剪贴板失败", e)
        }
    }

    private fun inspectClipboard() {
        try {
            val clipboard = getSystemService(CLIPBOARD_SERVICE) as ClipboardManager
            val clip = clipboard.primaryClip ?: error("系统剪贴板为空")
            val item = clip.getItemAt(0)
            val uri = item.uri ?: error("系统剪贴板不是 URI 类型：${item.text}")
            val bytes = contentResolver.openInputStream(uri)?.use { it.readBytes() }
                ?: error("无法读取剪贴板 URI：$uri")
            val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                ?: error("剪贴板 URI 不是可解码图片")
            val declared = clip.description.filterMimeTypes("image/*")?.joinToString().orEmpty()
            val resolved = contentResolver.getType(uri).orEmpty()
            Log.i(
                AUTOMATION_TAG,
                "检查剪贴板成功 标签=${clip.description.label} URI=$uri 声明=$declared 解析=$resolved " +
                    "字节=${bytes.size} 宽=${bitmap.width} 高=${bitmap.height}",
            )
            bitmap.recycle()
        } catch (e: Throwable) {
            Log.e(AUTOMATION_TAG, "检查剪贴板失败", e)
        }
    }

    private fun createAutomationPng(): ByteArray {
        val bitmap = Bitmap.createBitmap(37, 23, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        canvas.drawColor(Color.rgb(18, 93, 210))
        val paint = Paint().apply { color = Color.WHITE }
        canvas.drawRect(3f, 3f, 12f, 12f, paint)
        return ByteArrayOutputStream().use { output ->
            check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)) { "PNG 编码失败" }
            bitmap.recycle()
            output.toByteArray()
        }
    }

    override fun onDestroy() {
        ioExecutor.shutdownNow()
        super.onDestroy()
    }

    private inner class RichContentEditText(
        private val acceptedMimeTypes: Array<String> = arrayOf("image/*", "application/*"),
        private val declareMimeTypes: Boolean = true,
        private val hintText: String = "点这里测试 commitContent",
        private val accessibilityName: String = "标准富内容输入框",
    ) : EditText(this) {

        init {
            hint = hintText
            contentDescription = accessibilityName
            minHeight = dp(56)
            ViewCompat.setOnReceiveContentListener(this, acceptedMimeTypes) { _, payload ->
                val clip = payload.clip
                val item = clip.getItemAt(0)
                val uri = item.uri ?: return@setOnReceiveContentListener payload
                val input = try {
                    contentResolver.openInputStream(uri)
                } catch (e: Throwable) {
                    showImmediateFailure(
                        source = sourceName(payload.source),
                        label = clip.description.label?.toString().orEmpty(),
                        uri = uri,
                        declaredMimeTypes = clip.description.filterMimeTypes("*/*")?.toList().orEmpty(),
                        error = e,
                    )
                    return@setOnReceiveContentListener payload
                } ?: return@setOnReceiveContentListener payload
                readAttachment(
                    source = sourceName(payload.source),
                    uri = uri,
                    label = clip.description.label?.toString().orEmpty(),
                    declaredMimeTypes = clip.description.filterMimeTypes("*/*")?.toList().orEmpty(),
                    openedInput = input,
                )
                null
            }
        }

        override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? {
            val connection = super.onCreateInputConnection(outAttrs) ?: return null
            if (declareMimeTypes) {
                EditorInfoCompat.setContentMimeTypes(outAttrs, acceptedMimeTypes)
            }
            return InputConnectionCompat.createWrapper(this, connection, outAttrs)
        }
    }

    private fun showImmediateFailure(
        source: String,
        label: String,
        uri: Uri,
        declaredMimeTypes: List<String>,
        error: Throwable,
    ) {
        resultView.text = AttachmentReadResult(
            source = source,
            label = label,
            uri = uri.toString(),
            declaredMimeTypes = declaredMimeTypes,
            resolvedMimeType = null,
            byteCount = null,
            error = error.message ?: error.javaClass.simpleName,
        ).asDisplayText().also { Log.e(TAG, it, error) }
    }

    private fun readAttachment(
        source: String,
        uri: Uri,
        label: String,
        declaredMimeTypes: List<String>,
        openedInput: InputStream?,
    ) {
        resultView.text = "正在读取 $uri"
        ioExecutor.execute {
            var byteCount: Long? = null
            var error: String? = null
            var preview: ByteArray? = null
            try {
                val input = openedInput ?: contentResolver.openInputStream(uri)
                input?.use {
                    val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
                    val firstBytes = java.io.ByteArrayOutputStream()
                    var total = 0L
                    while (true) {
                        val count = it.read(buffer)
                        if (count < 0) break
                        total += count
                        if (firstBytes.size() < MAX_PREVIEW_BYTES) {
                            val copyCount = minOf(count, MAX_PREVIEW_BYTES - firstBytes.size())
                            firstBytes.write(buffer, 0, copyCount)
                        }
                    }
                    byteCount = total
                    preview = firstBytes.toByteArray()
                } ?: error("ContentResolver 返回空输入流")
            } catch (e: Throwable) {
                error = e.message ?: e.javaClass.simpleName
            }

            val result = AttachmentReadResult(
                source = source,
                label = label,
                uri = uri.toString(),
                declaredMimeTypes = declaredMimeTypes,
                resolvedMimeType = contentResolver.getType(uri),
                byteCount = byteCount,
                error = error,
            )
            mainHandler.post {
                val displayText = result.asDisplayText()
                resultView.text = displayText
                if (result.error == null) Log.i(TAG, displayText) else Log.e(TAG, displayText)
                val bytes = preview
                val bitmap = if (error == null && result.resolvedMimeType?.startsWith("image/") == true && bytes != null) {
                    BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                } else {
                    null
                }
                previewView.setImageBitmap(bitmap)
            }
        }
    }

    private fun sourceName(source: Int): String = when (source) {
        ContentInfoCompat.SOURCE_INPUT_METHOD -> "commitContent"
        ContentInfoCompat.SOURCE_CLIPBOARD -> "系统剪贴板"
        ContentInfoCompat.SOURCE_DRAG_AND_DROP -> "拖放"
        ContentInfoCompat.SOURCE_APP -> "应用"
        ContentInfoCompat.SOURCE_AUTOFILL -> "自动填充"
        ContentInfoCompat.SOURCE_PROCESS_TEXT -> "文本处理"
        else -> "未知来源($source)"
    }

    private fun label(text: String, size: Float, color: Int) = TextView(this).apply {
        this.text = text
        textSize = size
        setTextColor(color)
        setPadding(0, dp(4), 0, dp(8))
    }

    private fun sectionTitle(text: String) = label(text, 18f, Color.rgb(40, 40, 40)).apply {
        gravity = Gravity.START
        setPadding(0, dp(18), 0, dp(4))
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

    companion object {
        private const val TAG = "shurufa-attachment-test"
        private const val AUTOMATION_TAG = "shurufa-sync-test"
        private const val MAX_PREVIEW_BYTES = 4 * 1024 * 1024
        const val ACTION_PUBLISH_IMAGE = "com.shurufa.attachmenttest.PUBLISH_IMAGE"
        const val ACTION_INSPECT_CLIPBOARD = "com.shurufa.attachmenttest.INSPECT_CLIPBOARD"
    }
}
