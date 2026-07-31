package com.shurufa.ime

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.provider.Settings

/** 系统启动、应用更新或切换默认输入法后恢复无感剪贴板同步服务。 */
class SyncBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action in setOf(
                Intent.ACTION_BOOT_COMPLETED,
                Intent.ACTION_MY_PACKAGE_REPLACED,
                ACTION_INPUT_METHOD_CHANGED,
            ) && isDefaultIme(context)
        ) {
            ClipboardSyncService.start(context)
        }
    }

    private fun isDefaultIme(context: Context): Boolean {
        val selected = Settings.Secure.getString(
            context.contentResolver,
            Settings.Secure.DEFAULT_INPUT_METHOD,
        ).orEmpty()
        return selected.startsWith("${context.packageName}/")
    }

    companion object {
        private const val ACTION_INPUT_METHOD_CHANGED = "android.intent.action.INPUT_METHOD_CHANGED"
    }
}
