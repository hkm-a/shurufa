package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Test

/** M-A5-1 工具栏配置的纯逻辑验证（解析/收敛/显隐/排序）。 */
class ToolbarPrefsTest {

    private val all = listOf("a", "b", "c", "d")

    @Test
    fun 编码解析往返() {
        assertEquals(listOf("a", "c"), ToolbarPrefs.decode(ToolbarPrefs.encode(listOf("a", "c"))))
        assertEquals(emptyList<String>(), ToolbarPrefs.decode(null))
        assertEquals(emptyList<String>(), ToolbarPrefs.decode(" , "))
    }

    @Test
    fun 空配置回退默认顺序() {
        assertEquals(ToolbarPrefs.defaultIds, ToolbarPrefs.resolve(emptyList(), ToolbarPrefs.defaultIds))
    }

    @Test
    fun 保存顺序优先且缺省补齐() {
        assertEquals(listOf("c", "a", "b", "d"), ToolbarPrefs.resolve(listOf("c", "a"), all))
        // 非法 id 被过滤，保存项仍排前
        assertEquals(listOf("b", "a", "c", "d"), ToolbarPrefs.resolve(listOf("zzz", "b"), all))
    }

    @Test
    fun 显隐切换() {
        assertEquals(listOf("a", "c"), ToolbarPrefs.toggle(listOf("a", "b", "c"), "b"))
        assertEquals(listOf("a", "b", "c", "d"), ToolbarPrefs.toggle(listOf("a", "b", "c"), "d"))
    }

    @Test
    fun 上移下移() {
        // 上移：b 从下标 1 到 0
        assertEquals(listOf("b", "a", "c"), ToolbarPrefs.move(listOf("a", "b", "c"), "b", -1))
        // 下移：a 从下标 0 到 1
        assertEquals(listOf("b", "a", "c"), ToolbarPrefs.move(listOf("a", "b", "c"), "a", 1))
        // 越界忽略
        assertEquals(listOf("a", "b", "c"), ToolbarPrefs.move(listOf("a", "b", "c"), "a", -1))
        assertEquals(listOf("a", "b", "c"), ToolbarPrefs.move(listOf("a", "b", "c"), "x", 1))
    }
}
