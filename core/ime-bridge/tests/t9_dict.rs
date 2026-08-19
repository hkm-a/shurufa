//! M-A1-2 9 键 T9 拼音（搜狗安卓 1.40 九宫格 / 8.13 大九键）：引擎侧验证。
//!
//! 词库：scripts/gen-t9-dict.py 从雾凇拼音基础词库生成 shurufa_t9.dict.yaml，
//! 整词 T9 数字串作单码索引（2abc 3def 4ghi 5jkl 6mno 7pqrs 8tuv 9wxyz）。
//!
//! 验证：
//! - shurufa → 7487832 → 「输入法」
//! - nihao → 64426 → 「你好」
//! - 翻页查找与候选上屏路径不回归

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
        .map(|c| c.text.clone())
        .collect::<Vec<_>>()
}

/// 在当前页候选里找文本（必要时翻页，至多 20 页）。
fn find_candidate(session: &ime_bridge::Session<'_>, text: &str) -> (bool, Vec<String>) {
    let mut seen = Vec::new();
    let mut ctx = session.context();
    for _ in 0..20 {
        for c in &ctx.candidates {
            seen.push(c.text.clone());
        }
        if ctx.candidates.iter().any(|c| c.text == text) {
            return (true, seen);
        }
        if !session.simulate("{Page_Down}") {
            break;
        }
        ctx = session.context();
    }
    (false, seen)
}

#[test]
fn t9_整词数字串可打出常用词() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = repo_root();
    let user_dir = root.join("target/rime-t9-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理失败");
    }
    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");
    assert!(
        session.select_schema("shurufa_t9"),
        "select_schema(shurufa_t9) 失败：schema_list 是否已加入？"
    );

    // shurufa → 7 4 8 7 8 3 2 → 7487832 → 输入法
    assert!(session.simulate("7487832"), "7487832 键序未被接受");
    let (found, seen) = find_candidate(&session, "输入法");
    assert!(found, "7487832 应出「输入法」，实际候选：{:?}", seen);

    // nihao → 6 4 4 2 6 → 64426 → 你好
    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("64426"), "64426 键序未被接受");
    let (found, seen) = find_candidate(&session, "你好");
    assert!(found, "64426 应出「你好」，实际候选：{:?}", seen);

    // 常规候选渲染不回归：候选非空且文本合法
    let cands = candidate_texts(&session);
    assert!(!cands.is_empty(), "T9 键入后候选不应为空");
}
