//! Shurufa Tauri 控制中心后端。
//!
//! 前端只负责界面与交互；中继配置、后台宿主和词库更新仍沿用既有 Rust 模块与
//! 同目录可执行文件，避免为桌面 UI 再建立平行的业务实现。

#![cfg(windows)]
#![windows_subsystem = "windows"]

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use clipboard_store::{ClipEntry, ClipKind, ClipboardStore};
use serde::{Deserialize, Serialize};
use shurufa_options::ImeOptions;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(feature = "ui-e2e")]
fn e2e_trace(message: &str) {
    use std::io::Write;
    let directory = app_data_dir();
    let _ = std::fs::create_dir_all(&directory);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("ui-e2e-trace.log"))
    {
        let _ = writeln!(file, "{message}");
    }
}

#[cfg(feature = "ui-e2e")]
const UI_E2E_SCRIPT: &str = r##"
(() => {
  if (window.__SHURUFA_UI_E2E_RUNNING__) return;
  window.__SHURUFA_UI_E2E_RUNNING__ = true;

  const delay = (milliseconds) => new Promise((resolve) => window.setTimeout(resolve, milliseconds));
  const waitFor = async (predicate, label) => {
    for (let attempt = 0; attempt < 100; attempt += 1) {
      if (predicate()) return;
      await delay(100);
    }
    throw new Error(`${label} 超时`);
  };
  const results = [];
  const calls = [];
  const runAction = async (page, action, command) => {
    const navigation = document.querySelector(`[data-page="${page}"]`);
    if (!navigation) throw new Error(`缺少 ${page} 导航按钮`);
    navigation.click();
    await waitFor(() => document.querySelector(`[data-action="${action}"]`) !== null, `${action} 按钮出现`);
    const firstCall = calls.length;
    const button = document.querySelector(`[data-action="${action}"]`);
    button.click();
    await waitFor(() => calls.slice(firstCall).some((call) => call.command === command), `${action} command`);
    const call = calls.slice(firstCall).find((item) => item.command === command);
    await Promise.race([
      call.completion,
      delay(5000).then(() => { throw new Error(`${action} command 超时`); })
    ]);
    results.push(action);
  };
  const report = (message) => {
    document.title = message;
    const node = document.createElement("pre");
    node.id = "ui-e2e-result";
    node.textContent = message;
    document.body.append(node);
  };

  window.setTimeout(async () => {
    try {
      const internals = window.__TAURI_INTERNALS__;
      if (!internals || typeof internals.invoke !== "function") {
        throw new Error("Tauri IPC bridge 尚未就绪");
      }
      const originalInvoke = internals.invoke.bind(internals);
      internals.invoke = (...args) => {
        const call = { command: args[0], completion: null };
        call.completion = Promise.resolve(originalInvoke(...args));
        calls.push(call);
        return call.completion;
      };
      await Promise.race([
        internals.invoke("dashboard_state"),
        delay(3000).then(() => { throw new Error("dashboard_state IPC 超时"); })
      ]);
      results.push("dashboard_state");
      await Promise.race([
        internals.invoke("e2e_ping"),
        delay(3000).then(() => { throw new Error("e2e_ping IPC 超时"); })
      ]);
      results.push("e2e_ping");
      await runAction("settings", "open-data-directory", "open_data_directory");
      await runAction("input", "open-settings", "open_system_settings");
      await runAction("dictionary", "update-dictionary", "update_dictionary");
      await runAction("history", "clear-history", "clear_unpinned_history");
      await runAction("workspace", "stop-service", "stop_service");
      report(`UI E2E PASS: ${results.join(",")}`);
    } catch (error) {
      report(`UI E2E FAIL: ${String(error)}`);
    }
  }, 800);
})();
"##;

#[derive(Serialize)]
struct DashboardState {
    relay: String,
    service_status: String,
    data_directory: String,
}

#[derive(Serialize)]
struct HistoryEntry {
    id: i64,
    kind: String,
    text: String,
    source_app: String,
    updated_at: i64,
    pinned: bool,
}

fn app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
}

fn sync_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("SHURUFA_SYNC_DIR") {
        PathBuf::from(path)
    } else {
        app_data_dir().join("sync")
    }
}

