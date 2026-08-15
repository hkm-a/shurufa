//! librime 引擎的安全 Rust 封装。
//!
//! 使用方式：`Engine::init` 完成部署与初始化（进程内仅允许一个实例），
//! `Engine::create_session` 建立输入会话，会话上喂键、取候选、取上屏文本。

pub mod ffi;

use std::ffi::{CStr, CString};
use std::mem::MaybeUninit;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

static ENGINE_ALIVE: AtomicBool = AtomicBool::new(false);

/// 输入上下文快照：预编辑串、组合光标与当前页候选。
#[derive(Debug, Default, Clone)]
pub struct Context {
    pub preedit: String,
    pub candidates: Vec<Candidate>,
    pub highlighted: usize,
    /// 组合光标在 preedit 中的位置（UTF-16 码元数，0 表示串首）。
    pub cursor_pos: usize,
    /// 当前候选页页码（从 0 开始）。
    pub page_no: usize,
    /// 每页候选条数上限。
    pub page_size: usize,
    /// 是否为候选最后一页。
    pub is_last_page: bool,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub text: String,
    pub comment: String,
}

pub struct Engine {
    api: *mut ffi::RimeApi,
    // 保持 traits 指向的 C 字符串在引擎生命周期内有效
    _strings: Vec<CString>,
    // librime 官方说明非线程安全：所有 FFI 调用必须串行进入。
    // 该栈上所有 API 指针都短命、全局进程唯一，持锁代价可忽略。
    lock: Mutex<()>,
}

// 裸指针本身可 Send/Sync；真正的线程安全靠 lock 串行化所有 FFI 入口。
unsafe impl Send for Engine {}
unsafe impl Sync for Engine {}

pub struct Session<'e> {
    engine: &'e Engine,
    id: ffi::RimeSessionId,
}

fn to_cstring(s: &str) -> CString {
    CString::new(s).expect("路径中不允许包含 NUL 字符")
}

/// 把 librime 的 UTF-8 字节偏移光标转换为 UTF-16 码元数，
/// 供 Android InputConnection 与 Windows TSF 的 ACP 选区直接使用。
fn cursor_to_utf16(preedit: &str, byte_pos: usize) -> usize {
    let boundary = preedit.floor_char_boundary(byte_pos.min(preedit.len()));
    preedit[..boundary].encode_utf16().count()
}

unsafe fn cstr_to_string(p: *const std::os::raw::c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

impl Engine {
    /// 初始化引擎：指定共享数据目录（输入方案）与用户数据目录，
    /// 首次运行会触发方案编译（部署），阻塞直至完成。
    pub fn init(shared_data_dir: &Path, user_data_dir: &Path) -> Result<Self, String> {
        if ENGINE_ALIVE.swap(true, Ordering::SeqCst) {
            return Err("进程内已存在 Engine 实例".into());
        }
        let lock = Mutex::new(());
        std::fs::create_dir_all(user_data_dir).map_err(|e| format!("创建用户数据目录失败: {e}"))?;

        let shared = to_cstring(&shared_data_dir.to_string_lossy());
        let user = to_cstring(&user_data_dir.to_string_lossy());
        let name = to_cstring("shurufa");
        let code = to_cstring("shurufa");
        let version = to_cstring(env!("CARGO_PKG_VERSION"));
        let app = to_cstring("rime.shurufa");
        let log_dir = to_cstring("");

        unsafe {
            let api = match ffi::get_api() {
                Ok(api) => api,
                Err(e) => {
                    ENGINE_ALIVE.store(false, Ordering::SeqCst);
                    return Err(e);
                }
            };
            let api_ref = &*api;

            // 每个待调用的 FFI 字段都必须非空，否则后续调用是未定义行为。
            let api_ptrs: [*const (); 14] = [
                api_ref.setup as *const (),
                api_ref.initialize as *const (),
                api_ref.start_maintenance as *const (),
                api_ref.join_maintenance_thread as *const (),
                api_ref.create_session as *const (),
                api_ref.destroy_session as *const (),
                api_ref.process_key as *const (),
                api_ref.get_context as *const (),
                api_ref.free_context as *const (),
                api_ref.get_commit as *const (),
                api_ref.free_commit as *const (),
                api_ref.get_option as *const (),
                api_ref.set_option as *const (),
                api_ref.simulate_key_sequence as *const (),
            ];
            if api_ptrs.iter().any(|p| p.is_null()) {
                ENGINE_ALIVE.store(false, Ordering::SeqCst);
                return Err("librime API 表不完整（含空函数指针）".into());
            }

            let mut traits = MaybeUninit::<ffi::RimeTraits>::zeroed().assume_init();
            ffi::rime_struct_init::<ffi::RimeTraits>(&mut traits.data_size);
            traits.shared_data_dir = shared.as_ptr();
            traits.user_data_dir = user.as_ptr();
            traits.distribution_name = name.as_ptr();
            traits.distribution_code_name = code.as_ptr();
            traits.distribution_version = version.as_ptr();
            traits.app_name = app.as_ptr();
            traits.min_log_level = 2; // 仅记录 ERROR 及以上
            traits.log_dir = log_dir.as_ptr(); // 空串表示仅输出到 stderr

            (api_ref.setup)(&mut traits);
            (api_ref.initialize)(&mut traits);

            if (api_ref.start_maintenance)(1) != 0 {
                (api_ref.join_maintenance_thread)();
            }

            Ok(Engine {
                api,
                _strings: vec![shared, user, name, code, version, app, log_dir],
                lock,
            })
        }
    }

    /// 串行进入 librime：所有 FFI 调用必须经这把锁，防止并发破坏 composer/context。
    fn lock(&self) -> MutexGuard<'_, ()> {
        self.lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn api(&self) -> &ffi::RimeApi {
        unsafe { &*self.api }
    }

    pub fn create_session(&self) -> Result<Session<'_>, String> {
        let _guard = self.lock();
        let id = unsafe { (self.api().create_session)() };
        if id == 0 {
            return Err("创建 Rime 会话失败".into());
        }
        Ok(Session { engine: self, id })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _guard = self.lock();
        unsafe {
            (self.api().cleanup_all_sessions)();
            (self.api().finalize)();
        }
        ENGINE_ALIVE.store(false, Ordering::SeqCst);
    }
}

