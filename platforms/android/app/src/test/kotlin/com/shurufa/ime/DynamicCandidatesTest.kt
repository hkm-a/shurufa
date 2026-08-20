package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** P4-2 动态快捷码：日期/时间/星期/金额大写转换。 */
class DynamicCandidatesTest {

    @Test
    fun testRqDate() {
        val list = DynamicCandidates.forPreedit("rq")
        assertNotNull(list)
        assertTrue(list!!.size == 1)
        assertTrue(list[0].matches(Regex("\\d{4}年\\d{1,2}月\\d{1,2}日")))
    }

    @Test
    fun testSjTime() {
        val list = DynamicCandidates.forPreedit("sj")
        assertNotNull(list)
        assertTrue(list!![0].matches(Regex("\\d{2}:\\d{2}:\\d{2}")))
    }

    @Test
    fun testXqWeekday() {
        val list = DynamicCandidates.forPreedit("xq")
        assertNotNull(list)
        assertTrue(list!![0].startsWith("星期"))
    }

    @Test
    fun testRMoneyTwoCandidates() {
        val list = DynamicCandidates.moneyCandidates("R123")
        assertNotNull(list)
        assertEquals(listOf("123元", "壹佰贰拾叁元"), list)
    }

    @Test
    fun testRMoneyWithJiaoFen() {
        val list = DynamicCandidates.moneyCandidates("R34.56")
        assertNotNull(list)
        assertEquals("34.56元", list!![0])
        assertEquals("叁拾肆元伍角陆分", list[1])
    }

    @Test
    fun testUpperIntegerRules() {
        assertEquals("壹佰零壹元", DynamicCandidates.upperInteger("101"))
        assertEquals("壹仟零壹拾元", DynamicCandidates.upperInteger("1010"))
        assertEquals("壹万零壹元", DynamicCandidates.upperInteger("10001"))
    }

    @Test
    fun testNonTriggerInputs() {
        assertNull(DynamicCandidates.moneyCandidates("abc"))
        assertNull(DynamicCandidates.moneyCandidates("R"))
        assertNull(DynamicCandidates.moneyCandidates("R1.2.3"))
    }
}
