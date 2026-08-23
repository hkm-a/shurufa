//! 候选窗事件管道 e2e：客户端（TSF 位）发送 CandEvent，服务端（shurufa-ui 位）
//! 收到后回发 CandCommand——验证 create_named/connect_named 传输与事件编解码
//! 在真实命名管道上的往返（阶段 6 候选窗迁出 S1）。

use std::thread;

use ime_ipc::{
    decode_cand_command, encode_cand_command, encode_cand_event, CandCommand, CandEvent, Candidate,
    Context,
};
use windows_ipc::pipe::{PipeClient, PipeServer, CAND_PIPE_NAME};

fn start_cand_server() {
    thread::spawn(move || loop {
        let server = match PipeServer::create_named(CAND_PIPE_NAME) {
            Ok(s) => s,
            Err(_) => return,
        };
        if server.accept().is_err() {
            return;
        }
        // 单连接生命周期：读到对端关闭为止；每条事件回发一条命令
        while let Ok(frame) = server.read_frame() {
            let Ok(event) = ime_ipc::decode_cand_event(&frame) else {
                continue;
            };
            let reply = match &event {
                CandEvent::Show {
                    client_id, context, ..
                } => {
                    let n = context.candidates.len();
                    CandCommand::Select {
                        client_id: *client_id,
                        index: n.saturating_sub(1),
                    }
                }
                CandEvent::Hide { client_id } => CandCommand::PageNext {
                    client_id: *client_id,
                },
            };
            let _ = server.write_frame(&encode_cand_command(&reply).unwrap());
        }
        server.reset();
    });
}

fn ctx(cands: &[(&str, &str)]) -> Context {
    Context {
        preedit: "nihao".into(),
        candidates: cands
            .iter()
            .map(|(text, comment)| Candidate {
                text: (*text).into(),
                comment: (*comment).into(),
            })
            .collect(),
        highlighted: 0,
        page_size: 9,
        ..Context::default()
    }
}

#[test]
fn cand_pipe_event_command_roundtrip() {
    start_cand_server();
    // 服务端线程就绪窗口
    thread::sleep(std::time::Duration::from_millis(200));
    let client = PipeClient::connect_named(CAND_PIPE_NAME).expect("连接 cand 管道失败");

    let event = CandEvent::Show {
        client_id: 4242,
        context: ctx(&[("你好", ""), ("拟好", "")]),
        caret_rect: (10, 20, 8, 16),
        dpi: 96,
        multi_line: false,
    };
    let reqdbg = ime_ipc::encode_request(&ime_ipc::Request::Context).unwrap();
    eprintln!("DBGREQ len={} first4={:?}", reqdbg.len(), &reqdbg[..8]);
    let dbg = encode_cand_event(&event).unwrap();
    eprintln!(
        "DBG frame len={} first4={:?}",
        dbg.len(),
        &dbg[..8.min(dbg.len())]
    );
    client.write_frame(&dbg).expect("发送事件失败");
    let frame = client
        .read_frame_timeout(std::time::Duration::from_secs(5))
        .expect("等待命令超时");
    match decode_cand_command(&frame).unwrap() {
        CandCommand::Select { client_id, index } => {
            assert_eq!(client_id, 4242);
            assert_eq!(index, 1); // 服务端回发最后一个候选索引
        }
        other => panic!("unexpected: {other:?}"),
    }

    let hide = CandEvent::Hide { client_id: 4242 };
    client
        .write_frame(&encode_cand_event(&hide).unwrap())
        .unwrap();
    let frame = client
        .read_frame_timeout(std::time::Duration::from_secs(5))
        .expect("等待 Hide 应答超时");
    assert_eq!(
        decode_cand_command(&frame).unwrap(),
        CandCommand::PageNext { client_id: 4242 }
    );
}