fn history_db_path() -> PathBuf {
    std::env::var_os("SHURUFA_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| app_data_dir().join("clipboard.db"))
}

fn open_history_store() -> Result<ClipboardStore, String> {
    ClipboardStore::open(&history_db_path()).map_err(|error| format!("打开剪贴板历史失败：{error}"))
}

fn history_entry(entry: ClipEntry) -> HistoryEntry {
    let kind = match entry.kind {
        ClipKind::Text => "文本",
        ClipKind::Image => "图片",
        ClipKind::Files => "文件",
    };
    let text = match entry.kind {
        ClipKind::Image => format!("图片（{} KB）", (entry.data_size.max(0) + 1023) / 1024),
        ClipKind::Files => entry.text.lines().next().unwrap_or("文件").to_owned(),
        ClipKind::Text => entry.text,
    };
    HistoryEntry {
        id: entry.id,
        kind: kind.to_owned(),
        text,
        source_app: entry.source_app,
        updated_at: entry.updated_at,
        pinned: entry.pinned,
    }
}

fn sibling_exe(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

async fn run_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("后台操作异常中断：{error}"))?
}

async fn launch_host(args: &[&str]) -> Result<String, String> {
    let executable = sibling_exe("shurufa-host.exe");
    let arguments: Vec<String> = args.iter().map(|argument| (*argument).to_owned()).collect();
    run_blocking(move || {
        Command::new(executable)
            .args(arguments)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| "后台宿主已启动".to_owned())
            .map_err(|error| format!("无法启动后台宿主：{error}"))
    })
    .await
}

fn read_service_status() -> String {
    let state = std::fs::read_to_string(app_data_dir().join("daemon.state")).unwrap_or_default();
    if state.contains("status=running") {
        "运行中".to_owned()
    } else if state.contains("status=restarting") {
        "正在恢复".to_owned()
    } else {
        "待启动".to_owned()
    }
}

#[tauri::command]
fn dashboard_state() -> DashboardState {
    DashboardState {
        relay: sync_core::load_relay_addr(&sync_dir()).unwrap_or_default(),
        service_status: read_service_status(),
        data_directory: app_data_dir().display().to_string(),
    }
}

#[tauri::command]
fn e2e_ping() -> String {
    #[cfg(feature = "ui-e2e")]
    e2e_trace("e2e_ping: 接收调用并返回");
    "pong".to_owned()
}

#[tauri::command]
fn save_relay(relay: String) -> Result<(), String> {
    let relay = relay.trim();
    let relay = if relay.is_empty() {
        None
    } else {
        // 白名单格式：仅允许 host:port（域名/IPv4字面量/[IPv6]:port）。
        // 拒绝其他字符避免 CRLF/路径/协议头注入到下层持久化与 TLS 连接。
        let (host, port_str) = relay.rsplit_once(':')
            .ok_or_else(|| "中继地址格式无效：应为 host:port".to_owned())?;
        if host.is_empty() {
            return Err("主机不能为空".to_owned());
        }
        let host_body = host.trim_start_matches('[').trim_end_matches(']');
        let host_valid = !host_body.is_empty()
            && host_body
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
            && !host_body.starts_with('-')
            && !host_body.ends_with('-');
        if !host_valid {
            return Err("主机名含非法字符：仅允许字母数字 . - :".to_owned());
        }
        // 端口 1..=65535
        if port_str.is_empty() || port_str.len() > 5 || !port_str.chars().all(|c| c.is_ascii_digit()) {
            return Err("端口必须是 1-5 位数字".to_owned());
        }
        let port: u32 = port_str.parse().map_err(|_| "端口必须是数字".to_owned())?;
        if port == 0 || port > 65535 {
            return Err("端口必须在 1..=65535".to_owned());
        }
        Some(relay)
    };
    sync_core::save_relay_addr(&sync_dir(), relay)
        .map_err(|error| format!("保存中继设置失败：{error}"))
}

#[tauri::command]
async fn start_service() -> Result<String, String> {
    launch_host(&["supervise"]).await
}

#[tauri::command]
async fn stop_service() -> Result<String, String> {
    launch_host(&["stop"]).await
}

#[tauri::command]
async fn update_dictionary() -> Result<String, String> {
    launch_host(&["dict-update", "rime-ice"]).await
}

