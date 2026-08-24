package com.shurufa.ime

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import java.io.File
import java.util.Locale

/**
 * 同步 JNI 桥（platforms/android/rimejni/src/sync_jni.rs）的 Kotlin 声明。
 *
 * 入站条目与配对码协议以 \u0001 分隔字段。所有 native 调用线程安全，
 * 但 `nativePairBegin` 阻塞直至配对结束，须在后台线程调用。
 *
 * 文件 v3 入站 Offer：Rust 侧把决策回调挂进 SyncService，
 * 触发时同步调用 [IFileConfirmCallback.onOffer] 并由本类负责弹
 * 系统通知；用户在 Android 系统通知上点「接受/拒绝」后经
 * [nativeConfirmOffer] 回传给 Rust 唤醒被阻塞的回调。
 */
/** 一次已自动合并、等待用户确认的配置冲突。 */
data class ConfigConflict(
    val tsMs: Long,
    val kind: String,
    val name: String,
    val localBackup: String,
    val remoteBackup: String,
    val mergedSha256: String,
)

object SyncBridge {
    init {
        System.loadLibrary("shurufa_rime")
    }

    external fun nativeStart(configDir: String, deviceName: String): Boolean
    external fun nativePoll(): String
    external fun nativeSendClip(text: String)
    external fun nativeSendImage(png: ByteArray)
    external fun nativeSendFile(name: String, mimeType: String, data: ByteArray)
    /** v3 文件同步：整路径直送，由 SyncService 内部完成分块/ACK 状态机。 */
    external fun nativeSendFilePath(path: String): Boolean
    /** 配置/短语/皮肤同步（config-sync-v1）：读本地文件后广播给已配对设备。 */
    external fun nativeSendConfig(kind: String, path: String): Boolean
    /** 列出配置同步备份文件名（每行一个，来自 sync-config-backups/）。 */
    external fun nativeConfigBackups(): String
    /** 从备份文件恢复配置/短语/皮肤，返回是否成功。 */
    external fun nativeRestoreConfigBackup(file: String): Boolean
    /** 列出待用户确认的配置冲突记录（每行字段以 \u0001 分隔）。 */
    external fun nativeConfigConflicts(): String
    /** 移除一条已处理的配置冲突记录。 */
    external fun nativeRemoveConfigConflict(remoteBackup: String): Boolean
    external fun nativeMaxImageBytes(): Int
    external fun nativeMaxFileBytes(): Int
    external fun nativeDevices(): String
    external fun nativePairBegin(addr: String): Boolean
    external fun nativePairCode(): String
    external fun nativePairRespond(accept: Boolean)
    external fun nativeSetRelayAddr(configDir: String, relayAddr: String): Boolean

    /**
     * 注册 Kotlin 侧的入站 Offer 回调；传 `null` 退回到 sync-core 的
     * 自动决策（< FILE_AUTO_ACCEPT_MAX + MIME 白名单）。
     * JNI 侧会把回调包装成 `FileConfirmFn` 注入 SyncService。
     */
    external fun nativeSetFileConfirmCallback(cb: IFileConfirmCallback?): Boolean

    /** 当前挂起的最近一笔 Offer 的 transfer_id；无挂起 Offer 时返回 0。 */
    external fun nativeLatestPendingOfferId(): Long

    /** 把用户对某条 Offer 的「接受/拒绝」回送给 Rust，解除回调阻塞。 */
    external fun nativeConfirmOffer(transferId: Long, accept: Boolean)

    /**
     * 入站文件 Offer 回调：Rust 在 FileOffer 到达时同步触发。
     *
     * 约定（与 platforms/android/rimejni/src/sync_jni.rs::make_file_confirm
     * 一一对应）：返回值并非最终决策，而是「是否会稍后经
     * [nativeConfirmOffer] 汇报」——true 表示已弹通知、Rust 继续等；
     * false 表示立即拒绝。最终决策经 [nativeConfirmOffer] 单独回送。
     *
     * 该回调在 Rust 的 spawn_blocking 工作线程上被调用，不能直接
     * 触碰 Android UI；它只能 post 通知/广播/Handler，然后立即返回。
     */
    fun interface IFileConfirmCallback {
        fun onOffer(name: String, sizeBytes: Long, mime: String, peerFp: String): Boolean
    }

    @Volatile
    private var started = false

    @Volatile
    private var offerReceiverRegistered = false

