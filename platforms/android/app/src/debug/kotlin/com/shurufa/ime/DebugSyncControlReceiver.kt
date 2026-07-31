package com.shurufa.ime

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/** 仅 Debug APK 使用：自动化导入配对表后显式恢复后台同步服务。 */
class DebugSyncControlReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_START) return
        ClipboardSyncService.start(context)
        Log.i(TAG, "自动化已请求启动后台同步服务")
    }

    companion object {
        private const val ACTION_START = "com.shurufa.ime.DEBUG_START_SYNC"
        private const val TAG = "shurufa-sync"
    }
}
