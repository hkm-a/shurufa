package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Test

/** M-A3-1 触觉层次的纯逻辑验证（启动/重复/末位强振）。 */
class HapticProfileTest {

    @Test
    fun 长按删除启动用长按级反馈() {
        assertEquals(HapticProfile.LONG_PRESS, HapticProfile.deleteFeedback(0))
    }

    @Test
    fun 连续删除每tick用中等反馈() {
        assertEquals(HapticProfile.REPEAT, HapticProfile.deleteFeedback(1))
        assertEquals(HapticProfile.REPEAT, HapticProfile.deleteFeedback(5))
    }

    @Test
    fun 末位强振与启动同级() {
        assertEquals(HapticProfile.FINAL_FEEDBACK, HapticProfile.LONG_PRESS)
    }
}
