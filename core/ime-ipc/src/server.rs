//! 算法服务的会话处理：把命名管道上的 [Request] 映射到 ime_bridge 会话。

use ime_bridge::Session;

use crate::pipe::PipeServer;
use crate::{decode_request, encode_response, Request, Response};

/// 处理一个客户端连接：循环读取请求、基于会话执行、写回应答。
/// `create_session` 可在会话丢失时重建（引擎由调用方持有）。
pub fn serve_connection(
    server: &PipeServer,
    mut create_session: impl FnMut() -> Result<Session<'static>, String>,
) {
    let mut session: Option<Session<'static>> = None;
    let _ = create_session().map(|s| session = Some(s));
    loop {
        let frame = match server.read_frame() {
            Ok(f) => f,
            Err(_) => return, // 对端关闭/出错，结束本连接
        };
        let request = match decode_request(&frame) {
            Ok(r) => r,
            Err(e) => {
                let _ = server.write_frame(&encode_response(&Response::Error(e)).unwrap());
                continue;
            }
        };
        let response = handle_request(request, &mut session, &mut create_session);
        if server
            .write_frame(&encode_response(&response).unwrap())
            .is_err()
        {
            return;
        }
    }
}

/// 单条请求 → 应答。会话缺失时按需重建。
fn handle_request(
    req: Request,
    session: &mut Option<Session<'static>>,
    create_session: &mut impl FnMut() -> Result<Session<'static>, String>,
) -> Response {
    // 先处理与生命周期无关的分支
    match &req {
        Request::DestroySession => {
            *session = None; // 关闭后由连接循环结束时 drop
            return Response::Ok;
        }
        _ => {}
    }
    if session.is_none() {
        match create_session() {
            Ok(s) => *session = Some(s),
            Err(e) => return Response::Error(e),
        }
    }
    let s = session.as_ref().expect("已确保会话存在");
    match req {
        Request::CreateSession => Response::Session(Some(1)),
        Request::DestroySession => Response::Ok,
        Request::ProcessKey { keysym, mask } => {
            let eaten = s.process_key(keysym, mask);
            let context = crate::context_from_bridge(&s.context());
            Response::ProcessKey {
                eaten,
                commit: s.commit(),
                context,
            }
        }
        Request::Commit => Response::Commit(s.commit()),
        Request::Context => Response::Context(crate::context_from_bridge(&s.context())),
        Request::Simulate(keys) => Response::Simulate(s.simulate(&keys)),
        Request::GetOption(name) => Response::Option(s.get_option(&name)),
        Request::SetOption { name, value } => {
            s.set_option(&name, value);
            Response::Ok
        }
        Request::ToggleAscii => {
            let now = s.toggle_ascii();
            Response::Ascii(now)
        }
    }
}
