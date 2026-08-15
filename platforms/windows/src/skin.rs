//! 跨端皮肤 JSON（shurufa-skin.json）的 Windows 端实现：主题感知 Skin。
//!
//! 本轮改动摘要（v2 schema + 系统主题感知 + DWM 外观助手）：
//! - 升级为 schema version 2（旧 version=1 文件安全读取，缺省字段全部回退默认）。
//! - 新增 `Skin`：候选窗颜色 + 度量（圆角/字缩放/透明度） + 阴影 + dark_mode。
//! - `load()` 读取 HKCU\...\Themes\Personalize\SystemUsesLightTheme 自动选 dark/light 变体。
//! - 新增 `refresh_on_setting_change()`：WM_SETTINGCHANGE 到达后刷新线程缓存。
//! - 新增 `apply_appearance()`（DWM 圆角 + 沉浸式深色边框 + 分层透明度）与
//!   `ShadowShell`（WS_EX_NOACTIVATE 阴影壳窗口，命中测试透明）。
//! - 旧 API `candidate_colors_from_json` / `load_candidate_colors` 保留并委托到新结构。
//!
//! 共享文件位于 schemas/shurufa-skin.json。用户可把同名文件放入
//! %APPDATA%\shurufa，或以 SHURUFA_SKIN_PATH 指定开发期文件。
//!
//! ## shurufa-skin.json schema v2 文档
//!
//! ```json
//! {
//!   "version": 2,                       // 必填；1 或 2 均可被本模块读取
//!   "light": {                          // 亮色变体
//!     "keyboard": { "background": "#RRGGBB", ... },   // Android 键盘消费；Windows 忽略
//!     "candidate": {                    // Windows 候选窗/面板颜色（#RRGGBB 或 #AARRGGBB）
//!       "background": "#FFFFFF",
//!       "highlight_background": "#D6EBE1",
//!       "text": "#111418",
//!       "preedit": "#9AA2AB",
//!       "label": "#1B9E77"
//!     },
//!     "metrics": {                      // v2 新增，整体可选
//!       "radius": 8,                    // 圆角半径基准（像素；Win11 由 DWM 取值）
//!       "font_scale": 1.0,              // 字号倍率，0.5..=2.0 以外按 1.0 处理
//!       "opacity": 0.96,                // 窗口整体透明度，(0,1]；>=1 不启用分层窗口
//!       "scrollbar": true,              // 候选窗翻页滚动条（右缘 4px，按页绘制；默认开）
//!       "icon": "xxx"                   // 候选图标槽位（预留，本版本不渲染）
//!     }
//!   },
//!   "dark":  { ... 同上结构 ... },       // 暗色变体；按系统主题自动切换
//!   "shadow": {                          // v2 新增，整体可选
//!     "enabled": true,                  // 是否在主窗下方绘制阴影壳
//!     "radius": 18,                     // 阴影外延基准（越大壳越宽）
//!     "alpha": 64                       // 阴影壳整体不透明度 0..=255
//!   }
//! }
//! ```
//!
//! 解析规则：所有 v2 新字段均为 Optional（`#[serde(default)]`），
//! 缺失字段回退到内置默认（亮色/暗色各一套颜色、metrics 8/1.0/1.0、阴影关）。
//! 颜色非法字符串只回退该字段，不影响其余字段。读取大小上限 128 KiB。
//!
//! 本文件同时被 shurufa-tsf（`mod skin`）与 shurufa-host
//! （`panel.rs` 内 `#[path] pub(crate) mod skin`）编译，因此**不得引用
//! `crate::` 下的任何 TSF 专属符号**；部署目录差异通过 `load_with` 的
//! 注入参数解决。

use std::cell::RefCell;
use std::path::PathBuf;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// 颜色与度量结构
// ---------------------------------------------------------------------------

/// GDI COLORREF 颜色（0x00BBGGRR）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandidateColors {
    pub background: u32,
    pub highlight_background: u32,
    pub text: u32,
    pub preedit: u32,
    pub label: u32,
}

impl CandidateColors {
    /// 亮色候选窗默认（与内置 light 变体一致；皮肤文件缺失时的安全网）。
    pub fn light() -> Self {
        CandidateColors {
            background: 0x00FF_FFFF,
            highlight_background: 0x00E1_EBD6,
            text: 0x0018_1411,
            preedit: 0x00AB_A29A,
            label: 0x0077_9E1B,
        }
    }

