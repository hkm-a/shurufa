//! FOX 安装器安装/卸载引擎。
//!
//! 逻辑移植自旧 NSIS 脚本（installer/shurufa.nsi）与 scripts/install.ps1 约定：
//! 停旧进程 → 写入 payload → rime 词典预构建 → icacls 权限 → regsvr32 注册 TSF →
//! 后台服务自启动 → 快捷方式 → 卸载注册表 → 启动宿主 → 终态验证。
//! 安装器为 64 位进程，System32 即 64 位，无 WOW64 重定向问题。
//!
//! payload 由 build.rs 嵌入（release 构建自包含；debug 构建为空清单 → 走模拟进度）。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::Emitter;

include!(concat!(env!("OUT_DIR"), "/payload_manifest.rs"));

/// 默认安装位置（与欢迎页输入框一致；安装需管理员权限）。
pub const DEFAULT_INSTALL_DIR: &str = "C:\\Program Files\\FOX";

const START_MENU_FOLDER: &str = "FOX";
const SHORTCUT_NAME: &str = "FOX输入法.lnk";
const UNINSTALL_REG_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\FOX输入法";
const POWERSHELL: &str = "powershell.exe";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// 输入法 TIP（与 installer/Deploy-Shurufa.ps1 的 $script:ShurufaInputTip 保持一致，SSOT）。
const DEFAULT_IME_TIP: &str =
    r"0804:{8A5C1B49-3D2E-4F7A-9C61-0B7E2D5A9F13}{C4E9D2A7-6B31-4A58-8F0D-1E9A7C3B5D26}";
/// 默认输入法覆盖的注册表位置（Set-WinDefaultInputMethodOverride 的底层实现）。
const DEFAULT_IME_REG_KEY: &str = r"HKCU\Control Panel\International\User Profile";
/// 单条外部命令执行上限（防 WinRT/PowerShell 偶发挂起导致安装流程卡死）。
const CMD_TIMEOUT_SECS: u64 = 60;

/// IFEO 高优先级注册表基路径（进程名追加其后，如 `...\shurufa-algo.exe`）。
/// 用途（2026-08-16，weasel#1250 同类问题）：Windows 功耗管理会把空闲的
/// 算法服务压到 0.5-1GHz，按键后频率爬升慢 → 选词"莫名其妙卡顿"。在
/// IFEO PerfOptions 写 CpuPriorityClass=3 (High)，让 algo/host 每次启动自动
/// 以高优先级运行——**无需进程自身提权**，保持普通用户运行（提权进程创建的
/// IPC 管道会拒绝普通应用，见 stop_process 注释的历史坑），安装器以管理员
/// 写 HKLM 注册表即可全局生效。
const IFEO_ROOT: &str =
    r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options";
/// 需要高优先级的进程（安装时写入 / 卸载时清除）。
const HIGH_PRIORITY_PROCS: [&str; 2] = ["shurufa-algo.exe", "shurufa-host.exe"];

/// 给指定进程写 IFEO 高优先级（CpuPriorityClass=3=High）。需管理员权限
/// （安装器已提权）。返回值：成功/失败日志由调用方处理。
fn set_high_priority(name: &str) -> Result<(), String> {
    let key = format!(r"{IFEO_ROOT}\{name}\PerfOptions");
    run_cmd(
        "reg.exe",
        &[
            "add",
            &key,
            "/v",
            "CpuPriorityClass",
            "/t",
            "REG_DWORD",
            "/d",
            "3",
            "/f",
        ],
    )
    .map(|_| ())
    .map_err(|e| format!("设置 {name} 高优先级失败：{e}"))
}

/// 清除指定进程的 IFEO 高优先级配置（卸载时还原系统默认）。
fn clear_high_priority(name: &str) {
    let key = format!(r"{IFEO_ROOT}\{name}\PerfOptions");
    let _ = run_cmd("reg.exe", &["delete", &key, "/v", "CpuPriorityClass", "/f"]);
    // 子键空了就删掉整个 PerfOptions（避免留空壳）
    let _ = run_cmd("reg.exe", &["delete", &key, "/f"]);
    let _ = run_cmd(
        "reg.exe",
        &["delete", &format!(r"{IFEO_ROOT}\{name}"), "/f"],
    );
}

