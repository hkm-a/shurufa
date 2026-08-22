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
    /// 语音转写设置（Ctrl+Shift+S）；老版本缺 `speech` 键时整体回退默认
    #[serde(default)]
    pub speech: SpeechSettings,
    /// 输入方案 id：pinyin（默认）/ double_pinyin / wubi / cangjie。
    /// wave 4 仅完成持久化与 UI；引擎热重部署（librime schema redeploy）
    /// 留给 wave 5。老 JSON 无此字段时 serde 回退 pinyin，参见
    /// `default_input_scheme` 与 [`validate_input_scheme`]。
    #[serde(default = "default_input_scheme")]
    pub input_scheme: String,
    /// 候选窗位置策略（P1 #10）：follow（默认，跟随光标）/ bottom_right /
    /// bottom_left（固定屏幕角落）。TSF 每键热读，改动约 2 秒内生效。
    #[serde(default = "default_candidate_position")]
    pub candidate_position: String,
    /// 候选面板模式（M7，搜狗 16.3b 候选条/多行候选同类）：single（默认，
    /// 单行候选条）/ multi（多行候选面板，↓ 键唤出）。TSF 每键热读，改动
    /// 约 2 秒内生效。多行布局随 M7 逐步落地，选项层先行。
    #[serde(default = "default_candidate_panel_mode")]
    pub candidate_panel_mode: String,
    /// 按应用选项（weasel app_options 同款，2026-08-17 引入）：进程名
    /// （小写，如 "windowsterminal.exe"）→ 该应用下的输入法行为覆盖。
    /// 支持 ascii_mode（进入该应用自动切英文直输，离开恢复）与 vim_mode
    /// （该应用按 vim 回 normal 模式键时自动切英文，2026-08-18 引入）；
    /// 空表 = 全部应用走全局行为。TSF 在前台应用变化时应用覆盖。
    #[serde(default)]
    pub app_options: std::collections::BTreeMap<String, AppOption>,
    /// 符号配对（微信输入法同类，2026-08-18 引入）：中文态、无组合时按
    /// `(` `[` `{` `《` 自动补配对符并把光标居中（`()` `[]` `{}` `《》`）。
    /// 默认关（避免与 IDE 自动补全/括号高亮冲突；微信默认同样关闭）。
    #[serde(default = "default_symbol_pairing")]
    pub symbol_pairing: bool,
    /// M10-1 专业词场景（搜狗 16.2 场景词库同类）：none（默认）/ doctor /
    /// lawyer / code。设置中心保存后生成 rime_ice.custom.yaml 挂载场景词库
    /// 并重建词典；改动需重新部署生效。
    #[serde(default = "default_scenario_dict")]
    pub scenario_dict: String,
    /// AI 候选预测（搜狗 AI 化主线，2026-08-20）：拼音暂停约 800ms 后基于
    /// 当前拼音调 agnès 预测候选，注入候选行尾部（🤖 标注）。默认关
    /// （云端消耗 + 隐私）；需设置环境变量 AGNES_API_KEY（与 AI 帮写面板
    /// 同源，永不落盘）。TSF 每键热读，改动约 2 秒内生效。
    #[serde(default = "default_ai_candidates")]
    pub ai_candidates: bool,
}

/// 符号配对默认值：关闭（微信输入法默认同样关闭，避免与应用冲突）。
pub fn default_symbol_pairing() -> bool {
    false
}

/// AI 候选预测默认值：关闭（云端消耗 + 隐私，与 Android 端一致）。
pub fn default_ai_candidates() -> bool {
    false
}

/// 专业词场景默认值：无（M10-1）。
pub fn default_scenario_dict() -> String {
    "none".to_owned()
}

/// 校验专业词场景 id（M10-1 / v1.2 生僻字）。
pub fn validate_scenario_dict(s: &str) -> bool {
    matches!(s, "none" | "doctor" | "lawyer" | "code" | "rare")
}

