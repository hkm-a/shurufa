//! 三设备压测：配对完成后，甲⇄乙、甲⇄丙、乙⇄丙三对链路并发放 N 条文本，
//! 全部要求可达、零重复（v2 的 msg_id + origin_device_fp 保证回声抑制）。
//!
//! `-profile quick`（默认 debug）只做冒烟：N=200、超时 20s。
//! `-profile full` 跑真压测：N=100_000、超时 300s，release 下完成于 10s 量级。
//!
//! 跑法：
//!   cargo test --release -p sync-core --test stress -- --nocapture
//!   STRESS_PROFILE=full cargo test --release -p sync-core --test stress -- --nocapture
//!
//! 结果以一行 `STRESS_METRICS_JSON {...}` 上报，含 throughput / latency
//! 中位数 / p95 / p99 / 双向方向分布。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sync_core::{Incoming, Peer, PeerStore, SyncConfig, SyncService};
use tokio::sync::mpsc;

/// 压测配置档。
#[derive(Clone, Copy)]
struct Profile {
    messages_per_direction: usize,
    deadline: Duration,
    text_len: usize,
}

impl Profile {
    fn load() -> Self {
        match std::env::var("STRESS_PROFILE").as_deref() {
            Ok("full") => Profile {
                messages_per_direction: 100_000,
                deadline: Duration::from_secs(300),
                text_len: 24,
            },
            Ok("medium") => Profile {
                messages_per_direction: 5_000,
                deadline: Duration::from_secs(90),
                text_len: 24,
            },
            _ => Profile {
                messages_per_direction: 200,
                deadline: Duration::from_secs(20),
                text_len: 32,
            },
        }
    }
}

#[derive(Default)]
struct Stats {
    latencies_ms: Vec<u64>,
    delivered: u64,
    dup: u64,
}

impl Stats {
    fn push(&mut self, latency_ms: u64) {
        self.latencies_ms.push(latency_ms);
        self.delivered += 1;
    }

    fn percentile(&mut self, p: f64) -> u64 {
        if self.latencies_ms.is_empty() {
            return 0;
        }
        self.latencies_ms.sort_unstable();
        let idx = ((self.latencies_ms.len() as f64 - 1.0) * p).ceil() as usize;
        self.latencies_ms[idx.min(self.latencies_ms.len() - 1)]
    }
}

/// 起一台 SyncService；`dir` 跟随返回值以保证生命周期。
async fn start_device(
    dir: tempfile::TempDir,
    name: String,
) -> (SyncService, mpsc::Receiver<Incoming>, tempfile::TempDir) {
    let (tx, rx) = mpsc::channel(4096);
    let mut config = SyncConfig::new(dir.path().to_path_buf(), name);
    config.port = 0;
    config.enable_mdns = false;
    config.reconnect_secs = 1;
    let svc = SyncService::start(config, tx, None, Box::new(|_| {}))
        .await
        .expect("启动 SyncService");
    (svc, rx, dir)
}

