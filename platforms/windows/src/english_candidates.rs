//! 英文候选（M7-5 候选 Tab 多服务切换 · P2 英文 Tab）。
//!
//! 基于内置高频英文词表做前缀联想：输入串为 ≥2 位 ASCII 字母时，返回
//! 以该串开头的英文词（按长度升序，最多 [MAX_SUGGESTIONS] 个）。纯本地、
//! 零网络，单次全量扫描词表（数百词，微秒级）即可，无需索引。
//!
//! 交互：候选窗 Tab 行显示「英文」标签（有英文候选时）；点击英文候选
//! 走 AI_COMMIT 式钩子提交（不走引擎数字选词）。默认 Tab 仍是拼音，
//! 英文候选不挤占 Rime 槽位。

/// 英文候选最大条数（与 AI 候选一致，避免候选行过宽）。
pub const MAX_SUGGESTIONS: usize = 5;

/// 内置高频英文词表（手写常用词 + 高频功能词；v1 精简版，后续可扩展）。
/// 全部小写；`suggest` 输入按小写匹配。
const WORDS: &[&str] = &[
    // 功能词（最高频）
    "the", "be", "to", "of", "and", "a", "in", "that", "have", "it", "for", "not", "on",
    "with", "he", "as", "you", "do", "at", "this", "but", "his", "by", "from", "they", "we",
    "say", "her", "she", "or", "an", "will", "my", "one", "all", "would", "there", "their",
    "what", "so", "up", "out", "if", "about", "who", "get", "which", "go", "me", "when",
    "make", "can", "like", "time", "no", "just", "him", "know", "take", "people", "into",
    "year", "your", "good", "some", "could", "them", "see", "other", "than", "then", "now",
    "look", "only", "come", "its", "over", "think", "also", "back", "after", "use", "two",
    "how", "our", "work", "first", "well", "way", "even", "new", "want", "because", "any",
    "these", "give", "day", "most", "us",
    // 常用动词（含常见变位）
    "have", "has", "had", "will", "would", "can", "could", "should", "must", "may", "might",
    "do", "does", "did", "done", "going", "went", "gone", "come", "came", "coming", "take",
    "took", "taken", "give", "gave", "given", "made", "making", "see", "saw", "seen", "know",
    "knew", "known", "think", "thought", "got", "put", "set", "let", "said", "tell", "told",
    "ask", "asked", "helped", "played", "run", "ran", "running", "read", "reading", "write",
    "wrote", "written", "speak", "spoke", "spoken", "learn", "learned", "study", "studied",
    "teach", "taught", "bring", "brought", "buy", "bought", "call", "called", "change",
    "changed", "find", "found", "hear", "heard", "hold", "held", "keep", "kept", "leave",
    "left", "live", "lived", "look", "looked", "love", "loved", "move", "moved", "need",
    "needed", "open", "opened", "pay", "paid", "play", "played", "put", "reach", "reached",
    "remember", "remembered", "run", "send", "sent", "show", "showed", "shown", "sit", "sat",
    "sleep", "slept", "stand", "stood", "start", "started", "stay", "stayed", "stop", "stopped",
    "talk", "talked", "think", "try", "tried", "turn", "turned", "understand", "understood",
    "use", "used", "wait", "waited", "walk", "walked", "want", "wanted", "watch", "watched",
    "win", "won", "work", "worked", "worry", "worried",
    // 常用名词/形容词/副词
    "hello", "world", "thanks", "thank", "please", "sorry", "okay", "ok", "yes", "hi",
    "help", "home", "school", "work", "meeting", "lunch", "dinner", "breakfast", "coffee",
    "water", "food", "friend", "family", "happy", "sad", "great", "nice", "beautiful",
    "amazing", "welcome", "good", "morning", "afternoon", "evening", "night", "today",
    "tomorrow", "yesterday", "week", "month", "year", "hour", "minute", "second", "day",
    "time", "date", "phone", "email", "address", "message", "call", "text", "photo",
    "picture", "video", "music", "movie", "book", "news", "search", "google", "baidu",
    "apple", "windows", "linux", "phone", "computer", "keyboard", "mouse", "screen",
    "window", "file", "edit", "view", "close", "cancel", "retry", "save", "delete",
    "copy", "paste", "cut", "print", "download", "upload", "login", "logout", "password",
    "user", "name", "number", "money", "price", "cost", "free", "expensive", "cheap",
    "big", "small", "large", "little", "long", "short", "high", "low", "fast", "slow",
    "new", "old", "young", "hot", "cold", "warm", "cool", "clean", "dirty", "easy",
    "hard", "simple", "difficult", "important", "interesting", "boring", "fun", "boring",
    "right", "wrong", "true", "false", "yes", "no", "maybe", "sure", "really", "very",
    "too", "also", "only", "just", "still", "already", "yet", "ever", "never", "always",
    "often", "sometimes", "usually", "early", "late", "soon", "now", "here", "there",
    "everywhere", "inside", "outside", "above", "below", "before", "after", "during",
    "between", "among", "through", "around", "behind", "front", "back", "left", "right",
    "top", "bottom", "middle", "begin", "end", "start", "finish", "continue", "stop",
    "open", "close", "enter", "exit", "leave", "arrive", "depart", "travel", "visit",
    "meet", "join", "leave", "stay", "return", "come", "go", "walk", "run", "jump",
    "sit", "stand", "lie", "sleep", "wake", "eat", "drink", "cook", "bake", "order",
    "buy", "sell", "pay", "spend", "save", "earn", "lose", "find", "search", "look",
    "watch", "see", "listen", "hear", "say", "speak", "talk", "tell", "ask", "answer",
    "question", "problem", "solution", "idea", "plan", "goal", "dream", "hope", "wish",
    "want", "need", "like", "love", "hate", "prefer", "choose", "decide", "think",
    "believe", "know", "understand", "learn", "study", "teach", "read", "write",
    "draw", "paint", "sing", "dance", "play", "sport", "game", "team", "win", "lose",
    "draw", "score", "goal", "ball", "football", "basketball", "tennis", "swimming",
    "running", "walking", "health", "doctor", "hospital", "medicine", "pain", "sick",
    "healthy", "strong", "weak", "tired", "energy", "sleep", "rest", "work", "job",
    "boss", "colleague", "office", "company", "business", "market", "money", "bank",
    "account", "card", "cash", "coin", "pay", "buy", "sell", "price", "cheap", "expensive",
    "rich", "poor", "happy", "sad", "angry", "afraid", "surprised", "excited", "bored",
    "tired", "lonely", "proud", "nervous", "calm", "brave", "careful", "careless", "kind",
    "friendly", "polite", "rude", "honest", "lazy", "busy", "free", "available", "ready",
    "able", "unable", "possible", "impossible", "certain", "sure", "clear", "obvious",
    "correct", "wrong", "same", "different", "similar", "equal", "enough", "plenty",
    "few", "several", "many", "much", "more", "most", "less", "least", "some", "any",
    "each", "every", "both", "either", "neither", "other", "another", "next", "last",
    "first", "second", "third", "final", "main", "major", "minor", "general", "special",
    "particular", "specific", "common", "rare", "usual", "normal", "strange", "weird",
    "funny", "serious", "important", "necessary", "useful", "useless", "beautiful", "ugly",
    "pretty", "handsome", "cute", "sweet", "bitter", "sour", "salty", "delicious", "tasty",
    "hungry", "thirsty", "full", "empty", "clean", "dirty", "wet", "dry", "soft", "hard",
    "smooth", "rough", "sharp", "dull", "bright", "dark", "light", "heavy", "thin", "thick",
    "wide", "narrow", "deep", "shallow", "long", "short", "tall", "high", "low", "big",
    "small", "large", "tiny", "huge", "giant", "enormous", "massive", "tiny",
];

