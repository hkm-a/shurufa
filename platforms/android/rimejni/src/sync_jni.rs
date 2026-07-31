//! 安卓同步 JNI 桥：把 sync-core 暴露给 Kotlin。
//!
//! 安卓端设计取舍：
//! - 关闭 mDNS（需要 multicast lock，复杂且省电策略下不可靠），
//!   手机通过输入电脑 IP 直连配对，配对后 last_addr 直连重连。
//! - 入站条目走轮询（`poll`）而非 JNI 反向回调：键盘服务在活跃时
//!   定时取队列，避免持有 JavaVM 全局引用的复杂生命周期。
//! - 配对确认为两阶段：`pair_begin`/入站请求把确认码塞入 pending 槽，
//!   Kotlin 读码展示给用户比对，`pair_respond` 放行或拒绝。发起端与
//!   接收端共用同一 pending 槽（同一时刻只处理一个配对）。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, OnceLock};

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jstring};
use jni::JNIEnv;
use tokio::runtime::Runtime;

use sync_core::{ConfirmFn, Incoming, PairPrompt, SyncConfig, SyncService};

struct PairPending {
    code: String,
    peer_name: String,
    respond: std_mpsc::Sender<bool>,
}

struct SyncState {
    rt: Runtime,
    service: SyncService,
    /// 入站条目队列：(kind, from, payload)。kind=text 时 payload 为文本；
    /// kind=image 时图片已存入历史库，payload 为条目 id。
    incoming: Arc<Mutex<VecDeque<(String, String, String)>>>,
    pending: Arc<Mutex<Option<PairPending>>>,
}

static STATE: OnceLock<SyncState> = OnceLock::new();

fn jstr(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s)
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn to_jstring(env: &JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|v| v.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// 构造一个把确认码塞入 pending 槽、阻塞等待 Kotlin 放行的确认回调。
fn make_confirm(pending: Arc<Mutex<Option<PairPending>>>) -> ConfirmFn {
    Arc::new(move |prompt: PairPrompt| {
        let (tx, rx) = std_mpsc::channel();
        {
            let mut slot = pending.lock().expect("pending 锁不可恢复");
            *slot = Some(PairPending {
                code: prompt.code,
                peer_name: prompt.peer_name,
                respond: tx,
            });
        }
        // 阻塞等待用户比对确认；连接断开等异常按拒绝处理
        let accepted = rx.recv().unwrap_or(false);
        pending.lock().expect("pending 锁不可恢复").take();
        accepted
    })
}

/// 启动同步服务。configDir 为身份/配对表目录，deviceName 展示名。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    config_dir: JString,
    device_name: JString,
) -> jboolean {
    if STATE.get().is_some() {
        return 1;
    }
    let dir = PathBuf::from(jstr(&mut env, &config_dir));
    let name = jstr(&mut env, &device_name);

    let rt = match Runtime::new() {
        Ok(rt) => rt,
        Err(_) => return 0,
    };
    let incoming = Arc::new(Mutex::new(VecDeque::new()));
    let pending = Arc::new(Mutex::new(None));

    let incoming_task = incoming.clone();
    let confirm = make_confirm(pending.clone());
    let service = rt.block_on(async move {
        let (in_tx, mut in_rx) = tokio::sync::mpsc::channel::<Incoming>(64);
        let mut config = SyncConfig::new(dir, name);
        config.enable_mdns = false; // 安卓靠 IP 直连
        let service = SyncService::start(config, in_tx, Some(confirm), Box::new(|_| {})).await?;
        // 入站条目排入轮询队列
        tokio::spawn(async move {
            while let Some(inc) = in_rx.recv().await {
                let item = match inc {
                    Incoming::Clip { from_name, text } => ("text".to_string(), from_name, text),
                    Incoming::Image { from_name, png } => {
                        // 图片直接存入历史库，队列只带条目 id
                        match crate::clip_jni::store_image(&png, &format!("同步·{from_name}")) {
                            Some(id) => ("image".to_string(), from_name, id.to_string()),
                            None => continue,
                        }
                    }
                };
                let mut q = incoming_task.lock().expect("入站队列锁不可恢复");
                if q.len() >= 64 {
                    q.pop_front();
                }
                q.push_back(item);
            }
        });
        Ok::<_, String>(service)
    });

    match service {
        Ok(service) => {
            let _ = STATE.set(SyncState {
                rt,
                service,
                incoming,
                pending,
            });
            1
        }
        Err(_) => 0,
    }
}

/// 取一条入站条目：`kind\u{1}from\u{1}payload`；队列为空返回空串。
/// kind=text 时 payload 为文本，kind=image 时 payload 为历史库条目 id。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativePoll(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let Some(state) = STATE.get() else {
        return to_jstring(&env, "");
    };
    let item = state
        .incoming
        .lock()
        .expect("入站队列锁不可恢复")
        .pop_front();
    match item {
        Some((kind, from, payload)) => {
            to_jstring(&env, &format!("{kind}\u{1}{from}\u{1}{payload}"))
        }
        None => to_jstring(&env, ""),
    }
}

/// 推送本机文本给已配对设备。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeSendClip(
    mut env: JNIEnv,
    _class: JClass,
    text: JString,
) {
    if let Some(state) = STATE.get() {
        let t = jstr(&mut env, &text);
        state.service.send_clip(&t);
    }
}

/// 已配对设备列表：每行 `指纹前12\u{1}名称`，换行分隔。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeDevices(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let Some(state) = STATE.get() else {
        return to_jstring(&env, "");
    };
    let text = state
        .service
        .peers()
        .iter()
        .map(|p| format!("{}\u{1}{}", &p.fingerprint[..12], p.name))
        .collect::<Vec<_>>()
        .join("\n");
    to_jstring(&env, &text)
}

/// 发起配对：连接 addr（ip 或 ip:port），触发本端确认回调。
/// 阻塞直至配对完成或失败，Kotlin 须在后台线程调用，并另起线程
/// 轮询 `nativePairCode` 取确认码、经 `nativePairRespond` 放行。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativePairBegin(
    mut env: JNIEnv,
    _class: JClass,
    addr: JString,
) -> jboolean {
    let Some(state) = STATE.get() else {
        return 0;
    };
    let mut addr = jstr(&mut env, &addr);
    if !addr.contains(':') {
        addr.push_str(":48632");
    }
    let confirm = make_confirm(state.pending.clone());
    let result = state
        .rt
        .block_on(async { state.service.pair_with(&addr, confirm).await });
    result.is_ok() as jboolean
}

/// 取当前待确认的配对码；无待确认返回空串。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativePairCode(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let Some(state) = STATE.get() else {
        return to_jstring(&env, "");
    };
    let guard = state.pending.lock().expect("pending 锁不可恢复");
    match guard.as_ref() {
        Some(p) => to_jstring(&env, &format!("{}\u{1}{}", p.code, p.peer_name)),
        None => to_jstring(&env, ""),
    }
}

/// 用户比对确认码后放行（accept=true）或拒绝。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativePairRespond(
    _env: JNIEnv,
    _class: JClass,
    accept: jboolean,
) {
    if let Some(state) = STATE.get() {
        let guard = state.pending.lock().expect("pending 锁不可恢复");
        if let Some(p) = guard.as_ref() {
            let _ = p.respond.send(accept != 0);
        }
    }
}
