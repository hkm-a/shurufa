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
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import java.io.File
import java.util.concurrent.ConcurrentLinkedQueue
import kotlin.concurrent.thread

/**
 * 设备配对与管理页（应用图标入口）。
 *
 * 同一 Wi-Fi 下用 NsdManager 自动发现电脑（_shurufa._tcp），列表点击
 * 即配对，无需手输 IP；手输 IP 作为发现失败时的兜底。配对确认沿用
 * 八位码人眼比对。纯代码构建 UI，零第三方依赖。
 */
class PairActivity : Activity() {

    private lateinit var status: TextView
    private lateinit var deviceList: TextView
    private lateinit var discoveredBox: LinearLayout
    private lateinit var ipInput: EditText
    private lateinit var pairButton: Button
    private lateinit var relayInput: EditText
    private lateinit var dictionaryInput: EditText
    private lateinit var codeArea: LinearLayout
    private lateinit var codeText: TextView

    private val main = Handler(Looper.getMainLooper())
    private var polling = false
    /// 配对网络线程引用：onDestroy 时 interrupt + join，避免页面销毁后仍回写已 detach 的 UI
    private var pairThread: Thread? = null
    /// 配对整体超时：网络阻塞路径不会停在 Rust 的 is_pairing_idle，JNI 层独立设上限
    private var pairExpired = java.util.concurrent.atomic.AtomicBoolean(false)
    private var pairGeneration = 0
    /// 记录是否已把超时兜底 post 到主线程，避免与 onDestroy 交错出现两条"超时"提示
    private var pairTimeoutPosted = false

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

        root.addView(subtitle("自托管中继（跨网段可选）"))
        root.addView(hint("已配对设备在直连失败时会回退到此地址，如 relay.example.com:48633。首次配对仍需直连。"))
        relayInput = EditText(this).apply {
            hint = "中继主机:端口；留空或 off 关闭"
            setSingleLine()
            setText(SyncBridge.relayAddr(applicationContext))
        }
        root.addView(relayInput)
        root.addView(Button(this).apply {
            text = "保存中继配置"
            setOnClickListener { saveRelay() }
        })

        root.addView(subtitle("云词库更新（可选）"))
        root.addView(hint("默认 rime-ice 使用已验证的雾凇拼音稳定词典；也可填写自托管 HTTPS 清单。所有词典会按 SHA-256 校验；更新后请完全重启输入法。"))
        dictionaryInput = EditText(this).apply {
            hint = "rime-ice 或 https://dict.example.com/manifest.json"
            setSingleLine()
            setText(CloudDictionaryUpdater.source(applicationContext))
        }
        root.addView(dictionaryInput)
        root.addView(Button(this).apply {
            text = "更新云词库"
            setOnClickListener { updateCloudDictionary() }
        })

        status = hint("")
        root.addView(status)

        codeArea = buildCodeArea()
        root.addView(codeArea)

        root.addView(subtitle("已配对设备"))
        deviceList = hint("")
        root.addView(deviceList)

        root.addView(Button(this).apply {
            text = "同步配置到电脑"
            setOnClickListener { sendConfigsToPc() }
        })

        setContentView(ScrollView(this).apply { addView(root) })
        refreshDevices()
        // 入站配对也要能显示确认码：Windows 发起、本机接收时不再“静默拒绝”。
        startCodePolling(0)
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
        // 前一次配对线程若还活着（用户重复点击），先打断再重开
        pairThread?.interrupt()
        val generation = ++pairGeneration
        pairExpired.set(false)
        pairTimeoutPosted = false
        pairButton.isEnabled = false
        status.text = "正在连接 $addr …"
        startCodePolling(generation)

        val worker = thread(name = "pair", isDaemon = true) {
            val ok = try {
                SyncBridge.nativePairBegin(addr)
            } catch (e: InterruptedException) {
                false // 被 onDestroy/超时打断
            } catch (e: Throwable) {
                android.util.Log.e("shurufa", "pairBegin 异常", e)
                false
            }
            main.post {
                // 已过期（超时已提示）或已被新一轮配对所取代时，忽略本次结果
                if (pairExpired.get() || generation != pairGeneration) return@post
                finishPairingUi(ok)
            }
        }
        pairThread = worker

