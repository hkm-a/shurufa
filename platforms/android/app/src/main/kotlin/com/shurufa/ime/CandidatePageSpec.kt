package com.shurufa.ime

/** 候选页与展开网格的共享规格，限制在 Rime 数字直选键可覆盖的范围内。 */
internal object CandidatePageSpec {
    const val PAGE_SIZE = 9
    const val GRID_COLUMNS = 3

    fun visibleCount(candidateCount: Int): Int = candidateCount.coerceIn(0, PAGE_SIZE)

    fun gridRowCount(candidateCount: Int): Int =
        (visibleCount(candidateCount) + GRID_COLUMNS - 1) / GRID_COLUMNS
}
