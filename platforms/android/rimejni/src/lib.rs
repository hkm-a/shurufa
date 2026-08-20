//! Android JNI 桥：把 ime-bridge 的引擎会话暴露给 Kotlin 键盘服务。
//!
//! 生命周期：`nativeInit` 在后台线程完成引擎初始化与部署（首次约
//! 数秒到数十秒），成功后建立进程级单一会话；按键与查询都走该会话。
//! 上下文以 `\u{1}` 分隔的扁平字符串返回，避免引入 JSON 依赖：
//! 取输入上下文：`preedit \u{1} highlighted \u{1} cursor \u{1} 候选…`；空组合返回空串。
//!
//! 安全约定：所有 `#[no_mangle]` JNI 入口都经 [`jni_catch`] 包裹，
//! 把 panic 拦截在 FFI 边界内，否则 panic 跨 JNI 展开属于未定义行为
//!（绝大多数 Android 设备上直接 abort 整个进程）。

use std::sync::{Mutex, OnceLock};

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jint, jstring};
use jni::JNIEnv;

use ime_bridge::{Engine, Session};

mod clip_jni;
mod scheme_jni;
mod sync_jni;

static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();
static SESSION: Mutex<Option<Session<'static>>> = Mutex::new(None);

/// options 方案 id → librime schema_id（与 schemas/ 文件名一致；事实源在
/// shurufa_options::schema_id_of，这里仅是转发，避免 JNI 层重复维护）。
fn schema_id_for(scheme: &str) -> &'static str {
    shurufa_options::schema_id_of(scheme)
}

/// 把 options 的 input_scheme 应用到全局会话（librime select_schema）。
/// 会话未就绪（引擎还在预热）时返回 false，调用方自行决定是否容忍。
pub(crate) fn apply_input_scheme(scheme: &str) -> bool {
    let session = SESSION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match session.as_ref() {
        Some(s) => s.select_schema(schema_id_for(scheme)),
        None => false,
    }
}

/// JNI 入口的统一 panic 守卫。
///
/// 任何从 Rust panic 越出 `extern "C"`/`extern "system"` 函数回 Java
/// 都是未定义行为。这里用 `catch_unwind` 把 panic 拦下，仅打印日志并
/// 返回 `default` 兜底值，避免整个进程崩溃。锁被其他线程 panic 污染时
/// 同样借此路径安全退化而非继续 `expect` abort。
pub(crate) fn jni_catch<T>(f: impl FnOnce() -> T, default: T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(v) => v,
        Err(payload) => {
            // catch_unwind 不会触发默认 panic hook，这里手动输出定位信息
            let msg = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(|s| s.as_str()))
                .unwrap_or("未知 panic");
            eprintln!("[rimejni] 已捕获 panic（防止跨 FFI 传播）: {msg}");
            default
        }
    }
}

fn jstring_to_string(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s)
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn to_jstring(env: &JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|v| v.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
/// 把会话上下文序列化为扁平串：`preedit \u{1} highlighted \u{1} cursor \u{1} 候选…`。
/// 与 nativeContext / nativeChangePage 共用，避免两份重复逻辑漂移。
fn session_context_string(s: &Session) -> String {
    let ctx = s.context();
    if ctx.preedit.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(64);
    out.push_str(&ctx.preedit);
    out.push('\u{1}');
    out.push_str(&ctx.highlighted.to_string());
    out.push('\u{1}');
    out.push_str(&ctx.cursor_pos.to_string());
    for c in &ctx.candidates {
        out.push('\u{1}');
        out.push_str(&c.text);
    }
    out
}


/// 主动部署指定方案（编译附加词典，如 radical_pinyin 反查词典）。
/// 增量部署不编译附加 translator 的词典，需初始化后显式调用一次。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeDeploySchema(
    mut env: JNIEnv,
    _class: JClass,
    schema: JString,
) -> jboolean {
    jni_catch(
        || {
            let schema = jstring_to_string(&mut env, &schema);
            let ok = match ENGINE.get() {
                Some(Ok(engine)) => engine.deploy_schema(&schema),
                _ => false,
            };
            ok as jboolean
        },
        false as jboolean,
    )
}

