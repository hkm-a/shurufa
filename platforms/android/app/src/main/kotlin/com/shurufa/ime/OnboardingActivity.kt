package com.shurufa.ime

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.inputmethod.InputMethodManager
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.ViewFlipper

/**
 * 首次启用引导：4 页向导（欢迎 → 启用输入法 → 设为默认 → 完成）。
 *
 * 纯代码布局 + ViewFlipper 换页，不引入任何新依赖。启用 / 默认两页用
 * Handler 每 500ms 轮询系统状态，达成后先显示打勾提示再自动翻页；
 * 轮询只在对应页面可见时进行（onPause / 翻页都会停止）。
 *
 * 完成状态持久化在 SharedPreferences（onboarding_done），MainActivity
 * 根据它决定是否启动本向导。
 */
class OnboardingActivity : Activity() {

    private lateinit var flipper: ViewFlipper
    private val handler = Handler(Looper.getMainLooper())
    private var currentPage = 0
    private var polling = false

    private val pollRunnable = object : Runnable {
        override fun run() {
            if (!polling) return
            val satisfied = when (currentPage) {
                PAGE_ENABLE -> isImeEnabled()
                PAGE_DEFAULT -> isImeDefault()
                else -> true
            }
            if (satisfied) {
                polling = false
                showCheckAndAdvance(currentPage)
            } else {
                handler.postDelayed(this, POLL_INTERVAL_MS)
            }
        }
    }

