package com.shurufa.ime

import android.app.Activity
import android.os.Bundle

/**
 * 仅 Debug APK 使用（模拟器验收）：前台启动、读取 extras 驱动 IME。
 * extras：kind ∈ {panel, scheme, type}；value 为面板名 / 方案 id / 键序。
 */
class DebugUiActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val kind = intent.getStringExtra("kind").orEmpty()
        val value = intent.getStringExtra("value")
        val service = ShurufaImeService.debugInstance
        if (service != null) {
            service.handleDebugCommand(kind, value)
        }
        finish()
    }
}
