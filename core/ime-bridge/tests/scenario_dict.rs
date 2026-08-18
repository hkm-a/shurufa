//! M10-1 专业词模式（搜狗 16.2 场景词库同类）：部署即生效验证。
//!
//! 实测：librime 编译 schemas/ 目录下全部 .dict.yaml，场景词库文件存在即
//! 生效（拼音可直接打出场景词）；librime 1.17 的列表 patch 不支持 "+item"
//! 追加（会把 engine/translators 整体替换、拼音失效），故不用 custom patch
//! 挂载（见 platforms/windows-settings 的 save_scenario_dict 注释）。
//!
//! 验证：
//! - zhuanyeciku → 「专业词库」（scenario_doctor.dict.yaml 词条）
//! - 常规拼音 nihao → 你好（引擎不回归）

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

#[test]
fn 场景词库部署即生效且常规输入正常() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = repo_root();
    let user_dir = root.join("target/rime-scenario-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理失败");
    }
    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    // 场景词库（scenario_doctor.dict.yaml）部署即生效
    assert!(session.simulate("zhuanyeciku"), "zhuanyeciku 键序未被接受");
    let cands = candidate_texts(&session);
    assert!(
        cands.iter().any(|c| c.contains("专业词库")),
        "zhuanyeciku 应出专业词库（场景词库部署即生效），实际：{cands:?}"
    );

    // 常规拼音不受影响
    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("nihao"), "nihao 键序未被接受");
    let cands = candidate_texts(&session);
    assert!(
        cands.iter().any(|c| c.contains("你好")),
        "常规输入 nihao 应出你好，实际：{cands:?}"
    );
}
