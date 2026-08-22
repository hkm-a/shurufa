//! 跨端皮肤 JSON（shurufa-skin.json）的纯数据模型与解析器。
//!
//! 这是 `core/skin`：只包含 schema v1/v2 的模型、默认值、颜色解析与
//! 滚动条纯计算，**不依赖任何 Windows API**。Windows 专属的 DWM 外观、
//! 阴影壳、系统主题读取与文件缓存位于 `platforms/windows-skin`。
//!
//! ## shurufa-skin.json schema v2 文档
//!
//! 完整字段说明见 `platforms/windows-skin` 或原 `platforms/windows/src/skin.rs`
//! 的历史注释；这里保持与 v2 相同的解析语义：
//! - 所有 v2 新字段均为 Optional（`#[serde(default)]`），缺失回退内置默认；
//! - 颜色非法字符串只回退该字段，不影响其余字段；
//! - `version` 只能是 1 或 2，其他版本整体回退默认。
//!
//! 本 crate 可被 Windows / Android / 测试直接复用。

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

/// 皮肤度量：圆角、字号倍率、整体透明度 + 间距参数 + 候选窗滚动条/图标槽位。
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
    /// 窗口内边距（基准 px，随 DPI 缩放）；0 = 用内置默认 12。
    pub padding: i32,
    /// 候选间距（基准 px，随 DPI 缩放）；0 = 用内置默认 22。
    pub item_gap: i32,
    /// 序号与候选词间距（基准 px）；0 = 用内置默认 6。
    pub label_gap: i32,
    /// 高亮候选左右留白（基准 px）；0 = 用内置默认 7。
    pub hl_pad: i32,
    /// 单候选行高（基准 px）；0 = 用内置默认 40。
    pub row_h: i32,
    /// preedit 区高度（基准 px）；0 = 用内置默认 26。
    pub preedit_h: i32,
    /// 候选文本超过该字符数时截断显示省略号（weasel candidate_abbreviate_length
    /// 同款；0 = 不截断）。只影响显示，上屏/选中仍用完整文本（引擎按索引提交）。
    pub abbreviate_length: i32,
    /// 候选来源角标（搜狗/百度来源标识同类）：按文本特征启发式分类
    /// （英文/emoji/日期/单字/词）渲染小角标；默认关（信息价值有限，避免噪音）。
    pub show_candidate_badge: bool,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            radius: 8,
            font_scale: 1.0,
            opacity: 1.0,
            scrollbar: true,
            icon: None,
            padding: 0,
            item_gap: 0,
            label_gap: 0,
            hl_pad: 0,
            row_h: 0,
            preedit_h: 0,
            // 长候选缩写默认开（weasel candidate_abbreviate_length 同款）：
            // 单条候选超 24 字符截断显示省略号，避免长词/日期/ID 撑爆候选行。
            abbreviate_length: 24,
            // 候选来源角标默认关（启发式分类信息价值有限，按需开启）。
            show_candidate_badge: false,
        }
    }
}

