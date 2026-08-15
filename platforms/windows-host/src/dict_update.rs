//! 自托管云词库更新：下载带 SHA-256 的清单，校验后原子替换 Rime 源词典。
//!
//! 清单和内容均要求 HTTPS。更新只允许仓库现有 `schemas/` 下的 YAML 相对
//! 路径，避免把远端内容写到词典目录之外。完成后调用同目录的
//! `rime_deployer.exe` 重建二进制词典。

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILES: usize = 32;
const DOWNLOAD_ATTEMPTS: u8 = 3;
/// 本地快照保留的最大代数（不含当前正在运行的版本）。
const MAX_SNAPSHOTS: usize = 5;

#[derive(Debug, Deserialize)]
struct Manifest {
    version: u32,
    revision: String,
    files: Vec<ManifestFile>,
    /// v2 新增：历史 revision 列表（仅元信息，不强制顺序）。
    /// 真实的回滚依据是本地 snapshot 栈，history 只是"上游发布轨迹"。
    #[serde(default)]
    history: Option<Vec<HistoryEntry>>,
}

/// manifest v2 的历史条目。字段全部可选兼容最小实现。
#[derive(Debug, Clone, serde::Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntry {
    pub revision: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub applied_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    path: String,
    url: String,
    #[serde(default)]
    fallback_urls: Vec<String>,
    sha256: String,
    size: usize,
}

/// 一次替换所保留的旧文件状态，用于部署器失败或中途替换失败时恢复。
struct AppliedFile {
    target: PathBuf,
    backup: PathBuf,
    existed: bool,
}

pub fn cli_update(manifest_url: &str) {
    let schema_dir = schema_dir();
    match update_from_url(&schema_dir, manifest_url) {
        Ok(revision) => println!("词库已更新到版本 {revision}，请重启输入法。"),
        Err(error) => {
            eprintln!("词库更新失败：{error}");
            std::process::exit(1);
        }
    }
}

/// 打印当前词库版本（无 `.current-revision` 时为发布自带的内置版本）。
pub fn cli_current() {
    match read_current_revision(&schema_dir()) {
        Some(revision) => println!("{revision}"),
        None => println!("内置"),
    }
}

/// 回滚到上次更新前的词库；无备份时打印提示并正常返回。
/// 成功后重建二进制词典。
pub fn cli_rollback(revision: Option<&str>) {
    let schema_dir = schema_dir();
    let result = match revision {
        Some(rev) if !rev.trim().is_empty() => rollback_to(&schema_dir, rev.trim()),
        _ => rollback(&schema_dir),
    };
    let result = result.and_then(|restored| {
        if restored.is_some() {
            rebuild(&schema_dir)?;
        }
        Ok(restored)
    });
    match result {
        Ok(Some(revision)) => println!("已回滚到 {revision}"),
        Ok(None) => println!("无可回滚版本"),
        Err(error) => {
            eprintln!("词库回滚失败：{error}");
            std::process::exit(1);
        }
    }
}

/// 列出本地可回滚的历史版本（最近的在前）。
pub fn cli_history() {
    let schema_dir = schema_dir();
    let stack = read_snapshot_stack(&schema_dir);
    if stack.is_empty() {
        println!("（无历史快照）");
        return;
    }
    for entry in stack.iter().rev() {
        // revision 为空表示"内置"（发布自带版本，无 .current-revision 时）
        let label = if entry.revision.is_empty() {
            "内置"
        } else {
            entry.revision.as_str()
        };
        println!("{label}");
    }
}

fn schemas_backup_dir(schema_dir: &Path) -> PathBuf {
    schema_dir.join(".backup")
}

fn current_revision_path(schema_dir: &Path) -> PathBuf {
    schema_dir.join(".current-revision")
}

fn read_current_revision(schema_dir: &Path) -> Option<String> {
    let revision = fs::read_to_string(current_revision_path(schema_dir)).ok()?;
    let revision = revision.trim().to_string();
    if revision.is_empty() {
        None
    } else {
        Some(revision)
    }
}

/// 单代快照的元信息（`.backup/<slot>/` 目录 + 栈索引一行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotEntry {
    /// 该快照回滚后会到达的 revision；空串表示"内置"（出厂状态）。
    pub revision: String,
    /// 快照槽位名（同时也是 `.backup/` 下的子目录名）。
    pub slot: String,
}

fn snapshot_index_path(backup_dir: &Path) -> PathBuf {
    backup_dir.join(".snapshot-index")
}