#[tauri::command]
async fn open_system_settings() -> Result<String, String> {
    run_blocking(|| {
        Command::new("cmd.exe")
            .args(["/c", "start", "", "ms-settings:regionlanguage"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| "已请求打开系统设置".to_owned())
            .map_err(|error| format!("打开系统设置失败：{error}"))
    })
    .await
}

#[tauri::command]
async fn open_data_directory() -> Result<String, String> {
    let directory = app_data_dir();
    #[cfg(feature = "ui-e2e")]
    e2e_trace("open_data_directory: 接收调用");
    let result = run_blocking(move || {
        #[cfg(feature = "ui-e2e")]
        e2e_trace("open_data_directory: 开始创建目录");
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("创建数据目录失败：{error}"))?;
        #[cfg(feature = "ui-e2e")]
        e2e_trace("open_data_directory: 开始启动资源管理器");
        Command::new("explorer.exe")
            .arg(&directory)
            .spawn()
            .map(|_| {
                #[cfg(feature = "ui-e2e")]
                e2e_trace("open_data_directory: 资源管理器已启动");
                directory.display().to_string()
            })
            .map_err(|error| format!("打开数据目录失败：{error}"))
    })
    .await;
    #[cfg(feature = "ui-e2e")]
    e2e_trace(if result.is_ok() {
        "open_data_directory: 完成返回"
    } else {
        "open_data_directory: 返回错误"
    });
    result
}

#[tauri::command]
fn history_entries() -> Result<Vec<HistoryEntry>, String> {
    open_history_store()?
        .list(80, 0)
        .map(|entries| entries.into_iter().map(history_entry).collect())
        .map_err(|error| format!("读取剪贴板历史失败：{error}"))
}

#[tauri::command]
async fn copy_history(id: i64) -> Result<String, String> {
    let id = id.to_string();
    launch_host(&["copy", &id]).await
}

#[tauri::command]
fn set_history_pinned(id: i64, pinned: bool) -> Result<(), String> {
    let updated = open_history_store()?
        .set_pinned(id, pinned)
        .map_err(|error| format!("更新置顶状态失败：{error}"))?;
    if updated {
        Ok(())
    } else {
        Err("该历史条目已不存在".to_owned())
    }
}

#[tauri::command]
fn delete_history(id: i64) -> Result<(), String> {
    let deleted = open_history_store()?
        .delete(id)
        .map_err(|error| format!("删除历史条目失败：{error}"))?;
    if deleted {
        Ok(())
    } else {
        Err("该历史条目已不存在".to_owned())
    }
}

#[tauri::command]
fn clear_unpinned_history() -> Result<usize, String> {
    open_history_store()?
        .clear_unpinned()
        .map_err(|error| format!("清空未置顶历史失败：{error}"))
}

/// 输入法四项快捷键选项（options.json 单一事实源，TSF 端 2 秒内热加载）。
#[derive(Serialize, Deserialize)]
struct ImeOptionsDto {
    shift_switch_cn_en: bool,
    shift_space_full_shape: bool,
    ctrl_period_ascii_punct: bool,
    capslock_to_english: bool,
}

impl From<ImeOptions> for ImeOptionsDto {
    fn from(o: ImeOptions) -> Self {
        Self {
            shift_switch_cn_en: o.shift_switch_cn_en,
            shift_space_full_shape: o.shift_space_full_shape,
            ctrl_period_ascii_punct: o.ctrl_period_ascii_punct,
            capslock_to_english: o.capslock_to_english,
        }
    }
}

impl From<ImeOptionsDto> for ImeOptions {
    fn from(d: ImeOptionsDto) -> Self {
        Self {
            shift_switch_cn_en: d.shift_switch_cn_en,
            shift_space_full_shape: d.shift_space_full_shape,
            ctrl_period_ascii_punct: d.ctrl_period_ascii_punct,
            capslock_to_english: d.capslock_to_english,
        }
    }
}

#[tauri::command]
fn ime_options() -> ImeOptionsDto {
    shurufa_options::load().into()
}

#[tauri::command]
fn save_ime_options(opts: ImeOptionsDto) -> Result<(), String> {
    shurufa_options::save(&opts.into()).map_err(|error| format!("保存输入选项失败：{error}"))
}

/// 打字统计面板 DTO：四项合计 + 最近 8 天字符数序列。
#[derive(Serialize, Clone)]
struct TypingStatsDto {
    total_chars: u64,
    today_chars: u64,
    total_keys: u64,
    today_keys: u64,
    /// (日期 "MM-DD", 字符数)，最近 8 天
    days: Vec<(String, u64)>,
}

