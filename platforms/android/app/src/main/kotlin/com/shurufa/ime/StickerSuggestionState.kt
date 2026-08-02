package com.shurufa.ime

/** 表情包联想的延迟、最小输入长度与过期结果失效规则。 */
internal object StickerSuggestionPolicy {
    const val delayMillis = 420L
    const val visibleLimit = 2

    fun terms(preedit: String, candidates: List<String>): List<String> {
        val primary = preedit.trim()
        if (primary.length < 2) return emptyList()
        return buildList {
            add(primary)
            candidates.take(3).map(String::trim).filter(String::isNotEmpty).forEach(::add)
        }.distinct()
    }
}

/**
 * 将每次候选更新标记为一个不可复用的请求，后台任务只能展示仍处于当前代次的结果。
 * 该类不依赖 Android，保证输入防抖的核心竞态规则可由本地单元测试覆盖。
 */
internal class StickerSuggestionState {
    internal data class Request(
        val generation: Long,
        val terms: List<String>,
    )

    private var generation = 0L

    fun replace(preedit: String, candidates: List<String>): Request? {
        generation += 1
        val terms = StickerSuggestionPolicy.terms(preedit, candidates)
        return terms.takeIf(List<String>::isNotEmpty)?.let { Request(generation, it) }
    }

    fun invalidate() {
        generation += 1
    }

    fun isCurrent(request: Request): Boolean = request.generation == generation
}
