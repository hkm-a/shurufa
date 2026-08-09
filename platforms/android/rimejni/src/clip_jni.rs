//! 手机端剪贴板历史 JNI：复用桌面 clipboard-store（SQLite）。
//!
//! 手机的历史含两类来源：本机（键盘弹出时读到的系统剪贴板）与
//! 同步（电脑推送来的）。列表返回以 `\u{2}` 分记录、`\u{1}` 分字段
//! 的扁平串（id/来源/文本），避免引入 JSON。
//!
//! 所有 `#[no_mangle]` 入口经 [`crate::jni_catch`] 包裹，panic 不会
//! 跨 FFI 传播；历史库锁被污染时以 `.ok()` 安全降级而不 abort。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jbyteArray, jint, jstring};
use jni::JNIEnv;

use clipboard_store::{ClipKind, ClipboardStore};
use sha2::{Digest, Sha256};

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
    crate::jni_catch(
        || {
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
        },
        0,
    )
}

/// 插入一条文本历史（去重由存储层负责）；source 为来源标记。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeInsert(
    mut env: JNIEnv,
    _class: JClass,
    text: JString,
    source: JString,
) {
    crate::jni_catch(
        || {
            if let Some(store) = STORE.get() {
                let t = jstr(&mut env, &text);
                let src = jstr(&mut env, &source);
                if !t.is_empty() {
                    if let Ok(guard) = store.lock() {
                        let _ = guard.insert_text(&t, &src);
                    }
                }
            }
        },
        (),
    )
}

/// 最近 limit 条历史：记录以 `\u{2}` 分隔，字段 `id\u{1}类型\u{1}来源\u{1}文本`。
/// 类型为 text/image/files；图片文本为空，缩略图另经 nativeImageData 取。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeList(
    env: JNIEnv,
    _class: JClass,
    limit: jint,
) -> jstring {
    let default = to_jstring(&env, "");
    crate::jni_catch(
        || {
            let Some(store) = STORE.get() else {
                return to_jstring(&env, "");
            };
            let entries = match store.lock() {
                Ok(guard) => guard.list(limit.max(0) as u32, 0).unwrap_or_default(),
                Err(_) => Vec::new(),
            };
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
        },
        default,
    )
}

/// 图片条目的 PNG 字节；非图片或不存在返回空数组。供缩略图与写回剪贴板。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeImageData(
    env: JNIEnv,
    _class: JClass,
    id: jint,
) -> jbyteArray {
    let default = std::ptr::null_mut();
    crate::jni_catch(
        || {
            let data = STORE
                .get()
                .and_then(|s| s.lock().ok()?.image_data(id as i64).ok().flatten())
                .unwrap_or_default();
            env.byte_array_from_slice(&data)
                .map(|a| a.into_raw())
                .unwrap_or(std::ptr::null_mut())
        },
        default,
    )
}

/// 插入本机图片（PNG 字节）到历史；供键盘读到系统剪贴板图片时调用。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeInsertImage(
    mut env: JNIEnv,
    _class: JClass,
    data: JByteArray,
    source: JString,
) {
    crate::jni_catch(
        || {
            if let Some(store) = STORE.get() {
                if let Ok(bytes) = env.convert_byte_array(&data) {
                    if !bytes.is_empty() {
                        let src = jstr(&mut env, &source);
                        if let Ok(guard) = store.lock() {
                            let _ = guard.insert_image(&bytes, &src);
                        }
                    }
                }
            }
        },
        (),
    )
}

/// 插入文件路径历史；Kotlin 侧以换行分隔路径，与存储层格式一致。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeInsertFiles(
    mut env: JNIEnv,
    _class: JClass,
    paths: JString,
    source: JString,
) {
    crate::jni_catch(
        || {
            if let Some(store) = STORE.get() {
                let values = jstr(&mut env, &paths)
                    .lines()
                    .filter(|path| !path.trim().is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                if !values.is_empty() {
                    let src = jstr(&mut env, &source);
                    if let Ok(guard) = store.lock() {
                        let _ = guard.insert_files(&values, &src);
                    }
                }
            }
        },
        (),
    )
}

/// 供同步桥调用：把收到的图片（PNG 字节）存入历史，返回条目 id。
pub(crate) fn store_image(png: &[u8], source: &str) -> Option<i64> {
    let store = STORE.get()?;
    store.lock().ok()?.insert_image(png, source).ok().flatten()
}

/// 供同步桥调用：把收到的本地文件路径存入历史，返回条目 id。
pub(crate) fn store_files(paths: &[String], source: &str) -> Option<i64> {
    let store = STORE.get()?;
    store
        .lock()
        .ok()?
        .insert_files(paths, source)
        .ok()
        .flatten()
}

/// 删除单条历史。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeDelete(
    _env: JNIEnv,
    _class: JClass,
    id: jint,
) {
    crate::jni_catch(
        || {
            if let Some(store) = STORE.get() {
                if let Ok(guard) = store.lock() {
                    let _ = guard.delete(id as i64);
                }
            }
        },
        (),
    )
}

/// 置顶/取消置顶单条历史；返回是否更新成功（条目存在）。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeSetPinned(
    _env: JNIEnv,
    _class: JClass,
    id: jint,
    pinned: jboolean,
) -> jboolean {
    crate::jni_catch(
        || {
            let Some(store) = STORE.get() else {
                return 0;
            };
            match store.lock() {
                Ok(guard) => guard
                    .set_pinned(id as i64, pinned != 0)
                    .unwrap_or(false) as jboolean,
                Err(_) => 0,
            }
        },
        0,
    )
}

/// 搜索历史：记录协议与 nativeList 一致（`id\u{1}类型\u{1}来源\u{1}文本`，记录间 `\u{2}`）。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeSearch(
    mut env: JNIEnv,
    _class: JClass,
    query: JString,
    limit: jint,
) -> jstring {
    let default = to_jstring(&env, "");
    crate::jni_catch(
        || {
            let Some(store) = STORE.get() else {
                return to_jstring(&env, "");
            };
            let q = jstr(&mut env, &query);
            if q.is_empty() {
                return to_jstring(&env, "");
            }
            let entries = match store.lock() {
                Ok(guard) => guard.search(&q, limit.max(0) as u32).unwrap_or_default(),
                Err(_) => Vec::new(),
            };
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
        },
        default,
    )
}

/// 最新文本类条目的变更指纹：`updatedMs \u{1} sha256(文本)前16hex`；
/// 无文本条目或空库返回空串。供 Kotlin 侧感知剪贴板历史变化去重。
#[no_mangle]
pub extern "system" fn Java_com_shurufa_ime_ClipStore_nativeLatestSignature(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let default = to_jstring(&env, "");
    crate::jni_catch(
        || {
            let Some(store) = STORE.get() else {
                return to_jstring(&env, "");
            };
            let entries = match store.lock() {
                Ok(guard) => guard.list(1, 0).unwrap_or_default(),
                Err(_) => Vec::new(),
            };
            let Some(latest) = entries.first() else {
                return to_jstring(&env, "");
            };
            if latest.kind != ClipKind::Text {
                return to_jstring(&env, "");
            }
            let digest = Sha256::digest(latest.text.as_bytes());
            let hash16: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
            to_jstring(&env, &format!("{}\u{1}{}", latest.updated_at, hash16))
        },
        default,
    )
}
