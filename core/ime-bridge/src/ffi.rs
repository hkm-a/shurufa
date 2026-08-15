//! librime C API 的原始 FFI 声明。
//!
//! 结构体布局与 `third_party/librime/dist/include/rime_api.h`（1.17.0）逐字段对应，
//! `RimeApi` 只声明到本项目实际使用的函数为止（librime 通过 `data_size`
//! 做版本兼容，前缀布局合法）。修改时必须与头文件比对字段顺序。

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use std::os::raw::{c_char, c_double, c_int, c_void};

pub type Bool = c_int;
pub type RimeSessionId = usize;

#[repr(C)]
pub struct RimeTraits {
    pub data_size: c_int,
    pub shared_data_dir: *const c_char,
    pub user_data_dir: *const c_char,
    pub distribution_name: *const c_char,
    pub distribution_code_name: *const c_char,
    pub distribution_version: *const c_char,
    pub app_name: *const c_char,
    pub modules: *const *const c_char,
    pub min_log_level: c_int,
    pub log_dir: *const c_char,
    pub prebuilt_data_dir: *const c_char,
    pub staging_dir: *const c_char,
}

#[repr(C)]
pub struct RimeComposition {
    pub length: c_int,
    pub cursor_pos: c_int,
    pub sel_start: c_int,
    pub sel_end: c_int,
    pub preedit: *mut c_char,
}

#[repr(C)]
pub struct RimeCandidate {
    pub text: *mut c_char,
    pub comment: *mut c_char,
    pub reserved: *mut c_void,
}

#[repr(C)]
pub struct RimeMenu {
    pub page_size: c_int,
    pub page_no: c_int,
    pub is_last_page: Bool,
    pub highlighted_candidate_index: c_int,
    pub num_candidates: c_int,
    pub candidates: *mut RimeCandidate,
    pub select_keys: *mut c_char,
}

#[repr(C)]
pub struct RimeCommit {
    pub data_size: c_int,
    pub text: *mut c_char,
}

#[repr(C)]
pub struct RimeContext {
    pub data_size: c_int,
    pub composition: RimeComposition,
    pub menu: RimeMenu,
    pub commit_text_preview: *mut c_char,
    pub select_labels: *mut *mut c_char,
}

#[repr(C)]
pub struct RimeStatus {
    pub data_size: c_int,
    pub schema_id: *mut c_char,
    pub schema_name: *mut c_char,
    pub is_disabled: Bool,
    pub is_composing: Bool,
    pub is_ascii_mode: Bool,
    pub is_full_shape: Bool,
    pub is_simplified: Bool,
    pub is_traditional: Bool,
    pub is_ascii_punct: Bool,
}

#[repr(C)]
pub struct RimeConfig {
    pub ptr: *mut c_void,
}

#[repr(C)]
pub struct RimeConfigIterator {
    pub list: *mut c_void,
    pub map: *mut c_void,
    pub index: c_int,
    pub key: *const c_char,
    pub path: *const c_char,
}

#[repr(C)]
pub struct RimeSchemaListItem {
    pub schema_id: *mut c_char,
    pub name: *mut c_char,
    pub reserved: *mut c_void,
}

#[repr(C)]
pub struct RimeSchemaList {
    pub size: usize,
    pub list: *mut RimeSchemaListItem,
}

pub type RimeNotificationHandler = Option<
    unsafe extern "C" fn(
        context_object: *mut c_void,
        session_id: RimeSessionId,
        message_type: *const c_char,
        message_value: *const c_char,
    ),
>;

/// `rime_api.h` 中 `RimeApi` 函数指针表的前缀（截至 simulate_key_sequence）。
#[repr(C)]
pub struct RimeApi {
    pub data_size: c_int,

    pub setup: unsafe extern "C" fn(traits: *mut RimeTraits),
    pub set_notification_handler:
        unsafe extern "C" fn(handler: RimeNotificationHandler, context_object: *mut c_void),

    pub initialize: unsafe extern "C" fn(traits: *mut RimeTraits),
    pub finalize: unsafe extern "C" fn(),
    pub start_maintenance: unsafe extern "C" fn(full_check: Bool) -> Bool,
    pub is_maintenance_mode: unsafe extern "C" fn() -> Bool,
    pub join_maintenance_thread: unsafe extern "C" fn(),

