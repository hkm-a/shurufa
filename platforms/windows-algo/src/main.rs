//! shurufa-algo：librime 独立算法服务进程。
//!
//! 架构 M6 前置的一部分：把 librime 引擎从每个 TSF 宿主进程内移出到本进程，
//! 通过命名管道 `\\.\pipe\shurufa-algo` 服务按键与上下文。这样用户词库
//! （leveldb LOCK）只在**这一个**进程加载，多宿主进程不再互相抢锁。
//!
//! 为验证演进正确性，同时提供 `--once` 单次模式：从 stdin 读一串键序列
//! （如 `nihao`），服务一次后打印候选并退出，便于命令行回归验证引擎可用。

// GUI 子系统构建（见 build.rs）：由 supervisor/TSF 以子进程方式拉起时不挂
// 控制台窗口，不会出现在任务栏；`--once` 命令行模式下输出走 stdout 不受影响。
#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::process::exit;
use std::time::Duration;

use ime_ipc::pipe::{PipeServer, PIPE_NAME};

fn log(msg: &str) {
    eprintln!("[algo] {msg}");
}

// ---------------------------------------------------------------------------
// wave 4 预留：输入方案热重载。
//
// 目标：options.json 里 `input_scheme` 字段变更后，wave 5 将触发 librime
// 的 schema deploy（替换 shared_data_dir 指向方案目录，重建引擎 / 会话）。
// wave 4 此处只完成"加载 options.json 并记日志"，不做任何引擎动作。
// 纯判定函数 input_scheme_differs 的测试见 platforms/windows/src/service.rs。
// ---------------------------------------------------------------------------

mod mru;

/// 供 service 调用：按会话的 raw composition 作 key 做 MRU boost。
/// `pinyin` 由调用方提供；当前会话层由 ime-ipc 上游（TSF 宿主）记录，
/// 本服务仅按上下文里"已选"回调『记录』。MRU 查询在候选返回后做。
fn mru_store() -> &'static std::sync::Mutex<mru::MruStore> {
    static INSTANCE: std::sync::OnceLock<std::sync::Mutex<mru::MruStore>> =
        std::sync::OnceLock::new();
    INSTANCE.get_or_init(|| std::sync::Mutex::new(mru::MruStore::load()))
}

/// 处理一个候选 list 并返回按 MRU 提升后的新列表（不改 librime 原序的剩余部分）。
fn mru_boost_candidates(pinyin: &str, candidates: Vec<String>) -> Vec<String> {
    mru_store()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .boost(pinyin, candidates)
}

/// 处理一次提交：把选中的词记入 MRU。
fn mru_record_commit(pinyin: &str, committed: &str) {
    mru_store()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .record(pinyin, committed);
    let _ = mru_store().lock().unwrap_or_else(|p| p.into_inner()).save();
}

/// ProcessKey 应答装饰器（经 ime-ipc::server::serve_connection 每键调用）：
/// - 记录：上屏文本非空时，以提交前的 raw 拼音为 key 记入 MRU；
/// - 提频：当前组合有候选时，以 raw 拼音为 key 把最近选过的词前置。
///
/// 历史坑：此前 MRU 只在 main.rs 定义了 store/boost/record 却从未接线到
/// 请求路径（死代码），用户"最近选过的词"永不前置（2026-08-14 发现）。
/// 接入点选在服务端应答前统一装饰，避免在 TSF 客户端重复实现。
fn decorate_process_key(
    raw_before: &str,
    raw_after: &str,
    resp: ime_ipc::Response,
) -> ime_ipc::Response {
    let ime_ipc::Response::ProcessKey {
        eaten,
        commit,
        mut context,
    } = resp
    else {
        return resp;
    };
    // 记录：commit 已取走、组合已被引擎清空，必须用提交前的 raw
    if let Some(text) = commit.as_deref() {
        if !text.is_empty() {
            mru_record_commit(raw_before, text);
        }
    }
    // 提频：候选按 MRU 重排，高亮跟随原高亮词的新位置
    if !context.candidates.is_empty() && !raw_after.is_empty() {
        let original = context.candidates.clone();
        let boosted =
            mru_boost_candidates(raw_after, original.iter().map(|c| c.text.clone()).collect());
        if boosted.len() == original.len() {
            let hl_text = original.get(context.highlighted).map(|c| c.text.clone());
            context.candidates = boosted
                .iter()
                .map(|text| {
                    original
                        .iter()
                        .find(|c| &c.text == text)
                        .cloned()
                        .unwrap_or_else(|| ime_ipc::Candidate {
                            text: text.clone(),
                            comment: String::new(),
                        })
                })
                .collect();
            if let Some(t) = hl_text {
                if let Some(pos) = context.candidates.iter().position(|c| c.text == t) {
                    context.highlighted = pos;
                }
            }
        }
    }
    ime_ipc::Response::ProcessKey {
        eaten,
        commit,
        context,
    }
}

