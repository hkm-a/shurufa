//! 候选条右键菜单的引擎链路（M7，搜狗 16.3b 候选条菜单入口同类）：
//!
//! 验证菜单动作在真实引擎上的行为：方向键把高亮移到右键项（不提交）→
//! 引擎快捷键（Control+d 删词 / Control+j 降频）作用于当前高亮候选。
//! 键序与候选窗右键菜单 dispatch 完全一致（{Down}×index + {Control+…}）。
//!
//! 依赖：schemas/lua/cold_word_drop/（processor + filter + key_binder 绑定）。

use ime_bridge::Engine;
use std::path::PathBuf;
use std::sync::Mutex;

/// librime 引擎每进程仅一个实例（ENGINE_ALIVE 全局标志），测试串行化。
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// 全新引擎 + 把 schemas/lua/ 全量拷进用户目录（含 cold_word_drop/ 子目录）。
fn fresh_engine(tag: &str) -> (Engine, PathBuf) {
    let root = repo_root();
    let user_dir = root.join(format!("target/rime-menudrop-{tag}-user-data"));
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理测试用户词典失败");
    }
    let user_lua = user_dir.join("lua");
    std::fs::create_dir_all(&user_lua).expect("创建用户 lua 目录失败");
    let src_lua = root.join("schemas/lua");
    copy_dir_recursive(&src_lua, &user_lua);
    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    (engine, user_dir)
}

fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) {
    std::fs::create_dir_all(dst).expect("创建目标 lua 目录失败");
    for entry in std::fs::read_dir(src).expect("读取 schemas/lua 失败") {
        let entry = entry.expect("读取目录项失败");
        let file_type = entry.file_type().expect("读取文件类型失败");
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path);
        } else {
            let name = entry.file_name().to_string_lossy().into_owned();
            std::fs::copy(entry.path(), &dst_path).unwrap_or_else(|_| panic!("复制 {name} 失败"));
        }
    }
}

/// 把高亮移动到第 index 个候选（0 起；方向键不提交组合）。
fn move_highlight_to(session: &ime_bridge::Session, index: usize) {
    for _ in 0..index {
        assert!(session.simulate("{Down}"), "{{Down}} 未被引擎接受");
    }
}

#[test]
fn menu_drop_candidate_via_control_d() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (engine, _dir) = fresh_engine("drop");

    let session = engine.create_session().expect("创建会话失败");
    assert!(session.simulate("nihao"), "nihao 键序未被引擎接受");

    let before = session.context().candidates;
    assert!(!before.is_empty(), "nihao 应有候选");
    let target = before[2].text.clone();

    move_highlight_to(&session, 2);
    assert_eq!(session.context().highlighted, 2, "高亮应停在右键候选");
    assert!(
        session.simulate("{Control+d}"),
        "{{Control+d}} 未被引擎接受"
    );

    let after = session.context().candidates;
    assert!(
        !after.is_empty(),
        "删词后候选不应为空（preedit={:?}）",
        session.context().preedit
    );
    assert!(
        !session.context().preedit.is_empty(),
        "删词不应破坏组合（preedit 为空说明组合被意外提交）"
    );
    assert!(
        !after.iter().any(|c| c.text == target),
        "候选「{target}」应被 Control+d 丢弃，剩余：{:?}",
        after.iter().map(|c| c.text.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn menu_demote_candidate_via_control_j() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (engine, _dir) = fresh_engine("demote");

    let session = engine.create_session().expect("创建会话失败");
    assert!(session.simulate("nihao"), "nihao 键序未被引擎接受");

    let before = session.context().candidates;
    assert!(
        before.len() >= 4,
        "nihao 应至少 4 个候选，实际 {}",
        before.len()
    );
    // 降频语义（rime-ice）：把词条排除出前 3 位、放到第 4 候选位（index 3）。
    // 目标取词候选「你好」（emoji 影子候选由 simplifier 在 filter 之后附加，
    // cold_word_drop 对 emoji 天然不可见——与按 Ctrl+J 行为一致）。
    let target = before[0].text.clone();
    assert!(
        session.simulate("{Control+j}"),
        "{{Control+j}} 未被引擎接受"
    );

    let after = session.context().candidates;
    assert!(
        !after.is_empty(),
        "降频后候选不应为空（preedit={:?}）",
        session.context().preedit
    );
    let pos = after.iter().position(|c| c.text == target);
    // 降频不变量：目标词移出首位（base 列表第 4 位；emoji 影子候选在
    // filter 之后插入，最终索引会随影子数量偏移，只断言"移出原位"）。
    assert!(
        pos.is_some_and(|p| p != 0),
        "候选「{target}」应被 Control+j 移出第 1 位，现位置：{pos:?}（剩余 {:?}）",
        after.iter().map(|c| c.text.clone()).collect::<Vec<_>>()
    );
}