use std::os::windows::process::CommandExt;

/// 尽力而为步骤（目标进程可能本就不在），结果不判定成败。
///
/// 历史坑（2026-08-16 实机复现）：单发一次 `taskkill /f /im` 不够——
/// - `shurufa-algo.exe --once xiufu` 挂起时，taskkill 报"没有运行实例"（按
///   PID 找不到）但进程实际存活并锁住 exe，安装器步骤 2 写入持续失败；
/// - 安装器杀掉 algo 后，host 的 supervise/自启动逻辑可能在 1-2s 内把它重新
///   拉起，写入时又撞上文件锁。
///
/// 因此这里做"多轮 杀 → 确认"：taskkill 后轮询 tasklist 直到目标全部消失，
/// 顽固进程再用 WMI Terminate 兜底，全部超时后仍继续（后续写文件的重试循环
/// 还会再杀一次并等待解锁）。
fn stop_process(name: &str) {
    const ROUNDS: u32 = 3;
    const VERIFY_MS: u64 = 250;

    let taskkill = |extra: &[&str]| {
        let mut args = vec!["/f"];
        args.extend_from_slice(extra);
        args.push("/im");
        args.push(name);
        let _ = Command::new("taskkill.exe")
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    };

    for round in 0..ROUNDS {
        taskkill(&[]);
        // 轮询确认：目标进程全部消失才算这一轮成功
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        loop {
            if !process_exists(name) {
                return; // 已全部退出
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(VERIFY_MS));
        }
        if round + 1 == ROUNDS {
            break;
        }
        // 未退干净：WMI Terminate 兜底（对 taskkill 看不见的挂起进程有效）
        let _ = run_cmd(
            POWERSHELL,
            &[
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &format!(
                    "Get-CimInstance Win32_Process -Filter \"Name='{name}'\" | ForEach-Object {{ Invoke-CimMethod -InputObject $_ -MethodName Terminate | Out-Null }}"
                ),
            ],
        );
    }
}

/// 轮询确认指定名字的进程是否还存在（tasklist 逐行比对，区分大小写不敏感）。
fn process_exists(name: &str) -> bool {
    let out = Command::new("tasklist.exe")
        .args(["/fo", "csv", "/nh"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    out.lines().any(|line| {
        line.to_ascii_lowercase()
            .contains(&name.to_ascii_lowercase())
    })
}

/// 等待文件不再被独占锁定（可写）。进程被杀后句柄释放需要时间，
/// 轮询尝试以写模式打开，超时返回（是否成功由后续写入决定）。
fn wait_file_unlocked(path: &Path, timeout: std::time::Duration) {
    use std::io::ErrorKind;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match fs::OpenOptions::new().write(true).open(path) {
            Ok(f) => {
                drop(f);
                return;
            }
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::PermissionDenied | ErrorKind::WouldBlock
                ) && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            _ => return,
        }
    }
}

/// 关键步骤：失败返回 Err（上层中止并弹窗）。带 60s 超时，超时强杀并报错。
fn run_cmd(program: &str, args: &[&str]) -> Result<String, String> {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let mut child = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 {program} 失败：{e}"))?;

    let deadline = Instant::now() + Duration::from_secs(CMD_TIMEOUT_SECS);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() > deadline => {
                let _ = child.kill();
                return Err(format!("{program} 执行超时（{CMD_TIMEOUT_SECS}s），已终止"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(format!("等待 {program} 失败：{e}")),
        }
    };

    let mut out = String::new();
    let mut err = String::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_string(&mut out);
    }
    if let Some(mut se) = child.stderr.take() {
        let _ = se.read_to_string(&mut err);
    }
    if status.success() {
        Ok(out)
    } else {
        Err(format!(
            "{program} 退出码 {}：{}",
            status.code().unwrap_or(-1),
            err.trim()
        ))
    }
}

fn emit_step(app: &tauri::AppHandle, percent: u8, text: &str) {
    let _ = app.emit("install-step", text);
    let _ = app.emit("install-progress", percent);
}

