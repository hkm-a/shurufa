//! 部件拆字辅码（rime-ice search.lua）集成测试。
//!
//! 验证：
//! - nihao（你好）输入后，加辅码引导符 + ren：nihao`ren → 只留首字含
//!   亻(ren) 部件的候选（你好/倪/伲/伱），拟/尼/妮 被过滤
//! - 无辅码时候选不变（passthrough）
//! - schema 反查走 namespace=radical_pinyin（lua_filter@*search@radical_pinyin）
//!
//! 依赖：schemas/lua/search.lua + rime_ice.schema.yaml 的
//! lua_filter@*search@radical_pinyin + key_binder/search: "`" +
//! speller/alphabet 含 `（initials 不含）。
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
fn 部件拆字辅码过滤候选() {
    let root = repo_root();
    let user_dir = root.join("target/rime-search-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理辅码测试用户词典失败");
    }
    std::fs::create_dir_all(user_dir.join("lua")).expect("创建 lua 目录失败");
    copy_dir_recursive(&root.join("schemas/lua"), &user_dir.join("lua"));
    let engine: &'static ime_bridge::Engine = Box::leak(Box::new(
        Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败"),
    ));
    let session = engine.create_session().expect("创建会话失败");
    // 关 emoji，避免 OpenCC 别名候选干扰（如 你 → 你/倪 的 emoji 变体）
    session.set_option("emoji", false);

    // 1) 无辅码：nihao 候选含 你好（passthrough 基线）
    assert!(session.simulate("nihao"), "nihao 键序未被引擎接受");
    assert!(
        candidates_contain(&session, "你好"),
        "nihao 基线候选丢失 你好：{:?}",
        candidate_texts(&session)
    );
    assert!(
        candidates_contain(&session, "拟好"),
        "nihao 基线候选应有 拟好（首字 拟 非 ren 部首，用于对照）：{:?}",
        candidate_texts(&session)
    );
    session.simulate("{Escape}");

    // 2) 辅码过滤：nihao`ren → 拟好/尼 被过滤，你好 保留
    //    （你/倪/伲/伱 部件码以 ren 开头；拟/尼/妮 不是）
    assert!(session.simulate("nihao`ren"), "nihao`ren 键序未被引擎接受");
    assert!(
        candidates_contain(&session, "你好"),
        "nihao`ren 后 你好 丢失：{:?}",
        candidate_texts(&session)
    );
    let filtered = !candidates_contain(&session, "拟好") && !candidates_contain(&session, "尼");
    assert!(
        filtered,
        "nihao`ren 未过滤 拟好/尼（非 ren 部件）：{:?}",
        candidate_texts(&session)
    );
    session.simulate("{Escape}");

    // 3) vhelp：输入 vhelp → 列出全部 V 模式触发码（含日期/计算器/辅码）
    assert!(session.simulate("vhelp"), "vhelp 键序未被引擎接受");
    let texts = candidate_texts(&session);
    assert!(
        texts.iter().any(|t| t == "rq") && texts.iter().any(|t| t == "sj"),
        "vhelp 未列出日期触发码：{:?}",
        texts
    );
    // 翻页找计算器/辅码触发码（首屏 9 个被 pinyin 噪音占一位）
    let mut found_calc = false;
    let mut found_uu = false;
    for _ in 0..4 {
        let page = candidate_texts(&session);
        if page.iter().any(|t| t == "cC<算式>") {
            found_calc = true;
        }
        if page.iter().any(|t| t == "uU<部件>") {
            found_uu = true;
        }
        if found_calc && found_uu {
            break;
        }
        if !session.simulate("{Page_Down}") {
            break;
        }
    }
    assert!(
        found_calc && found_uu,
        "vhelp 未列出计算器/辅码触发码（calc={found_calc} uU={found_uu}）"
    );
    session.simulate("{Escape}");

    // 4) vhelp 不干扰正常输入：nihao 候选仍含 你好（v_help 只响应精确 vhelp）
    assert!(session.simulate("nihao"), "nihao 键序未被引擎接受");
    assert!(
        candidates_contain(&session, "你好"),
        "vhelp 后 nihao 候选丢失 你好：{:?}",
        candidate_texts(&session)
    );
    session.simulate("{Escape}");
}
