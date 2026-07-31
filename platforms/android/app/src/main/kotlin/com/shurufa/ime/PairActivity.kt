package com.shurufa.ime

import android.app.Activity
import android.content.Context
import android.graphics.Color
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
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
import java.util.concurrent.ConcurrentLinkedQueue
import kotlin.concurrent.thread

/**
 * 设备配对与管理页（应用图标入口）。
 *
 * 同一 Wi-Fi 下用 NsdManager 自动发现电脑（_shurufa._tcp），列表点击
 * 即配对，无需手输 IP；手输 IP 作为发现失败时的兜底。配对确认沿用
 * 六位码人眼比对。纯代码构建 UI，零第三方依赖。
 */
class PairActivity : Activity() {

    private lateinit var status: TextView
    private lateinit var deviceList: TextView
    private lateinit var discoveredBox: LinearLayout
    private lateinit var ipInput: EditText
    private lateinit var pairButton: Button
    private lateinit var codeArea: LinearLayout
    private lateinit var codeText: TextView

    private val main = Handler(Looper.getMainLooper())
    private var polling = false

    private var nsd: NsdManager? = null
    private var discoveryListener: NsdManager.DiscoveryListener? = null
    /// 待解析的服务名队列（NsdManager 经典 API 一次只能 resolve 一个）
    private val resolveQueue = ConcurrentLinkedQueue<NsdServiceInfo>()
    private var resolving = false
    /// 已发现设备：展示名 → "ip:port"
    private val found = LinkedHashMap<String, String>()