/// 追加一行到安装日志（UTF-8 + BOM，notepad 可读）。目标目录不可写时回退 $TEMP。
fn log_append(dir: &Path, msg: &str) {
    let primary = dir.join("install.log");
    let fallback =
        PathBuf::from(std::env::var("TEMP").unwrap_or_else(|_| ".".into())).join("FOX-install.log");
    let mut opened = None;
    if let Ok(f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&primary)
    {
        opened = Some(f);
    } else if let Ok(f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&fallback)
    {
        opened = Some(f);
    }
    let Some(mut file) = opened else { return };
    let _ = file.write_all(b"\xEF\xBB\xBF"); // UTF-8 BOM
    let _ = file.write_all(msg.as_bytes());
    let _ = file.write_all(b"\r\n");
}

/// 关键步骤包装：记录命令与结果到安装日志。
fn run_cmd_logged(
    log_dir: &Path,
    label: &str,
    program: &str,
    args: &[&str],
) -> Result<String, String> {
    log_append(log_dir, &format!("▶ {label}: {program} {}", args.join(" ")));
    match run_cmd(program, args) {
        Ok(out) => {
            log_append(log_dir, &format!("  ✓ {label}"));
            Ok(out)
        }
        Err(e) => {
            log_append(log_dir, &format!("  ✗ {label} 失败：{e}"));
            Err(e)
        }
    }
}

fn run_powershell_script(script: &Path, extra: &[&str]) -> Result<String, String> {
    let mut args = vec![
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ];
    let script_str = script.to_str().ok_or("脚本路径非 UTF-8")?;
    args.push(script_str);
    args.extend_from_slice(extra);
    run_cmd(POWERSHELL, &args)
}

fn unregister_tsf(target: &Path) {
    // 反注册当前版本与遗留文件名，再枚举 shurufa_tsf-*.dll 覆盖跨版本残留。
    let mut candidates = vec![
        target.join(format!("shurufa_tsf-{PAYLOAD_VERSION}.dll")),
        target.join("shurufa_tsf.dll"),
    ];
    if let Ok(entries) = fs::read_dir(target) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("shurufa_tsf-") && name.ends_with(".dll") {
                candidates.push(entry.path());
            }
        }
    }
    for dll in candidates {
        if dll.exists() {
            let _ = run_cmd("regsvr32.exe", &["/s", "/u", dll.to_str().unwrap()]);
        }
    }
}

/// 用 WScript.Shell 创建 .lnk 快捷方式（目标为 exe）。
fn create_shortcut(lnk: &Path, target: &Path) -> Result<(), String> {
    let lnk_q = lnk.to_str().ok_or("快捷方式路径非 UTF-8")?;
    let target_q = target.to_str().ok_or("目标路径非 UTF-8")?;
    let ps = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{lnk_q}'); \
         $s.TargetPath='{target_q}'; $s.Save()"
    );
    run_cmd(
        POWERSHELL,
        &[
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &ps,
        ],
    )
    .map(|_| ())
}

fn start_menu_path() -> Result<PathBuf, String> {
    let pd = std::env::var("ProgramData").map_err(|_| "缺少 ProgramData 环境变量")?;
    Ok(PathBuf::from(pd)
        .join("Microsoft\\Windows\\Start Menu\\Programs")
        .join(START_MENU_FOLDER))
}

fn desktop_path() -> Result<PathBuf, String> {
    let up = std::env::var("USERPROFILE").map_err(|_| "缺少 USERPROFILE 环境变量")?;
    let plain = PathBuf::from(&up).join("Desktop");
    if plain.exists() {
        return Ok(plain);
    }
    // OneDrive 重定向的桌面
    let onedrive = PathBuf::from(&up).join("OneDrive\\Desktop");
    if onedrive.exists() {
        return Ok(onedrive);
    }
    Ok(plain)
}

fn write_uninstall_registry(install_dir: &str, exe: &str) -> Result<(), String> {
    let key = UNINSTALL_REG_KEY;
    let reg = |name: &str, value: &str, ty: &str| {
        run_cmd(
            "reg.exe",
            &[
                "add",
                &format!("HKLM\\{key}"),
                "/v",
                name,
                "/t",
                ty,
                "/d",
                value,
                "/f",
            ],
        )
    };
    reg("DisplayName", "FOX输入法", "REG_SZ")?;
    reg("DisplayVersion", PAYLOAD_VERSION, "REG_SZ")?;
    reg("Publisher", "FOX", "REG_SZ")?;
    reg("InstallLocation", install_dir, "REG_SZ")?;
    reg(
        "UninstallString",
        &format!("\"{exe}\" /uninstall"),
        "REG_SZ",
    )?;
    reg("NoModify", "1", "REG_DWORD")?;
    reg("NoRepair", "1", "REG_DWORD")?;
    Ok(())
}