/// 比对 options.json 前后两份快照的 input_scheme 是否不同。同一份判定逻辑
/// 同时被 TSF (service.rs) 与 algo (此处) 消费；本函数本身留在 algo 是因为
/// algo 是 wave 5 真实 redeploy 的宿主，TSF 只是日志转发。
fn input_scheme_differs(a: &shurufa_options::ImeOptions, b: &shurufa_options::ImeOptions) -> bool {
    a.input_scheme != b.input_scheme
}

/// 把 options.json 的 `input_scheme` 映射为 librime schema_id。
/// 与 schemas/ 目录下 schema 的 schema_id 一一对应；未知值回退 pinyin
/// （与 ImeOptions::default 的 input_scheme 一致）。
fn schema_id_for(scheme: &str) -> &'static str {
    match scheme {
        "double_pinyin" => "shurufa_double_pinyin",
        "wubi" => "shurufa_wubi",
        "cangjie" => "shurufa_cangjie",
        _ => "rime_ice",
    }
}

/// 输入方案热切换：2 秒轮询 options.json，方案变化时把最新 schema_id
/// 写入共享槽；每个新会话创建后按槽值 select_schema。
fn watch_input_scheme(
    last_known: std::sync::Arc<std::sync::Mutex<shurufa_options::ImeOptions>>,
    current_scheme: std::sync::Arc<std::sync::Mutex<String>>,
) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        let current = shurufa_options::load();
        let old = {
            let mut guard = last_known.lock().unwrap_or_else(|p| p.into_inner());
            let stale = guard.clone();
            *guard = current.clone();
            stale
        };
        if input_scheme_differs(&old, &current) {
            let sid = schema_id_for(&current.input_scheme);
            {
                let mut slot = current_scheme.lock().unwrap_or_else(|p| p.into_inner());
                *slot = sid.to_owned();
            }
            log(&format!(
                "input_scheme 变化：{} → {}（schema={}），新会话将热切换",
                old.input_scheme, current.input_scheme, sid
            ));
        }
    });
}

/// 创建会话并按当前方案选择 schema（如果非默认 pinyin）。
fn create_session_with_scheme(
    engine: &'static ime_bridge::Engine,
    current_scheme: &std::sync::Arc<std::sync::Mutex<String>>,
) -> Result<ime_bridge::Session<'static>, String> {
    let session = engine.create_session()?;
    let scheme = current_scheme
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    if scheme != "rime_ice" {
        if !session.select_schema(&scheme) {
            log(&format!("select_schema({scheme}) 失败，回退 rime_ice"));
        } else {
            log(&format!("会话已切换方案：{scheme}"));
        }
    }
    Ok(session)
}