impl Metrics {
    // 供 shurufa-tsf 候选窗布局消费；host（#[path] 引入本模块）暂未使用
    // 间距参数，允许 dead_code 避免跨 crate 编译告警。
    #[allow(dead_code)]
    /// 取有效间距值：>0 用皮肤值，否则用内置默认（调用方传入布局常量）。
    pub fn padding_or(&self, default: i32) -> i32 {
        if self.padding > 0 {
            self.padding
        } else {
            default
        }
    }
    #[allow(dead_code)]
    pub fn item_gap_or(&self, default: i32) -> i32 {
        if self.item_gap > 0 {
            self.item_gap
        } else {
            default
        }
    }
    #[allow(dead_code)]
    pub fn label_gap_or(&self, default: i32) -> i32 {
        if self.label_gap > 0 {
            self.label_gap
        } else {
            default
        }
    }
    #[allow(dead_code)]
    pub fn hl_pad_or(&self, default: i32) -> i32 {
        if self.hl_pad > 0 {
            self.hl_pad
        } else {
            default
        }
    }
    #[allow(dead_code)]
    pub fn row_h_or(&self, default: i32) -> i32 {
        if self.row_h > 0 {
            self.row_h
        } else {
            default
        }
    }
    #[allow(dead_code)]
    pub fn preedit_h_or(&self, default: i32) -> i32 {
        if self.preedit_h > 0 {
            self.preedit_h
        } else {
            default
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
    padding: Option<i32>,
    item_gap: Option<i32>,
    label_gap: Option<i32>,
    hl_pad: Option<i32>,
    row_h: Option<i32>,
    preedit_h: Option<i32>,
    abbreviate_length: Option<i32>,
    show_candidate_badge: Option<bool>,
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
        // 间距参数：合法范围 0..=256（0 = 用内置默认），超界回退默认。
        padding: parse_spacing(variant.metrics.padding, fallback.metrics.padding),
        item_gap: parse_spacing(variant.metrics.item_gap, fallback.metrics.item_gap),
        label_gap: parse_spacing(variant.metrics.label_gap, fallback.metrics.label_gap),
        hl_pad: parse_spacing(variant.metrics.hl_pad, fallback.metrics.hl_pad),
        row_h: parse_spacing(variant.metrics.row_h, fallback.metrics.row_h),
        preedit_h: parse_spacing(variant.metrics.preedit_h, fallback.metrics.preedit_h),
        // 候选截断长度：合法范围 4..=64（0 = 不截断），超界回退默认。
        abbreviate_length: variant
            .metrics
            .abbreviate_length
            .filter(|n| *n == 0 || (4..=64).contains(n))
            .unwrap_or(fallback.metrics.abbreviate_length),
        show_candidate_badge: variant
            .metrics
            .show_candidate_badge
            .unwrap_or(fallback.metrics.show_candidate_badge),
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

/// 间距参数解析：合法 0..=256（0 = 用内置默认），其余回退 fallback。
fn parse_spacing(value: Option<i32>, fallback: i32) -> i32 {
    value.filter(|v| (0..=256).contains(v)).unwrap_or(fallback)
}

/// 按 Windows COLORREF 所需的 BGR 排列转换颜色文本（#RRGGBB / #AARRGGBB / CSS 颜色名）。
fn parse_colorref(text: &str) -> Option<u32> {
    let color = csscolorparser::parse(text).ok()?;
    let [red, green, blue, _] = color.to_rgba8();
    Some(red as u32 | ((green as u32) << 8) | ((blue as u32) << 16))
}

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

#[cfg(test)]
mod tests {
    use super::{
        build_skin, candidate_colors_from_json, scrollbar_colors, scrollbar_geo, CandidateColors,
        Metrics, Shadow, Skin,
    };

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
            r##"{"version":1,"light":{"candidate":{"background":"#xyz"}}}"##,
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
        assert!(skin.metrics.scrollbar);
        assert!(skin.metrics.icon.is_none());
        assert_eq!(skin.metrics.padding, 0);
        assert_eq!(skin.metrics.item_gap, 0);
        assert_eq!(skin.metrics.label_gap, 0);
        assert_eq!(skin.metrics.hl_pad, 0);
        assert_eq!(skin.metrics.row_h, 0);
        assert_eq!(skin.metrics.preedit_h, 0);
    }

    #[test]
    fn spacing_metrics_parse_and_clamp() {
        let text = r##"{
            "version": 2,
            "light": { "metrics": {
                "padding": 16, "item_gap": 28, "label_gap": 8,
                "hl_pad": 9, "row_h": 48, "preedit_h": 30
            } },
            "dark": { "metrics": { "padding": 999, "item_gap": -5 } }
        }"##;
        let skin = build_skin(Some(text), false);
        assert_eq!(skin.metrics.padding, 16);
        assert_eq!(skin.metrics.item_gap, 28);
        assert_eq!(skin.metrics.label_gap, 8);
        assert_eq!(skin.metrics.hl_pad, 9);
        assert_eq!(skin.metrics.row_h, 48);
        assert_eq!(skin.metrics.preedit_h, 30);
        let dark = build_skin(Some(text), true);
        assert_eq!(dark.metrics.padding, 0);
        assert_eq!(dark.metrics.item_gap, 0);
        assert_eq!(skin.metrics.padding_or(12), 16);
        assert_eq!(skin.metrics.row_h_or(40), 48);
        assert_eq!(dark.metrics.padding_or(12), 12);
    }

    #[test]
    fn scrollbar_requires_multiple_pages() {
        assert!(scrollbar_geo(400, 100, 80, 12, 4, 0, 1).is_none());
        assert!(scrollbar_geo(400, 100, 80, 12, 4, 0, 0).is_none());
        let geo = scrollbar_geo(400, 100, 40, 12, 4, 1, 3).expect("3 页应有几何");
        assert_eq!(geo.track, [396, 12, 400, 88]);
        assert_eq!(geo.thumb, [396, 30, 400, 70]);
        let last = scrollbar_geo(400, 100, 40, 12, 4, 2, 3).expect("末页");
        assert_eq!(last.thumb, [396, 48, 400, 88]);
    }

    #[test]
    fn scrollbar_track_darkens_background() {
        let mut skin = Skin::default();
        skin.candidate.background = 0x00F0_F0F0;
        skin.candidate.highlight_background = 0x0000_80FF;
        let (track, thumb) = scrollbar_colors(&skin);
        assert_eq!(track, 0x00D9_D9D9);
        assert_eq!(thumb, 0x0000_80FF);
        skin.candidate.background = 0x0018_1818;
        let (track, _) = scrollbar_colors(&skin);
        assert_eq!(track, 0x0030_3030);
    }
}
