//! 配置/短语/皮肤的冲突合并与增量同步状态。
//!
//! 同步协议仍走 `ConfigFile` 全量文本（配置文件小，线格式不需要碎片化）；
//! 这里的“增量”指**发送侧只广播相对上次同步发生过变化的配置类型**，
//! “冲突合并”指接收侧用本地状态做三方比较：
//! - 本地未变、远端变了 → 直接采用远端；
//! - 本地变了、远端未变 → 保留本地；
//! - 两端都变 / 首次收到且内容不同 → 自动合并（custom_phrase 按码合并，
//!   options/skin 按 JSON 深度合并），合并前先把本地旧文件备份到
//!   `sync-config-backups/`。
//!
//! 状态文件为配置根目录下的 `.sync-config-state.json`，记录每种配置的
//! `local_sha256` / `remote_sha256` / `base_sha256`，用于三方 diff。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CONFIG_STATE_FILE: &str = ".sync-config-state.json";
pub const CONFIG_BACKUP_DIR: &str = "sync-config-backups";
pub const CONFIG_KINDS: [&str; 3] = ["options", "skin", "custom_phrase"];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigSyncState {
    #[serde(default)]
    pub files: HashMap<String, FileSyncState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSyncState {
    pub local_sha256: String,
    pub remote_sha256: String,
    pub base_sha256: String,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyStatus {
    /// 内容完全一致，未做任何写入。
    Noop,
    /// 直接采用远端（本地未变化）。
    AppliedRemote,
    /// 两端均有修改，自动合并后写入（本地旧文件已备份）。
    Merged,
    /// 仅本地修改，保留本地（远端仍是旧版）。
    KeptLocal,
}

#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    pub status: ApplyStatus,
    pub backup: Option<PathBuf>,
}

/// 一次已自动合并但需要用户确认的配置冲突。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConflictRecord {
    pub ts_ms: u64,
    pub kind: String,
    pub name: String,
    /// `sync-config-backups/` 下的本地旧文件（合并前）。
    pub local_backup: String,
    /// `sync-config-backups/` 下的远端原文件（合并前）。
    pub remote_backup: String,
    /// 自动合并结果的 SHA-256。
    pub merged_sha256: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConflictLog {
    #[serde(default)]
    pub conflicts: Vec<ConflictRecord>,
}

pub const CONFLICT_RECORD_FILE: &str = ".sync-config-conflicts.json";

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn state_path(root: &Path) -> PathBuf {
    root.join(CONFIG_STATE_FILE)
}

pub fn backup_dir(root: &Path) -> PathBuf {
    root.join(CONFIG_BACKUP_DIR)
}

pub fn config_path(root: &Path, kind: &str) -> Option<PathBuf> {
    match kind {
        "options" => Some(root.join("options.json")),
        "skin" => Some(root.join("shurufa-skin.json")),
        "custom_phrase" => Some(root.join("rime").join("custom_phrase.txt")),
        _ => None,
    }
}

/// 从备份文件名 `<ts>_<kind>_<safe>` 解析配置类型。
/// `custom_phrase` 本身含下划线，不能用简单的 split_once 取第二段。
pub fn kind_from_backup_name(file: &str) -> Option<&str> {
    let rest = file.split_once('_')?.1;
    CONFIG_KINDS
        .iter()
        .find(|kind| {
            rest == **kind
                || rest
                    .strip_prefix(**kind)
                    .is_some_and(|s| s.starts_with('_'))
        })
        .copied()
}

pub fn load_state(root: &Path) -> ConfigSyncState {
    std::fs::read_to_string(state_path(root))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_state(root: &Path, state: &ConfigSyncState) -> Result<(), String> {
    let path = state_path(root);
    let parent = path.parent().ok_or("状态文件无父目录")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("创建配置状态目录失败：{e}"))?;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| format!("序列化同步状态失败：{e}"))?;
    std::fs::write(&tmp, bytes).map_err(|e| format!("写入同步状态失败：{e}"))?;
    // Windows 上 rename 到已存在目标会失败，先移除旧文件再替换。
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("移除旧同步状态失败：{e}"))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("替换同步状态失败：{e}"))
}