    pub deployer_initialize: unsafe extern "C" fn(traits: *mut RimeTraits),
    pub prebuild: unsafe extern "C" fn() -> Bool,
    pub deploy: unsafe extern "C" fn() -> Bool,
    pub deploy_schema: unsafe extern "C" fn(schema_file: *const c_char) -> Bool,
    pub deploy_config_file:
        unsafe extern "C" fn(file_name: *const c_char, version_key: *const c_char) -> Bool,
    pub sync_user_data: unsafe extern "C" fn() -> Bool,

    pub create_session: unsafe extern "C" fn() -> RimeSessionId,
    pub find_session: unsafe extern "C" fn(session_id: RimeSessionId) -> Bool,
    pub destroy_session: unsafe extern "C" fn(session_id: RimeSessionId) -> Bool,
    pub cleanup_stale_sessions: unsafe extern "C" fn(),
    pub cleanup_all_sessions: unsafe extern "C" fn(),

    pub process_key:
        unsafe extern "C" fn(session_id: RimeSessionId, keycode: c_int, mask: c_int) -> Bool,
    pub commit_composition: unsafe extern "C" fn(session_id: RimeSessionId) -> Bool,
    pub clear_composition: unsafe extern "C" fn(session_id: RimeSessionId),

    pub get_commit:
        unsafe extern "C" fn(session_id: RimeSessionId, commit: *mut RimeCommit) -> Bool,
    pub free_commit: unsafe extern "C" fn(commit: *mut RimeCommit) -> Bool,
    pub get_context:
        unsafe extern "C" fn(session_id: RimeSessionId, context: *mut RimeContext) -> Bool,
    pub free_context: unsafe extern "C" fn(context: *mut RimeContext) -> Bool,
    pub get_status:
        unsafe extern "C" fn(session_id: RimeSessionId, status: *mut RimeStatus) -> Bool,
    pub free_status: unsafe extern "C" fn(status: *mut RimeStatus) -> Bool,

    pub set_option:
        unsafe extern "C" fn(session_id: RimeSessionId, option: *const c_char, value: Bool),
    pub get_option: unsafe extern "C" fn(session_id: RimeSessionId, option: *const c_char) -> Bool,
    pub set_property:
        unsafe extern "C" fn(session_id: RimeSessionId, prop: *const c_char, value: *const c_char),
    pub get_property: unsafe extern "C" fn(
        session_id: RimeSessionId,
        prop: *const c_char,
        value: *mut c_char,
        buffer_size: usize,
    ) -> Bool,

    pub get_schema_list: unsafe extern "C" fn(schema_list: *mut RimeSchemaList) -> Bool,
    pub free_schema_list: unsafe extern "C" fn(schema_list: *mut RimeSchemaList),
    pub get_current_schema: unsafe extern "C" fn(
        session_id: RimeSessionId,
        schema_id: *mut c_char,
        buffer_size: usize,
    ) -> Bool,
    pub select_schema:
        unsafe extern "C" fn(session_id: RimeSessionId, schema_id: *const c_char) -> Bool,

    pub schema_open:
        unsafe extern "C" fn(schema_id: *const c_char, config: *mut RimeConfig) -> Bool,
    pub config_open:
        unsafe extern "C" fn(config_id: *const c_char, config: *mut RimeConfig) -> Bool,
    pub config_close: unsafe extern "C" fn(config: *mut RimeConfig) -> Bool,
    pub config_get_bool:
        unsafe extern "C" fn(config: *mut RimeConfig, key: *const c_char, value: *mut Bool) -> Bool,
    pub config_get_int: unsafe extern "C" fn(
        config: *mut RimeConfig,
        key: *const c_char,
        value: *mut c_int,
    ) -> Bool,
    pub config_get_double: unsafe extern "C" fn(
        config: *mut RimeConfig,
        key: *const c_char,
        value: *mut c_double,
    ) -> Bool,
    pub config_get_string: unsafe extern "C" fn(
        config: *mut RimeConfig,
        key: *const c_char,
        value: *mut c_char,
        buffer_size: usize,
    ) -> Bool,
    pub config_get_cstring:
        unsafe extern "C" fn(config: *mut RimeConfig, key: *const c_char) -> *const c_char,
    pub config_update_signature:
        unsafe extern "C" fn(config: *mut RimeConfig, signer: *const c_char) -> Bool,
    pub config_begin_map: unsafe extern "C" fn(
        iterator: *mut RimeConfigIterator,
        config: *mut RimeConfig,
        key: *const c_char,
    ) -> Bool,
    pub config_next: unsafe extern "C" fn(iterator: *mut RimeConfigIterator) -> Bool,
    pub config_end: unsafe extern "C" fn(iterator: *mut RimeConfigIterator),

