package com.shurufa.ime

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VoiceGestureSpecTest {
    @Test
    fun upward_swipe_beyond_threshold_cancels_voice_input() {
        assertTrue(VoiceGestureSpec.shouldCancel(downY = 120f, currentY = 79f, thresholdPx = 40f))
    }

    @Test
    fun threshold_or_downward_motion_does_not_cancel_voice_input() {
        assertFalse(VoiceGestureSpec.shouldCancel(downY = 120f, currentY = 80f, thresholdPx = 40f))
        assertFalse(VoiceGestureSpec.shouldCancel(downY = 120f, currentY = 150f, thresholdPx = 40f))
    }
}
