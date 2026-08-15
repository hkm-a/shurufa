//! 守护监管层（shurufa-supervisor）。
//!
//! 解决桌面常驻进程长期欠缺的整体性命周期管理：
//! - **单实例强制**：`run`、`supervise`、算法服务都用命名 Mutex 保证唯一，
//!   杜绝“两个 host 抢端口/热键回退”与“两个引擎抢用户词库锁”这类实测问题。
//! - **自动自愈**：supervisor 作为父进程看护 worker（`run`）与算法服务
//!   （`shurufa-algo`，librime 引擎所在进程），异常退出（非 0 退出码）按
//!   退避策略自动重启；正常退出则不重启。
//! - **统一状态**：健康信息（PID、重启计数、端口、启动时间）写入
//!   `%APPDATA%\shurufa\daemon.state`，`status` 子命令读取。
//! - **统一停机**：`stop` 子命令写停止令牌，supervisor 读到后先结束 worker
//!   与算法服务，再清理退出，替代“窗口外乱杀进程”。

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{
    CreateMutexW, OpenProcess, TerminateProcess, PROCESS_TERMINATE,
};

/// 监督进程单实例锁名。
const SUPERVISOR_MUTEX: &str = "Global\\shurufa-host-supervisor";
/// worker（run）单实例锁名。
pub const WORKER_MUTEX: &str = "Global\\shurufa-host-worker";
/// 算法服务（shurufa-algo）单实例锁名。
const ALGO_MUTEX: &str = "Global\\shurufa-algo";
/// 过快重启时最大幂次退避（秒上限）。
const BACKOFF_CAP: u32 = 5;

/// 算法服务健康探针间隔：进程存活 ≠ 服务健康，serve 线程死锁（如引擎锁被
/// 永久占用）时进程活着但 IPC 全线无响应。supervisor 若只看进程退出码，
/// 永远检测不到这类"卡死"（2026-08-12 实测：toggle_ascii 嵌套锁自死锁，
/// 全部请求 2s 超时，用户 Shift 卡 500ms）。
const ALGO_PROBE_INTERVAL: Duration = Duration::from_secs(5);
/// 单次探针读响应超时：正常引擎亚毫秒级，500ms 足以区分健康/卡死。
const ALGO_PROBE_TIMEOUT: Duration = Duration::from_millis(500);
/// 连续探针失败达到该次数即判定卡死，杀进程触发重启。
const ALGO_PROBE_FAIL_LIMIT: u32 = 2;
/// 算法服务启动宽限期：期间只做进程存活检查，不做健康探针——引擎首次
/// 部署（词典编译，数十秒）在管道监听之前，此时探针必失败，须避免误杀。
const ALGO_GRACE_PERIOD: Duration = Duration::from_secs(30);

/// 健康探针：连接算法服务管道，发一个轻量请求并在超时预算内读到合法应答。
/// 任何一环失败（连不上 / 写不进 / 读超时 / 应答非法）都视为不健康。
fn algo_health_check() -> bool {
    use ime_ipc::pipe::PipeClient;
    use ime_ipc::{decode_response, encode_request, Request};
    let Ok(client) = PipeClient::connect() else {
        return false;
    };
    let Ok(frame) = encode_request(&Request::GetOption("ascii_mode".to_string())) else {
        return false;
    };
    if client.write_frame(&frame).is_err() {
        return false;
    }
    match client.read_frame_timeout(ALGO_PROBE_TIMEOUT) {
        Ok(f) => decode_response(&f).is_ok(),
        Err(_) => false,
    }
}

/// 持有的命名 Mutex 句柄；Drop 时释放。
pub struct SingletonLock(HANDLE);

impl Drop for SingletonLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// 尝试获取命名 Mutex。
///
/// 返回 `Ok(None)` 表示“已存在持有该名字的进程”；返回 `Ok(Some)` 表示本进程
/// 成为该 Mutex 的唯一持有者。
pub fn acquire_singleton(name: &str) -> std::io::Result<Option<SingletonLock>> {
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::WaitForSingleObject;
    let mutex = unsafe { CreateMutexW(None, true, &HSTRING::from(name)) }?;
    // bInitialOwner=true：若这是新创建的命名 Mutex，我们立即拥有它；若已存在，
    // 我们拿不到所有权。用 WaitForSingleObject(0) 探测所有权：
    //  - 已拥有 → WAIT_OBJECT_0
    //  - 原主进程崩溃被系统接管 → WAIT_ABANDONED（视为我们拿到，接管继续）
    //  - 被他人持有 → WAIT_TIMEOUT（需释放句柄）
    let r = unsafe { WaitForSingleObject(mutex, 0) };
    let owned = r == WAIT_OBJECT_0 || r == windows::Win32::Foundation::WAIT_ABANDONED;
    if owned {
        Ok(Some(SingletonLock(mutex)))
    } else {
        drop(SingletonLock(mutex));
        Ok(None)
    }
}

