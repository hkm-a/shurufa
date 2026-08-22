//! 农历/节气链路（lunar.lua + lunar.db）：
//! `nl` 触发今日农历（含二十四节气附注，当天有节气时 comment 带「节气」）。
//! solar_terms.lua 已删除——它的近似公式与 lunar.db 重复，且误差可达 ±1 天。
//!
//! 验证：
//! - `nl` 必有今日农历候选（含「年」）
//! - 常规拼音不受影响

use ime_bridge::Engine;
use std::path::PathBuf;
use std::sync::Mutex;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn candidate_texts(session: &ime_bridge::Session) -> Vec<String> {
    session
        .context()
        .candidates
        .iter()
        .map(|c| c.text.clone())
        .collect::<Vec<_>>()
}

#[test]
fn 农历链路与常规拼音不受影响() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = repo_root();
    let user_dir = root.join("target/rime-solar-terms-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理测试用户词典失败");
    }
    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    // 对照：emoji_timing 的 okok 应出 OK 手势（确认 lua translator 链路在本引擎可用）
    assert!(session.simulate("okok"), "okok 键序未被接受");
    let okok = candidate_texts(&session);
    assert!(
        okok.iter().any(|c| c.contains("\u{1F44C}")),
        "对照失败：okok 应出 OK 手势（lua 链路），实际：{okok:?}"
    );
    assert!(session.simulate("{Escape}"));

    // nl → 今日农历/节气候选（必有「年」）
    assert!(session.simulate("nl"), "nl 键序未被接受");
    let cands = candidate_texts(&session);
    assert!(
        cands.iter().any(|c| c.contains("年")),
        "nl 应出今日农历候选（含「年」），实际前50：{:?}",
        cands.iter().take(50).collect::<Vec<_>>()
    );

    // 常规拼音不受影响：nihao → 你好
    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("nihao"), "nihao 键序未被接受");
    let cands = candidate_texts(&session);
    assert!(
        cands.iter().any(|c| c.contains("你好")),
        "常规输入 nihao 应出你好，实际：{cands:?}"
    );
}
