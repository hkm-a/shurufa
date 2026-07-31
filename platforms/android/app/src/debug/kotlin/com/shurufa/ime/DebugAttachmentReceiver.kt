package com.shurufa.ime

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.util.Log
import java.io.ByteArrayOutputStream
import java.io.File
import kotlin.concurrent.thread

/** 为设备端自动化注入固定附件，仅编译进 Debug APK。 */
class DebugAttachmentReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION) return
        val pending = goAsync()
        thread(name = "debug-attachment-inject") {
            try {
                check(ClipStore.ensureInit(context.applicationContext)) { "历史库初始化失败" }
                ClipStore.insertImage(createPng(), SOURCE)

                val dir = File(context.filesDir, "sync/received").apply { mkdirs() }
                val file = File(dir, FILE_NAME).apply {
                    writeText("Shurufa Android 附件回退自动化测试\n", Charsets.UTF_8)
                }
                ClipStore.insertFiles(listOf(file.absolutePath), SOURCE)
                Log.i(TAG, "已注入图片与文件测试条目")
            } catch (e: Throwable) {
                Log.e(TAG, "注入测试附件失败", e)
            } finally {
                pending.finish()
            }
        }
    }

    private fun createPng(): ByteArray {
        val bitmap = Bitmap.createBitmap(160, 100, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        canvas.drawColor(Color.rgb(25, 118, 210))
        val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.WHITE
            textSize = 24f
        }
        canvas.drawText("Shurufa", 28f, 58f, paint)
        return ByteArrayOutputStream().use { output ->
            check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)) { "PNG 编码失败" }
            bitmap.recycle()
            output.toByteArray()
        }
    }

    companion object {
        private const val ACTION = "com.shurufa.ime.DEBUG_INJECT_ATTACHMENTS"
        private const val TAG = "shurufa-debug-test"
        private const val SOURCE = "自动化测试"
        private const val FILE_NAME = "shurufa-attachment-test.bin"
    }
}
