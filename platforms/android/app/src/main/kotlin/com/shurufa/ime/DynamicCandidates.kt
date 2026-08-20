package com.shurufa.ime

import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Locale

/**
 * P4-2 借鉴搜狗 V 模式 / rime-ice date_translator：动态快捷码（Kotlin 层，零 librime 依赖）。
 * Android librime 无 lua 插件，date_translator.lua 不可用，故在候选层直接注入动态值：
 *   rq = 今日日期（2026年8月20日）  sj = 当前时间（19:19:04）
 *   xq = 今日星期（星期四）         dt = ISO 日期（2026-08-20）
 *   ts = Unix 时间戳               R<数字> = 金额（小写+大写两种候选）
 * 触发即候选（引擎候选之前注入），选择时直接提交文本，不占用引擎索引。
 */
object DynamicCandidates {

    private val CN_NUM = listOf("零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖")
    private val CN_UNIT = listOf("", "拾", "佰", "仟", "万", "拾", "佰", "仟", "亿")

    fun forPreedit(preedit: String): List<String>? = when (preedit.lowercase(Locale.ROOT)) {
        "rq" -> listOf(SimpleDateFormat("yyyy年M月d日", Locale.CHINA).format(Calendar.getInstance().time))
        "sj" -> listOf(SimpleDateFormat("HH:mm:ss", Locale.CHINA).format(Calendar.getInstance().time))
        "xq" -> listOf(weekday())
        "dt" -> listOf(SimpleDateFormat("yyyy-MM-dd", Locale.CHINA).format(Calendar.getInstance().time))
        "ts" -> listOf(System.currentTimeMillis().toString())
        else -> moneyCandidates(preedit)
    }

    /** R 前缀金额：R123 → [123元, 壹佰贰拾叁元]；R34.56 → [34.56元, 叁拾肆元伍角陆分]。 */
    fun moneyCandidates(preedit: String): List<String>? {
        val raw = preedit.lowercase(Locale.ROOT)
        if (!raw.startsWith("r")) return null
        val num = raw.substring(1)
        if (num.isBlank() || !num.all { it.isDigit() || it == '.' }) return null
        val parts = num.split('.')
        if (parts.size > 2) return null
        val integer = parts[0].ifEmpty { "0" }
        val decimal = parts.getOrNull(1)?.take(2)?.ifEmpty { "00" } ?: "00"
        if (integer.length > 10 || !integer.all { it.isDigit() } || !decimal.all { it.isDigit() }) return null
        val small = num + "元"
        val big = upperInteger(integer) + moneyDecimal(decimal)
        return listOf(small, big)
    }

    private fun weekday(): String {
        val w = Calendar.getInstance().get(Calendar.DAY_OF_WEEK)
        return "星期" + listOf("日", "一", "二", "三", "四", "五", "六")[w - 1]
    }

    /** 整数部分大写：123 → 壹佰贰拾叁。 */
    fun upperInteger(n: String): String {
        val digits = n.trimStart('0').ifEmpty { "0" }
        if (digits == "0") return "零元"
        val sb = StringBuilder()
        val len = digits.length
        for (i in digits.indices) {
            val d = digits[i] - '0'
            val unit = len - 1 - i
            if (d == 0) {
                if (sb.isNotEmpty() && sb.last() != '零' && unit % 4 != 0) sb.append('零')
            } else {
                sb.append(CN_NUM[d]).append(CN_UNIT[unit])
            }
        }
        var s = sb.toString().trimEnd('零')
        s = s.replace("亿万", "亿")
        return s + "元"
    }

    private fun moneyDecimal(decimal: String): String {
        val jiao = decimal[0] - '0'
        val fen = if (decimal.length > 1) decimal[1] - '0' else 0
        val sb = StringBuilder()
        if (jiao > 0) sb.append(CN_NUM[jiao]).append("角")
        if (fen > 0) sb.append(CN_NUM[fen]).append("分")
        return sb.toString()
    }
}
