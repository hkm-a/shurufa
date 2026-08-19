//! M-A3-2/3 生僻字与笔画（搜狗安卓 11.13.1 / PC 4.1 拆字）：引擎侧验证。
//!
//! - stroke 方案：五笔画 h/s/p/n/z（横竖撇捺折），一 → h、人 → pn
//! - 拆字词条（内联 rime_ice.dict.yaml）：niuniuniu → 犇、mamama → 骉

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
fn 笔画方案与拆字词条可用() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = repo_root();
    let user_dir = root.join("target/rime-stroke-chai-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理失败");
    }
    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    // 1) 笔画方案：一 = h
    assert!(
        session.select_schema("stroke"),
        "select_schema(stroke) 失败：schema_list 是否已加入？"
    );
    assert!(session.simulate("h"), "h 键序未被接受");
    let (found, seen) = find_candidate(&session, "一");
    assert!(found, "笔画 h 应出「一」，实际候选：{seen:?}");

    // 2) 拆字词条：切回雾凇拼音，niuniuniu → 犇
    assert!(session.simulate("{Escape}"));
    assert!(
        session.select_schema("rime_ice"),
        "select_schema(rime_ice) 失败"
    );
    assert!(session.simulate("niuniuniu"), "niuniuniu 键序未被接受");
    let (found, seen) = find_candidate(&session, "犇");
    assert!(
        found,
        "niuniuniu 应出「犇」（拆字词条），实际候选：{seen:?}"
    );

    // 3) mamama → 骉
    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("mamama"), "mamama 键序未被接受");
    let (found, seen) = find_candidate(&session, "骉");
    assert!(found, "mamama 应出「骉」（拆字词条），实际候选：{seen:?}");
}
