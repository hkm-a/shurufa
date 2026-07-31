package com.shurufa.attachmenttest

import android.app.Activity
import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView

/** 验证输入法通过系统 ACTION_SEND 公开入口交付的附件能否被目标应用实际读取。 */
class ShareReceiverActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val resultView = TextView(this).apply {
            textSize = 15f
            setPadding(dp(18), dp(24), dp(18), dp(16))
        }
        val previewView = ImageView(this).apply {
            adjustViewBounds = true
            scaleType = ImageView.ScaleType.FIT_CENTER
        }
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(resultView)
            addView(previewView, LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
            ))
        })

        val uri = sharedUri(intent)
        if (intent.action != Intent.ACTION_SEND || uri == null) {
            val message = "ACTION_SEND 中没有附件 URI"
            resultView.text = message
            Log.e(TAG, message)
            return
        }

        try {
            val bytes = contentResolver.openInputStream(uri)?.use { it.readBytes() }
                ?: error("ContentResolver 返回空输入流")
            val resolvedMimeType = contentResolver.getType(uri)
            val result = AttachmentReadResult(
                source = "ACTION_SEND",
                label = uri.lastPathSegment.orEmpty(),
                uri = uri.toString(),
                declaredMimeTypes = listOfNotNull(intent.type),
                resolvedMimeType = resolvedMimeType,
                byteCount = bytes.size.toLong(),
                error = null,
            )
            val displayText = result.asDisplayText()
            resultView.text = displayText
            if (resolvedMimeType?.startsWith("image/") == true) {
                previewView.setImageBitmap(BitmapFactory.decodeByteArray(bytes, 0, bytes.size))
            }
            Log.i(TAG, displayText)
        } catch (e: Throwable) {
            val result = AttachmentReadResult(
                source = "ACTION_SEND",
                label = uri.lastPathSegment.orEmpty(),
                uri = uri.toString(),
                declaredMimeTypes = listOfNotNull(intent.type),
                resolvedMimeType = contentResolver.getType(uri),
                byteCount = null,
                error = e.message ?: e.javaClass.simpleName,
            )
            resultView.text = result.asDisplayText()
            Log.e(TAG, result.asDisplayText(), e)
        }
    }

    @Suppress("DEPRECATION")
    private fun sharedUri(intent: Intent): Uri? {
        val stream = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            intent.getParcelableExtra(Intent.EXTRA_STREAM)
        }
        return stream ?: intent.clipData?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.uri
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

    companion object {
        private const val TAG = "shurufa-attachment-test"
    }
}
