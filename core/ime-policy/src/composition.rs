//! 超长组合防护（weasel#649 同类，2026-08-16）。
//!
//! 组合输入串达到阈值时，调用方应在喂下一键前清空组合，让超长串转纯字母
//! 直通，防止 librime translator 在超大音节图上爆炸（内存/CPU 暴涨导致
//! 按键卡死）。正常整句输入（“zhonghuarenmingongheguo” 21 码）远低于阈值。

/// 超长组合阈值。
pub const MAX_COMPOSITION_LEN: usize = 64;

/// 组合长度是否已达超长阈值（需在下一键前清空）。
pub fn is_overlong_composition(input_len: usize) -> bool {
    input_len >= MAX_COMPOSITION_LEN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlong_composition_threshold() {
        assert!(!is_overlong_composition(0));
        assert!(!is_overlong_composition(21)); // "zhonghuarenmingongheguo"
        assert!(!is_overlong_composition(63));
        assert!(is_overlong_composition(64));
        assert!(is_overlong_composition(100));
        assert_eq!(MAX_COMPOSITION_LEN, 64);
    }
}
