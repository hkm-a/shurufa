package com.shurufa.ime

/**
 * M-A2-4 键盘计算器（搜狗安卓 9.5 计算器 / 11.28 长度上限与 % ‰ 运算）。
 * 纯状态机 + 两遍优先级解析（× ÷ % 高于 + -），无 Android 依赖，便于 JVM 单测。
 */
class Calculator(private val maxLen: Int = 24) {

    var expression: String = ""
        private set

    /** 追加字符；长度/字符集/运算符连续等非法输入返回 false。 */
    fun input(ch: Char): Boolean {
        if (expression.length >= maxLen) return false
        val valid = ch.isDigit() || ch == '.' || ch in "+-x/%"
        if (!valid) return false
        if (ch in "+x/%") {
            if (expression.isEmpty() || expression.last() in "+-x/%") return false
        }
        if (ch == '.') {
            // 同段数字里只能有一个小数点
            val segStart = expression.indexOfLast { it in "+-x/%" } + 1
            if (expression.substring(segStart).contains('.')) return false
        }
        expression += ch
        return true
    }

    fun backspace() {
        if (expression.isNotEmpty()) expression = expression.dropLast(1)
    }

    fun clear() {
        expression = ""
    }

    /** 解析计算；非法/除零返回 null。支持一元负号与小数。 */
    fun evaluate(): Double? {
        if (expression.isBlank()) return null
        val tokens = mutableListOf<String>()
        var i = 0
        while (i < expression.length) {
            val c = expression[i]
            when {
                c.isDigit() || c == '.' -> {
                    val sb = StringBuilder()
                    while (i < expression.length && (expression[i].isDigit() || expression[i] == '.')) {
                        sb.append(expression[i])
                        i++
                    }
                    tokens.add(sb.toString())
                }
                c in "+-x/%" -> {
                    if (c == '-' && (tokens.isEmpty() || tokens.last() in setOf("+", "-", "x", "/", "%"))) {
                        // 一元负号：负号后紧跟数字
                        var j = i + 1
                        while (j < expression.length && (expression[j].isDigit() || expression[j] == '.')) j++
                        if (j == i + 1) return null
                        val num = expression.substring(i + 1, j).toDoubleOrNull() ?: return null
                        tokens.add((-num).toString())
                        i = j
                    } else {
                        if (tokens.isEmpty() || tokens.last() in setOf("+", "-", "x", "/", "%")) return null
                        tokens.add(c.toString())
                        i++
                    }
                }
                else -> return null
            }
        }
        if (tokens.isEmpty() || tokens.last() in setOf("+", "-", "x", "/", "%")) return null

        // 第一遍：x / %（左结合）
        val high = mutableListOf<String>()
        var k = 0
        while (k < tokens.size) {
            val t = tokens[k]
            if (t in setOf("x", "/", "%")) {
                val left = high.removeAt(high.size - 1).toDouble()
                val right = tokens[k + 1].toDouble()
                val r = when (t) {
                    "x" -> left * right
                    "/" -> if (right == 0.0) return null else left / right
                    else -> if (right == 0.0) return null else left % right
                }
                high.add(r.toString())
                k += 2
            } else {
                high.add(t)
                k++
            }
        }
        // 第二遍：+ -
        var result = high[0].toDouble()
        var idx = 1
        while (idx < high.size) {
            val op = high[idx]
            val right = high[idx + 1].toDouble()
            result = if (op == "+") result + right else result - right
            idx += 2
        }
        return result
    }

    /** 结果展示串：整数去小数点；小数最多 8 位去尾零。 */
    fun formatResult(value: Double): String {
        if (value.isNaN() || value.isInfinite()) return "错误"
        if (value == value.toLong().toDouble() && kotlin.math.abs(value) < 1e15) {
            return value.toLong().toString()
        }
        return String.format("%.8f", value).trimEnd('0').trimEnd('.')
    }

    companion object {
        /** 键盘键位（4 列 x 5 行，等号由面板单独处理上屏）。 */
        val KEYS = listOf(
            listOf("C", "del", "%", "/"),
            listOf("7", "8", "9", "x"),
            listOf("4", "5", "6", "-"),
            listOf("1", "2", "3", "+"),
            listOf("0", ".", "=", ""),
        )
    }
}
