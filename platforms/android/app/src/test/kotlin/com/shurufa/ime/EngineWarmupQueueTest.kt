package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class EngineWarmupQueueTest {
    @Test
    fun 同一输入框按键保持原始顺序() {
        val queue = EngineWarmupQueue<Char>()
        queue.enqueue(7, 'n')
        queue.enqueue(7, 'i')
        queue.enqueue(7, ' ')

        assertEquals(listOf('n', 'i', ' '), queue.drain(7))
    }

    @Test
    fun 焦点切换后旧输入框按键不会回放() {
        val queue = EngineWarmupQueue<Char>()
        queue.enqueue(3, 'n')
        queue.enqueue(4, 'h')

        assertEquals(listOf('h'), queue.drain(4))
        assertTrue(queue.drain(3).isEmpty())
    }

    @Test
    fun 清理会丢弃尚未处理的按键() {
        val queue = EngineWarmupQueue<Char>()
        queue.enqueue(1, 'n')
        queue.clear()

        assertTrue(queue.drain(1).isEmpty())
    }
}
