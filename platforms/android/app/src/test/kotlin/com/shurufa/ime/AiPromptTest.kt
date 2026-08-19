package com.shurufa.ime

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** M-A4 AI 提示词构造验证（模式 / 风格 / 场景）。 */
class AiPromptTest {

    @Test
    fun 帮写默认提示包含输出约定() {
        val sys = AiPrompt.systemFor(AiMode.WRITE)
        assertTrue(sys.contains("AI 帮写"))
        assertTrue(sys.contains("不要解释"))
    }

    @Test
    fun 风格与场景指令注入() {
        val sys = AiPrompt.systemFor(AiMode.WRITE, AiStyle.HUMOR, AiScene.MOMENT)
        assertTrue(sys.contains("幽默"))
        assertTrue(sys.contains("朋友圈"))
        assertFalse(AiPrompt.styleDirective(AiStyle.DEFAULT).isNotEmpty())
        assertFalse(AiPrompt.sceneDirective(AiScene.DEFAULT).isNotEmpty())
    }

    @Test
    fun 各模式提示相互区分() {
        assertTrue(AiPrompt.systemFor(AiMode.QUICK).contains("快查"))
        assertTrue(AiPrompt.systemFor(AiMode.TRANSLATE).contains("翻译"))
        assertTrue(AiPrompt.systemFor(AiMode.POLISH).contains("书面语"))
        assertTrue(AiPrompt.systemFor(AiMode.WRITE, scene = AiScene.LONG).contains("600 字"))
    }

    @Test
    fun 长文场景模板() {
        assertTrue(AiPrompt.sceneDirective(AiScene.LONG).contains("600 字"))
        assertTrue(AiPrompt.sceneDirective(AiScene.TITLE).contains("标题"))
    }
}
