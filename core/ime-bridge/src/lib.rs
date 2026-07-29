//! librime 引擎的安全 Rust 封装。
//!
//! 使用方式：`Engine::init` 完成部署与初始化（进程内仅允许一个实例），
//! `Engine::create_session` 建立输入会话，会话上喂键、取候选、取上屏文本。

pub mod ffi;

use std::ffi::{CStr, CString};
use std::mem::MaybeUninit;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

static ENGINE_ALIVE: AtomicBool = AtomicBool::new(false);

/// 输入上下文快照：预编辑串与当前页候选。
#[derive(Debug, Default, Clone)]
pub struct Context {
    pub preedit: String,
    pub candidates: Vec<Candidate>,
    pub highlighted: usize,
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
}

// librime 内部自带线程同步（会话表有锁保护）；指针表进程级唯一。
unsafe impl Send for Engine {}
unsafe impl Sync for Engine {}

pub struct Session<'e> {
    engine: &'e Engine,
    id: ffi::RimeSessionId,
}

fn to_cstring(s: &str) -> CString {
    CString::new(s).expect("路径中不允许包含 NUL 字符")
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
        std::fs::create_dir_all(user_data_dir)
            .map_err(|e| format!("创建用户数据目录失败: {e}"))?;

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
            })
        }
    }

    fn api(&self) -> &ffi::RimeApi {
        unsafe { &*self.api }
    }

    pub fn create_session(&self) -> Result<Session<'_>, String> {
        let id = unsafe { (self.api().create_session)() };
        if id == 0 {
            return Err("创建 Rime 会话失败".into());
        }
        Ok(Session { engine: self, id })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
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
        let keys = to_cstring(keys);
        unsafe { (self.engine.api().simulate_key_sequence)(self.id, keys.as_ptr()) != 0 }
    }

    /// 发送单个键（X11 keysym 编码，与 librime 约定一致）。
    pub fn process_key(&self, keycode: i32, mask: i32) -> bool {
        unsafe { (self.engine.api().process_key)(self.id, keycode, mask) != 0 }
    }

    /// 读取当前输入上下文（预编辑串与候选列表）。
    pub fn context(&self) -> Context {
        let api = self.engine.api();
        unsafe {
            let mut ctx = MaybeUninit::<ffi::RimeContext>::zeroed().assume_init();
            ffi::rime_struct_init::<ffi::RimeContext>(&mut ctx.data_size);
            if (api.get_context)(self.id, &mut ctx) == 0 {
                return Context::default();
            }
            let mut result = Context {
                preedit: cstr_to_string(ctx.composition.preedit),
                candidates: Vec::new(),
                highlighted: ctx.menu.highlighted_candidate_index.max(0) as usize,
            };
            if !ctx.menu.candidates.is_null() && ctx.menu.num_candidates > 0 {
                let list =
                    std::slice::from_raw_parts(ctx.menu.candidates, ctx.menu.num_candidates as usize);
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
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.engine.api().destroy_session)(self.id);
        }
    }
}