fn sanitize_slot_revision(revision: &str) -> String {
    if revision.is_empty() {
        return "builtin".to_string();
    }
    revision
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 读取快照栈（旧→新顺序）。兼容 v1 布局：`.backup/.previous-revision` 存在
/// 但无 `.snapshot-index` 时，把 `.backup` 根目录视作唯一一代快照。
fn read_snapshot_stack(schema_dir: &Path) -> Vec<SnapshotEntry> {
    let backup_dir = schemas_backup_dir(schema_dir);
    if !backup_dir.is_dir() {
        return Vec::new();
    }
    let index_path = snapshot_index_path(&backup_dir);
    if let Ok(text) = fs::read_to_string(&index_path) {
        let mut stack = Vec::new();
        for line in text.lines() {
            // 不能用 trim()：revision 为空时行末恰好是 "\t"，trim 会把它吞掉
            if line.is_empty() {
                continue;
            }
            let Some((slot, revision)) = line.split_once('\t') else {
                continue;
            };
            let slot = slot.trim();
            let revision = revision.trim();
            if slot.is_empty() {
                continue;
            }
            // 只把真实存在的槽位列入栈，索引与目录脱节时静默跳过
            if backup_dir.join(slot).is_dir() {
                stack.push(SnapshotEntry {
                    revision: revision.to_string(),
                    slot: slot.to_string(),
                });
            }
        }
        return stack;
    }
    // v1 兼容：单代快照（文件直接放在 .backup 根下）
    if backup_dir.join(".previous-revision").is_file() {
        let revision = fs::read_to_string(backup_dir.join(".previous-revision"))
            .unwrap_or_default()
            .trim()
            .to_string();
        return vec![SnapshotEntry {
            revision,
            slot: String::new(), // 空串 = 直接使用 .backup 根目录
        }];
    }
    Vec::new()
}

fn write_snapshot_stack(schema_dir: &Path, stack: &[SnapshotEntry]) -> Result<(), String> {
    let backup_dir = schemas_backup_dir(schema_dir);
    fs::create_dir_all(&backup_dir).map_err(|e| format!("创建回滚备份目录失败: {e}"))?;
    let mut text = String::new();
    for entry in stack {
        text.push_str(&entry.slot);
        text.push('\t');
        text.push_str(&entry.revision);
        text.push('\n');
    }
    fs::write(snapshot_index_path(&backup_dir), text).map_err(|e| format!("写入快照索引失败: {e}"))
}

/// 每次更新成功后入栈一代新快照；栈超过 MAX_SNAPSHOTS 时淘汰最老一代（FIFO）。
/// 若此前存在 v1 布局（文件直接在 .backup 根目录），先迁移为 slot 形式再追加。
fn push_snapshot(
    schema_dir: &Path,
    applied: &[AppliedFile],
    previous_revision: &str,
) -> Result<(), String> {
    let backup_dir = schemas_backup_dir(schema_dir);
    let mut stack = read_snapshot_stack(schema_dir);

    // v1 → v2 迁移：根目录下既有文件整体搬入 legacy slot
    if stack.len() == 1 && stack[0].slot.is_empty() {
        let legacy_slot = "0001-legacy".to_string();
        let legacy_dir = backup_dir.join(&legacy_slot);
        fs::create_dir_all(&legacy_dir).map_err(|e| format!("迁移旧回滚备份失败: {e}"))?;
        for entry in fs::read_dir(&backup_dir).map_err(|e| format!("读取回滚备份失败: {e}"))?
        {
            let entry = entry.map_err(|e| format!("读取回滚备份失败: {e}"))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name == legacy_slot || name == ".snapshot-index" {
                continue;
            }
            let dest = legacy_dir.join(&name);
            fs::rename(&path, &dest).map_err(|e| format!("迁移旧回滚备份 {name} 失败: {e}"))?;
        }
        stack[0].slot = legacy_slot;
    }

    // 计算新槽位序号（取现有槽位最大前缀 +1，避免与 legacy 冲突）
    let next_id = stack
        .iter()
        .filter_map(|entry| entry.slot.split('-').next()?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let slot = format!("{next_id:04}-{}", sanitize_slot_revision(previous_revision));
    let slot_dir = backup_dir.join(&slot);
    if slot_dir.exists() {
        fs::remove_dir_all(&slot_dir).map_err(|e| format!("清理冲突快照槽位失败: {e}"))?;
    }
    fs::create_dir_all(&slot_dir).map_err(|e| format!("创建快照槽位失败: {e}"))?;

    let mut added = Vec::new();
    for file in applied {
        if file.existed {
            let dest = slot_dir.join(
                file.target
                    .strip_prefix(schema_dir)
                    .map_err(|_| "词库备份路径解析失败")?,
            );
            let parent = dest.parent().ok_or("词库备份路径没有父目录")?;
            fs::create_dir_all(parent).map_err(|e| format!("创建回滚备份子目录失败: {e}"))?;
            fs::copy(&file.backup, &dest).map_err(|e| format!("写入回滚备份失败: {e}"))?;
        } else {
            added.push(
                file.target
                    .strip_prefix(schema_dir)
                    .map_err(|_| "词库备份路径解析失败")?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    fs::write(slot_dir.join(".added"), added.join("\n"))
        .map_err(|e| format!("写入新增清单失败: {e}"))?;
    stack.push(SnapshotEntry {
        revision: previous_revision.to_string(),
        slot,
    });

    // FIFO 压缩：只保留最近 MAX_SNAPSHOTS 代
    while stack.len() > MAX_SNAPSHOTS {
        let oldest = stack.remove(0);
        if !oldest.slot.is_empty() {
            let _ = fs::remove_dir_all(backup_dir.join(&oldest.slot));
        }
    }
    write_snapshot_stack(schema_dir, &stack)
}

/// 弹出栈顶一代快照：恢复文件、删除新增、回写版本号、从索引移除。
fn pop_snapshot(schema_dir: &Path) -> Result<Option<String>, String> {
    let mut stack = read_snapshot_stack(schema_dir);
    let Some(top) = stack.pop() else {
        return Ok(None);
    };
    restore_snapshot(schema_dir, &top)?;
    // 索引重写；若已空则连 .backup 一并清掉，保持与 v1"备份已消费"语义一致
    if stack.is_empty() {
        let backup_dir = schemas_backup_dir(schema_dir);
        if backup_dir.exists() {
            fs::remove_dir_all(&backup_dir).map_err(|e| format!("清理回滚备份失败: {e}"))?;
        }
    } else {
        write_snapshot_stack(schema_dir, &stack)?;
    }
    Ok(Some(if top.revision.is_empty() {
        "内置".into()
    } else {
        top.revision
    }))
}

/// 回滚到指定 revision。若目标在栈中间，把比它新的快照全部弹出丢弃，
/// 然后把目标代弹出来恢复（单层失败时中止，不跨层盲目尝试）。
fn rollback_to(schema_dir: &Path, revision: &str) -> Result<Option<String>, String> {
    let target = if revision == "内置" { "" } else { revision };
    let stack = read_snapshot_stack(schema_dir);
    if !stack.iter().any(|entry| entry.revision == target) {
        return Err(format!("没有可回滚到 {revision} 的快照"));
    }
    let mut restored = None;
    while let Some(revision_text) = pop_snapshot(schema_dir)? {
        let done = revision_text == revision;
        restored = Some(revision_text);
        if done {
            break;
        }
    }
    // 最后一次 restore 已把 .current-revision 回写为目标 revision，可直接返回
    Ok(restored)
}

/// 把某代快照内容恢复回 schemas 目录：复制旧文件、删除新增文件、回写版本号。
fn restore_snapshot(schema_dir: &Path, snapshot: &SnapshotEntry) -> Result<(), String> {
    let backup_dir = schemas_backup_dir(schema_dir);
    let slot_root = if snapshot.slot.is_empty() {
        backup_dir.clone()
    } else {
        backup_dir.join(&snapshot.slot)
    };

    // 1. 恢复被替换的旧文件（跳过备份元数据文件与 v2 的子槽位目录）
    for entry in fs::read_dir(&slot_root).map_err(|e| format!("读取回滚备份失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取回滚备份失败: {e}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            restore_backup_tree(&path, &slot_root, schema_dir)?;
        } else {
            restore_backup_file(&path, &slot_root, schema_dir)?;
        }
    }

    // 2. 删除该次更新新增的文件
    let added_path = slot_root.join(".added");
    if let Ok(list) = fs::read_to_string(&added_path) {
        for relative in list.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let relative_path = Path::new(relative);
            if relative_path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(format!("回滚新增清单含非法路径：{relative}"));
            }
            let target = schema_dir.join(relative_path);
            if target.exists() {
                fs::remove_file(&target)
                    .map_err(|e| format!("删除新增词库 {relative} 失败: {e}"))?;
            }
        }
    }

    // 3. 版本号回写（空 revision 表示内置版本）
    if snapshot.revision.is_empty() {
        let _ = fs::remove_file(current_revision_path(schema_dir));
    } else {
        fs::write(current_revision_path(schema_dir), &snapshot.revision)
            .map_err(|e| format!("写回当前版本号失败: {e}"))?;
    }
    Ok(())
}

/// 更新成功后生成回滚快照：被替换的旧文件复制到 `<schemas>/.backup/<slot>/`，
/// 新增文件记入 slot 内 `.added` 清单（回滚时删除），上一版 revision 进栈索引
/// `.backup/.snapshot-index`；栈最多保留最近 5 代（FIFO 淘汰最老）。
/// 最后把本次 revision 写入 `<schemas>/.current-revision`。
fn persist_revision_snapshot(
    schema_dir: &Path,
    applied: &[AppliedFile],
    new_revision: &str,
) -> Result<(), String> {
    let previous = read_current_revision(schema_dir);
    push_snapshot(schema_dir, applied, previous.as_deref().unwrap_or(""))?;
    fs::write(current_revision_path(schema_dir), new_revision)
        .map_err(|e| format!("写入当前版本号失败: {e}"))?;
    Ok(())
}

/// 回滚到上一代：弹出栈顶快照、恢复文件、回写版本号。不调用 rebuild，
/// 由调用方决定是否重建二进制词典（测试里跳过 deployer）。
fn rollback(schema_dir: &Path) -> Result<Option<String>, String> {
    pop_snapshot(schema_dir)
}

/// 递归恢复备份树（备份目录下的子目录，如 cn_dicts/）。
fn restore_backup_tree(dir: &Path, backup_root: &Path, schema_dir: &Path) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("读取回滚备份失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取回滚备份失败: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            restore_backup_tree(&path, backup_root, schema_dir)?;
        } else {
            restore_backup_file(&path, backup_root, schema_dir)?;
        }
    }
    Ok(())
}

/// 把单个备份文件复制回 schemas 目录对应位置。
fn restore_backup_file(file: &Path, backup_root: &Path, schema_dir: &Path) -> Result<(), String> {
    let relative = file
        .strip_prefix(backup_root)
        .map_err(|_| "词库备份路径解析失败")?;
    let target = schema_dir.join(relative);
    let parent = target.parent().ok_or("词库恢复路径没有父目录")?;
    fs::create_dir_all(parent).map_err(|e| format!("创建词库恢复目录失败: {e}"))?;
    fs::copy(file, &target).map_err(|e| format!("恢复 {} 失败: {e}", target.display()))?;
    Ok(())
}

fn schema_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("SHURUFA_SCHEMAS") {
        return PathBuf::from(path);
    }
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
        .join("schemas")
}

