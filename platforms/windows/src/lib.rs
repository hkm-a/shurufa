//! Windows TSF 文本服务入口：DLL 导出与全局状态。
//!
//! 注册流程（regsvr32，需管理员）：DllRegisterServer 写入 CLSID 注册表项，
//! 并向 TSF 注册 zh-CN 语言配置；运行流程：宿主应用经 COM 创建 TextService。

#![cfg(windows)]

mod candidate_window;
mod candidate_window_d2d;
mod candidate_window_dcomp;
mod composition;
mod factory;
mod ipc_client;
mod keys;
mod registry;
mod service;
mod skin;

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

/// 引擎已迁出本进程：由独立算法服务（shurufa-algo）提供，本 DLL 只作 IPC 客户端。
/// （见 core/ime-ipc 与 platforms/windows-algo）

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
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
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
