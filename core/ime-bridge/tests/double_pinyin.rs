//! 双拼方案集成测试：小鹤双拼真实可用性验证。
//!
//! 验证路径：Engine::init 部署 schemas/（含 shurufa_double_pinyin.schema.yaml）
//! → select_schema("shurufa_double_pinyin") → 用双拼键序喂入 → 候选/上屏正确。
//!
//! 注意：librime Engine 是进程级单例（"进程内已存在 Engine 实例"），
//! 因此本文件所有用例合并为**一个** #[test] 串行执行，共享同一个进程。
//!
//! 小鹤双拼键序：
//! - 你好 = ni hao = n + i, h + ao(→c) → "nihc"
//! - 上海 = shang hai = sh(→u) + ang(→k), h + ai(→l) → "ukhl"
//! - 世界 = shi jie = sh(→u) + i, j + ie(→p) → "uijp"

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

/// 建一个双拼会话（每个用例独立 user_dir 避免学习串扰；Engine 全局单例复用，
/// 重复 init 会报"进程内已存在 Engine 实例"，故测试串行且只 init 一次）。
fn fresh_user_dir(tag: &str) -> PathBuf {
    let root = repo_root();
    let user_dir = root.join(format!("target/rime-{tag}-user-data"));
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理旧的用户词典失败");
    }
    user_dir
}

/// 喂键序并断言候选里出现预期词（翻页搜索最多 3 页）。
fn assert_candidate_contains(session: &ime_bridge::Session<'_>, keys: &str, expected: &str) {
    assert!(session.simulate(keys), "键序列 {keys:?} 未被引擎接受");
    let mut ctx = session.context();
    println!(
        "[双拼] {keys} → preedit={:?} 候选={:?}",
        ctx.preedit,
        ctx.candidates.iter().map(|c| &c.text).collect::<Vec<_>>()
    );
    for _ in 0..3 {
        if ctx.candidates.iter().any(|c| c.text.contains(expected)) {
            return;
        }
        if !session.simulate("{Page_Down}") {
            break;
        }
        ctx = session.context();
    }
    panic!("候选未出现 {expected:?}（键序 {keys:?}）：{ctx:?}");
}

/// 喂键序 + 空格上屏，断言首选上屏为预期词。
fn assert_commit(session: &ime_bridge::Session<'_>, keys: &str, expected: &str) {
    assert!(session.simulate(keys), "键序列 {keys:?} 未被引擎接受");
    assert!(session.simulate(" "), "空格未被引擎接受");
    let committed = session.commit().expect("未取得上屏文本");
    println!("[双拼] {keys} → 上屏 {committed:?}");
    assert_eq!(committed, expected, "上屏文本不符（键序 {keys:?}）");
}

#[test]
fn 双拼方案_真实可用_五例串行() {
    let root = repo_root();
    // Engine 进程级单例：只 init 一次，此后建会话复用。
    // 双拼/五笔/仓颉已加入 default.yaml schema_list，默认 maintenance
    // 会一并编译，select_schema 可直接使用（无需额外 deploy）。
    let engine: &'static ime_bridge::Engine = Box::leak(Box::new(
        Engine::init(&root.join("schemas"), &fresh_user_dir("double-pinyin-main"))
            .expect("引擎初始化失败"),
    ));

    // 1) 你好 候选 + 上屏
    let s1 = engine.create_session().expect("创建会话失败");
    assert!(
        s1.select_schema("shurufa_double_pinyin"),
        "select_schema 失败"
    );
    assert_candidate_contains(&s1, "nihc", "你好");
    // 新会话验证上屏（旧会话可能残留组合）
    let s1b = engine.create_session().expect("创建会话失败");
    assert!(
        s1b.select_schema("shurufa_double_pinyin"),
        "select_schema 失败"
    );
    assert_commit(&s1b, "nihc", "你好");

    // 2) 上海 候选
    let s2 = engine.create_session().expect("创建会话失败");
    assert!(
        s2.select_schema("shurufa_double_pinyin"),
        "select_schema 失败"
    );
    assert_candidate_contains(&s2, "ukhl", "上海");

    // 3) 世界 候选
    let s3 = engine.create_session().expect("创建会话失败");
    assert!(
        s3.select_schema("shurufa_double_pinyin"),
        "select_schema 失败"
    );
    assert_candidate_contains(&s3, "uijp", "世界");

    // 4) 同一 session 运行时切回 rime_ice：拼音键序立即按全拼解析
    //    （librime select_schema 是运行时切换，验证方案间不互害）。
    let s4 = engine.create_session().expect("创建会话失败");
    assert!(
        s4.select_schema("shurufa_double_pinyin"),
        "select_schema 双拼失败"
    );
    assert!(
        s4.select_schema("rime_ice"),
        "select_schema 切回 rime_ice 失败"
    );
    assert!(s4.simulate("nihao"), "拼音键序未被引擎接受");
    let ctx4 = s4.context();
    assert!(
        ctx4.candidates.iter().any(|c| c.text.contains("你好")),
        "切回拼音后 nihao 候选未出现「你好」：{ctx4:?}"
    );
}
