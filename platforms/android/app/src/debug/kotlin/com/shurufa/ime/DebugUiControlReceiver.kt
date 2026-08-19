package com.shurufa.ime

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * 仅 Debug APK 使用：自动化驱动 IME 面板/方案/键序（模拟器验收）。
 * kind ∈ {panel, scheme, type}；value 分别为面板名 / 方案 id / 键序。
 */
class DebugUiControlReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION) return
        val service = ShurufaImeService.debugInstance ?: run {
            Log.i(TAG, "IME 服务未运行，忽略调试命令")
            return
        }
        service.handleDebugCommand(
            intent.getStringExtra("kind").orEmpty(),
            intent.getStringExtra("value"),
        )
        Log.i(TAG, "debug ui command received")
    }

    companion object {
        const val ACTION = "com.shurufa.ime.DEBUG_UI"
        private const val TAG = "shurufa-debug-ui"
    }
}