    /// 暗色候选窗默认（与内置 dark 变体一致）。
    pub fn dark() -> Self {
        CandidateColors {
            background: 0x0026_211E,
            highlight_background: 0x0038_402E,
            text: 0x00F3_F1F0,
            preedit: 0x0099_938E,
            label: 0x00A2_CD4E,
        }
    }
}

impl Default for CandidateColors {
    /// 历史默认：亮色（保留旧行为，v1 文件读取路径不受影响）。
    fn default() -> Self {
        CandidateColors::light()
    }
}

/// 皮肤度量：圆角、字号倍率、整体透明度 + 候选窗滚动条/图标槽位。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// 圆角半径基准像素（Win11 实际半径由 DWM 决定，此值供绘制与文档）
    pub radius: i32,
    /// 字号倍率；非法值（<=0 或非有限）在解析时归一为 1.0
    pub font_scale: f32,
    /// 整体窗口透明度 0..=1；>=1 不启用 WS_EX_LAYERED
    pub opacity: f32,
    /// 候选窗是否绘制翻页滚动条；缺省 true（v2 老文件无该字段照常工作）
    pub scrollbar: bool,
    /// 候选图标槽位（预留字段；本版本仅透传与一次性日志，不渲染）。
    /// 预留为 Copy 友好的固定槽，避免 Option<String> 破坏 Metrics/Skin 的 Copy。
    pub icon: Option<IconSlot>,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            radius: 8,
            font_scale: 1.0,
            opacity: 1.0,
            scrollbar: true,
            icon: None,
        }
    }
}

/// metrics.icon 的 Copy 承载体：64 字节 UTF-8 槽（超长内容截断丢弃，永不 panic）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconSlot {
    buf: [u8; 64],
    len: u8,
}

impl IconSlot {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }
}

impl std::fmt::Display for IconSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for IconSlot {
    fn from(text: &str) -> Self {
        let mut slot = IconSlot {
            buf: [0; 64],
            len: 0,
        };
        // 按 UTF-8 边界截断，避免半个码元
        let mut end = text.len().min(64);
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        slot.buf[..end].copy_from_slice(&text.as_bytes()[..end]);
        slot.len = end as u8;
        slot
    }
}

/// 阴影壳配置（schema v2 顶层 `shadow` 段）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shadow {
    pub enabled: bool,
    pub radius: i32,
    pub alpha: u8,
}

impl Default for Shadow {
    /// v1 文件没有 shadow 段：默认关闭，行为与旧版完全一致。
    fn default() -> Self {
        Shadow {
            enabled: false,
            radius: 18,
            alpha: 64,
        }
    }
}

/// 全量皮肤：当前主题变体的颜色 + 度量 + 阴影 + 主题标记。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skin {
    pub candidate: CandidateColors,
    pub metrics: Metrics,
    pub shadow: Shadow,
    pub dark_mode: bool,
}

impl Default for Skin {
    /// 亮色默认皮肤（不落盘、不读注册表的安全初值）。
    fn default() -> Self {
        Skin::default_for(false)
    }
}

impl Skin {
    fn default_for(dark: bool) -> Self {
        Skin {
            candidate: if dark {
                CandidateColors::dark()
            } else {
                CandidateColors::light()
            },
            metrics: Metrics::default(),
            shadow: Shadow::default(),
            dark_mode: dark,
        }
    }

    /// 纯函数解析：给定 JSON 文本与目标主题构建 Skin；任何损坏都安全回退。
    #[allow(dead_code)] // TSF 侧经由旧 API 间接调用；host 侧直接使用
    pub fn from_json(text: &str, dark: bool) -> Skin {
        build_skin(Some(text), dark)
    }

    /// 读皮肤文件 + 系统主题，返回并缓存 Skin（线程本地）。
    #[allow(dead_code)] // host 侧与旧 API `load_candidate_colors` 使用
    pub fn load() -> Skin {
        load_with(|| None)
    }

