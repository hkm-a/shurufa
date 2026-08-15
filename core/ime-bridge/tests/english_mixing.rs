//! 英文混输集成测试：中文输入时英文前缀/整词候选混排（搜狗/微软拼音同款）。
//!
//! 验证路径：Engine::init 部署 schemas/（含 english.dict.yaml + rime_ice 的
//! english_translator）→ 喂英文键序 → 断言英文候选出现且中文候选不被挤掉。
//!
//! 注意：librime Engine 是进程级单例（与 smoke.rs / double_pinyin.rs 同理，
//! 每个集成测试文件是独立进程，互不干扰）。本文件所有用例合并为一个 #[test]。

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

/// 在当前页候选里找文本（可能在第 1 页；英文候选排在中文之后，必要时翻页）。
fn find_candidate(session: &ime_bridge::Session<'_>, text: &str) -> bool {
    let mut ctx = session.context();
    for _ in 0..4 {
        if ctx.candidates.iter().any(|c| c.text == text) {
            return true;
        }
        if !session.simulate("{Page_Down}") {
            break;
        }
        ctx = session.context();
    }
    false
}

#[test]
fn 英文整词与拼音候选混排且中文优先() {
    let root = repo_root();
    let user_dir = root.join("target/rime-english-mixing-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理英文混输测试用户词典失败");
    }
    let engine: &'static ime_bridge::Engine = Box::leak(Box::new(
        Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败"),
    ));
    let session = engine.create_session().expect("创建会话失败");

    // 1) 整词命中："hello" 应是候选（英文词典 exact match）
    assert!(session.simulate("hello"), "hello 键序未被引擎接受");
    assert!(
        find_candidate(&session, "hello"),
        "候选未出现英文整词 hello：{:?}",
        session
            .context()
            .candidates
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );

    // 2) 前缀联想："hel" 应能补出 hello/help（enable_completion）
    session.simulate("{Escape}");
    assert!(session.simulate("hel"), "hel 键序未被引擎接受");
    let completed = ["hello", "help"]
        .iter()
        .any(|w| find_candidate(&session, w));
    assert!(
        completed,
        "前缀 hel 未补出英文词：{:?}",
        session
            .context()
            .candidates
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );

    // 3) 中文不被挤掉："nihao" 候选仍含 你好（英文翻译器不产生 nihao 前缀词）
    session.simulate("{Escape}");
    assert!(session.simulate("nihao"), "nihao 键序未被引擎接受");
    assert!(
        find_candidate(&session, "你好"),
        "加入英文混输后 nihao 候选丢失 你好：{:?}",
        session
            .context()
            .candidates
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );

    // 4) 英文候选排在中文之后（quality 0.35 < 拼音 1.2）：
    //    "ma" 的首屏第一项必须是 吗（中文优先），hello 类英文词只能在后面。
    session.simulate("{Escape}");
    assert!(session.simulate("ma"), "ma 键序未被引擎接受");
    let ctx = session.context();
    assert!(
        ctx.candidates.first().map(|c| c.text.as_str()) == Some("吗"),
        "ma 首屏首选应为中文 吗（英文不得抢占），实际 {:?}",
        ctx.candidates
            .iter()
            .take(5)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    session.simulate("{Escape}");
}