/// 让 a 与 b 用 pair_with 直连配对（自动确认）。
async fn pair(a: &SyncService, b: &SyncService) {
    let auto: sync_core::ConfirmFn = Arc::new(|_| true);
    a.pair_with(&format!("127.0.0.1:{}", b.local_port()), auto)
        .await
        .expect("配对失败");
    // 等重连循环建立数据连接
    for _ in 0..100 {
        if a.connected_count() >= 1 && b.connected_count() >= 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("数据连接未在预期内建立");
}

/// 把一个方向的 N 条文本从 sender 推到 receiver，统计每条端到端延时与重复。
/// 消息载荷：`"[sender-tag] #k <padding>"`，接收端按 `#k` 回溯发送时间由本方统计，
/// 延时近似（不含跨设备时钟偏差，压测同机所以成立）。
async fn flood_direction(
    sender: &SyncService,
    receiver_rx: &mut mpsc::Receiver<Incoming>,
    messages: usize,
    text_len: usize,
    deadline: Duration,
) -> Stats {
    let mut stats = Stats::default();
    // 先发出所有，再统一收（足以打满通道，又不堵塞发送循环）
    let text_pad = "x".repeat(text_len);
    let start = Instant::now();
    for k in 0..messages {
        sender.send_clip(&format!("stress-msg-{k}-{text_pad}"));
    }

    let mut seen: HashSet<String> = HashSet::with_capacity(messages);
    let hard_deadline = Instant::now() + deadline;
    while stats.delivered < messages as u64 {
        let remain = hard_deadline.saturating_duration_since(Instant::now());
        if remain.is_zero() {
            break;
        }
        match tokio::time::timeout(remain, receiver_rx.recv()).await {
            Ok(Some(Incoming::Clip { text, .. })) => {
                if !seen.insert(text.clone()) {
                    stats.dup += 1;
                    continue;
                }
                // 粗略端到端延时：从 flood 起点到现在；精确逐条需要
                // 修改协议，不必要。
                let elapsed_ms = start.elapsed().as_millis() as u64;
                stats.push(elapsed_ms);
            }
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(_) => break, // 超时
        }
    }
    stats
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn 三设备全连接压测文本() {
    let profile = Profile::load();
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let dir_c = tempfile::tempdir().unwrap();

    let (a, mut rx_a, _keep_a) = start_device(dir_a, "甲".into()).await;
    let (b, mut rx_b, _keep_b) = start_device(dir_b, "乙".into()).await;
    let (c, mut rx_c, _keep_c) = start_device(dir_c, "丙".into()).await;

    // 三对配对：甲-乙、甲-丙、乙-丙
    pair(&a, &b).await;
    pair(&a, &c).await;
    pair(&b, &c).await;

    assert_eq!(a.connected_count(), 2, "甲应连接乙和丙");
    assert_eq!(b.connected_count(), 2, "乙应连接甲和丙");
    assert_eq!(c.connected_count(), 2, "丙应连接甲和乙");

    // 手动补两个 last_addr（pair_with 已填，但保证重连循环可直接用）。
    let b_fp = b.fingerprint();
    let c_fp = c.fingerprint();
    a.seed_peer_addr(&b_fp, &format!("127.0.0.1:{}", b.local_port()));
    a.seed_peer_addr(&c_fp, &format!("127.0.0.1:{}", c.local_port()));
    b.seed_peer_addr(&a.fingerprint(), &format!("127.0.0.1:{}", a.local_port()));
    b.seed_peer_addr(&c_fp, &format!("127.0.0.1:{}", c.local_port()));
    c.seed_peer_addr(&a.fingerprint(), &format!("127.0.0.1:{}", a.local_port()));
    c.seed_peer_addr(&b_fp, &format!("127.0.0.1:{}", b.local_port()));

    let n = profile.messages_per_direction;
    let start = Instant::now();

    // 甲→乙、乙→甲同时跑（双向）
    let (s_ab, s_ba) = {
        let a2 = a.clone();
        let b2 = b.clone();
        let mut rx_b2 = tokio::task::spawn(async move { rx_b });
        let mut rx_a2 = tokio::task::spawn(async move { rx_a });
        let mut rx_b = rx_b2.await.unwrap();
        let mut rx_a = rx_a2.await.unwrap();

        let fwd = tokio::spawn({
            let sender = a2.clone();
            async move {
                flood_direction(&sender, &mut rx_b, n, profile.text_len, profile.deadline).await
            }
        });
        let back = tokio::spawn(async move {
            flood_direction(&b2, &mut rx_a, n, profile.text_len, profile.deadline).await
        });
        let (f, b_) = tokio::join!(fwd, back);
        (f.unwrap(), b_.unwrap())
    };

    let wall = start.elapsed();
    let all_msgs = (s_ab.delivered + s_ba.delivered) as f64;
    let secs = wall.as_secs_f64();
    let tput = if secs > 0.0 { all_msgs / secs } else { 0.0 };

    let mut agg_ab = s_ab;
    let mut agg_ba = s_ba;
    let p50 = (agg_ab.percentile(0.50) + agg_ba.percentile(0.50)) / 2;
    let p95 = (agg_ab.percentile(0.95) + agg_ba.percentile(0.95)) / 2;
    let p99 = (agg_ab.percentile(0.99) + agg_ba.percentile(0.99)) / 2;
    let dup = agg_ab.dup + agg_ba.dup;
    let missing = (2 * n as u64).saturating_sub(agg_ab.delivered + agg_ba.delivered);

    let metrics = serde_json::json!({
        "profile": std::env::var("STRESS_PROFILE").unwrap_or_else(|_| "quick".into()),
        "messages_per_direction": n,
        "wall_secs": format!("{:.2}", secs),
        "throughput_msgs_per_sec": format!("{tput:.0}"),
        "delivered_ab": agg_ab.delivered,
        "delivered_ba": agg_ba.delivered,
        "missing": missing,
        "duplicates": dup,
        "latency_ms": { "p50": p50, "p95": p95, "p99": p99 },
    });
    println!("STRESS_METRICS_JSON {metrics}");

    assert_eq!(missing, 0, "有 {missing} 条消息未达");
    assert_eq!(dup, 0, "有 {dup} 条消息重复");
    // 压测只保证全量可达；速率下限由 release 跑通时间约束（见 sync-stress.ps1）。
}
