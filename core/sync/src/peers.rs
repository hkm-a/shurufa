//! 已配对设备的持久化存储（JSON 文件，原子替换写入）。
//!
//! 即读即写：配对可能由独立的 `pair` 子命令进程完成，常驻守护
//! 进程必须在下一轮重连扫描时看到新条目，因此不做内存缓存。
//! 文件很小（每设备一行级别），读写成本可忽略。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Peer {
    pub name: String,
    /// 对端证书 SHA-256 指纹（64 位十六进制），身份唯一标识
    pub fingerprint: String,
    /// 最近一次成功连接的地址（ip:port），用于 mDNS 不可用时直连
    pub last_addr: Option<String>,
    /// 最近一次成功连接/更新地址的毫秒时间戳（M8-2 设备状态用；
    /// 老 peers.json 无此字段时回退 None = 尚未连通过）。
    #[serde(default)]
    pub last_seen_ms: Option<i64>,
}

pub struct PeerStore {
    path: PathBuf,
    /// 只串行化本进程内的读改写序列；跨进程由原子替换保证完整性
    write_lock: Mutex<()>,
}

impl PeerStore {
    pub fn open(dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {e}"))?;
        Ok(PeerStore {
            path: dir.join("peers.json"),
            write_lock: Mutex::new(()),
        })
    }

    fn read(&self) -> Vec<Peer> {
        let Ok(text) = fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn list(&self) -> Vec<Peer> {
        self.read()
    }

    pub fn contains(&self, fingerprint: &str) -> bool {
        self.read().iter().any(|p| p.fingerprint == fingerprint)
    }

    /// 新增或更新（按指纹合并），随后落盘。
    pub fn upsert(&self, peer: Peer) -> Result<(), String> {
        let _guard = self.write_lock.lock().expect("配对表锁不可恢复");
        let mut peers = self.read();
        match peers.iter_mut().find(|p| p.fingerprint == peer.fingerprint) {
            Some(existing) => *existing = peer,
            None => peers.push(peer),
        }
        self.persist(&peers)
    }

    pub fn update_addr(&self, fingerprint: &str, addr: &str) -> Result<(), String> {
        let _guard = self.write_lock.lock().expect("配对表锁不可恢复");
        let mut peers = self.read();
        if let Some(p) = peers.iter_mut().find(|p| p.fingerprint == fingerprint) {
            let changed = p.last_addr.as_deref() != Some(addr);
            p.last_addr = Some(addr.to_string());
            // 连接成功即刷新最近在线时间（M8-2：设备状态可视化）
            p.last_seen_ms = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
            );
            if changed {
                return self.persist(&peers);
            }
        }
        Ok(())
    }

    pub fn remove(&self, fingerprint_prefix: &str) -> Result<bool, String> {
        let _guard = self.write_lock.lock().expect("配对表锁不可恢复");
        let mut peers = self.read();
        let before = peers.len();
        peers.retain(|p| !p.fingerprint.starts_with(fingerprint_prefix));
        let removed = peers.len() != before;
        if removed {
            self.persist(&peers)?;
        }
        Ok(removed)
    }

    fn persist(&self, peers: &[Peer]) -> Result<(), String> {
        let text = serde_json::to_string_pretty(peers).expect("序列化配对表不应失败");
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|e| format!("写入配对表失败: {e}"))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("替换配对表失败: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 配对表增删改查与持久化() {
        let dir = tempfile::tempdir().unwrap();
        let store = PeerStore::open(dir.path()).unwrap();
        assert!(store.list().is_empty());

        let peer = Peer {
            name: "手机".into(),
            fingerprint: "ab".repeat(32),
            last_addr: None,
            last_seen_ms: None,
        };
        store.upsert(peer.clone()).unwrap();
        assert!(store.contains(&peer.fingerprint));

        store
            .update_addr(&peer.fingerprint, "192.168.1.5:48632")
            .unwrap();

        // 另一个实例（模拟独立进程）应立即看到写入
        let other = PeerStore::open(dir.path()).unwrap();
        assert_eq!(
            other.list()[0].last_addr.as_deref(),
            Some("192.168.1.5:48632")
        );

        assert!(other.remove(&peer.fingerprint[..12]).unwrap());
        assert!(!store.contains(&peer.fingerprint));
    }
}
