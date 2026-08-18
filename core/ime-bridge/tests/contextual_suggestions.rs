//! M7-8 上下文调频（搜狗 16.6 打字模型同类方向）：translator 开启
//! `contextual_suggestions: true` 后，上屏上文对紧随其后的候选加权。
//!
//! 验证（单引擎单会话，避免 ENGINE_ALIVE 单实例限制）：
//! 无上文输入 renmin 时「人民」位置 p0；先上屏「中国」再输入 renmin
//! 时「人民」位置 p1 —— 断言 p1 <= p0（语境加权不应让词条更靠后）。

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

#[test]
fn contextual_suggestions_does_not_worsen_rank() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = repo_root();
    let user_dir = root.join("target/rime-context-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理测试用户词典失败");
    }
    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    // 1) 无上文基线
    assert!(session.simulate("renmin"), "renmin 键序未被引擎接受");
    let p0 = session
        .context()
        .candidates
        .iter()
        .position(|c| c.text.contains("人民"));
    assert!(p0.is_some(), "无上文时 renmin 应出 人民");

    // 2) 清空组合 → 上屏「中国」→ 再输入 renmin
    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("zhongguo"), "zhongguo 键序未被引擎接受");
    assert!(session.simulate(" "), "空格提交未被引擎接受");
    assert!(
        session.context().preedit.is_empty(),
        "上屏后组合应清空，实际 preedit={:?}",
        session.context().preedit
    );
    assert!(session.simulate("renmin"), "renmin 键序未被引擎接受");
    let p1 = session
        .context()
        .candidates
        .iter()
        .position(|c| c.text.contains("人民"));
    assert!(p1.is_some(), "有上文时 renmin 仍应出 人民");

    eprintln!("CONTEXT: p0={p0:?} p1={p1:?}（人民 无上文 / 中国后）");
    assert!(
        p1.unwrap() <= p0.unwrap(),
        "语境加权不应让「人民」更靠后：p0={p0:?} p1={p1:?}"
    );
}
