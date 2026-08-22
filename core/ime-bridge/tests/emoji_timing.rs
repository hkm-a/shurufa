//! M7-9 多时机表情推荐（搜狗 15.9「输入 okok 出表情」同类）：
//! lua_translator 对精确输入码附加 emoji 候选。
//!
//! 验证：
//! - okok → 候选含 👌（OpenCC 管不到的非中文词触发）
//! - wanan → 候选含 🌙
//! - 常规输入不受影响：nihao → 你好

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

fn fresh_engine(tag: &str) -> Engine {
    let root = repo_root();
    let user_dir = root.join(format!("target/rime-emojitiming-{tag}-user-data"));
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理测试用户词典失败");
    }
    Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败")
}

fn candidate_texts(session: &ime_bridge::Session) -> Vec<String> {
    session
        .context()
        .candidates
        .iter()
        .take(9)
        .map(|c| c.text.clone())
        .collect::<Vec<_>>()
}

#[test]
fn emoji_timing_triggers() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let engine = fresh_engine("okok");

    let session = engine.create_session().expect("创建会话失败");

    // okok → 👌
    assert!(session.simulate("okok"), "okok 键序未被引擎接受");
    let cands = candidate_texts(&session);
    assert!(
        cands.iter().any(|t| t.contains("👌")),
        "okok 应出 👌 候选，实际：{cands:?}"
    );

    // wanan（晚安）→ 🌙
    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("wanan"), "wanan 键序未被引擎接受");
    let cands = candidate_texts(&session);
    assert!(
        cands.iter().any(|t| t.contains("🌙")),
        "wanan 应出 🌙 候选，实际：{cands:?}"
    );

    // 常规输入不受影响
    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("nihao"), "nihao 键序未被引擎接受");
    let cands = candidate_texts(&session);
    assert!(
        cands.iter().any(|t| t.contains("你好")),
        "nihao 仍应出 你好，实际：{cands:?}"
    );
}
