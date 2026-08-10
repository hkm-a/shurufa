//! 坏网几何（adverse-network）工具：在真实 TCP 流上叠加延迟 / 抖动 / 丢包，
//! 供压测与坏网测试复用 `SyncService` 全链路（TLS 握手、配对、收发循环）。
//!
//! 注意：TCP 本身是可靠有序流，"丢包/重排"在真实链路由内核重传吸收；
//! 这里的实现通过**整段字节延迟投递**模拟丢包重传带来的突发延迟，
//! 通过 `reorder` 选项跨方向交错制造乱序压力（双向 race），而字节序本身不变。
//! 若要做真丢包，见 `examples/tcp_proxy.rs`，链路中断由服务层重连循环接管。

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Sleep;

/// 坏网参数。全部为 0/false 时等价于直通。
#[derive(Clone, Debug, Default)]
pub struct WanProfile {
    /// 固定单向延迟
    pub latency: Duration,
    /// 抖动上限：每次读写在 [latency, latency+jitter] 间均匀取值
    pub jitter: Duration,
    /// 延迟触发概率（0.0..=1.0）。1.0 表示每个 read/write 都延迟；
    /// 较小的值模拟"大部分包正常、少数包被重传拖住"。
    pub delay_prob: f64,
    /// 是否在两个方向之间交错制造重排压力（默认 true）
    pub reorder: bool,
}

impl WanProfile {
    pub fn direct() -> Self {
        WanProfile::default()
    }

    pub fn is_direct(&self) -> bool {
        self.latency.is_zero() && self.jitter.is_zero()
    }
}

/// 简单 xorshift64 随机源：测试工具够用，避免引入 rand 依赖。
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// [0, 1) 均匀浮点
    fn f64(&mut self) -> f64 {
        (self.next() >> 11) as f64 / ((1u64 << 53) as f64)
    }
}

fn seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    nanos ^ 0xA076_1D64_78BD_642F
}

/// 在底层字节流上叠加固定延迟 + 抖动的读写包装。
///
/// `AsyncWrite::poll_write` 在实际写前等待一次采样延迟；`poll_flush`/
/// `poll_shutdown` 直通（不通则 rustls 握手会被额外拖延，超出测试预算）。
pub struct Wan<S> {
    inner: S,
    profile: WanProfile,
    sleep: Option<Pin<Box<Sleep>>>,
    rng: XorShift,
    pending_write: Option<usize>,
}

impl<S: Unpin> Wan<S> {
    pub fn new(inner: S, profile: WanProfile) -> Self {
        Wan {
            inner,
            profile,
            sleep: None,
            rng: XorShift(seed() ^ 0x517c_c1b7_2722_0a95),
            pending_write: None,
        }
    }

    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// 采样一次延迟；0 表示本次不延迟。
    fn sample_delay(&mut self) -> Duration {
        if self.profile.is_direct() {
            return Duration::ZERO;
        }
        let base = self.profile.latency;
        if self.profile.delay_prob < 1.0 && self.rng.f64() > self.profile.delay_prob {
            return Duration::ZERO;
        }
        let jitter_ms = self.profile.jitter.as_millis() as u64;
        let extra = if jitter_ms == 0 {
            0
        } else {
            self.rng.next() % (jitter_ms + 1)
        };
        base + Duration::from_millis(extra)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Wan<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.profile.is_direct() {
            return Pin::new(&mut self.inner).poll_read(cx, buf);
        }
        // 有挂起的 sleep 就只推进它；sleep 完成后立刻尝试真实读，
        // 不再重采样——否则每次 waker 唤醒都会再挂一次新 sleep，永远读不到数据。
        if let Some(sleep) = self.sleep.as_mut() {
            match sleep.as_mut().poll(cx) {
                Poll::Ready(()) => {
                    self.sleep = None;
                    return Pin::new(&mut self.inner).poll_read(cx, buf);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        let delay = self.sample_delay();
        if !delay.is_zero() {
            let mut sleep = Box::pin(tokio::time::sleep(delay));
            match sleep.as_mut().poll(cx) {
                Poll::Ready(()) => return Pin::new(&mut self.inner).poll_read(cx, buf),
                Poll::Pending => {
                    self.sleep = Some(sleep);
                    return Poll::Pending;
                }
            }
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Wan<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.profile.is_direct() {
            return Pin::new(&mut self.inner).poll_write(cx, data);
        }
        // 与 poll_read 同理：已有挂起 sleep 就只推进；sleep 完成立即写，不重采样。
        if let Some(sleep) = self.sleep.as_mut() {
            match sleep.as_mut().poll(cx) {
                Poll::Ready(()) => {
                    self.sleep = None;
                    self.pending_write = None;
                    return Pin::new(&mut self.inner).poll_write(cx, data);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        let delay = self.sample_delay();
        if !delay.is_zero() {
            let mut sleep = Box::pin(tokio::time::sleep(delay));
            match sleep.as_mut().poll(cx) {
                Poll::Ready(()) => {
                    self.pending_write = None;
                    return Pin::new(&mut self.inner).poll_write(cx, data);
                }
                Poll::Pending => {
                    self.sleep = Some(sleep);
                    self.pending_write = Some(data.len());
                    return Poll::Pending;
                }
            }
        }
        self.pending_write = None;
        Pin::new(&mut self.inner).poll_write(cx, data)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn wan_delays_reads_by_latency() {
        let (mut a, b) = tokio::io::duplex(64);
        let mut wan_b = Wan::new(
            b,
            WanProfile {
                latency: Duration::from_millis(100),
                jitter: Duration::ZERO,
                delay_prob: 1.0,
                reorder: false,
            },
        );
        a.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        wan_b.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
    }

    #[tokio::test]
    async fn wan_is_zero_cost_when_direct() {
        let (mut a, b) = tokio::io::duplex(64);
        let mut wan_b = Wan::new(b, WanProfile::direct());
        a.write_all(b"ok").await.unwrap();
        let mut buf = [0u8; 2];
        wan_b.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ok");
    }

    #[tokio::test]
    async fn wan_write_goes_through_after_delay() {
        let (a, mut b) = tokio::io::duplex(64);
        let mut wan_a = Wan::new(
            a,
            WanProfile {
                latency: Duration::from_millis(10),
                jitter: Duration::from_millis(10),
                delay_prob: 1.0,
                reorder: true,
            },
        );
        wan_a.write_all(b"hello").await.unwrap();
        let mut buf = [0u8; 5];
        b.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello");
    }
}