/// 备份当前本地文件到 `sync-config-backups/`。内容相同时不备份。
pub fn backup_local(
    root: &Path,
    kind: &str,
    remote_name: &str,
    local_bytes: &[u8],
    remote_bytes: &[u8],
) -> Option<PathBuf> {
    if local_bytes == remote_bytes {
        return None;
    }
    let dir = backup_dir(root);
    std::fs::create_dir_all(&dir).ok()?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let safe = remote_name.replace(['/', '\\'], "_");
    let backup_path = dir.join(format!("{ts}_{kind}_{safe}"));
    std::fs::copy(config_path(root, kind)?, &backup_path)
        .ok()
        .map(|_| backup_path)
}

pub fn conflicts_path(root: &Path) -> PathBuf {
    root.join(CONFLICT_RECORD_FILE)
}

pub fn load_conflicts(root: &Path) -> ConflictLog {
    std::fs::read_to_string(conflicts_path(root))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_conflicts(root: &Path, log: &ConflictLog) -> Result<(), String> {
    let path = conflicts_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建冲突记录目录失败：{e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(log).map_err(|e| format!("序列化冲突记录失败：{e}"))?;
    std::fs::write(&tmp, bytes).map_err(|e| format!("写入冲突记录失败：{e}"))?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("移除旧冲突记录失败：{e}"))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("替换冲突记录失败：{e}"))
}

/// 把一次合并冲突追加到记录；最多保留 20 条，旧记录自动淘汰。
pub fn record_conflict(root: &Path, record: ConflictRecord) {
    let mut log = load_conflicts(root);
    log.conflicts
        .retain(|r| r.remote_backup != record.remote_backup);
    log.conflicts.push(record);
    if log.conflicts.len() > 20 {
        let remove_count = log.conflicts.len() - 20;
        log.conflicts.drain(0..remove_count);
    }
    let _ = save_conflicts(root, &log);
}

/// 按远端备份文件名移除一条已处理的冲突记录。
pub fn remove_conflict(root: &Path, remote_backup: &str) -> Result<bool, String> {
    let mut log = load_conflicts(root);
    let before = log.conflicts.len();
    log.conflicts.retain(|r| r.remote_backup != remote_backup);
    if log.conflicts.len() == before {
        return Ok(false);
    }
    save_conflicts(root, &log)?;
    Ok(true)
}

fn write_file(path: &Path, data: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败：{e}"))?;
    }
    std::fs::write(path, data.as_bytes()).map_err(|e| format!("写入配置失败：{e}"))
}