        // 配对上限 150s（Rust TLS 探测 ~120s + 等待用户确认 ~30s）：
        // nativePairBegin 内部 error 路径不会归零 is_pairing_idle 的 180s 看门狗，
        // UI 层必须有自己的超时，否则用户会看到"正在连接…"无限转圈。
        main.postDelayed({
            if (generation == pairGeneration && pairThread?.isAlive == true) {
                pairExpired.set(true)
                pairTimeoutPosted = true
                pairThread?.interrupt()
                finishPairingUi(ok = false, timeout = true)
            }
        }, PAIR_TIMEOUT_MS)
    }

    /** 归位配对结束的 UI 状态（成功或失败都走这里）。 */
    private fun finishPairingUi(ok: Boolean, timeout: Boolean = false) {
        polling = false
        codeArea.visibility = View.GONE
        pairButton.isEnabled = true
        if (pairThread?.isAlive == false) pairThread = null
        status.text = when {
            timeout -> "配对超时：网络不通或对端未在 2.5 分钟内确认，请检查后重试"
            ok -> "配对成功"
            else -> "配对失败：检查网络或对方是否确认"
        }
        refreshDevices()
    }

    private fun saveRelay() {
        val value = relayInput.text.toString().trim()
        if (!SyncBridge.setRelayAddr(applicationContext, value)) {
            Toast.makeText(this, "中继地址无效，应为主机名或 IP 加端口", Toast.LENGTH_SHORT).show()
            return
        }
        val message = if (value.isEmpty() || value.equals("off", ignoreCase = true)) {
            "已关闭中继。请完全重启输入法后生效。"
        } else {
            "已保存中继。请完全重启输入法后生效。"
        }
        Toast.makeText(this, message, Toast.LENGTH_LONG).show()
    }

    private fun updateCloudDictionary() {
        val url = dictionaryInput.text.toString().trim()
        if (!url.equals("rime-ice", ignoreCase = true) && !url.startsWith("https://")) {
            Toast.makeText(this, "请输入 rime-ice 或 HTTPS 词库清单地址", Toast.LENGTH_SHORT).show()
            return
        }
        status.text = "正在下载并校验云词库…"
        thread(name = "cloud-dict-update") {
            val result = CloudDictionaryUpdater.update(applicationContext, url)
            main.post {
                result.fold(
                    onSuccess = { revision ->
                        status.text = "云词库已更新到 $revision，请完全重启输入法后生效"
                    },
                    onFailure = { error ->
                        status.text = "云词库更新失败：${error.message ?: "未知错误"}"
                    },
                )
            }
        }
    }

    private fun startCodePolling(generation: Int) {
        polling = true
        val tick = object : Runnable {
            override fun run() {
                if (!polling || generation != pairGeneration) return
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
            // 拒绝时不打断线程：等待 nativePairBegin 走到对端不确认的自然错误返回，
            // 由 UI 层超时（PAIR_TIMEOUT_MS）兜底；interrupt 只作为 onDestroy 的最后手段。
            status.text = "已取消"
        }
    }

    private fun sendConfigsToPc() {
        val dir = filesDir
        val candidates = listOf(
            Pair("custom_phrase", File(dir, "rime/custom_phrase.txt")),
            Pair("skin", File(dir, "shurufa-skin.json")),
            Pair("options", File(dir, "options.json")),
        )
        val sent = candidates.count { (kind, file) ->
            file.isFile && SyncBridge.sendConfig(this, kind, file)
        }
        Toast.makeText(this, "已发送 $sent 份配置到电脑", Toast.LENGTH_SHORT).show()
    }

    private fun refreshDevices() {
        val names = SyncBridge.deviceNames()
        deviceList.text = if (names.isEmpty()) "（暂无）" else names.joinToString("\n") { "· $it" }
    }

    override fun onDestroy() {
        // 停轮询 + 打断配对线程：后台 nativePairBegin 仍在 TCP 阻塞时，
        // interrupt 让自身 catch (InterruptedException) 立即返回，再由 join 等待真正退出，
        // 防止 finish() 后线程仍持 Activity 引用并更新已 detach 的视图。
        polling = false
        main.removeCallbacksAndMessages(null)
        pairThread?.let { t ->
            if (t.isAlive) {
                t.interrupt()
                try {
                    t.join(500)
                } catch (e: InterruptedException) {
                    Thread.currentThread().interrupt()
                }
            }
        }
        pairThread = null
        super.onDestroy()
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
        /// 配对总超时：Rust 直连重试 6 次 × ~2s ≈ 12s；TLS 与等待用户确认给足余量到 2.5 分钟。
        /// 超过则 UI 主动标超时并 interrupt 网络线程（JNI 侧下次返回时被丢弃）。
        private const val PAIR_TIMEOUT_MS = 150_000L
    }
}
