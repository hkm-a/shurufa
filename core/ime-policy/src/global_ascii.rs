//! 全局中/英状态（模拟搜狗“全局中英”语义）。
//!
//! 内存态即可：中英切换属于会话运行时状态，无需持久化。
//! 任一会话切换后，所有应用下一个按键自动跟上。

use std::sync::atomic::{AtomicBool, Ordering};

/// 线程安全的全局中/英开关。
#[derive(Debug, Default)]
pub struct GlobalAscii(AtomicBool);

impl GlobalAscii {
    /// 初始为中文态（false）。
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// 当前是否英文直输。
    pub fn load(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// 设置全局中/英状态。
    pub fn store(&self, value: bool) {
        self.0.store(value, Ordering::Relaxed);
    }

    /// 翻转并返回切换后的状态。
    pub fn toggle(&self) -> bool {
        let next = !self.load();
        self.store(next);
        next
    }
}

/// 进程内共享的全局中/英状态。
pub static GLOBAL_ASCII: GlobalAscii = GlobalAscii::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_ascii_defaults_to_chinese() {
        assert!(!GLOBAL_ASCII.load());
    }

    #[test]
    fn global_ascii_store_and_toggle() {
        let g = GlobalAscii::new();
        assert!(!g.load());
        g.store(true);
        assert!(g.load());
        assert!(!g.toggle(), "true 翻转后应回到中文态 false");
        assert!(!g.load());
        assert!(g.toggle(), "false 翻转后应进入英文态 true");
        assert!(g.load());
    }
}