/// 处理一份远端配置：三方比较 + 自动合并，并更新 `.sync-config-state.json`。
pub fn apply_incoming(
    root: &Path,
    kind: &str,
    name: &str,
    data: &str,
) -> Result<ApplyOutcome, String> {
    let path = config_path(root, kind).ok_or_else(|| format!("未知配置类型：{kind}"))?;
    let local_bytes = std::fs::read(&path).ok();
    let remote_bytes = data.as_bytes();
    let incoming_hash = sha256_hex(remote_bytes);

    let mut state = load_state(root);
    let file_state = state
        .files
        .get(kind)
        .cloned()
        .unwrap_or_else(|| FileSyncState {
            // 无状态时不给 local_sha256 赋当前值：否则首次收到远端会被误判为
            // “本地未变、远端变化”而直接覆盖本地定制。
            local_sha256: String::new(),
            remote_sha256: String::new(),
            base_sha256: String::new(),
            updated_at_ms: now_ms(),
        });

    // 本地不存在：直接落盘。
    let Some(local) = local_bytes else {
        write_file(&path, data)?;
        let hash = sha256_hex(remote_bytes);
        state.files.insert(
            kind.to_string(),
            FileSyncState {
                local_sha256: hash.clone(),
                remote_sha256: hash.clone(),
                base_sha256: hash,
                updated_at_ms: now_ms(),
            },
        );
        save_state(root, &state)?;
        return Ok(ApplyOutcome {
            status: ApplyStatus::AppliedRemote,
            backup: None,
        });
    };

    let local_hash = sha256_hex(&local);
    let local_text = String::from_utf8_lossy(&local);

    // 内容完全一致：无需写入，但补一条同步状态，避免后续被误判为冲突。
    if local == remote_bytes {
        if state
            .files
            .get(kind)
            .map(|e| e.local_sha256 != local_hash)
            .unwrap_or(true)
        {
            let hash = incoming_hash.clone();
            state.files.insert(
                kind.to_string(),
                FileSyncState {
                    local_sha256: hash.clone(),
                    remote_sha256: hash.clone(),
                    base_sha256: hash,
                    updated_at_ms: now_ms(),
                },
            );
            save_state(root, &state)?;
        }
        return Ok(ApplyOutcome {
            status: ApplyStatus::Noop,
            backup: None,
        });
    }

    let local_unchanged =
        !file_state.local_sha256.is_empty() && file_state.local_sha256 == local_hash;
    let remote_unchanged =
        !file_state.remote_sha256.is_empty() && file_state.remote_sha256 == incoming_hash;

    // 远端仍是上次同步的版本：无论本地是否已记录变化都保留本地。
    // （本地刚改过但尚未同步出去时，重复收到旧远端不应再次触发合并。）
    if remote_unchanged {
        let mut next = file_state;
        next.local_sha256 = local_hash.clone();
        next.updated_at_ms = now_ms();
        state.files.insert(kind.to_string(), next);
        save_state(root, &state)?;
        return Ok(ApplyOutcome {
            status: ApplyStatus::KeptLocal,
            backup: None,
        });
    }

    // 本地未变、远端变化：快进到远端。
    if local_unchanged {
        write_file(&path, data)?;
        let hash = incoming_hash.clone();
        state.files.insert(
            kind.to_string(),
            FileSyncState {
                local_sha256: hash.clone(),
                remote_sha256: hash.clone(),
                base_sha256: hash,
                updated_at_ms: now_ms(),
            },
        );
        save_state(root, &state)?;
        return Ok(ApplyOutcome {
            status: ApplyStatus::AppliedRemote,
            backup: None,
        });
    }

    // 两端都变（或没有状态可参考但本地内容不同）：先备份，再自动合并。
    let backup = backup_local(root, kind, name, &local, remote_bytes);
    // 同时保存一份“远端原文件”，供用户在冲突 UI 里选择“采用远端”。
    let safe = name.replace(['/', '\\'], "_");
    let ts = now_ms();
    let remote_backup_name = format!("{ts}_{kind}_remote_{safe}");
    let remote_backup_path = backup_dir(root).join(&remote_backup_name);
    let _ = std::fs::create_dir_all(backup_dir(root));
    let _ = std::fs::write(&remote_backup_path, data);

    // JSON 无法自动合并（如本地正处于半编辑状态）时退化为远端优先，旧文件仍已备份。
    let merged = merge(kind, &local_text, data).unwrap_or_else(|_| data.to_string());
    write_file(&path, &merged)?;
    let merged_hash = sha256_hex(merged.as_bytes());
    if let Some(local_backup) = &backup {
        if let Some(local_backup_name) = local_backup.file_name().and_then(|n| n.to_str()) {
            record_conflict(
                root,
                ConflictRecord {
                    ts_ms: ts,
                    kind: kind.to_string(),
                    name: name.to_string(),
                    local_backup: local_backup_name.to_string(),
                    remote_backup: remote_backup_name,
                    merged_sha256: merged_hash.clone(),
                },
            );
        }
    }
    let mut next = file_state;
    next.local_sha256 = merged_hash;
    next.remote_sha256 = incoming_hash;
    next.updated_at_ms = now_ms();
    state.files.insert(kind.to_string(), next);
    save_state(root, &state)?;
    Ok(ApplyOutcome {
        status: ApplyStatus::Merged,
        backup,
    })
}

/// 发送前检查：仅当文件相对上次同步发生变化时才返回内容。
/// 返回 `None` 表示无需发送（增量跳过）。
pub fn prepare_send(root: &Path, kind: &str, path: &Path) -> Result<Option<String>, String> {
    if config_path(root, kind).is_none() {
        return Err(format!("未知配置类型：{kind}"));
    }
    let data = std::fs::read_to_string(path).map_err(|e| format!("读取配置失败：{e}"))?;
    let hash = sha256_hex(data.as_bytes());
    let state = load_state(root);
    if let Some(entry) = state.files.get(kind) {
        if entry.local_sha256 == hash && entry.remote_sha256 == hash {
            return Ok(None);
        }
    }
    Ok(Some(data))
}

/// 发送成功后记录：本机内容与远端已同步为同一版本。
pub fn mark_sent(root: &Path, kind: &str, data: &str) -> Result<(), String> {
    let mut state = load_state(root);
    let hash = sha256_hex(data.as_bytes());
    state.files.insert(
        kind.to_string(),
        FileSyncState {
            local_sha256: hash.clone(),
            remote_sha256: hash.clone(),
            base_sha256: hash,
            updated_at_ms: now_ms(),
        },
    );
    save_state(root, &state)
}

