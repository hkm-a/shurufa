//! 实机管道驱动：连接运行中的 shurufa-algo 服务，模拟键序并打印候选。
//! 用法：cargo run -p ime-ipc --example pipe_drive -- hello
//! 多参数依次模拟（每组前自动 Escape 清空）。
//! 参数以 `pk:` 开头时改为逐键 ProcessKey 喂入（走真实按键路径，
//! 候选窗装饰逻辑只在 ProcessKey 应答上生效），如 `pk:hello`。
//! 参数以 `pg:N` 开头时在当前组合上连续下翻 N 页并打印每页候选。
use ime_ipc::pipe::PipeClient;
use ime_ipc::{decode_response, encode_request, Request, Response};
use std::io::{self, Write};

fn send(client: &PipeClient, req: &Request) -> Response {
    let bytes = encode_request(req).expect("编码请求失败");
    client.write_frame(&bytes).expect("写入请求失败");
    let frame = client
        .read_frame_timeout(std::time::Duration::from_secs(10))
        .expect("读取应答超时/失败");
    decode_response(&frame).expect("解码应答失败")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let client = PipeClient::connect().expect("连接 shurufa-algo 失败（服务未运行？）");
    // 建会话
    let resp = send(&client, &Request::CreateSession);
    println!("CreateSession => {resp:?}");
    let session_ok = matches!(resp, Response::Session(Some(_)));
    if !session_ok {
        std::process::exit(1);
    }
    for seq in &args {
        // 清空上次组合
        let _ = send(&client, &Request::Simulate("{Escape}".into()));
        if let Some(word) = seq.strip_prefix("pk:") {
            let mut final_ctx = None;
            for ch in word.chars() {
                if !ch.is_ascii_graphic() {
                    continue;
                }
                let resp = send(
                    &client,
                    &Request::ProcessKey {
                        keysym: ch as i32,
                        mask: 0,
                    },
                );
                match resp {
                    Response::ProcessKey {
                        eaten,
                        commit,
                        context,
                    } => {
                        println!(
                            "--- key {ch:?} => eaten={eaten} commit={commit:?} preedit={:?}",
                            context.preedit
                        );
                        final_ctx = Some(context);
                    }
                    other => println!("--- key {ch:?} => {other:?}"),
                }
            }
            if let Some(ctx) = final_ctx {
                println!(
                    "preedit={:?} candidates={}",
                    ctx.preedit,
                    ctx.candidates.len()
                );
                for (i, c) in ctx.candidates.iter().take(12).enumerate() {
                    println!("  {i}: {} ({})", c.text, c.comment);
                }
            }
            continue;
        }
        if let Some(n_str) = seq.strip_prefix("pg:") {
            let n: usize = n_str.parse().unwrap_or(1);
            for page in 0..n {
                let r = send(&client, &Request::Simulate("{Page_Down}".into()));
                println!("--- page down {page} => {r:?}");
                if let Response::Simulate(true) = r {
                    if let Response::Context(c) = send(&client, &Request::Context) {
                        println!("  page_no={} candidates={}", c.page_no, c.candidates.len());
                        for (i, cand) in c.candidates.iter().take(12).enumerate() {
                            println!("  {i}: {} ({})", cand.text, cand.comment);
                        }
                    }
                }
            }
            continue;
        }
        let r = send(&client, &Request::Simulate(seq.clone()));
        println!("--- simulate {seq:?} => {r:?}");
        if let Response::Simulate(true) = r {
            let ctx = match send(&client, &Request::Context) {
                Response::Context(c) => c,
                other => {
                    println!("Context => {other:?}");
                    continue;
                }
            };
            println!(
                "preedit={:?} candidates={}",
                ctx.preedit,
                ctx.candidates.len()
            );
            for (i, c) in ctx.candidates.iter().take(12).enumerate() {
                println!("  {i}: {} ({})", c.text, c.comment);
            }
        }
    }
    let _ = io::stdout().flush();
}