/// 初始化引擎并建立会话；重复调用幂等。阻塞直至部署完成，
/// Kotlin 侧必须在后台线程调用。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeInit(
    mut env: JNIEnv,
    _class: JClass,
    shared_dir: JString,
    user_dir: JString,
) -> jboolean {
    jni_catch(
        || {
            let shared = jstring_to_string(&mut env, &shared_dir);
            let user = jstring_to_string(&mut env, &user_dir);
            // Android 没有 APPDATA：options/stats 的数据目录指向应用用户目录，
            // 必须在 Engine::init 之前设置（options crate 亦可能被引擎路径引用）。
            std::env::set_var("SHURUFA_DATA_DIR", &user);
            let engine = ENGINE.get_or_init(|| {
                Engine::init(std::path::Path::new(&shared), std::path::Path::new(&user))
            });
            let Ok(engine) = engine.as_ref() else {
                return 0;
            };
            // P4-3：增量部署不编译附加词典（radical_pinyin），必须在创建会话前
            // 主动部署一次，否则 session 加载 rime_ice 时 radical_lookup translator
            // 因词典缺失而创建失败，uU 反查永远无候选。
            let _ = engine.deploy_schema("radical_pinyin.schema.yaml");
            // 锁被 panic 污染时经 into_inner 直接恢复其内部值继续使用
            let mut session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if session.is_none() {
                match engine.create_session() {
                    Ok(s) => *session = Some(s),
                    Err(_) => return 0,
                }
            }
            // 把持久化的 input_scheme 应用到会话（M-A1-3：方案真正生效，
            // 此前仅写偏好未切引擎；t9 等新方案依赖此步）。
            // 注意：必须先释放 SESSION 锁再调用（apply_input_scheme 会再次
            // 加锁，Mutex 非重入——模拟器实测在持有锁时调用会自死锁，导致
            // 部署线程永不返回、engineReady 永远为 false）。
            let scheme = shurufa_options::load().input_scheme;
            drop(session);
            let _ = apply_input_scheme(&scheme);
            1
        },
        0,
    )
}

/// 喂键（X11 keysym 与修饰掩码，与桌面端一致）；返回是否被引擎吃掉。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeProcessKey(
    _env: JNIEnv,
    _class: JClass,
    keysym: jint,
    mask: jint,
) -> jboolean {
    jni_catch(
        || {
            // 打字统计埋点：每次按键计一次（进程内缓存 + 定期落盘，开销可忽略）
            shurufa_options::stats::note_keys(1);
            let session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match session.as_ref() {
                Some(s) => s.process_key(keysym, mask) as jboolean,
                None => 0,
            }
        },
        0,
    )
}

/// 取上屏文本；无则返回空串。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeCommit(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let default = to_jstring(&env, "");
    jni_catch(
        || {
            let session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let text = session
                .as_ref()
                .and_then(|s| s.commit())
                .unwrap_or_default();
            if !text.is_empty() {
                shurufa_options::stats::note_chars(text.chars().count());
            }
            to_jstring(&env, &text)
        },
        default,
    )
}

/// 取输入上下文：`preedit \u{1} highlighted \u{1} cursor \u{1} 候选…`；空组合返回空串。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeContext(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let default = to_jstring(&env, "");
    jni_catch(
        || {
            let session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(s) = session.as_ref() else {
                return to_jstring(&env, "");
            };
            to_jstring(&env, &session_context_string(s))
        },
        default,
    )
}

/// 将 Rime 的组合光标移动到指定的 UTF-16 偏移。
///
/// librime 保存的是 raw input 字节偏移；先把编辑器的 UTF-16 目标
/// 换算为 UTF-8 字节偏移，再直接调 set_caret_pos（一步直达，不逐键）。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeSetCursor(
    _env: JNIEnv,
    _class: JClass,
    cursor_pos: jint,
) {
    jni_catch(
        || {
            let session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(session) = session.as_ref() else {
                return;
            };
            let context = session.context();
            let target = (cursor_pos.max(0) as usize).min(context.preedit.encode_utf16().count());
            if context.preedit.is_empty() || context.cursor_pos == target {
                return;
            }
            // 从 UTF-16 位置反推 UTF-8 字节偏移；ASCII 拼音下两者相等。
            let mut utf16_seen = 0usize;
            let mut byte_pos = context.preedit.len();
            for (i, ch) in context.preedit.char_indices() {
                if utf16_seen >= target {
                    byte_pos = i;
                    break;
                }
                utf16_seen += ch.len_utf16();
            }
            session.set_caret_pos(byte_pos);
        },
        (),
    )
}