#[tauri::command]
fn typing_stats() -> TypingStatsDto {
    let totals = shurufa_options::stats::totals();
    // "2026-08-08" -> "08-08"：截取月日短格式，兼容非法输入原样透出
    let days = shurufa_options::stats::last_days(8)
        .into_iter()
        .map(|(date, chars)| {
            let short = date.get(5..).map(str::to_owned).unwrap_or(date);
            (short, chars)
        })
        .collect();
    TypingStatsDto {
        total_chars: totals.total_chars,
        today_chars: totals.today_chars,
        total_keys: totals.total_keys,
        today_keys: totals.today_keys,
        days,
    }
}

#[derive(Serialize)]
struct DictionaryInfo {
    revision: String,
}

async fn run_host_capture(args: &[&str]) -> Result<String, String> {
    let executable = sibling_exe("shurufa-host.exe");
    let arguments: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    run_blocking(move || {
        let output = Command::new(executable)
            .args(arguments)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|error| format!("无法启动后台宿主：{error}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if output.status.success() {
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            Err(if stderr.is_empty() {
                format!("后台宿主退出码 {:?}：{}", output.status.code(), stdout)
            } else {
                format!("后台宿主退出码 {:?}：{}", output.status.code(), stderr)
            })
        }
    })
    .await
}

/// 当前词典 revision（无记录时为"内置"）。
#[tauri::command]
async fn dictionary_info() -> Result<DictionaryInfo, String> {
    let revision = run_host_capture(&["dict-current"]).await?;
    Ok(DictionaryInfo {
        revision: if revision.is_empty() { "内置".to_owned() } else { revision },
    })
}

/// 列出本地可回滚的历史版本（最近的在前， dict-history 每个 revision 一行）。
#[tauri::command]
async fn dictionary_history() -> Result<Vec<String>, String> {
    let stdout = run_host_capture(&["dict-history"]).await?;
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('（'))
        .map(str::to_owned)
        .collect())
}

/// 回滚到上一代词典并重建。
#[tauri::command]
async fn rollback_dictionary() -> Result<String, String> {
    run_host_capture(&["dict-rollback"]).await
}

/// 回滚到指定 revision 的词典并重建（revision 须存在于本地快照栈）。
#[tauri::command]
async fn rollback_dictionary_to(revision: String) -> Result<String, String> {
    let revision = revision.trim().to_owned();
    if revision.is_empty() {
        return Err("目标版本不能为空".to_owned());
    }
    run_host_capture(&["dict-rollback", "--revision", &revision]).await
}

// ---------------------------------------------------------------------------
// 皮肤文件读写（skin 编辑器）：用户覆盖版与内置版合并供 UI 编辑
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SkinPayload {
    /// %APPDATA%\shurufa\shurufa-skin.json 内容；不存在则回退到 exe 旁
    /// schemas/shurufa-skin.json；两者都无 → None（UI 显示空模板）
    content: Option<String>,
    /// 用户覆盖文件路径；UI 保存按钮直接写这里
    user_path: String,
    /// 内容来源标记：User 覆盖 / Builtin 内置 / None 都无
    source: String,
}

#[tauri::command]
fn skin_payload() -> Result<SkinPayload, String> {
    let user_path = app_data_dir().join("shurufa-skin.json");
    if let Ok(text) = std::fs::read_to_string(&user_path) {
        return Ok(SkinPayload {
            content: Some(text),
            user_path: user_path.display().to_string(),
            source: "user".to_owned(),
        });
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let builtin = dir.join("schemas").join("shurufa-skin.json");
            if let Ok(text) = std::fs::read_to_string(&builtin) {
                return Ok(SkinPayload {
                    content: Some(text),
                    user_path: user_path.display().to_string(),
                    source: "builtin".to_owned(),
                });
            }
        }
    }
    Ok(SkinPayload {
        content: None,
        user_path: user_path.display().to_string(),
        source: "none".to_owned(),
    })
}

#[tauri::command]
fn save_skin(content: String) -> Result<(), String> {
    // 基本完整性检查：必须是合法 JSON 且 version 字段为 1 或 2。
    let value: serde_json::Value =
        serde_json::from_str(&content).map_err(|error| format!("JSON 无效：{error}"))?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "缺少 version 字段".to_owned())?;
    if version != 1 && version != 2 {
        return Err(format!("version 仅支持 1 或 2，当前为 {version}"));
    }
    let path = app_data_dir().join("shurufa-skin.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建目录失败：{error}"))?;
    }
    std::fs::write(&path, content).map_err(|error| format!("写入失败：{error}"))
}

