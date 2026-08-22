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

    @Test
    fun v2_metrics_and_shadow_are_parsed() {
        val skin = SkinPalette.fromJson(
            """
            {
              "version": 2,
              "light": {
                "keyboard": { "background": "#112233" },
                "candidate": {
                  "text": "#102030",
                  "background": "#F0F0F0",
                  "highlight_background": "#D6EBE1",
                  "label": "#778899",
                  "preedit": "#AABBCC"
                },
                "metrics": {
                  "radius": 10,
                  "font_scale": 1.25,
                  "opacity": 0.9,
                  "scrollbar": false
                }
              },
              "shadow": { "enabled": true, "radius": 18, "alpha": 64 }
            }
            """.trimIndent(),
            dark = false,
            fallback = SkinPalette.lightDefault(),
        )

        assertEquals(0xFFF0F0F0.toInt(), skin.candidateBackground)
        assertEquals(0xFFD6EBE1.toInt(), skin.candidateHighlight)
        assertEquals(0xFF778899.toInt(), skin.candidateLabel)
        assertEquals(0xFFAABBCC.toInt(), skin.candidatePreedit)
        assertEquals(10, skin.metricsRadius)
        assertEquals(1.25f, skin.metricsFontScale)
        assertEquals(0.9f, skin.metricsOpacity)
        assertEquals(false, skin.metricsScrollbar)
        assertEquals(true, skin.shadowEnabled)
        assertEquals(18, skin.shadowRadius)
        assertEquals(64, skin.shadowAlpha)
    }
