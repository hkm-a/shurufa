//! 应用/网站直达（M8-4，搜狗 15.2 灵犀候选直达同类）：
//! 提交带标记的候选（🖥 应用 / 🌐 网址）时启动目标且不把标记文本落进文档。
//! 标记由 schemas/lua/app_direct.lua 生成，目标来自 app-shortcuts.json。

use shurufa_options::{AppShortcut, AppShortcutKind};

/// 解析提交文本 → 直达目标（None = 普通文本，走正常上屏）。
pub fn resolve_commit(text: &str) -> Option<AppShortcut> {
    let (label, kind) = text
        .strip_prefix("🖥 ")
        .map(|label| (label, AppShortcutKind::App))
        .or_else(|| {
            text.strip_prefix("🌐 ")
                .map(|label| (label, AppShortcutKind::Url))
        })?;
    shurufa_options::app_shortcuts::load()
        .entries
        .into_iter()
        .find(|s| s.label == label && s.kind == kind)
}

/// 启动目标：应用直接执行；网址交给默认浏览器（cmd start）。
pub fn spawn_target(shortcut: &AppShortcut) -> std::io::Result<()> {
    match shortcut.kind {
        AppShortcutKind::App => std::process::Command::new(&shortcut.target)
            .spawn()
            .map(|_| ()),
        AppShortcutKind::Url => std::process::Command::new("cmd")
            .args(["/c", "start", "", &shortcut.target])
            .spawn()
            .map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 标记前缀解析() {
        assert!(resolve_commit("普通文本").is_none());
        assert!(resolve_commit("").is_none());
        // 非真实配置的标记也会走到 load()（返回 None = 配置缺失），
        // 这里只验证前缀判定 + 配置查找的路径不 panic。
        let _ = resolve_commit("🖥 不存在");
        let _ = resolve_commit("🌐 不存在");
    }
}
