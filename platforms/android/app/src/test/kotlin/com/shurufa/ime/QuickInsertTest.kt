package com.shurufa.ime

import java.util.Calendar
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** M-A2-3 时间/日期/邮箱后缀的纯逻辑验证（固定 2026-08-19 周三 20:30:45）。 */
class QuickInsertTest {

    private fun fixed(): Calendar = Calendar.getInstance().apply {
        clear()
        set(2026, Calendar.AUGUST, 19, 20, 30, 45)
    }

    @Test
    fun 时间条目格式化() {
        val items = QuickInsert.timeItems(fixed())
        assertEquals("现在时间 20:30", items[0].label)
        assertEquals("20:30", items[0].value)
        assertEquals("20:30:45", items[1].value)
    }

    @Test
    fun 日期条目含星期() {
        val items = QuickInsert.dateItems(fixed())
        assertEquals("2026-08-19", items[0].value)
        assertEquals("2026年8月19日", items[1].value)
        assertEquals("星期三", items[2].value)
        assertEquals("星期三", items[2].label)
    }

    @Test
    fun 邮箱后缀带at且含常用域() {
        val items = QuickInsert.emailItems()
        assertTrue(items.isNotEmpty())
        assertTrue(items.all { it.value.startsWith("@") })
        assertTrue(items.any { it.value == "@qq.com" })
        assertTrue(items.any { it.value == "@gmail.com" })
    }
}
