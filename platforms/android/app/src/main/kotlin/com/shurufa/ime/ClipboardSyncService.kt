package com.shurufa.ime

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.ServiceCompat
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URLConnection
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.TimeUnit

/**
 * 与输入法界面解耦的剪贴板同步所有者。默认输入法 UID 可在 Android 10+
 * 后台读取系统剪贴板；服务持续监听本机变化并消费唯一的 JNI 入站队列。
 */
class ClipboardSyncService : Service(), ClipboardManager.OnPrimaryClipChangedListener {
    private lateinit var clipboard: ClipboardManager
    private lateinit var capture: ClipboardCapture
    private lateinit var executor: ScheduledExecutorService
    private var lastClipboardSignature: Int? = null
    private var lastClipboardAtNanos = 0L

    override fun onCreate() {
        super.onCreate()
        startAsForeground()
        clipboard = getSystemService(CLIPBOARD_SERVICE) as ClipboardManager
        capture = ClipboardCapture(applicationContext)
        executor = java.util.concurrent.Executors.newSingleThreadScheduledExecutor { task ->
            Thread(task, "clipboard-sync").apply { isDaemon = true }
        }
        clipboard.addPrimaryClipChangedListener(this)
        executor.scheduleWithFixedDelay(::pollOnce, 0, POLL_MILLIS, TimeUnit.MILLISECONDS)
        // 启动时不立即读取剪贴板：服务随 IME 弹出而创建，立即读取会触发
        // 系统的剪贴板访问面板（点开文本框时闪现）。延迟数秒后再捕获当前
        // 内容，之后的变化由 onPrimaryClipChanged 监听器驱动。
        executor.schedule(::captureCurrentClipboard, INITIAL_CAPTURE_DELAY_SECONDS, TimeUnit.SECONDS)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    override fun onPrimaryClipChanged() {
        if (::executor.isInitialized && !executor.isShutdown) executor.execute(::captureCurrentClipboard)
    }

    override fun onDestroy() {
        if (::clipboard.isInitialized) clipboard.removePrimaryClipChangedListener(this)
        if (::executor.isInitialized) executor.shutdownNow()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun ensureReady(): Boolean = try {
        ClipStore.ensureInit(applicationContext) && SyncBridge.ensureStarted(applicationContext)
    } catch (e: Throwable) {
        Log.e(TAG, "同步服务初始化失败", e)
        false
    }

    private fun captureCurrentClipboard() {
        if (!ensureReady()) return
        val clip = try {
            clipboard.primaryClip
        } catch (e: Throwable) {
            Log.w(TAG, "后台读取系统剪贴板失败，请确认 Shurufa 是默认输入法", e)
            null
        } ?: return
        val content = capture.capture(clip) ?: return
        val now = System.nanoTime()
        if (content.signature == lastClipboardSignature &&
            now - lastClipboardAtNanos < DUPLICATE_WINDOW_NANOS
        ) return
        lastClipboardSignature = content.signature
        lastClipboardAtNanos = now
        try {
            when (content) {
                is CapturedClipboard.Text -> {
                    ClipStore.insert(content.value, "本机")
                    SyncBridge.nativeSendClip(content.value)
                }
                is CapturedClipboard.Image -> {
                    ClipStore.insertImage(content.png, "本机")
                    SyncBridge.nativeSendImage(content.png)
                }
                is CapturedClipboard.FileContent -> {
                    val file = receivedDir().resolve(
                        "local-${System.currentTimeMillis()}-${safeFileName(content.name)}",
                    ).apply { writeBytes(content.data) }
                    ClipStore.insertFiles(listOf(file.absolutePath), "本机")
                    SyncBridge.nativeSendFile(content.name, content.mimeType, content.data)
                }
            }
        } catch (e: Throwable) {
            Log.e(TAG, "剪贴板内容发送失败", e)
        }
    }

    private fun pollOnce() {
        if (!ensureReady()) return
        repeat(MAX_EVENTS_PER_TICK) {
            val incoming = try {
                SyncIncomingProtocol.parse(SyncBridge.nativePoll())
            } catch (e: Throwable) {
                Log.e(TAG, "读取同步入站队列失败", e)
                null
            } ?: return
            applyIncoming(incoming)
        }
    }

    private fun applyIncoming(incoming: SyncIncomingProtocol.Incoming) {
        try {
            when (incoming.kind) {
                "text" -> {
                    if (incoming.payload.isEmpty()) return
                    ClipStore.insert(incoming.payload, "同步·${incoming.from}")
                    val clip = ClipData.newPlainText(
                        ClipboardTypePolicy.REMOTE_LABEL_PREFIX + incoming.from,
                        incoming.payload,
                    )
                    clipboard.setPrimaryClip(clip)
                }
                "image" -> {
                    val id = incoming.payload.toIntOrNull() ?: return
                    val png = ClipStore.imageData(id) ?: return
                    setImageClipboard(png, "来自 ${incoming.from} 的图片")
                }
                "file" -> {
                    val id = incoming.payload.toIntOrNull() ?: return
                    val entry = ClipStore.list(100).firstOrNull { it.id == id && it.kind == "files" }
                        ?: return
                    val path = entry.text.lineSequence().firstOrNull()?.takeIf { it.isNotBlank() }
                        ?: return
                    val file = File(path)
                    if (!file.isFile) return
                    val mime = URLConnection.guessContentTypeFromName(file.name)
                        ?: "application/octet-stream"
                    setUriClipboard(file, mime, "Shurufa 同步·${incoming.from}·${file.name}")
                }
            }
            Log.i(TAG, "已接收 ${incoming.from} 的${kindLabel(incoming.kind)}并写入系统剪贴板")
        } catch (e: Throwable) {
            Log.e(TAG, "入站内容写入系统剪贴板失败", e)
        }
    }

    private fun setImageClipboard(png: ByteArray, label: String) {
        val uri = ImageClipboard.setClipboard(this, png, label)
        Log.i(TAG, "图片已写入系统剪贴板 字节=${png.size} URI=$uri")
    }

    private fun setUriClipboard(file: File, mimeType: String, label: String) {
        val uri: Uri = FileProvider.getUriForFile(this, "$packageName.fileprovider", file)
        setClipboardUri(uri, mimeType, label)
    }

    private fun setClipboardUri(uri: Uri, mimeType: String, label: String) {
        val description = ClipDescription(label, arrayOf(mimeType))
        clipboard.setPrimaryClip(ClipData(description, ClipData.Item(uri)))
    }

    private fun receivedDir(): File = File(filesDir, "sync/received").apply { mkdirs() }

    private fun kindLabel(kind: String): String = when (kind) {
        "image" -> "图片"
        "file" -> "文件"
        else -> "文本"
    }

    private fun startAsForeground() {
        val manager = getSystemService(NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_ID,
                    getString(R.string.sync_channel_name),
                    NotificationManager.IMPORTANCE_LOW,
                ).apply { description = getString(R.string.sync_channel_description) },
            )
        }
        val openApp = PendingIntent.getActivity(
            this,
            0,
            Intent(this, PairActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentTitle(getString(R.string.sync_notification_title))
            .setContentText(getString(R.string.sync_notification_text))
            .setContentIntent(openApp)
            .setOngoing(true)
            .setCategory(Notification.CATEGORY_SERVICE)
            .build()
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
        } else {
            0
        }
        ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, type)
    }

    private fun safeFileName(name: String): String {
        val value = name.substringAfterLast('/').substringAfterLast('\\')
            .filter { it.isLetterOrDigit() || it in setOf('.', '-', '_', ' ') }
            .trim('.', ' ')
        return value.take(120).ifEmpty { "attachment" }
    }

    companion object {
        private const val TAG = "shurufa-sync"
        private const val CHANNEL_ID = "clipboard_sync"
        private const val NOTIFICATION_ID = 48632
        private const val POLL_MILLIS = 500L
        private const val MAX_EVENTS_PER_TICK = 8
        private const val DUPLICATE_WINDOW_NANOS = 1_500_000_000L
        /// 初始捕获延迟：避开 IME 弹出窗口期，防止系统剪贴板面板闪现
        private const val INITIAL_CAPTURE_DELAY_SECONDS = 8L

        fun start(context: Context) {
            ContextCompat.startForegroundService(
                context.applicationContext,
                Intent(context.applicationContext, ClipboardSyncService::class.java),
            )
        }
    }
}