impl Session<'_> {
    /// 模拟一段按键序列（Rime 键序语法，如 "nihao"）。
    pub fn simulate(&self, keys: &str) -> bool {
        let _guard = self.engine.lock();
        let keys = to_cstring(keys);
        unsafe { (self.engine.api().simulate_key_sequence)(self.id, keys.as_ptr()) != 0 }
    }

    /// 发送单个键（X11 keysym 编码，与 librime 约定一致）。
    pub fn process_key(&self, keycode: i32, mask: i32) -> bool {
        let _guard = self.engine.lock();
        unsafe { (self.engine.api().process_key)(self.id, keycode, mask) != 0 }
    }

    /// 读取布尔开关（如 "ascii_mode"、"simplification"）。
    pub fn get_option(&self, option: &str) -> bool {
        let _guard = self.engine.lock();
        let opt = to_cstring(option);
        unsafe { (self.engine.api().get_option)(self.id, opt.as_ptr()) != 0 }
    }

    /// 设置布尔开关。
    pub fn set_option(&self, option: &str, value: bool) {
        let _guard = self.engine.lock();
        let opt = to_cstring(option);
        unsafe {
            (self.engine.api().set_option)(self.id, opt.as_ptr(), value as ffi::Bool);
        }
    }

    /// 切换中英文（ascii_mode）；返回切换后是否为英文直输模式。
    ///
    /// **绝不能嵌套调用本类的其它方法**：`self.engine.lock()` 是 std Mutex，
    /// 不可重入，`get_option()`/`set_option()` 内部会再次 lock 造成自死锁，
    /// 死锁线程将永久持有引擎锁，拖垮全部并发会话（2026-08-12 实测：
    /// 每次 Shift 触发 toggle_ascii → serve 线程死锁 → 全局 IPC 无响应）。
    /// 这里已持有锁，一律直接走底层 FFI。
    pub fn toggle_ascii(&self) -> bool {
        let _guard = self.engine.lock();
        let api = self.engine.api();
        let opt = to_cstring("ascii_mode");
        let now = unsafe { (api.get_option)(self.id, opt.as_ptr()) == 0 };
        unsafe {
            (api.set_option)(self.id, opt.as_ptr(), now as ffi::Bool);
        }
        now
    }

    /// 读取当前输入上下文（预编辑串与候选列表）。
    pub fn context(&self) -> Context {
        let _guard = self.engine.lock();
        let api = self.engine.api();
        unsafe {
            let mut ctx = MaybeUninit::<ffi::RimeContext>::zeroed().assume_init();
            ffi::rime_struct_init::<ffi::RimeContext>(&mut ctx.data_size);
            if (api.get_context)(self.id, &mut ctx) == 0 {
                return Context::default();
            }
            let preedit = cstr_to_string(ctx.composition.preedit);
            let cursor_pos = cursor_to_utf16(&preedit, ctx.composition.cursor_pos.max(0) as usize);
            let mut result = Context {
                preedit,
                candidates: Vec::new(),
                highlighted: ctx.menu.highlighted_candidate_index.max(0) as usize,
                cursor_pos,
                page_no: ctx.menu.page_no.max(0) as usize,
                page_size: ctx.menu.page_size.max(0) as usize,
                is_last_page: ctx.menu.is_last_page != 0,
            };
            if !ctx.menu.candidates.is_null() && ctx.menu.num_candidates > 0 {
                let list = std::slice::from_raw_parts(
                    ctx.menu.candidates,
                    ctx.menu.num_candidates as usize,
                );
                for c in list {
                    result.candidates.push(Candidate {
                        text: cstr_to_string(c.text),
                        comment: cstr_to_string(c.comment),
                    });
                }
            }
            (api.free_context)(&mut ctx);
            result
        }
    }

    /// 取出已上屏文本；无上屏内容时返回 None。
    pub fn commit(&self) -> Option<String> {
        let _guard = self.engine.lock();
        let api = self.engine.api();
        unsafe {
            let mut commit = MaybeUninit::<ffi::RimeCommit>::zeroed().assume_init();
            ffi::rime_struct_init::<ffi::RimeCommit>(&mut commit.data_size);
            if (api.get_commit)(self.id, &mut commit) == 0 {
                return None;
            }
            let text = cstr_to_string(commit.text);
            (api.free_commit)(&mut commit);
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
    }

    /// 设置组合内光标（字节偏移，以待编辑的 raw input 为准，即 UTF-8）。
    pub fn set_caret_pos(&self, byte_pos: usize) {
        let _guard = self.engine.lock();
        unsafe { (self.engine.api().set_caret_pos)(self.id, byte_pos) }
    }

    /// 当前组合内光标的字节偏移。
    pub fn caret_pos(&self) -> usize {
        let _guard = self.engine.lock();
        unsafe { (self.engine.api().get_caret_pos)(self.id) }
    }

    /// 当前原始输入串（raw input，ASCII 拼音）。
    pub fn input(&self) -> String {
        let _guard = self.engine.lock();
        unsafe {
            let p = (self.engine.api().get_input)(self.id);
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }

    /// 选择当前页第 `index` 个候选；成功返回 true。
    pub fn select_candidate_on_current_page(&self, index: usize) -> bool {
        let _guard = self.engine.lock();
        unsafe { (self.engine.api().select_candidate_on_current_page)(self.id, index) != 0 }
    }

    /// 翻一页候选；`backward` 为 true 是上一页。
    pub fn change_page(&self, backward: bool) -> bool {
        let _guard = self.engine.lock();
        unsafe { (self.engine.api().change_page)(self.id, backward as ffi::Bool) != 0 }
    }

    /// 删除当前页第 `index` 个候选（"忘记该词"）；成功返回 true。
    pub fn forget_on_current_page(&self, index: usize) -> bool {
        let _guard = self.engine.lock();
        unsafe { (self.engine.api().delete_candidate_on_current_page)(self.id, index) != 0 }
    }

    /// 读取引擎状态位：(是否英文直输, 是否全角, 是否英文标点)。
    /// 失败（取不到状态）时按默认中文/半角/中文标点返回。
    pub fn status_bits(&self) -> (bool, bool, bool) {
        let _guard = self.engine.lock();
        let api = self.engine.api();
        unsafe {
            let mut status = MaybeUninit::<ffi::RimeStatus>::zeroed().assume_init();
            ffi::rime_struct_init::<ffi::RimeStatus>(&mut status.data_size);
            if (api.get_status)(self.id, &mut status) == 0 {
                return (false, false, false);
            }
            let bits = (
                status.is_ascii_mode != 0,
                status.is_full_shape != 0,
                status.is_ascii_punct != 0,
            );
            // RimeStatus 内含 C 字符串，必须交还引擎释放
            (api.free_status)(&mut status);
            bits
        }
    }

    /// 切换本会话的输入方案（如 "shurufa_double_pinyin"）。成功返回 true。
    /// 底层走 librime `select_schema`；方案必须在已部署 schema 列表内。
    pub fn select_schema(&self, schema_id: &str) -> bool {
        let _guard = self.engine.lock();
        let id = to_cstring(schema_id);
        unsafe { (self.engine.api().select_schema)(self.id, id.as_ptr()) != 0 }
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        let _guard = self.engine.lock();
        unsafe {
            (self.engine.api().destroy_session)(self.id);
        }
    }
}