fn update_from_url(schema_dir: &Path, manifest_url: &str) -> Result<String, String> {
    let manifest_bytes = if manifest_url.trim().eq_ignore_ascii_case("rime-ice") {
        include_bytes!("../../../schemas/rime-ice-2026.06.30.json").to_vec()
    } else {
        download(manifest_url, MAX_MANIFEST_BYTES)?
    };
    let manifest = parse_manifest(&manifest_bytes)?;
    if !schema_dir.is_dir() {
        return Err(format!("未找到已部署的词典目录：{}", schema_dir.display()));
    }

    let stage = schema_dir.join(".dict-update-staging");
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|e| format!("清理旧词库暂存失败: {e}"))?;
    }
    fs::create_dir_all(&stage).map_err(|e| format!("创建词库暂存目录失败: {e}"))?;

    let result = (|| {
        for file in &manifest.files {
            let bytes = download_verified(file)?;
            let staged = stage.join(&file.path);
            let parent = staged.parent().ok_or("词库暂存路径没有父目录")?;
            fs::create_dir_all(parent).map_err(|e| format!("创建词库暂存子目录失败: {e}"))?;
            fs::write(&staged, bytes).map_err(|e| format!("写入词库暂存文件失败: {e}"))?;
        }
        ensure_deployer(schema_dir)?;
        let applied = apply_staged(schema_dir, &stage, &manifest.files)?;
        match rebuild(schema_dir) {
            Ok(()) => {
                // 部署成功后才固化回滚快照与版本号；快照失败不回退已部署内容。
                // 若当前 revision 与 manifest 相同（重复更新同一版），把当前 revision
                // 也作为"上一代"入栈，保证 dict-rollback 始终有上一代可弹。
                persist_revision_snapshot(schema_dir, &applied, &manifest.revision)?;
                Ok(manifest.revision.clone())
            }
            Err(error) => {
                rollback_staged(&applied).map_err(|rollback_error| {
                    format!("{error}；恢复旧词库失败：{rollback_error}")
                })?;
                Err(format!("{error}；已恢复到更新前的词库"))
            }
        }
    })();
    let _ = fs::remove_dir_all(&stage);
    result
}

