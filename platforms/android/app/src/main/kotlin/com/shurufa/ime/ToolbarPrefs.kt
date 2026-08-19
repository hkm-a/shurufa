package com.shurufa.ime

/**
 * M-A5-1 键盘工具栏自定义（搜狗安卓 20.10/20.11「工具栏 icon 扩增、自定义
 * 左中右、最近使用」）。
 *
 * 持久化：逗号分隔的启用 id 顺序列表；隐藏 = 从列表移除，显示 = 追加
 * （resolve 时按全量默认顺序补齐缺失项），排序 = move 上移/下移。
 * 纯函数便于 JVM 单测。
 */
object ToolbarPrefs {
    const val KEY = "toolbar_ids"

    val defaultIds: List<String> = listOf(
        "history", "images", "ai", "scheme", "settings",
        "phrases", "quick", "emoji", "calc",
    )

    fun encode(ids: List<String>): String = ids.joinToString(",")

    fun decode(raw: String?): List<String> =
        raw?.split(",")?.map { it.trim() }?.filter { it.isNotBlank() } ?: emptyList()

    /** 收敛出最终渲染顺序：已存顺序在前，缺省项按全量顺序补在末尾。 */
    fun resolve(saved: List<String>, all: List<String>): List<String> {
        val valid = all.toSet()
        if (saved.isEmpty()) return defaultIds.filter { it in valid }
        return saved.filter { it in valid } + all.filter { it !in saved }
    }

    /** 显隐切换：显示则追加到末尾，隐藏则移除。 */
    fun toggle(ids: List<String>, id: String): List<String> =
        if (id in ids) ids - id else ids + id

    /** 排序：上移（delta=-1）/下移（delta=1），越界忽略。 */
    fun move(ids: List<String>, id: String, delta: Int): List<String> {
        val idx = ids.indexOf(id)
        if (idx < 0) return ids
        val to = (idx + delta).coerceIn(0, ids.size - 1)
        if (to == idx) return ids
        val out = ids.toMutableList()
        out.removeAt(idx)
        out.add(to, id)
        return out
    }
}
