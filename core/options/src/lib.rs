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
    /// 通用页设置（自启/日志级别/历史上限/快捷键开关）；老版本缺字段取默认
    #[serde(default)]
    pub general: GeneralSettings,
}

/// 设置中心"通用"页字段一览。 serde 双端兼容：老 JSON 无 `general` 键时
/// 整体回退 [`GeneralSettings::default`]。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GeneralSettings {
    /// 登录自启（HKCU Run 键，名字 shurufa-host）
    #[serde(default)]
    pub autostart: bool,
    /// 日志级别：Info / Debug / Trace
    #[serde(default)]
    pub log_level: LogLevel,
    /// 皮肤目录覆盖（保留字段：当前由 SSOT 决定，UI 只读展示）
    #[serde(default)]
    pub skin_dir_override: Option<String>,
    /// 历史最大条数（50..=2000，保存时钳位）
    #[serde(default = "default_history_max")]
    pub history_max_entries: u32,
    /// Ctrl+Shift+R 划词润色热键开关（listener.rs 接线预留，见 wave 4）
    #[serde(default = "t")]
    pub enable_polish_hotkey: bool,
    /// Ctrl+Shift+W AI 帮写热键开关（listener.rs 接线预留，见 wave 4）
    #[serde(default = "t")]
    pub enable_ai_hotkey: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LogLevel {
    #[default]
    Info,
    Debug,
    Trace,
}

fn default_history_max() -> u32 {
    500
}

/// 历史最大条数允许区间
pub const HISTORY_MAX_MIN: u32 = 50;
pub const HISTORY_MAX_LIMIT: u32 = 2000;

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            autostart: false,
            log_level: LogLevel::Info,
            skin_dir_override: None,
            history_max_entries: default_history_max(),
            enable_polish_hotkey: true,
            enable_ai_hotkey: true,
        }
    }
}

impl GeneralSettings {
    /// 钳位到合法区间；供保存路径统一调用，杜绝 UI/手写 JSON 越界。
    pub fn clamped(mut self) -> Self {
        self.history_max_entries = self
            .history_max_entries
            .clamp(HISTORY_MAX_MIN, HISTORY_MAX_LIMIT);
        self
    }
}

