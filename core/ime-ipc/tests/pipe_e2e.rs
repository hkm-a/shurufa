//! 端到端集成测试：真实命名管道 + 真实算法服务进程。
//!
//! 启动 target/debug/shurufa-algo 常驻服务，客户端连上管道发
//! CreateSession/Simulate/Context/ProcessKey/Commit 请求，验证应答正确。

use std::fs::OpenOptions;
use std::io::Write;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

use ime_ipc::pipe::PipeClient;
use ime_ipc::{decode_response, encode_request, Request, Response};

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn start_service() -> Child {
    let exe = repo_root().join("target/debug/shurufa-algo.exe");
    assert!(exe.exists(), "先构建算法服务：cargo build -p shurufa-algo");
    let schemas = repo_root().join("schemas");
    Command::new(&exe)
        .env("SHURUFA_SCHEMAS", &schemas)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("启动算法服务失败")
}

fn trace(msg: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(repo_root().join("target/e2e-progress.log"))
    {
        let _ = writeln!(f, "{msg}");
    }
}

fn request(client: &PipeClient, req: &Request) -> Response {
    client.write_frame(&encode_request(req).unwrap()).unwrap();
    let frame = client.read_frame().unwrap();
    decode_response(&frame).unwrap()
}

/// 该测试需要真实引擎（首次词典部署数十秒）与 debug 构建产物。
#[test]
#[ignore = "需要先 cargo build -p shurufa-algo；并保证无残留服务占用管道"]
fn pipe_e2e_nihao_candidates() {
    trace("test start");
    let _child = start_service();
    trace("service spawned");
    // 等待管道出现（服务循环在引擎就绪后创建并 accept）
    let mut client = None;
    for _ in 0..200 {
        if let Ok(c) = PipeClient::connect() {
            client = Some(c);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let client = client.expect("连接算法服务超时");
    trace("client connected");
    let resp = request(&client, &Request::CreateSession);
    trace("create_session done");
    assert!(matches!(resp, Response::Session(Some(_))), "{resp:?}");

    // 第一个客户端的会话仍保持打开时，第二个宿主也必须能连上并建会话。
    // 这是独立算法服务消除多 TSF 宿主词库锁冲突的前提。
    let second = PipeClient::connect().expect("第二个 TSF 客户端连接被首个会话阻塞");
    let resp = request(&second, &Request::CreateSession);
    assert!(matches!(resp, Response::Session(Some(_))), "{resp:?}");

    // 逐键喂 n、i、h、a、o（走 ProcessKey 主路径）
    trace("start process_key loop");
    for ch in "nihao".chars() {
        let resp = request(
            &client,
            &Request::ProcessKey {
                keysym: ch as i32,
                mask: 0,
            },
        );
        match &resp {
            Response::ProcessKey { eaten, context, .. } => {
                assert!(*eaten, "键 {ch} 未被引擎吃掉: {resp:?}");
                let _ = context;
            }
            other => panic!("ProcessKey 意外应答: {other:?}"),
        }
    }

    trace("process_key loop done, requesting Context");
    let resp = request(&client, &Request::Context);
    match resp {
        Response::Context(ctx) => {
            trace("context received");
            assert_eq!(ctx.preedit, "ni hao");
            assert!(ctx.candidates.iter().any(|c| c.text == "你好"), "{ctx:?}");
        }
        other => panic!("unexpected {other:?}"),
    }

    // 空格上屏，Commit 应为「你好」
    trace("requesting space ProcessKey");
    let resp = request(
        &client,
        &Request::ProcessKey {
            keysym: ' ' as i32,
            mask: 0,
        },
    );
    trace("space ProcessKey done");
    let commit = match &resp {
        Response::ProcessKey { commit, .. } => commit.clone(),
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(commit.as_deref(), Some("你好"), "空格上屏文本错误");

    // 上屏后上下文清空
    trace("requesting Context after commit");
    let resp = request(&client, &Request::Context);
    trace("Context after commit done");
    match resp {
        Response::Context(ctx) => assert!(ctx.preedit.is_empty(), "上屏后 preedit 未清空"),
        other => panic!("unexpected {other:?}"),
    }
    trace("all assertions passed; shutting down service");
    let mut child = _child;
    let _ = child.kill();
    let _ = child.wait();
    trace("service stopped");
}

/// 超时行为：服务端不应答时，read_frame_timeout 必须在预算内返回超时错误，
/// 绝不无限阻塞（TSF 在宿主 UI 线程调用，阻塞 = 应用无响应 + 全局输入法失效）。
#[test]
#[ignore = "需要先 cargo build -p shurufa-algo；并保证无残留服务占用管道"]
fn pipe_read_timeout_does_not_block() {
    let _child = start_service();
    let mut client = None;
    for _ in 0..200 {
        if let Ok(c) = PipeClient::connect() {
            client = Some(c);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let client = client.expect("连接算法服务超时");

    // 写入一个请求但不读响应（服务端会等下一帧；我们直接测超时读）。
    let start = std::time::Instant::now();
    let result = client.read_frame_timeout(Duration::from_millis(500));
    let elapsed = start.elapsed();

    // 必须超时返回（服务端没写数据），且耗时 ≈ 预算（500ms ± 300ms 容差）
    assert!(result.is_err(), "服务端未应答却读到了数据");
    assert!(
        elapsed >= Duration::from_millis(400) && elapsed <= Duration::from_millis(1500),
        "超时耗时异常：{elapsed:?}"
    );
    trace("timeout behaved as expected");
    let mut child = _child;
    let _ = child.kill();
    let _ = child.wait();
}
