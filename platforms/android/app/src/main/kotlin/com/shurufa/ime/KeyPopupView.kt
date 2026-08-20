package com.shurufa.ime

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.util.TypedValue
import android.view.View

/**
 * P0 借鉴搜狗输入法：按键气泡预览（Key Popup Preview）。
 *
 * 按下字母/数字/笔画键时，在键上方（顶部行则在下方）自绘圆角气泡，
 * 显示主字符与右上角副字符（如 q 键的 1），松手隐藏。参照
 * Sogou imskit KeyboardPopupView 的「自绘 + 方向自适应」设计。
 */
internal class KeyPopupView(context: Context) : View(context) {
    /** 主字符。 */
    var primaryText: String = ""
    /** 副字符（右上角小字，如字母键上的数字/符号）。 */
    var secondaryText: String = ""
    /** 深色皮肤。 */
    var dark: Boolean = false
    /** true=气泡在键上方（箭头朝下）；false=在键下方（箭头朝上）。 */
    var popupAbove: Boolean = true

    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val bgPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        setShadowLayer(dp(5f), 0f, dp(2f), 0x66000000)
    }

    override fun onDraw(canvas: Canvas) {
        val w = width.toFloat()
        val h = height.toFloat()
        val arrow = dp(7f)
        // 气泡主体（圆角矩形），为箭头留出底部/顶部缺口
        val body = if (popupAbove) {
            RectF(dp(5f), 0f, w - dp(5f), h - arrow)
        } else {
            RectF(dp(5f), arrow, w - dp(5f), h)
        }
        bgPaint.color = if (dark) 0xFF2A2F36.toInt() else 0xFFFFFFFF.toInt()
        canvas.drawRoundRect(body, dp(11f), dp(11f), bgPaint)
        // 小三角（指向按键）
        val cx = w / 2f
        val path = android.graphics.Path()
        if (popupAbove) {
            path.moveTo(cx - arrow, body.bottom)
            path.lineTo(cx, h)
            path.lineTo(cx + arrow, body.bottom)
        } else {
            path.moveTo(cx - arrow, body.top)
            path.lineTo(cx, 0f)
            path.lineTo(cx + arrow, body.top)
        }
        path.close()
        canvas.drawPath(path, bgPaint)

        // 主字符
        paint.color = if (dark) 0xFFFFFFFF.toInt() else 0xFF1F2329.toInt()
        paint.textSize = sp(20f)
        paint.textAlign = Paint.Align.CENTER
        val baseline = if (popupAbove) {
            body.centerY() + (paint.descent() - paint.ascent()) / 2f - paint.descent()
        } else {
            body.centerY() + (paint.descent() - paint.ascent()) / 2f - paint.descent()
        }
        canvas.drawText(primaryText, w / 2f, baseline, paint)

        // 副字符（右上角）
        if (secondaryText.isNotEmpty()) {
            paint.textSize = sp(10f)
            paint.textAlign = Paint.Align.RIGHT
            val smallY = if (popupAbove) body.top + dp(12f) else body.top + dp(12f)
            canvas.drawText(secondaryText, w - dp(10f), smallY, paint)
        }
    }

    private fun dp(v: Float): Float =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, resources.displayMetrics)

    private fun sp(v: Float): Float =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, v, resources.displayMetrics)
}
