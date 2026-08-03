//! 自托管中继：按目标设备指纹配对两条 TCP 流，随后透明转发。
//!
//! 中继只读取连接开始的固定路由头；其后的 rustls 握手和全部同步消息
//! 仍在两台设备之间端到端加密，中继既没有证书私钥，也不解析剪贴板内容。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

const MAGIC: &[u8; 4] = b"SRLY";
const HEADER_LEN: usize = 4 + 1 + 64 + 64;
const STATUS_READY: u8 = 1;
const STATUS_UNAVAILABLE: u8 = 0;

/// 中继客户端连接角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayMode {
    Register = 1,
    Connect = 2,
}

/// 明文路由头；指纹仅用于连接配对，TLS 会在随后重新确认对端身份。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayHeader {
    pub mode: RelayMode,
    pub source_fingerprint: String,
    pub target_fingerprint: String,
}

impl RelayHeader {
    pub fn register(fingerprint: &str) -> Result<Self, String> {
        validate_fingerprint(fingerprint)?;
        Ok(RelayHeader {
            mode: RelayMode::Register,
            source_fingerprint: fingerprint.to_string(),
            target_fingerprint: String::new(),
        })
    }

    pub fn connect(source: &str, target: &str) -> Result<Self, String> {
        validate_fingerprint(source)?;
        validate_fingerprint(target)?;
        Ok(RelayHeader {
            mode: RelayMode::Connect,
            source_fingerprint: source.to_string(),
            target_fingerprint: target.to_string(),
        })
    }
}

/// 启动中继 TCP 监听。此函数持续运行，适合独立的 shurufa-relay 进程。
pub async fn run_relay(listen_addr: &str) -> Result<(), String> {
    let listener = TcpListener::bind(listen_addr)
        .await
        .map_err(|e| format!("绑定中继地址 {listen_addr} 失败: {e}"))?;
    serve_listener(listener).await
}

/// 在已绑定监听器上运行中继；供本 crate 的集成测试以零端口竞态启动服务。
pub(crate) async fn serve_listener(listener: TcpListener) -> Result<(), String> {
    let waiters = Arc::new(Mutex::new(HashMap::new()));
    loop {
        let (stream, addr) = listener
            .accept()
            .await
            .map_err(|e| format!("接受中继连接失败: {e}"))?;
        let waiters = waiters.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, waiters).await {
                eprintln!("中继连接 {addr} 已结束：{e}");
            }
        });
    }
}

/// 通过中继主动连接已配对设备，成功后返回可直接交给 rustls 的字节流。
pub async fn connect_via_relay(
    relay_addr: &str,
    source_fingerprint: &str,
    target_fingerprint: &str,
) -> Result<TcpStream, String> {
    let mut stream = TcpStream::connect(relay_addr)
        .await
        .map_err(|e| format!("连接中继 {relay_addr} 失败: {e}"))?;
    write_header(
        &mut stream,
        &RelayHeader::connect(source_fingerprint, target_fingerprint)?,
    )
    .await?;
    expect_ready(&mut stream).await?;
    Ok(stream)
}

/// 在中继上注册本机并等待某个已配对设备接入，成功后返回 TLS 入站字节流。
pub async fn accept_via_relay(relay_addr: &str, fingerprint: &str) -> Result<TcpStream, String> {
    let mut stream = TcpStream::connect(relay_addr)
        .await
        .map_err(|e| format!("连接中继 {relay_addr} 失败: {e}"))?;
    write_header(&mut stream, &RelayHeader::register(fingerprint)?).await?;
    expect_ready(&mut stream).await?;
    Ok(stream)
}

async fn handle_connection(
    mut stream: TcpStream,
    waiters: Arc<Mutex<HashMap<String, TcpStream>>>,
) -> Result<(), String> {
    let header = read_header(&mut stream).await?;
    match header.mode {
        RelayMode::Register => {
            let previous = waiters
                .lock()
                .await
                .insert(header.source_fingerprint, stream);
            drop(previous);
            Ok(())
        }
        RelayMode::Connect => {
            let Some(mut target) = waiters.lock().await.remove(&header.target_fingerprint) else {
                stream
                    .write_all(&[STATUS_UNAVAILABLE])
                    .await
                    .map_err(|e| format!("写中继不可达状态失败: {e}"))?;
                return Ok(());
            };
            stream
                .write_all(&[STATUS_READY])
                .await
                .map_err(|e| format!("写中继连接状态失败: {e}"))?;
            target
                .write_all(&[STATUS_READY])
                .await
                .map_err(|e| format!("写中继注册状态失败: {e}"))?;
            tokio::io::copy_bidirectional(&mut stream, &mut target)
                .await
                .map_err(|e| format!("中继转发失败: {e}"))?;
            Ok(())
        }
    }
}

