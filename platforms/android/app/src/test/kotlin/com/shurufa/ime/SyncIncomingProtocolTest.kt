package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class SyncIncomingProtocolTest {
    @Test
    fun 文本载荷保留字段分隔符() {
        val incoming = SyncIncomingProtocol.parse("text\u0001电脑\u0001第一段\u0001第二段")
        requireNotNull(incoming)
        assertEquals("text", incoming.kind)
        assertEquals("电脑", incoming.from)
        assertEquals("第一段\u0001第二段", incoming.payload)
    }

    @Test
    fun 附件条目编号正常解析() {
        assertEquals("42", SyncIncomingProtocol.parse("image\u0001电脑\u000142")?.payload)
        assertEquals("7", SyncIncomingProtocol.parse("file\u0001电脑\u00017")?.payload)
    }

    @Test
    fun 空值和未知类型被拒绝() {
        assertNull(SyncIncomingProtocol.parse(""))
        assertNull(SyncIncomingProtocol.parse("audio\u0001电脑\u0001x"))
        assertNull(SyncIncomingProtocol.parse("text\u0001电脑"))
    }
}
