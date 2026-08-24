//! Windows TSF 文本服务入口：DLL 导出与全局状态。
//!
//! 注册流程（regsvr32，需管理员）：DllRegisterServer 写入 CLSID 注册表项，
//! 并向 TSF 注册 zh-CN 语言配置；运行流程：宿主应用经 COM 创建 TextService。

#![cfg(windows)]

mod ai_candidates;
mod cand_client;
mod candidate_window;
mod composition;
mod direct_launch;
mod emoji_question;
mod english_candidates;
mod factory;
mod ipc_client;
mod keys;
mod registry;
mod service;
mod skin;
mod toast;
mod toast_pipe;
pub mod uia_provider;

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::OnceLock;

use windows::core::{IUnknown, Interface, GUID, HRESULT};
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, E_FAIL, HINSTANCE, S_FALSE, S_OK};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

/// 文本服务 COM 类标识
pub const CLSID_SHURUFA: GUID = GUID::from_u128(0x8a5c1b49_3d2e_4f7a_9c61_0b7e2d5a9f13);
/// zh-CN 语言配置标识
pub const GUID_PROFILE: GUID = GUID::from_u128(0xc4e9d2a7_6b31_4a58_8f0d_1e9a7c3b5d26);
/// 输入法显示名称
pub const IME_NAME: &str = "Shurufa 拼音";

static DLL_INSTANCE: AtomicIsize = AtomicIsize::new(0);

/// 当前 DLL 的完整路径。
pub fn dll_path() -> PathBuf {
    let hinst = HINSTANCE(DLL_INSTANCE.load(Ordering::Relaxed) as *mut c_void);
    let mut buf = [0u16; 512];
    let len = unsafe { GetModuleFileNameW(Some(hinst.into()), &mut buf) } as usize;
    PathBuf::from(String::from_utf16_lossy(&buf[..len]))
}

// 引擎已迁出本进程：由独立算法服务（shurufa-algo）提供，本 DLL 只作 IPC 客户端。
// （见 core/ime-ipc 与 platforms/windows-algo）

/// 轻量排障日志：写入 %TEMP%\shurufa-tsf.log（AppContainer 有各自的
/// TEMP，均可写）。失败静默——日志不能反过来影响输入法。
///
/// 历史坑（2026-08-16 实机反馈"打字莫名卡顿"）：旧实现每次调用同步
/// OpenOptions::append + write_all，而每键热路径（service.rs 的"键 vk=…"与
/// candidate_window.rs 的"cand show"）每键写 2 次文件 ≈ 1.8ms，叠加
/// ui.show() 重读皮肤 ~1ms，快速打字时每键 ~3ms 同步磁盘 I/O；且多宿主进程
/// 并发 append 同一文件存在锁竞争，磁盘抖动时单次可飙到数十 ms——表现为
/// "莫名其妙"的间歇卡顿。
/// 现改为**内存缓冲 + 后台节流落盘**：热路径只 push 进 Vec（微秒级零 I/O），
/// 后台线程每 500ms 或满 200 行一次性整批写入（单次 write_all 仍保持整行
/// 不穿插）。缓冲上限 5000 行，日志风暴时丢最旧保内存。计划外进程退出至多
/// 丢最后 500ms 日志，对排障可接受（与 MRU 后台节流同理）。
pub fn debug_log(msg: &str) {
    let line = {
        static EXE_NAME: OnceLock<String> = OnceLock::new();
        let exe = EXE_NAME.get_or_init(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "?".into())
        });
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("[{ts}] [{exe}:{}] {msg}\n", std::process::id())
    };
    let mut guard = LOG_BUF.lock().unwrap_or_else(|p| p.into_inner());
    if guard.len() >= 5000 {
        // 日志风暴保护：丢最旧 1/4，保留最近行（排障时最近的最有用）
        let keep = 3750;
        let drop_count = guard.len() - keep;
        guard.drain(0..drop_count);
    }
    guard.push(line);
    drop(guard);
    spawn_log_flusher();
}

/// 日志缓冲（模块级共享：debug_log push，后台线程 flush）。
/// const Mutex：零初始化、无 OnceLock 空锁风险；首次使用即就绪。
static LOG_BUF: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
/// 落盘线程只启动一次的标志。
static FLUSHER_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 后台日志落盘线程：每 500ms 取走缓冲一次性写盘。只在本进程首次
/// debug_log 时启动一次；写失败静默（下次再试）。
fn spawn_log_flusher() {
    use std::io::Write;
    if FLUSHER_STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("shurufa-log-flusher".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            // 取走缓冲：热路径只短暂持锁做 take，落盘在锁外完成
            let batch = {
                let mut guard = LOG_BUF.lock().unwrap_or_else(|p| p.into_inner());
                if guard.is_empty() {
                    continue;
                }
                std::mem::take(&mut *guard)
            };
            if batch.is_empty() {
                continue;
            }
            let path = std::env::temp_dir().join("shurufa-tsf.log");
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let joined = batch.concat();
                let _ = f.write_all(joined.as_bytes());
            }
        })
        .ok();
}

#[no_mangle]
extern "system" fn DllMain(hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> bool {
    if reason == DLL_PROCESS_ATTACH {
        DLL_INSTANCE.store(hinst.0 as isize, Ordering::Relaxed);
    }
    true
}

#[no_mangle]
extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        if rclsid.is_null() || riid.is_null() || ppv.is_null() {
            return E_FAIL;
        }
        if *rclsid != CLSID_SHURUFA {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IUnknown = factory::ClassFactory.into();
        factory.query(riid, ppv)
    }
}

#[no_mangle]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    // 引擎与窗口状态挂在进程全局，保持常驻，交由进程退出统一回收
    S_FALSE
}

#[no_mangle]
extern "system" fn DllRegisterServer() -> HRESULT {
    match registry::register() {
        Ok(()) => S_OK,
        Err(e) => e.into(),
    }
}

#[no_mangle]
extern "system" fn DllUnregisterServer() -> HRESULT {
    match registry::unregister() {
        Ok(()) => S_OK,
        Err(e) => e.into(),
    }
}
