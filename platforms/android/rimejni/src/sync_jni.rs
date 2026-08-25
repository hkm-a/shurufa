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

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jint, jlong, jstring};
use jni::{JNIEnv, JavaVM};
use tokio::runtime::Runtime;

use sync_core::{
    config_sync, ConfirmFn, FileConfirmFn, FileOfferPrompt, Incoming, PairPrompt, SyncConfig,
    SyncService, MAX_CLIP_FILE_BYTES,
};

struct PairPending {
    code: String,
    peer_name: String,
    respond: std_mpsc::Sender<bool>,
}

struct SyncState {
    rt: Runtime,
    service: SyncService,
    /// filesDir（配置根目录），用于备份列表/恢复。
    config_root: PathBuf,
    /// 入站条目队列：(kind, from, payload)。kind=text 时 payload 为文本；
    /// kind=image/file 时内容已存入历史库，payload 为条目 id。
    incoming: Arc<Mutex<VecDeque<(String, String, String)>>>,
    pending: Arc<Mutex<Option<PairPending>>>,
}

static STATE: OnceLock<SyncState> = OnceLock::new();

/// 当前在等用户异步决策的 Offer 集合（transfer_id → 决策通道）；
/// Kotlin 通知上的「接受 / 拒绝」通过 `nativeConfirmOffer` 把布尔送回，
/// 阻塞在回调上的 Rust 线程被唤醒后即返回给 sync-core。
static PENDING_DECISIONS: OnceLock<Mutex<HashMap<u64, std_mpsc::Sender<bool>>>> = OnceLock::new();

fn pending_decisions() -> &'static Mutex<HashMap<u64, std_mpsc::Sender<bool>>> {
    PENDING_DECISIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 决策超时：发送侧看门狗 30s 后会自动放弃，上行留出 5s 余量。
const OFFER_DECISION_TIMEOUT: Duration = Duration::from_secs(25);

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
            let config_root = dir.parent().unwrap_or(&dir).to_path_buf();
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
                                Incoming::Clip {
                                    from_name, text, ..
                                } => ("text".to_string(), from_name, text),
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
                                // 配置/短语/皮肤同步：三方冲突合并 + 增量状态落盘。
                                Incoming::ConfigFile {
                                    from_name,
                                    kind,
                                    name,
                                    data,
                                } => {
                                    let root = received_dir.parent().unwrap_or(&received_dir);
                                    let _ = config_sync::apply_incoming(root, &kind, &name, &data);
                                    let _ = (from_name, name);
                                    return;
                                }
                                // custom_phrase 按码增量（config-patch-v1）：
                                // 基准匹配走三方合并，不匹配保守合并。
                                Incoming::ConfigPatch {
                                    from_name,
                                    kind,
                                    name,
                                    base_sha256,
                                    ops,
                                } => {
                                    let root = received_dir.parent().unwrap_or(&received_dir);
                                    let _ = config_sync::apply_patch_incoming(
                                        root,
                                        &kind,
                                        &name,
                                        &base_sha256,
                                        &ops,
                                    );
                                    let _ = (from_name, name);
                                    return;
                                }
                                // 文件 v3 事件：Android 侧仅做日志级提示，
                                // 由宿主的 ClipboardSyncService 自行弹出
                                // Notification / 历史入库。
                                Incoming::FileOffer { .. } => return,
                                Incoming::FileProgress { .. } => return,
                                Incoming::FileTransferDone { .. } => return,
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
                        config_root: config_root.clone(),
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

/// 推送本机路径指向的文件给已配对设备（v3）：由 SyncService 完成
/// 分块 / 等待 Accept / 等 ACK，全程在同步 runtime 内推进，不占额外线程。
/// 返回 false=同步未启动 / 路径非法 / 文件超过 64MB。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeSendFilePath(
    mut env: JNIEnv,
    _class: JClass,
    path: JString,
) -> jboolean {
    crate::jni_catch(
        || {
            let Some(state) = STATE.get() else { return 0 };
            let path = jstr(&mut env, &path);
            if path.is_empty() {
                return 0;
            }
            match state.service.send_file_path(std::path::Path::new(&path)) {
                Ok(_) => 1,
                Err(_) => 0,
            }
        },
        0,
    )
}

