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
//!
//! 所有 `#[no_mangle]` 入口经 [`crate::jni_catch`] 包裹，防止 panic
//! 跨 FFI 传播导致进程 abort；锁污染时以 `.lock().ok()` 安全降级。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, OnceLock};

use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jint, jstring};
use jni::JNIEnv;
use tokio::runtime::Runtime;

use sync_core::{ConfirmFn, Incoming, PairPrompt, SyncConfig, SyncService, MAX_CLIP_FILE_BYTES};

struct PairPending {
    code: String,
    peer_name: String,
    respond: std_mpsc::Sender<bool>,
}

struct SyncState {
    rt: Runtime,
    service: SyncService,
    /// 入站条目队列：(kind, from, payload)。kind=text 时 payload 为文本；
    /// kind=image/file 时内容已存入历史库，payload 为条目 id。
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

fn received_file_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let name = Path::new(name).file_name()?.to_str()?;
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    let received = dir.join("received");
    std::fs::create_dir_all(&received).ok()?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    Some(received.join(format!("{stamp}_{name}")))
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
    crate::jni_catch(
        || {
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
            let received_dir = dir.clone();
            let confirm = make_confirm(pending.clone());
            let service = rt.block_on(async move {
                let (in_tx, mut in_rx) = tokio::sync::mpsc::channel::<Incoming>(64);
                let mut config = SyncConfig::new(dir, name);
                config.enable_mdns = false; // 安卓靠 IP 直连
                let service =
                    SyncService::start(config, in_tx, Some(confirm), Box::new(|_| {})).await?;
                // 入站条目排入轮询队列。图片/文件入库与落盘是阻塞 IO，
                // 移到 spawn_blocking，避免占用 tokio worker 线程。
                tokio::spawn(async move {
                    while let Some(inc) = in_rx.recv().await {
                        let incoming_task = incoming_task.clone();
                        let received_dir = received_dir.clone();
                        tokio::task::spawn_blocking(move || {
                            let item = match inc {
                                Incoming::Clip { from_name, text, .. } => {
                                    ("text".to_string(), from_name, text)
                                }
                                Incoming::Image { from_name, png } => {
                                    // 图片直接存入历史库，队列只带条目 id
                                    match crate::clip_jni::store_image(
                                        &png,
                                        &format!("同步·{from_name}"),
                                    ) {
                                        Some(id) => {
                                            ("image".to_string(), from_name, id.to_string())
                                        }
                                        None => return,
                                    }
                                }
                                Incoming::File {
                                    from_name,
                                    name,
                                    data,
                                    ..
                                } => {
                                    if data.is_empty() || data.len() > MAX_CLIP_FILE_BYTES {
                                        return;
                                    }
                                    let Some(path) = received_file_path(&received_dir, &name)
                                    else {
                                        return;
                                    };
                                    if std::fs::write(&path, data).is_err() {
                                        return;
                                    }
                                    let paths = vec![path.to_string_lossy().into_owned()];
                                    match crate::clip_jni::store_files(
                                        &paths,
                                        &format!("同步·{from_name}"),
                                    ) {
                                        Some(id) => ("file".to_string(), from_name, id.to_string()),
                                        None => return,
                                    }
                                }
                                // 对端的搜索响应当前只供 PC 侧 CLI/日志消费，Android 暂不展示。
                                Incoming::SearchResults { .. } => return,
                            };
                            let mut q = incoming_task.lock().expect("入站队列锁不可恢复");
                            if q.len() >= 64 {
                                q.pop_front();
                            }
                            q.push_back(item);
                        });
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
        },
        0,
    )
}

/// 保存自托管中继地址。已启动的 `SyncService` 由 OnceLock 持有，配置将在
/// 下次重启输入法进程后读取并生效。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeSetRelayAddr(
    mut env: JNIEnv,
    _class: JClass,
    config_dir: JString,
    relay_addr: JString,
) -> jboolean {
    crate::jni_catch(
        || {
            let dir = PathBuf::from(jstr(&mut env, &config_dir));
            let relay_addr = jstr(&mut env, &relay_addr);
            let relay_addr = relay_addr.trim();
            let relay = if relay_addr.is_empty() || relay_addr.eq_ignore_ascii_case("off") {
                None
            } else {
                Some(relay_addr)
            };
            sync_core::save_relay_addr(&dir, relay).is_ok() as jboolean
        },
        0,
    )
}

/// 取一条入站条目：`kind\u{1}from\u{1}payload`；队列为空返回空串。
/// kind=text 时 payload 为文本，kind=image/file 时 payload 为历史库条目 id。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativePoll(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let default = to_jstring(&env, "");
    crate::jni_catch(
        || {
            let Some(state) = STATE.get() else {
                return to_jstring(&env, "");
            };
            let item = match state.incoming.lock() {
                Ok(mut guard) => guard.pop_front(),
                Err(_) => None,
            };
            match item {
                Some((kind, from, payload)) => {
                    to_jstring(&env, &format!("{kind}\u{1}{from}\u{1}{payload}"))
                }
                None => to_jstring(&env, ""),
            }
        },
        default,
    )
}

/// 推送本机文本给已配对设备。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeSendClip(
    mut env: JNIEnv,
    _class: JClass,
    text: JString,
) {
    crate::jni_catch(
        || {
            if let Some(state) = STATE.get() {
                let t = jstr(&mut env, &text);
                state.service.send_clip(&t);
            }
        },
        (),
    )
}

/// 推送本机图片（PNG 字节）给已配对设备。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeSendImage(
    env: JNIEnv,
    _class: JClass,
    data: JByteArray,
) {
    crate::jni_catch(
        || {
            if let Some(state) = STATE.get() {
                if let Ok(bytes) = env.convert_byte_array(&data) {
                    state.service.send_image(&bytes);
                }
            }
        },
        (),
    )
}

/// 推送本机文件给已配对设备。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeSendFile(
    mut env: JNIEnv,
    _class: JClass,
    name: JString,
    mime_type: JString,
    data: JByteArray,
) {
    crate::jni_catch(
        || {
            if let Some(state) = STATE.get() {
                if let Ok(bytes) = env.convert_byte_array(&data) {
                    let name = jstr(&mut env, &name);
                    let mime_type = jstr(&mut env, &mime_type);
                    state.service.send_file(&name, &mime_type, &bytes);
                }
            }
        },
        (),
    )
}

