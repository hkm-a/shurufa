//! 更新清单与灰度发布判定（平台无关，可单测）。
//!
//! 客户端流程：
//! 1. 拉取 `update.json`；
//! 2. 按当前 channel（stable / canary / beta）找到对应条目；
//! 3. 若目标版本比当前新，且 `machine_id` 哈希落入 `rollout_percent` 区间，
//!    则提示/执行更新。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 一个渠道的更新信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelInfo {
    pub version: String,
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    /// 0-100：灰度比例。100 = 全量。
    #[serde(default = "default_rollout")]
    pub rollout_percent: u8,
}

fn default_rollout() -> u8 {
    100
}

/// 更新清单（`update.json`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UpdateManifest {
    /// channel 名 → 更新信息。
    #[serde(default)]
    pub channels: BTreeMap<String, ChannelInfo>,
}

impl UpdateManifest {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// 简单版本比较：按 `.` 分段数值比较，支持 `-pre` 后缀（预发布低于同号正式版）。
/// 非严格 SemVer，够更新判断用。
pub fn version_gt(a: &str, b: &str) -> bool {
    let (a_main, a_pre) = split_pre(a);
    let (b_main, b_pre) = split_pre(b);
    let a_parts: Vec<u32> = a_main
        .split('.')
        .map(|s| s.parse().unwrap_or(0))
        .collect();
    let b_parts: Vec<u32> = b_main
        .split('.')
        .map(|s| s.parse().unwrap_or(0))
        .collect();
    let len = a_parts.len().max(b_parts.len());
    for i in 0..len {
        let av = a_parts.get(i).copied().unwrap_or(0);
        let bv = b_parts.get(i).copied().unwrap_or(0);
        if av != bv {
            return av > bv;
        }
    }
    // 主版本相同：无预发布 > 有预发布；都有则按字典序（简单近似）。
    match (a_pre.is_empty(), b_pre.is_empty()) {
        (true, false) => true,
        (false, true) => false,
        (false, false) => a_pre > b_pre,
        (true, true) => false,
    }
}

fn split_pre(v: &str) -> (&str, &str) {
    match v.split_once('-') {
        Some((main, pre)) => (main, pre),
        None => (v, ""),
    }
}

/// 稳定哈希：把 machine_id + channel 映射到 0..100。
fn hash_bucket(machine_id: &str, channel: &str) -> u8 {
    let mut h: u64 = 1469598103934665603;
    for b in format!("{machine_id}\u{1F916}{channel}").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    (h % 100) as u8
}

/// 灰度更新判定。
///
/// - 渠道不存在 → false
/// - 目标版本不大于当前版本 → false
/// - `rollout_percent >= 100` → true
/// - 否则按 `machine_id + channel` 稳定哈希分桶，落入 `[0, rollout_percent)` 才 true
pub fn should_update(
    manifest: &UpdateManifest,
    channel: &str,
    machine_id: &str,
    current_version: &str,
) -> bool {
    let Some(info) = manifest.channels.get(channel) else {
        return false;
    };
    if !version_gt(&info.version, current_version) {
        return false;
    }
    let rollout = info.rollout_percent.min(100);
    if rollout >= 100 {
        return true;
    }
    hash_bucket(machine_id, channel) < rollout
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> UpdateManifest {
        UpdateManifest::from_json(
            r#"{
                "channels": {
                    "stable": {"version": "1.0.0", "url": "https://example.com/stable.exe", "sha256": "abc", "rollout_percent": 100},
                    "canary": {"version": "0.9.0-canary.abc123", "url": "https://example.com/canary.exe", "rollout_percent": 50}
                }
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn parse_and_serialize() {
        let m = manifest();
        let json = m.to_json_pretty().unwrap();
        let back = UpdateManifest::from_json(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn version_compare_basic() {
        assert!(version_gt("1.0.0", "0.9.9"));
        assert!(!version_gt("1.0.0", "1.0.0"));
        assert!(version_gt("1.0.1", "1.0.0"));
        assert!(version_gt("1.0.0", "1.0.0-rc.1"));
        assert!(!version_gt("1.0.0-rc.1", "1.0.0"));
    }

    #[test]
    fn full_rollout_always_true() {
        let m = manifest();
        assert!(should_update(&m, "stable", "any-machine", "0.9.0"));
        assert!(!should_update(&m, "stable", "any-machine", "1.0.0"));
    }

    #[test]
    fn unknown_channel_false() {
        let m = manifest();
        assert!(!should_update(&m, "beta", "any", "0.0.1"));
    }

    #[test]
    fn rollout_respects_percent() {
        let m = manifest();
        // canary rollout=50：一半机器应更新，一半不应（取决于 machine_id）。
        let updated = (0..200)
            .map(|i| format!("machine-{i}"))
            .filter(|id| should_update(&m, "canary", id, "0.8.0"))
            .count();
        let ratio = updated as f64 / 200.0;
        assert!((0.30..=0.70).contains(&ratio), "ratio={ratio}");
    }

    #[test]
    fn same_machine_stable_decision() {
        let m = manifest();
        let id = "user-42";
        let a = should_update(&m, "canary", id, "0.8.0");
        let b = should_update(&m, "canary", id, "0.8.0");
        assert_eq!(a, b, "同一 machine_id 决策必须稳定");
    }
}
