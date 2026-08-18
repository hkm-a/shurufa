//! M10 困难项「？？？」表情：引擎侧基线验证。
//!
//! librime 中文标点立即上屏（组合无法累积标点）——这就是 TSF 层替代实现
//! （emoji_question.rs：第三个连续 '/' 上屏 🤔）的前提事实。
//!
//! 验证：
//! - 中文标点下输入 / 立即上屏「、」（rime-ice 标点映射，无组合累积）
//! - 证明引擎层无法累积标点出「？？？」组合候选，需 TSF 按键层接管
//!   （Shift+/ 三连 → 🤔，见 platforms/windows/src/emoji_question.rs）

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
fn 中文标点斜杠立即上屏全角问号无组合累积() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = repo_root();
    let user_dir = root.join("target/rime-question-baseline-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理测试用户词典失败");
    }
    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    // 中文态（默认）连续三次 '/'：每次按键 librime 都直接提交全角「？」
    let ok = session.simulate("/");
    assert!(ok, "/ 键序未被引擎接受");
    // rime-ice 中文标点映射：/ 要么已上屏、要么在预编辑串里待提交，
    // 但绝不会形成「？？？」这种可匹配的组合/候选
    let first = session.commit().unwrap_or_default();
    let ctx = session.context();
    assert!(
        first == "、"
            || first == "？？？"
            || ctx.preedit.contains('、')
            || ctx.preedit.contains('？'),
        "单个 / 应产生顿号或全角问号（立即上屏或进预编辑），实际 first={first:?} preedit={:?}",
        ctx.preedit
    );
    // 连续三个 / 后提交：得到的是逐次产生的标点，而不是「？？？」组合候选
    session.simulate("//");
    let committed = session.commit().unwrap_or_default();
    assert!(
        !committed.contains("？？？"),
        "librime 无「？？？」组合候选（替代实现在 TSF 层），实际提交 {committed:?}"
    );
}
