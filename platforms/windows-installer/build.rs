//! FOX 安装器 build.rs：将安装 payload（运行时二进制 + rime + schemas + 部署脚本）
//! 打包成一个 blob 嵌入安装器本体（自包含安装包）。
//!
//! 仅 release 构建（或 FOX_EMBED_PAYLOAD=1）嵌入；debug 构建生成空清单，
//! 引擎走"模拟进度"模式，便于 UI 迭代。产物写到 OUT_DIR：
//!   payload.blob          所有文件的字节拼接
//!   payload_manifest.rs   (dest 路径, offset, len) 清单 + 版本

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    tauri_build::build();

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .expect("仓库根")
        .parent()
        .expect("仓库根");

    let embed = env::var("PROFILE").unwrap_or_default() == "release"
        || env::var("FOX_EMBED_PAYLOAD").as_deref() == Ok("1");
    if !embed {
        fs::write(
            out_dir.join("payload_manifest.rs"),
            "pub struct PayloadFile { pub dest: &'static str, pub offset: usize, pub len: usize }\n\
             pub static PAYLOAD_FILES: &[PayloadFile] = &[];\n\
             pub static PAYLOAD_BYTES: &[u8] = &[];\n\
             pub static PAYLOAD_VERSION: &str = \"dev\";\n",
        )
        .expect("写空清单");
        return;
    }

    // 版本（从仓库根 version.json 提取，用于 TSF DLL 版本化命名）
    let vtext = fs::read_to_string(root.join("version.json")).expect("version.json");
    let version = vtext
        .split("\"version\"")
        .nth(1)
        .and_then(|s| s.split('"').nth(1))
        .unwrap_or("0.0.0")
        .to_string();

    // 文件清单：(源路径, 安装后相对路径)
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut add = |src: &str, dest: &str| files.push((root.join(src), dest.to_string()));
    add(
        "target/release/shurufa_tsf.dll",
        &format!("shurufa_tsf-{version}.dll"),
    );
    add("target/release/shurufa-algo.exe", "shurufa-algo.exe");
    add("target/release/shurufa-host.exe", "shurufa-host.exe");
    add("target/release/Shurufa.exe", "Shurufa.exe");
    add("third_party/librime/dist/lib/rime.dll", "rime.dll");
    add(
        "third_party/librime/dist/bin/rime_deployer.exe",
        "rime_deployer.exe",
    );
    add(
        "installer/activate-default-ime.ps1",
        "activate-default-ime.ps1",
    );
    add(
        "installer/register-host-startup.ps1",
        "register-host-startup.ps1",
    );
    add("installer/verify-install.ps1", "verify-install.ps1");
    // 上面三个脚本都点源 Deploy-Shurufa.ps1（共享 GUID/注册函数），必须一并打入
    add("installer/Deploy-Shurufa.ps1", "Deploy-Shurufa.ps1");
    collect_schemas(&root.join("schemas"), "", &mut files);

    // 拼接 blob
    let mut blob: Vec<u8> = Vec::new();
    let mut manifest_lines = Vec::new();
    for (src, dest) in &files {
        let bytes =
            fs::read(src).unwrap_or_else(|e| panic!("读取 payload {} 失败：{e}", src.display()));
        let offset = blob.len();
        let len = bytes.len();
        blob.extend_from_slice(&bytes);
        manifest_lines.push(format!(
            "    PayloadFile {{ dest: \"{dest}\", offset: {offset}, len: {len} }},"
        ));
        println!("cargo:rerun-if-changed={}", src.display());
    }

    fs::write(out_dir.join("payload.blob"), &blob).expect("写 payload.blob");
    let manifest = format!(
        "pub struct PayloadFile {{ pub dest: &'static str, pub offset: usize, pub len: usize }}\n\
         pub static PAYLOAD_FILES: &[PayloadFile] = &[\n{}\n];\n\
         pub static PAYLOAD_BYTES: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/payload.blob\"));\n\
         pub static PAYLOAD_VERSION: &str = \"{version}\";\n",
        manifest_lines.join("\n")
    );
    fs::write(out_dir.join("payload_manifest.rs"), manifest).expect("写 payload_manifest.rs");
    println!(
        "cargo:warning=已嵌入 {} 个 payload 文件（{} 字节）",
        files.len(),
        blob.len()
    );
}

fn collect_schemas(dir: &Path, prefix: &str, out: &mut Vec<(PathBuf, String)>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            collect_schemas(&path, &format!("{prefix}{name}/"), out);
        } else {
            out.push((path, format!("schemas/{prefix}{name}")));
        }
    }
}
