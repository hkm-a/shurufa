//! M0 冒烟测试：加载 librime，部署朙月拼音方案，喂键取候选并上屏。
//!
//! 首次运行会编译词典（约数十秒），产物缓存在 target/rime-user-data。

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

#[test]
fn pinyin_input_produces_candidates_and_commit() {
    let root = repo_root();
    let engine = Engine::init(&root.join("schemas"), &root.join("target/rime-user-data"))
        .expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    // 喂入拼音 nihao，应得到预编辑串与非空候选
    assert!(session.simulate("nihao"), "键序列未被引擎接受");
    let ctx = session.context();
    println!("预编辑串: {}", ctx.preedit);
    for (i, c) in ctx.candidates.iter().take(5).enumerate() {
        println!("候选{}: {} {}", i + 1, c.text, c.comment);
    }
    assert!(!ctx.preedit.is_empty(), "预编辑串为空");
    assert!(!ctx.candidates.is_empty(), "候选列表为空");
    assert!(
        ctx.candidates.iter().any(|c| c.text == "你好"),
        "候选中未出现「你好」"
    );

    // 空格上屏首选候选
    assert!(session.simulate(" "), "空格键未被引擎接受");
    let committed = session.commit().expect("未取得上屏文本");
    println!("上屏文本: {committed}");
    assert_eq!(committed, "你好");

    // 上屏后上下文应清空
    assert!(session.context().candidates.is_empty(), "上屏后候选未清空");

    // 默认方案必须输出简体：吗（简）在候选中，嗎（繁）不应是首选
    assert!(session.simulate("ma"), "键序列未被引擎接受");
    let ctx = session.context();
    assert!(
        ctx.candidates.iter().any(|c| c.text == "吗"),
        "默认方案候选未出现简体「吗」，当前候选：{:?}",
        ctx.candidates.iter().take(5).map(|c| &c.text).collect::<Vec<_>>()
    );
    session.simulate("{Escape}");
}
