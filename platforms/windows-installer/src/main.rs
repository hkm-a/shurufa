//! FOX 输入法安装器（Tauri 自定义外壳）。
//!
//! UI 为 HTML/CSS（无边框窗口），安装/卸载引擎在 engine.rs。
//! - 图形安装：欢迎页 → 正在安装（引擎进度）→ 完成页
//! - 卸载：FOX-Setup.exe /uninstall（注册表卸载入口）
//! - 提权：release 构建启动即检测管理员，否则以 runas 重启（安装需写 Program Files/HKLM）

#![cfg(windows)]
#![windows_subsystem = "windows"]

mod engine;

use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use serde::Serialize;
use tauri::{Emitter, LogicalSize, Manager};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

#[allow(dead_code)] // 仅在 release（提权/卸载）使用
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Serialize)]
struct DiskSpace {
    free_bytes: u64,
    total_bytes: u64,
}

/// 选择安装位置（原生目录选择框；指定父窗口避免被安装器遮挡）。
#[tauri::command]
fn browse_install_dir(window: tauri::Window) -> Result<String, String> {
    match rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("选择安装位置")
        .pick_folder()
    {
        Some(dir) => Ok(dir.to_string_lossy().into_owned()),
        None => Err("用户取消选择".into()),
    }
}

/// 查询目标盘符的可用/总空间（供欢迎页"所需空间/可用空间"展示）。
/// 目标目录可能尚未创建（GetDiskFreeSpaceExW 对不存在的路径会失败），回退到盘符根查询。
#[tauri::command]
fn free_space(path: String) -> Result<DiskSpace, String> {
    let root = if path.as_bytes().get(1) == Some(&b':') {
        format!("{}\\", &path[..2])
    } else {
        path.clone()
    };
    let wide: Vec<u16> = Path::new(&root).as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut free_to_caller = 0u64;
        let mut total = 0u64;
        let mut total_free = 0u64;
        let ok = GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_to_caller, &mut total, &mut total_free);
        if ok == 0 {
            return Err("查询磁盘空间失败".into());
        }
        Ok(DiskSpace { free_bytes: total_free, total_bytes: total })
    }
}

/// 安装引擎：payload 已嵌入（release）→ 真实安装；否则（debug）→ 模拟进度供 UI 目验。
#[tauri::command]
fn start_install(app: tauri::AppHandle, dir: String, create_start_menu: bool) -> Result<(), String> {
    if engine::PAYLOAD_FILES.is_empty() {
        simulate_progress(&app);
        return Ok(());
    }
    match engine::run_install(&app, &dir, create_start_menu) {
        Ok(()) => Ok(()),
        Err(e) => {
            // 错误同步给前端（安装中页显示失败原因，而不是被前端 catch 吞掉看起来"卡住"）
            let _ = app.emit("install-error", &e);
            Err(e)
        }
    }
}

/// 完成页动作：设为默认输入法 / 立即运行控制中心 / 创建桌面快捷方式。
#[tauri::command]
fn finish_install(
    dir: String,
    default_ime: bool,
    run_fox: bool,
    desktop_shortcut: bool,
) -> Result<(), String> {
    if engine::PAYLOAD_FILES.is_empty() {
        return Ok(());
    }
    engine::run_finish_actions(&dir, default_ime, run_fox, desktop_shortcut)
}