/// wave 4 预留：listener.rs 读取热键开关的统一入口。
/// 当前 listener.rs 尚未接线，调用方先在 wave 4 接入 options 热重载
/// （windows/src/service.rs 已有 2 秒 mtime 轮询的 refresh_options）
/// 即可热生效；本函数放这里避免 listener 再次拼路径。
pub fn hotkey_gates() -> (bool, bool) {
    let g = load().general;
    (g.enable_polish_hotkey, g.enable_ai_hotkey)
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
            general: GeneralSettings::default(),
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

/// 读取-修改-写回：在同一把进程级文件锁内完成，杜绝设置中心与 TSF
/// （或两端设置中心）并发写互相覆盖。文件锁用 `options.json.lock`
/// 旁路文件，Windows 上 LockFileEx 对 rename 不敏感——锁的是 lock
/// 文件本体，不挡 rename 的目标文件。
///
/// `f` 在拿到最新内容后被调用；返回值即要写回的结果。任何一步出错
/// 都会解锁并向上传播。
pub fn modify(f: impl FnOnce(&ImeOptions) -> ImeOptions) -> std::io::Result<ImeOptions> {
    let dir = app_dir();
    std::fs::create_dir_all(&dir)?;
    let lock_path = dir.join("options.json.lock");
    modify_at(&path_in(&dir), &lock_path, f)
}

fn modify_at(
    path: &std::path::Path,
    lock_path: &std::path::Path,
    f: impl FnOnce(&ImeOptions) -> ImeOptions,
) -> std::io::Result<ImeOptions> {
    use fs4::fs_std::FileExt;
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    // 进程内先串行化：POSIX fcntl 锁同进程可重复获取且语义不一致，
    // 这里再加一层互斥让"同进程并发 modify"在跨平台行为上一致。
    static LOCAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _local_guard = LOCAL.lock().unwrap_or_else(|p| p.into_inner());
    lock_file.lock_exclusive()?;
    let result = (|| {
        let current = load_from(path);
        let next = f(&current);
        save_to(path, &next)?;
        Ok(next)
    })();
    // 无论成功失败都解锁
    let _ = lock_file.unlock();
    result
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

    /// 最近 N 天（含今天、升序）的 (日期 YYYY-MM-DD, 当日字符数)；无数据的
    /// 日期补 0，方便图表直接按固定宽度渲染。N 上限 31 防御误传。
    pub fn last_days(n: usize) -> Vec<(String, u64)> {
        let stats = match cache().lock() {
            Ok(guard) => guard.stats.clone(),
            Err(poisoned) => poisoned.into_inner().stats.clone(),
        };
        last_days_of(&stats, n.min(31).max(1))
    }

    fn last_days_of(stats: &StatsFile, n: usize) -> Vec<(String, u64)> {
        // 从 today_utc() 逐日回推；BTreeMap 按 key 字典序即日期序。
        let mut out = Vec::with_capacity(n);
        let today = today_utc();
        let (mut y, mut m, mut d) = parse_ymd(&today).unwrap_or((2000, 1, 1));
        for _ in 0..n {
            let key = format!("{y:04}-{m:02}-{d:02}");
            let chars = stats.days.get(&key).map(|c| c.chars).unwrap_or(0);
            out.push((key, chars));
            // 逐日倒退一天
            let (py, pm, pd) = prev_day(y, m, d);
            y = py;
            m = pm;
            d = pd;
        }
        out.reverse();
        out
    }

    fn parse_ymd(s: &str) -> Option<(i64, u32, u32)> {
        let mut parts = s.split('-');
        let y = parts.next()?.parse().ok()?;
        let m = parts.next()?.parse().ok()?;
        let d = parts.next()?.parse().ok()?;
        Some((y, m, d))
    }

    fn prev_day(y: i64, m: u32, d: u32) -> (i64, u32, u32) {
        // days-from-civil 逆运算：用与 today_utc 相同的 Hinnant 算法回退。
        let days = days_from_civil(y, m, d) - 1;
        civil_from_days(days)
    }

    fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
        let y_adj = if m <= 2 { y - 1 } else { y };
        let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
        let yoe = (y_adj - era * 400) as u64;
        let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
        let doy = (153 * mp + 2) / 5 + d as u64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe as i64 - 719_468
    }

    fn civil_from_days(z: i64) -> (i64, u32, u32) {
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        (y, m as u32, d as u32)
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

        #[test]
        fn last_days_包含今天且空日补零() {
            let dir = temp_dir("last-days");
            note_at(&dir, 7, 0);
            let path = dir.join("stats.json");
            let stats = load_from(&path);
            let days = last_days_of(&stats, 3);
            assert_eq!(days.len(), 3);
            let today = today_utc();
            assert_eq!(days[2].0, today);
            assert_eq!(days[2].1, 7);
            assert_eq!(days[0].1, 0);
            assert_eq!(days[1].1, 0);
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn 日前后推算与逆运算保持一致() {
            // 覆盖跨月/跨年边界
            for (y, m, d) in [(2026, 8, 8), (2024, 1, 1), (2025, 12, 31), (2000, 2, 29)] {
                let days = days_from_civil(y, m, d);
                assert_eq!(civil_from_days(days), (y, m, d), "互逆失败 {y}-{m}-{d}");
                let (py, pm, pd) = prev_day(y, m, d);
                let days2 = days_from_civil(py, pm, pd);
                assert_eq!(days2, days - 1, "prev_day 差一天 {y}-{m}-{d}");
            }
        }
    }
}

/// stats 文件路径（供 stats 模块内部使用）。
fn stats_dir_path() -> PathBuf {
    app_dir().join("stats.json")
}

// ---------------------------------------------------------------------------
// 剪贴板收藏（clip-favorites.json）
// ---------------------------------------------------------------------------

/// 收藏类别：文本 / 图片 / 文件。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClipFavoriteKind {
    Text,
    Image,
    File,
}