    private fun dp(v: Float): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, resources.displayMetrics).toInt()

    private fun isNight(): Boolean =
        (resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        flipper = ViewFlipper(this).apply {
            addView(buildWelcomePage())
            addView(buildEnablePage())
            addView(buildDefaultPage())
            addView(buildFinishPage())
        }
        setContentView(flipper)
    }

    override fun onPause() {
        super.onPause()
        polling = false
        handler.removeCallbacks(pollRunnable)
    }

    // ---------- 轮询 ----------

    private fun startPolling() {
        polling = true
        handler.removeCallbacks(pollRunnable)
        handler.postDelayed(pollRunnable, POLL_INTERVAL_MS)
    }

    /** 打勾提示用简单 alpha 淡入，随后自动翻到下一页。 */
    private fun showCheckAndAdvance(page: Int) {
        val pageRoot = flipper.getChildAt(page) as? LinearLayout ?: run {
            showPage(page + 1)
            return
        }
        val status = pageRoot.findViewWithTag<TextView>(TAG_STATUS) ?: run {
            showPage(page + 1)
            return
        }
        status.setText(if (page == PAGE_ENABLE) R.string.onboarding_enable_done else R.string.onboarding_default_done)
        status.setTextColor(BRAND_GREEN)
        status.animate().cancel()
        status.alpha = 0f
        status.animate().alpha(1f).setDuration(300L).withEndAction {
            handler.postDelayed({ showPage(page + 1) }, 600L)
        }.start()
    }

    private fun showPage(page: Int) {
        currentPage = page
        handler.removeCallbacks(pollRunnable)
        if (page >= flipper.childCount) {
            finishWizard()
            return
        }
        flipper.displayedChild = page
        // 翻到轮询页时立即检查一次：有可能用户在设置里已经开好。
        if (page == PAGE_ENABLE || page == PAGE_DEFAULT) {
            val satisfied = if (page == PAGE_ENABLE) isImeEnabled() else isImeDefault()
            if (satisfied) {
                showCheckAndAdvance(page)
            } else {
                startPolling()
            }
        }
    }

    private fun isImeEnabled(): Boolean {
        val imm = getSystemService(InputMethodManager::class.java) ?: return false
        return imm.enabledInputMethodList.any { it.packageName == packageName }
    }

    private fun isImeDefault(): Boolean {
        val default = Settings.Secure.getString(
            contentResolver,
            Settings.Secure.DEFAULT_INPUT_METHOD,
        ) ?: return false
        return default.startsWith("$packageName/")
    }

    private fun finishWizard() {
        getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .putBoolean(KEY_DONE, true)
            .apply()
        startActivity(Intent(this, MainActivity::class.java))
        finish()
    }

    // ---------- 页面 ----------

    private fun pageScaffold(): Pair<LinearLayout, LinearLayout> {
        val scroll = ScrollView(this)
        val content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(24f), dp(48f), dp(24f), dp(24f))
        }
        scroll.addView(content)
        val page = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(scroll, LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                0,
                1f,
            ))
        }
        return page to content
    }

    private fun titleView(text: CharSequence): TextView = TextView(this).apply {
        this.text = text
        textSize = 26f
        setTypeface(typeface, Typeface.BOLD)
    }

    private fun bodyView(text: CharSequence): TextView = TextView(this).apply {
        this.text = text
        textSize = 15f
        setPadding(0, dp(12f), 0, 0)
    }

    private fun primaryButton(textRes: Int, onClick: () -> Unit): TextView =
        TextView(this).apply {
            setText(textRes)
            textSize = 16f
            gravity = Gravity.CENTER
            setTextColor(0xFFFFFFFF.toInt())
            background = GradientDrawable().apply {
                setColor(BRAND_GREEN)
                cornerRadius = dp(8f).toFloat()
            }
            setOnClickListener { onClick() }
        }

    private fun bullet(textRes: Int): TextView = TextView(this).apply {
        setText(textRes)
        textSize = 14f
        setPadding(0, dp(8f), 0, 0)
    }

    private fun statusLine(): TextView = TextView(this).apply {
        tag = TAG_STATUS
        textSize = 14f
        setTextColor(if (isNight()) 0xFF8A8F99.toInt() else 0xFF888888.toInt())
        setPadding(0, dp(12f), 0, 0)
    }

    private fun buildWelcomePage(): View {
        val (page, content) = pageScaffold()
        // 品牌图标：与启动器同一套资源
        content.addView(ImageView(this).apply {
            setImageResource(R.drawable.ic_launcher_foreground)
            contentDescription = getString(R.string.main_brand_title)
        }, LinearLayout.LayoutParams(dp(72f), dp(72f)).apply { bottomMargin = dp(16f) })
        content.addView(titleView(getString(R.string.onboarding_welcome_title)))
        content.addView(bodyView(getString(R.string.onboarding_welcome_subtitle)))
        content.addView(bullet(R.string.onboarding_welcome_bullet_1))
        content.addView(bullet(R.string.onboarding_welcome_bullet_2))
        content.addView(bullet(R.string.onboarding_welcome_bullet_3))
        content.addView(
            primaryButton(R.string.onboarding_welcome_start) { showPage(PAGE_ENABLE) },
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                dp(48f),
            ).apply { topMargin = dp(28f) },
        )
        return page
    }

    private fun buildEnablePage(): View {
        val (page, content) = pageScaffold()
        content.addView(titleView(getString(R.string.onboarding_enable_title)))
        content.addView(bodyView(getString(R.string.onboarding_enable_desc)))
        content.addView(
            primaryButton(R.string.onboarding_enable_button) {
                startActivity(Intent(Settings.ACTION_INPUT_METHOD_SETTINGS))
            },
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                dp(48f),
            ).apply { topMargin = dp(24f) },
        )
        content.addView(statusLine().apply {
            setText(R.string.onboarding_enable_waiting)
        })
        return page
    }

    private fun buildDefaultPage(): View {
        val (page, content) = pageScaffold()
        content.addView(titleView(getString(R.string.onboarding_default_title)))
        content.addView(bodyView(getString(R.string.onboarding_default_desc)))
        content.addView(
            primaryButton(R.string.onboarding_default_button) {
                getSystemService(InputMethodManager::class.java)?.showInputMethodPicker()
            },
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                dp(48f),
            ).apply { topMargin = dp(24f) },
        )
        content.addView(statusLine().apply {
            setText(R.string.onboarding_default_waiting)
        })
        return page
    }

    private fun buildFinishPage(): View {
        val (page, content) = pageScaffold()
        content.addView(titleView(getString(R.string.onboarding_finish_title)))
        content.addView(bodyView(getString(R.string.onboarding_finish_desc)))
        content.addView(
            primaryButton(R.string.onboarding_finish_button) { finishWizard() },
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                dp(48f),
            ).apply { topMargin = dp(24f) },
        )
        return page
    }

    companion object {
        private const val PREFS = "shurufa_onboarding"
        private const val KEY_DONE = "onboarding_done"
        private const val TAG_STATUS = "onboarding_status"
        private const val PAGE_ENABLE = 1
        private const val PAGE_DEFAULT = 2
        private const val POLL_INTERVAL_MS = 500L
        private val BRAND_GREEN = 0xFF1B9E77.toInt()

        fun isDone(context: Context): Boolean =
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .getBoolean(KEY_DONE, false)

        fun reset(context: Context) {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .edit()
                .putBoolean(KEY_DONE, false)
                .apply()
        }
    }
}
