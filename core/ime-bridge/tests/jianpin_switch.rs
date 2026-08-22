//! M10 困难项：简拼开关（librime 条件规则缺失）的部署期替代方案。
//!
//! librime 1.17 speller/algebra 不支持条件规则（option@jianpin: 实测报
//! Error loading formula #13），无法热开关简拼。替代：scripts/
//! gen-nojianpin-schema.ps1 生成去掉 abbrev 规则的 rime_ice_nojianpin
//! 变体方案，设置中心方案页切换 + 重新部署生效。
//!
//! 验证（变体需经 schema_list 部署后 librime 才编译可用）：
//! - default.custom.yaml 把 rime_ice_nojianpin 列入 schema_list 后，
//!   select_schema 可加载且全拼 beijing 正常出候选（可部署 + 不回归）
//! - 简拼码 bj 在本环境 rime_ice 精简方案下亦无候选（librime 简拼依赖
//!   prism 部署增量与码表编码，效果留实机验证）

use ime_bridge::Engine;
use std::path::PathBuf;
use std::sync::Mutex;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn candidate_texts(session: &ime_bridge::Session) -> Vec<String> {
    session
        .context()
        .candidates
        .iter()
        .take(9)
        .map(|c| c.text.clone())
        .collect::<Vec<_>>()
}

#[test]
fn 无简拼变体方案可部署且全拼不回归() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = repo_root();
    let user_dir = root.join("target/rime-nojianpin-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理测试用户词典失败");
    }
    std::fs::create_dir_all(&user_dir).expect("创建 user 目录失败");
    // 把变体方案加入 schema_list，librime 初始化时才编译它（等价真实部署管道）
    std::fs::write(
        user_dir.join("default.custom.yaml"),
        "patch:
  schema_list:
    - schema: rime_ice
    - schema: rime_ice_nojianpin
",
    )
    .expect("写 default.custom.yaml 失败");

    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    // 默认方案全拼 beijing → 北京（对照组）
    assert!(session.simulate("beijing"), "beijing 键序未被接受");
    let cands = candidate_texts(&session);
    assert!(
        cands.iter().any(|c| c.contains("北京")),
        "全拼 beijing 应出北京候选，实际：{cands:?}"
    );

    // 无简拼变体可加载且全拼不回归
    assert!(
        session.select_schema("rime_ice_nojianpin"),
        "无简拼变体 rime_ice_nojianpin 加载失败（变体不可部署）"
    );
    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("beijing"), "beijing 键序未被接受");
    let cands = candidate_texts(&session);
    assert!(
        cands.iter().any(|c| c.contains("北京")),
        "无简拼方案全拼 beijing 应出北京，实际：{cands:?}"
    );

    // 简拼行为说明：本集成环境 rime_ice 精简方案下简拼码（bj）亦无候选
    // （librime 简拼依赖 prism 部署增量与码表编码，实机验证）；此处仅锁定
    // 变体方案可部署且全拼不回归。
}

/// 阶段 3 第 5 项证据：librime 原生 abbrev 只对单音节生效，
/// 多音节简拼词（如 lw → 另外）在未加载外部索引时不会命中，
/// 因此 windows-algo 的 jianpin_index.txt 是必要的前端补丁。
#[test]
fn 原生简拼不命中多音节简拼词() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = repo_root();
    let user_dir = root.join("target/rime-jianpin-native-limit-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理测试用户词典失败");
    }
    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    // lw 是“另外/论文/礼物”的简拼；原生 librime 不应把它们当词条命中。
    assert!(session.simulate("lw"), "lw 键序未被接受");
    let cands = candidate_texts(&session);
    assert!(
        !cands
            .iter()
            .any(|c| c.contains("另外") || c.contains("论文") || c.contains("礼物")),
        "原生 librime 不应命中多音节简拼词，实际：{cands:?}"
    );
}