/// 以普通用户身份启动一个程序（经 schtasks 一次性任务 + 交互用户受限令牌）。
///
/// **绝不能从提权进程直接 spawn**：安装器以管理员运行，直接 spawn 会让子进程
/// 继承提权。宿主链提权 → 算法服务以 High 完整性创建 IPC 管道，普通应用连接
/// 被完整性策略拒绝（err=5）输入法整体失效；控制中心提权 → 悬浮条以管理员
/// 窗口运行（跨提权层级无法被普通应用托举/交互）。两者都是 2026-08-14 实机
/// 复现的 bug 类。schtasks 任务默认"仅当用户登录时运行、受限权限"（交互用户
/// 的非提权令牌），跑完立即删任务。
fn launch_as_user(target: &Path, args: &[&str]) -> Result<(), String> {
    let tn = "FOXUserLaunch";
    let mut quoted = format!("\"{}\"", target.display());
    for a in args {
        quoted.push(' ');
        quoted.push_str(&format!("\"{a}\""));
    }
    // /SC ONCE + /ST 过去时刻仅为满足 /Create 必填项；实际触发靠 /Run。
    // /RL LIMITED 显式声明受限权限（默认即如此，写清避免未来误改）。
    run_cmd(
        "schtasks.exe",
        &[
            "/Create", "/TN", tn, "/TR", &quoted, "/SC", "ONCE", "/ST", "00:00", "/RL", "LIMITED",
            "/F",
        ],
    )
    .map_err(|e| format!("创建用户启动任务失败：{e}"))?;
    let run_result = run_cmd("schtasks.exe", &["/Run", "/TN", tn]);
    let _ = run_cmd("schtasks.exe", &["/Delete", "/TN", tn, "/F"]);
    run_result
        .map(|_| ())
        .map_err(|e| format!("启动 {} 失败：{e}", target.display()))
}

fn start_host(target: &Path) -> Result<(), String> {
    let host = target.join("shurufa-host.exe");
    launch_as_user(&host, &["supervise"])?;
    let _ = Command::new("ctfmon.exe")
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    Ok(())
}

