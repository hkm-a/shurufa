//! 雾凇拼音冒烟测试：部署真实词典，验证候选、首选、长词与限定模糊音。
//!
//! 首次运行会编译词典（约数十秒），产物缓存在 target/rime-user-data。

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

#[test]
fn rime_ice_supports_core_input_and_personalization() {
    let root = repo_root();
    let user_dir = root.join("target/rime-ice-smoke-user-data");
    if user_dir.exists() {
        std::fs::remove_dir_all(&user_dir).expect("清理旧的冒烟测试用户词典失败");
    }
    let engine = Engine::init(&root.join("schemas"), &user_dir).expect("引擎初始化失败");
    let session = engine.create_session().expect("创建会话失败");

    // 喂入拼音 nihao，应得到预编辑串与非空候选
    assert!(session.simulate("nihao"), "键序列未被引擎接受");
    let ctx = session.context();
    println!("预编辑串: {}", ctx.preedit);
    for (i, c) in ctx.candidates.iter().take(5).enumerate() {
        println!("候选{}: {} {}", i + 1, c.text, c.comment);
    }
    assert!(!ctx.preedit.is_empty(), "预编辑串为空");
    assert!(!ctx.candidates.is_empty(), "候选列表为空");
    assert!(
        ctx.candidates.iter().any(|c| c.text == "你好"),
        "候选中未出现「你好」"
    );

    // 空格上屏首选候选
    assert!(session.simulate(" "), "空格键未被引擎接受");
    let committed = session.commit().expect("未取得上屏文本");
    println!("上屏文本: {committed}");
    assert_eq!(committed, "你好");

    // try_process_key（weasel#1867 手段3 同类）：锁空闲时与 process_key
    // 等价（引擎忙时才返回 None，单会话测试里锁必然空闲）。
    assert!(session.simulate("nihao"), "try 前重新输入 nihao 失败");
    assert_eq!(
        session.try_process_key(b' ' as i32, 0),
        Some(true),
        "try_process_key 在锁空闲时应返回 Some(eaten)"
    );
    assert_eq!(
        session.commit().as_deref(),
        Some("你好"),
        "try_process_key 后上屏异常"
    );

    // 上屏后上下文应清空
    assert!(session.context().candidates.is_empty(), "上屏后候选未清空");

    // 连续选择非首选候选，确保本地学习路径可写入 userdb。
    // 注意：利好 = lì hǎo，拼音是 lihao 而非 nihao；旧版本用 nihao 找
    // "利好"纯靠残留 userdb 掩盖（历史测试误把利好学进过 nihao 候选）。
    for _ in 0..8 {
        assert!(session.simulate("lihao"), "学习用键序列未被引擎接受");
        let context = session.context();
        let index = context
            .candidates
            .iter()
            .position(|candidate| candidate.text == "利好")
            .expect("学习候选中未出现「利好」");
        if index == 0 {
            session.simulate("{Escape}");
            break;
        }
        assert!(
            session.simulate(&(index + 1).to_string()),
            "选择「利好」的候选键未被引擎接受",
        );
        assert_eq!(
            session.commit().as_deref(),
            Some("利好"),
            "学习候选上屏异常"
        );
    }

    assert!(session.simulate("lihao"), "复验学习的键序列未被引擎接受");
    let learned_context = session.context();
    assert!(
        learned_context
            .candidates
            .iter()
            .any(|candidate| candidate.text == "利好"),
        "本地学习后候选中不再包含「利好」：{:?}",
        learned_context
            .candidates
            .iter()
            .take(5)
            .map(|candidate| &candidate.text)
            .collect::<Vec<_>>(),
    );
    session.simulate("{Escape}");

    // 默认方案必须输出简体：吗（简）在候选中。
    assert!(session.simulate("ma"), "键序列未被引擎接受");
    let ctx = session.context();
    assert!(
        ctx.candidates.iter().any(|c| c.text == "吗"),
        "默认方案候选未出现简体「吗」，当前候选：{:?}",
        ctx.candidates
            .iter()
            .take(5)
            .map(|c| &c.text)
            .collect::<Vec<_>>()
    );
    session.simulate("{Escape}");

    // 连续拼音应提供至少一个四字词或更长候选，而不是只能逐字选择。
    assert!(
        session.simulate("jintianqitianhenhao"),
        "长拼音未被引擎接受"
    );
    let long_context = session.context();
    assert!(
        long_context
            .candidates
            .iter()
            .any(|candidate| candidate.text.chars().count() >= 4),
        "长输入未出现四字及以上候选：{:?}",
        long_context
            .candidates
            .iter()
            .take(10)
            .map(|candidate| &candidate.text)
            .collect::<Vec<_>>(),
    );
    session.simulate("{Escape}");

    // 模糊音（n/l、前后鼻音）功能已按产品决策砍掉（2026-08-12）：librime 的
    // derive 规则在当前构建下不参与词典查表，干净 userdb 下模糊词不出现，
    // 依赖残留 userdb 掩盖不可靠。rime_ice.schema.yaml 中的 derive 规则保留
    // （对拼音切分无害），但不再作为功能承诺。若未来启用，需词典层别名实现。
}
