package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** P1-3 候选表情混排映射的纯逻辑验证。 */
class EmojiSuggestionTest {
    @Test
    fun 常见词有表情() {
        assertEquals("😊", EmojiSuggestion.emojiFor("我"))
        assertEquals("👑", EmojiSuggestion.emojiFor("王"))
        assertEquals("❤️", EmojiSuggestion.emojiFor("爱"))
        assertEquals("🐱", EmojiSuggestion.emojiFor("猫"))
    }

    @Test
    fun 未收录词无表情() {
        assertNull(EmojiSuggestion.emojiFor("的"))
        assertNull(EmojiSuggestion.emojiFor(""))
        assertNull(EmojiSuggestion.emojiFor("zzzz"))
    }
}
