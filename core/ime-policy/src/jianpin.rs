//! 简拼索引查询：引擎无候选且输入为纯辅音穿（简拼）时，
//! 从词库预生成的 jianpin_index.txt 查表注入词候选。
//! 索引由 scripts/gen-jianpin-index.py 生成（词条的简拼编码，
//! 模拟 rime-ice abbrev 规则：zh/ch/sh 双字母、其余首字母）。

use std::collections::HashMap;
use std::path::Path;

/// 简拼索引：编码 -> 候选词列表（权重降序，文件已排序）。
pub struct JianpinIndex {
    map: HashMap<String, Vec<String>>,
    /// 编码最长度（过长的纯辅音串不查表，避免无意义扫描）
    max_code_len: usize,
}

impl JianpinIndex {
    pub fn new() -> Self {
        JianpinIndex {
            map: HashMap::new(),
            max_code_len: 8,
        }
    }
}

impl Default for JianpinIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl JianpinIndex {
    /// 加载索引文件：每行 `编码\t词\t权重`，同编码按权重降序。
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("读简拼索引失败 {}: {e}", path.display()))?;
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        let mut max_code_len = 0usize;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split('\t');
            let (Some(code), Some(word), _) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            if code.len() > max_code_len {
                max_code_len = code.len();
            }
            let entry = map.entry(code.to_string()).or_default();
            // 文件按权重降序排列，同词去重
            if !entry.iter().any(|w| w == word) {
                entry.push(word.to_string());
            }
        }
        Ok(JianpinIndex { map, max_code_len })
    }

    /// 纯辅音穿判定（无元音 a/e/i/o/u/v，与 AI 跳过逻辑同源）
    pub fn is_pure_consonant(code: &str) -> bool {
        !code.is_empty()
            && code.len() <= 8
            && code
                .bytes()
                .all(|b| b.is_ascii_lowercase() && !b"aeiouv".contains(&b))
    }

    /// 查简拼编码，返回前 max 个词。
    pub fn lookup(&self, code: &str, max: usize) -> Vec<String> {
        if code.len() > self.max_code_len {
            return Vec::new();
        }
        self.map
            .get(code)
            .map(|v| v.iter().take(max).cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_consonant_detection() {
        assert!(JianpinIndex::is_pure_consonant("lwyg"));
        assert!(JianpinIndex::is_pure_consonant("wsh"));
        assert!(JianpinIndex::is_pure_consonant("bm"));
        assert!(!JianpinIndex::is_pure_consonant("nihao"));
        assert!(!JianpinIndex::is_pure_consonant("wo"));
        assert!(!JianpinIndex::is_pure_consonant("lv"));
        assert!(!JianpinIndex::is_pure_consonant(""));
        assert!(!JianpinIndex::is_pure_consonant("abcdefghi"));
        assert!(!JianpinIndex::is_pure_consonant("Wm"));
    }

    #[test]
    fn load_and_lookup() {
        let dir = std::env::temp_dir();
        let p = dir.join("jianpin_index_test.txt");
        std::fs::write(
            &p,
            "lwyg\t来玩鱼羹\t99\nlwyg\t另一个\t158\nwsh\t晚上\t500\n",
        )
        .unwrap();
        let idx = JianpinIndex::load(&p).unwrap();
        let r = idx.lookup("lwyg", 9);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], "来玩鱼羹");
        assert_eq!(r[1], "另一个");
        assert_eq!(idx.lookup("wsh", 9), vec!["晚上"]);
        assert!(idx.lookup("zzzz", 9).is_empty());
        let _ = std::fs::remove_file(&p);
    }
}