fn parse_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|e| format!("词库清单格式错误: {e}"))?;
    if manifest.version != 1 && manifest.version != 2 {
        return Err("词库清单版本不受支持".into());
    }
    if manifest.revision.trim().is_empty()
        || manifest.files.is_empty()
        || manifest.files.len() > MAX_FILES
    {
        return Err("词库清单的版本或文件数无效".into());
    }
    if let Some(history) = &manifest.history {
        if history.len() > 64 {
            return Err("词库清单历史版本数无效".into());
        }
        for entry in history {
            if entry.revision.trim().is_empty() {
                return Err("词库清单历史版本号无效".into());
            }
        }
    }
    for file in &manifest.files {
        validate_file(file)?;
    }
    Ok(manifest)
}

fn validate_file(file: &ManifestFile) -> Result<(), String> {
    let path = Path::new(&file.path);
    if path.extension().and_then(|value| value.to_str()) != Some("yaml")
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("词库路径非法：{}", file.path));
    }
    if file.source_urls().any(|url| !url.starts_with("https://")) {
        return Err(format!("词库下载地址必须使用 HTTPS：{}", file.path));
    }
    if file.size == 0
        || file.size > MAX_FILE_BYTES
        || file.sha256.len() != 64
        || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("词库文件元数据无效：{}", file.path));
    }
    Ok(())
}

impl ManifestFile {
    fn source_urls(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.url.as_str()).chain(self.fallback_urls.iter().map(String::as_str))
    }
}

fn download_verified(file: &ManifestFile) -> Result<Vec<u8>, String> {
    let mut errors = Vec::new();
    for url in file.source_urls() {
        match download(url, file.size).and_then(|bytes| {
            verify_file(file, &bytes)?;
            Ok(bytes)
        }) {
            Ok(bytes) => return Ok(bytes),
            Err(error) => errors.push(format!("{url}：{error}")),
        }
    }
    Err(format!("所有词库下载源均失败：{}", errors.join("；")))
}

fn download(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err("下载地址必须使用 HTTPS".into());
    }
    let mut last_error = String::new();
    for _ in 0..DOWNLOAD_ATTEMPTS {
        match download_once(url, limit) {
            Ok(data) => return Ok(data),
            Err(error) => last_error = error,
        }
    }
    Err(format!(
        "下载 {url} 失败（已尝试 {DOWNLOAD_ATTEMPTS} 次）：{last_error}"
    ))
}

