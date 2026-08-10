package com.shurufa.ime

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
import android.app.Activity
import com.google.android.material.card.MaterialCardView
import java.io.File
import org.json.JSONObject

/**
 * 应用首页：品牌入口 + 输入法启用状态 + 功能卡片导航。
 *
 * 布局遵循项目惯例纯代码构建（零 XML 布局）；主题用 Material3 DayNight，
 * 颜色语义与 schemas/shurufa-skin.json 的品牌绿保持一致。
 */
class MainActivity : Activity() {

    private lateinit var imeStatusLine: TextView
    private lateinit var imeEnableButton: TextView
    private lateinit var aiStatusLine: TextView
    private lateinit var cardSync: MaterialCardView
    private lateinit var cardAi: MaterialCardView
    private lateinit var cardSkin: MaterialCardView

    private fun dp(v: Float): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, resources.displayMetrics).toInt()

    private fun isNight(): Boolean =
        (resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(buildContent())
    }

    override fun onResume() {
        super.onResume()
        refreshImeStatus()
        refreshAiStatus()
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
            text = "Shurufa"
            textSize = 34f
            setTypeface(typeface, Typeface.BOLD)
        })
        imeStatusLine = TextView(this).apply {
            textSize = 14f
            setPadding(0, dp(4f), 0, 0)
        }
        root.addView(imeStatusLine)
        imeEnableButton = TextView(this).apply {
            text = "去启用"
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
            "跨设备同步",
            "与电脑配对，同步剪贴板 / 图片 / 文件",
        ) {
            startActivity(Intent(this, PairActivity::class.java))
        }
        cardAi = addCard(
            root,
            "AI 帮写",
            "",
            onClick = null,
        )
        aiStatusLine = cardAi.findViewById(CARD_DESC_ID)
        cardSkin = addCard(
            root,
            "皮肤",
            "",
            onClick = null,
        )

        // About 行
        root.addView(TextView(this).apply {
            text = "Shurufa v${BuildConfig.VERSION_NAME} · 雾凇拼音方案"
            textSize = 12f
            setTextColor(if (isNight()) 0xFF8A8F99.toInt() else 0xFF888888.toInt())
            gravity = Gravity.CENTER
            setPadding(0, dp(20f), 0, 0)
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
            "输入法已启用 — 在任意输入框切换为 Shurufa 拼音"
        } else {
            "输入法未启用 — 去系统设置中打开"
        }
        imeEnableButton.visibility = if (enabled) View.GONE else View.VISIBLE
    }

    private fun refreshAiStatus() {
        // 只展示 key 是否配置，绝不读取/显示其内容
        aiStatusLine.text = if (BuildConfig.AGNES_API_KEY.isNotBlank()) {
            aiStatusLine.setTextColor(BRAND_GREEN)
            "✓ 已配置 API Key，键盘工具条可直接使用"
        } else {
            aiStatusLine.setTextColor(if (isNight()) 0xFF8A8F99.toInt() else 0xFF888888.toInt())
            "未配置 API Key — 在 gradle.properties 写 AGNES_API_KEY 重新打包"
        }
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
        row.addView(swatch("浅色", candidateBg(json, "light")))
        row.addView(swatch("深色", candidateBg(json, "dark")))
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