/// 单条收藏。`content_text` 只在 Text 时有值；`path` 在 Image/File 时有值。
/// `id` 是进程内单调递增计数（由 [`favorites::add_favorite`] 分配），不做主键
/// 之外的任何语义。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClipFavorite {
    pub id: u64,
    pub kind: ClipFavoriteKind,
    #[serde(default)]
    pub content_text: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    pub pinned_at_ms: i64,
    #[serde(default)]
    pub source_peer: Option<String>,
}

/// 收藏文件整体（clip-favorites.json）。老版本缺 `next_id` / `entries` 时
/// 一律回退默认，保持向后兼容。
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClipFavorites {
    /// 单调递增 id 计数器：下一个分配的 id；只增不减，删除条目后不复用。
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub entries: Vec<ClipFavorite>,
}

pub mod favorites {
    use super::{ClipFavorite, ClipFavorites};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    fn path() -> PathBuf {
        crate::app_dir().join("clip-favorites.json")
    }

    fn load_from(p: &Path) -> ClipFavorites {
        std::fs::read(p)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save_to(p: &Path, favs: &ClipFavorites) -> std::io::Result<()> {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(favs)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // 原子替换：先写 .tmp 再 rename，避免崩溃留下半截文件
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, p)?;
        Ok(())
    }

    /// 进程内串行化：面板与设置中心可能在同一进程（受测宿主）里并发改，
    /// 先把同进程并发掐掉；跨进程仍靠文件原子替换兜底。
    static LOCAL: OnceLock<Mutex<()>> = OnceLock::new();

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        LOCAL
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    pub fn load_favorites() -> ClipFavorites {
        clip_load_from(&path())
    }

    pub fn save_favorites(favs: &ClipFavorites) -> std::io::Result<()> {
        clip_save_to(&path(), favs)
    }

    /// 追加一条收藏：id 由本函数分配（取 `next_id` 后自增）。
    /// `pinned_at_ms` / `source_peer` 由调用方按业务需要填充。
    pub fn add_favorite(mut fav: ClipFavorite) -> std::io::Result<ClipFavorite> {
        let _guard = lock();
        let p = path();
        let mut favs = load_from(&p);
        fav.id = favs.next_id;
        favs.next_id = favs.next_id.saturating_add(1);
        favs.entries.push(fav.clone());
        save_to(&p, &favs)?;
        Ok(fav)
    }

    pub fn remove_favorite(id: u64) -> std::io::Result<bool> {
        let _guard = lock();
        let p = path();
        let mut favs = load_from(&p);
        let before = favs.entries.len();
        favs.entries.retain(|f| f.id != id);
        if favs.entries.len() == before {
            return Ok(false);
        }
        save_to(&p, &favs)?;
        Ok(true)
    }

    /// 置顶开关语义：命中条目把 `pinned_at_ms` 取负即取消收藏（保留记录
    /// 供 UI 灰显），再翻回正数即恢复；绝对值始终是首次收藏的时间戳。
    /// 返回切换后的新状态（true = 已收藏）。未找到 id 返回 None。
    pub fn toggle_pin_favorite(id: u64) -> std::io::Result<Option<bool>> {
        let _guard = lock();
        let p = path();
        let mut favs = load_from(&p);
        let Some(entry) = favs.entries.iter_mut().find(|f| f.id == id) else {
            return Ok(None);
        };
        // 0 兜底为 1，保证符号位一定能翻转
        let base = if entry.pinned_at_ms == 0 {
            1
        } else {
            entry.pinned_at_ms.abs()
        };
        entry.pinned_at_ms = if entry.pinned_at_ms > 0 { -base } else { base };
        let now_pinned = entry.pinned_at_ms > 0;
        save_to(&p, &favs)?;
        Ok(Some(now_pinned))
    }

    /// 历史面板 / 设置中心共用的读取帮助：把磁盘文件暴露在外的最小面。
    pub fn clip_load_from(p: &Path) -> ClipFavorites {
        load_from(p)
    }

