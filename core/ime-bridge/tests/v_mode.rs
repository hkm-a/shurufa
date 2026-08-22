//! V 模式快捷转写（librime-lua，rime-ice 官方脚本）集成测试。
//!
//! 验证：
//! - rq → 当前日期（date_translator.lua）
//! - R123 → 金额大写「壹佰贰拾叁」（number_translator.lua，R 前缀）
//! - cC1+1 → 计算结果 2（calc_translator.lua，cC 前缀）
//! - N20240210 → 甲辰年正月初一（lunar.lua + lunar.db，N 前缀）
//! - nl → 今日农历（lunar 触发码）
//!
//! 依赖：schemas/lua/*.lua + lunar.db 与 rime_ice.schema.yaml 的
//! lua_translator 挂载 + recognizer/patterns（number/calculator/
//! gregorian_to_lunar）+ speller alphabet 含大写字母。
//!
//! 注意：librime 引擎每进程仅一个实例（ENGINE_ALIVE 全局标志），
//! 全部场景在同一个引擎上顺序执行、各自新建会话，避免并行冲突。

use ime_bridge::{Engine, Session};
use std::path::PathBuf;
use std::sync::Mutex;

/// librime 引擎每进程仅一个实例（ENGINE_ALIVE 全局标志），本文件多个
/// #[test] 并行会互相抢实例。全部测试串行化：每个测试入口先取锁。
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// 全新引擎 + 把 schemas/lua/ 全量拷进用户目录（require 与 ReverseDb
/// 搜索路径都覆盖），避免历史 user_dir 污染。
fn fresh_engine(tag: &str) -> (Engine, PathBuf) {
    let root = repo_root();
    let user_dir = root.join(format!("target/rime-vmode-{tag}-user-data"));
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

fn candidates_contain(session: &Session, needle: &str) -> bool {
    session
        .context()
        .candidates
        .iter()
        .any(|c| c.text.contains(needle))
}

fn candidate_texts(session: &Session) -> Vec<String> {
    session
        .context()
        .candidates
        .iter()
        .take(9)
        .map(|c| c.text.clone())
        .collect::<Vec<_>>()
}

#[test]
fn v_mode_all_triggers() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (engine, _dir) = fresh_engine("all");

    // 1) rq → 日期候选（形如 2026-08-16；只断言格式避免日期 flaky）
    let session = engine.create_session().expect("创建会话失败");
    assert!(session.simulate("rq"), "rq 键序未被引擎接受");
    let has_date = session
        .context()
        .candidates
        .iter()
        .any(|c| c.text.len() == 10 && c.text.chars().filter(|ch| *ch == '-').count() == 2);
    assert!(
        has_date,
        "rq 未产生日期候选，实际: {:?}",
        candidate_texts(&session)
    );

    // 2) R123 → 金额大写「壹佰贰拾叁」
    let session = engine.create_session().expect("创建会话失败");
    assert!(session.simulate("R123"), "R123 键序未被引擎接受");
    assert!(
        candidates_contain(&session, "壹佰贰拾叁"),
        "R123 未产生金额大写候选，实际: {:?}",
        candidate_texts(&session)
    );

    // 3) cC1+1 → 计算器结果 2
    let session = engine.create_session().expect("创建会话失败");
    assert!(session.simulate("cC1+1"), "cC1+1 键序未被引擎接受");
    assert!(
        candidates_contain(&session, "2"),
        "cC1+1 未产生计算结果候选，实际: {:?}",
        candidate_texts(&session)
    );

    // 4) N20240210 → 甲辰年正月初一（2024-02-10 春节）
    let session = engine.create_session().expect("创建会话失败");
    assert!(session.simulate("N20240210"), "N20240210 键序未被引擎接受");
    assert!(
        candidates_contain(&session, "甲辰"),
        "N20240210 未产生农历候选，实际: {:?}",
        candidate_texts(&session)
    );

    // 5) nl → 今日农历（时间相关，只断言有候选）
    let session = engine.create_session().expect("创建会话失败");
    assert!(session.simulate("nl"), "nl 键序未被引擎接受");
    assert!(
        !session.context().candidates.is_empty(),
        "nl 未产生任何候选"
    );

    // 6) 错音错字提示（corrector.lua）：gei yu → 「给予」，comment 应显示
    //    正确读音 jǐ yǔ（纠错表命中）；未命中的候选 comment 被清空（keep_comments
    //    关闭），不残留 ［拼音］ 标记。
    let session = engine.create_session().expect("创建会话失败");
    assert!(session.simulate("geiyu"), "geiyu 键序未被引擎接受");
    let corrector_cands = session.context().candidates;
    let jiyu = corrector_cands
        .iter()
        .find(|c| c.text == "给予")
        .expect("geiyu 候选中未出现「给予」");
    assert!(
        jiyu.comment.contains("jǐ yǔ"),
        "「给予」comment 应为正确读音 jǐ yǔ，实际: {:?}",
        jiyu.comment
    );
    // 未命中纠错表的候选：comment 应为空（不留 ［拼音］ 噪音）
    for c in corrector_cands.iter().take(5) {
        assert!(
            !c.comment.contains('［'),
            "候选「{}」残留拼音标记 comment: {:?}",
            c.text,
            c.comment
        );
    }

    let _ = engine;
}