/// 自动合并策略：
/// - custom_phrase：保留本地表头/注释与顺序，按“码”合并；同码冲突远端优先。
/// - options/skin：JSON 深度合并，对象递归、标量与数组远端优先。
pub fn merge(kind: &str, local: &str, remote: &str) -> Result<String, String> {
    match kind {
        "custom_phrase" => Ok(merge_custom_phrase(local, remote)),
        "options" | "skin" => merge_json(local, remote),
        _ => Err(format!("未知配置类型：{kind}")),
    }
}

fn merge_custom_phrase(local: &str, remote: &str) -> String {
    fn key_of(line: &str) -> String {
        let mut parts = line.splitn(3, '\t');
        let _phrase = parts.next().unwrap_or("");
        let code = parts.next().unwrap_or("");
        if code.trim().is_empty() {
            line.to_string()
        } else {
            code.trim().to_string()
        }
    }

    let mut headers: Vec<String> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, String> = HashMap::new();

    for line in local.lines().map(str::trim_end) {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            headers.push(line.to_string());
        } else if !trimmed.is_empty() {
            let key = key_of(line);
            if !map.contains_key(&key) {
                order.push(key.clone());
            }
            map.insert(key, line.to_string());
        }
    }

    for line in remote.lines().map(str::trim_end) {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            if !headers.contains(&line.to_string()) {
                headers.push(line.to_string());
            }
        } else if !trimmed.is_empty() {
            let key = key_of(line);
            if !map.contains_key(&key) {
                order.push(key.clone());
            }
            map.insert(key, line.to_string());
        }
    }

    let mut out = headers;
    out.extend(
        order
            .into_iter()
            .map(|key| map.remove(&key).unwrap_or_default()),
    );
    out.join("\n")
}

fn merge_json(local: &str, remote: &str) -> Result<String, String> {
    let mut local: serde_json::Value =
        serde_json::from_str(local).map_err(|e| format!("本地 JSON 解析失败：{e}"))?;
    let remote: serde_json::Value =
        serde_json::from_str(remote).map_err(|e| format!("远端 JSON 解析失败：{e}"))?;
    if !local.is_object() || !remote.is_object() {
        return Err("配置 JSON 根节点不是对象，无法自动合并".to_owned());
    }
    merge_json_into(&mut local, &remote);
    serde_json::to_string_pretty(&local).map_err(|e| format!("合并 JSON 序列化失败：{e}"))
}