    pub simulate_key_sequence:
        unsafe extern "C" fn(session_id: RimeSessionId, key_sequence: *const c_char) -> Bool,

    // module
    pub register_module: unsafe extern "C" fn(module: *mut RimeModule) -> Bool,
    pub find_module: unsafe extern "C" fn(module_name: *const c_char) -> *mut RimeModule,
    pub run_task: unsafe extern "C" fn(task_name: *const c_char) -> Bool,

    // deprecated data-dir getters
    pub get_shared_data_dir: unsafe extern "C" fn() -> *const c_char,
    pub get_user_data_dir: unsafe extern "C" fn() -> *const c_char,
    pub get_sync_dir: unsafe extern "C" fn() -> *const c_char,

    pub get_user_id: unsafe extern "C" fn() -> *const c_char,
    pub get_user_data_sync_dir: unsafe extern "C" fn(dir: *mut c_char, buffer_size: usize),

    // config init / load
    pub config_init: unsafe extern "C" fn(config: *mut RimeConfig) -> Bool,
    pub config_load_string:
        unsafe extern "C" fn(config: *mut RimeConfig, yaml: *const c_char) -> Bool,

    // config value setters
    pub config_set_bool:
        unsafe extern "C" fn(config: *mut RimeConfig, key: *const c_char, value: Bool) -> Bool,
    pub config_set_int:
        unsafe extern "C" fn(config: *mut RimeConfig, key: *const c_char, value: c_int) -> Bool,
    pub config_set_double:
        unsafe extern "C" fn(config: *mut RimeConfig, key: *const c_char, value: c_double) -> Bool,
    pub config_set_string: unsafe extern "C" fn(
        config: *mut RimeConfig,
        key: *const c_char,
        value: *const c_char,
    ) -> Bool,

    // config complex structures
    pub config_get_item: unsafe extern "C" fn(
        config: *mut RimeConfig,
        key: *const c_char,
        value: *mut RimeConfig,
    ) -> Bool,
    pub config_set_item: unsafe extern "C" fn(
        config: *mut RimeConfig,
        key: *const c_char,
        value: *mut RimeConfig,
    ) -> Bool,
    pub config_clear: unsafe extern "C" fn(config: *mut RimeConfig, key: *const c_char) -> Bool,
    pub config_create_list:
        unsafe extern "C" fn(config: *mut RimeConfig, key: *const c_char) -> Bool,
    pub config_create_map:
        unsafe extern "C" fn(config: *mut RimeConfig, key: *const c_char) -> Bool,
    pub config_list_size:
        unsafe extern "C" fn(config: *mut RimeConfig, key: *const c_char) -> usize,
    pub config_begin_list: unsafe extern "C" fn(
        iterator: *mut RimeConfigIterator,
        config: *mut RimeConfig,
        key: *const c_char,
    ) -> Bool,

    // raw input
    pub get_input: unsafe extern "C" fn(session_id: RimeSessionId) -> *const c_char,
    pub get_caret_pos: unsafe extern "C" fn(session_id: RimeSessionId) -> usize,
    pub select_candidate: unsafe extern "C" fn(session_id: RimeSessionId, index: usize) -> Bool,
    pub get_version: unsafe extern "C" fn() -> *const c_char,
    pub set_caret_pos: unsafe extern "C" fn(session_id: RimeSessionId, caret_pos: usize),
    pub select_candidate_on_current_page:
        unsafe extern "C" fn(session_id: RimeSessionId, index: usize) -> Bool,

    // candidate list iterator
    pub candidate_list_begin: unsafe extern "C" fn(
        session_id: RimeSessionId,
        iterator: *mut RimeCandidateListIterator,
    ) -> Bool,
    pub candidate_list_next: unsafe extern "C" fn(iterator: *mut RimeCandidateListIterator) -> Bool,
    pub candidate_list_end: unsafe extern "C" fn(iterator: *mut RimeCandidateListIterator),

    pub user_config_open:
        unsafe extern "C" fn(config_id: *const c_char, config: *mut RimeConfig) -> Bool,
    pub candidate_list_from_index: unsafe extern "C" fn(
        session_id: RimeSessionId,
        iterator: *mut RimeCandidateListIterator,
        index: c_int,
    ) -> Bool,