/// 辅码检字（P0 #2 剩余，rime-ice 部件拆字反查）：uU + 部件码反查汉字。
/// `uUbai'shao` 应反查出「的」（radical_pinyin 词典，bai'shao = 白勺）。
///
/// 撇号是多部件码之间的**必需**分隔符：radical_pinyin.schema.yaml 的
/// algebra 自 c12a576 起不再有 `xform/'//`（改为把 ' 纳入 alphabet 当部件
/// 分隔符），因此无撇号的 `baishao` 不再匹配。Android 符号页为此专门补了
/// 撇号键（2914139）。本测试此前断言的无撇号写法是该改动前的行为。
#[test]
fn radical_lookup_reverses_character() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (engine, _dir) = fresh_engine("radical");
    let session = engine.create_session().expect("创建会话失败");
    // 单部件码无需分隔符：heng（横）→ 一。逐键喂（大写 U 需 process_key）。
    for ch in "uUheng".chars() {
        assert!(session.process_key(ch as i32, 0), "键 {ch} 未被引擎接受");
    }
    let cands = session.context().candidates;
    assert!(
        cands.iter().any(|c| c.text == "一"),
        "uUheng 未反查出「一」，实际: {:?}",
        cands
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    // 多部件码必须带撇号分隔：uUbai'shao → 的
    let session = engine.create_session().expect("创建会话失败");
    for ch in "uUbai'shao".chars() {
        assert!(session.process_key(ch as i32, 0), "键 {ch} 未被引擎接受");
    }
    let cands = session.context().candidates;
    assert!(
        cands.iter().any(|c| c.text == "的"),
        "uUbai'shao 未反查出「的」，实际: {:?}",
        cands
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    // 回归护栏：无撇号写法必须**不**命中，否则说明 algebra 又被加回
    // `xform/'//`，部件分隔语义会随之丢失（Android 撇号键将变成无用键）。
    let session = engine.create_session().expect("创建会话失败");
    for ch in "uUbaishao".chars() {
        assert!(session.process_key(ch as i32, 0), "键 {ch} 未被引擎接受");
    }
    assert!(
        !session.context().candidates.iter().any(|c| c.text == "的"),
        "无撇号的 uUbaishao 不应命中「的」——撇号分隔语义已丢失"
    );
    let _ = engine;
}

/// 自定义短语（P1 #6）：用户目录的 custom_phrase.txt 中 `gs → 公司`，
/// 输入 gs 时公司应出现在候选中（权重 99 置顶）。
#[test]
fn custom_phrase_commits_user_phrase() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = repo_root();
    let user_dir = root.join("target/rime-customphrase-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理测试用户词典失败");
    }
    let user_lua = user_dir.join("lua");
    std::fs::create_dir_all(&user_lua).expect("创建用户 lua 目录失败");
    copy_dir_recursive(&root.join("schemas/lua"), &user_lua);
    // 写自定义短语：格式为「词汇<Tab>编码<Tab>权重」（词在前，参考 rime-ice
    // custom_phrase.txt；需 db 头指令让 librime 识别为 custom phrase 表）。
    std::fs::write(
        user_dir.join("custom_phrase.txt"),
        "# Rime table\n# coding: utf-8\n#@/db_name\tcustom_phrase.txt\n#@/db_type\ttabledb\n\n公司\tgs\t100\n位置\twz\t90\n",
    )
    .expect("写入 custom_phrase.txt 失败");

    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");
    assert!(session.simulate("gs"), "gs 键序未被引擎接受");
    let cands = session.context().candidates;
    let has_company = cands.iter().any(|c| c.text == "公司");
    assert!(
        has_company,
        "gs 候选中未出现自定义短语「公司」，实际: {:?}",
        cands
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    // 公司应在第一位（权重 99 压过拼音候选）
    assert_eq!(
        cands.first().map(|c| c.text.as_str()),
        Some("公司"),
        "自定义短语应置顶，实际首位: {:?}",
        cands.first().map(|c| c.text.clone())
    );
    let _ = engine;
}

