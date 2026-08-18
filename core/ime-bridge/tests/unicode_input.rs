//! Unicode 输入（rime-ice unicode.lua 同款）集成测试。
//!
//! 验证：
//! - U4F60 → 你（BMP 内，附带 U4F60~U4F6F 变体候选）
//! - U1F600 → 😀（增补平面 emoji）
//! - U03B1 → α（希腊字母）
//! - 非 hex 输入（Uxyz）不触发（pass-through 不破坏其它候选）
//!
//! 依赖：schemas/lua/unicode.lua + rime_ice.schema.yaml 的
//! lua_translator@*unicode + recognizer/patterns/unicode: "^U[a-f0-9]+"。
//!
//! 注意：librime 引擎每进程仅一个实例（ENGINE_ALIVE 全局标志），
//! 全部场景在同一个引擎上顺序执行、各自新建会话，避免并行冲突。

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
fn unicode输入输出码点对应字符() {
    let root = repo_root();
    let user_dir = root.join("target/rime-unicode-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理 Unicode 测试用户词典失败");
    }
    std::fs::create_dir_all(user_dir.join("lua")).expect("创建 lua 目录失败");
    copy_dir_recursive(&root.join("schemas/lua"), &user_dir.join("lua"));
    let engine: &'static ime_bridge::Engine = Box::leak(Box::new(
        Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败"),
    ));
    let session = engine.create_session().expect("创建会话失败");
    // 关 emoji，避免 OpenCC 候选干扰 Unicode 结果
    session.set_option("emoji", false);

    // 1) U4F60 → 你（BMP）
    assert!(session.simulate("U4F60"), "U4F60 键序未被引擎接受");
    let ctx0 = session.context();
    println!(
        "U4F60 preedit={:?} candidates={:?}",
        ctx0.preedit,
        candidate_texts(&session)
    );
    assert!(
        candidates_contain(&session, "你"),
        "U4F60 未输出 你：{:?}",
        candidate_texts(&session)
    );
    session.simulate("{Escape}");

    // 2) U1F600 → 😀（增补平面 emoji）
    assert!(session.simulate("U1F600"), "U1F600 键序未被引擎接受");
    assert!(
        candidates_contain(&session, "😀"),
        "U1F600 未输出 😀：{:?}",
        candidate_texts(&session)
    );
    session.simulate("{Escape}");

    // 3) U03B1 → α（希腊字母）
    assert!(session.simulate("U03B1"), "U03B1 键序未被引擎接受");
    assert!(
        candidates_contain(&session, "α"),
        "U03B1 未输出 α：{:?}",
        candidate_texts(&session)
    );
    session.simulate("{Escape}");

    // 4) 非 hex 输入不破坏其它输入：Uxyz 无 Unicode 候选但引擎正常
    assert!(session.simulate("Uxyz"), "Uxyz 键序未被引擎接受");
    session.simulate("{Escape}");
    assert!(session.simulate("nihao"), "nihao 键序未被引擎接受");
    assert!(
        candidates_contain(&session, "你好"),
        "Unicode 引入后 nihao 候选丢失 你好：{:?}",
        candidate_texts(&session)
    );
    session.simulate("{Escape}");
}
