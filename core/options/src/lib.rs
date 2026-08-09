//! 输入法用户选项与打字统计的单一事实源。
//!
//! 选项文件：`%APPDATA%\shurufa\options.json`（Windows）。Android 没有
//! APPDATA，由调用方在引擎初始化前设置环境变量 `SHURUFA_DATA_DIR`
//! 覆盖数据目录（见 [`app_dir`]）。
//!
//! 读取语义：文件缺失或损坏一律返回默认值，不做写回（读取路径不得
//! 产生副作用）；保存采用"先写 .tmp 再 rename"的原子替换，避免崩溃
//! 留下半截 JSON。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 输入法用户选项；serde 字段全部带默认值，老版本 JSON 缺字段仍可解析。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImeOptions {
    /// Shift 切换中英文（默认 true；现行行为）
    #[serde(default = "t")]
    pub shift_switch_cn_en: bool,
    /// Shift+Space 切换全/半角（默认 true；关闭后空格不被接管）
    #[serde(default = "t")]
    pub shift_space_full_shape: bool,
    /// Ctrl+. 切换中/英标点 ascii_punct（默认 true）
    #[serde(default = "t")]
    pub ctrl_period_ascii_punct: bool,
    /// CapsLock 切到英文直输（默认 true；按 Sogou/微习惯）
    #[serde(default = "t")]
    pub capslock_to_english: bool,
}

fn t() -> bool {
    true
}

impl Default for ImeOptions {
    fn default() -> Self {
        Self {
            shift_switch_cn_en: true,
            shift_space_full_shape: true,
            ctrl_period_ascii_punct: true,
            capslock_to_english: true,
        }
    }
}

/// 数据目录：优先 `SHURUFA_DATA_DIR`（Android 端覆盖用），否则
/// `%APPDATA%\shurufa`；两者都没有时以当前目录兜底。
pub fn app_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SHURUFA_DATA_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("shurufa")
}

/// 选项文件路径：`app_dir()/options.json`。
pub fn path() -> PathBuf {
    path_in(&app_dir())
}

fn path_in(dir: &std::path::Path) -> PathBuf {
    dir.join("options.json")
}

/// 读取选项；文件缺失或损坏时返回 [`ImeOptions::default`]（不写回）。
pub fn load() -> ImeOptions {
    load_from(&path())
}