/// debug 构建（未嵌入 payload）时的模拟进度，跑通 welcome → installing → finish。
fn simulate_progress(app: &tauri::AppHandle) {
    let steps = ["正在准备安装目录…", "正在复制程序文件…", "正在注册输入法…", "正在配置自启动…"];
    for (i, step) in steps.iter().enumerate() {
        let _ = app.emit("install-step", step);
        let base = (i as u8) * 25;
        for n in 0..=24u8 {
            let _ = app.emit("install-progress", base + n);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    let _ = app.emit("install-progress", 100);
    let _ = app.emit("install-finished", ());
}

#[allow(dead_code)] // 仅在 release 使用
fn is_admin() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

/// 安装器单实例锁：命名 Mutex 保证同一时间只有一个安装器进程（含卸载模式）。
/// Drop 时释放句柄；进程崩溃时由系统接管（WaitForSingleObject 返回 WAIT_ABANDONED，视为可接管）。
struct SingletonGuard(isize);
impl Drop for SingletonGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

fn acquire_single_instance() -> Option<SingletonGuard> {
    use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    let name = "Global\\FOX-Setup";
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 1, wide.as_ptr()) };
    if mutex == 0 {
        return None;
    }
    // bInitialOwner=true：新建则归本进程；已存在则探测所有权。
    // WAIT_OBJECT_0(0) 已拥有；WAIT_ABANDONED(0x80) 原主崩溃可接管；WAIT_TIMEOUT 被他人持有。
    let r = unsafe { WaitForSingleObject(mutex, 0) };
    if r == 0 || r == 0x80 {
        Some(SingletonGuard(mutex))
    } else {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(mutex);
        }
        None
    }
}

/// 以管理员身份重启本安装器（UAC 提示）。
/// 注意：无参数时不能传 `-ArgumentList ''`（PowerShell 会拒绝空参数导致静默退出）。
#[allow(dead_code)] // 仅在 release 使用
fn relaunch_elevated() {
    let Some(exe) = std::env::current_exe().ok() else { return };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut ps = format!("Start-Process -FilePath '{}'", exe.display());
    if !args.is_empty() {
        let argstr = args.join(" ").replace('\'', "''");
        ps.push_str(&format!(" -ArgumentList '{}'", argstr));
    }
    ps.push_str(" -Verb RunAs");
    let _ = Command::new("powershell.exe")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // ---- 单实例：一台机器同一时间只允许一个安装器（含卸载）进程 ----
    let Some(_singleton) = acquire_single_instance() else {
        let _ = rfd::MessageDialog::new()
            .set_title("FOX 输入法安装器")
            .set_description("安装器已在运行，请先关闭后再试。")
            .set_level(rfd::MessageLevel::Info)
            .show();
        return;
    };

    // ---- 卸载模式（注册表卸载入口指向 本exe /uninstall）----
    if args.iter().any(|a| a == "/uninstall" || a == "--uninstall") {
        #[cfg(not(debug_assertions))]
        if !is_admin() {
            relaunch_elevated();
            // 提权实例需要拿到单例锁，先释放本实例的锁再退出
            drop(_singleton);
            return;
        }
        let result = engine::run_uninstall();
        let (title, desc) = match &result {
            Ok(()) => ("卸载完成", "FOX 输入法已从本机卸载。".to_string()),
            Err(e) => ("卸载未完全完成", e.clone()),
        };
        let _ = rfd::MessageDialog::new()
            .set_title(title)
            .set_description(&desc)
            .set_level(rfd::MessageLevel::Info)
            .show();
        return;
    }

    // ---- 安装模式：release 启动即要求管理员（写 Program Files + HKLM 需要）----
    #[cfg(not(debug_assertions))]
    if !is_admin() {
        relaunch_elevated();
        // 提权实例需要拿到单例锁，先释放本实例的锁再退出
        drop(_singleton);
        return;
    }

    let builder = tauri::Builder::default()
        // 开发期调试：tauri-plugin-mcp。TCP 127.0.0.1:4000，配合
        // `npx tauri-plugin-mcp-server`（ZCode MCP 配置见 ~/.zcode/cli/config.json）。
        // release 构建下插件默认不启动 socket server（惰性），不影响发布。
        .plugin(tauri_plugin_mcp::init_with_config(
            tauri_plugin_mcp::PluginConfig::new("FOX输入法".to_string())
                .start_socket_server(true)
                .tcp_localhost(4000),
        ));
    builder
        .setup(|app| {
            // 逻辑尺寸（随 DPI 缩放）：200% 屏幕上物理 1560×1160，
            // HTML 设计稿 780×580 CSS px 以 1:1 缩放渲染。
            let window = app.get_webview_window("main").expect("主窗口不存在");
            let _ = window.set_size(LogicalSize::new(780.0, 580.0));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            browse_install_dir,
            free_space,
            start_install,
            finish_install
        ])
        .run(tauri::generate_context!())
        .expect("安装器启动失败");
}