/// 返回同步核心允许传输的单张 PNG 上限，供 Kotlin 转码阶段使用同一约束。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeMaxImageBytes(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    crate::jni_catch(|| sync_core::MAX_CLIP_IMAGE_BYTES as jint, 0)
}

/// 返回同步核心允许传输的单文件上限。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeMaxFileBytes(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    crate::jni_catch(|| sync_core::MAX_CLIP_FILE_BYTES as jint, 0)
}

/// 已配对设备列表：每行 `指纹前12\u{1}名称`，换行分隔。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeDevices(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let default = to_jstring(&env, "");
    crate::jni_catch(
        || {
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
        },
        default,
    )
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
    crate::jni_catch(
        || {
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
        },
        0,
    )
}

/// 取当前待确认的配对码；无待确认返回空串。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativePairCode(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let default = to_jstring(&env, "");
    crate::jni_catch(
        || {
            let Some(state) = STATE.get() else {
                return to_jstring(&env, "");
            };
            let guard = match state.pending.lock() {
                Ok(g) => g,
                Err(_) => return to_jstring(&env, ""),
            };
            match guard.as_ref() {
                Some(p) => to_jstring(&env, &format!("{}\u{1}{}", p.code, p.peer_name)),
                None => to_jstring(&env, ""),
            }
        },
        default,
    )
}

/// 用户比对确认码后放行（accept=true）或拒绝。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativePairRespond(
    _env: JNIEnv,
    _class: JClass,
    accept: jboolean,
) {
    crate::jni_catch(
        || {
            if let Some(state) = STATE.get() {
                if let Ok(guard) = state.pending.lock() {
                    if let Some(p) = guard.as_ref() {
                        let _ = p.respond.send(accept != 0);
                    }
                }
            }
        },
        (),
    )
}
