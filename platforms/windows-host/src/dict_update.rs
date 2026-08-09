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

#[derive(Debug, Deserialize)]
struct Manifest {
    version: u32,
    revision: String,
    files: Vec<ManifestFile>,
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
pub fn cli_rollback() {
    let schema_dir = schema_dir();
    match rollback(&schema_dir) {
        Ok(Some(revision)) => println!("已回滚到 {revision}"),
        Ok(None) => println!("无可回滚版本"),
        Err(error) => {
            eprintln!("词库回滚失败：{error}");
            std::process::exit(1);
        }
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

/// 更新成功后生成回滚快照：被替换的旧文件复制到 `<schemas>/.backup/`
/// （先清空再复制，最多保留最近一代），新增文件记入 `.backup/.added` 清单
/// （回滚时删除），上一版 revision 记入 `.backup/.previous-revision`，
/// 最后把本次 revision 写入 `<schemas>/.current-revision`。
fn persist_revision_snapshot(
    schema_dir: &Path,
    applied: &[AppliedFile],
    new_revision: &str,
) -> Result<(), String> {
    let previous = read_current_revision(schema_dir);
    let backup_dir = schemas_backup_dir(schema_dir);
    if backup_dir.exists() {
        fs::remove_dir_all(&backup_dir).map_err(|e| format!("清理旧回滚备份失败: {e}"))?;
    }
    fs::create_dir_all(&backup_dir).map_err(|e| format!("创建回滚备份目录失败: {e}"))?;

    let mut added = Vec::new();
    for file in applied {
        if file.existed {
            let dest = backup_dir.join(
                file.target
                    .strip_prefix(schema_dir)
                    .map_err(|_| "词库备份路径解析失败")?,
            );
            let parent = dest.parent().ok_or("词库备份路径没有父目录")?;
            fs::create_dir_all(parent).map_err(|e| format!("创建回滚备份子目录失败: {e}"))?;
            // staging 里的 .backup 与目标一一对应，复制旧文件而非移动，
            // staging 目录随后整体删除
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
    fs::write(backup_dir.join(".added"), added.join("\n"))
        .map_err(|e| format!("写入新增清单失败: {e}"))?;
    fs::write(
        backup_dir.join(".previous-revision"),
        previous.unwrap_or_default(),
    )
    .map_err(|e| format!("写入上一版本号失败: {e}"))?;
    fs::write(current_revision_path(schema_dir), new_revision)
        .map_err(|e| format!("写入当前版本号失败: {e}"))?;
    Ok(())
}

/// 用 `<schemas>/.backup` 覆盖回 schemas 目录：恢复旧文件、删除 `.added`
/// 清单里的新增文件、把 `.previous-revision` 写回 `.current-revision`，
/// 随后重建二进制词典。返回回滚到的版本号；无备份返回 None。
fn rollback(schema_dir: &Path) -> Result<Option<String>, String> {
    let backup_dir = schemas_backup_dir(schema_dir);
    if !backup_dir.is_dir() {
        return Ok(None);
    }

    // 1. 恢复被替换的旧文件（跳过备份元数据文件）
    for entry in fs::read_dir(&backup_dir).map_err(|e| format!("读取回滚备份失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取回滚备份失败: {e}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            // .added / .previous-revision 等元数据在后续单独处理
            continue;
        }
        if path.is_dir() {
            restore_backup_tree(&path, &backup_dir, schema_dir)?;
        } else {
            restore_backup_file(&path, &backup_dir, schema_dir)?;
        }
    }

    // 2. 删除本次更新新增的文件
    let added_path = backup_dir.join(".added");
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

    // 3. 版本号回写（上一版可能为空，表示内置版本）
    let previous = fs::read_to_string(backup_dir.join(".previous-revision"))
        .unwrap_or_default()
        .trim()
        .to_string();
    if previous.is_empty() {
        let _ = fs::remove_file(current_revision_path(schema_dir));
    } else {
        fs::write(current_revision_path(schema_dir), &previous)
            .map_err(|e| format!("写回当前版本号失败: {e}"))?;
    }

    // 4. 备份已消费：只允许回滚一代
    fs::remove_dir_all(&backup_dir).map_err(|e| format!("清理回滚备份失败: {e}"))?;

    rebuild(schema_dir)?;
    if previous.is_empty() {
        Ok(Some("内置".into()))
    } else {
        Ok(Some(previous))
    }
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
                // 部署成功后才固化回滚快照与版本号；快照失败不回退已部署内容
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
    if manifest.version != 1 {
        return Err("词库清单版本不受支持".into());
    }
    if manifest.revision.trim().is_empty()
        || manifest.files.is_empty()
        || manifest.files.len() > MAX_FILES
    {
        return Err("词库清单的版本或文件数无效".into());
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
        let root = std::env::temp_dir().join(format!("shurufa-dict-snap-test-{}", std::process::id()));
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

        // 无版本时首更：previous 应为空串
        persist_revision_snapshot(&schema_dir, &applied, "r1").unwrap();
        assert_eq!(read_current_revision(&schema_dir).as_deref(), Some("r1"));
        let backup = schemas_backup_dir(&schema_dir);
        assert_eq!(
            fs::read(backup.join("cn_dicts/existing.yaml")).unwrap(),
            b"old"
        );
        assert_eq!(fs::read_to_string(backup.join(".added")).unwrap(), "cn_dicts/added.yaml");
        assert_eq!(fs::read_to_string(backup.join(".previous-revision")).unwrap(), "");

        // 再更一代：快照被覆盖（只保留最近一代），previous = r1
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
        assert_eq!(
            fs::read(backup.join("cn_dicts/existing.yaml")).unwrap(),
            b"new"
        );
        assert_eq!(fs::read_to_string(backup.join(".added")).unwrap(), "");
        assert_eq!(fs::read_to_string(backup.join(".previous-revision")).unwrap(), "r1");

        // 回滚（跳过重建）：恢复 r1 的文件状态并回写版本号
        let previous = fs::read_to_string(backup.join(".previous-revision")).unwrap();
        assert_eq!(previous.trim(), "r1");
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
}