/// 安装后引擎预热（2026-08-16，weasel#1250 同类问题）：经命名管道向算法
/// 服务做一次真实 IPC 往返（CreateSession + ToggleAscii），把"首键成本"
/// （会话创建、词典加载、首次候选生成）移到安装收尾，用户第一次输入即已
/// 就绪，同时验证管道连通。失败非致命（服务可能仍在启动，由 host 接管）。
///
/// 用 kernel32 extern 直调（windows-sys 0.52 未导出 ReadFile/WriteFile）。
fn warmup_algo_service(target: &Path) {
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            sec: *const std::ffi::c_void,
            disp: u32,
            flags: u32,
            tmpl: *mut std::ffi::c_void,
        ) -> isize;
        fn SetNamedPipeHandleState(
            pipe: isize,
            mode: *const u32,
            max_count: *const u32,
            name: *const u16,
        ) -> i32;
        fn WriteFile(
            file: isize,
            buf: *const u8,
            n: u32,
            written: *mut u32,
            ov: *mut std::ffi::c_void,
        ) -> i32;
        fn ReadFile(
            file: isize,
            buf: *mut u8,
            n: u32,
            read: *mut u32,
            ov: *mut std::ffi::c_void,
        ) -> i32;
        fn CloseHandle(h: isize) -> i32;
    }
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const OPEN_EXISTING: u32 = 3;
    const INVALID_HANDLE: isize = -1;
    const PIPE_READMODE_MESSAGE: u32 = 2;

    let pipe_name: Vec<u16> = "\\\\.\\pipe\\shurufa-algo".encode_utf16().collect();
    unsafe {
        // algo 由 host supervise 拉起，需要一点时间；轮询等待管道就绪
        // （最多 8 秒，覆盖首轮会话/词典初始化），就绪后才预热。
        let mut handle = INVALID_HANDLE;
        for _ in 0..16 {
            handle = CreateFileW(
                pipe_name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            );
            if handle != INVALID_HANDLE {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        if handle == INVALID_HANDLE {
            log_append(target, "  预热：8s 内算法服务未就绪（跳过，host 会接管）");
            return;
        }
        SetNamedPipeHandleState(
            handle,
            &PIPE_READMODE_MESSAGE,
            std::ptr::null(),
            std::ptr::null(),
        );

        // 帧：4 字节小端长度前缀 + JSON 体（消息模式单写单读即一帧）
        fn roundtrip(handle: isize, json: &str) -> bool {
            unsafe {
                let body = json.as_bytes();
                let mut frame = Vec::with_capacity(4 + body.len());
                frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
                frame.extend_from_slice(body);
                let mut written: u32 = 0;
                if WriteFile(
                    handle,
                    frame.as_ptr(),
                    frame.len() as u32,
                    &mut written,
                    std::ptr::null_mut(),
                ) == 0
                {
                    return false;
                }
                let mut buf = [0u8; 65_536];
                let mut read: u32 = 0;
                if ReadFile(
                    handle,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                ) == 0
                {
                    return false;
                }
                read >= 4
            }
        }

        let ok = roundtrip(handle, r#"{"CreateSession":{}}"#)
            && roundtrip(handle, r#"{"ToggleAscii":{}}"#);
        CloseHandle(handle);
        if ok {
            log_append(target, "  ✓ 引擎预热完成（会话已就绪）");
        } else {
            log_append(target, "  预热：IPC 往返异常（服务可能仍在启动，跳过）");
        }
    }
}

/// 清理旧安装（确保"一台机器只有一个版本"）：
/// - 旧版默认目录 %ProgramData%\shurufa（SetShellVarContext all 时代的安装位置）
/// - 卸载注册表里记录的其它安装位置（用户改过安装目录的旧版本）
///
/// 反注册其 TSF 并整目录删除；删除失败（占用）只记录告警，不阻断新装。
fn clean_legacy_install(target: &Path) {
    let mut legacy: Vec<PathBuf> =
        vec![PathBuf::from(std::env::var("ProgramData").unwrap_or_default()).join("shurufa")];
    if let Some(loc) = read_install_location() {
        let p = PathBuf::from(&loc);
        if p != target && !legacy.contains(&p) {
            legacy.push(p);
        }
    }
    for dir in legacy {
        if dir == target || !dir.exists() || !dir.join("shurufa-host.exe").exists() {
            continue; // 不是本产品的安装目录
        }
        log_append(
            target,
            &format!("检测到旧安装目录 {}，正在清理…", dir.display()),
        );
        unregister_tsf(&dir);
        match fs::remove_dir_all(&dir) {
            Ok(()) => log_append(target, &format!("✓ 旧安装目录已删除：{}", dir.display())),
            Err(e) => log_append(
                target,
                &format!("  ⚠ 旧安装目录删除失败（可能被占用，稍后可手动删除）：{e}"),
            ),
        }
    }
}

/// 真实安装（release 构建、payload 已嵌入时调用）。
pub fn run_install(
    app: &tauri::AppHandle,
    dir: &str,
    create_start_menu: bool,
) -> Result<(), String> {
    let target = Path::new(dir);
    log_append(
        target,
        &format!("══ 开始安装 FOX 输入法 {PAYLOAD_VERSION} → {dir}"),
    );
    emit_step(app, 3, "正在准备安装目录…");

    // 1. 停旧进程（控制中心 / 宿主 / 算法 / 输入法宿主），反注册旧 TSF。
    //    注意：shurufa-algo.exe 也要杀——宿主被停后算法进程不会自动退出，
    //    会一直锁住 shurufa-algo.exe 导致后续写入失败。
    log_append(target, "步骤 1/10 停旧进程与反注册旧 TSF");
    stop_process("Shurufa.exe");
    stop_process("shurufa-host.exe");
    stop_process("shurufa-algo.exe");
    stop_process("ctfmon.exe");
    stop_process("TextInputHost.exe");
    // 进程退出后句柄释放需要时间；TextInputHost 被杀后可能立即重启，
    // 稍作等待并允许后续写文件重试。
    std::thread::sleep(std::time::Duration::from_millis(1500));
    unregister_tsf(target);
    // 一台机器只保留一个安装：清理旧版目录（ProgramData\shurufa / 异路径旧安装）
    clean_legacy_install(target);
    std::thread::sleep(std::time::Duration::from_millis(500));

    // 2. 写入 payload（被占用文件允许重试；TSF DLL 会被已注册的旧版锁住——
    //    所有文本输入进程都会加载它，无法杀光。被锁时回退唯一文件名并注册新文件）
    emit_step(app, 15, "正在复制程序文件…");
    log_append(
        target,
        &format!("步骤 2/10 写入 {} 个 payload 文件", PAYLOAD_FILES.len()),
    );
    fs::create_dir_all(target).map_err(|e| {
        log_append(target, &format!("✗ 创建安装目录失败：{e}"));
        format!("创建安装目录失败：{e}")
    })?;

    let tsf_primary = format!("shurufa_tsf-{PAYLOAD_VERSION}.dll");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let tsf_fallback = format!("shurufa_tsf-{PAYLOAD_VERSION}-{ts}.dll");
    let mut tsf_dest = tsf_primary.clone();
    let mut tsf_fell_back = false;

    for f in PAYLOAD_FILES {
        let data = &PAYLOAD_BYTES[f.offset..f.offset + f.len];
        let mut dest_name = f.dest;
        if f.dest == tsf_primary && tsf_fell_back {
            dest_name = tsf_dest.as_str();
        }
        let write_result = (|| -> Result<(), String> {
            let dest = target.join(dest_name);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录 {} 失败：{e}", parent.display()))?;
            }
            for attempt in 1..=6 {
                match fs::write(&dest, data) {
                    Ok(()) => return Ok(()),
                    Err(e) if attempt < 6 => {
                        log_append(
                            target,
                            &format!(
                                "  ⚠ 写入 {} 第 {attempt} 次失败（{e}），重试…",
                                dest.display()
                            ),
                        );
                        // 失败多为运行中的宿主/算法进程锁住 exe：先再杀一轮，
                        // 再等待文件句柄释放，然后重试（对抗 supervise 重新拉起）。
                        if dest_name.ends_with(".exe") || dest_name.ends_with(".dll") {
                            stop_process("shurufa-algo.exe");
                            stop_process("shurufa-host.exe");
                            stop_process("Shurufa.exe");
                        }
                        wait_file_unlocked(&dest, std::time::Duration::from_secs(4));
                        std::thread::sleep(std::time::Duration::from_millis(300 * attempt as u64));
                    }
                    Err(e) => {
                        log_append(target, &format!("✗ 写入 {} 失败：{e}", dest.display()));
                        return Err(format!("写入 {} 失败：{e}", dest.display()));
                    }
                }
            }
            unreachable!()
        })();
        if let Err(e) = write_result {
            if f.dest == tsf_primary && !tsf_fell_back {
                // TSF DLL 被锁 → 回退唯一文件名，注册新文件
                tsf_fell_back = true;
                tsf_dest = tsf_fallback.clone();
                log_append(
                    target,
                    &format!("  ⚠ TSF DLL 被旧进程锁定，回退唯一文件名：{tsf_dest}"),
                );
                let dest = target.join(&tsf_dest);
                fs::write(&dest, data).map_err(|e| {
                    log_append(target, &format!("✗ 写入 {} 失败：{e}", dest.display()));
                    format!("写入 {} 失败：{e}", dest.display())
                })?;
            } else {
                return Err(e);
            }
        }
    }

    // 3. rime 词典预构建
    emit_step(app, 35, "正在构建词典…");
    let schemas = target.join("schemas");
    let deployer = target.join("rime_deployer.exe");
    let s = schemas.to_str().unwrap();
    run_cmd_logged(
        target,
        "rime 词典预构建",
        deployer.to_str().unwrap(),
        &["--build", s, s, &format!("{s}\\build")],
    )
    .map_err(|e| format!("词典预构建失败：{e}"))?;

    // 4. 授予输入法宿主读取权限（AppContainer）
    emit_step(app, 50, "正在配置权限…");
    run_cmd_logged(
        target,
        "icacls 权限",
        "icacls.exe",
        &[dir, "/grant", "*S-1-15-2-1:(OI)(CI)(RX)", "/t", "/c"],
    )
    .map_err(|e| format!("授予权限失败：{e}"))?;

    // 5. 注册 TSF 输入法（用实际写入的文件名，可能已回退为唯一名）
    emit_step(app, 62, "正在注册输入法…");
    let tsf = target.join(&tsf_dest);
    run_cmd_logged(
        target,
        "regsvr32 注册 TSF",
        "regsvr32.exe",
        &["/s", tsf.to_str().unwrap()],
    )
    .map_err(|e| format!("注册 TSF 输入法失败：{e}"))?;

    // 5.5 清理孤儿 TSF DLL：只保留本次注册的那个文件，删掉历次安装累积的
    //     shurufa_tsf-*.dll / .old-* 残留（被占用时跳过，下次安装再清）
    if let Ok(entries) = fs::read_dir(target) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_tsf = name.starts_with("shurufa_tsf") && name.ends_with(".dll");
            let is_old_backup = name.starts_with("shurufa_tsf") && name.contains(".old-");
            if !is_tsf && !is_old_backup {
                continue;
            }
            if name == tsf_dest {
                continue; // 本次注册的文件保留
            }
            let path = entry.path();
            match fs::remove_file(&path) {
                Ok(()) => log_append(target, &format!("  ✓ 清理孤儿 TSF 文件：{name}")),
                Err(_) => log_append(target, &format!("  ⚠ 孤儿 TSF 文件占用中，暂留：{name}")),
            }
        }
    }

    // 6. 配置后台服务登录自启动
    emit_step(app, 75, "正在配置自启动…");
    run_cmd_logged(
        target,
        "配置自启动",
        POWERSHELL,
        &[
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            target.join("register-host-startup.ps1").to_str().unwrap(),
            "-InstallDir",
            dir,
        ],
    )
    .map_err(|e| format!("配置自启动失败：{e}"))?;

    // 7. 开始菜单快捷方式（欢迎页勾选）
    if create_start_menu {
        log_append(target, "步骤 7/10 创建开始菜单快捷方式");
        let sm = start_menu_path()?;
        fs::create_dir_all(&sm).map_err(|e| format!("创建开始菜单目录失败：{e}"))?;
        create_shortcut(&sm.join(SHORTCUT_NAME), &target.join("Shurufa.exe"))?;
    }

    // 8. 卸载注册表项（卸载入口 = 本 exe /uninstall）
    log_append(target, "步骤 8/10 写入卸载注册表项");
    let exe = std::env::current_exe()
        .map_err(|e| format!("获取安装器路径失败：{e}"))?
        .to_string_lossy()
        .into_owned();
    write_uninstall_registry(dir, &exe)?;

    // 8.5 高优先级（IFEO PerfOptions）：algo/host 以 High 优先级运行，抗
    // Windows 功耗降频导致的"首键/选词莫名卡顿"（见 set_high_priority 注释）。
    for proc_name in HIGH_PRIORITY_PROCS {
        match set_high_priority(proc_name) {
            Ok(()) => log_append(target, &format!("  ✓ {proc_name} 高优先级已设置")),
            Err(e) => log_append(target, &format!("  ⚠ {e}")),
        }
    }

    // 9. 启动宿主与 ctfmon
    emit_step(app, 88, "正在启动后台服务…");
    log_append(target, "步骤 9/10 启动宿主与 ctfmon");
    start_host(target)?;
    // 引擎预热：内部会轮询等待 algo 就绪后做一次 IPC 往返，
    // 把首键成本（会话/词典加载）移到安装收尾
    warmup_algo_service(target);

    // 10. 终态验证
    emit_step(app, 95, "正在验证安装…");
    run_cmd_logged(
        target,
        "终态验证",
        POWERSHELL,
        &[
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            target.join("verify-install.ps1").to_str().unwrap(),
            "-InstallDir",
            dir,
        ],
    )
    .map_err(|e| format!("安装终态验证未通过：{e}"))?;

    log_append(target, "══ 安装完成");
    let _ = app.emit("install-step", "安装完成");
    let _ = app.emit("install-progress", 100);
    let _ = app.emit("install-finished", ());
    Ok(())
}