fn merge_json_into(local: &mut serde_json::Value, remote: &serde_json::Value) {
    if let serde_json::Value::Object(local_obj) = local {
        if let serde_json::Value::Object(remote_obj) = remote {
            for (key, remote_value) in remote_obj {
                if let Some(local_value) = local_obj.get_mut(key) {
                    if local_value.is_object() && remote_value.is_object() {
                        merge_json_into(local_value, remote_value);
                    } else if local_value != remote_value {
                        // 标量/数组冲突：远端优先（远端是刚收到的显式修改）。
                        *local_value = remote_value.clone();
                    }
                } else {
                    local_obj.insert(key.clone(), remote_value.clone());
                }
            }
            return;
        }
    }
    *local = remote.clone();
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    fn tmp_root() -> PathBuf {
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "shurufa-config-sync-test-{}-{}",
            std::process::id(),
            id
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 备份文件名解析custom_phrase类型() {
        assert_eq!(
            kind_from_backup_name("123_options_options.json"),
            Some("options")
        );
        assert_eq!(
            kind_from_backup_name("123_skin_shurufa-skin.json"),
            Some("skin")
        );
        assert_eq!(
            kind_from_backup_name("123_custom_phrase_custom_phrase.txt"),
            Some("custom_phrase")
        );
        assert_eq!(kind_from_backup_name("bad-name"), None);
    }

    #[test]
    fn 首次远端写入并记录状态() {
        let root = tmp_root();
        let out = apply_incoming(&root, "options", "options.json", r#"{"a":1}"#).unwrap();
        assert_eq!(out.status, ApplyStatus::AppliedRemote);
        let state = load_state(&root);
        let entry = state.files.get("options").unwrap();
        assert_eq!(entry.local_sha256, entry.remote_sha256);
        assert!(root.join("options.json").exists());
    }

    #[test]
    fn 本地未变远端变化直接采用() {
        let root = tmp_root();
        apply_incoming(
            &root,
            "custom_phrase",
            "custom_phrase.txt",
            "公司\tgs\t100\n",
        )
        .unwrap();
        let out = apply_incoming(
            &root,
            "custom_phrase",
            "custom_phrase.txt",
            "公司\tgs\t100\n位置\twz\t90\n",
        )
        .unwrap();
        assert_eq!(out.status, ApplyStatus::AppliedRemote);
        let text = std::fs::read_to_string(root.join("rime/custom_phrase.txt")).unwrap();
        assert!(text.contains("位置\twz"));
    }

    #[test]
    fn 两端修改自动合并并备份() {
        let root = tmp_root();
        apply_incoming(
            &root,
            "custom_phrase",
            "custom_phrase.txt",
            "公司\tgs\t100\n",
        )
        .unwrap();
        // 本地修改：加一条本地专属。
        let local_path = root.join("rime/custom_phrase.txt");
        std::fs::write(&local_path, "公司\tgs\t100\n本地\tbd\t90\n").unwrap();
        // 远端也修改：加一条远端专属。
        let out = apply_incoming(
            &root,
            "custom_phrase",
            "custom_phrase.txt",
            "公司\tgs\t100\n远端\tyd\t80\n",
        )
        .unwrap();
        assert_eq!(out.status, ApplyStatus::Merged);
        assert!(out.backup.is_some());
        let text = std::fs::read_to_string(&local_path).unwrap();
        assert!(text.contains("本地\tbd\t90"));
        assert!(text.contains("远端\tyd\t80"));
        // 备份目录应有本地旧文件 + 远端原文件两份。
        assert_eq!(std::fs::read_dir(backup_dir(&root)).unwrap().count(), 2);
        // 冲突记录应可被 UI 读取。
        let conflicts = load_conflicts(&root).conflicts;
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, "custom_phrase");
    }

    #[test]
    fn 首次收到且本地已有不同内容自动合并() {
        let root = tmp_root();
        std::fs::write(
            root.join("options.json"),
            r#"{"local":true,"shared":"old"}"#,
        )
        .unwrap();
        let out = apply_incoming(
            &root,
            "options",
            "options.json",
            r#"{"remote":true,"shared":"new"}"#,
        )
        .unwrap();
        assert_eq!(out.status, ApplyStatus::Merged);
        let text = std::fs::read_to_string(root.join("options.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["local"], true);
        assert_eq!(v["remote"], true);
        assert_eq!(v["shared"], "new");
        assert!(out.backup.is_some());
    }

    #[test]
    fn json深度合并远端标量优先() {
        let root = tmp_root();
        apply_incoming(
            &root,
            "skin",
            "shurufa-skin.json",
            r##"{"light":{"keyboard":{"key":"#fff"}},"version":1}"##,
        )
        .unwrap();
        let local = r##"{"light":{"keyboard":{"key":"#000","bg":"#eee"}},"version":1}"##;
        let remote = r##"{"light":{"keyboard":{"key":"#123456"},"candidate":{"background":"#fafafa"}},"version":1}"##;
        let merged = merge("skin", local, remote).unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["light"]["keyboard"]["key"], "#123456");
        assert_eq!(v["light"]["keyboard"]["bg"], "#eee");
        assert_eq!(v["light"]["candidate"]["background"], "#fafafa");
    }

    #[test]
    fn 增量发送只返回变化文件() {
        let root = tmp_root();
        let path = root.join("options.json");
        std::fs::write(&path, r#"{"a":1}"#).unwrap();
        assert_eq!(
            prepare_send(&root, "options", &path).unwrap().unwrap(),
            r#"{"a":1}"#
        );
        mark_sent(&root, "options", r#"{"a":1}"#).unwrap();
        assert!(prepare_send(&root, "options", &path).unwrap().is_none());
        std::fs::write(&path, r#"{"a":2}"#).unwrap();
        assert!(prepare_send(&root, "options", &path).unwrap().is_some());
    }

    #[test]
    fn 本地仅变化保留本地不覆盖() {
        let root = tmp_root();
        apply_incoming(&root, "options", "options.json", r#"{"a":1}"#).unwrap();
        std::fs::write(root.join("options.json"), r#"{"a":1,"local":true}"#).unwrap();
        let out = apply_incoming(&root, "options", "options.json", r#"{"a":1}"#).unwrap();
        assert_eq!(out.status, ApplyStatus::KeptLocal);
        let text = std::fs::read_to_string(root.join("options.json")).unwrap();
        assert!(text.contains("local"));
    }
}
