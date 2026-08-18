//! 「？？？」表情推荐（搜狗 15.9 同类，TSF 层替代实现）。
//!
//! librime 中文标点立即上屏（组合无法累积标点），TSF 自建组合会与引擎
//! 抢 composition，回退删除又侵入用户文档——因此采用零风险替代：
//! 中文态空组合下连续按 `/` 时，前两个上屏全角「？」，第三个上屏 🤔
//! （文档呈「？？🤔」，语义等同"疑惑"推荐）；再按继续循环，任何其他键重置。

pub const EMOJI: &str = "🤔";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuestionState {
    count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionAction {
    /// 上屏一个全角问号，计数 +1
    EmitQuestion,
    /// 上屏 🤔（第三个连续问号），计数重置
    EmitEmoji,
}

impl QuestionState {
    pub fn on_slash(&mut self) -> QuestionAction {
        self.count += 1;
        if self.count >= 3 {
            self.count = 0;
            QuestionAction::EmitEmoji
        } else {
            QuestionAction::EmitQuestion
        }
    }

    pub fn reset(&mut self) {
        self.count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 三连问号第三个替换为表情() {
        let mut state = QuestionState::default();
        assert_eq!(state.on_slash(), QuestionAction::EmitQuestion);
        assert_eq!(state.on_slash(), QuestionAction::EmitQuestion);
        assert_eq!(state.on_slash(), QuestionAction::EmitEmoji);
        // 重置后重新计数
        assert_eq!(state.on_slash(), QuestionAction::EmitQuestion);
    }

    #[test]
    fn 非问号键重置计数() {
        let mut state = QuestionState::default();
        let _ = state.on_slash();
        state.reset();
        assert_eq!(state.on_slash(), QuestionAction::EmitQuestion);
    }
}
