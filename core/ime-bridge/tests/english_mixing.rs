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

/// 在当前页候选里找文本（可能在第 1 页；英文候选排在中文之后，必要时翻页）。
/// 2026-08-16：emoji 特性会为中文词附带 emoji 变体候选，候选总数比无 emoji
/// 时多，搜索窗口从 4 页放宽到 6 页。
fn find_candidate(session: &ime_bridge::Session<'_>, text: &str) -> bool {
    let mut ctx = session.context();
    for _ in 0..6 {
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
    // schema 的 lua 组件（corrector/cold_word_drop/V-mode 等）需要脚本在
    // 搜索路径里：把 schemas/lua 递归拷进 user_dir/lua（与 v_mode.rs 同法）。
    std::fs::create_dir_all(user_dir.join("lua")).expect("创建 lua 目录失败");
    copy_dir_recursive(&root.join("schemas/lua"), &user_dir.join("lua"));
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

    // 5) 英文自动大小写（autocap_filter + english.schema.yaml 大小写派生）：
    //    输入 Hello（首字母大写）→ 候选词转为首字母大写 Hello；
    //    输入 HELLO（全大写）→ 候选词转为全大写 HELLO；
    //    输入 hello（全小写）→ 保持小写不变。
    session.simulate("{Escape}");
    assert!(session.simulate("Hello"), "Hello 键序未被引擎接受");
    assert!(
        find_candidate(&session, "Hello"),
        "输入 Hello 未出现首字母大写候选：{:?}",
        session
            .context()
            .candidates
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        !session
            .context()
            .candidates
            .iter()
            .any(|c| c.text == "hello"),
        "输入 Hello 不应再出现小写 hello 候选（应被自动大写转换）"
    );
    session.simulate("{Escape}");
    assert!(session.simulate("HELLO"), "HELLO 键序未被引擎接受");
    assert!(
        find_candidate(&session, "HELLO"),
        "输入 HELLO 未出现全大写候选：{:?}",
        session
            .context()
            .candidates
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    session.simulate("{Escape}");
    assert!(session.simulate("hello"), "hello 键序未被引擎接受");
    assert!(
        find_candidate(&session, "hello"),
        "输入全小写 hello 候选应保持原样：{:?}",
        session
            .context()
            .candidates
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    session.simulate("{Escape}");

    // 6) 英文自动大小写：前缀联想也要转（Hel → Hello/Help 首字母大写）。
    session.simulate("{Escape}");
    assert!(session.simulate("Hel"), "Hel 键序未被引擎接受");
    let capitalized_completed = ["Hello", "Help"]
        .iter()
        .any(|w| find_candidate(&session, w));
    assert!(
        capitalized_completed,
        "前缀 Hel 未补出首字母大写英文词：{:?}",
        session
            .context()
            .candidates
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    session.simulate("{Escape}");

    // 7) 英文自动大小写不破坏既有大写输入行为：Nihao（大写 N 是 V 模式
    //    触发字母，非拼音首字母）在加 autocap 前后候选一致（基线实测：
    //    大写 N 走 rime_ice.dict.yaml 的字母条目 → N + 哦好）。此处只验证
    //    autocap 不丢候选、不崩溃（若 Lua 报错整个候选流会被丢弃）。
    session.simulate("{Escape}");
    assert!(session.simulate("Nihao"), "Nihao 键序未被引擎接受");
    let ctx = session.context();
    assert!(
        !ctx.candidates.is_empty(),
        "输入 Nihao 后候选为空（autocap 可能丢弃候选流）：{:?}",
        ctx.candidates
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    session.simulate("{Escape}");
}