    // deprecated data-dir getters
    pub get_prebuilt_data_dir: unsafe extern "C" fn() -> *const c_char,
    pub get_staging_dir: unsafe extern "C" fn() -> *const c_char,

    // capnproto (deprecated)
    pub commit_proto: unsafe extern "C" fn(session_id: RimeSessionId, commit_builder: *mut c_void),
    pub context_proto:
        unsafe extern "C" fn(session_id: RimeSessionId, context_builder: *mut c_void),
    pub status_proto: unsafe extern "C" fn(session_id: RimeSessionId, status_builder: *mut c_void),

    pub get_state_label: unsafe extern "C" fn(
        session_id: RimeSessionId,
        option_name: *const c_char,
        state: Bool,
    ) -> *const c_char,

    pub delete_candidate: unsafe extern "C" fn(session_id: RimeSessionId, index: usize) -> Bool,
    pub delete_candidate_on_current_page:
        unsafe extern "C" fn(session_id: RimeSessionId, index: usize) -> Bool,

    // abbreviated state label returns RimeStringSlice (ptr+len)
    pub get_state_label_abbreviated: unsafe extern "C" fn(
        session_id: RimeSessionId,
        option_name: *const c_char,
        state: Bool,
        abbreviated: Bool,
    ) -> RimeStringSlice,

    pub set_input: unsafe extern "C" fn(session_id: RimeSessionId, input: *const c_char) -> Bool,

    // data-dir setters with _s suffix
    pub get_shared_data_dir_s: unsafe extern "C" fn(dir: *mut c_char, buffer_size: usize),
    pub get_user_data_dir_s: unsafe extern "C" fn(dir: *mut c_char, buffer_size: usize),
    pub get_prebuilt_data_dir_s: unsafe extern "C" fn(dir: *mut c_char, buffer_size: usize),
    pub get_staging_dir_s: unsafe extern "C" fn(dir: *mut c_char, buffer_size: usize),
    pub get_sync_dir_s: unsafe extern "C" fn(dir: *mut c_char, buffer_size: usize),

    pub highlight_candidate: unsafe extern "C" fn(session_id: RimeSessionId, index: usize) -> Bool,
    pub highlight_candidate_on_current_page:
        unsafe extern "C" fn(session_id: RimeSessionId, index: usize) -> Bool,
    pub change_page: unsafe extern "C" fn(session_id: RimeSessionId, backward: Bool) -> Bool,
}

/// RimeStringSlice = { const char* str; size_t len; }
#[repr(C)]
#[derive(Copy, Clone)]
pub struct RimeStringSlice {
    pub str_: *const c_char,
    pub len: usize,
}

/// RimeModule placeholder (opaque)
#[repr(C)]
pub struct RimeModule {
    _opaque: [u8; 0],
}

/// RimeCandidateListIterator placeholder (opaque layout is implementation-defined)
#[repr(C)]
pub struct RimeCandidateListIterator {
    _opaque: [u8; 0],
}

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
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn GetModuleHandleExW(flags: u32, addr: *const c_void, module: *mut *mut c_void) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, buf: *mut u16, len: u32) -> u32;
    }

    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

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
            let mut module = std::ptr::null_mut();
            if let Some(dir) = self_module_dir() {
                let candidate = dir.join("rime.dll");
                module = LoadLibraryW(wide(&candidate.to_string_lossy()).as_ptr());
            }
            if module.is_null() {
                // 回落到常规搜索路径（PATH、系统目录）
                module = LoadLibraryW(wide("rime.dll").as_ptr());
            }
            if module.is_null() {
                return Err("加载 rime.dll 失败：本模块目录与搜索路径均未找到".into());
            }
            let proc = GetProcAddress(module, c"rime_get_api".as_ptr() as *const u8);
            if proc.is_null() {
                return Err("rime.dll 中未找到 rime_get_api 导出".into());
            }
            let get_api: unsafe extern "C" fn() -> *mut RimeApi = std::mem::transmute(proc);
            let api = get_api();
            if api.is_null() {
                return Err("rime_get_api 返回空指针".into());
            }
            Ok(api as usize)
        });
        cached.clone().map(|p| p as *mut RimeApi)
    }
}

#[cfg(windows)]
pub use loader::get_api;

// 非 Windows 平台（Android/Linux）按常规动态链接
#[cfg(not(windows))]
extern "C" {
    fn rime_get_api() -> *mut RimeApi;
}

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
