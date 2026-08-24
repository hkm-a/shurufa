//! librime C API 的原始 FFI 声明。
//!
//! 结构体布局与 `third_party/librime/dist/include/rime_api.h`（1.17.0）逐字段对应，
//! `RimeApi` 只声明到本项目实际使用的函数为止（librime 通过 `data_size`
//! 做版本兼容，前缀布局合法）。修改时必须与头文件比对字段顺序。

#![allow(
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals,
    dead_code
)]

// librime C API 绑定由 build.rs 用 bindgen 从 third_party/librime/dist/include/rime_api.h
// 生成。这里不再手抄结构体/函数指针表；修改头文件后重新构建即可。
include!(concat!(env!("OUT_DIR"), "/rime_bindings.rs"));

use std::os::raw::c_int;

/// librime 头文件用 `Bool` 宏表示 int；bindgen 不生成该别名，这里补上。
pub type Bool = c_int;

// Windows 下 rime.dll 在运行期显式加载：TSF 场景中宿主应用按自身 exe
// 目录解析静态导入，必然找不到与本 DLL 同目录的 rime.dll；显式加载还能
// 把"引擎缺失"降级为可恢复错误而非模块加载失败。
#[cfg(windows)]
mod loader {
    use super::RimeApi;
    use std::ffi::c_void;
    use std::sync::OnceLock;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleExW(flags: u32, addr: *const c_void, module: *mut *mut c_void) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, buf: *mut u16, len: u32) -> u32;
    }

    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;

    /// 包含本段代码的模块（TSF 场景是 shurufa_tsf.dll，测试场景是测试 exe）
    /// 所在目录，rime.dll 与其同目录分发。
    fn self_module_dir() -> Option<std::path::PathBuf> {
        unsafe {
            let mut module = std::ptr::null_mut();
            if GetModuleHandleExW(
                FROM_ADDRESS | UNCHANGED_REFCOUNT,
                self_module_dir as *const c_void,
                &mut module,
            ) == 0
            {
                return None;
            }
            let mut buf = [0u16; 512];
            let len = GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) as usize;
            if len == 0 {
                return None;
            }
            let path = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len]));
            path.parent().map(|p| p.to_path_buf())
        }
    }

    pub fn get_api() -> Result<*mut RimeApi, String> {
        static API: OnceLock<Result<usize, String>> = OnceLock::new();
        let cached = API.get_or_init(|| unsafe {
            let library = if let Some(dir) = self_module_dir() {
                let candidate = dir.join("rime.dll");
                libloading::Library::new(&candidate).ok()
            } else {
                None
            };
            let library = match library {
                Some(lib) => lib,
                None => match libloading::Library::new("rime.dll") {
                    Ok(lib) => lib,
                    Err(e) => {
                        return Err(format!(
                            "加载 rime.dll 失败：本模块目录与搜索路径均未找到（{e}）"
                        ))
                    }
                },
            };
            let get_api = match library
                .get::<unsafe extern "C" fn() -> *mut RimeApi>(c"rime_get_api".to_bytes())
            {
                Ok(symbol) => symbol,
                Err(e) => return Err(format!("rime.dll 中未找到 rime_get_api 导出: {e}")),
            };
            let api = get_api();
            if api.is_null() {
                return Err("rime_get_api 返回空指针".into());
            }
            // 保持 rime.dll 常驻：Library drop 会 FreeLibrary，导致已取出的
            // RimeApi 指针失效。这里与旧实现一样有意泄漏模块句柄。
            std::mem::forget(library);
            Ok(api as usize)
        });
        cached.clone().map(|p| p as *mut RimeApi)
    }
}

#[cfg(windows)]
pub use loader::get_api;

// 非 Windows 平台（Android/Linux）按常规动态链接；rime_get_api 由
// 生成的 rime_bindings.rs 提供（include! 已引入）。
#[cfg(not(windows))]
pub fn get_api() -> Result<*mut RimeApi, String> {
    let api = unsafe { rime_get_api() };
    if api.is_null() {
        Err("rime_get_api 返回空指针".into())
    } else {
        Ok(api)
    }
}

/// 按 `RIME_STRUCT_INIT` 宏的语义初始化自带版本号的结构体。
pub fn rime_struct_init<T>(data_size_field: &mut c_int) {
    *data_size_field = (std::mem::size_of::<T>() - std::mem::size_of::<c_int>()) as c_int;
}