/// 单个应用的输入法行为覆盖（weasel app_options 同款）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AppOption {
    /// 进入该应用自动切英文直输（true）；离开该应用恢复进入前的状态。
    /// None = 不覆盖（跟随全局）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ascii_mode: Option<bool>,
    /// vim 模式（weasel app_options vim_mode 同款，2026-08-18 引入）：
    /// 该应用下无组合时按 vim 的"回 normal 模式键"（Esc / Ctrl+C /
    /// Ctrl+[）自动切英文直输，让 vim / emacs / 终端拿到这些键；
    /// 有组合时由引擎先处理（Esc 取消组合），不抢不切。None = 不覆盖。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vim_mode: Option<bool>,
}

/// 候选窗位置策略默认值：跟随光标（Fcitx5/微软拼音默认行为一致）。
pub fn default_candidate_position() -> String {
    "follow".to_owned()
}

/// 候选面板模式默认值：单行候选条（现行布局；多行面板待 M7 布局落地）。
pub fn default_candidate_panel_mode() -> String {
    "single".to_owned()
}

/// 候选面板模式合法值："single"（单行候选条）| "multi"（多行候选面板）。
pub fn validate_candidate_panel_mode(s: &str) -> bool {
    matches!(s, "single" | "multi")
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
    /// Ctrl+Shift+T 划词翻译热键开关（2026-08-18 引入，微信/搜狗划词翻译同类）。
    /// 选中文本后按热键 → AI 翻译成中文/英文，回车覆盖选区。
    #[serde(default = "t")]
    pub enable_translate_hotkey: bool,
    /// 悬浮球/控制中心窗口不透明度（%，30..=100；搜狗 16.1 状态栏不透明度同类）。
    /// 由设置中心读取并 `setOpacity` 应用；保存路径统一钳位。
    #[serde(default = "default_ball_opacity")]
    pub ball_opacity: u8,
    /// M9-6：划词工具应用白名单（exe 文件名，如 WINWORD.EXE；空 = 所有应用）。
    #[serde(default)]
    pub selection_app_whitelist: Vec<String>,
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

fn default_ball_opacity() -> u8 {
    100
}

/// 悬浮球不透明度允许区间（下限 30% 保证始终可见可点）
pub const BALL_OPACITY_MIN: u8 = 30;
pub const BALL_OPACITY_MAX: u8 = 100;

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
            enable_translate_hotkey: true,
            ball_opacity: default_ball_opacity(),
            selection_app_whitelist: Vec::new(),
        }
    }
}

impl GeneralSettings {
    /// 钳位到合法区间；供保存路径统一调用，杜绝 UI/手写 JSON 越界。
    pub fn clamped(mut self) -> Self {
        self.history_max_entries = self
            .history_max_entries
            .clamp(HISTORY_MAX_MIN, HISTORY_MAX_LIMIT);
        self.ball_opacity = self.ball_opacity.clamp(BALL_OPACITY_MIN, BALL_OPACITY_MAX);
        self
    }
}

/// AI 帮写 / 划词润色 / 划词翻译热键开关的统一入口（enable_polish_hotkey,
/// enable_ai_hotkey, enable_translate_hotkey）。windows-host 的 listener.rs
/// 在启动时读取一次，之后每 2 秒轮询（SetTimer + refresh_hotkey_gates）——
/// 设置中心开关即改即存，约 2 秒内在宿主侧热生效，无需重启进程。
pub fn hotkey_gates() -> (bool, bool, bool) {
    let g = load().general;
    (
        g.enable_polish_hotkey,
        g.enable_ai_hotkey,
        g.enable_translate_hotkey,
    )
}

fn t() -> bool {
    true
}

/// `input_scheme` 的 serde 默认值；"pinyin" 与历史版本 JSON 兼容。
fn default_input_scheme() -> String {
    "pinyin".to_owned()
}