/// 完成页动作：设为默认输入法 / 立即运行控制中心 / 创建桌面快捷方式。
pub fn run_finish_actions(
    install_dir: &str,
    default_ime: bool,
    run_fox: bool,
    desktop_shortcut: bool,
) -> Result<(), String> {
    let target = Path::new(install_dir);
    log_append(target, &format!("完成页动作 dir={install_dir} default_ime={default_ime} run_fox={run_fox} desktop={desktop_shortcut}"));

    if default_ime {
        // 直接写 InputMethodOverride 注册表，绕开偶发挂起的 WinRT 命令
        // Set-WinDefaultInputMethodOverride（实测可在非交互会话中永久阻塞）。
        log_append(target, "→ 设置默认输入法（写注册表 InputMethodOverride）");
        run_cmd(
            "reg.exe",
            &[
                "add",
                DEFAULT_IME_REG_KEY,
                "/v",
                "InputMethodOverride",
                "/t",
                "REG_SZ",
                "/d",
                DEFAULT_IME_TIP,
                "/f",
            ],
        )
        .map_err(|e| format!("设置默认输入法失败：{e}"))?;
        log_append(target, "✓ 默认输入法已设置");
    }
    if desktop_shortcut {
        log_append(target, "→ 创建桌面快捷方式");
        let desktop = desktop_path()?;
        create_shortcut(&desktop.join(SHORTCUT_NAME), &target.join("Shurufa.exe"))?;
        log_append(
            target,
            &format!(
                "✓ 桌面快捷方式已创建：{}",
                desktop.join(SHORTCUT_NAME).display()
            ),
        );
    }
    if run_fox {
        log_append(target, "→ 启动控制中心（普通用户，勿提权）");
        // 与 start_host 同款降权：提权进程直接 spawn 会让悬浮条以管理员窗口
        // 运行（2026-08-14 实机复现）。经 schtasks 以交互用户受限令牌拉起。
        let _ = launch_as_user(&target.join("Shurufa.exe"), &[]);
    }
    Ok(())
}

