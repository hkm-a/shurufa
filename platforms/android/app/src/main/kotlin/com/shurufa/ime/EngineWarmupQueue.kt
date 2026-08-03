package com.shurufa.ime

import java.util.ArrayDeque

/**
 * 首次部署词典期间暂存当前输入框的按键。
 *
 * 每项携带输入会话代号。焦点切换后旧代号会被丢弃，不能把用户在 A 输入框
 * 的内容误写到 B 输入框。
 */
internal class EngineWarmupQueue<T> {
    private data class Entry<T>(val inputGeneration: Long, val value: T)

    private val entries = ArrayDeque<Entry<T>>()

    fun enqueue(inputGeneration: Long, value: T) {
        entries.addLast(Entry(inputGeneration, value))
    }

    fun drain(inputGeneration: Long): List<T> {
        val values = ArrayList<T>(entries.size)
        while (entries.isNotEmpty()) {
            val entry = entries.removeFirst()
            if (entry.inputGeneration == inputGeneration) values += entry.value
        }
        return values
    }

    fun clear() {
        entries.clear()
    }
}