/// Debug 隔离验收可指定独立 worker 锁，避免与用户常驻进程互相阻塞。
pub fn worker_mutex_name() -> String {
    #[cfg(any(debug_assertions, test))]
    {
        debug_worker_mutex(std::env::var("SHURUFA_TEST_WORKER_MUTEX").ok())
    }

    #[cfg(not(any(debug_assertions, test)))]
    WORKER_MUTEX.to_owned()
}

#[cfg(any(debug_assertions, test))]
fn debug_worker_mutex(value: Option<String>) -> String {
    value
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| WORKER_MUTEX.to_owned())
}

fn state_path() -> PathBuf {
    crate::app_data_dir().join("daemon.state")
}

fn stop_token_path() -> PathBuf {
    crate::app_data_dir().join("stop.token")
}

pub fn write_state(status: &str, worker_pid: Option<u32>, algo_pid: Option<u32>, restarts: u32) {
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file = state_path();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut out = String::new();
    out.push_str(&format!("status={status}\n"));
    out.push_str(&format!("supervisor_pid={}\n", std::process::id()));
    out.push_str(&format!(
        "worker_pid={}\n",
        worker_pid.map(|p| p.to_string()).unwrap_or_default()
    ));
    out.push_str(&format!(
        "algo_pid={}\n",
        algo_pid.map(|p| p.to_string()).unwrap_or_default()
    ));
    out.push_str(&format!("started_at={started}\n"));
    out.push_str(&format!("restarts={restarts}\n"));
    out.push_str(&format!("port={}\n", crate::sync::sync_port()));
    let _ = std::fs::write(&file, out);
}

/// 状态文件中不把尚未由看护线程登记的 PID 0 伪装成有效进程。
fn tracked_pid(pid: &AtomicU32) -> Option<u32> {
    match pid.load(Ordering::SeqCst) {
        0 => None,
        value => Some(value),
    }
}

/// 按 PID 强制结束进程（仅用于停机清理算法服务；worker 由 Child 句柄处理）。
pub fn kill_pid(pid: u32) {
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) {
            let _ = TerminateProcess(handle, 1);
            let _ = CloseHandle(handle);
        }
    }
}

/// 当前 exe 同目录下的兄弟可执行文件（shurufa-algo.exe）。
fn algo_exe_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("shurufa-algo.exe")))
        .unwrap_or_else(|| PathBuf::from("shurufa-algo.exe"))
}

/// 探测算法服务单实例锁：已被他人持有返回 true（跳过拉起）。
/// 锁由算法服务进程自己持有（见 shurufa-algo），探测后立即释放句柄。
fn algo_lock_held() -> bool {
    match acquire_singleton(ALGO_MUTEX) {
        Ok(Some(_)) => false, // 我们能拿到 → 无人在跑
        Ok(None) => true,     // 拿不到 → 已有算法服务在跑
        Err(e) => {
            crate::log_line(&format!("探测算法服务锁失败：{e}"));
            true // 保守：不重复拉起
        }
    }
}

/// 拉一个算法服务进程；若已有算法服务在跑则返回 None。
fn spawn_algo() -> Option<Child> {
    if algo_lock_held() {
        crate::log_line("检测到算法服务已在运行，跳过拉起");
        return None;
    }
    let exe = algo_exe_path();
    if !exe.exists() {
        crate::log_line(&format!("算法服务不存在：{}", exe.display()));
        return None;
    }
    match Command::new(&exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => {
            crate::log_line(&format!("算法服务已启动 pid={}", c.id()));
            Some(c)
        }
        Err(e) => {
            crate::log_line(&format!("spawn 算法服务失败：{e}"));
            None
        }
    }
}

