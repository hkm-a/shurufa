package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Test

class CandidatePageSpecTest {
    @Test
    fun 当前页最多展示九个可直接选择的候选词() {
        assertEquals(9, CandidatePageSpec.visibleCount(12))
        assertEquals(9, CandidatePageSpec.visibleCount(9))
        assertEquals(0, CandidatePageSpec.visibleCount(-1))
    }

    @Test
    fun 展开页按三列排列当前候选() {
        assertEquals(3, CandidatePageSpec.gridRowCount(9))
        assertEquals(2, CandidatePageSpec.gridRowCount(4))
        assertEquals(0, CandidatePageSpec.gridRowCount(0))
    }
}