#[tauri::command]
fn reset_skin() -> Result<(), String> {
    let path = app_data_dir().join("shurufa-skin.json");
    match std::fs::remove_file(&path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("删除失败：{error}")),
    }
}

// ---------------------------------------------------------------------------
// 预设皮肤包（skins/ 目录 + schemas/skins-index.json 本地清单）
// ---------------------------------------------------------------------------

/// skins-index.json 中单个预设条目（与 schemas/skins-index.json 一致）。
#[derive(Serialize, Deserialize, Clone)]
struct SkinMeta {
    id: String,
    name_zh: String,
    name_en: String,
    file: String,
    author: String,
    #[serde(default)]
    tags: Vec<String>,
    preview_hint: String,
}

/// 与 `skin_payload` 相同的查找顺序：用户覆盖目录 %APPDATA%\shurufa\ 优先，
/// 然后 exe 旁 schemas/。name 为相对文件名（"skins-index.json" 或 "../skins/x.json"）。
fn resolve_skins_meta_path(name: &str) -> Option<PathBuf> {
    let user = app_data_dir().join(name);
    if user.is_file() {
        return Some(user);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let builtin = dir.join("schemas").join(name);
            if builtin.is_file() {
                return Some(builtin);
            }
        }
    }
    None
}

#[tauri::command]
fn list_skins() -> Result<Vec<SkinMeta>, String> {
    let path = resolve_skins_meta_path("skins-index.json")
        .ok_or_else(|| "未找到 skins-index.json".to_owned())?;
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("读取清单失败：{error}"))?;
    serde_json::from_str::<Vec<SkinMeta>>(&text).map_err(|error| format!("清单 JSON 无效：{error}"))
}

/// 应用预设：把 skins/<file> 写入用户覆盖路径（SSOT 文件更新），
/// 候选窗下次弹出（每次 show 都重读皮肤）即生效；工厂默认不被修改。
#[tauri::command]
fn apply_skin(id: String) -> Result<(), String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("皮肤 id 不能为空".to_owned());
    }
    let index_path = resolve_skins_meta_path("skins-index.json")
        .ok_or_else(|| "未找到 skins-index.json".to_owned())?;
    let index_text = std::fs::read_to_string(&index_path)
        .map_err(|error| format!("读取清单失败：{error}"))?;
    let metas: Vec<SkinMeta> = serde_json::from_str(&index_text)
        .map_err(|error| format!("清单 JSON 无效：{error}"))?;
    let meta = metas
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("清单中不存在 id：{id}"))?;
    // 文件名白名单：仅允许简单相对文件名，防路径穿越
    if meta.file.contains("..") || meta.file.contains('\\') || meta.file.contains('/') {
        return Err(format!("清单项 file 不允许路径分隔符：{}", meta.file));
    }
    // 皮肤文件先查 index 旁 skins/ 目录（开发期 repo 布局 <index>/../skins/），再退回 exe 旁 skins/
    let index_dir = index_path.parent().map(|p| p.to_path_buf());
    let skin_path = index_dir
        .clone()
        .and_then(|d| {
            let p = d.join("..").join("skins").join(&meta.file);
            if p.is_file() { Some(p) } else { None }
        })
        .or_else(|| {
            index_dir.and_then(|d| {
                let p = d.join("skins").join(&meta.file);
                if p.is_file() { Some(p) } else { None }
            })
        })
        .or_else(|| {
            std::env::current_exe().ok().and_then(|exe| {
                exe.parent().and_then(|d| {
                    let p = d.join("skins").join(&meta.file);
                    if p.is_file() { Some(p) } else { None }
                })
            })
        })
        .ok_or_else(|| format!("未找到皮肤文件：{}", meta.file))?;
    let content = std::fs::read_to_string(&skin_path)
        .map_err(|error| format!("读取皮肤文件失败：{error}"))?;
    // 与 save_skin 相同的完整性检查（version ∈ {1,2}）
    save_skin(content)
}