/// `supervise` 子命令：常驻监管循环。
pub fn supervise() -> ! {
    match acquire_singleton(SUPERVISOR_MUTEX) {
        Ok(None) => {
            eprintln!("已有 supervisor 在运行；如需重启请先 stop。");
            std::process::exit(1);
        }
        Ok(Some(handle)) => {
            // 必须持有锁直到进程退出（与 run 的 mem::forget 一致）；若这里 drop，
            // 会释放最后一个 mutex 句柄导致锁被销毁，第二个 supervise 就能再抢到。
            std::mem::forget(handle);
        }
        Err(e) => {
            eprintln!("无法创建 supervisor 单实例锁：{e}");
            std::process::exit(1);
        }
    }
    crate::log_line("supervisor 启动，进入监管循环");
    // stop 子命令通过令牌请求当前监管器退出。新的启动必须清掉上一次
    // 已处理或中途遗留的令牌，否则“停止后再启动”会立刻再次停机。
    let _ = std::fs::remove_file(stop_token_path());
    write_state("starting", None, None, 0);

    let exe = std::env::current_exe().expect("获取本进程路径失败");
    let mut restarts: u32 = 0;
    // 算法服务看护：stop 标志 + 当前 pid（供停机时按 pid 结束）
    let algo_stop = Arc::new(AtomicBool::new(false));
    let algo_pid: Arc<std::sync::atomic::AtomicU32> =
        Arc::new(std::sync::atomic::AtomicU32::new(0));
    // worker pid 也放进共享单元，供看护线程刷新状态文件
    let worker_pid_cell: Arc<std::sync::atomic::AtomicU32> =
        Arc::new(std::sync::atomic::AtomicU32::new(0));

    // 先确保算法服务在线（引擎会话需要它）
    {
        let stop = algo_stop.clone();
        let pid = algo_pid.clone();
        let wcell = worker_pid_cell.clone();
        std::thread::spawn(move || {
            let mut algo_restarts: u32 = 0;
            while !stop.load(Ordering::SeqCst) {
                let mut child = match spawn_algo() {
                    Some(c) => {
                        pid.store(c.id(), Ordering::SeqCst);
                        write_state(
                            "running",
                            {
                                let w = wcell.load(Ordering::SeqCst);
                                if w == 0 {
                                    None
                                } else {
                                    Some(w)
                                }
                            },
                            Some(c.id()),
                            algo_restarts,
                        );
                        c
                    }
                    None => {
                        std::thread::sleep(Duration::from_secs(3));
                        continue;
                    }
                };
                // 健康探针状态：宽限期从本次 spawn 起算；失败计数跨 restart 重置
                let spawn_time = std::time::Instant::now();
                let mut probe_last = std::time::Instant::now();
                let mut probe_failures: u32 = 0;
                // 等待算法服务退出或判定卡死
                loop {
                    if stop.load(Ordering::SeqCst) {
                        let _ = child.kill();
                        let _ = child.wait();
                        return;
                    }
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            algo_restarts += 1;
                            crate::log_line(&format!(
                                "算法服务退出（code={:?}），第 {algo_restarts} 次重启",
                                status.code()
                            ));
                            pid.store(0, Ordering::SeqCst);
                            write_state(
                                "restarting",
                                {
                                    let w = wcell.load(Ordering::SeqCst);
                                    if w == 0 {
                                        None
                                    } else {
                                        Some(w)
                                    }
                                },
                                None,
                                algo_restarts,
                            );
                            break;
                        }
                        Ok(None) => {
                            // 进程还活着：宽限期过后周期性做健康探针，连续失败判定卡死
                            if spawn_time.elapsed() >= ALGO_GRACE_PERIOD
                                && probe_last.elapsed() >= ALGO_PROBE_INTERVAL
                            {
                                probe_last = std::time::Instant::now();
                                if algo_health_check() {
                                    probe_failures = 0;
                                } else {
                                    probe_failures += 1;
                                    crate::log_line(&format!(
                                        "算法服务健康探针失败 {probe_failures}/{ALGO_PROBE_FAIL_LIMIT}"
                                    ));
                                    if probe_failures >= ALGO_PROBE_FAIL_LIMIT {
                                        crate::log_line("算法服务无响应（疑似卡死），强制重启");
                                        algo_restarts += 1;
                                        pid.store(0, Ordering::SeqCst);
                                        write_state(
                                            "restarting",
                                            {
                                                let w = wcell.load(Ordering::SeqCst);
                                                if w == 0 {
                                                    None
                                                } else {
                                                    Some(w)
                                                }
                                            },
                                            None,
                                            algo_restarts,
                                        );
                                        let _ = child.kill();
                                        let _ = child.wait();
                                        break;
                                    }
                                }
                            }
                            std::thread::sleep(Duration::from_millis(250));
                        }
                        Err(_) => {
                            algo_restarts += 1;
                            crate::log_line("算法服务监视出错，重启");
                            break;
                        }
                    }
                }
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_secs(backoff_secs(algo_restarts)));
            }
        });
    }

    // 停机辅助：置 stop 标志并按 pid 结束算法服务（watchdog 线程会回收）
    fn stop_algo(stop: &Arc<AtomicBool>, pid: &Arc<std::sync::atomic::AtomicU32>) {
        stop.store(true, Ordering::SeqCst);
        let p = pid.load(Ordering::SeqCst);
        if p != 0 {
            kill_pid(p);
        }
    }

    loop {
        crate::log_line("supervisor 拉起 worker（run）…");
        let worker = Command::new(&exe)
            .arg("run")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut worker: Child = match worker {
            Ok(c) => c,
            Err(e) => {
                crate::log_line(&format!("spawn worker 失败：{e}，退避后重试"));
                restarts += 1;
                write_state("restarting", None, tracked_pid(&algo_pid), restarts);
                std::thread::sleep(Duration::from_secs(backoff_secs(restarts)));
                continue;
            }
        };
        let worker_pid = worker.id();
        worker_pid_cell.store(worker_pid, Ordering::SeqCst);
        crate::log_line(&format!("worker 已启动 pid={worker_pid}"));
        write_state(
            "running",
            Some(worker_pid),
            tracked_pid(&algo_pid),
            restarts,
        );

        // 轮询：worker 退出或被 stop。
        let outcome = wait_worker_or_stop(&mut worker);

        match outcome {
            Outcome::Stopped => {
                let _ = worker.kill();
                let _ = worker.wait();
                // 停止算法服务
                stop_algo(&algo_stop, &algo_pid);
                let _ = std::fs::remove_file(stop_token_path());
                write_state("stopped", None, None, restarts);
                crate::log_line("supervisor 已退出");
                std::process::exit(0);
            }
            Outcome::Exit { code: Some(0) } => {
                crate::log_line("worker 正常退出（受控停机），不再重启");
                stop_algo(&algo_stop, &algo_pid);
                write_state("stopped", None, None, restarts);
                std::process::exit(0);
            }
            Outcome::Exit { code } => {
                restarts += 1;
                crate::log_line(&format!(
                    "worker 异常退出（code={:?}），第 {restarts} 次重启",
                    code
                ));
                write_state("restarting", None, tracked_pid(&algo_pid), restarts);
                std::thread::sleep(Duration::from_secs(backoff_secs(restarts)));
            }
        }
    }
}