/// 英文候选联想（纯函数）：输入串为 ≥2 位 ASCII 字母时返回前缀匹配词
/// （按长度升序，最多 MAX_SUGGESTIONS 个）；否则返回空。
///
/// 大小写不敏感匹配（输入 "Hel"/"hel" 均匹配 hello）；返回词表原样（小写）。
pub fn suggest(preedit: &str) -> Vec<String> {
    let p = preedit.trim().to_ascii_lowercase();
    if p.len() < 2 || !p.chars().all(|c| c.is_ascii_alphabetic()) {
        return Vec::new();
    }
    let mut hits: Vec<&str> = WORDS
        .iter()
        .copied()
        .filter(|w| w.starts_with(&p))
        .collect();
    // 长度升序（短词优先，如 "hi" 优先于 "high"）；同长按词表顺序稳定
    hits.sort_by_key(|w| w.len());
    hits.truncate(MAX_SUGGESTIONS);
    hits.into_iter().map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_match_hello() {
        let got = suggest("hel");
        assert!(got.contains(&"hello".to_owned()), "hel 应命中 hello: {got:?}");
        assert!(got.iter().all(|w| w.starts_with("hel")));
        assert!(got.len() <= MAX_SUGGESTIONS);
    }

    #[test]
    fn prefix_shortest_first() {
        let got = suggest("hi");
        assert_eq!(got.first().map(String::as_str), Some("hi"), "hi 应排最前: {got:?}");
        assert!(got.iter().all(|w| w.starts_with("hi")));
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(suggest("Hel"), suggest("hel"));
        assert!(suggest("THAN").contains(&"thanks".to_owned()));
    }

    #[test]
    fn too_short_or_non_alpha_empty() {
        assert!(suggest("").is_empty());
        assert!(suggest("h").is_empty(), "单字母不联想（避免噪音）");
        assert!(suggest("ni1hao").is_empty());
        assert!(suggest("你好").is_empty());
        assert!(suggest("nihao").is_empty(), "拼音串无英文前缀匹配时为空");
    }

    #[test]
    fn no_match_empty() {
        assert!(suggest("xyzq").is_empty());
    }

    #[test]
    fn max_cap() {
        let got = suggest("a");
        // "a" 是单字母不联想；用两字母前缀验证上限
        let got = suggest("an");
        assert!(got.len() <= MAX_SUGGESTIONS);
        assert!(got.iter().all(|w| w.starts_with("an")));
    }
}
