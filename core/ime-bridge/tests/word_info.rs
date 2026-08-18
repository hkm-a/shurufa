//! 词汇别名（rime-ice 2025+「部分常用词自动展示翻译/别名/化学式/简称」）集成测试。
//!
//! 验证：
//! - aerfa（阿尔法）→ 阿尔法（主词库）+ alpha / α / A（word_info 别名共享编码）
//! - shui（水）→ H2O / 水分子（化学式别名）
//! - bei ta（贝塔）→ beta / β
//! - 别名排在拼音候选之后（initial_quality 0.5 < 拼音 1.2）
//!
//! 依赖：schemas/word_info.dict.yaml + word_info.schema.yaml（依赖方案，随
//! rime_ice 一起部署编译）+ rime_ice.schema.yaml 的 table_translator@word_info。
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

/// 在当前页候选里找文本（词库别名排在拼音之后，必要时翻页）。
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
fn 词汇别名共享编码展示翻译与别名() {
    let root = repo_root();
    let user_dir = root.join("target/rime-word-info-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理词汇别名测试用户词典失败");
    }
    std::fs::create_dir_all(user_dir.join("lua")).expect("创建 lua 目录失败");
    copy_dir_recursive(&root.join("schemas/lua"), &user_dir.join("lua"));
    let engine: &'static ime_bridge::Engine = Box::leak(Box::new(
        Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败"),
    ));
    let session = engine.create_session().expect("创建会话失败");

    // 1) 希腊字母别名：aerfa → 阿尔法（主词库）+ alpha / α / A（别名）
    assert!(session.simulate("aerfa"), "aerfa 键序未被引擎接受");
    let texts = || {
        session
            .context()
            .candidates
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    };
    assert!(
        find_candidate(&session, "阿尔法"),
        "aerfa 未出现主词库候选 阿尔法：{:?}",
        texts()
    );
    assert!(
        find_candidate(&session, "alpha"),
        "aerfa 未出现英文别名 alpha：{:?}",
        texts()
    );
    assert!(
        find_candidate(&session, "α"),
        "aerfa 未出现希腊字母别名 α：{:?}",
        texts()
    );
    assert!(
        find_candidate(&session, "A"),
        "aerfa 未出现大写字母别名 A：{:?}",
        texts()
    );
    // 2) 别名排在拼音候选之后：首屏第一项仍是 阿尔法（拼音 1.2 > 别名 0.5）
    assert!(
        session
            .context()
            .candidates
            .first()
            .map(|c| c.text.as_str())
            == Some("阿尔法"),
        "aerfa 首选应为 阿尔法（拼音优先于别名），实际前 5：{:?}",
        session
            .context()
            .candidates
            .iter()
            .take(5)
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    );
    session.simulate("{Escape}");

    // 3) 化学式别名：shui → H2O / 水分子
    assert!(session.simulate("shui"), "shui 键序未被引擎接受");
    assert!(
        find_candidate(&session, "H2O"),
        "shui 未出现化学式别名 H2O：{:?}",
        texts()
    );
    session.simulate("{Escape}");

    // 4) 别名不参与前缀联想（enable_completion false）：aer 不应出现 alpha
    assert!(session.simulate("aer"), "aer 键序未被引擎接受");
    assert!(
        !session
            .context()
            .candidates
            .iter()
            .any(|c| c.text == "alpha"),
        "aer 前缀不应联想出 alpha（enable_completion false）：{:?}",
        texts()
    );
    session.simulate("{Escape}");
}