/// 纯函数校验：合法方案 id 集合（与 schemas/shurufa_*.schema.yaml 一一对应）。
/// 供 settings UI / TSF watcher / JNI 三端共享同一事实源；wave 5 引入
/// 新方案时只改这一个函数。
pub fn validate_input_scheme(s: &str) -> bool {
    matches!(
        s,
        "pinyin" | "double_pinyin" | "wubi" | "cangjie" | "t9" | "stroke" | "radical"
    )
}

/// options 方案 id → librime schema_id 的映射（与 schemas/ 文件名一致）；
/// 未知 id 回退雾凇拼音。Windows TSF 与 Android JNI 共用。
pub fn schema_id_of(scheme: &str) -> &'static str {
    match scheme {
        "double_pinyin" => "shurufa_double_pinyin",
        "wubi" => "shurufa_wubi",
        "cangjie" => "shurufa_cangjie",
        "t9" => "shurufa_t9",
        "stroke" => "stroke",
        "radical" => "radical_pinyin",
        _ => "rime_ice",
    }
}

/// 语音转写设置（Ctrl+Shift+S → 剪贴板 + 书面语化）。serde 双端兼容：
/// 老 JSON 无 `speech` 键时整体回退 [`SpeechSettings::default`]。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeechSettings {
    /// 功能总开关：false 时 listener 不注册 Ctrl+Shift+S
    #[serde(default)]
    pub enabled: bool,
    /// Ctrl+Shift+S 热键开关（与 enabled 同时 true 才注册）
    #[serde(default = "t")]
    pub hotkey_enabled: bool,
    /// 无新语音片段超过该秒数时自动收尾提交（本轮 stub 仅记录，由真实引擎消费）
    #[serde(default = "default_auto_commit_threshold_secs")]
    pub auto_commit_threshold_secs: u32,
    /// 收尾后走书面语化润色（agnes-2.5-flash）；默认关闭，失败回退原文
    #[serde(default)]
    pub written_style_polish: bool,
    /// 单次会话最长时长，超时自动停止并提交当前结果
    #[serde(default = "default_max_session_secs")]
    pub max_session_secs: u32,
    /// v1.2 语音后端：stub（演示节奏）/ cloud（真实录音 → OpenAI 兼容
    /// /v1/audio/transcriptions 云端转写）
    #[serde(default = "default_speech_backend")]
    pub backend: String,
    /// 云端转写 Base URL（如 https://api.openai.com/v1）；留空时读环境变量
    /// SHURUFA_ASR_BASE_URL，再回退到默认值
    #[serde(default = "default_cloud_base_url")]
    pub cloud_base_url: String,
    /// 云端转写模型（默认 whisper-1）
    #[serde(default = "default_cloud_model")]
    pub cloud_model: String,
}

fn default_auto_commit_threshold_secs() -> u32 {
    5
}

fn default_max_session_secs() -> u32 {
    120
}

pub fn default_speech_backend() -> String {
    "stub".to_owned()
}

pub fn default_cloud_base_url() -> String {
    "https://api.openai.com/v1".to_owned()
}

pub fn default_cloud_model() -> String {
    "whisper-1".to_owned()
}

/// 校验语音后端 id（v1.2）。
pub fn validate_speech_backend(s: &str) -> bool {
    matches!(s, "stub" | "cloud")
}

impl Default for SpeechSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            hotkey_enabled: true,
            auto_commit_threshold_secs: default_auto_commit_threshold_secs(),
            written_style_polish: false,
            max_session_secs: default_max_session_secs(),
            backend: default_speech_backend(),
            cloud_base_url: default_cloud_base_url(),
            cloud_model: default_cloud_model(),
        }
    }
}

