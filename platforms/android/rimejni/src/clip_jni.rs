//! 手机端剪贴板历史 JNI：复用桌面 clipboard-store（SQLite）。
//!
//! 手机的历史含两类来源：本机（键盘弹出时读到的系统剪贴板）与
//! 同步（电脑推送来的）。列表返回以 `\u{2}` 分记录、`\u{1}` 分字段
//! 的扁平串（id/来源/文本），避免引入 JSON。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jbyteArray, jint, jstring};
use jni::JNIEnv;

use clipboard_store::{ClipKind, ClipboardStore};

static STORE: OnceLock<Mutex<ClipboardStore>> = OnceLock::new();

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

/// 打开历史库（dbPath 为 filesDir/clipboard.db）；幂等。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeInit(
    mut env: JNIEnv,
    _class: JClass,
    db_path: JString,
) -> jboolean {
    if STORE.get().is_some() {
        return 1;
    }
    let path = PathBuf::from(jstr(&mut env, &db_path));
    match ClipboardStore::open(&path) {
        Ok(store) => {
            let _ = STORE.set(Mutex::new(store));
            1
        }
        Err(_) => 0,
    }
}

/// 插入一条文本历史（去重由存储层负责）；source 为来源标记。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeInsert(
    mut env: JNIEnv,
    _class: JClass,
    text: JString,
    source: JString,
) {
    if let Some(store) = STORE.get() {
        let t = jstr(&mut env, &text);
        let src = jstr(&mut env, &source);
        if !t.is_empty() {
            let _ = store.lock().expect("历史库锁不可恢复").insert_text(&t, &src);
        }
    }
}

/// 最近 limit 条历史：记录以 `\u{2}` 分隔，字段 `id\u{1}类型\u{1}来源\u{1}文本`。
/// 类型为 text/image/files；图片文本为空，缩略图另经 nativeImageData 取。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeList(
    env: JNIEnv,
    _class: JClass,
    limit: jint,
) -> jstring {
    let Some(store) = STORE.get() else {
        return to_jstring(&env, "");
    };
    let entries = store
        .lock()
        .expect("历史库锁不可恢复")
        .list(limit.max(0) as u32, 0)
        .unwrap_or_default();
    let text = entries
        .iter()
        .map(|e| {
            let kind = match e.kind {
                ClipKind::Text => "text",
                ClipKind::Image => "image",
                ClipKind::Files => "files",
            };
            format!("{}\u{1}{}\u{1}{}\u{1}{}", e.id, kind, e.source_app, e.text)
        })
        .collect::<Vec<_>>()
        .join("\u{2}");
    to_jstring(&env, &text)
}

/// 图片条目的 PNG 字节；非图片或不存在返回空数组。供缩略图与写回剪贴板。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeImageData(
    env: JNIEnv,
    _class: JClass,
    id: jint,
) -> jbyteArray {
    let data = STORE
        .get()
        .and_then(|s| s.lock().ok()?.image_data(id as i64).ok().flatten())
        .unwrap_or_default();
    env.byte_array_from_slice(&data)
        .map(|a| a.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// 供同步桥调用：把收到的图片（PNG 字节）存入历史，返回条目 id。
pub(crate) fn store_image(png: &[u8], source: &str) -> Option<i64> {
    let store = STORE.get()?;
    store.lock().ok()?.insert_image(png, source).ok().flatten()
}

/// 删除单条历史。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeDelete(
    _env: JNIEnv,
    _class: JClass,
    id: jint,
) {
    if let Some(store) = STORE.get() {
        let _ = store.lock().expect("历史库锁不可恢复").delete(id as i64);
    }
}
