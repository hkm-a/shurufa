package com.shurufa.ime

import org.junit.Assert.assertEquals
import org.junit.Test

class SkinPaletteTest {
    @Test
    fun shared_light_skin_overrides_keyboard_and_candidate_semantics() {
        val skin = SkinPalette.fromJson(
            """
            {
              "version": 1,
              "light": {
                "keyboard": {
                  "background": "#112233",
                  "key": "#FFFFFF",
                  "key_pressed": "#000000",
                  "function_key": "#000000",
                  "function_key_pressed": "#000000",
                  "text": "#000000",
                  "function_text": "#000000",
                  "muted_text": "#778899",
                  "accent": "#ABCDEF"
                },
                "candidate": { "text": "#102030" }
              }
            }
            """.trimIndent(),
            dark = false,
            fallback = SkinPalette.lightDefault(),
        )

        assertEquals(0xFF112233.toInt(), skin.bg)
        assertEquals(0xFFABCDEF.toInt(), skin.accent)
        assertEquals(0xFF102030.toInt(), skin.candidate)
        assertEquals(0xFF778899.toInt(), skin.preedit)
    }

    @Test
    fun invalid_or_incompatible_skin_uses_fallback() {
        val fallback = SkinPalette.darkDefault()
        assertEquals(fallback, SkinPalette.fromJson("""{"version":2}""", dark = true, fallback))
        assertEquals(fallback, SkinPalette.fromJson("""{"version":1,"dark":{}}""", dark = true, fallback))
    }
}
