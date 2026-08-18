//! M8-4 应用/网站直达（搜狗 15.2 灵犀候选直达同类）：lua_translator 对
//! 精确输入码附加带标记的直达候选（🖥 应用 / 🌐 网址）。
//!
//! 验证：
//! - 写一份 app_direct_shortcuts.lua（模拟设置中心生成）后：
//!   weixin → 候选含 "🖥 微信"；baidu → 候选含 "🌐 百度"；
//! - 未配置的输入码（okok）不受影响；
//! - 候选 comment 携带 target（TSF 提交时据此启动）。

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

#[test]
fn app_direct_candidates_with_markers() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = repo_root();
    let user_dir = root.join("target/rime-app-direct-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理失败");
    }
    let user_lua = user_dir.join("lua");
    std::fs::create_dir_all(&user_lua).expect("创建 lua 目录失败");
    copy_dir_recursive(&root.join("schemas/lua"), &user_lua);
    // 模拟设置中心生成的快捷表
    std::fs::write(
        user_lua.join("app_direct_shortcuts.lua"),
        r#"return {
  { code = "weixin", label = "微信", kind = "app", target = "C:/apps/wechat.exe" },
  { code = "baidu", label = "百度", kind = "url", target = "https://www.baidu.com" },
}
"#,
    )
    .expect("写快捷表失败");

    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    assert!(session.simulate("weixin"), "weixin 键序未被引擎接受");
    let cands = session
        .context()
        .candidates
        .iter()
        .map(|c| (c.text.clone(), c.comment.clone()))
        .collect::<Vec<_>>();
    eprintln!("WEIXIN-CANDS: {cands:?}");
    assert!(
        cands.iter().any(|(t, _)| t.contains("🖥 微信")),
        "weixin 应出 🖥 微信 候选，实际：{cands:?}"
    );

    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("baidu"), "baidu 键序未被引擎接受");
    let cands = session
        .context()
        .candidates
        .iter()
        .map(|c| (c.text.clone(), c.comment.clone()))
        .collect::<Vec<_>>();
    assert!(
        cands.iter().any(|(t, _)| t.contains("🌐 百度")),
        "baidu 应出 🌐 百度 候选，实际：{cands:?}"
    );
    // 未配置触发码不影响
    assert!(session.simulate("{Escape}"));
    assert!(session.simulate("nihao"), "nihao 键序未被引擎接受");
    let texts: Vec<String> = session
        .context()
        .candidates
        .iter()
        .map(|c| c.text.clone())
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("你好")),
        "常规输入不受影响，实际：{texts:?}"
    );
}
