package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Test

class KeyboardHeightSpecTest {
    @Test
    fun 常规窗口保持屏幕百分之二十五点五的键区高度() {
        assertEquals(612, KeyboardHeightSpec.normalHeight(2400))
    }

    @Test
    fun 小窗键区不能超过输入视图实际余量() {
        assertEquals(317, KeyboardHeightSpec.resolve(normalHeightPx = 612, availableHeightPx = 317))
    }

    @Test
    fun 足够大的窗口不改变常规键区高度() {
        assertEquals(612, KeyboardHeightSpec.resolve(normalHeightPx = 612, availableHeightPx = 900))
    }

    @Test
    fun 横屏四行键区保证每行至少舒适触控高度() {
        assertEquals(460, KeyboardHeightSpec.preferredHeight(275, rowCount = 4, minimumRowHeightPx = 115))
    }

    @Test
    fun 横屏键高下限仍服从实际输入视图余量() {
        assertEquals(317, KeyboardHeightSpec.resolve(normalHeightPx = 460, availableHeightPx = 317))
    }

    @Test
    fun 异常尺寸不会产生负高度() {
        assertEquals(0, KeyboardHeightSpec.resolve(normalHeightPx = -1, availableHeightPx = -20))
    }
}
