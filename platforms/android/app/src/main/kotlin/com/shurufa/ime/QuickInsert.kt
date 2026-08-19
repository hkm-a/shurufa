package com.shurufa.ime

import java.util.Calendar

/** 快捷输入条目：label 展示、value 上屏。 */
data class QuickInsertItem(val label: String, val value: String)

/**
 * M-A2-3 时间/日期/邮箱后缀快捷输入（搜狗安卓 7.4）。
 * 纯函数基于 Calendar 生成，便于 JVM 单测；UI 面板点击即上屏。
 */
object QuickInsert {
    val EMAIL_SUFFIXES = listOf(
        "qq.com", "163.com", "126.com", "gmail.com", "outlook.com",
        "foxmail.com", "hotmail.com", "sina.com", "139.com", "icloud.com",
    )

    private val WEEK = arrayOf("日", "一", "二", "三", "四", "五", "六")

    fun timeItems(now: Calendar): List<QuickInsertItem> {
        val h = now.get(Calendar.HOUR_OF_DAY)
        val m = now.get(Calendar.MINUTE)
        val s = now.get(Calendar.SECOND)
        return listOf(
            QuickInsertItem("现在时间 " + pad(h) + ":" + pad(m), pad(h) + ":" + pad(m)),
            QuickInsertItem(
                "当前时间 " + pad(h) + ":" + pad(m) + ":" + pad(s),
                pad(h) + ":" + pad(m) + ":" + pad(s),
            ),
        )
    }

    fun dateItems(now: Calendar): List<QuickInsertItem> {
        val y = now.get(Calendar.YEAR)
        val m = now.get(Calendar.MONTH) + 1
        val d = now.get(Calendar.DAY_OF_MONTH)
        val week = "星期" + WEEK[(now.get(Calendar.DAY_OF_WEEK) - Calendar.SUNDAY + 7) % 7]
        return listOf(
            QuickInsertItem("今天日期 " + pad(y, 4) + "-" + pad(m) + "-" + pad(d), pad(y, 4) + "-" + pad(m) + "-" + pad(d)),
            QuickInsertItem("年月日 " + y + "年" + m + "月" + d + "日", y.toString() + "年" + m + "月" + d + "日"),
            QuickInsertItem(week, week),
        )
    }

    fun emailItems(): List<QuickInsertItem> =
        EMAIL_SUFFIXES.map { QuickInsertItem("@" + it, "@" + it) }

    private fun pad(value: Int, width: Int = 2): String =
        value.toString().padStart(width, '0')
}
