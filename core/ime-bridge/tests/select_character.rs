//! 以词定字（lua_processor@select_character）集成测试。
//!
//! 验证：输入 zhongguo（候选"中国"）后按 `[` 取词首字上屏，应得到"中"；
//! 按 `]` 取词末字上屏，应得到"国"。
//!
//! 依赖：schemas/lua/select_character.lua + rime_ice.schema.yaml 的
//! `lua_processor@*select_character` 与 key_binder 配置。
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
fn select_character_takes_first_and_last_char() {
    let root = repo_root();
    let user_dir = root.join("target/rime-select-char-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理测试用户词典失败");
    }
    // librime-lua 的 require 搜索路径含 user_data_dir/lua/：把脚本复制过去，
    // 与 shared schemas/lua/ 双保险（librime-lua 按包搜索路径决定）。
    let user_lua = user_dir.join("lua");
    std::fs::create_dir_all(&user_lua).expect("创建用户 lua 目录失败");
    std::fs::copy(
        root.join("schemas/lua/select_character.lua"),
        user_lua.join("select_character.lua"),
    )
    .expect("复制 select_character.lua 到用户目录失败");
    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    // 输入 zhongguo，候选应含"中国"
    assert!(session.simulate("zhongguo"), "键序列未被引擎接受");
    let ctx = session.context();
    assert!(
        ctx.candidates.iter().any(|c| c.text == "中国"),
        "候选中未出现「中国」，实际: {:?}",
        ctx.candidates
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );

    // 按 [ 取首字：用真实按键路径（X11 keysym 91 = '['），与 TSF 一致
    // （librime 模拟键序 {bracketleft} 的 repr 是 "bracketleft" 而非 "[",
    // 无法触发 select_character 的按键比较——必须走 process_key）。
    assert!(
        session.process_key(91, 0),
        "bracketleft keysym 未被引擎接受"
    );
    let after = session.context();
    println!(
        "按 [ 后 preedit: {:?}, 候选: {:?}",
        after.preedit,
        after
            .candidates
            .iter()
            .take(5)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    let committed = session.commit().expect("未取得上屏文本");
    println!("取首字上屏: {committed}");
    assert_eq!(committed, "中", "按 [ 应上屏首字「中」");

    // 重新输入 zhongguo，按 ]（keysym 93）取末字
    assert!(session.simulate("zhongguo"), "重输 zhongguo 失败");
    assert!(
        session.process_key(93, 0),
        "bracketright keysym 未被引擎接受"
    );
    let committed = session.commit().expect("未取得上屏文本");
    println!("取末字上屏: {committed}");
    assert_eq!(committed, "国", "按 ] 应上屏末字「国」");
}