fn main() {
    let builder = tauri::Builder::default().invoke_handler(tauri::generate_handler![
        dashboard_state,
        e2e_ping,
        save_relay,
        start_service,
        stop_service,
        update_dictionary,
        open_system_settings,
        open_data_directory,
        history_entries,
        copy_history,
        set_history_pinned,
        delete_history,
        clear_unpinned_history,
        ime_options,
        save_ime_options,
        typing_stats,
        dictionary_info,
        dictionary_history,
        rollback_dictionary,
        rollback_dictionary_to,
        skin_payload,
        save_skin,
        reset_skin,
        list_skins,
        apply_skin
    ]);
    #[cfg(feature = "ui-e2e")]
    let builder = builder.on_page_load(|webview, payload| {
        e2e_trace("页面加载回调已触发");
        if std::env::var_os("SHURUFA_UI_E2E_DISABLE").is_some() {
            e2e_trace("页面自测脚本已按环境变量禁用");
            return;
        }
        if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
            if let Err(error) = webview.eval(UI_E2E_SCRIPT) {
                e2e_trace(&format!("页面自测脚本注入失败：{error}"));
            } else {
                e2e_trace("页面自测脚本已注入");
            }
        }
    });
    builder
        .run(tauri::generate_context!())
        .expect("启动 Shurufa 控制中心失败");
}

#[cfg(test)]
mod tests {
    use super::{history_entry, sync_dir, TypingStatsDto};
    use clipboard_store::{ClipEntry, ClipKind};

    #[test]
    fn 默认同步目录位于应用数据目录下() {
        assert!(sync_dir().ends_with("shurufa\\sync"));
    }

    #[test]
    fn 打字统计序列化含天数序列与今日计数() {
        let dto = TypingStatsDto {
            total_chars: 12345,
            today_chars: 67,
            total_keys: 8901,
            today_keys: 23,
            days: vec![("08-07".to_owned(), 40), ("08-08".to_owned(), 67)],
        };
        let value = serde_json::to_value(&dto).expect("DTO 序列化失败");
        assert!(value.get("days").is_some());
        assert_eq!(value["today_chars"], serde_json::json!(67_u64));
        assert_eq!(value["days"][1][0], serde_json::json!("08-08"));
    }

    #[test]
    fn 图片历史显示数据大小而非空文本() {
        let entry = history_entry(ClipEntry {
            id: 1,
            kind: ClipKind::Image,
            text: String::new(),
            source_app: "pixpin.exe".to_owned(),
            created_at: 0,
            updated_at: 1,
            use_count: 1,
            pinned: false,
            data_size: 1025,
        });
        assert_eq!(entry.kind, "图片");
        assert_eq!(entry.text, "图片（2 KB）");
    }

    #[test]
    fn 预设清单结构可反序列化() {
        // 与 schemas/skins-index.json 保持一致的字段形态；防后续手改清单破坏编译期固化的字段名
        let text = r##"[{"id":"mist","name_zh":"雾灰蓝","name_en":"Mist","file":"mist.json","author":"Shurufa","tags":["light"],"preview_hint":"#F7FAFC"}]"##;
        let metas: Vec<super::SkinMeta> = serde_json::from_str(text).expect("skins-index 行可反序列化");
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, "mist");
        assert_eq!(metas[0].tags, vec!["light".to_owned()]);
        assert_eq!(metas[0].preview_hint, "#F7FAFC");
    }

    #[test]
    fn 预设皮肤文件均通过_version_校验() {
        // 仓库内 5 套预设 + 索引都在源码树内：直接相对校验
        let here = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = here.parent().and_then(|p| p.parent()).expect("repo root");
        let index_path = root.join("schemas").join("skins-index.json");
        let text = std::fs::read_to_string(&index_path).expect("skins-index.json 存在");
        let metas: Vec<super::SkinMeta> = serde_json::from_str(&text).expect("skins-index.json 合法");
        assert_eq!(metas.len(), 5, "本期上线 5 套预设");
        for meta in &metas {
            let file = root.join("skins").join(&meta.file);
            let content = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("读取 {} 失败: {}", file.display(), e));
            let value: serde_json::Value = serde_json::from_str(&content)
                .unwrap_or_else(|e| panic!("{} JSON 无效: {}", meta.file, e));
            let version = value
                .get("version")
                .and_then(serde_json::Value::as_u64)
                .expect("version 存在");
            assert!(version == 2, "皮肤 {} version 应为 2（实际 {version}）", meta.file);
            assert!(meta.preview_hint.starts_with('#'), "preview_hint 应为 #RRGGBB");
        }
    }
}
