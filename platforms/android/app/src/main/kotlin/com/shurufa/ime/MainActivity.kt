package com.shurufa.ime

import android.app.Activity
import android.content.Intent
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.provider.Settings
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.inputmethod.InputMethodManager
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import com.google.android.material.card.MaterialCardView
import java.io.File
import org.json.JSONObject

/**
 * 应用首页：品牌入口 + 输入法启用状态 + 功能卡片导航。
 *
 * 布局遵循项目惯例纯代码构建（零 XML 布局）；主题用 Material3 DayNight，
 * 颜色语义与 schemas/shurufa-skin.json 的品牌绿保持一致。
 *
 * 首次启动（onboarding_done=false）时立即跳到 OnboardingActivity 并 finish，
 * 保证返回栈干净；版本页脚长按可重新查看引导。
 */
class MainActivity : Activity() {

    private lateinit var imeStatusLine: TextView
    private lateinit var imeEnableButton: TextView
    private lateinit var aiStatusLine: TextView
    private lateinit var schemeStatusLine: TextView
    private lateinit var cardSync: MaterialCardView
    private lateinit var cardAi: MaterialCardView
    private lateinit var cardScheme: MaterialCardView
    private lateinit var cardSkin: MaterialCardView

    private fun dp(v: Float): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, resources.displayMetrics).toInt()

    private fun isNight(): Boolean =
        (resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // 未完成首次引导：直接进引导并结束自身，保证返回键不会回到半初始化首页。
        if (!OnboardingActivity.isDone(this)) {
            startActivity(Intent(this, OnboardingActivity::class.java))
            finish()
            return
        }
        setContentView(buildContent())
    }

    override fun onResume() {
        super.onResume()
        refreshImeStatus()
        refreshAiStatus()
        refreshSchemeStatus()
        refreshSkinPreview()
    }

    // ---------- 布局 ----------

    private fun buildContent(): View {
        val scroll = ScrollView(this)
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20f), dp(28f), dp(20f), dp(24f))
        }
        scroll.addView(root)

        // 品牌标题
        root.addView(TextView(this).apply {
            text = getString(R.string.main_brand_title)
            textSize = 34f
            setTypeface(typeface, Typeface.BOLD)
        })
        imeStatusLine = TextView(this).apply {
            textSize = 14f
            setPadding(0, dp(4f), 0, 0)
        }
        root.addView(imeStatusLine)
        imeEnableButton = TextView(this).apply {
            text = getString(R.string.main_ime_enable_button)
            textSize = 14f
            gravity = Gravity.CENTER
            setTextColor(0xFFFFFFFF.toInt())
            background = GradientDrawable().apply {
                setColor(BRAND_GREEN)
                cornerRadius = dp(8f).toFloat()
            }
            setOnClickListener {
                startActivity(Intent(Settings.ACTION_INPUT_METHOD_SETTINGS))
            }
        }
        root.addView(
            imeEnableButton,
            LinearLayout.LayoutParams(dp(96f), dp(38f)).apply { topMargin = dp(10f) },
        )

        // 功能卡片
        cardSync = addCard(
            root,
            getString(R.string.main_card_sync_title),
            getString(R.string.main_card_sync_desc),
        ) {
            startActivity(Intent(this, PairActivity::class.java))
        }
        cardAi = addCard(
            root,
            getString(R.string.main_card_ai_title),
            "",
            onClick = null,
        )
        aiStatusLine = cardAi.findViewById(CARD_DESC_ID)
        // wave 4：与 AI 卡并列的只读「当前输入方案」卡；
        // 渲染态在 onResume → refreshSchemeStatus() 从 SharedPreferences 回填。
        cardScheme = addCard(
            root,
            getString(R.string.main_card_scheme_title),
            "",
            onClick = null,
        )
        schemeStatusLine = cardScheme.findViewById(CARD_DESC_ID)
        cardSkin = addCard(
            root,
            getString(R.string.main_card_skin_title),
            "",
            onClick = null,
        )
        // M-A5-2 设置极简：键盘设置与小秘籍卡片（搜狗 20.3.1「设置极简」）
        addCard(
            root,
            getString(R.string.main_card_keyboard_title),
            getString(R.string.main_card_keyboard_desc),
        ) {
            Toast.makeText(
                this,
                R.string.main_card_keyboard_toast,
                Toast.LENGTH_LONG,
            ).show()
        }

        // About 行（长按：重置引导标记并重新打开向导）
        root.addView(TextView(this).apply {
            text = getString(R.string.main_footer_version, BuildConfig.VERSION_NAME)
            textSize = 12f
            setTextColor(if (isNight()) 0xFF8A8F99.toInt() else 0xFF888888.toInt())
            gravity = Gravity.CENTER
            setPadding(0, dp(20f), 0, 0)
            setOnLongClickListener {
                OnboardingActivity.reset(this@MainActivity)
                Toast.makeText(
                    this@MainActivity,
                    R.string.main_footer_replay_onboarding,
                    Toast.LENGTH_SHORT,
                ).show()
                startActivity(Intent(this@MainActivity, OnboardingActivity::class.java))
                finish()
                true
            }
        })

        return scroll
    }

    private fun addCard(
        parent: LinearLayout,
        title: String,
        desc: String,
        onClick: (() -> Unit)?,
    ): MaterialCardView {
        val card = MaterialCardView(this).apply {
            radius = dp(12f).toFloat()
            cardElevation = dp(1f).toFloat()
            useCompatPadding = true
            if (onClick != null) {
                isClickable = true
                isFocusable = true
                setOnClickListener { onClick() }
            }
        }
        val inner = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16f), dp(14f), dp(16f), dp(14f))
        }
        inner.addView(TextView(this).apply {
            text = title
            textSize = 17f
            setTypeface(typeface, Typeface.BOLD)
        })
        inner.addView(TextView(this).apply {
            id = CARD_DESC_ID
            text = desc
            textSize = 13f
            setPadding(0, dp(6f), 0, 0)
        })
        card.addView(inner)
        parent.addView(
            card,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
            ).apply { topMargin = dp(14f) },
        )
        return card
    }

    // ---------- 状态刷新 ----------

    /** 自己是否在系统已启用输入法列表中。 */
    private fun isImeEnabled(): Boolean {
        val imm = getSystemService(InputMethodManager::class.java) ?: return false
        return imm.enabledInputMethodList.any { it.packageName == packageName }
    }

    private fun refreshImeStatus() {
        val enabled = isImeEnabled()
        imeStatusLine.text = if (enabled) {
            getString(R.string.main_ime_status_enabled)
        } else {
            getString(R.string.main_ime_status_disabled)
        }
        imeEnableButton.visibility = if (enabled) View.GONE else View.VISIBLE
    }

    private fun refreshAiStatus() {
        // 只展示 key 是否配置，绝不读取/显示其内容
        aiStatusLine.text = if (BuildConfig.AGNES_API_KEY.isNotBlank()) {
            aiStatusLine.setTextColor(BRAND_GREEN)
            getString(R.string.main_ai_status_configured)
        } else {
            aiStatusLine.setTextColor(if (isNight()) 0xFF8A8F99.toInt() else 0xFF888888.toInt())
            getString(R.string.main_ai_status_missing)
        }
    }

    /** 只读显示当前方案；点击交互走键盘工具条上的"方案" chip。 */
    private fun refreshSchemeStatus() {
        val id = getSharedPreferences("shurufa", MODE_PRIVATE)
            .getString("shurufa_input_scheme", null)
            ?: runCatching { RimeBridge.nativeGetInputScheme() }.getOrNull()
            ?: "pinyin"
        val labelRes = when (id) {
            "double_pinyin" -> R.string.scheme_double_pinyin
            "wubi" -> R.string.scheme_wubi
            "cangjie" -> R.string.scheme_cangjie
            else -> R.string.scheme_pinyin
        }
        val label = getString(labelRes)
        schemeStatusLine.text = label
        val isDefault = id == "pinyin"
        schemeStatusLine.setTextColor(
            if (isDefault) {
                if (isNight()) 0xFF8A8F99.toInt() else 0xFF888888.toInt()
            } else {
                // 非默认方案（preview 双拼/五笔/仓颉）时给品牌绿强调，便于一眼看出"当前不是默认"
                BRAND_GREEN
            }
        )
    }

    /** 读取 filesDir 覆盖件 / 打包 skin.json，预览 light/dark 两个 candidate.background。 */
    private fun refreshSkinPreview() {
        val container = (cardSkin.getChildAt(0) as? LinearLayout) ?: return
        // 清掉上次 onResume 加的色块行，只保留标题与描述
        while (container.childCount > 2) container.removeViewAt(2)

        val json = loadSkinJson()
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, dp(10f), 0, 0)
        }
        row.addView(swatch(getString(R.string.main_skin_light), candidateBg(json, "light")))
        row.addView(swatch(getString(R.string.main_skin_dark), candidateBg(json, "dark")))
        container.addView(row)
    }

    private fun swatch(label: String, color: Int): View {
        val wrap = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            setPadding(0, 0, dp(18f), 0)
        }
        wrap.addView(View(this).apply {
            background = GradientDrawable().apply {
                setColor(color)
                cornerRadius = dp(6f).toFloat()
                setStroke(dp(1f), if (isNight()) 0xFF4A5059.toInt() else 0xFFD9D9D9.toInt())
            }
        }, LinearLayout.LayoutParams(dp(56f), dp(36f)))
        wrap.addView(TextView(this).apply {
            text = label
            textSize = 11f
            setPadding(0, dp(4f), 0, 0)
        })
        return wrap
    }

    private fun loadSkinJson(): JSONObject? {
        val override = File(filesDir, "shurufa-skin.json")
        return try {
            val text = if (override.isFile) {
                override.readText()
            } else {
                assets.open("schemas/shurufa-skin.json").bufferedReader().use { it.readText() }
            }
            JSONObject(text)
        } catch (_: Exception) {
            null
        }
    }

    private fun candidateBg(root: JSONObject?, variant: String): Int {
        val fallback = if (variant == "dark") 0xFF1E2126.toInt() else 0xFFFFFFFF.toInt()
        val hex = root
            ?.optJSONObject(variant)
            ?.optJSONObject("candidate")
            ?.optString("background", "")
            .orEmpty()
        if (!hex.matches(Regex("^#[0-9A-Fa-f]{6}([0-9A-Fa-f]{2})?$"))) return fallback
        return try {
            val rgb = hex.substring(1)
            (if (rgb.length == 6) "FF$rgb" else rgb).toLong(16).toInt()
        } catch (_: NumberFormatException) {
            fallback
        }
    }

    companion object {
        private const val CARD_DESC_ID = 0x5F0A11
        private val BRAND_GREEN = 0xFF1B9E77.toInt()
    }
}
