package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Test

/** M-A1-1 键盘偏好的纯函数验证（高度夹取 + 单手模式解析回退）。 */
class KeyboardPrefsTest {

    @Test
    fun heightPercentClampedToBounds() {
        assertEquals(40, KeyboardPrefs.clampHeight(0))
        assertEquals(40, KeyboardPrefs.clampHeight(10))
        assertEquals(120, KeyboardPrefs.clampHeight(300))
        assertEquals(100, KeyboardPrefs.clampHeight(100))
        assertEquals(75, KeyboardPrefs.clampHeight(75))
        assertEquals(KeyboardPrefs.MIN_HEIGHT_PERCENT, KeyboardPrefs.clampHeight(-5))
    }

    @Test
    fun parseSingleHandFallsBackToOff() {
        assertSame(SingleHandMode.OFF, KeyboardPrefs.parseSingleHand(null))
        assertSame(SingleHandMode.OFF, KeyboardPrefs.parseSingleHand(""))
        assertSame(SingleHandMode.OFF, KeyboardPrefs.parseSingleHand("bogus"))
        assertSame(SingleHandMode.LEFT, KeyboardPrefs.parseSingleHand("LEFT"))
        assertSame(SingleHandMode.RIGHT, KeyboardPrefs.parseSingleHand("RIGHT"))
    }

    @Test
    fun defaultsKeepCurrentBehavior() {
        val prefs = KeyboardPrefs()
        assertEquals(100, prefs.heightPercent)
        assertEquals(true, prefs.keySound)
        assertEquals(true, prefs.haptic)
        assertSame(SingleHandMode.OFF, prefs.singleHand)
    }
}
