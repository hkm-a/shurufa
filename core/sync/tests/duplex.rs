//! 双实例集成测试：同进程内两个同步服务完成配对并互传剪贴板文本。
//!
//! 不依赖 mDNS（直连 127.0.0.1），确认回调自动通过——覆盖
//! 身份创建、TLS 双向认证、指纹钉扎、配对协议与收发循环全链路。

use std::sync::Arc;
use std::time::Duration;

use sync_core::{Incoming, SyncConfig, SyncService};
use tokio::sync::mpsc;
use tokio::time::timeout;

fn test_config(dir: &std::path::Path, name: &str) -> SyncConfig {
    let mut config = SyncConfig::new(dir.to_path_buf(), name.to_string());
    config.port = 0;
    config.enable_mdns = false;
    config.reconnect_secs = 1;
    config
}

async fn recv_clip(rx: &mut mpsc::Receiver<Incoming>) -> Incoming {
    timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("等待入站消息超时")
        .expect("入站通道关闭")
}

#[tokio::test(flavor = "multi_thread")]
async fn 配对后双向同步文本() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let (tx_a, mut rx_a) = mpsc::channel(16);
    let (tx_b, mut rx_b) = mpsc::channel(16);

    let auto_confirm: sync_core::ConfirmFn = Arc::new(|_| true);

    let a = SyncService::start(
        test_config(dir_a.path(), "甲机"),
        tx_a,
        Some(auto_confirm.clone()),
        Box::new(|m| println!("[甲] {m}")),
    )
    .await
    .unwrap();
    let b = SyncService::start(
        test_config(dir_b.path(), "乙机"),
        tx_b,
        Some(auto_confirm.clone()),
        Box::new(|m| println!("[乙] {m}")),
    )
    .await
    .unwrap();

    assert_ne!(a.fingerprint(), b.fingerprint());

    // 甲 → 乙 发起配对（两端自动确认）
    let peer = a
        .pair_with(
            &format!("127.0.0.1:{}", b.local_port()),
            auto_confirm.clone(),
        )
        .await
        .expect("配对失败");
    assert_eq!(peer.name, "乙机");
    assert!(a.peers().iter().any(|p| p.fingerprint == b.fingerprint()));
    assert!(b.peers().iter().any(|p| p.fingerprint == a.fingerprint()));

    // 等重连循环建立数据连接
    for _ in 0..50 {
        if !a.connected_fingerprints().is_empty() && !b.connected_fingerprints().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(!a.connected_fingerprints().is_empty(), "数据连接未建立");

    // 甲 → 乙
    a.send_clip("你好，来自甲机");
    let got = recv_clip(&mut rx_b).await;
    match got {
        Incoming::Clip {
            from_name,
            text,
            sent_at_ms,
            msg_id,
        } => {
            assert_eq!(from_name, "甲机");
            assert_eq!(text, "你好，来自甲机");
            assert!(sent_at_ms.is_some(), "v2 协议必须携带 sent_at_ms");
            assert!(msg_id.is_some(), "v2 协议必须携带 msg_id");
        }
        other => panic!("期待 Clip，实际 {other:?}"),
    }

    // 甲 → 乙 传文件（名称、MIME 与字节均保持不变）
    let file = b"sync file bytes";
    a.send_file("note.txt", "text/plain", file);
    let got = recv_clip(&mut rx_b).await;
    assert_eq!(
        got,
        Incoming::File {
            from_name: "甲机".into(),
            name: "note.txt".into(),
            mime_type: "text/plain".into(),
            data: file.to_vec(),
        }
    );

    // 甲 → 乙 传图片（PNG 字节原样往返）
    let png: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 250, 0, 128,
    ];
    a.send_image(&png);
    let got = recv_clip(&mut rx_b).await;
    assert_eq!(
        got,
        Incoming::Image {
            from_name: "甲机".into(),
            png: png.clone()
        }
    );

    // 乙 → 甲
    b.send_clip("收到，乙机回礼");
    let got = recv_clip(&mut rx_a).await;
    match got {
        Incoming::Clip {
            from_name,
            text,
            sent_at_ms,
            msg_id,
        } => {
            assert_eq!(from_name, "乙机");
            assert_eq!(text, "收到，乙机回礼");
            assert!(sent_at_ms.is_some());
            assert!(msg_id.is_some());
        }
        other => panic!("期待 Clip，实际 {other:?}"),
    }

    // 未配对的第三方不可见：新服务无法凭空收到广播
    let dir_c = tempfile::tempdir().unwrap();
    let (tx_c, mut rx_c) = mpsc::channel(16);
    let _c = SyncService::start(
        test_config(dir_c.path(), "丙机"),
        tx_c,
        None,
        Box::new(|m| println!("[丙] {m}")),
    )
    .await
    .unwrap();
    a.send_clip("丙机不应看到");
    let _ = recv_clip(&mut rx_b).await; // 乙正常收到
    assert!(
        timeout(Duration::from_millis(800), rx_c.recv())
            .await
            .is_err(),
        "未配对设备不应收到同步"
    );
}

/// 双机搜索：乙机注入搜索回调，甲机发请求后应收到乙机的命中结果。
#[tokio::test(flavor = "multi_thread")]
async fn 跨设备搜索请求得到响应() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let (tx_a, mut rx_a) = mpsc::channel(16);
    let (tx_b, mut _rx_b) = mpsc::channel(16);

    let auto_confirm: sync_core::ConfirmFn = Arc::new(|_| true);

    let a = SyncService::start(
        test_config(dir_a.path(), "甲机"),
        tx_a,
        Some(auto_confirm.clone()),
        Box::new(|m| println!("[甲] {m}")),
    )
    .await
    .unwrap();
    let b = SyncService::start(
        test_config(dir_b.path(), "乙机"),
        tx_b,
        Some(auto_confirm.clone()),
        Box::new(|m| println!("[乙] {m}")),
    )
    .await
    .unwrap();

    // 乙机提供两条假历史命中
    b.set_search_handler(Arc::new(|query: &str| {
        assert!(!query.is_empty());
        vec![
            sync_core::SearchHit {
                text: "主题：周报邮件".into(),
                source_app: "Mail".into(),
                updated_at: 1720000000000,
            },
            sync_core::SearchHit {
                text: "转发：邮件模板".into(),
                source_app: "Notes".into(),
                updated_at: 1720000001000,
            },
        ]
    }));

    a.pair_with(
        &format!("127.0.0.1:{}", b.local_port()),
        auto_confirm.clone(),
    )
    .await
    .expect("配对失败");

    for _ in 0..50 {
        if !a.connected_fingerprints().is_empty() && !b.connected_fingerprints().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(!a.connected_fingerprints().is_empty(), "数据连接未建立");

    a.send_search_request("邮件", "req-1".into());
    let got = recv_clip(&mut rx_a).await;
    match got {
        Incoming::SearchResults {
            from_name,
            req_id,
            hits,
        } => {
            assert_eq!(from_name, "乙机");
            assert_eq!(req_id.as_deref(), Some("req-1"));
            assert_eq!(hits.len(), 2);
            assert_eq!(hits[0].text, "主题：周报邮件");
        }
        other => panic!("期待 SearchResults，实际 {other:?}"),
    }
}