    /// 线程本地缓存的当前 Skin；未初始化时按默认路径加载。
    pub fn current() -> Skin {
        SKIN_CACHE.with_borrow_mut(|slot| match slot {
            Some(cached) => cached.skin,
            None => {
                let skin = load_with(|| None);
                *slot = Some(CachedSkin {
                    skin,
                    source: resolved_skin_path(None),
                });
                skin
            }
        })
    }

    /// WM_SETTINGCHANGE 到达后调用：重读系统主题 + 皮肤文件，刷新线程缓存。
    pub fn refresh_on_setting_change() -> Skin {
        SKIN_CACHE.with_borrow_mut(|slot| {
            let source = slot.as_ref().and_then(|c| c.source.clone());
            let skin = reload_from_source(source.clone());
            *slot = Some(CachedSkin { skin, source });
            skin
        })
    }
}

struct CachedSkin {
    skin: Skin,
    source: Option<PathBuf>,
}

thread_local! {
    static SKIN_CACHE: RefCell<Option<CachedSkin>> = const { RefCell::new(None) };
}

// ---------------------------------------------------------------------------
// JSON 解析（v1/v2 兼容，serde(default) 全覆盖）
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(default)]
struct SkinFile {
    version: u32,
    light: SkinVariant,
    dark: SkinVariant,
    shadow: ShadowSection,
}