/// 清空当前组合（切换输入框、收起键盘时调用）。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeReset(_env: JNIEnv, _class: JClass) {
    jni_catch(
        || {
            let session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(s) = session.as_ref() {
                s.simulate("{Escape}");
            }
        },
        (),
    )
}

/// 切换中英文（ascii_mode）；返回切换后是否为英文直输。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeToggleAscii(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    jni_catch(
        || {
            let session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match session.as_ref() {
                Some(s) => s.toggle_ascii() as jboolean,
                None => 0,
            }
        },
        0,
    )
}

/// 查询当前是否处于英文直输模式。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeIsAscii(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    jni_catch(
        || {
            let session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match session.as_ref() {
                Some(s) => s.get_option("ascii_mode") as jboolean,
                None => 0,
            }
        },
        0,
    )
}

/// 取引擎状态串：`is_ascii \u{1} full_shape \u{1} ascii_punct`，各为 "0"/"1"；无会话返回空串。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeStatus(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let default = to_jstring(&env, "");
    jni_catch(
        || {
            let session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(s) = session.as_ref() else {
                return to_jstring(&env, "");
            };
            let (is_ascii, is_full_shape, is_ascii_punct) = s.status_bits();
            let text = format!(
                "{}\u{1}{}\u{1}{}",
                is_ascii as u8, is_full_shape as u8, is_ascii_punct as u8
            );
            to_jstring(&env, &text)
        },
        default,
    )
}

/// 删除当前页第 index 个候选（"忘记该词"）；成功返回 true。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeForgetOnCurrentPage(
    _env: JNIEnv,
    _class: JClass,
    index: jint,
) -> jboolean {
    jni_catch(
        || {
            let session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match session.as_ref() {
                Some(s) => s.forget_on_current_page(index.max(0) as usize) as jboolean,
                None => 0,
            }
        },
        0,
    )
}

/// 取打字统计合计：`totalChars \u{1} todayChars \u{1} totalKeys \u{1} todayKeys`；无数据全 0。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeStatsTotals(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let default = to_jstring(&env, "");
    jni_catch(
        || {
            let t = shurufa_options::stats::totals();
            let text = format!(
                "{}\u{1}{}\u{1}{}\u{1}{}",
                t.total_chars, t.today_chars, t.total_keys, t.today_keys
            );
            to_jstring(&env, &text)
        },
        default,
    )
}
/// 选择当前页第 `index` 个候选并上屏。返回提交文本（空串=失败/无上屏）。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeSelectCandidate(
    env: JNIEnv,
    _class: JClass,
    index: jint,
) -> jstring {
    let default = to_jstring(&env, "");
    jni_catch(
        || {
            let session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(s) = session.as_ref() else {
                return to_jstring(&env, "");
            };
            // 选择当前页候选：librime 会把上屏文本压入提交队列，
            // 随后 commit() 取出；失败（索引越界等）返回空串。
            if !s.select_candidate_on_current_page(index.max(0) as usize) {
                return to_jstring(&env, "");
            }
            let text = s.commit().unwrap_or_default();
            if !text.is_empty() {
                shurufa_options::stats::note_chars(text.chars().count());
            }
            to_jstring(&env, &text)
        },
        default,
    )
}

/// 候选列表翻页；backward=true 为上一页。返回 `<上下文串>`（同 nativeContext 协议）。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeChangePage(
    env: JNIEnv,
    _class: JClass,
    backward: jboolean,
) -> jstring {
    let default = to_jstring(&env, "");
    jni_catch(
        || {
            let session = SESSION
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(s) = session.as_ref() else {
                return to_jstring(&env, "");
            };
            s.change_page(backward != 0);
            to_jstring(&env, &session_context_string(s))
        },
        default,
    )
}