/// 共享数据目录：优先取 `SHURUFA_SCHEMAS` 环境变量，否则沿 exe 上级找 schemas，
/// 最后回落到 `%APPDATA%\shurufa\schemas`。
fn shared_data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SHURUFA_SCHEMAS") {
        return PathBuf::from(p);
    }
    for dir in std::env::current_exe().ok().into_iter().flat_map(|exe| {
        let mut d = exe;
        let mut out = Vec::new();
        while let Some(p) = d.parent() {
            out.push(p.to_path_buf());
            d = p.to_path_buf();
        }
        out
    }) {
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

/// 初始化引擎（含词典部署）。
fn init_engine() -> ime_bridge::Engine {
    let shared = shared_data_dir();
    let user = user_config_root().join("rime");
    log(&format!("初始化引擎：shared={}", shared.display()));
    match ime_bridge::Engine::init(&shared, &user) {
        Ok(e) => {
            log("引擎就绪");
            e
        }
        Err(e) => {
            log(&format!("引擎初始化失败：{e}"));
            exit(2);
        }
    }
}

/// 进入常驻服务循环：反复新建管道实例、接受连接、服务请求。
fn run_service() -> ! {
    let engine = init_engine();
    let engine: &'static ime_bridge::Engine = Box::leak(Box::new(engine));
    // 输入方案热切换：2 秒轮询 options.json，把最新 schema_id 写入共享槽；
    // 每个新会话创建后 select_schema（见 create_session_with_scheme）。
    let shared_opts = std::sync::Arc::new(std::sync::Mutex::new(shurufa_options::load()));
    let current_scheme = std::sync::Arc::new(std::sync::Mutex::new(String::from(schema_id_for(
        &shared_opts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .input_scheme,
    ))));
    watch_input_scheme(shared_opts, current_scheme.clone());
    log(&format!("监听 {} …", PIPE_NAME));
    loop {
        let server = match PipeServer::create() {
            Ok(s) => s,
            Err(e) => {
                log(&format!("创建管道失败：{e}；退出"));
                exit(3);
            }
        };
        if let Err(e) = server.accept() {
            log(&format!("接受连接失败：{e}；继续"));
            server.reset();
            continue;
        }
        log("收到连接，服务会话…");
        // 每个 TSF 宿主都会长期持有一个连接。接受后立即回到循环创建下一个
        // 管道实例，连接处理在独立线程中进行，避免首个宿主阻塞全部后续宿主。
        let scheme_slot = current_scheme.clone();
        std::thread::spawn(move || {
            ime_ipc::server::serve_connection(
                &server,
                move || create_session_with_scheme(engine, &scheme_slot),
                decorate_process_key,
            );
            log("会话结束");
        });
    }
}

/// `--once` 模式：喂一串键序列打印候选（供自检/被 supervisor 拉起时冒烟）。
fn run_once(keys: &str) -> ! {
    let root = user_config_root();
    let shared = shared_data_dir();
    log(&format!("--once 模式，键序列：{keys}"));
    let engine = match ime_bridge::Engine::init(&shared, &root.join("rime")) {
        Ok(e) => e,
        Err(e) => {
            log(&format!("引擎初始化失败：{e}"));
            exit(2);
        }
    };
    let session = match engine.create_session() {
        Ok(s) => s,
        Err(e) => {
            log(&format!("创建会话失败：{e}"));
            exit(2);
        }
    };
    if !session.simulate(keys) {
        log("键序列未被引擎接受");
        exit(4);
    }
    let ctx = session.context();
    println!("preedit: {}", ctx.preedit);
    for (i, c) in ctx.candidates.iter().enumerate() {
        println!("  {}: {} {}", i, c.text, c.comment);
    }
    exit(0);
}

/// 算法服务单实例锁：同进程内保持持有直至退出，避免两个算法服务抢
/// 用户词库锁。supervisor 用探测方式判断是否已有算法服务在跑。
const ALGO_MUTEX: &str = r"Global\shurufa-algo";

fn hold_algo_lock() {
    use windows::Win32::Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    let name = windows::core::HSTRING::from(ALGO_MUTEX);
    static HELD: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    unsafe {
        let handle = match CreateMutexW(None, true, &name) {
            Ok(h) => h,
            Err(e) => {
                log(&format!("创建算法服务单实例锁失败：{e}"));
                exit(0);
            }
        };
        let r = WaitForSingleObject(handle, 0);
        if r == WAIT_OBJECT_0 || r == WAIT_ABANDONED {
            // 本进程成为唯一算法服务；把句柄存入 static 并永久持有，
            // 进程退出时由系统回收（HANDLE 是 Copy，mem::forget 无效）。
            let _ = HELD.set(());
            let _ = &handle; // 保持引用，防止被优化掉
            return;
        }
        let _ = CloseHandle(handle);
    }
    log("已有算法服务在运行，本实例退出");
    exit(0);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--once") => {
            // 命令行回归模式：若系统没分控制台（我们从 GUI 子系统启动），
            // 临时 attach 到父进程控制台让 stdout/stderr 可见。
            attach_console_to_parent();
            let keys = args.get(1).map(String::as_str).unwrap_or("nihao");
            run_once(keys);
        }
        _ => {
            // 常驻模式：先抢单实例锁，抢不到直接退出（已有算法服务在跑）
            hold_algo_lock();
            run_service();
        }
    }
}

/// windows_subsystem="windows" 构建下 stdout/stderr 默认无宿主；我们只在
/// `--once` 调用时把句柄接到调用方控制台（cmd/pwsh），其余常驻分支不接。
#[cfg(windows)]
fn attach_console_to_parent() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        // 父进程不是控制台时返回 Err；静默忽略，因为此时没人会看 stdout。
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console_to_parent() {}
