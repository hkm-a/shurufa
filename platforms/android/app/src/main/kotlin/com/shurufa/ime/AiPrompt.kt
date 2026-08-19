package com.shurufa.ime

/** M-A4 AI 面板模式（搜狗 11.48/11.49 AI 帮写 / 11.49 AI 快查 / 20.2 转写书面语 / 8.21 翻译）。 */
enum class AiMode(val id: String, val label: String) {
    WRITE("write", "帮写"),
    QUICK("quick", "快查"),
    TRANSLATE("translate", "翻译"),
    POLISH("polish", "书面语化"),
}

/** 帮写风格（搜狗 11.49 自定义风格：幽默/文学/极简/古风/搞怪）。 */
enum class AiStyle(val id: String, val label: String) {
    DEFAULT("default", "默认"),
    HUMOR("humor", "幽默"),
    LITERARY("literary", "文学"),
    MINIMAL("minimal", "极简"),
    ANCIENT("ancient", "古风"),
    FUNKY("funky", "搞怪"),
}

/** 帮写场景模板（搜狗 11.48 长文/种草笔记/爆款标题/高赞朋友圈）。 */
enum class AiScene(val id: String, val label: String) {
    DEFAULT("default", "通用"),
    MOMENT("moment", "朋友圈"),
    NOTE("note", "种草笔记"),
    TITLE("title", "爆款标题"),
    CHAT("chat", "聊天润色"),
    LONG("long", "长文"),
}

/**
 * AI 提示词构造（纯函数便于 JVM 单测）：按模式 + 风格 + 场景生成 system 提示。
 * 与 PC 端一致约定：直接输出可粘贴文本，不解释、不用 Markdown 代码块。
 */
object AiPrompt {

    fun styleDirective(style: AiStyle): String = when (style) {
        AiStyle.DEFAULT -> ""
        AiStyle.HUMOR -> "用幽默轻松的语气。"
        AiStyle.LITERARY -> "用文学化的表达，适当使用修辞。"
        AiStyle.MINIMAL -> "极简风格，惜字如金。"
        AiStyle.ANCIENT -> "用古风雅致的措辞。"
        AiStyle.FUNKY -> "用搞怪俏皮的网络化表达。"
    }

    fun sceneDirective(scene: AiScene): String = when (scene) {
        AiScene.DEFAULT -> ""
        AiScene.MOMENT -> "输出适合发朋友圈的文案，语气自然带点松弛感。"
        AiScene.NOTE -> "输出种草笔记，突出使用体验与推荐理由。"
        AiScene.TITLE -> "输出几个爆款标题候选，简洁抓眼球。"
        AiScene.CHAT -> "把输入润色成适合聊天的自然回复。"
        AiScene.LONG -> "展开写成较长的段落（600 字以上），结构完整。"
    }

    fun systemFor(mode: AiMode, style: AiStyle = AiStyle.DEFAULT, scene: AiScene = AiScene.DEFAULT): String =
        when (mode) {
            AiMode.WRITE -> buildString {
                append("你是用户输入法里的 AI 帮写助手。直接输出可粘贴的中文段落，不要解释、不要 Markdown 代码块；除非用户另有要求，控制在 300 字以内。")
                styleDirective(style).takeIf { it.isNotEmpty() }?.let { append(it) }
                sceneDirective(scene).takeIf { it.isNotEmpty() }?.let { append(it) }
            }
            AiMode.QUICK -> "你是 AI 快查助手。用户会输入诗词、汇率换算、公式、天气等查询，直接给出简洁准确的答案，不要解释过程，可直接粘贴，300 字以内。"
            AiMode.TRANSLATE -> "你是翻译助手。把用户输入翻译成目标语言（用户会说“翻译成X”；未指定时中译英/英译中自动判断），只输出译文，不要解释。"
            AiMode.POLISH -> "你是书面语润色助手。把用户输入的口语表达转换成规范书面语，保持原意与情绪，只输出润色结果，不要解释。"
        }
}