impl Default for ImeOptions {
    fn default() -> Self {
        Self {
            shift_switch_cn_en: true,
            shift_space_full_shape: true,
            ctrl_period_ascii_punct: true,
            capslock_to_english: true,
            general: GeneralSettings::default(),
            speech: SpeechSettings::default(),
            input_scheme: default_input_scheme(),
            candidate_position: default_candidate_position(),
            candidate_panel_mode: default_candidate_panel_mode(),
            app_options: std::collections::BTreeMap::new(),
            symbol_pairing: default_symbol_pairing(),
            scenario_dict: default_scenario_dict(),
            ai_candidates: default_ai_candidates(),
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

/// Rime 用户数据目录（与 host 的 user_rime_dir / algo 的 user_config_root 一致）：
/// app_dir()/rime；引擎部署与 lua 生成都写这里。
pub fn rime_dir() -> PathBuf {
    app_dir().join("rime")
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

    /// 日期按 UTC 日切换：打字统计按 UTC 日切在业务上可接受（注释说明）。
    fn today_utc() -> String {
        chrono::Utc::now()
            .date_naive()
            .format("%Y-%m-%d")
            .to_string()
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
        last_days_of(&stats, n.clamp(1, 31))
    }

    fn last_days_of(stats: &StatsFile, n: usize) -> Vec<(String, u64)> {
        // 从 today_utc() 逐日回推；BTreeMap 按 key 字典序即日期序。
        let mut out = Vec::with_capacity(n);
        let mut day = chrono::NaiveDate::parse_from_str(&today_utc(), "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
        for _ in 0..n {
            let key = day.format("%Y-%m-%d").to_string();
            let chars = stats.days.get(&key).map(|c| c.chars).unwrap_or(0);
            out.push((key, chars));
            day = day.pred_opt().unwrap_or(day);
        }
        out.reverse();
        out
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
            // 覆盖跨月/跨年边界；日期算术交给 chrono，这里只验证 API 语义。
            for (y, m, d) in [(2026, 8, 8), (2024, 1, 1), (2025, 12, 31), (2000, 2, 29)] {
                let date = chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
                let prev = date.pred_opt().expect("日期应有前一天");
                assert_eq!(
                    prev.succ_opt(),
                    Some(date),
                    "prev 的次日应回到原日期 {y}-{m}-{d}"
                );
                let diff = date.signed_duration_since(prev).num_days();
                assert_eq!(diff, 1, "prev 应差一天 {y}-{m}-{d}");
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

// ---------------------------------------------------------------------------
// 跨设备同步活动流（M8-1：同步状态可视化 / 来源标签）
// ---------------------------------------------------------------------------

/// 同步活动类型。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncActivityKind {
    Text,
    Image,
    File,
}

/// 同步活动方向：收到（in）/ 发出（out）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncDirection {
    In,
    Out,
}

/// 单条跨设备同步活动：来源标签（peer）、状态、时间、预览。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncActivityEntry {
    pub id: u64,
    pub direction: SyncDirection,
    pub kind: SyncActivityKind,
    /// 预览：文本截断 ~60 字符 / 图片"图片 N 字节" / 文件名
    pub preview: String,
    /// 来源/目标设备名（收到 = 对端 from_name；发出 = 对端应答来源）
    #[serde(default)]
    pub peer: Option<String>,
    /// "ok" | "failed"
    pub status: String,
    #[serde(default)]
    pub detail: Option<String>,
    /// 失败重试句柄：host 为可重试失败生成的载荷 id（设置中心据此显示「重试」按钮）。
    #[serde(default)]
    pub retry_id: Option<String>,
    pub ts_ms: i64,
}

/// 同步活动流文件（sync-activity.json）；最多保留 sync_activity::CAP 条。
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncActivity {
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub entries: Vec<SyncActivityEntry>,
}

pub mod sync_activity {
    use super::{SyncActivity, SyncActivityEntry};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    /// 活动流保留上限：设置中心最多展示最近 50 条。
    pub const CAP: usize = 50;

    fn path() -> PathBuf {
        crate::app_dir().join("sync-activity.json")
    }

    fn load_from(p: &Path) -> SyncActivity {
        std::fs::read(p)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save_to(p: &Path, act: &SyncActivity) -> std::io::Result<()> {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(act)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // 原子替换：先写 .tmp 再 rename，避免崩溃留下半截文件
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, p)?;
        Ok(())
    }

    /// 进程内串行化：host 多个事件回调与设置中心读取可能并发，先掐同进程。
    static LOCAL: OnceLock<Mutex<()>> = OnceLock::new();

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        LOCAL
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// 读取活动流（磁盘缺失/损坏回退空）。
    pub fn load() -> SyncActivity {
        load_from(&path())
    }

    /// 记录一条活动到指定文件：分配单调 id、追加并裁剪到 CAP 条、原子落盘。
    /// 返回带 id 的条目；id 由本函数分配（入参 id 字段被覆盖）。
    /// 测试经此函数注入临时路径，生产走 [`record`] 的默认路径。
    fn record_to(p: &Path, mut entry: SyncActivityEntry) -> std::io::Result<SyncActivityEntry> {
        let _guard = lock();
        let mut act = load_from(p);
        entry.id = act.next_id;
        act.next_id = act.next_id.saturating_add(1);
        act.entries.push(entry.clone());
        if act.entries.len() > CAP {
            let drop_n = act.entries.len() - CAP;
            act.entries.drain(0..drop_n);
        }
        save_to(p, &act)?;
        Ok(entry)
    }

    /// 记录一条活动（默认文件路径）：见 [`record_to`]。
    pub fn record(entry: SyncActivityEntry) -> std::io::Result<SyncActivityEntry> {
        record_to(&path(), entry)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::{SyncActivityKind, SyncDirection};

        fn temp_dir(tag: &str) -> PathBuf {
            let d = std::env::temp_dir().join(format!(
                "shurufa-sync-activity-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            d
        }

        fn sample(id: u64) -> SyncActivityEntry {
            SyncActivityEntry {
                id,
                direction: SyncDirection::In,
                kind: SyncActivityKind::Text,
                preview: "你好".into(),
                peer: Some("手机".into()),
                status: "ok".into(),
                detail: None,
                retry_id: None,
                ts_ms: 1_700_000_000_000,
            }
        }

        #[test]
        fn 活动可往返序列化() {
            let dir = temp_dir("roundtrip");
            let p = dir.join("sync-activity.json");
            let mut with_retry = sample(0);
            with_retry.retry_id = Some("r1".into());
            let act = SyncActivity {
                next_id: 2,
                entries: vec![with_retry, sample(1)],
            };
            save_to(&p, &act).unwrap();
            assert_eq!(load_from(&p), act);
            // 旧文件缺 retry_id 字段仍可解析（serde default → None）
            let legacy = r#"{"entries":[{"id":0,"direction":"in","kind":"text","preview":"旧","status":"failed","ts_ms":1}]}"#;
            let parsed: crate::SyncActivity = serde_json::from_str(legacy).unwrap();
            assert_eq!(parsed.entries[0].retry_id, None);
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn 记录分配单调id且失败状态可标记() {
            let dir = temp_dir("record");
            let p = dir.join("sync-activity.json");
            let mut entry = sample(u64::MAX);
            entry.status = "failed".into();
            entry.detail = Some("写入系统剪贴板失败".into());
            let saved = record_to(&p, entry).unwrap();
            assert_eq!(saved.id, 0, "首次记录 id 应为 0");
            assert_eq!(saved.status, "failed");
            let back = load_from(&p);
            assert_eq!(back.next_id, 1);
            assert_eq!(back.entries.len(), 1);
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn 超过上限时裁剪最旧() {
            let dir = temp_dir("cap");
            let p = dir.join("sync-activity.json");
            for _ in 0..(CAP + 5) {
                record_to(&p, sample(u64::MAX)).unwrap();
            }
            let back = load_from(&p);
            assert_eq!(back.entries.len(), CAP);
            // 最旧的 5 条被裁掉：第一条 id 应为 5
            assert_eq!(back.entries[0].id, 5);
            assert_eq!(back.next_id, (CAP + 5) as u64);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// 应用/网站直达（M8-4，搜狗 15.2 灵犀候选直达同类）
// ---------------------------------------------------------------------------

/// 直达目标类型：应用（可执行文件）或网址（浏览器打开）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppShortcutKind {
    App,
    Url,
}

/// 一条直达快捷：输入 code 命中候选 → 提交后启动 target（不落文本）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppShortcut {
    pub id: u64,
    /// 触发码（小写字母/数字，≤32）
    pub code: String,
    /// 候选显示名（≤30）
    pub label: String,
    pub kind: AppShortcutKind,
    /// 应用可执行文件绝对路径 或 URL（http/https）
    pub target: String,
}

/// 直达清单（app-shortcuts.json）；设置中心整表读写。
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppShortcuts {
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub entries: Vec<AppShortcut>,
}

pub mod app_shortcuts {
    use super::{AppShortcutKind, AppShortcuts};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    /// 直达清单上限（防止误填刷爆；正常使用远达不到）。
    pub const CAP: usize = 100;

    pub fn path() -> PathBuf {
        crate::app_dir().join("app-shortcuts.json")
    }

    fn load_from(p: &Path) -> AppShortcuts {
        std::fs::read(p)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save_to(p: &Path, shortcuts: &AppShortcuts) -> std::io::Result<()> {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(shortcuts)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, p)?;
        Ok(())
    }

    static LOCAL: OnceLock<Mutex<()>> = OnceLock::new();

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        LOCAL
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    pub fn load() -> AppShortcuts {
        load_from(&path())
    }

    /// 规范化清单（纯函数，可单测）：去空 code、同 code 去重（保留首个）、
    /// 分配单调 id、code 转小写、裁剪到 CAP。
    fn normalize(shortcuts: &mut AppShortcuts) {
        let mut seen = std::collections::HashSet::new();
        shortcuts.entries.retain(|s| {
            let code = s.code.trim().to_ascii_lowercase();
            !code.is_empty() && seen.insert(code)
        });
        for entry in shortcuts.entries.iter_mut() {
            entry.id = shortcuts.next_id;
            shortcuts.next_id = shortcuts.next_id.saturating_add(1);
            entry.code = entry.code.trim().to_ascii_lowercase();
        }
        if shortcuts.entries.len() > CAP {
            shortcuts.entries.truncate(CAP);
        }
    }

    /// 整表保存（设置中心编辑后整体写回）：规范化后原子落盘。
    pub fn save(mut shortcuts: AppShortcuts) -> std::io::Result<AppShortcuts> {
        let _guard = lock();
        normalize(&mut shortcuts);
        save_to(&path(), &shortcuts)?;
        Ok(shortcuts)
    }

    /// 生成供引擎 lua 读取的快捷表（写往 user_rime_dir()/lua/app_direct_shortcuts.lua）。
    pub fn generate_lua(shortcuts: &AppShortcuts) -> String {
        let mut out = String::from(
            "-- 应用/网站直达（M8-4）：由设置中心从 app-shortcuts.json 生成，勿手改。
",
        );
        out.push_str(
            "return {
",
        );
        for s in &shortcuts.entries {
            let kind = match s.kind {
                AppShortcutKind::App => "app",
                AppShortcutKind::Url => "url",
            };
            out.push_str(&format!(
                "  {{ code = {:?}, label = {:?}, kind = {:?}, target = {:?} }},
",
                s.code, s.label, kind, s.target
            ));
        }
        out.push_str(
            "}
",
        );
        out
    }

    /// 把快捷表写为 lua 模块（engine 的 user_data_dir/lua 下，require 直接命中）。
    pub fn write_lua(shortcuts: &AppShortcuts) -> std::io::Result<()> {
        let dir = crate::rime_dir().join("lua");
        std::fs::create_dir_all(&dir)?;
        let text = generate_lua(shortcuts);
        std::fs::write(dir.join("app_direct_shortcuts.lua"), text)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        use crate::AppShortcut;

        fn temp_dir(tag: &str) -> PathBuf {
            let d = std::env::temp_dir().join(format!(
                "shurufa-app-shortcuts-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            d
        }

        fn sample(code: &str, kind: AppShortcutKind) -> AppShortcut {
            AppShortcut {
                id: 0,
                code: code.to_owned(),
                label: format!("目标-{code}"),
                kind,
                target: if kind == AppShortcutKind::Url {
                    "https://example.com".into()
                } else {
                    "C:\\apps\\x.exe".into()
                },
            }
        }

        #[test]
        fn 清单往返序列化与去重() {
            let dir = temp_dir("roundtrip");
            let p = dir.join("app-shortcuts.json");
            let mut list = AppShortcuts::default();
            list.entries.push(sample("weixin", AppShortcutKind::App));
            list.entries.push(sample("weixin", AppShortcutKind::App)); // 重复 code
            list.entries.push(sample("baidu", AppShortcutKind::Url));
            save_to(&p, &list).unwrap();
            let back = load_from(&p);
            assert_eq!(back, list);
            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn lua生成包含转义与类型() {
            let mut list = AppShortcuts::default();
            list.entries.push(AppShortcut {
                id: 1,
                code: "weixin".into(),
                label: "微信".into(),
                kind: AppShortcutKind::App,
                target: "C:\\Program Files\\wechat.exe".into(),
            });
            list.entries.push(sample("baidu", AppShortcutKind::Url));
            let lua = generate_lua(&list);
            assert!(lua.contains("code = \"weixin\""));
            assert!(lua.contains("kind = \"app\""));
            assert!(lua.contains("C:\\\\Program Files\\\\wechat.exe"));
            assert!(lua.contains("kind = \"url\""));
        }

        #[test]
        fn 规范化去重分配id并裁剪上限() {
            let mut list = AppShortcuts::default();
            for i in 0..(CAP as u64 + 3) {
                list.entries
                    .push(sample(&format!("code{i}"), AppShortcutKind::App));
            }
            // 重复 code（大小写不同）只留首个；空 code 剔除
            list.entries.push(sample("CODE0", AppShortcutKind::App));
            list.entries.push(sample("", AppShortcutKind::App));
            normalize(&mut list);
            assert_eq!(list.entries.len(), CAP);
            // code 归一化小写；id 从 0 起连续
            assert_eq!(list.entries[0].code, "code0");
            assert_eq!(list.entries[0].id, 0);
            assert_eq!(list.entries[1].id, 1);
            // 裁剪不回退 next_id（已分配的 id 不复用）
            assert_eq!(list.next_id, (CAP as u64) + 3);
            assert!(list.entries.iter().all(|e| !e.code.is_empty()));
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
    fn 输入方案字段_缺省回退且写读往返() {
        // 老版本 JSON 无 input_scheme：serde default 为 "pinyin"
        let parsed: ImeOptions = serde_json::from_str(r#"{"shift_switch_cn_en":true}"#).unwrap();
        assert_eq!(parsed.input_scheme, "pinyin");
        // Default 实例也带 pinyin
        assert_eq!(ImeOptions::default().input_scheme, "pinyin");
        // 写读往返：双拼方案能经 save_to/load_from 保留
        let dir = temp_dir("options-scheme");
        let path = path_in(&dir);
        let opts = ImeOptions {
            input_scheme: "double_pinyin".to_owned(),
            ..ImeOptions::default()
        };
        save_to(&path, &opts).unwrap();
        let back = load_from(&path);
        assert_eq!(back.input_scheme, "double_pinyin");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 输入方案校验器_只接受四个已知id() {
        assert!(validate_input_scheme("pinyin"));
        assert!(validate_input_scheme("double_pinyin"));
        assert!(validate_input_scheme("wubi"));
        assert!(validate_input_scheme("t9"));
        assert!(validate_input_scheme("stroke"));
        assert_eq!(schema_id_of("t9"), "shurufa_t9");
        assert_eq!(schema_id_of("stroke"), "stroke");
        assert_eq!(schema_id_of("pinyin"), "rime_ice");
        assert_eq!(schema_id_of("unknown"), "rime_ice");
        assert!(validate_input_scheme("cangjie"));
        assert!(!validate_input_scheme("abc"));
        assert!(!validate_input_scheme(""));
        assert!(!validate_input_scheme("Pinyin")); // 大小写敏感
        assert!(!validate_input_scheme("double-pinyin")); // 连字符不是下划线
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
    fn 语音设置_老版本缺speech键走默认值() {
        // 老版本 JSON 完全没有 speech 段： serde 整体回退 SpeechSettings::default
        let parsed: ImeOptions = serde_json::from_str(r#"{"shift_switch_cn_en":false}"#).unwrap();
        assert_eq!(parsed.speech, SpeechSettings::default());
        assert!(!parsed.speech.enabled, "语音默认关闭");
        assert!(parsed.speech.hotkey_enabled);
        assert_eq!(parsed.speech.auto_commit_threshold_secs, 5);
        assert!(!parsed.speech.written_style_polish);
        assert_eq!(parsed.speech.max_session_secs, 120);
        // speech 段缺字段：按字段默认补齐
        let parsed: ImeOptions = serde_json::from_str(r#"{"speech":{"enabled":true}}"#).unwrap();
        assert!(parsed.speech.enabled);
        assert!(parsed.speech.hotkey_enabled);
        assert_eq!(parsed.speech.max_session_secs, 120);
    }

    #[test]
    fn 按应用选项_缺省空表且写读往返() {
        // 老版本 JSON 无 app_options：空表
        let parsed: ImeOptions = serde_json::from_str(r#"{"shift_switch_cn_en":true}"#).unwrap();
        assert!(parsed.app_options.is_empty());
        // 写读往返：windowsterminal 自动英文
        let dir = temp_dir("options-app");
        let path = path_in(&dir);
        let opts = ImeOptions {
            app_options: [(
                "windowsterminal.exe".to_owned(),
                AppOption {
                    ascii_mode: Some(true),
                    vim_mode: None,
                },
            )]
            .into_iter()
            .collect(),
            ..ImeOptions::default()
        };
        save_to(&path, &opts).unwrap();
        let back = load_from(&path);
        assert_eq!(
            back.app_options.get("windowsterminal.exe"),
            Some(&AppOption {
                ascii_mode: Some(true),
                vim_mode: None
            })
        );
        // 缺字段的 AppOption 默认不覆盖
        let parsed: ImeOptions = serde_json::from_str(r#"{"app_options":{"a.exe":{}}}"#).unwrap();
        assert_eq!(parsed.app_options.get("a.exe"), Some(&AppOption::default()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn 符号配对_缺省关闭且写读往返() {
        // 默认关（微信同类：避免与 IDE 自动补全冲突）
        assert!(!ImeOptions::default().symbol_pairing);
        let parsed: ImeOptions = serde_json::from_str(r#"{"shift_switch_cn_en":true}"#).unwrap();
        assert!(!parsed.symbol_pairing);
        // 开启后写读往返
        let dir = temp_dir("options-symbol");
        let path = path_in(&dir);
        let opts = ImeOptions {
            symbol_pairing: true,
            ..ImeOptions::default()
        };
        save_to(&path, &opts).unwrap();
        let back = load_from(&path);
        assert!(back.symbol_pairing);
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
