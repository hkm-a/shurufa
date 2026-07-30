package com.shurufa.ime

import android.app.Activity
import android.graphics.Color
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import kotlin.concurrent.thread

/**
 * 设备配对与管理页（应用图标入口）。
 *
 * 手机作发起端：输入电脑局域网 IP → 连接后本机与电脑各显示同一
 * 六位确认码 → 用户比对一致后双方放行。纯代码构建 UI，不引入
 * appcompat 等依赖，与键盘服务保持零第三方依赖风格。
 */
class PairActivity : Activity() {

    private lateinit var status: TextView
    private lateinit var deviceList: TextView
    private lateinit var ipInput: EditText
    private lateinit var pairButton: Button
    private lateinit var codeArea: LinearLayout
    private lateinit var codeText: TextView

    private val main = Handler(Looper.getMainLooper())
    private var polling = false

    private fun dp(v: Int): Int = (v * resources.displayMetrics.density).toInt()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        SyncBridge.ensureStarted(applicationContext)

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.WHITE)
            setPadding(dp(24), dp(32), dp(24), dp(24))
        }

        root.addView(title("剪贴板同步"))
        root.addView(hint("与电脑同一 Wi-Fi 下，输入电脑显示的 IP 地址后配对。\n电脑端先运行 shurufa-host，用 devices 查看 IP。"))

        ipInput = EditText(this).apply {
            hint = "电脑 IP，如 192.168.1.8"
            setSingleLine()
        }
        root.addView(ipInput)

        pairButton = Button(this).apply {
            text = "配对"
            setOnClickListener { startPairing() }
        }
        root.addView(pairButton)

        status = hint("")
        root.addView(status)

        // 确认码区（默认隐藏）
        codeArea = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            setPadding(0, dp(16), 0, 0)
        }
        codeArea.addView(hint("确认电脑屏幕显示相同的确认码："))
        codeText = TextView(this).apply {
            textSize = 32f
            setTextColor(Color.parseColor("#B06030"))
            gravity = Gravity.CENTER
            letterSpacing = 0.2f
        }
        codeArea.addView(codeText)
        val btnRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        btnRow.addView(Button(this).apply {
            text = "一致，确认"
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
            setOnClickListener { respond(true) }
        })
        btnRow.addView(Button(this).apply {
            text = "取消"
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
            setOnClickListener { respond(false) }
        })
        codeArea.addView(btnRow)
        root.addView(codeArea)

        root.addView(title("已配对设备").apply { textSize = 16f; setPadding(0, dp(24), 0, dp(8)) })
        deviceList = hint("")
        root.addView(deviceList)

        setContentView(root)
        refreshDevices()
    }

    private fun startPairing() {
        val ip = ipInput.text.toString().trim()
        if (ip.isEmpty()) {
            Toast.makeText(this, "请输入电脑 IP", Toast.LENGTH_SHORT).show()
            return
        }
        pairButton.isEnabled = false
        status.text = "正在连接 $ip …"
        startCodePolling()
        thread(name = "pair") {
            val ok = SyncBridge.nativePairBegin(ip)
            main.post {
                polling = false
                codeArea.visibility = View.GONE
                pairButton.isEnabled = true
                status.text = if (ok) "配对成功" else "配对失败：检查 IP、网络或对方是否确认"
                refreshDevices()
            }
        }
    }

    /** 轮询确认码：一旦 native 侧产生待确认码就展示。 */
    private fun startCodePolling() {
        polling = true
        val tick = object : Runnable {
            override fun run() {
                if (!polling) return
                val raw = SyncBridge.nativePairCode()
                if (raw.isNotEmpty()) {
                    val code = raw.split('').firstOrNull().orEmpty()
                    codeText.text = code
                    codeArea.visibility = View.VISIBLE
                    status.text = "等待双方确认…"
                }
                main.postDelayed(this, 400)
            }
        }
        main.postDelayed(tick, 400)
    }

    private fun respond(accept: Boolean) {
        SyncBridge.nativePairRespond(accept)
        codeArea.visibility = View.GONE
        if (!accept) {
            polling = false
            status.text = "已取消"
        }
    }

    private fun refreshDevices() {
        val names = SyncBridge.deviceNames()
        deviceList.text = if (names.isEmpty()) "（暂无）" else names.joinToString("\n") { "· $it" }
    }

    override fun onDestroy() {
        super.onDestroy()
        polling = false
    }

    // ---- 纯代码控件工厂 ----

    private fun title(t: String) = TextView(this).apply {
        text = t
        textSize = 22f
        setTextColor(Color.parseColor("#202020"))
        setPadding(0, 0, 0, dp(12))
    }

    private fun hint(t: String) = TextView(this).apply {
        text = t
        textSize = 14f
        setTextColor(Color.parseColor("#606060"))
        setPadding(0, dp(6), 0, dp(6))
    }
}
