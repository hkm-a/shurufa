//! 中英混输自动空格（rime-ice en_spacer.lua 同款）集成测试。
//!
//! 验证：
//! - hello 上屏后，再输入 world → world 候选带前导空格（" world"）
//! - 首次输入英文（无 commit_history）→ 不加空格（passthrough）
//! - 开关 en_spacer=false 时 → 不加空格
//!
//! 依赖：schemas/lua/en_spacer.lua + rime_ice.schema.yaml 的
//! lua_filter@*en_spacer + switches/en_spacer（reset: 1）。

use ime_bridge::Engine;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// 递归拷贝 lua 脚本目录（含 cold_word_drop/ 子目录）。
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

fn candidates_contain(session: &ime_bridge::Session<'_>, text: &str) -> bool {
    session.context().candidates.iter().any(|c| c.text == text)
}

fn candidate_texts(session: &ime_bridge::Session<'_>) -> Vec<String> {
    session
        .context()
        .candidates
        .iter()
        .map(|c| c.text.clone())
        .collect()
}

#[test]
fn 中英混输英文词自动加前导空格() {
    let root = repo_root();
    let user_dir = root.join("target/rime-en-spacer-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理空格测试用户词典失败");
    }
    std::fs::create_dir_all(user_dir.join("lua")).expect("创建 lua 目录失败");
    copy_dir_recursive(&root.join("schemas/lua"), &user_dir.join("lua"));
    let engine: &'static ime_bridge::Engine = Box::leak(Box::new(
        Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败"),
    ));
    let session = engine.create_session().expect("创建会话失败");
    session.set_option("emoji", false);
    // 开关默认开（reset: 1）
    assert!(session.get_option("en_spacer"), "en_spacer 默认应开启");

    // 1) 无 commit_history：首次输入 hello → 候选不加空格（passthrough）
    assert!(session.simulate("hello"), "hello 键序未被引擎接受");
    assert!(
        candidates_contain(&session, "hello"),
        "首次输入 hello 候选应为原样 hello：{:?}",
        candidate_texts(&session)
    );
    assert!(
        !candidates_contain(&session, " hello"),
        "无 commit_history 时不应加前导空格：{:?}",
        candidate_texts(&session)
    );
    // 上屏 hello（选中当前页第 0 项）
    session.simulate("{Escape}");
    session.simulate("hello");
    let committed = session.select_candidate_on_current_page(0);
    assert!(committed, "选中 hello 候选失败");
    assert_eq!(session.commit().as_deref(), Some("hello"));

    // 2) 上次上屏英文 + 本次英文候选 → 自动加前导空格
    assert!(session.simulate("world"), "world 键序未被引擎接受");
    assert!(
        candidates_contain(&session, " world"),
        "hello 后输入 world，候选应带前导空格（en_spacer）：{:?}",
        candidate_texts(&session)
    );
    session.simulate("{Escape}");

    // 3) 开关关闭 → 不再加空格
    session.set_option("en_spacer", false);
    // 先再上屏一个英文词维持 commit_history
    session.simulate("hello");
    let _ = session.select_candidate_on_current_page(0);
    assert!(session.simulate("world"), "world 键序未被引擎接受");
    assert!(
        !candidates_contain(&session, " world"),
        "en_spacer 关闭后不应加前导空格：{:?}",
        candidate_texts(&session)
    );
    session.simulate("{Escape}");
}
