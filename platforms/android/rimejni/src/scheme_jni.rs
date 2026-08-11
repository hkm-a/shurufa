//! Android JNI 桥：把 shurufa-options 的 input_scheme 暴露给 Kotlin 键盘。
//!
//! wave 4 此处仅完成"持久化 SharedPreferences + 进程内 Mutex<String> 缓存"两件事；
//! 真正触发 librime 的 schema redeploy（替换 ply_dir / 重新部署 / 重建 session）
//! 留给 wave 5，由专门的 schema 管理器在 IME 侧接管。
//!
//! 所有 `#[no_mangle]` 入口经 [`crate::jni_catch`] 包裹，panic 拦截在 FFI 边界内。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, jstring};
use jni::JNIEnv;

/// 当前选中的输入方案（进程内缓存；持久化底稿在 options.json 的 input_scheme）。
/// SharedPreferences 只是"observers 更快读"的冗余副本，不要求三端同步。
static CURRENT_SCHEME: Mutex<Option<String>> = Mutex::new(None);
/// 单调递增写入计数（wave 5 redeploy 触发器以 0 → 非 0 感知一次新写入）。
static SCHEME_WRITE_GEN: AtomicUsize = AtomicUsize::new(0);

const SCHEME_PREF_KEY: &str = "shurufa_input_scheme";

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

/// 通过 JNI Context.getSharedPreferences 写入 `shurufa_input_scheme`；返回是否成功。
/// 采用 context 参数显式传入而非 JavaVM 全局 static —— 生命周期简单、测试可注入。
fn persist_to_shared_prefs(env: &mut JNIEnv, ctx: &JObject, scheme: &str) -> bool {
    // getSharedPreferences("shurufa", Context.MODE_PRIVATE) → SharedPreferences
    let name = match env.new_string("shurufa") {
        Ok(s) => s,
        Err(_) => return false,
    };
    let prefs = env
        .call_method(
            ctx,
            "getSharedPreferences",
            "(Ljava/lang/String;I)Landroid/content/SharedPreferences;",
            &[
                jni::objects::JValue::Object(&name.into()),
                jni::objects::JValue::Int(0),
            ],
        )
        .and_then(|v| v.l());
    let Ok(prefs) = prefs else {
        return false;
    };
    // prefs.edit() → Editor
    let editor = env
        .call_method(
            &prefs,
            "edit",
            "()Landroid/content/SharedPreferences$Editor;",
            &[],
        )
        .and_then(|v| v.l());
    let Ok(editor) = editor else {
        return false;
    };
    // editor.putString(key, value)
    let (Ok(key), Ok(val)) = (env.new_string(SCHEME_PREF_KEY), env.new_string(scheme)) else {
        return false;
    };
    if env
        .call_method(
            &editor,
            "putString",
            "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;",
            &[
                jni::objects::JValue::Object(&key.into()),
                jni::objects::JValue::Object(&val.into()),
            ],
        )
        .is_err()
    {
        return false;
    }
    // editor.apply()
    env.call_method(&editor, "apply", "()V", &[]).is_ok()
}

/// 读取当前方案（进程内缓存，缓存未命中时从 options.json 读并填充）。
/// Kotlin 端 MainActivity / 方案 chip dialog 在 onResume 时调它，避免直接碰 SharedPreferences。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeGetInputScheme(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let default = to_jstring(&env, "pinyin");
    crate::jni_catch(
        || {
            let cached = CURRENT_SCHEME
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let current = match cached {
                Some(s) => s,
                None => {
                    let disk = shurufa_options::load().input_scheme;
                    *CURRENT_SCHEME.lock().unwrap_or_else(|p| p.into_inner()) = Some(disk.clone());
                    disk
                }
            };
            to_jstring(&env, &current)
        },
        default,
    )
}

/// 列出 4 个可选方案 id（逗号分隔）；与 Kotlin 端轮播枚举及 options::validate 一一对应。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeListInputSchemes(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let default = to_jstring(&env, "pinyin");
    crate::jni_catch(|| to_jstring(&env, "pinyin,double_pinyin,wubi,cangjie"), default)
}

/// 选方案：scheme ∈ {"pinyin","double_pinyin","wubi","cangjie"}；
/// 持久化 options.json + SharedPreferences + 进程内缓存，全部就绪返回 true。
/// 不合法 id 或 SharedPreferences 写入失败返回 false（options.json 已写时仍返回 false ——
/// 因为 SharedPreferences 是 UI 长驻读的副本，失败即整体失败，wave 5 可再演化）。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_RimeBridge_nativeSetInputScheme(
    mut env: JNIEnv,
    _class: JClass,
    ctx: JObject,
    scheme: JString,
) -> jboolean {
    crate::jni_catch(
        || {
            let scheme_str = jstr(&mut env, &scheme);
            if !shurufa_options::validate_input_scheme(&scheme_str) {
                return 0;
            }
            // 1) 持久化到 options.json（唯一事实源；modify 内部带跨进程文件锁）
            if shurufa_options::modify(|current| shurufa_options::ImeOptions {
                input_scheme: scheme_str.clone(),
                ..current.clone()
            })
            .is_err()
            {
                return 0;
            }
            // 2) 写 SharedPreferences（供 wave 5 其它 UI 快速读）
            if !persist_to_shared_prefs(&mut env, &ctx, &scheme_str) {
                return 0;
            }
            // 3) 更新进程内缓存 + 写计数
            *CURRENT_SCHEME.lock().unwrap_or_else(|p| p.into_inner()) = Some(scheme_str);
            SCHEME_WRITE_GEN.fetch_add(1, Ordering::Relaxed);
            1
        },
        0,
    )
}