/// 冷词丢弃（cold_word_drop 模块）：预先在用户目录写入 drop_words.lua，
/// 声明丢弃词条后，该词条不应再出现在候选中（filter 生效）。
#[test]
fn cold_word_drop_removes_declared_word() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    use std::io::Write;
    let root = repo_root();
    let user_dir = root.join("target/rime-coldword-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理测试用户词典失败");
    }
    let user_lua = user_dir.join("lua");
    std::fs::create_dir_all(&user_lua).expect("创建用户 lua 目录失败");
    copy_dir_recursive(&root.join("schemas/lua"), &user_lua);

    // 声明丢弃「示例」（输入 shili 时该词条不应出现）
    let cold_dir = user_lua.join("cold_word_drop");
    std::fs::create_dir_all(&cold_dir).expect("创建 cold_word_drop 目录失败");
    let mut f =
        std::fs::File::create(cold_dir.join("drop_words.lua")).expect("创建 drop_words.lua 失败");
    f.write_all("local drop_words =\n{ \"示例\", }\nreturn drop_words\n".as_bytes())
        .expect("写入 drop_words.lua 失败");

    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");
    assert!(session.simulate("shili"), "shili 键序未被引擎接受");
    let cands = session.context().candidates;
    assert!(
        !cands.iter().any(|c| c.text == "示例"),
        "丢弃词「示例」仍出现在候选中: {:?}",
        cands
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    assert!(!cands.is_empty(), "丢弃词后候选不应为空（其余词条仍在）");
    let _ = engine;
}

/// Emoji（simplifier@emoji，OpenCC text 词典）：输入中文词时附带 emoji
/// 候选（weixiao → 微笑 + 😊）。数据在 schemas/opencc/（emoji.json +
/// emoji.txt + others.txt），无需 .ocd2 编译。
#[test]
fn emoji_appears_as_candidate() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (engine, _dir) = fresh_engine("emoji");
    let session = engine.create_session().expect("创建会话失败");
    // emoji 开关默认开（reset: 1）
    assert!(session.get_option("emoji"), "emoji 开关应默认开启");
    assert!(session.simulate("weixiao"), "weixiao 键序未被引擎接受");
    let cands = session.context().candidates;
    assert!(
        cands.iter().any(|c| c.text == "😊"),
        "weixiao 未出现 😊 候选，实际: {:?}",
        cands
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    // 关闭开关后 emoji 候选消失（同会话内切换，选项是会话级状态）
    session.set_option("emoji", false);
    let _ = session.simulate("{Escape}");
    assert!(session.simulate("weixiao"), "weixiao 键序未被引擎接受");
    let cands2 = session.context().candidates;
    assert!(
        !cands2.iter().any(|c| c.text == "😊"),
        "关闭 emoji 后仍出现 😊: {:?}",
        cands2
            .iter()
            .take(9)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    let _ = engine;
}
