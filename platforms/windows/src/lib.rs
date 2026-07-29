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

/// 进程级共享的 librime 引擎；首次调用触发初始化与部署。
pub fn engine() -> Result<&'static Engine, HRESULT> {
    let result = ENGINE.get_or_init(|| {
        Engine::init(&shared_data_dir(), &user_config_root().join("rime"))
    });
    result.as_ref().map_err(|_| E_FAIL)
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
