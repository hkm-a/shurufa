//! 打字统计埋点薄封装：统一由本层调用 `shurufa_options::stats`。

/// 记录一次按键计数。
pub fn note_key() {
    shurufa_options::stats::note_keys(1);
}

/// 记录一次上屏的字符数（空串不计）。
pub fn note_commit(text: &str) {
    let n = text.chars().count();
    if n > 0 {
        shurufa_options::stats::note_chars(n);
    }
}