    private fun dp(v: Int): Int = (v * resources.displayMetrics.density).toInt()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ClipboardSyncService.start(applicationContext)
        SyncBridge.ensureStarted(applicationContext)

        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.WHITE)
            setPadding(dp(24), dp(32), dp(24), dp(24))
        }

        root.addView(title("剪贴板同步"))
        root.addView(hint("版本 ${BuildConfig.VERSION_NAME}（${BuildConfig.VERSION_CODE}）"))
        root.addView(hint("与电脑同一 Wi-Fi。电脑端运行 shurufa-host 后，下方会自动出现电脑；点击即可配对。"))

        root.addView(subtitle("发现的电脑"))
        discoveredBox = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        root.addView(discoveredBox)
        renderDiscovered()

        root.addView(subtitle("手动输入（自动发现失败时）"))
        ipInput = EditText(this).apply {
            hint = "电脑 IP，如 192.168.1.8"
            setSingleLine()
        }
        root.addView(ipInput)
        pairButton = Button(this).apply {
            text = "配对"
            setOnClickListener { startPairing(ipInput.text.toString().trim()) }
        }
        root.addView(pairButton)

        status = hint("")
        root.addView(status)

        codeArea = buildCodeArea()
        root.addView(codeArea)

        root.addView(subtitle("已配对设备"))
        deviceList = hint("")
        root.addView(deviceList)

        setContentView(root)
        refreshDevices()
    }

    override fun onStart() {
        super.onStart()
        startDiscovery()
    }

    override fun onStop() {
        super.onStop()
        stopDiscovery()
    }

    // ---------- 自动发现 ----------

    private fun startDiscovery() {
        val manager = getSystemService(Context.NSD_SERVICE) as? NsdManager ?: return
        nsd = manager
        val listener = object : NsdManager.DiscoveryListener {
            override fun onStartDiscoveryFailed(t: String?, e: Int) {}
            override fun onStopDiscoveryFailed(t: String?, e: Int) {}
            override fun onDiscoveryStarted(t: String?) {}
            override fun onDiscoveryStopped(t: String?) {}
            override fun onServiceFound(info: NsdServiceInfo) {
                resolveQueue.add(info)
                pumpResolve()
            }
            override fun onServiceLost(info: NsdServiceInfo) {
                main.post {
                    found.remove(info.serviceName)
                    renderDiscovered()
                }
            }
        }
        discoveryListener = listener
        try {
            manager.discoverServices("_shurufa._tcp.", NsdManager.PROTOCOL_DNS_SD, listener)
        } catch (e: Exception) {
            // 发现不可用时退回手输 IP
        }
    }

    private fun stopDiscovery() {
        val l = discoveryListener ?: return
        try {
            nsd?.stopServiceDiscovery(l)
        } catch (e: Exception) {
        }
        discoveryListener = null
    }

    /// 串行 resolve：经典 NsdManager 不允许并发 resolveService。
    private fun pumpResolve() {
        if (resolving) return
        val info = resolveQueue.poll() ?: return
        resolving = true
        val manager = nsd ?: return
        manager.resolveService(info, object : NsdManager.ResolveListener {
            override fun onResolveFailed(i: NsdServiceInfo?, e: Int) {
                resolving = false
                pumpResolve()
            }
            override fun onServiceResolved(resolved: NsdServiceInfo) {
                val host = resolved.host?.hostAddress
                if (host != null) {
                    val name = attrName(resolved) ?: resolved.serviceName
                    main.post {
                        found[resolved.serviceName] = "$host:${resolved.port}"
                        foundNames[resolved.serviceName] = name
                        renderDiscovered()
                    }
                }
                resolving = false
                pumpResolve()
            }
        })
    }

    private val foundNames = LinkedHashMap<String, String>()

    private fun attrName(info: NsdServiceInfo): String? {
        val bytes = info.attributes?.get("name") ?: return null
        return String(bytes, Charsets.UTF_8)
    }

    private fun renderDiscovered() {
        discoveredBox.removeAllViews()
        if (found.isEmpty()) {
            discoveredBox.addView(hint("正在搜索…（确保电脑端已运行）"))
            return
        }
        for ((serviceName, addr) in found) {
            val label = foundNames[serviceName] ?: serviceName
            discoveredBox.addView(Button(this).apply {
                text = "$label  ($addr)"
                setOnClickListener { startPairing(addr) }
            })
        }
    }

    // ---------- 配对 ----------

    private fun startPairing(addr: String) {
        if (addr.isEmpty()) {
            Toast.makeText(this, "请选择或输入电脑地址", Toast.LENGTH_SHORT).show()
            return
        }
        pairButton.isEnabled = false
        status.text = "正在连接 $addr …"
        startCodePolling()
        thread(name = "pair") {
            val ok = SyncBridge.nativePairBegin(addr)
            main.post {
                polling = false
                codeArea.visibility = View.GONE
                pairButton.isEnabled = true
                status.text = if (ok) "配对成功" else "配对失败：检查网络或对方是否确认"
                refreshDevices()
            }
        }
    }

    private fun startCodePolling() {
        polling = true
        val tick = object : Runnable {
            override fun run() {
                if (!polling) return
                val raw = SyncBridge.nativePairCode()
                if (raw.isNotEmpty()) {
                    codeText.text = raw.substringBefore(FIELD)
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

    // ---------- 控件工厂 ----------

    private fun buildCodeArea(): LinearLayout {
        val area = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            setPadding(0, dp(16), 0, 0)
        }
        area.addView(hint("确认电脑屏幕显示相同的确认码："))
        codeText = TextView(this).apply {
            textSize = 32f
            setTextColor(Color.parseColor("#B06030"))
            gravity = Gravity.CENTER
            letterSpacing = 0.2f
        }
        area.addView(codeText)
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
        area.addView(btnRow)
        return area
    }

    private fun title(t: String) = TextView(this).apply {
        text = t
        textSize = 22f
        setTextColor(Color.parseColor("#202020"))
        setPadding(0, 0, 0, dp(12))
    }

    private fun subtitle(t: String) = TextView(this).apply {
        text = t
        textSize = 16f
        setTextColor(Color.parseColor("#202020"))
        setPadding(0, dp(20), 0, dp(6))
    }

    private fun hint(t: String) = TextView(this).apply {
        text = t
        textSize = 14f
        setTextColor(Color.parseColor("#606060"))
        setPadding(0, dp(6), 0, dp(6))
    }

    companion object {
        private const val FIELD = "\u0001"
    }
}