enum Outcome {
    Stopped,
    Exit { code: Option<i32> },
}

/// 轮询直到 worker 退出或出现停止令牌。
fn wait_worker_or_stop(worker: &mut Child) -> Outcome {
    loop {
        if stop_token_present() {
            return Outcome::Stopped;
        }
        match worker.try_wait() {
            Ok(Some(status)) => {
                return Outcome::Exit {
                    code: status.code(),
                }
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(250)),
            // try_wait 出错视为异常退出。
            Err(_) => return Outcome::Exit { code: None },
        }
    }
}

fn backoff_secs(restarts: u32) -> u64 {
    // 1,2,4,8,16 秒封顶
    (1u64 << restarts.min(BACKOFF_CAP).saturating_sub(1)).min(16)
}

fn stop_token_present() -> bool {
    stop_token_path().is_file()
}

/// `stop` 子命令：写入停止令牌，触发 supervisor 退出。
pub fn cmd_stop() {
    let path = stop_token_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&path, b"stop\n") {
        Ok(()) => {
            std::thread::sleep(Duration::from_millis(300));
            println!("已发出停机信号。若 supervisor 未运行则无影响。");
        }
        Err(e) => {
            eprintln!("写入停止令牌失败：{e}");
            std::process::exit(1);
        }
    }
}

/// `status` 子命令：读取并展示监管状态。
pub fn cmd_status() {
    let path = state_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => {
            println!("尚无监管状态（supervisor 未运行或从未启动）。");
            return;
        }
    };
    let mut map = std::collections::HashMap::new();
    for line in raw.lines() {
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim(), v.trim());
        }
    }
    let status = map.get("status").copied().unwrap_or("unknown");
    println!("状态       : {status}");
    println!("supervisor: {}", map.get("supervisor_pid").unwrap_or(&"-"));
    println!("worker    : {}", map.get("worker_pid").unwrap_or(&"-"));
    println!("算法服务  : {}", map.get("algo_pid").unwrap_or(&"-"));
    println!("端口      : {}", map.get("port").unwrap_or(&"-"));
    println!("重启次数  : {}", map.get("restarts").unwrap_or(&"0"));
    if let Some(started) = map.get("started_at") {
        if let Ok(ts) = started.parse::<u64>() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            println!("运行时长  : {} 秒", now.saturating_sub(ts));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;

    use super::{debug_worker_mutex, tracked_pid, WORKER_MUTEX};

    #[test]
    fn 未登记的算法服务不写成零号进程() {
        assert_eq!(tracked_pid(&AtomicU32::new(0)), None);
        assert_eq!(tracked_pid(&AtomicU32::new(2468)), Some(2468));
    }

    #[test]
    fn 调试隔离锁仅接受非空名称() {
        assert_eq!(debug_worker_mutex(None), WORKER_MUTEX);
        assert_eq!(debug_worker_mutex(Some(" ".to_owned())), WORKER_MUTEX);
        assert_eq!(
            debug_worker_mutex(Some(
                "Global\\shurufa-background-sync-worker-48634".to_owned()
            )),
            "Global\\shurufa-background-sync-worker-48634"
        );
    }
}
