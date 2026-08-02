package com.shurufa.ime

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BackspaceGestureSpecTest {
    @Test
    fun 上滑超过阈值才触发清空() {
        assertTrue(BackspaceGestureSpec.shouldClear(100f, 60f, 36f))
        assertFalse(BackspaceGestureSpec.shouldClear(100f, 64f, 36f))
        assertFalse(BackspaceGestureSpec.shouldClear(100f, 140f, 36f))
    }

    @Test
    fun 轻触删除而已清空或长按连删后不重复删除() {
        assertTrue(BackspaceGestureSpec.shouldDeleteOnRelease(cleared = false, repeated = false))
        assertFalse(BackspaceGestureSpec.shouldDeleteOnRelease(cleared = true, repeated = false))
        assertFalse(BackspaceGestureSpec.shouldDeleteOnRelease(cleared = false, repeated = true))
    }
}
