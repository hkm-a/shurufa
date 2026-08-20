package com.shurufa.ime

/**
 * P1-3 借鉴搜狗候选表情混排：常见词 → 表情映射，候选词后追加表情装饰。
 * 纯函数便于 JVM 单测。保持引擎候选索引不动（表情仅作视觉后缀，
 * 点击仍选词，避免与 nativeSelectCandidate 的索引错位）。
 */
object EmojiSuggestion {
    private val map: Map<String, String> = mapOf(
        "我" to "😊", "你" to "🤝", "好" to "👍", "爱" to "❤️", "笑" to "😄",
        "哭" to "😭", "王" to "👑", "心" to "💖", "花" to "🌸", "雨" to "🌧️",
        "太阳" to "☀️", "月亮" to "🌙", "火" to "🔥", "水" to "💧", "车" to "🚗",
        "飞机" to "✈️", "鱼" to "🐟", "猫" to "🐱", "狗" to "🐶", "谢谢" to "🙏",
        "厉害" to "💪", "棒" to "👏", "生日" to "🎂", "礼物" to "🎁", "加油" to "💪",
        "晚安" to "🌙", "早安" to "🌅", "开心" to "😁", "生气" to "😡", "累" to "😫",
        "吃饭" to "🍚", "水果" to "🍎", "啤酒" to "🍺", "咖啡" to "☕", "奶茶" to "🧋",
        "游戏" to "🎮", "音乐" to "🎵", "电影" to "🎬", "看书" to "📖", "睡觉" to "😴",
        "OK" to "👌", "好呀" to "😊", "拜拜" to "👋", "哈哈" to "😂", "嗯嗯" to "😌",
    )

    /** 词 → 表情；无映射返回 null。 */
    fun emojiFor(word: String): String? = map[word]
}
