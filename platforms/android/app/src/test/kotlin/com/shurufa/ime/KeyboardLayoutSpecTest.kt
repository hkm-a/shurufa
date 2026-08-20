package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class KeyboardLayoutSpecTest {
    @Test
    fun 中文默认页显示小写字母() {
        assertEquals("q", KeyboardLayoutSpec.displayLabel("Q", false))
    }

    @Test
    fun 英文页显示大写字母且不改变功能键文字() {
        assertEquals("Q", KeyboardLayoutSpec.displayLabel("q", true))
        assertEquals("换行", KeyboardLayoutSpec.displayLabel("换行", true))
    }

    @Test
    fun 中英文切换键明确当前输入状态() {
        assertEquals("中\n英", KeyboardLayoutSpec.languageLabel(false))
        assertEquals("英\n中", KeyboardLayoutSpec.languageLabel(true))
    }

    @Test
    fun 九键页三行数字加完整底栏() {
        val rows = KeyboardLayoutSpec.t9Rows()
        assertEquals(4, rows.size)
        // 前三行：3×3 数字键（1-9）
        val digits = rows.take(3).flatMap { it.keys }
        assertEquals(9, digits.size)
        assertEquals(listOf("1", "2", "3", "4", "5", "6", "7", "8", "9"), digits.map { it.label })
        assertTrue(digits.all { it.kind == KeyboardLayoutSpec.Kind.DIGIT })
        // 键面字母提示：2→abc … 9→wxyz
        assertEquals("abc", rows[0].keys[1].secondary)
        assertEquals("wxyz", rows[2].keys[2].secondary)
        // 底栏：符 / 中英 / 空格 / 删除 / 换行
        assertEquals(
            listOf(
                KeyboardLayoutSpec.Kind.NUMBER,
                KeyboardLayoutSpec.Kind.LANG,
                KeyboardLayoutSpec.Kind.SPACE,
                KeyboardLayoutSpec.Kind.BACKSPACE,
                KeyboardLayoutSpec.Kind.ENTER,
            ),
            rows[3].keys.map { it.kind },
        )
        assertTrue(rows[3].keys.first { it.kind == KeyboardLayoutSpec.Kind.SPACE }.weight > 1.5f)
    }

    @Test
    fun 笔画页五行笔画加数字直选与底栏() {
        val rows = KeyboardLayoutSpec.strokeRows()
        assertEquals(3, rows.size)
        assertEquals(
            listOf("一", "丨", "丿", "丶", "乙"),
            rows[0].keys.map { it.label },
        )
        assertTrue(rows[0].keys.all { it.kind == KeyboardLayoutSpec.Kind.STROKE })
        assertEquals(listOf("1", "2", "3", "4", "5"), rows[1].keys.map { it.label })
        assertEquals(
            listOf(
                KeyboardLayoutSpec.Kind.NUMBER,
                KeyboardLayoutSpec.Kind.LANG,
                KeyboardLayoutSpec.Kind.SPACE,
                KeyboardLayoutSpec.Kind.BACKSPACE,
                KeyboardLayoutSpec.Kind.ENTER,
            ),
            rows[2].keys.map { it.kind },
        )
    }

    @Test
    fun 默认页底栏保留完整的中英文与输入功能() {
        val row = KeyboardLayoutSpec.letterRows(false).last()
        assertEquals(
            listOf(
                KeyboardLayoutSpec.Kind.NUMBER,
                KeyboardLayoutSpec.Kind.CHAR,
                KeyboardLayoutSpec.Kind.SPACE,
                KeyboardLayoutSpec.Kind.LANG,
                KeyboardLayoutSpec.Kind.ENTER,
            ),
            row.keys.map { it.kind },
        )
        assertTrue(row.keys.first { it.kind == KeyboardLayoutSpec.Kind.SPACE }.weight > 3f)
    }

    @Test
    fun 字母长按保留符号而删除手势只属于退格键() {
        val rows = KeyboardLayoutSpec.letterRows(false)
        val q = rows.first().keys.first { it.label == "Q" }
        val x = rows[2].keys.first { it.label == "X" }
        val backspace = rows[2].keys.last()
        assertEquals("1", q.secondary)
        assertEquals(null, x.longKind)
        assertTrue(!x.swipeUpClears)
        assertEquals(KeyboardLayoutSpec.Kind.BACKSPACE, backspace.kind)
        assertEquals(KeyboardLayoutSpec.Kind.BACKSPACE, backspace.longKind)
        assertTrue(backspace.swipeUpClears)
    }

    @Test
    fun 数字入口进入三行九宫格数字内容() {
        val numberKey = KeyboardLayoutSpec.letterRows(false).last().keys.first()
        val rows = KeyboardLayoutSpec.symbolRows()
        assertEquals("123", numberKey.label)
        assertEquals(KeyboardLayoutSpec.Kind.NUMBER, numberKey.kind)
        assertEquals(listOf("1", "2", "3"), rows[0].keys.map { it.label })
        assertEquals(listOf("4", "5", "6"), rows[1].keys.map { it.label })
        assertEquals(listOf("7", "8", "9"), rows[2].keys.map { it.label })
        // P4-3：符号页第 4 行插入撇号键（拆字部件码分隔符），数字 0 顺移到 index 2；
        // P4-6：第 5 行为中文标点行，功能行固定在第 4 行（index 3）
        assertEquals("'", rows[3].keys[1].label)
        assertEquals("0", rows[3].keys[2].label)
        assertTrue(rows[4].keys.any { it.label == "，" })
        assertTrue(rows[4].keys.any { it.label == "……" })
    }

    @Test
    fun 九宫格页只保留数字输入必需功能() {
        val bottom = KeyboardLayoutSpec.symbolRows()[3]
        assertEquals(KeyboardLayoutSpec.Kind.BACK, bottom.keys[0].kind)
        assertTrue(bottom.keys.any { it.kind == KeyboardLayoutSpec.Kind.SPACE })
        assertTrue(bottom.keys.any { it.kind == KeyboardLayoutSpec.Kind.BACKSPACE })
        assertTrue(bottom.keys.any { it.kind == KeyboardLayoutSpec.Kind.ENTER })
    }

    @Test
    fun 表情包不再占用键盘底栏而由候选栏联想() {
        val allKeys = KeyboardLayoutSpec.letterRows(false).flatMap { it.keys } +
            KeyboardLayoutSpec.symbolRows().flatMap { it.keys }
        assertTrue(allKeys.none { it.description.contains("表情") })
    }
}