/// 推送本机配置/短语/皮肤文件给已配对设备（config-sync-v1）。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeSendConfig(
    mut env: JNIEnv,
    _class: JClass,
    kind: JString,
    path: JString,
) -> jboolean {
    crate::jni_catch(
        || {
            let Some(state) = STATE.get() else { return 0 };
            let kind = jstr(&mut env, &kind);
            let path = jstr(&mut env, &path);
            let path_buf = std::path::PathBuf::from(&path);
            let data = match config_sync::prepare_send(&state.config_root, &kind, &path_buf) {
                Ok(Some(data)) => data,
                _ => return 0,
            };
            let name = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("config.txt")
                .to_owned();
            // custom_phrase 优先按码增量；无基准/无变化时回退全量。
            match config_sync::prepare_patch(&state.config_root, &kind, &path_buf) {
                Ok(Some(payload)) => {
                    state.service.send_config_patch(
                        &kind,
                        &name,
                        &payload.base_sha256,
                        &payload.ops,
                        &payload.data,
                    );
                }
                _ => {
                    state.service.send_config(&kind, &name, &data);
                }
            }
            // 只有确有在线对端时才标记“已同步”；离线发送会丢，不能消耗增量状态。
            if state.service.connected_count() > 0 {
                let _ = config_sync::mark_sent(&state.config_root, &kind, &data);
            }
            1
        },
        0,
    )
}

/// 列出配置同步备份文件名（每行一个）。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeConfigBackups(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    crate::jni_catch(
        || {
            let Some(state) = STATE.get() else {
                return to_jstring(&env, "");
            };
            let dir = state.config_root.join("sync-config-backups");
            let mut names = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_file() {
                            if let Some(name) = entry.file_name().to_str() {
                                names.push(name.to_string());
                            }
                        }
                    }
                }
            }
            names.sort();
            to_jstring(&env, &names.join("\n"))
        },
        to_jstring(&env, ""),
    )
}

/// 从备份恢复配置/短语/皮肤。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeRestoreConfigBackup(
    mut env: JNIEnv,
    _class: JClass,
    file: JString,
) -> jboolean {
    crate::jni_catch(
        || {
            let Some(state) = STATE.get() else { return 0 };
            let file = jstr(&mut env, &file);
            let backup = state.config_root.join("sync-config-backups").join(&file);
            if !backup.is_file() {
                return 0;
            }
            let Some(kind) = config_sync::kind_from_backup_name(&file) else {
                return 0;
            };
            let root = &state.config_root;
            let target = match kind {
                "options" => root.join("options.json"),
                "skin" => root.join("shurufa-skin.json"),
                "custom_phrase" => root.join("rime").join("custom_phrase.txt"),
                _ => return 0,
            };
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::copy(&backup, &target) {
                Ok(_) => 1,
                Err(_) => 0,
            }
        },
        0,
    )
}

