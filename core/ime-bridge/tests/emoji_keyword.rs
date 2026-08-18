//! Emoji 关键词联想（rime-ice simplifier@emoji 同款）集成测试。
//!
//! 验证（2026-08-18 实机核对后锁定为回归测试）：
//! - xiexie（谢谢）→ 候选含 🙏，且紧随 谢谢 之后（emoji 是词条的附加候选）
//! - weixiao（微笑）→ 候选含 😊
//! - kaixin（开心）→ 候选含 😄
//! - haha（哈哈）→ 候选含 😄 与 🐸（多 emoji 联想：蛤蛤→🐸）
//! - zan（赞）→ 候选含 👍（单字词同样附加 emoji）
//! - emoji 开关关闭（set_option emoji=false）→ 无任何 emoji 候选（门控生效）
//!
//! 实现机制（调研结论，2026-08-18）：rime-ice 并无独立"拼音→emoji"词典，
//! emoji 候选来自 `simplifier@emoji`（OpenCC emoji.json 文本词典）把**中文词
//! 候选**转换成附加候选（谢谢→🙏、微笑→😊）。我们已完整落地该机制：
//! schemas/opencc/emoji.txt 是 rime-ice 全量 4858 行词典，开关在
//! switches/emoji（默认开）。本文件把"输入拼音出 emoji 候选"锁定为回归
//! 行为，防止未来改动破坏。
//!
//! 依赖：schemas/opencc/（emoji.json + emoji.txt + others.txt）+ rime_ice
//! schema 的 simplifier@emoji。librime 引擎每进程仅一个实例，场景顺序执行。

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

/// 在当前页候选里找文本（必要时翻页）。
fn find_candidate(session: &ime_bridge::Session<'_>, text: &str) -> bool {
    let mut ctx = session.context();
    for _ in 0..8 {
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

/// 收集当前页候选文本。
fn texts(session: &ime_bridge::Session<'_>) -> Vec<String> {
    session
        .context()
        .candidates
        .iter()
        .map(|c| c.text.clone())
        .collect()
}

/// 当前页候选是否包含任何"纯 emoji"候选（文本非空且首字符在 emoji 区段，
/// 用于开关关闭时的否定断言——此时不应出现任何 emoji 附加候选）。
fn has_emoji_candidate(session: &ime_bridge::Session<'_>) -> bool {
    fn is_emoji(c: char) -> bool {
        matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF)
    }
    session.context().candidates.iter().any(|c| {
        let s: Vec<char> = c.text.chars().collect();
        !s.is_empty() && is_emoji(s[0]) && s.len() <= 2
    })
}

#[test]
fn emoji_关键词联想_输入拼音出emoji候选() {
    let root = repo_root();
    let user_dir = root.join("target/rime-emoji-keyword-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理 emoji 测试用户词典失败");
    }
    std::fs::create_dir_all(user_dir.join("lua")).expect("创建 lua 目录失败");
    copy_dir_recursive(&root.join("schemas/lua"), &user_dir.join("lua"));
    let engine: &'static ime_bridge::Engine = Box::leak(Box::new(
        Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败"),
    ));
    let session = engine.create_session().expect("创建会话失败");
    // 本测试依赖 emoji 开关开（schema 默认 reset:1，这里显式确保）
    session.set_option("emoji", true);

    // 1) xiexie → 谢谢 + 🙏，且 🙏 紧随 谢谢（附加候选紧跟词条）
    assert!(session.simulate("xiexie"), "xiexie 键序未被引擎接受");
    let ctx = session.context();
    let first = ctx.candidates.first().map(|c| c.text.as_str());
    assert_eq!(
        first,
        Some("谢谢"),
        "xiexie 首选应为 谢谢，实际：{:?}",
        texts(&session)
    );
    assert!(
        find_candidate(&session, "🙏"),
        "xiexie 未出现 emoji 候选 🙏：{:?}",
        texts(&session)
    );
    let cands: Vec<String> = session
        .context()
        .candidates
        .iter()
        .map(|c| c.text.clone())
        .collect();
    let xiexie_pos = cands.iter().position(|t| t == "谢谢").unwrap_or(usize::MAX);
    let pray_pos = cands.iter().position(|t| t == "🙏").unwrap_or(usize::MAX);
    assert!(
        pray_pos == xiexie_pos + 1,
        "🙏 应紧随 谢谢（附加候选紧跟词条），实际位置 谢谢={xiexie_pos} 🙏={pray_pos}：{cands:?}"
    );
    session.simulate("{Escape}");

    // 2) weixiao → 微笑 + 😊
    assert!(session.simulate("weixiao"), "weixiao 键序未被引擎接受");
    assert!(
        find_candidate(&session, "😊"),
        "weixiao 未出现 emoji 候选 😊：{:?}",
        texts(&session)
    );
    session.simulate("{Escape}");

    // 3) kaixin → 开心 + 😄
    assert!(session.simulate("kaixin"), "kaixin 键序未被引擎接受");
    assert!(
        find_candidate(&session, "😄"),
        "kaixin 未出现 emoji 候选 😄：{:?}",
        texts(&session)
    );
    session.simulate("{Escape}");

    // 4) haha → 哈哈 + 😄 + 🐸（多 emoji 联想：哈哈→😄、蛤蛤→🐸）
    assert!(session.simulate("haha"), "haha 键序未被引擎接受");
    assert!(
        find_candidate(&session, "😄"),
        "haha 未出现 emoji 候选 😄：{:?}",
        texts(&session)
    );
    assert!(
        find_candidate(&session, "🐸"),
        "haha 未出现 emoji 候选 🐸（蛤蛤→🐸）：{:?}",
        texts(&session)
    );
    session.simulate("{Escape}");

    // 5) zan → 赞 + 👍（单字词同样附加 emoji）
    assert!(session.simulate("zan"), "zan 键序未被引擎接受");
    assert!(
        find_candidate(&session, "👍"),
        "zan 未出现 emoji 候选 👍：{:?}",
        texts(&session)
    );
    session.simulate("{Escape}");

    // 6) 开关关闭 → 无 emoji 候选（门控生效；weixiao 只出中文词）
    session.set_option("emoji", false);
    assert!(session.simulate("weixiao"), "weixiao 键序未被引擎接受");
    assert!(
        !has_emoji_candidate(&session),
        "emoji 开关关闭后 weixiao 不应出现 emoji 候选：{:?}",
        texts(&session)
    );
    session.simulate("{Escape}");
    session.set_option("emoji", true);

    // 7) 开关恢复 → emoji 候选回来（门控可逆）
    assert!(session.simulate("weixiao"), "weixiao 键序未被引擎接受");
    assert!(
        find_candidate(&session, "😊"),
        "emoji 开关恢复后 weixiao 应重新出现 😊：{:?}",
        texts(&session)
    );
    session.simulate("{Escape}");
}