impl Default for SkinFile {
    fn default() -> Self {
        SkinFile {
            version: 1,
            light: SkinVariant::default(),
            dark: SkinVariant::default(),
            shadow: ShadowSection::default(),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SkinVariant {
    candidate: CandidateSection,
    metrics: MetricsSection,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CandidateSection {
    background: Option<String>,
    highlight_background: Option<String>,
    text: Option<String>,
    preedit: Option<String>,
    label: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct MetricsSection {
    radius: Option<i32>,
    font_scale: Option<f32>,
    opacity: Option<f32>,
    scrollbar: Option<bool>,
    icon: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ShadowSection {
    enabled: Option<bool>,
    radius: Option<i32>,
    alpha: Option<u8>,
}

/// 核心构建逻辑：可选的 JSON 文本 + 系统是否深色 → Skin。永不 panic。
fn build_skin(text: Option<&str>, dark: bool) -> Skin {
    let fallback = Skin::default_for(dark);
    let Some(text) = text else {
        return fallback;
    };
    let Ok(file) = serde_json::from_str::<SkinFile>(text) else {
        return fallback;
    };
    // 未识别的未来版本整体回退默认，避免读到语义漂移的字段
    if file.version != 1 && file.version != 2 {
        return fallback;
    }
    let variant = if dark { &file.dark } else { &file.light };
    let candidate = CandidateColors {
        background: variant
            .candidate
            .background
            .as_deref()
            .and_then(parse_colorref)
            .unwrap_or(fallback.candidate.background),
        highlight_background: variant
            .candidate
            .highlight_background
            .as_deref()
            .and_then(parse_colorref)
            .unwrap_or(fallback.candidate.highlight_background),
        text: variant
            .candidate
            .text
            .as_deref()
            .and_then(parse_colorref)
            .unwrap_or(fallback.candidate.text),
        preedit: variant
            .candidate
            .preedit
            .as_deref()
            .and_then(parse_colorref)
            .unwrap_or(fallback.candidate.preedit),
        label: variant
            .candidate
            .label
            .as_deref()
            .and_then(parse_colorref)
            .unwrap_or(fallback.candidate.label),
    };
    let metrics = Metrics {
        radius: variant
            .metrics
            .radius
            .filter(|r| (0..=64).contains(r))
            .unwrap_or(fallback.metrics.radius),
        font_scale: variant
            .metrics
            .font_scale
            .filter(|s| s.is_finite() && *s > 0.0 && *s <= 2.0)
            .unwrap_or(fallback.metrics.font_scale),
        opacity: variant
            .metrics
            .opacity
            .filter(|o| o.is_finite() && *o > 0.0)
            .map(|o| o.min(1.0))
            .unwrap_or(fallback.metrics.opacity),
        scrollbar: variant
            .metrics
            .scrollbar
            .unwrap_or(fallback.metrics.scrollbar),
        icon: variant
            .metrics
            .icon
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(IconSlot::from),
    };
    let shadow = Shadow {
        enabled: file.shadow.enabled.unwrap_or(fallback.shadow.enabled),
        radius: file
            .shadow
            .radius
            .filter(|r| (0..=64).contains(r))
            .unwrap_or(fallback.shadow.radius),
        alpha: file.shadow.alpha.unwrap_or(fallback.shadow.alpha),
    };
    Skin {
        candidate,
        metrics,
        shadow,
        dark_mode: dark,
    }
}

/// 按 Windows COLORREF 所需的 BGR 排列转换 #RRGGBB 或 #AARRGGBB。
fn parse_colorref(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#')?;
    let rgb = match hex.len() {
        6 => hex,
        8 => &hex[2..],
        _ => return None,
    };
    let value = u32::from_str_radix(rgb, 16).ok()?;
    let red = (value >> 16) & 0xff;
    let green = (value >> 8) & 0xff;
    let blue = value & 0xff;
    Some(red | (green << 8) | (blue << 16))
}

// ---------------------------------------------------------------------------
// 系统主题 + 文件装载
// ---------------------------------------------------------------------------

/// 系统是否处于深色模式：HKCU\...\Themes\Personalize\SystemUsesLightTheme
/// （0=dark, 1=light，缺失按 light）。
pub fn system_dark_mode() -> bool {
    windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|key| key.get_u32("SystemUsesLightTheme"))
        .map(|value| value == 0)
        .unwrap_or(false)
}

/// 按默认规则装载皮肤并写入线程缓存；`extra` 允许调用方注入额外的
/// 候选文件路径（TSF DLL 用它指向 DLL 旁的 schemas 目录）。
pub fn load_with(extra: impl FnOnce() -> Option<PathBuf>) -> Skin {
    let source = resolved_skin_path(extra());
    let skin = reload_from_source(source.clone());
    SKIN_CACHE.with_borrow_mut(|slot| {
        *slot = Some(CachedSkin { skin, source });
    });
    skin
}

fn reload_from_source(source: Option<PathBuf>) -> Skin {
    let text = source.and_then(read_skin_text);
    build_skin(text.as_deref(), system_dark_mode())
}

fn read_skin_text(path: PathBuf) -> Option<String> {
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.len() > 128 * 1024 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// 皮肤文件查找链（命中即止）：SHURUFA_SKIN_PATH → %APPDATA%\shurufa →
/// 当前 exe 旁的 schemas 目录（host 部署形态） → 调用方注入路径（TSF DLL 旁）。
pub fn resolved_skin_path(extra: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SHURUFA_SKIN_PATH").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    let user = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|root| root.join("shurufa").join("shurufa-skin.json"));
    if user.as_ref().is_some_and(|path| path.is_file()) {
        return user;
    }
    let exe_sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        .map(|dir| dir.join("schemas").join("shurufa-skin.json"));
    if exe_sibling.as_ref().is_some_and(|path| path.is_file()) {
        return exe_sibling;
    }
    extra.filter(|path| path.is_file())
}

/// WM_SETTINGCHANGE 的 lparam 是否意味着主题变化（"ImmersiveColorSet"）；
/// lparam 为空时是泛泛的设置广播，也按主题可能已变处理（保守刷新）。
/// 备注：Windows 目标之外恒为 true，方便纯逻辑测试。
pub fn is_immersive_color_change(lparam: windows::Win32::Foundation::LPARAM) -> bool {
    if lparam.0 == 0 {
        return true;
    }
    #[cfg(windows)]
    {
        unsafe {
            windows::core::PCWSTR(lparam.0 as *const u16)
                .to_string()
                .map(|s| s == "ImmersiveColorSet")
                .unwrap_or(false)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = lparam;
        true
    }
}

// ---------------------------------------------------------------------------
// DWM 外观与阴影壳（供三个自绘窗口统一调用）
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod dwm_impl {
    use super::{Shadow, Skin};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HBRUSH, HGDIOBJ,
        PAINTSTRUCT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, LoadCursorW,
        RegisterClassW, SetLayeredWindowAttributes, SetWindowLongPtrW, ShowWindow, CS_HREDRAW,
        CS_VREDRAW, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE,
        WM_NCHITTEST, WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_POPUP,
    };

    // DWM 属性裸值（DWMWA_WINDOW_CORNER_PREFERENCE / DWMWA_USE_IMMERSIVE_DARK_MODE），
    // 用数值构造避开 windows crate 版本间符号出现/改名差异。
    const DWMWA_WINDOW_CORNER_PREFERENCE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(33);
    const DWMWA_USE_IMMERSIVE_DARK_MODE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(20);
    const DWMWCP_ROUND: u32 = 2;

    const SHELL_CLASS: PCWSTR = w!("ShurufaShadowShell");
    /// SetWindowLongPtr 的索引常量（GWL_EXSTYLE）。
    const GWL_EXSTYLE_INDEX: i32 = -20;

    /// 对一个自绘弹窗统一应用皮肤外观：Win11 圆角、沉浸式深色边框、
    /// metrics.opacity < 1 时启用分层窗口整体透明。
    /// 全部调用失败静默——Win10 没有圆角属性，落回直角也不破坏功能。
    pub fn apply_appearance(hwnd: HWND, skin: &Skin) {
        unsafe {
            let corner: u32 = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const u32 as *const core::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            );
            let dark: u32 = u32::from(skin.dark_mode);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const u32 as *const core::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            );
            apply_opacity(hwnd, skin.metrics.opacity);
        }
    }

    fn apply_opacity(hwnd: HWND, opacity: f32) {
        if opacity >= 0.999 {
            return;
        }
        let alpha = (opacity.clamp(0.05, 1.0) * 255.0).round() as u8;
        unsafe {
            let style = GetWindowLongPtrW(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX(GWL_EXSTYLE_INDEX),
            );
            SetWindowLongPtrW(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX(GWL_EXSTYLE_INDEX),
                style | WS_EX_LAYERED.0 as isize,
            );
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        }
    }

    /// 阴影壳：主窗下层的一个半透明黑色 WS_POPUP。
    /// 不抢焦点（WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW）、不挡点击
    /// （WS_EX_LAYERED + WM_NCHITTEST 返回 HTTRANSPARENT），z 序紧贴主窗下方。
    pub struct ShadowShell {
        hwnd: Option<HWND>,
    }

    impl ShadowShell {
        pub fn new() -> Self {
            ShadowShell { hwnd: None }
        }

        fn ensure_window(&mut self) -> Option<HWND> {
            if let Some(hwnd) = self.hwnd {
                return Some(hwnd);
            }
            unsafe {
                let hinstance = GetModuleHandleW(PCWSTR::null()).ok()?;
                // 重复注册返回 0，忽略即可
                let class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(shell_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: SHELL_CLASS,
                    hbrBackground: HBRUSH::default(),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    ..Default::default()
                };
                RegisterClassW(&class);
                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                    SHELL_CLASS,
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(hinstance.into()),
                    None,
                )
                .ok()?;
                // 壳自身也拿 Win11 圆角，阴影轮廓跟主窗一致
                let corner: u32 = DWMWCP_ROUND;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &corner as *const u32 as *const core::ffi::c_void,
                    std::mem::size_of::<u32>() as u32,
                );
                self.hwnd = Some(hwnd);
                Some(hwnd)
            }
        }

        /// 让阴影壳贴着主窗 (x,y,w,h)：向外扩 radius/2，向下偏 radius/6，
        /// 然后调整 z 序为 壳 → 主窗（SetWindowPos 以壳为 insert-after）。
        pub fn sync(&mut self, owner: HWND, x: i32, y: i32, w: i32, h: i32, shadow: &Shadow) {
            if !shadow.enabled {
                self.hide();
                return;
            }
            let Some(shell) = self.ensure_window() else {
                return;
            };
            let pad = (shadow.radius / 2).max(2);
            let drop = (shadow.radius / 6).max(1);
            let alpha = shadow.alpha;
            unsafe {
                let _ = SetLayeredWindowAttributes(shell, COLORREF(0), alpha, LWA_ALPHA);
                let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                    shell,
                    Some(HWND_TOPMOST),
                    x - pad,
                    y - pad + drop,
                    w + pad * 2,
                    h + pad * 2,
                    SWP_NOACTIVATE,
                );
                let _ = ShowWindow(shell, SW_SHOWNOACTIVATE);
                // 主窗紧贴壳之上（保持 NOACTIVATE 语义）
                let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                    owner,
                    Some(shell),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE,
                );
            }
        }

        pub fn hide(&mut self) {
            if let Some(hwnd) = self.hwnd {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
        }

        #[allow(dead_code)] // candidates 用它；host 面板常驻不销毁壳
        pub fn destroy(&mut self) {
            if let Some(hwnd) = self.hwnd.take() {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
    }

    unsafe extern "system" fn shell_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            // 命中测试透明：鼠标事件穿透到下层窗口，壳绝不拦截候选窗/面板交互
            value if value == WM_NCHITTEST => LRESULT(-1), // HTTRANSPARENT
            value if value == WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                // 纯黑填充，整体透明度由 LWA_ALPHA 控制
                let black = CreateSolidBrush(COLORREF(0));
                FillRect(hdc, &ps.rcPaint, black);
                let _ = DeleteObject(HGDIOBJ(black.0));
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(windows)]
pub use dwm_impl::{apply_appearance, ShadowShell};

// ---------------------------------------------------------------------------
// 候选窗翻页滚动条（metrics.scrollbar；GDI/D2D 两路径共用的纯计算）
// ---------------------------------------------------------------------------
// 本段由 TSF（candidate_window GDI/D2D 路径）消费；host 以 #[path] 复用
// 同一份 skin.rs 但只用到候选/面板配色——宿主构建里这些项是死代码，
// 统一豁免，避免两处编译配置漂移。

/// 滚动条轨道宽度（96 DPI 基准像素），绘制时按 dpi 缩放。
#[allow(dead_code)]
pub const SCROLLBAR_BASE_WIDTH: i32 = 4;

/// 一页的滚动条几何：thumb 呼吸一个 item 槽位；进度 = page_no / max(total-1,1)。
/// total_pages <= 1 时调用方应跳过绘制。坐标全为客户区像素。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct ScrollbarGeo {
    pub track: [i32; 4],
    pub thumb: [i32; 4],
}

/// 由深色 RGB 明度推一个"略深一档"的轨道色（COLORREF 输入输出）。
/// 输入 <0x20 视为近黑（暗色皮肤），改为把三色各 +24 提亮；
/// 避免暗色皮肤的轨道算成死黑导致隐没。
#[allow(dead_code)]
fn darkened_colorref(c: u32) -> u32 {
    let r = c & 0xff;
    let g = (c >> 8) & 0xff;
    let b = (c >> 16) & 0xff;
    let near_black = r.max(g).max(b) < 0x20;
    let ch = |v: u32| -> u32 {
        if near_black {
            (v + 24).min(0xff)
        } else {
            (v * 29) / 32
        }
    };
    ch(r) | (ch(g) << 8) | (ch(b) << 16)
}

/// 滚动条几何（track 右缘贴边、上下各留 v_pad；thumb 高度 = 一个 item 槽位）。
/// `width/height` 客户区像素，`item_w` 当前页最宽槽位，`v_pad` 上下内边距。
#[allow(dead_code)]
pub fn scrollbar_geo(
    width: i32,
    height: i32,
    item_w: i32,
    v_pad: i32,
    track_w: i32,
    page_no: usize,
    total_pages: usize,
) -> Option<ScrollbarGeo> {
    if total_pages <= 1 || track_w <= 0 || width <= 0 || height <= 0 {
        return None;
    }
    let right = width;
    let left = right - track_w;
    let top = v_pad;
    let bottom = (height - v_pad).max(top);
    let span = bottom - top;
    let thumb_h = item_w.clamp(20, span.max(1));
    let progress_span = (span - thumb_h).max(0);
    let thumb_y = top
        + ((progress_span as i64) * (page_no as i64) / ((total_pages - 1).max(1) as i64)) as i32;
    Some(ScrollbarGeo {
        track: [left, top, right, bottom],
        thumb: [left, thumb_y, right, thumb_y + thumb_h],
    })
}

/// 皮肤派色的滚动条配色（COLORREF BGR）：track = 背景略深色，thumb = 高亮色。
#[allow(dead_code)]
pub fn scrollbar_colors(skin: &Skin) -> (u32, u32) {
    (
        darkened_colorref(skin.candidate.background),
        skin.candidate.highlight_background,
    )
}

// ---------------------------------------------------------------------------
// 旧 API（向后兼容，委托到新结构）
// ---------------------------------------------------------------------------

/// 从 JSON 文本取 Windows 候选窗颜色；错误与未知版本全部安全回退。
/// 委托到 `Skin::from_json(text, false)`（旧行为只看亮色变体）。
#[allow(dead_code)] // 向后兼容保留；窗口代码已改用 Skin
pub fn candidate_colors_from_json(text: &str) -> CandidateColors {
    Skin::from_json(text, false).candidate
}

/// 读取用户覆盖、开发覆盖或部署的默认皮肤（亮色；主题感知请用 `Skin::load`）。
#[allow(dead_code)] // 向后兼容保留；窗口代码已改用 Skin
pub fn load_candidate_colors() -> CandidateColors {
    Skin::load().candidate
}

#[cfg(test)]
mod tests {
    use super::{build_skin, candidate_colors_from_json, CandidateColors, Metrics, Shadow, Skin};

    const V1_JSON: &str = r##"{
        "version": 1,
        "light": {
            "candidate": {
                "background": "#112233",
                "highlight_background": "#445566",
                "text": "#778899",
                "preedit": "#AABBCC",
                "label": "#DDEEFF"
            }
        },
        "dark": {
            "candidate": {
                "background": "#010203",
                "highlight_background": "#040506",
                "text": "#070809",
                "preedit": "#0A0B0C",
                "label": "#0D0E0F"
            }
        }
    }"##;

    const V2_JSON: &str = r##"{
        "version": 2,
        "light": {
            "candidate": {
                "background": "#FFFFFF",
                "highlight_background": "#D6EBE1",
                "text": "#111418",
                "preedit": "#9AA2AB",
                "label": "#1B9E77"
            },
            "metrics": { "radius": 10, "font_scale": 1.25, "opacity": 0.9, "scrollbar": false, "icon": "asset://icons/cand" }
        },
        "dark": {
            "candidate": {
                "background": "#1E2126",
                "highlight_background": "#2E4038",
                "text": "#F0F1F3",
                "preedit": "#8E9399",
                "label": "#4ECDA2"
            },
            "metrics": { "radius": 12, "font_scale": 0.9, "opacity": 0.85 }
        },
        "shadow": { "enabled": true, "radius": 18, "alpha": 64 }
    }"##;

