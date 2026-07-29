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
    pub set_property: unsafe extern "C" fn(
        session_id: RimeSessionId,
        prop: *const c_char,
        value: *const c_char,
    ),
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
    pub config_get_bool: unsafe extern "C" fn(
        config: *mut RimeConfig,
        key: *const c_char,
        value: *mut Bool,
    ) -> Bool,
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
}

extern "C" {
    /// 获取 librime 函数指针表，librime 保证其生命周期与进程一致。
    pub fn rime_get_api() -> *mut RimeApi;
}

/// 按 `RIME_STRUCT_INIT` 宏的语义初始化自带版本号的结构体。
pub fn rime_struct_init<T>(data_size_field: &mut c_int) {
    *data_size_field = (std::mem::size_of::<T>() - std::mem::size_of::<c_int>()) as c_int;
}