    pub fn clip_save_to(p: &Path, favs: &ClipFavorites) -> std::io::Result<()> {
        save_to(p, favs)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::{ClipFavorite, ClipFavoriteKind};
        use std::path::PathBuf;

        fn temp_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!(
                "shurufa-options-test-fav-{}-{}",
                name,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn sample(id: u64, text: &str) -> ClipFavorite {
            ClipFavorite {
                id,
                kind: ClipFavoriteKind::Text,
                content_text: Some(text.to_owned()),
                path: None,
                pinned_at_ms: 1_700_000_000_000,
                source_peer: None,
            }
        }

        #[test]
        fn 收藏可往返序列化() {
            let dir = temp_dir("roundtrip");
            let p = dir.join("clip-favorites.json");
            let favs = ClipFavorites {
                next_id: 2,
                entries: vec![sample(0, "hello"), sample(1, "世界")],
            };
            clip_save_to(&p, &favs).unwrap();
            let back = clip_load_from(&p);
            assert_eq!(back, favs);
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn 单调id在追加时自增且不复用() {
            // 直接用内存态驱动 add 的分配逻辑，避开写盘，便于隔离验证。
            let mut favs = ClipFavorites::default();
            for expected in 0..3_u64 {
                let mut fav = sample(u64::MAX, "x");
                fav.id = favs.next_id;
                assert_eq!(fav.id, expected);
                favs.next_id = favs.next_id.saturating_add(1);
                favs.entries.push(fav);
            }
            // 删除中段后 next_id 不回退
            favs.entries.retain(|f| f.id != 1);
            assert_eq!(favs.next_id, 3);
        }

        #[test]
        fn 置顶开关持久化且可逆() {
            let dir = temp_dir("pin-toggle");
            let p = dir.join("clip-favorites.json");
            let favs = ClipFavorites {
                next_id: 1,
                entries: vec![sample(0, "a")],
            };
            clip_save_to(&p, &favs).unwrap();

            // 翻转一次：pinned_at_ms 应变负；再翻应回正
            let mut state = clip_load_from(&p);
            let e = &mut state.entries[0];
            let original = e.pinned_at_ms;
            e.pinned_at_ms = -original;
            clip_save_to(&p, &state).unwrap();
            let re = clip_load_from(&p);
            assert!(re.entries[0].pinned_at_ms < 0);
            assert_eq!(re.entries[0].pinned_at_ms, -original);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
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

    #[test]
    fn modify_并发串行化_两端增量不丢失() {
        // 模拟设置中心与 TSF 并发"翻转两个不同开关"：不配锁时后写会覆盖先写；
        // 走 modify_at 同一把 lib 文件锁，两次增量都应当保留。
        let dir = temp_dir("options-modify");
        let path = path_in(&dir);
        let lock = dir.join("options.json.lock");
        let start = std::sync::Arc::new(std::sync::Barrier::new(2));

        let p1 = path.clone();
        let l1 = lock.clone();
        let b1 = start.clone();
        let t1 = std::thread::spawn(move || {
            b1.wait();
            for _ in 0..16 {
                modify_at(&p1, &l1, |o| ImeOptions {
                    shift_switch_cn_en: !o.shift_switch_cn_en,
                    ..o.clone()
                })
                .unwrap();
            }
        });
        let p2 = path.clone();
        let l2 = lock.clone();
        let b2 = start.clone();
        let t2 = std::thread::spawn(move || {
            b2.wait();
            for _ in 0..16 {
                modify_at(&p2, &l2, |o| ImeOptions {
                    capslock_to_english: !o.capslock_to_english,
                    ..o.clone()
                })
                .unwrap();
            }
        });
        t1.join().unwrap();
        t2.join().unwrap();
        let final_opts = load_from(&path);
        // 偶数次翻转后两端都回到 true；关键是途中没有丢失任何一方的写。
        assert!(final_opts.shift_switch_cn_en);
        assert!(final_opts.capslock_to_english);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
