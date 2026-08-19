package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** M-A2-4 计算器纯逻辑验证（优先级 / 取模 / 负号 / 除零 / 长度）。 */
class CalculatorTest {

    @Test
    fun 优先级乘除先于加减() {
        val c = Calculator()
        "1+2x3".forEach { assertTrue(c.input(it)) }
        assertEquals(7.0, c.evaluate()!!, 1e-9)
        assertEquals("7", c.formatResult(c.evaluate()!!))
    }

    @Test
    fun 取模与除法() {
        val c = Calculator()
        "10%3".forEach { assertTrue(c.input(it)) }
        assertEquals(1.0, c.evaluate()!!, 1e-9)
        c.clear()
        "8/4".forEach { assertTrue(c.input(it)) }
        assertEquals(2.0, c.evaluate()!!, 1e-9)
    }

    @Test
    fun 一元负号与小数() {
        val c = Calculator()
        "-5+3".forEach { assertTrue(c.input(it)) }
        assertEquals(-2.0, c.evaluate()!!, 1e-9)
        c.clear()
        "1.5x2".forEach { assertTrue(c.input(it)) }
        assertEquals(3.0, c.evaluate()!!, 1e-9)
        assertEquals("3", c.formatResult(c.evaluate()!!))
    }

    @Test
    fun 除零与非法表达式() {
        val c = Calculator()
        "1/0".forEach { assertTrue(c.input(it)) }
        assertNull(c.evaluate())
        c.clear()
        // 连续运算符不被接受
        assertFalse(c.input('+'))
        assertTrue(c.input('1'))
        assertTrue(c.input('+'))
        assertFalse(c.input('x'))
    }

    @Test
    fun 长度上限() {
        val c = Calculator(maxLen = 6)
        "123456".forEach { assertTrue(c.input(it)) }
        assertFalse(c.input('7'))
        assertEquals(6, c.expression.length)
    }
}
