//! 算法服务的会话处理：把命名管道上的 [Request] 映射到 ime_bridge 会话。

use std::sync::atomic::{AtomicBool, Ordering};

use ime_bridge::Session;

use crate::pipe::PipeServer;
use crate::{decode_request, encode_response, Request, Response};

/// 全局中/英状态（模拟搜狗"全局中英"语义，悬浮条可显示/切换）：
/// - 任一会话 ToggleAscii / SetOption("ascii_mode") 都会更新它；
/// - ProcessKey 喂键前把本会话同步到全局值——所有应用下一个按键自动跟上；
/// - GetOption("ascii_mode") 返回全局值，供悬浮条/控制中心查询。
///
/// 内存态即可：中英切换属于会话运行时状态，无需持久化。
static GLOBAL_ASCII: AtomicBool = AtomicBool::new(false);

/// 处理一个客户端连接：循环读取请求、基于会话执行、写回应答。
/// `create_session` 可在会话丢失时重建（引擎由调用方持有）。
/// `decorate_process_key(raw_before, raw_after, response)` 在每条 ProcessKey
/// 应答组装后、写回客户端前调用，供调用方做候选级装饰（如 MRU 提频/记录）：
/// - `raw_before`：本键处理**前**的原始拼音（上屏落定的那串，commit 会清空
///   组合，必须提前捕获）；
/// - `raw_after`：本键处理**后**的当前组合拼音（有候选时即当前预编辑）。
pub fn serve_connection(
    server: &PipeServer,
    mut create_session: impl FnMut() -> Result<Session<'static>, String>,
    mut decorate_process_key: impl FnMut(&str, &str, Response) -> Response,
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
        let is_process_key = matches!(&request, Request::ProcessKey { .. });
        let raw_before = session.as_ref().map(|s| s.input()).unwrap_or_default();
        let response = handle_request(request, &mut session, &mut create_session);
        let response = if is_process_key {
            let raw_after = session.as_ref().map(|s| s.input()).unwrap_or_default();
            decorate_process_key(&raw_before, &raw_after, response)
        } else {
            response
        };
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
    if let Request::DestroySession = &req {
        *session = None; // 关闭后由连接循环结束时 drop
        return Response::Ok;
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
            // 打字统计埋点：按键必计一次；上屏非空时计字符数（失败静默）
            shurufa_options::stats::note_keys(1);
            // 全局中/英同步：喂键前让本会话跟上全局态（任一处切换，全部
            // 应用下一个按键即生效——搜狗全局中英语义；悬浮条切换依赖此）
            let global_ascii = GLOBAL_ASCII.load(Ordering::Relaxed);
            if s.get_option("ascii_mode") != global_ascii {
                s.set_option("ascii_mode", global_ascii);
            }
            let eaten = s.process_key(keysym, mask);
            let mut context = crate::context_from_bridge(&s.context());
            let (is_ascii, is_full_shape, _) = s.status_bits();
            context.is_ascii = is_ascii;
            context.is_full_shape = is_full_shape;
            let commit = s.commit();
            if let Some(text) = commit.as_deref() {
                if !text.is_empty() {
                    shurufa_options::stats::note_chars(text.chars().count());
                }
            }
            Response::ProcessKey {
                eaten,
                commit,
                context,
            }
        }
        Request::Commit => Response::Commit(s.commit()),
        Request::Context => Response::Context(crate::context_from_bridge(&s.context())),
        Request::Simulate(keys) => Response::Simulate(s.simulate(&keys)),
        Request::GetOption(name) => {
            // ascii_mode 是全局态：返回全局值（会话值可能滞后未同步）
            if name == "ascii_mode" {
                Response::Option(GLOBAL_ASCII.load(Ordering::Relaxed))
            } else {
                Response::Option(s.get_option(&name))
            }
        }
        Request::SetOption { name, value } => {
            if name == "ascii_mode" {
                GLOBAL_ASCII.store(value, Ordering::Relaxed);
            }
            s.set_option(&name, value);
            Response::Ok
        }
        Request::ToggleAscii => {
            // 全局优先翻转：保证任意来源的切换都落在全局态上
            let next = !GLOBAL_ASCII.load(Ordering::Relaxed);
            GLOBAL_ASCII.store(next, Ordering::Relaxed);
            s.set_option("ascii_mode", next);
            Response::Ascii(next)
        }
    }
}
