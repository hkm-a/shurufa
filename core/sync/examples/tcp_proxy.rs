//! 坏网几何 TCP 代理：listen 一个本地端口，对每条入站连接与目标建立
//! 真实 TCP 连接，然后在两个方向各自套一层 `Wan`（延迟 + 抖动 + 概率延迟
//! ⇒ 模拟丢包重传/突发拥塞）。
//!
//! 用法：
//!   cargo run --release -p sync-core --example tcp_proxy -- \
//!       --listen 127.0.0.1:40001 --target 127.0.0.1:40000 \
//!       --latency-ms 200 --jitter-ms 50 --drop-pct 5 [--reorder]
//!
//! 参数语义（均为每条消息每次 read/write 采样）：
//!   --latency-ms N   固定单向延迟（毫秒）
//!   --jitter-ms N    在 [latency, latency+jitter] 区间均匀抖动
//!   --drop-pct  P    以 P% 概率把一个 chunk 额外扣住 latency+jitter，
//!                    等价于"重传后才到"（TCP 不会真丢，只是延迟变大）
//!   --corrupt-pct P  （仅记录日志；端到端 TLS/AEAD 下注入损坏无意义，
//!                    真实的 bit error 由 TLS 记录层直接判失败，故不支持）
//!   --reorder        允许同一连接的两个方向在同一段时间窗内交错，
//!                    模拟乱序到达导致的双向竞速（协议层有序性仍由 TCP 保证）。
//!
//! 退出：Ctrl+C；每条连接进程内独立采样，互不干扰。

use std::net::SocketAddr;
use std::time::Duration;

use sync_core::wan::{Wan, WanProfile};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug)]
struct Args {
    listen: String,
    target: String,
    latency_ms: u64,
    jitter_ms: u64,
    drop_pct: f64,
    corrupt_pct: f64,
    reorder: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut listen = None;
    let mut target = None;
    let mut latency_ms = 0;
    let mut jitter_ms = 0;
    let mut drop_pct = 0.0;
    let mut corrupt_pct = 0.0;
    let mut reorder = false;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = it.next();
        match flag.as_str() {
            "--listen" => listen = value,
            "--target" => target = value,
            "--latency-ms" => {
                latency_ms = value
                    .and_then(|v| v.parse().ok())
                    .ok_or("--latency-ms 需要整数毫秒")?
            }
            "--jitter-ms" => {
                jitter_ms = value
                    .and_then(|v| v.parse().ok())
                    .ok_or("--jitter-ms 需要整数毫秒")?
            }
            "--drop-pct" => {
                drop_pct = value
                    .and_then(|v| v.parse().ok())
                    .ok_or("--drop-pct 需要 0..100 数字")?
            }
            "--corrupt-pct" => {
                corrupt_pct = value
                    .and_then(|v| v.parse().ok())
                    .ok_or("--corrupt-pct 需要 0..100 数字")?
            }
            "--reorder" => reorder = true,
            other => return Err(format!("未知参数 {other}")),
        }
    }
    let listen = listen.ok_or("缺 --listen host:port")?;
    let target = target.ok_or("缺 --target host:port")?;
    if !(0.0..=100.0).contains(&drop_pct) {
        return Err("--drop-pct 必须在 0..100".into());
    }
    if !(0.0..=100.0).contains(&corrupt_pct) {
        return Err("--corrupt-pct 必须在 0..100".into());
    }
    Ok(Args {
        listen,
        target,
        latency_ms,
        jitter_ms,
        drop_pct,
        corrupt_pct,
        reorder,
    })
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("tcp_proxy: {e}");
            eprintln!("用法: tcp_proxy --listen <addr> --target <addr> [--latency-ms N] [--jitter-ms N] [--drop-pct P] [--corrupt-pct P] [--reorder]");
            std::process::exit(2);
        }
    };
    if args.corrupt_pct > 0.0 {
        eprintln!(
            "tcp_proxy: 注意 --corrupt-pct={} 在 TLS 之上会被 AEAD 记录层判负，工具仅记录不注入",
            args.corrupt_pct
        );
    }

    let listen_addr: SocketAddr = args.listen.parse().expect("--listen 格式 host:port");
    let target: SocketAddr = args.target.parse().expect("--target 格式 host:port");
    let listener = TcpListener::bind(listen_addr)
        .await
        .expect("绑定 listen 失败");
    let profile = WanProfile {
        latency: Duration::from_millis(args.latency_ms),
        jitter: Duration::from_millis(args.jitter_ms),
        // drop_pct → 额外驻留概率，由 delay_prob 承担
        delay_prob: (args.drop_pct / 100.0).clamp(0.0, 1.0),
        reorder: args.reorder,
    };
    println!(
        "tcp_proxy listening on {} → {} | latency={}ms jitter={}ms drop={:.1}% reorder={}",
        listen_addr, target, args.latency_ms, args.jitter_ms, args.drop_pct, args.reorder
    );

    loop {
        let (inbound, peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("accept 失败: {e}");
                continue;
            }
        };
        let target = target.to_string();
        let profile = profile.clone();
        tokio::spawn(async move {
            match TcpStream::connect(&target).await {
                Ok(outbound) => {
                    if let Err(e) = pipe(inbound, outbound, profile).await {
                        eprintln!("连接 {peer} 结束: {e}");
                    }
                }
                Err(e) => eprintln!("连不上 target {target}: {e}"),
            }
        });
    }
}

async fn pipe(a: TcpStream, b: TcpStream, profile: WanProfile) -> Result<(), String> {
    let (ar, aw) = a.into_split();
    let (br, bw) = b.into_split();

    let mut a_to_b = Wan::new(ar, profile.clone());
    let mut b_to_a = Wan::new(br, profile);

    let mut aw = aw;
    let mut bw = bw;

    let forward_ab = async move {
        let mut buf = [0u8; 16 * 1024];
        loop {
            let n = a_to_b
                .read(&mut buf)
                .await
                .map_err(|e| format!("读客户端失败: {e}"))?;
            if n == 0 {
                return Ok::<(), String>(());
            }
            bw.write_all(&buf[..n])
                .await
                .map_err(|e| format!("写目标失败: {e}"))?;
        }
    };
    let forward_ba = async move {
        let mut buf = [0u8; 16 * 1024];
        loop {
            let n = b_to_a
                .read(&mut buf)
                .await
                .map_err(|e| format!("读目标失败: {e}"))?;
            if n == 0 {
                return Ok::<(), String>(());
            }
            aw.write_all(&buf[..n])
                .await
                .map_err(|e| format!("写客户端失败: {e}"))?;
        }
    };

    let _ = tokio::try_join!(forward_ab, forward_ba)?;
    Ok(())
}