fn load_from(path: &std::path::Path) -> ImeOptions {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// 保存选项：先写同目录 `.tmp` 再 rename 原子替换，避免半截文件。
pub fn save(options: &ImeOptions) -> std::io::Result<()> {
    save_to(&path(), options)
}

fn save_to(path: &std::path::Path, options: &ImeOptions) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // serde_json 只在写器出错时才失败，此处化简为 io 错误
    let bytes = serde_json::to_vec_pretty(options)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 打字统计：`app_dir()/stats.json`。
pub mod stats {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    use serde::{Deserialize, Serialize};

    /// 一天的打字计数。
    #[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq)]
    pub struct DayCount {
        #[serde(default)]
        pub chars: u64,
        #[serde(default)]
        pub keys: u64,
    }

    /// 统计文件结构（serde 全部带默认值，容许缺字段/半截演进）。
    #[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
    struct StatsFile {
        #[serde(default)]
        total_chars: u64,
        #[serde(default)]
        total_keys: u64,
        #[serde(default)]
        days: BTreeMap<String, DayCount>,
    }

    /// 对外返回的合计视图。
    #[derive(Debug, Default, Clone, Copy, PartialEq)]
    pub struct StatsTotals {
        pub total_chars: u64,
        pub today_chars: u64,
        pub total_keys: u64,
        pub today_keys: u64,
    }

    /// 日期按 UTC 日切换：从 unix 秒手工算 YYYY-MM-DD。Windows 上拿本地
    /// 时区偏移代价高且易错；打字统计按 UTC 日切在业务上可接受（注释说明）。
    fn today_utc() -> String {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let days = (secs / 86_400) as i64;
        // Howard Hinnant 的 civil-from-days 算法
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64; // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]
        let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
        let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
        let y = if m <= 2 { y + 1 } else { y };
        format!("{y:04}-{m:02}-{d:02}")
    }

    fn load_from(path: &Path) -> StatsFile {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// 原子写：先写 .tmp 再 rename。失败静默由调用方决定。
    fn save_to(path: &Path, stats: &StatsFile) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(stats)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 进程内缓存：避免每个按键都读盘。计划外进程退出至多丢 31 次计数，
    /// 对打字统计可接受。
    struct Cache {
        stats: StatsFile,
        /// 距上次落盘的累计次数；达到 FLUSH_EVERY 即写盘
        dirty: u32,
    }

    /// 每累计 32 次计数落盘一次（读改写 + 原子 rename，失败静默）。
    const FLUSH_EVERY: u32 = 32;

    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

    fn cache() -> &'static Mutex<Cache> {
        CACHE.get_or_init(|| {
            Mutex::new(Cache {
                stats: load_from(&crate::stats_dir_path()),
                dirty: 0,
            })
        })
    }

    fn bump(chars_delta: u64, keys_delta: u64) {
        let mut guard = cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let today = today_utc();
        guard.stats.total_chars = guard.stats.total_chars.saturating_add(chars_delta);
        guard.stats.total_keys = guard.stats.total_keys.saturating_add(keys_delta);
        let day = guard.stats.days.entry(today).or_default();
        day.chars = day.chars.saturating_add(chars_delta);
        day.keys = day.keys.saturating_add(keys_delta);
        guard.dirty += 1;
        if guard.dirty >= FLUSH_EVERY {
            // 失败静默：下次计数还会再试
            if save_to(&crate::stats_dir_path(), &guard.stats).is_ok() {
                guard.dirty = 0;
            }
        }
    }

    /// 记录一次上屏的字符数。
    pub fn note_chars(n: usize) {
        if n == 0 {
            return;
        }
        bump(n as u64, 0);
    }

    /// 记录一次按键计数。
    pub fn note_keys(n: usize) {
        if n == 0 {
            return;
        }
        bump(0, n as u64);
    }

    /// 读取合计：总数/今日数的字符与按键。读不到一律全 0。
    pub fn totals() -> StatsTotals {
        let stats = match cache().lock() {
            Ok(guard) => guard.stats.clone(),
            Err(poisoned) => poisoned.into_inner().stats.clone(),
        };
        totals_of(&stats)
    }

    fn totals_of(stats: &StatsFile) -> StatsTotals {
        let today = stats.days.get(&today_utc()).copied().unwrap_or_default();
        StatsTotals {
            total_chars: stats.total_chars,
            today_chars: today.chars,
            total_keys: stats.total_keys,
            today_keys: today.keys,
        }
    }

    /// 测试用：直接对指定目录读写，绕过进程内缓存。
    #[cfg(test)]
    fn note_at(dir: &Path, chars: u64, keys: u64) {
        let path = dir.join("stats.json");
        let mut stats = load_from(&path);
        let today = today_utc();
        stats.total_chars += chars;
        stats.total_keys += keys;
        let day = stats.days.entry(today).or_default();
        day.chars += chars;
        day.keys += keys;
        let _ = save_to(&path, &stats);
    }

    #[cfg(test)]
    fn totals_at(dir: &Path) -> StatsTotals {
        totals_of(&load_from(&dir.join("stats.json")))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::PathBuf;

        fn temp_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "shurufa-options-test-{}-{}",
                name,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        #[test]
        fn 记录后合计正确() {
            let dir = temp_dir("stats");
            note_at(&dir, 3, 5);
            note_at(&dir, 2, 1);
            let t = totals_at(&dir);
            assert_eq!(t.total_chars, 5);
            assert_eq!(t.today_chars, 5);
            assert_eq!(t.total_keys, 6);
            assert_eq!(t.today_keys, 6);
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn 文件缺失或损坏时合计全零() {
            let dir = temp_dir("stats-missing");
            assert_eq!(totals_at(&dir), StatsTotals::default());
            std::fs::write(dir.join("stats.json"), b"{broken json").unwrap();
            assert_eq!(totals_at(&dir), StatsTotals::default());
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn 日期按合法格式生成() {
            let day = today_utc();
            assert_eq!(day.len(), 10);
            assert_eq!(&day[4..5], "-");
            assert_eq!(&day[7..8], "-");
        }
    }
}

/// stats 文件路径（供 stats 模块内部使用）。
fn stats_dir_path() -> PathBuf {
    app_dir().join("stats.json")
}

// 说明：stats 模块的 path_in 与顶层的 path_in 各自返回完整文件路径；
// crate 顶层 stats_dir_path 走 app_dir()，测试私有函数则直接接受目录参数。

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "shurufa-options-test-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 选项默认全部为开() {
        let options = ImeOptions::default();
        assert!(options.shift_switch_cn_en);
        assert!(options.shift_space_full_shape);
        assert!(options.ctrl_period_ascii_punct);
        assert!(options.capslock_to_english);
    }

    #[test]
    fn 老版本缺字段仍可解析且取默认值() {
        // 模拟老版本 JSON：只有一个字段，其余缺失
        let parsed: ImeOptions = serde_json::from_str(r#"{"shift_switch_cn_en":false}"#).unwrap();
        assert!(!parsed.shift_switch_cn_en);
        assert!(parsed.shift_space_full_shape);
        assert!(parsed.ctrl_period_ascii_punct);
        assert!(parsed.capslock_to_english);
        // 空对象也合法
        let parsed: ImeOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, ImeOptions::default());
    }

    #[test]
    fn 文件缺失或损坏回退默认且不写回() {
        let dir = temp_dir("options-missing");
        let path = path_in(&dir);
        assert_eq!(load_from(&path), ImeOptions::default());
        // 读取不得产生副作用
        assert!(!path.exists());
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(load_from(&path), ImeOptions::default());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 保存后读回一致() {
        let dir = temp_dir("options-save");
        let path = path_in(&dir);
        let options = ImeOptions {
            shift_space_full_shape: false,
            ..ImeOptions::default()
        };
        save_to(&path, &options).unwrap();
        assert_eq!(load_from(&path), options);
        // 临时文件不得残留
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
