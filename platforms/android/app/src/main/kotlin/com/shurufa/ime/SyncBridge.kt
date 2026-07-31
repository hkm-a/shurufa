package com.shurufa.ime

import android.content.Context
import android.os.Build
import java.io.File

/**
 * 同步 JNI 桥（platforms/android/rimejni/src/sync_jni.rs）的 Kotlin 声明。
 *
 * 入站条目与配对码协议以 `` 分隔字段。所有 native 调用线程安全，
 * 但 `nativePairBegin` 阻塞直至配对结束，须在后台线程调用。
 */
object SyncBridge {
    init {
        System.loadLibrary("shurufa_rime")
    }

    external fun nativeStart(configDir: String, deviceName: String): Boolean
    external fun nativePoll(): String
    external fun nativeSendClip(text: String)
    external fun nativeSendImage(png: ByteArray)
    external fun nativeSendFile(name: String, mimeType: String, data: ByteArray)
    external fun nativeMaxImageBytes(): Int
    external fun nativeMaxFileBytes(): Int
    external fun nativeDevices(): String
    external fun nativePairBegin(addr: String): Boolean
    external fun nativePairCode(): String
    external fun nativePairRespond(accept: Boolean)

    @Volatile
    private var started = false

    /** 幂等启动同步服务（身份/配对表存 filesDir/sync）。 */
    @Synchronized
    fun ensureStarted(context: Context): Boolean {
        if (started) return true
        val dir = File(context.filesDir, "sync").apply { mkdirs() }
        started = nativeStart(dir.absolutePath, deviceName())
        return started
    }

    fun deviceName(): String = Build.MODEL ?: "Android 设备"

    /** 与 Rust 同步核心保持一致的单张 PNG 上限。 */
    fun maxImageBytes(): Int = nativeMaxImageBytes().takeIf { it > 0 } ?: 8 * 1024 * 1024

    fun maxFileBytes(): Int = nativeMaxFileBytes().takeIf { it > 0 } ?: 8 * 1024 * 1024

    /** 已配对设备名列表。 */
    fun deviceNames(): List<String> {
        val raw = nativeDevices()
        if (raw.isEmpty()) return emptyList()
        return raw.lines().mapNotNull { line ->
            line.split('').getOrNull(1)
        }
    }
}