    /** Notification channel 与 Intent action 都限定在本进程内。 */
    private const val CHANNEL_ID_FILE_RECEIVE = "file_receive"
    private const val ACTION_OFFER_ACCEPT = "com.shurufa.ime.action.FILE_OFFER_ACCEPT"
    private const val ACTION_OFFER_DECLINE = "com.shurufa.ime.action.FILE_OFFER_DECLINE"
    private const val EXTRA_TRANSFER_ID = "transfer_id"
    private const val EXTRA_PEER_FP = "peer_fp"
    private const val EXTRA_NAME = "name"

    /** 幂等启动同步服务（身份/配对表存 filesDir/sync）。 */
    @Synchronized
    fun ensureStarted(context: Context): Boolean {
        if (started) return true
        val dir = syncDir(context)
        started = nativeStart(dir.absolutePath, deviceName())
        if (started) {
            // 注册文件 Offer 通知的发起；通知里的广播接收器只在进程内
            // 生效（Android 14+ 对动态注册必须显式声明 exported 标志）。
            registerOfferCallback(context.applicationContext)
        }
        return started
    }

    fun deviceName(): String = Build.MODEL ?: "Android 设备"

    /** 保存中继地址；传空串或 off 可关闭。服务重启后会读取该配置。 */
    fun setRelayAddr(context: Context, relayAddr: String): Boolean =
        nativeSetRelayAddr(syncDir(context).absolutePath, relayAddr)

    /** 当前已保存的中继地址，仅用于配置页回显。 */
    fun relayAddr(context: Context): String =
        runCatching {
            File(syncDir(context), "relay.addr").takeIf { it.isFile }?.readText()?.trim().orEmpty()
        }.getOrDefault("")

    private fun syncDir(context: Context): File = File(context.filesDir, "sync").apply { mkdirs() }

    /** 与 Rust 同步核心保持一致的单张 PNG 上限。 */
    /** 发送一份配置/短语/皮肤文件到已配对电脑。 */
    fun sendConfig(context: Context, kind: String, path: File): Boolean {
        if (!ensureStarted(context)) return false
        return nativeSendConfig(kind, path.absolutePath)
    }

    fun configBackups(): List<String> =
        nativeConfigBackups().lines().filter { it.isNotBlank() }

    fun restoreConfigBackup(file: String): Boolean = nativeRestoreConfigBackup(file)

    fun configConflicts(): List<ConfigConflict> =
        nativeConfigConflicts().lines().filter { it.isNotBlank() }.mapNotNull { line ->
            val parts = line.split('\u0001')
            if (parts.size < 6) null
            else ConfigConflict(
                tsMs = parts[0].toLongOrNull() ?: 0L,
                kind = parts[1],
                name = parts[2],
                localBackup = parts[3],
                remoteBackup = parts[4],
                mergedSha256 = parts[5],
            )
        }

    fun removeConfigConflict(remoteBackup: String): Boolean =
        nativeRemoveConfigConflict(remoteBackup)

    fun maxImageBytes(): Int = nativeMaxImageBytes().takeIf { it > 0 } ?: 8 * 1024 * 1024

    fun maxFileBytes(): Int = nativeMaxFileBytes().takeIf { it > 0 } ?: 8 * 1024 * 1024

    /** 已配对设备名列表。 */
    fun deviceNames(): List<String> {
        val raw = nativeDevices()
        if (raw.isEmpty()) return emptyList()
        return raw.lines().mapNotNull { line ->
            line.split('\u0001').getOrNull(1)
        }
    }

    /**
     * 文件 v3：把本机 path 指向的文件分块流给所有协商了 file-v1 的对端。
     * 失败（文件过大 / 路径非法 / 同步未启动）返回 false。
     */
    fun sendFile(path: String): Boolean {
        if (path.isBlank()) return false
        return runCatching { nativeSendFilePath(path) }.getOrDefault(false)
    }