async fn write_header<W: AsyncWrite + Unpin>(
    writer: &mut W,
    header: &RelayHeader,
) -> Result<(), String> {
    writer
        .write_all(MAGIC)
        .await
        .map_err(|e| format!("写中继魔数失败: {e}"))?;
    writer
        .write_u8(header.mode as u8)
        .await
        .map_err(|e| format!("写中继模式失败: {e}"))?;
    write_fingerprint(writer, &header.source_fingerprint).await?;
    write_fingerprint(writer, &header.target_fingerprint).await?;
    writer
        .flush()
        .await
        .map_err(|e| format!("刷新中继头失败: {e}"))
}

async fn read_header<R: AsyncRead + Unpin>(reader: &mut R) -> Result<RelayHeader, String> {
    let mut header = [0u8; HEADER_LEN];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|e| format!("读中继头失败: {e}"))?;
    if &header[..4] != MAGIC {
        return Err("中继魔数不匹配".into());
    }
    let mode = match header[4] {
        1 => RelayMode::Register,
        2 => RelayMode::Connect,
        _ => return Err("中继模式非法".into()),
    };
    let source = read_fingerprint(&header[5..69])?;
    let target = read_fingerprint(&header[69..133])?;
    validate_fingerprint(&source)?;
    if mode == RelayMode::Connect {
        validate_fingerprint(&target)?;
    } else if !target.is_empty() {
        return Err("注册中继头不应包含目标指纹".into());
    }
    Ok(RelayHeader {
        mode,
        source_fingerprint: source,
        target_fingerprint: target,
    })
}

async fn write_fingerprint<W: AsyncWrite + Unpin>(
    writer: &mut W,
    fingerprint: &str,
) -> Result<(), String> {
    if fingerprint.is_empty() {
        return writer
            .write_all(&[0u8; 64])
            .await
            .map_err(|e| format!("写空中继指纹失败: {e}"));
    }
    validate_fingerprint(fingerprint)?;
    writer
        .write_all(fingerprint.as_bytes())
        .await
        .map_err(|e| format!("写中继指纹失败: {e}"))
}

fn read_fingerprint(bytes: &[u8]) -> Result<String, String> {
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(String::new());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "中继指纹不是 UTF-8")?;
    Ok(text.to_string())
}

fn validate_fingerprint(fingerprint: &str) -> Result<(), String> {
    if fingerprint.len() != 64 || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("中继指纹必须是 64 位十六进制字符串".into());
    }
    Ok(())
}

async fn expect_ready<R: AsyncRead + Unpin>(reader: &mut R) -> Result<(), String> {
    match reader
        .read_u8()
        .await
        .map_err(|e| format!("读取中继状态失败: {e}"))?
    {
        STATUS_READY => Ok(()),
        STATUS_UNAVAILABLE => Err("目标设备未连接到中继".into()),
        _ => Err("中继状态非法".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const A: &str = "aa00000000000000000000000000000000000000000000000000000000000000";
    const B: &str = "bb00000000000000000000000000000000000000000000000000000000000000";

    #[tokio::test]
    async fn relay_header_roundtrip() {
        let (mut writer, mut reader) = tokio::io::duplex(256);
        let expected = RelayHeader::connect(A, B).unwrap();
        let write = tokio::spawn(async move { write_header(&mut writer, &expected).await });
        let actual = read_header(&mut reader).await.unwrap();
        write.await.unwrap().unwrap();
        assert_eq!(actual, RelayHeader::connect(A, B).unwrap());
    }

    #[tokio::test]
    async fn registered_target_receives_transparent_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let waiters = Arc::new(Mutex::new(HashMap::new()));
        let server_waiters = waiters.clone();
        tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let waiters = server_waiters.clone();
                tokio::spawn(async move {
                    handle_connection(stream, waiters).await.unwrap();
                });
            }
        });

        let register =
            tokio::spawn(async move { accept_via_relay(&addr.to_string(), B).await.unwrap() });
        tokio::task::yield_now().await;
        let mut initiator = connect_via_relay(&addr.to_string(), A, B).await.unwrap();
        let mut target = register.await.unwrap();

        initiator.write_all(b"hello").await.unwrap();
        let mut received = [0u8; 5];
        target.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, b"hello");

        target.write_all(b"world").await.unwrap();
        initiator.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, b"world");
    }
}