    #[test]
    fn maps_shared_candidate_colors_to_colorref() {
        let colors = candidate_colors_from_json(V1_JSON);
        assert_eq!(colors.background, 0x0033_2211);
        assert_eq!(colors.highlight_background, 0x0066_5544);
        assert_eq!(colors.text, 0x0099_8877);
        assert_eq!(colors.preedit, 0x00CC_BBAA);
        assert_eq!(colors.label, 0x00FF_EEDD);
    }

    #[test]
    fn malformed_color_keeps_the_default() {
        let colors = candidate_colors_from_json(
            r##"{"version":1,"light":{"candidate":{"background":"orange"}}}"##,
        );
        assert_eq!(colors, CandidateColors::default());
    }

    #[test]
    fn v1_file_reads_light_variant_with_default_metrics() {
        let skin = build_skin(Some(V1_JSON), false);
        assert_eq!(skin.candidate.background, 0x0033_2211);
        assert_eq!(skin.metrics, Metrics::default());
        assert_eq!(skin.shadow, Shadow::default());
        assert!(!skin.dark_mode);
        // v1 文件也带 dark 段：深色模式下也能取到
        let dark = build_skin(Some(V1_JSON), true);
        assert_eq!(dark.candidate.background, 0x0003_0201);
        assert!(dark.dark_mode);
    }

