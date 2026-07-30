//! Android JNI 桥：把 ime-bridge 的引擎会话暴露给 Kotlin 键盘服务。
//!
//! 生命周期：`nativeInit` 在后台线程完成引擎初始化与部署（首次约
//! 数秒到数十秒），成功后建立进程级单一会话；按键与查询都走该会话。
//! 上下文以 `\u{1}` 分隔的扁平字符串返回，避免引入 JSON 依赖：
//! `preedit \u{1} highlighted \u{1} 候选1 \u{1} 候选2 …`

use std::sync::{Mutex, OnceLock};

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jint, jstring};
use jni::JNIEnv;

use ime_bridge::{Engine, Session};

mod clip_jni;
mod sync_jni;

static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();
static SESSION: Mutex<Option<Session<'static>>> = Mutex::new(None);

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
    let shared = jstring_to_string(&mut env, &shared_dir);
    let user = jstring_to_string(&mut env, &user_dir);
    let engine = ENGINE.get_or_init(|| {
        Engine::init(
            std::path::Path::new(&shared),
            std::path::Path::new(&user),
        )
    });
    let Ok(engine) = engine.as_ref() else {
        return 0;
    };
    let mut session = SESSION.lock().expect("会话锁不可恢复");
    if session.is_none() {
        match engine.create_session() {
            Ok(s) => *session = Some(s),
            Err(_) => return 0,
        }
    }
    1
}

/// 喂键（X11 keysym 与修饰掩码，与桌面端一致）；返回是否被引擎吃掉。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeProcessKey(
    _env: JNIEnv,
    _class: JClass,
    keysym: jint,
    mask: jint,
) -> jboolean {
    let session = SESSION.lock().expect("会话锁不可恢复");
    match session.as_ref() {
        Some(s) => s.process_key(keysym, mask) as jboolean,
        None => 0,
    }
}

/// 取上屏文本；无则返回空串。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeCommit(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let session = SESSION.lock().expect("会话锁不可恢复");
    let text = session
        .as_ref()
        .and_then(|s| s.commit())
        .unwrap_or_default();
    to_jstring(&env, &text)
}

/// 取输入上下文：`preedit \u{1} highlighted \u{1} 候选…`；空组合返回空串。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeContext(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let session = SESSION.lock().expect("会话锁不可恢复");
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
    for c in &ctx.candidates {
        out.push('\u{1}');
        out.push_str(&c.text);
    }
    to_jstring(&env, &out)
}

/// 清空当前组合（切换输入框、收起键盘时调用）。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeReset(
    _env: JNIEnv,
    _class: JClass,
) {
    let session = SESSION.lock().expect("会话锁不可恢复");
    if let Some(s) = session.as_ref() {
        s.simulate("{Escape}");
    }
}

/// 切换中英文（ascii_mode）；返回切换后是否为英文直输。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeToggleAscii(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let session = SESSION.lock().expect("会话锁不可恢复");
    match session.as_ref() {
        Some(s) => s.toggle_ascii() as jboolean,
        None => 0,
    }
}

/// 查询当前是否处于英文直输模式。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeIsAscii(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let session = SESSION.lock().expect("会话锁不可恢复");
    match session.as_ref() {
        Some(s) => s.get_option("ascii_mode") as jboolean,
        None => 0,
    }
}
