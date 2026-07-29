//! Windows TSF 文本服务入口：DLL 导出与全局状态。
//!
//! 注册流程（regsvr32，需管理员）：DllRegisterServer 写入 CLSID 注册表项，
//! 并向 TSF 注册 zh-CN 语言配置；运行流程：宿主应用经 COM 创建 TextService。

#![cfg(windows)]

mod candidate_window;
mod composition;
mod factory;
mod keys;
mod registry;
mod service;

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::OnceLock;

use windows::core::{IUnknown, Interface, GUID, HRESULT};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, E_FAIL, HINSTANCE, S_FALSE, S_OK,
};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

use ime_bridge::Engine;

/// 文本服务 COM 类标识
pub const CLSID_SHURUFA: GUID = GUID::from_u128(0x8a5c1b49_3d2e_4f7a_9c61_0b7e2d5a9f13);
/// zh-CN 语言配置标识
pub const GUID_PROFILE: GUID = GUID::from_u128(0xc4e9d2a7_6b31_4a58_8f0d_1e9a7c3b5d26);
/// 输入法显示名称
pub const IME_NAME: &str = "Shurufa 拼音";

static DLL_INSTANCE: AtomicIsize = AtomicIsize::new(0);
static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();

/// 当前 DLL 的完整路径。
pub fn dll_path() -> PathBuf {
    let hinst = HINSTANCE(DLL_INSTANCE.load(Ordering::Relaxed) as *mut c_void);
    let mut buf = [0u16; 512];
    let len = unsafe { GetModuleFileNameW(Some(hinst.into()), &mut buf) } as usize;
    PathBuf::from(String::from_utf16_lossy(&buf[..len]))
}

/// 共享数据目录：自 DLL 路径向上寻找 schemas 目录（开发期布局），
/// 找不到则回落到 %APPDATA%\shurufa\schemas（安装期布局）。
fn shared_data_dir() -> PathBuf {
    let dll = dll_path();
    for dir in dll.ancestors() {
        let candidate = dir.join("schemas");
        if candidate.join("default.yaml").exists() {
            return candidate;
        }
    }
    user_config_root().join("schemas")
}

fn user_config_root() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
}

/// 引擎就绪状态：TSF 回调绝不允许被引擎初始化阻塞。
pub enum EngineState {
    Ready(&'static Engine),
    /// 后台初始化进行中，按键应直通
    Pending,
    Failed,
}

static ENGINE_INIT_STARTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 非阻塞获取引擎：首次调用启动后台初始化线程（含词典部署，
/// 可能耗时），完成前返回 Pending。曾冻结系统输入进程导致输入法
/// 被系统从键盘列表移除，因此禁止在 TSF 回调线程上同步初始化。
pub fn try_engine() -> EngineState {
    if let Some(result) = ENGINE.get() {
        return match result {
            Ok(engine) => EngineState::Ready(engine),
            Err(_) => EngineState::Failed,
        };
    }
    if !ENGINE_INIT_STARTED.swap(true, Ordering::SeqCst) {
        std::thread::spawn(|| {
            ENGINE.get_or_init(|| {
                let shared = shared_data_dir();
                debug_log(&format!("引擎后台初始化开始：shared={}", shared.display()));
                let started = std::time::Instant::now();
                let r = Engine::init(&shared, &user_config_root().join("rime"));
                match &r {
                    Ok(_) => debug_log(&format!(
                        "引擎就绪，耗时 {} ms",
                        started.elapsed().as_millis()
                    )),
                    Err(e) => debug_log(&format!("引擎初始化失败：{e}")),
                }
                r
            });
        });
    }
    EngineState::Pending
}

/// 轻量排障日志：写入 %TEMP%\shurufa-tsf.log（AppContainer 有各自的
/// TEMP，均可写）。失败静默——日志不能反过来影响输入法。
/// 整行一次性写出，避免多进程并发追加时互相穿插。
pub fn debug_log(msg: &str) {
    use std::io::Write;
    static EXE_NAME: OnceLock<String> = OnceLock::new();
    let exe = EXE_NAME.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "?".into())
    });
    let path = std::env::temp_dir().join("shurufa-tsf.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let line = format!("[{ts}] [{exe}:{}] {msg}\n", std::process::id());
        let _ = f.write_all(line.as_bytes());
    }
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