fn download_once(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();
    let response = agent.get(url).call().map_err(|e| e.to_string())?;
    if let Some(length) = response
        .header("Content-Length")
        .and_then(|value| value.parse::<usize>().ok())
    {
        if length > limit {
            return Err(format!("下载内容超过上限：{url}"));
        }
    }
    let mut data = Vec::new();
    response
        .into_reader()
        .take((limit + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|e| format!("读取下载内容失败: {e}"))?;
    if data.len() > limit {
        return Err(format!("下载内容超过上限：{url}"));
    }
    Ok(data)
}

fn verify_file(file: &ManifestFile, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() != file.size {
        return Err(format!("词库大小校验失败：{}", file.path));
    }
    let actual = sha256_hex(bytes);
    if !actual.eq_ignore_ascii_case(&file.sha256) {
        return Err(format!("词库 SHA-256 校验失败：{}", file.path));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn apply_staged(
    schema_dir: &Path,
    stage: &Path,
    files: &[ManifestFile],
) -> Result<Vec<AppliedFile>, String> {
    let backup_root = stage.join(".backup");
    let mut applied = Vec::with_capacity(files.len());
    for file in files {
        let source = stage.join(&file.path);
        let target = schema_dir.join(&file.path);
        let parent = target.parent().ok_or("词库目标路径没有父目录")?;
        fs::create_dir_all(parent).map_err(|e| format!("创建词库目标目录失败: {e}"))?;
        let temporary = parent.join(format!(
            ".{}.new",
            target
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("词库文件名无效")?
        ));
        let backup = backup_root.join(&file.path);
        let existed = target.exists();
        if existed {
            let backup_parent = backup.parent().ok_or("词库备份路径没有父目录")?;
            fs::create_dir_all(backup_parent).map_err(|e| format!("创建词库备份目录失败: {e}"))?;
            fs::copy(&target, &backup).map_err(|e| format!("备份旧词库失败: {e}"))?;
        }
        applied.push(AppliedFile {
            target: target.clone(),
            backup,
            existed,
        });
        let result: Result<(), String> = (|| {
            fs::copy(&source, &temporary).map_err(|e| format!("复制词库更新失败: {e}"))?;
            fs::rename(&temporary, &target).map_err(|e| format!("替换词库文件失败: {e}"))?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            let rollback_error = rollback_staged(&applied).err();
            return Err(match rollback_error {
                Some(rollback_error) => format!("{error}；恢复已替换词库失败：{rollback_error}"),
                None => format!("{error}；已恢复此前已替换的词库"),
            });
        }
    }
    Ok(applied)
}

fn rollback_staged(applied: &[AppliedFile]) -> Result<(), String> {
    for file in applied.iter().rev() {
        if file.existed {
            fs::copy(&file.backup, &file.target)
                .map_err(|e| format!("恢复 {} 失败: {e}", file.target.display()))?;
        } else if file.target.exists() {
            fs::remove_file(&file.target)
                .map_err(|e| format!("删除新增词库 {} 失败: {e}", file.target.display()))?;
        }
    }
    Ok(())
}

fn ensure_deployer(schema_dir: &Path) -> Result<PathBuf, String> {
    let Some(root) = schema_dir.parent() else {
        return Err("词库目录没有部署根目录".into());
    };
    let deployer = root.join("rime_deployer.exe");
    if !deployer.is_file() {
        return Err(format!("未找到词典编译器：{}", deployer.display()));
    }
    Ok(deployer)
}

fn rebuild(schema_dir: &Path) -> Result<(), String> {
    let deployer = ensure_deployer(schema_dir)?;
    let status = std::process::Command::new(deployer)
        .args([
            "--build",
            &schema_dir.to_string_lossy(),
            &schema_dir.to_string_lossy(),
            &schema_dir.join("build").to_string_lossy(),
        ])
        .status()
        .map_err(|e| format!("启动词典编译器失败: {e}"))?;
    if !status.success() {
        return Err("词库编译失败".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 清单拒绝非_https_与路径穿越() {
        let invalid = br#"{"version":1,"revision":"r1","files":[{"path":"../bad.yaml","url":"https://example.com/bad","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":1}]}"#;
        assert!(parse_manifest(invalid).is_err());
        let invalid_url = br#"{"version":1,"revision":"r1","files":[{"path":"cn_dicts/custom.yaml","url":"http://example.com/dict","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":1}]}"#;
        assert!(parse_manifest(invalid_url).is_err());
    }

    #[test]
    fn 文件内容必须匹配清单哈希与大小() {
        let file = ManifestFile {
            path: "cn_dicts/custom.yaml".into(),
            url: "https://example.com/dict".into(),
            fallback_urls: vec![],
            sha256: sha256_hex(b"dictionary"),
            size: b"dictionary".len(),
        };
        assert!(verify_file(&file, b"dictionary").is_ok());
        assert!(verify_file(&file, b"incorrect").is_err());
    }

    #[test]
    fn 内置雾凇拼音清单可被解析() {
        let manifest =
            parse_manifest(include_bytes!("../../../schemas/rime-ice-2026.06.30.json")).unwrap();
        assert_eq!(manifest.revision, "rime-ice-2026.06.30");
        assert_eq!(manifest.files.len(), 4);
        assert!(manifest
            .files
            .iter()
            .all(|file| file.path.starts_with("cn_dicts/")));
        assert!(manifest
            .files
            .iter()
            .all(|file| file.fallback_urls.len() == 1));
    }

    #[test]
    fn 替换后可恢复旧词库与删除新增文件() {
        let root =
            std::env::temp_dir().join(format!("shurufa-dict-update-test-{}", std::process::id()));
        let schema_dir = root.join("schemas");
        let stage = root.join("stage");
        fs::create_dir_all(schema_dir.join("cn_dicts")).unwrap();
        fs::create_dir_all(stage.join("cn_dicts")).unwrap();
        fs::write(schema_dir.join("cn_dicts/existing.yaml"), b"old").unwrap();
        fs::write(stage.join("cn_dicts/existing.yaml"), b"new").unwrap();
        fs::write(stage.join("cn_dicts/added.yaml"), b"added").unwrap();
        let files = vec![
            ManifestFile {
                path: "cn_dicts/existing.yaml".into(),
                url: "https://example.com/existing.yaml".into(),
                fallback_urls: vec![],
                sha256: "a".repeat(64),
                size: 1,
            },
            ManifestFile {
                path: "cn_dicts/added.yaml".into(),
                url: "https://example.com/added.yaml".into(),
                fallback_urls: vec![],
                sha256: "a".repeat(64),
                size: 1,
            },
        ];

        let applied = apply_staged(&schema_dir, &stage, &files).unwrap();
        assert_eq!(
            fs::read(schema_dir.join("cn_dicts/existing.yaml")).unwrap(),
            b"new"
        );
        assert_eq!(
            fs::read(schema_dir.join("cn_dicts/added.yaml")).unwrap(),
            b"added"
        );
        rollback_staged(&applied).unwrap();
        assert_eq!(
            fs::read(schema_dir.join("cn_dicts/existing.yaml")).unwrap(),
            b"old"
        );
        assert!(!schema_dir.join("cn_dicts/added.yaml").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "需要访问 rime-ice 上游，手动运行以验证固定清单"]
    fn 内置雾凇拼音清单可下载并通过校验() {
        let manifest =
            parse_manifest(include_bytes!("../../../schemas/rime-ice-2026.06.30.json")).unwrap();
        for file in &manifest.files {
            let bytes = download(&file.url, file.size).unwrap();
            verify_file(file, &bytes).unwrap();
        }
    }

    #[test]
    fn 回滚快照记录旧文件与新增清单() {
        let root =
            std::env::temp_dir().join(format!("shurufa-dict-snap-test-{}", std::process::id()));
        let schema_dir = root.join("schemas");
        let stage = root.join("stage");
        fs::create_dir_all(schema_dir.join("cn_dicts")).unwrap();
        fs::create_dir_all(stage.join("cn_dicts")).unwrap();
        fs::write(schema_dir.join("cn_dicts/existing.yaml"), b"old").unwrap();
        fs::write(stage.join("cn_dicts/existing.yaml"), b"new").unwrap();
        fs::write(stage.join("cn_dicts/added.yaml"), b"added").unwrap();
        let files = vec![
            ManifestFile {
                path: "cn_dicts/existing.yaml".into(),
                url: "https://example.com/existing.yaml".into(),
                fallback_urls: vec![],
                sha256: "a".repeat(64),
                size: 1,
            },
            ManifestFile {
                path: "cn_dicts/added.yaml".into(),
                url: "https://example.com/added.yaml".into(),
                fallback_urls: vec![],
                sha256: "a".repeat(64),
                size: 1,
            },
        ];
        let applied = apply_staged(&schema_dir, &stage, &files).unwrap();

        // 无版本时首更：previous 应为空串（内置）
        persist_revision_snapshot(&schema_dir, &applied, "r1").unwrap();
        assert_eq!(read_current_revision(&schema_dir).as_deref(), Some("r1"));
        let backup = schemas_backup_dir(&schema_dir);
        let stack = read_snapshot_stack(&schema_dir);
        assert_eq!(stack.len(), 1);
        assert_eq!(stack[0].revision, "");
        let slot_dir = backup.join(&stack[0].slot);
        assert_eq!(
            fs::read(slot_dir.join("cn_dicts/existing.yaml")).unwrap(),
            b"old"
        );
        assert_eq!(
            fs::read_to_string(slot_dir.join(".added")).unwrap(),
            "cn_dicts/added.yaml"
        );

        // 再更一代：快照入栈（两代并存），最新一代 previous = r1
        fs::write(stage.join("cn_dicts/existing.yaml"), b"newer").unwrap();
        // added.yaml 本次不在清单内，模拟其作为既有文件存在
        let files2 = vec![ManifestFile {
            path: "cn_dicts/existing.yaml".into(),
            url: "https://example.com/existing.yaml".into(),
            fallback_urls: vec![],
            sha256: "a".repeat(64),
            size: 1,
        }];
        let applied2 = apply_staged(&schema_dir, &stage, &files2).unwrap();
        persist_revision_snapshot(&schema_dir, &applied2, "r2").unwrap();
        assert_eq!(read_current_revision(&schema_dir).as_deref(), Some("r2"));
        let stack = read_snapshot_stack(&schema_dir);
        assert_eq!(stack.len(), 2);
        assert_eq!(stack[0].revision, "");
        assert_eq!(stack[1].revision, "r1");
        let top_dir = backup.join(&stack[1].slot);
        assert_eq!(
            fs::read(top_dir.join("cn_dicts/existing.yaml")).unwrap(),
            b"new"
        );
        assert_eq!(fs::read_to_string(top_dir.join(".added")).unwrap(), "");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 无备份时回滚返回空() {
        let root =
            std::env::temp_dir().join(format!("shurufa-dict-rb-none-test-{}", std::process::id()));
        let schema_dir = root.join("schemas");
        fs::create_dir_all(&schema_dir).unwrap();
        assert_eq!(rollback(&schema_dir).unwrap(), None);
        assert_eq!(read_current_revision(&schema_dir), None);
        fs::remove_dir_all(root).unwrap();
    }

    /// 构造一个只含 applied 元数据（不真实复制）的 AppliedFile 列表，
    /// 供 persist_revision_snapshot 测试用。
    fn fake_applied(
        schema_dir: &Path,
        existed_rel: &[&str],
        added_rel: &[&str],
    ) -> Vec<AppliedFile> {
        let mut applied = Vec::new();
        for rel in existed_rel {
            let target = schema_dir.join(rel);
            let backup = target.with_extension("yaml.bak");
            // 备份文件需要真实存在，persist 会 fs::copy
            fs::copy(&target, &backup).unwrap();
            applied.push(AppliedFile {
                target,
                backup,
                existed: true,
            });
        }
        for rel in added_rel {
            applied.push(AppliedFile {
                target: schema_dir.join(rel),
                backup: schema_dir.join(rel).with_extension("yaml.bak"),
                existed: false,
            });
        }
        applied
    }

    #[test]
    fn manifest_v2_带_history_段可解析() {
        let bytes = br#"{
            "version": 2,
            "revision": "rime-ice-2026.08.01",
            "history": [
                {"revision": "rime-ice-2026.06.30", "channel": "stable", "applied_at": "2026-07-01T00:00:00Z"},
                {"revision": "rime-ice-2026.05.31"}
            ],
            "files": [
                {"path": "cn_dicts/a.yaml", "url": "https://example.com/a.yaml",
                 "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 1}
            ]
        }"#;
        let manifest = parse_manifest(bytes).unwrap();
        assert_eq!(manifest.version, 2);
        let history = manifest.history.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].revision, "rime-ice-2026.06.30");
        assert_eq!(history[0].channel.as_deref(), Some("stable"));
        assert!(history[1].channel.is_none());
    }

    #[test]
    fn manifest_v1_不带_history_保持兼容() {
        // version 1 老 manifest：无 history 字段照样可解析，其余行为不变
        let manifest =
            parse_manifest(include_bytes!("../../../schemas/rime-ice-2026.06.30.json")).unwrap();
        assert_eq!(manifest.version, 1);
        assert!(manifest.history.is_none());
    }

    #[test]
    fn manifest_v2_缺_history_字段也允许() {
        let bytes = br#"{
            "version": 2, "revision": "r1",
            "files": [{"path": "cn_dicts/a.yaml", "url": "https://example.com/a.yaml",
                       "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 1}]
        }"#;
        let manifest = parse_manifest(bytes).unwrap();
        assert!(manifest.history.is_none());
    }

    #[test]
    fn manifest_v2_history_拒绝空版本号与超限() {
        let base = r#""files": [{"path": "cn_dicts/a.yaml", "url": "https://example.com/a.yaml",
                       "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 1}]"#;
        let bad_revision = format!(
            r#"{{"version": 2, "revision": "r1",
             "history": [{{"revision": "  "}}], {base}}}"#
        );
        assert!(parse_manifest(bad_revision.as_bytes()).is_err());
        let too_many = format!(
            r#"{{"version": 2, "revision": "r1",
             "history": [{}], {base}}}"#,
            (0..65)
                .map(|i| format!(r#"{{"revision": "r{i}"}}"#))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(parse_manifest(too_many.as_bytes()).is_err());
    }

    #[test]
    fn 多代快照_可逐代回滚到任意历史() {
        let root =
            std::env::temp_dir().join(format!("shurufa-dict-multi-rb-{}", std::process::id()));
        let schema_dir = root.join("schemas");
        fs::create_dir_all(schema_dir.join("cn_dicts")).unwrap();
        fs::write(schema_dir.join("cn_dicts/existing.yaml"), b"v0").unwrap();

        // 依次更新到 r1 / r2 / r3（每次只替换 existing.yaml）
        for (new_rev, content) in [("r1", "v1"), ("r2", "v2"), ("r3", "v3")] {
            let applied = fake_applied(&schema_dir, &["cn_dicts/existing.yaml"], &[]);
            persist_revision_snapshot(&schema_dir, &applied, new_rev).unwrap();
            fs::write(schema_dir.join("cn_dicts/existing.yaml"), content).unwrap();
        }
        assert_eq!(read_current_revision(&schema_dir).as_deref(), Some("r3"));

        // rollback_to r1：应依次弹出 r2、r1 两代快照，最终 current = r1
        let restored = rollback_to(&schema_dir, "r1").unwrap();
        assert_eq!(restored.as_deref(), Some("r1"));
        assert_eq!(read_current_revision(&schema_dir).as_deref(), Some("r1"));

        // 快照栈只剩"内置"那一代；再回滚一次回到内置
        let stack = read_snapshot_stack(&schema_dir);
        assert_eq!(stack.len(), 1);
        assert_eq!(stack[0].revision, "");
        let restored = rollback_to(&schema_dir, "内置").unwrap();
        assert_eq!(restored.as_deref(), Some("内置"));
        assert_eq!(read_current_revision(&schema_dir), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rollback_to_不存在的_revision_报错且不动栈() {
        let root =
            std::env::temp_dir().join(format!("shurufa-dict-rb-miss-{}", std::process::id()));
        let schema_dir = root.join("schemas");
        fs::create_dir_all(schema_dir.join("cn_dicts")).unwrap();
        fs::write(schema_dir.join("cn_dicts/existing.yaml"), b"v0").unwrap();
        let applied = fake_applied(&schema_dir, &["cn_dicts/existing.yaml"], &[]);
        persist_revision_snapshot(&schema_dir, &applied, "r1").unwrap();

        assert!(rollback_to(&schema_dir, "no-such-rev").is_err());
        // 栈原封不动
        let stack = read_snapshot_stack(&schema_dir);
        assert_eq!(stack.len(), 1);
        assert_eq!(read_current_revision(&schema_dir).as_deref(), Some("r1"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 快照栈_fifo_压缩到五代() {
        let root = std::env::temp_dir().join(format!("shurufa-dict-fifo-{}", std::process::id()));
        let schema_dir = root.join("schemas");
        fs::create_dir_all(&schema_dir).unwrap();
        fs::write(schema_dir.join("a.yaml"), b"v0").unwrap();

        for i in 1..=8 {
            let applied = fake_applied(&schema_dir, &["a.yaml"], &[]);
            persist_revision_snapshot(&schema_dir, &applied, &format!("r{i}")).unwrap();
        }
        let stack = read_snapshot_stack(&schema_dir);
        assert_eq!(stack.len(), MAX_SNAPSHOTS);
        // 8 次更新的 previous 依次是 ["", "r1", .., "r7"]；只保留最近 5 代 = "r3"..="r7"
        assert_eq!(stack[0].revision, "r3");
        assert_eq!(stack.last().unwrap().revision, "r7");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v1布局备份自动迁移后可继续回滚() {
        // v1 布局：.backup 根目录直接放替换前的文件 + .added + .previous-revision
        let root = std::env::temp_dir().join(format!("shurufa-dict-v1-mig-{}", std::process::id()));
        let schema_dir = root.join("schemas");
        let backup = root.join("schemas/.backup");
        fs::create_dir_all(backup.join("cn_dicts")).unwrap();
        fs::create_dir_all(schema_dir.join("cn_dicts")).unwrap();
        fs::write(schema_dir.join("cn_dicts/existing.yaml"), b"v1").unwrap();
        fs::write(backup.join("cn_dicts/existing.yaml"), b"v0").unwrap();
        fs::write(backup.join(".added"), "").unwrap();
        fs::write(backup.join(".previous-revision"), "").unwrap();
        fs::write(schema_dir.join(".current-revision"), "r1").unwrap();

        // v1 布局下 rollback 依然按"回滚一代"工作
        let restored = rollback(&schema_dir).unwrap();
        assert_eq!(restored.as_deref(), Some("内置"));
        assert_eq!(read_current_revision(&schema_dir), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 回滚后可继续向更高版本update() {
        // 回滚只动 .current-revision 与快照栈；update_from_url 的新 revision 只来自
        // manifest，不做层级比较，因此回滚到旧版后再 update 更高版不会死锁。
        let root = std::env::temp_dir().join(format!("shurufa-dict-rb-fwd-{}", std::process::id()));
        let schema_dir = root.join("schemas");
        fs::create_dir_all(&schema_dir).unwrap();
        fs::write(schema_dir.join("a.yaml"), b"v0").unwrap();

        for new_rev in ["r1", "r2"] {
            let applied = fake_applied(&schema_dir, &["a.yaml"], &[]);
            persist_revision_snapshot(&schema_dir, &applied, new_rev).unwrap();
        }
        assert_eq!(read_current_revision(&schema_dir).as_deref(), Some("r2"));

        // 回滚一代到 r1，再模拟更新到 r3：previous 快照应记录 r1
        let restored = rollback(&schema_dir).unwrap();
        assert_eq!(restored.as_deref(), Some("r1"));
        let applied = fake_applied(&schema_dir, &["a.yaml"], &[]);
        persist_revision_snapshot(&schema_dir, &applied, "r3").unwrap();
        assert_eq!(read_current_revision(&schema_dir).as_deref(), Some("r3"));
        // 栈里应有 [内置, r1] 两代；再回滚一代能回 r1
        let stack = read_snapshot_stack(&schema_dir);
        assert_eq!(stack.len(), 2);
        let restored = rollback_to(&schema_dir, "r1").unwrap();
        assert_eq!(restored.as_deref(), Some("r1"));
        fs::remove_dir_all(root).unwrap();
    }
}
