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
mod sync_jni;

static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();
static SESSION: Mutex<Option<Session<'static>>> = Mutex::new(None);

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
            let engine = ENGINE.get_or_init(|| {
                Engine::init(std::path::Path::new(&shared), std::path::Path::new(&user))
            });
            let Ok(engine) = engine.as_ref() else {
                return 0;
            };
            // 锁被 panic 污染时经 into_inner 直接恢复其内部值继续使用
            let mut session = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if session.is_none() {
                match engine.create_session() {
                    Ok(s) => *session = Some(s),
                    Err(_) => return 0,
                }
            }
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
            let session = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
            let session = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let text = session
                .as_ref()
                .and_then(|s| s.commit())
                .unwrap_or_default();
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
            let session = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(s) = session.as_ref() else {
                return to_jstring(&env, "");
            };
            let ctx = s.context();
            if ctx.preedit.is_empty() {
                return to_jstring(&env, "");
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
            to_jstring(&env, &out)
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
            let session = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeReset(
    _env: JNIEnv,
    _class: JClass,
) {
    jni_catch(
        || {
            let session = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
            let session = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
            let session = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match session.as_ref() {
                Some(s) => s.get_option("ascii_mode") as jboolean,
                None => 0,
            }
        },
        0,
    )
}