/// 卸载：停进程 → 反注册 TSF → 移除自启动/默认输入法 → 删注册表/快捷方式/目录。
pub fn run_uninstall() -> Result<(), String> {
    // 安装目录：从卸载注册表读（未读到则回退默认）
    let install_dir = read_install_location().unwrap_or_else(|| DEFAULT_INSTALL_DIR.to_string());
    let target = Path::new(&install_dir);

    stop_process("Shurufa.exe");
    stop_process("shurufa-host.exe");
    stop_process("shurufa-algo.exe");
    stop_process("ctfmon.exe");
    stop_process("TextInputHost.exe");
    unregister_tsf(target);

    let _ = run_powershell_script(&target.join("register-host-startup.ps1"), &["-Remove"]);
    // 清除默认输入法：直接删 InputMethodOverride（绕开可能挂起的 WinRT 命令）
    let _ = run_cmd(
        "reg.exe",
        &[
            "delete",
            DEFAULT_IME_REG_KEY,
            "/v",
            "InputMethodOverride",
            "/f",
        ],
    );

    let _ = run_cmd(
        "reg.exe",
        &["delete", &format!("HKLM\\{UNINSTALL_REG_KEY}"), "/f"],
    );

    // 清理安装时写入的 IFEO 高优先级配置（还原系统默认）
    for proc_name in HIGH_PRIORITY_PROCS {
        clear_high_priority(proc_name);
    }

    // 快捷方式
    let _ = fs::remove_file(
        start_menu_path()
            .map(|p| p.join(SHORTCUT_NAME))
            .unwrap_or_default(),
    );
    let _ = fs::remove_file(
        desktop_path()
            .map(|p| p.join(SHORTCUT_NAME))
            .unwrap_or_default(),
    );
    if let Ok(sm) = start_menu_path() {
        let _ = fs::remove_dir(&sm);
    }

    // 目录删除（含残余 DLL 占用告警由上层处理）
    if target.exists() {
        fs::remove_dir_all(target).map_err(|e| format!("删除安装目录失败：{e}"))?;
    }
    Ok(())
}

fn read_install_location() -> Option<String> {
    let out = Command::new("reg.exe")
        .args([
            "query",
            &format!("HKLM\\{UNINSTALL_REG_KEY}"),
            "/v",
            "InstallLocation",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("InstallLocation"))?;
    let value = line.split_whitespace().last()?;
    Some(value.to_string())
}