    #[test]
    fn v2_full_fields_parse() {
        let skin = build_skin(Some(V2_JSON), false);
        assert_eq!(skin.candidate.background, 0x00FF_FFFF);
        assert_eq!(skin.candidate.label, 0x0077_9E1B);
        assert_eq!(skin.metrics.radius, 10);
        assert!((skin.metrics.font_scale - 1.25).abs() < 1e-6);
        assert!((skin.metrics.opacity - 0.9).abs() < 1e-6);
        assert!(!skin.metrics.scrollbar);
        assert_eq!(
            skin.metrics.icon.map(|s| s.as_str().to_owned()).as_deref(),
            Some("asset://icons/cand")
        );
        assert_eq!(
            skin.shadow,
            Shadow {
                enabled: true,
                radius: 18,
                alpha: 64
            }
        );
    }

    #[test]
    fn v2_missing_metrics_falls_back() {
        let text = r##"{
            "version": 2,
            "light": { "candidate": { "background": "#112233" } },
            "dark": { "candidate": { "background": "#010203" } }
        }"##;
        let skin = build_skin(Some(text), true);
        assert_eq!(skin.candidate.background, 0x0003_0201);
        assert_eq!(skin.metrics, Metrics::default());
        assert_eq!(skin.shadow, Shadow::default());
    }

    #[test]
    fn broken_json_returns_theme_defaults() {
        let light = build_skin(Some("{not json"), false);
        assert_eq!(light, Skin::default_for(false));
        let dark = build_skin(Some("{not json"), true);
        assert_eq!(dark, Skin::default_for(true));
        // 空文件路径场景
        assert_eq!(build_skin(None, false), Skin::default_for(false));
    }

    #[test]
    fn dark_to_light_switch_flips_variant_and_flag() {
        let dark = build_skin(Some(V2_JSON), true);
        assert!(dark.dark_mode);
        assert_eq!(dark.candidate.background, 0x0026_211E);
        assert_eq!(dark.candidate.label, 0x00A2_CD4E);
        assert!((dark.metrics.opacity - 0.85).abs() < 1e-6);

        let light = build_skin(Some(V2_JSON), false);
        assert!(!light.dark_mode);
        assert_eq!(light.candidate.background, 0x00FF_FFFF);
        assert_ne!(light.candidate.background, dark.candidate.background);
    }

    #[test]
    fn invalid_metrics_are_clamped() {
        let text = r##"{
            "version": 2,
            "light": { "metrics": { "radius": 999, "font_scale": -1.0, "opacity": 7.0 } },
            "dark": {}
        }"##;
        let skin = build_skin(Some(text), false);
        assert_eq!(skin.metrics.radius, 8);
        assert!((skin.metrics.font_scale - 1.0).abs() < 1e-6);
        assert!((skin.metrics.opacity - 1.0).abs() < 1e-6);
        // 新字段缺省：scrollbar 默认开、icon 预留为 None
        assert!(skin.metrics.scrollbar);
        assert!(skin.metrics.icon.is_none());
    }

    #[test]
    fn scrollbar_requires_multiple_pages() {
        assert!(super::scrollbar_geo(400, 100, 80, 12, 4, 0, 1).is_none());
        assert!(super::scrollbar_geo(400, 100, 80, 12, 4, 0, 0).is_none());
        let geo = super::scrollbar_geo(400, 100, 40, 12, 4, 1, 3).expect("3 页应有几何");
        assert_eq!(geo.track, [396, 12, 400, 88]);
        // 轨道跨度 76px；item_w=40 作为 thumb 高 → 进度跨度 76-40=36
        // 第 2/3 页（page_no=1）：thumb 顶 = 12 + 36*1/2 = 30
        assert_eq!(geo.thumb, [396, 30, 400, 70]);
        // 末页归底
        let last = super::scrollbar_geo(400, 100, 40, 12, 4, 2, 3).expect("末页");
        assert_eq!(last.thumb, [396, 48, 400, 88]);
    }

    #[test]
    fn scrollbar_track_darkens_background() {
        // 亮底 #F0F0F0 各通道 *29/32 = 217 = 0xD9（整除截断），thumb 取高亮色
        let mut skin = Skin::default();
        skin.candidate.background = 0x00F0_F0F0;
        skin.candidate.highlight_background = 0x0000_80FF;
        let (track, thumb) = super::scrollbar_colors(&skin);
        assert_eq!(track, 0x00D9_D9D9);
        assert_eq!(thumb, 0x0000_80FF);
        // 暗底 #181818 明度极低：轨道改为提亮而非压成死黑
        skin.candidate.background = 0x0018_1818;
        let (track, _) = super::scrollbar_colors(&skin);
        assert_eq!(track, 0x0030_3030);
    }
}
