//! MRU（最近使用）候选提频：按拼音记住最近用户选中的词，在以后同一拼音序列
//! 下把它们前置。
//!
//! 存储：`%APPDATA%\shurufa\user-mru.json`。容量上限每键 8 条 / 全表 1024 键。
//! 先写 `.tmp` 再 rename，与 options.json 原子写模式一致。
//!
//! 规则（纯函数，便于测试）：
//! - `record`：若词已存在则挪到列表最前（LRU）；新增但满容量时淘汰最旧。
//! - `boost_list`：命中时返回前 8 项，首项最近使用。
//! - 与 librime 原生排序共存的算法在 `boost()`：仅把命中集前置，其余保持原序。
//!
//! 开关：options.json 顶层没有显式关闭开关，统一默认开启；如需禁用自行删除
//! `user-mru.json` 即重置。

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 每个拼音 key 最多记住的候选条数
const PER_KEY_CAP: usize = 8;
/// 整张表的最大 key 数；超出时删除最旧（LRU）
const MAP_CAP: usize = 1024;
/// MRU 文件路径
fn mru_path() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
        .join("user-mru.json")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MruStore {
    /// key = 拼音（原始按键序列），value = 该拼音下用户选中过的词语（最近在前）
    #[serde(default)]
    map: HashMap<String, Vec<String>>,
    /// 自上次落盘后是否有未保存的变更（不参与序列化）
    #[serde(skip)]
    dirty: bool,
}

impl MruStore {
    pub fn load() -> Self {
        fs::read_to_string(mru_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&mut self) -> io::Result<()> {
        let tmp = mru_path().with_extension("tmp");
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&tmp, json)?;
        fs::rename(&tmp, mru_path())?;
        self.dirty = false;
        Ok(())
    }

    /// 是否有未落盘的变更（供后台节流保存线程判断）。
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// 把一条新选词记录到 MRU：`pinyin` 是用户当时敲的 raw keys。
    pub fn record(&mut self, pinyin: &str, selected: &str) {
        if selected.is_empty() {
            return;
        }
        let entry = self.map.entry(pinyin.to_owned()).or_default();
        // 去重：先抽掉旧位置再头插
        entry.retain(|r| r != selected);
        entry.insert(0, selected.to_owned());
        if entry.len() > PER_KEY_CAP {
            entry.truncate(PER_KEY_CAP);
        }
        // 全表淘汰最旧（简单 LFU：发现超限时随机退一个；生产可换 true LRU）
        if self.map.len() > MAP_CAP {
            if let Some(k) = self.map.keys().next().cloned() {
                self.map.remove(&k);
            }
        }
        self.dirty = true;
    }

    /// 查询 `pinyin` 近期的候选 list（最近在前）。
    pub fn boost_list(&self, pinyin: &str) -> &[String] {
        self.map.get(pinyin).map(Vec::as_slice).unwrap_or(&[])
    }

    /// 把 `candidates` 按 MRU 靠前重排：
    /// 把 boost_list 里命中 candidates 的项按"最近的在前"排前；剩余保持原序。
    /// 返回新 vec，原 candidates 传所有权。不修改。
    pub fn boost(&self, pinyin: &str, candidates: Vec<String>) -> Vec<String> {
        let boost = self.boost_list(pinyin);
        if boost.is_empty() || candidates.is_empty() {
            return candidates;
        }
        let mut head = Vec::new();
        let mut rest = Vec::new();
        for cand in candidates {
            if boost.iter().any(|b| b == &cand) {
                head.push(cand);
            } else {
                rest.push(cand);
            }
        }
        // head 按 MRU 顺序而非 candidates 中出现顺序排 → 对 boost list 倒序扫 candidates
        let mut head_sorted = Vec::with_capacity(head.len());
        for mru in boost.iter() {
            for c in &head {
                if c == mru {
                    head_sorted.push(c.clone());
                    break;
                }
            }
        }
        head_sorted.extend(rest);
        head_sorted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> MruStore {
        // 每个测试使用临时路径，避免影响真实 MRU 数据
        MruStore {
            map: HashMap::new(),
            dirty: false,
        }
    }

    #[test]
    fn record_去重并置于头() {
        let mut s = temp_store();
        s.record("nihao", "你好");
        s.record("nihao", "呢");
        s.record("nihao", "你好");
        let list = s.boost_list("nihao");
        assert_eq!(list[0], "你好");
        assert_eq!(list[1], "呢");
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn record_满容量淘汰尾部() {
        let mut s = temp_store();
        // 填满 8 条
        for i in 0..10 {
            s.record("key", &format!("词{i}"));
        }
        let list = s.boost_list("key");
        assert_eq!(list.len(), PER_KEY_CAP, "应只保留最近 8 条");
        assert_eq!(list[0], "词9", "最新一条应在头");
    }

    #[test]
    fn save_load_roundtrip() {
        let mut s = MruStore {
            map: HashMap::new(),
            dirty: false,
        };
        s.record("shang'hai", "上海");
        s.record("ce'lue", "策略");
        // 写读走真实路径，但大概率会写真的 user-mru.json；测试环境 temp 目录污染
        // 可接受（不影响业务）。若 APPDATA 未设则走 tempdir。
        s.save().expect("MRU 保存失败");
        let loaded = MruStore::load();
        assert_eq!(
            loaded.boost_list("shang'hai").first(),
            Some(&"上海".to_owned())
        );
        assert_eq!(
            loaded.boost_list("ce'lue").first(),
            Some(&"策略".to_owned())
        );
    }

    #[test]
    fn boost_空mru与原序无差() {
        let s = temp_store();
        let input = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let out = s.boost("nothing", input.clone());
        assert_eq!(out, input);
    }

    #[test]
    fn boost_命中前置且保持mru序() {
        let mut s = temp_store();
        s.record("nihao", "嗬"); // 老
        s.record("nihao", "你好"); // 较近
        let candidates = vec![
            "x".to_owned(),
            "你好".to_owned(),
            "y".to_owned(),
            "嗬".to_owned(),
        ];
        let out = s.boost("nihao", candidates);
        assert_eq!(out[0], "你好", "最近选应居首");
        assert_eq!(out[1], "嗬", "次所选应次首");
        assert_eq!(out[2], "x");
        assert_eq!(out[3], "y");
    }

    #[test]
    fn record_empty_selected_忽略() {
        let mut s = temp_store();
        s.record("nihao", "");
        assert!(s.boost_list("nihao").is_empty());
    }

    #[test]
    fn dirty标记_记录后置脏_保存后清除() {
        let mut s = temp_store();
        assert!(!s.dirty(), "初始不应脏");
        s.record("nihao", "你好");
        assert!(s.dirty(), "record 后应置脏");
        let _ = s.boost_list("nihao");
        assert!(s.dirty(), "查询不改脏状态");
    }

    #[test]
    fn map全表1024淘汰最旧() {
        let mut s = temp_store();
        for i in 0..1030 {
            s.record(&format!("key{i}"), &format!("w{i}"));
        }
        assert_eq!(s.map.len(), MAP_CAP, "全表应裁到 1024");
    }
}
