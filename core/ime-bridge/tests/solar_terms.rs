//! M10-6 节日/节气提醒（搜狗 5.2 节气提示同类）：
//! 输入 jieqi → 「今日节气：xxx」（无节气日显示"今日无节气"）；
//! 输入 jieri → 公历节日（当天是节日时）。
//!
//! 验证：
//! - jieqi 必有候选且以「今日」开头（无论当天是否有节气）
//! - jieri 候选若出现则以「今日节日」开头（当天无节日时允许无候选）
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
fn 节气候选与节日候选链路() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = repo_root();
    let user_dir = root.join("target/rime-solar-terms-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理测试用户词典失败");
    }
    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    // 对照：emoji_timing 的 okok 应出 👌（确认 lua translator 链路在本引擎可用）
    assert!(session.simulate("okok"), "okok 键序未被接受");
    let okok = candidate_texts(&session);
    assert!(
        okok.iter().any(|c| c.contains("👌")),
        "对照失败：okok 应出 👌（lua 链路），实际：{okok:?}"
    );
    assert!(session.simulate("{Escape}"));
    // jieqi → 必有「今日」候选
    assert!(session.simulate("jieqi"), "jieqi 键序未被接受");
    let cands = candidate_texts(&session);
    assert!(
        cands.iter().any(|c| c.starts_with("今日")),
        "jieqi 应出「今日节气/今日无节气」候选，实际前50：{:?}",
        cands.iter().take(50).collect::<Vec<_>>()
    );

    // jieri → 有候选则必须符合「今日节日」格式；无候选允许（当天无节日）
    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("jieri"), "jieri 键序未被接受");
    let cands = candidate_texts(&session);
    for c in cands.iter().filter(|c| c.contains("今日")) {
        assert!(
            c.starts_with("今日节日"),
            "jieri 候选应为「今日节日：xxx」，实际：{c}"
        );
    }

    // 常规拼音不受影响：nihao → 你好
    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("nihao"), "nihao 键序未被接受");
    let cands = candidate_texts(&session);
    assert!(
        cands.iter().any(|c| c.contains("你好")),
        "常规输入 nihao 应出你好，实际：{cands:?}"
    );
}
