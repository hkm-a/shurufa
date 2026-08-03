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
            Ok(()) => Ok(manifest.revision.clone()),
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
}