/// 列出待用户确认的配置冲突记录。
/// 每行一条，字段用 `\u{1}` 分隔：ts_ms、kind、name、local_backup、remote_backup、merged_sha256。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeConfigConflicts(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let default = to_jstring(&env, "");
    crate::jni_catch(
        || {
            let Some(state) = STATE.get() else {
                return to_jstring(&env, "");
            };
            let log = config_sync::load_conflicts(&state.config_root);
            let lines = log
                .conflicts
                .iter()
                .map(|r| {
                    format!(
                        "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
                        r.ts_ms, r.kind, r.name, r.local_backup, r.remote_backup, r.merged_sha256
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            to_jstring(&env, &lines)
        },
        default,
    )
}

/// 移除一条已处理的配置冲突记录。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeRemoveConfigConflict(
    mut env: JNIEnv,
    _class: JClass,
    remote_backup: JString,
) -> jboolean {
    crate::jni_catch(
        || {
            let Some(state) = STATE.get() else { return 0 };
            let remote_backup = jstr(&mut env, &remote_backup);
            config_sync::remove_conflict(&state.config_root, &remote_backup).unwrap_or(false)
                as jboolean
        },
        0,
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

/// 构造一个把 FileOffer 转交 Kotlin 的通知回调：
/// 1) 调用 `IFileConfirmCallback.onOffer(name, sizeBytes, mime, peerFp)`；
///    Kotlin 侧在此内只负责弹系统通知并立刻返回 true ——返回值约定
///    true = "已弹通知，稍后经 nativeConfirmOffer 给结果"；
///    false = "立即拒绝"。
/// 2) 若 Kotlin 返回 true，则阻塞在本工作线程上收 `nativeConfirmOffer`
///    的布尔结果，最长等 `OFFER_DECISION_TIMEOUT`，超时按拒绝处理。
///
/// 回调运行在 sync-core 的 spawn_blocking worker 上，可安全阻塞。
fn make_file_confirm(jvm: JavaVM, callback: GlobalRef) -> FileConfirmFn {
    Arc::new(move |prompt: FileOfferPrompt| {
        let (tx, rx) = std_mpsc::channel::<bool>();
        // 入栈 pending，让 Kotlin 同步调 nativeLatestPendingOfferId 拿到 id，
        // 也让 nativeConfirmOffer 能把布尔送回本条回调。
        pending_decisions()
            .lock()
            .expect("pending 决策锁不可恢复")
            .insert(prompt.transfer_id, tx);
        // 调 Kotlin onOffer。失败（JNI 异常 / attach 失败 / 方法返回 false）
        // 一律降级为立即拒绝。
        let defer = (|| -> bool {
            let mut env = match jvm.attach_current_thread() {
                Ok(env) => env,
                Err(_) => return false,
            };
            let name = match env.new_string(&prompt.name) {
                Ok(s) => s,
                Err(_) => return false,
            };
            let mime = match env.new_string(&prompt.mime) {
                Ok(s) => s,
                Err(_) => return false,
            };
            let peer = match env.new_string(&prompt.peer_fp) {
                Ok(s) => s,
                Err(_) => return false,
            };
            let ret = env.call_method(
                callback.as_obj(),
                "onOffer",
                "(Ljava/lang/String;JLjava/lang/String;Ljava/lang/String;)Z",
                &[
                    JValue::Object(&JObject::from(name)),
                    JValue::Long(prompt.size as jlong),
                    JValue::Object(&JObject::from(mime)),
                    JValue::Object(&JObject::from(peer)),
                ],
            );
            let _ = env.exception_check().map(|pending| {
                if pending {
                    let _ = env.exception_clear();
                }
            });
            match ret {
                Ok(v) => v.z().unwrap_or(false),
                Err(_) => false,
            }
        })();
        if !defer {
            pending_decisions()
                .lock()
                .expect("pending 决策锁不可恢复")
                .remove(&prompt.transfer_id);
            return false;
        }
        let accepted = rx.recv_timeout(OFFER_DECISION_TIMEOUT).unwrap_or(false);
        pending_decisions()
            .lock()
            .expect("pending 决策锁不可恢复")
            .remove(&prompt.transfer_id);
        accepted
    })
}

/// 由 Kotlin 把 IFileConfirmCallback 的全局引用注册为 sync-core 的
/// `FileConfirmFn`。重复调用会覆盖旧回调；传 null 则退回到自动决策
/// （< FILE_AUTO_ACCEPT_MAX + MIME 白名单）。
///
/// 要求 SyncBridge 的静态初始化块已 `System.loadLibrary`；调用方需保证
/// `nativeStart` 已先行初始化 `STATE`。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeSetFileConfirmCallback(
    env: JNIEnv,
    _class: JClass,
    callback: JObject,
) -> jboolean {
    crate::jni_catch(
        || {
            let Some(state) = STATE.get() else { return 0 };
            if callback.is_null() {
                state.service.set_file_confirm_handler(None);
                return 1;
            }
            let jvm = match env.get_java_vm() {
                Ok(v) => v,
                Err(_) => return 0,
            };
            let global = match env.new_global_ref(&callback) {
                Ok(g) => g,
                Err(_) => return 0,
            };
            state
                .service
                .set_file_confirm_handler(Some(make_file_confirm(jvm, global)));
            1
        },
        0,
    )
}

/// 返回当前最近一次待决策 transfer_id。
/// Kotlin 的 `onOffer` 被 Rust 同步调用时尚未拿到 transfer_id——通过这里
/// 反查；无待决策 Offer 时返回 0。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeLatestPendingOfferId(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    crate::jni_catch(
        || {
            let guard = match pending_decisions().lock() {
                Ok(g) => g,
                Err(_) => return 0,
            };
            guard.keys().copied().max().unwrap_or(0) as jlong
        },
        0,
    )
}

/// 用户在系统通知上点击「接受 / 拒绝」后经此放行阻塞中的 file_confirm 回调；
/// transfer_id 携带 `onOffer` 阶段通过 `nativeLatestPendingOfferId` 取到的同值。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_SyncBridge_nativeConfirmOffer(
    _env: JNIEnv,
    _class: JClass,
    transfer_id: jlong,
    accept: jboolean,
) {
    crate::jni_catch(
        || {
            if transfer_id <= 0 {
                return;
            }
            let tx = {
                let mut guard = match pending_decisions().lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                guard.remove(&(transfer_id as u64))
            };
            if let Some(tx) = tx {
                let _ = tx.send(accept != 0);
            }
        },
        (),
    )
}