    /**
     * 把 Kotlin 侧的 IFileConfirmCallback 挂到 sync-core；同时注册接收
     * 「接受/拒绝」动作的本进程 BroadcastReceiver。传 null 表示禁用。
     */
    @Synchronized
    private fun registerOfferCallback(appContext: Context) {
        ensureOfferChannel(appContext)
        if (!offerReceiverRegistered) {
            val filter = IntentFilter().apply {
                addAction(ACTION_OFFER_ACCEPT)
                addAction(ACTION_OFFER_DECLINE)
            }
            val receiver = object : BroadcastReceiver() {
                override fun onReceive(context: Context, intent: Intent) {
                    val id = intent.getLongExtra(EXTRA_TRANSFER_ID, 0L)
                    if (id <= 0L) return
                    val accept = intent.action == ACTION_OFFER_ACCEPT
                    nativeConfirmOffer(id, accept)
                    // 决策送达即收掉通知，避免用户重复点击。
                    val notifId = (NOTIF_ID_BASE + (id and 0xFFFF)).toInt()
                    NotificationManagerCompat.from(context).cancel(notifId)
                }
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                appContext.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
            } else {
                @Suppress("UnspecifiedRegisterReceiverFlag")
                appContext.registerReceiver(receiver, filter)
            }
            offerReceiverRegistered = true
        }
        // 每次 ensureStarted 都覆写回调：进程重启后旧 GlobalRef 已失效，
        // 传 null 再传新实例比依赖 Once 更稳。
        nativeSetFileConfirmCallback(
            IFileConfirmCallback { name, sizeBytes, mime, peerFp ->
                onOfferArrived(appContext, name, sizeBytes, mime, peerFp)
            },
        )
    }

    /** Rust onOffer 回调的 Kotlin 实现：只发通知，立即返回 true 表示稍后回送决策。 */
    private fun onOfferArrived(
        context: Context,
        name: String,
        sizeBytes: Long,
        mime: String,
        peerFp: String,
    ): Boolean {
        val transferId = nativeLatestPendingOfferId()
        if (transferId <= 0L) return false
        postOfferNotification(context, transferId, name, sizeBytes, mime, peerFp)
        return true
    }

    private fun ensureOfferChannel(context: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val existing = manager.getNotificationChannel(CHANNEL_ID_FILE_RECEIVE)
        if (existing != null) return
        val channel = NotificationChannel(
            CHANNEL_ID_FILE_RECEIVE,
            context.getString(R.string.file_offer_notif_title),
            NotificationManager.IMPORTANCE_HIGH,
        )
        manager.createNotificationChannel(channel)
    }

    private fun postOfferNotification(
        context: Context,
        transferId: Long,
        name: String,
        sizeBytes: Long,
        @Suppress("UNUSED_PARAMETER") mime: String,
        peerFp: String,
    ) {
        ensureOfferChannel(context)
        val peerShort = if (peerFp.length > 8) peerFp.take(8) else peerFp
        val peerLabel = if (peerFp.isBlank()) {
            context.getString(R.string.file_offer_unknown)
        } else {
            peerShort
        }
        val sizeText = formatSize(sizeBytes)
        val notifId = (NOTIF_ID_BASE + (transferId and 0xFFFF)).toInt()
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val acceptIntent = PendingIntent.getBroadcast(
            context,
            notifId * 2,
            offerActionIntent(context, ACTION_OFFER_ACCEPT, transferId, peerFp, name),
            flags,
        )
        val declineIntent = PendingIntent.getBroadcast(
            context,
            notifId * 2 + 1,
            offerActionIntent(context, ACTION_OFFER_DECLINE, transferId, peerFp, name),
            flags,
        )
        val body = context.getString(R.string.file_offer_notif_body, peerLabel, name, sizeText)
        val notification = NotificationCompat.Builder(context, CHANNEL_ID_FILE_RECEIVE)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(context.getString(R.string.file_offer_notif_title))
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .addAction(0, context.getString(R.string.file_offer_accept), acceptIntent)
            .addAction(0, context.getString(R.string.file_offer_decline), declineIntent)
            .build()
        runCatching {
            NotificationManagerCompat.from(context).notify(notifId, notification)
        }
    }

    private fun offerActionIntent(
        context: Context,
        action: String,
        transferId: Long,
        peerFp: String,
        name: String,
    ): Intent = Intent(action).apply {
        // 限定到本应用，避免其它应用伪造按钮广播。
        setPackage(context.packageName)
        putExtra(EXTRA_TRANSFER_ID, transferId)
        putExtra(EXTRA_PEER_FP, peerFp)
        putExtra(EXTRA_NAME, name)
    }

    private fun formatSize(sizeBytes: Long): String {
        val mib = sizeBytes.toDouble() / (1024.0 * 1024.0)
        return if (mib >= 0.1) {
            String.format(Locale.US, "%.1f MB", mib)
        } else {
            String.format(Locale.US, "%d KB", sizeBytes / 1024L)
        }
    }

    private const val NOTIF_ID_BASE: Long = 0x5000
}
