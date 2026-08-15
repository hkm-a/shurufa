//! 候选窗：GDI 绘制的置顶弹窗，横向排列编号候选（主流输入法布局）。
//!
//! 窗口在宿主应用的 UI 线程内创建（TSF 单元线程模型），
//! 不抢焦点（WS_EX_NOACTIVATE），随组合文本位置移动，
//! 宽度按候选文本实测宽度自适应，所有尺寸按窗口 DPI 缩放。
//!
//! 本轮改动摘要（皮肤 v2 / 现代化外观）：
//! - 颜色不再硬编码，全部来自 `crate::skin::Skin`（按系统 light/dark 主题选取）。
//! - 字号乘 `metrics.font_scale`；`metrics.opacity` < 1 时启用分层窗口整体透明。
//! - 创建后由 `skin::apply_appearance` 应用 Win11 DWM 圆角 + 沉浸式深色边框；
//!   `skin::ShadowShell` 在主窗下方绘制半透明阴影壳（NOACTIVATE + 命中穿透）。
//! - WM_SETTINGCHANGE("ImmersiveColorSet") 热切换主题：刷新 Skin 缓存并重绘。

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, GetDC, GetTextExtentPoint32W, InvalidateRect, LineTo, MoveToEx, ReleaseDC,
    SelectObject, SetBkMode, SetTextColor, ValidateRect, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, FW_NORMAL, HBRUSH, HDC, HFONT, HGDIOBJ, HPEN, PAINTSTRUCT, PS_SOLID, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, TrackMouseEvent, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, LoadCursorW, MoveWindow,
    RegisterClassW, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, IDC_ARROW,
    SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE,
    WM_DESTROY, WM_DPICHANGED, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT,
    WM_SETTINGCHANGE, WM_SIZE, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

use ime_ipc::Context;

use crate::skin::{self, ShadowShell, Skin};

// ---------------------------------------------------------------------------
// 渲染后端瀑布链：DComp → D2D → GDI
// ---------------------------------------------------------------------------

/// 候选窗渲染后端枚举。选路由 `backend_kind()` 懒解析：
/// 1. probe DComp（D3D11 + IDXGIFactory2 + DCompositionCreateDevice 全通）→ DComp
/// 2. D2D try_init 成功 → D2D
/// 3. 否则 → Gdi（纯软件绘制，恒可用，绝无 panic）
///
/// 与模块内 `Backend::{Pending,Ready,Failed}`
/// 状态机互补：那个管"是否处于 ready"，这个管"走哪条路径"。
/// 每帧 WM_PAINT 先取 kind；当帧渲染失败（返回 false）时当场降级到下一级重画，
/// **不在状态机里永久打 Failed 标记**——TDR/驱动重置由 D2D/DComp 各自的
/// notify_resize + 惰性重建吸收，不至于把整会话钉死在低档位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    /// DComp flip-model swapchain + premultiplied 圆角纹理（wave 4 新增）
    DComp,
    /// 既有 D2D 1.1 + DirectWrite（wave 1 落地）
    D2D,
    /// 兜底 GDI（legacy；布局/测量零漂移）
    Gdi,
}

thread_local! {
    /// 每个宿主 UI 线程解析一次；不在 wnd_proc 里反复 D3D11CreateDevice。
    static BACKEND_KIND: std::cell::Cell<Option<BackendKind>> = const { std::cell::Cell::new(None) };
}

/// 选路（懒解析 + thread-local 缓存）。首次调用即按 `probe_dcomp_available` → `candidate_window_d2d::is_enabled` 定档。
pub(crate) fn backend_kind() -> BackendKind {
    BACKEND_KIND.with(|c| {
        if let Some(k) = c.get() {
            return k;
        }
        let k = if crate::candidate_window_dcomp::probe_dcomp_available() {
            BackendKind::DComp
        } else if crate::candidate_window_d2d::is_enabled() {
            BackendKind::D2D
        } else {
            BackendKind::Gdi
        };
        c.set(Some(k));
        k
    })
}

/// 供测试/调试覆盖选路结果（正常生产路径不调）。
#[cfg(test)]
fn set_backend_kind_for_test(k: BackendKind) {
    BACKEND_KIND.with(|c| c.set(Some(k)));
}

const CLASS_NAME: PCWSTR = w!("ShurufaCandidateWindow");

// 96 DPI 下的基准尺寸，运行期按窗口实际 DPI 缩放
const BASE_ROW_HEIGHT: i32 = 40;
const BASE_PREEDIT_HEIGHT: i32 = 26;
const BASE_PADDING: i32 = 12;
const BASE_ITEM_GAP: i32 = 22;
const BASE_LABEL_GAP: i32 = 6;
const BASE_HL_PAD: i32 = 7;
const BASE_FONT_HEIGHT: i32 = 26;
const BASE_PREEDIT_FONT_HEIGHT: i32 = 18;
const BASE_MIN_WIDTH: i32 = 96;
const BASE_MODE_BADGE_GAP: i32 = 10;

/// 单个候选的横向布局槽位（坐标为窗口客户区像素）
struct Item {
    label: String,
    text: String,
    /// 词库附注（如同类推荐、近义词、emoji 提示）；为空则不绘制。
    comment: String,
    x: i32,
    label_w: i32,
    text_w: i32,
    highlighted: bool,
    /// 鼠标悬停中（不含已被选中的项：选中优先，见 make_paint_view）。
    hovered: bool,
    /// 主文本不含 comment 的实测宽度（GDI show() 时记录）；D2D comment 起点复用。
    pure_text_w: i32,
}

struct PaintData {
    preedit: String,
    items: Vec<Item>,
    dpi: u32,
    skin: Skin,
    is_ascii: bool,
    is_full_shape: bool,
    /// 长按 Shift 触发的大写视觉角标（不实际切换引擎 ascii_mode）。
    /// 由 service.rs 经 set_caps_visual 写入；true 时 mode_badge 固定为 "⇪大写"。
    caps_visual: bool,
    page: PageInfo,
}

/// 传给 D2D 后端的一帧绘制视图（纯纯值，不持 Ref 避免跨 thread_local 借）。
/// 所有尺寸都是窗口客户区像素，已按 (dpi, font_scale) 展开。
pub struct PaintView {
    pub preedit: String,
    pub items: Vec<ItemView>,
    pub dpi: u32,
    pub skin: Skin,
    pub padding: i32,
    pub preedit_h: i32,
    pub row_h: i32,
    pub label_gap: i32,
    pub hl_pad: i32,
    /// 右上角模式徽标（None = 当前模式不需要角标）。
    /// 取值："中" / "En" / "⇪大写"；由 mode_badge_text() 统一推导。
    pub mode_badge: Option<&'static str>,
    /// 候选主字体 em-height（px）
    pub cand_font_h: i32,
    /// preedit/副标 小字体 em-height（px）
    pub sub_font_h: i32,
    /// 当前页分页快照（滚动条用）
    pub page: PageInfo,
    /// 预编辑音节分隔符列位（UTF-16 码元索引；空 = 无断点、按原文整串绘制）。
    /// 三条渲染路径共用同一份断点数据，一次扫描全帧消费。
    pub syllable_breaks: Vec<u16>,
}

pub struct ItemView {
    pub label: String,
    pub text: String,
    pub comment: String,
    pub x: i32,
    pub label_w: i32,
    pub text_w: i32,
    pub pure_text_w: i32,
    pub highlighted: bool,
    /// 鼠标悬停中（选中项恒为 true 由 make_paint_view 归并，见 Item.hovered）。
    pub hovered: bool,
}

/// 预编辑串内的音节分隔符位置（UTF-16 码元索引）。
///
/// 三种渲染路径共用。分隔符分两类：
/// - **空格**（` `）：Rime 引擎自动插入的音节分隔（`nihao` → `"ni hao"`），
///   是用户可读的分隔，**保留原样绘制**（不画竖线），只做轻微色差分段。
/// - **撇号**（`'`）：用户敲入的音界符（`xi'an`），是输入的一部分，
///   同样保留原样绘制。
///
/// 本函数只负责**找出分隔位置**；绘制层决定如何呈现（空格/撇号本体仍画，
/// 竖线仅作为可选的视觉增强，见 `draw_preedit_segmented`）。
pub fn syllable_breaks(preedit: &str) -> Vec<u16> {
    let mut out = Vec::new();
    for (i, u) in preedit.encode_utf16().enumerate() {
        if u == b' ' as u16 || u == b'\'' as u16 {
            out.push(i as u16);
        }
    }
    out
}

/// 分页滚动条快照（来自引擎 Context；page 双字段由 ime-ipc 透传）。
#[derive(Clone, Copy, Debug, Default)]
pub struct PageInfo {
    pub page_no: usize,
    pub is_last_page: bool,
}

impl PageInfo {
    /// 总页数估计：引擎不直接提供 total；由 page_no + is_last_page 推导
    /// （末页时 total = page_no+1，非末页时 total = page_no+2 下界），
    /// 滚动条 thumb 只表达相对位置，无需精确总页数。
    pub fn total_pages(&self) -> usize {
        self.page_no + if self.is_last_page { 1 } else { 2 }
    }
}

/// 本帧是否需要画滚动条（开皮肤开关 + 非单页；空候选也画——翻页中会出现）。
pub fn scrollbar_active(v: &PaintView) -> bool {
    v.skin.metrics.scrollbar && v.page.total_pages() > 1
}

/// 滚动条轨道像素宽（已按 dpi 缩放；关闭开关/单页时为 0，布局完全不占位）。
pub fn scrollbar_width(v: &PaintView) -> i32 {
    if scrollbar_active(v) {
        scale(skin::SCROLLBAR_BASE_WIDTH, v.dpi)
    } else {
        0
    }
}

/// 生成一帧 D2D 消费的不可变快照；None 表示数据未初始化（窗口还没 show 过）。
pub fn make_paint_view() -> Option<PaintView> {
    PAINT_DATA.with_borrow(|data| {
        let dpi = data.dpi;
        let font_scale = data.skin.metrics.font_scale;
        // 悬停与选中合并：选中项永远全高亮（鼠标悬停在选中项上不降级）；
        // 悬停命中与 HOVER_INDEX 相同（WM_MOUSEMOVE 每次重建后重新命中）。
        let hover = HOVER_INDEX.with(|h| h.get());
        Some(PaintView {
            preedit: data.preedit.clone(),
            items: data
                .items
                .iter()
                .enumerate()
                .map(|(i, it)| ItemView {
                    label: it.label.clone(),
                    text: it.text.clone(),
                    comment: it.comment.clone(),
                    x: it.x,
                    label_w: it.label_w,
                    text_w: it.text_w,
                    pure_text_w: it.pure_text_w,
                    highlighted: it.highlighted,
                    hovered: it.highlighted || hover == Some(i),
                })
                .collect(),
            dpi,
            skin: data.skin,
            padding: scale(BASE_PADDING, dpi),
            preedit_h: scale(BASE_PREEDIT_HEIGHT, dpi),
            row_h: scale(BASE_ROW_HEIGHT, dpi),
            label_gap: scale(BASE_LABEL_GAP, dpi),
            hl_pad: scale(BASE_HL_PAD, dpi),
            mode_badge: mode_badge_text(data),
            cand_font_h: font_height(BASE_FONT_HEIGHT, dpi, font_scale),
            sub_font_h: font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale),
            page: data.page,
            syllable_breaks: syllable_breaks(&data.preedit),
        })
    })
}

/// 徽标宽度（与 GDI paint 内 text_width + BASE_MODE_BADGE_GAP 同公式；
/// GDI 实际测宽结果 ∈ [3*cand_h, 3.5*cand_h]，用近似值差 1px 视觉无感）。
pub fn mode_badge_width(view: &PaintView) -> i32 {
    view.cand_font_h * 3 + scale(BASE_MODE_BADGE_GAP, view.dpi)
}

/// 把 PaintData 快照归一为右上角徽标文案；None = 不该显示角标。
/// 单独成函数让 GDI / D2D / DComp 三条渲染路径共用同一套规则书：
/// - caps_visual 优先级最高（长按 Shift 触发），只显示 "⇪大写"。
/// - 否则按引擎 ascii_mode 输出 "En" / "中"。
///
/// `is_full_shape` 不参与角标：候选行已经反映全/半角字符形态，角标保持最简。
fn mode_badge_text(data: &PaintData) -> Option<&'static str> {
    if data.caps_visual {
        Some("⇪大写")
    } else if data.is_ascii {
        Some("En")
    } else {
        Some("中")
    }
}

/// 长按 Shift 的"大写视觉"状态写入口（service.rs 调用；同 UI 线程）。
///
/// true：角标固定为 "⇪大写"；不清 preedit/candidates，不触碰 IPC。
/// false：角标回退到按 ascii_mode 推导的"中"/"En"。
/// 幂等；返回是否发生变化，供调用方决定是否 InvalidateRect 重绘。
pub fn set_caps_visual(active: bool) -> bool {
    PAINT_DATA.with_borrow_mut(|data| {
        if data.caps_visual == active {
            false
        } else {
            data.caps_visual = active;
            true
        }
    })
}

/// 当前是否处于"大写视觉"提示状态；仅供 service.rs / 单元测试读取真值。
#[allow(dead_code)]
pub fn caps_visual_active() -> bool {
    PAINT_DATA.with_borrow(|data| data.caps_visual)
}

/// 当前 PAINT_DATA 中 preedit 的 syllable_breaks 快照；service.rs 的 Tab
/// 音节导航用它判断是否需要把 Tab 重映射为 Left/Right。None = 还未 show 过。
/// 目前 Tab 导航改由引擎实时 context 驱动，本函数保留为调试/回归查询口。
#[allow(dead_code)]
pub fn current_preedit_breaks() -> Vec<u16> {
    PAINT_DATA.with_borrow(|data| syllable_breaks(&data.preedit))
}

// 绘制数据挂在线程本地：窗口与 TSF 回调同属宿主 UI 线程
thread_local! {
    static PAINT_DATA: RefCell<PaintData> = RefCell::new(PaintData {
        preedit: String::new(),
        items: Vec::new(),
        dpi: 96,
        skin: Skin::default(),
        is_ascii: false,
        is_full_shape: false,
        caps_visual: false,
        page: PageInfo::default(),
    });
    static CLASS_REGISTERED: RefCell<bool> = const { RefCell::new(false) };
    /// 当前鼠标悬停的候选序号（None = 不在任何候选上）。
    /// 每次 show() 重建 items 后由 WM_MOUSEMOVE 重新命中；
    /// 与 PAINT_DATA 同线程（宿主 UI 线程），Cell 足够。
    static HOVER_INDEX: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// 是否已向系统注册 WM_MOUSELEAVE 跟踪（TrackMouseEvent 幂等控制）。
    static HOVER_TRACKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 当前候选窗 HWND（hover 处理与 paint 共用；ensure_window 创建后写入）。
    static PAINT_HWND: std::cell::Cell<HWND> = const { std::cell::Cell::new(HWND(std::ptr::null_mut())) };
}

pub(crate) fn scale(base: i32, dpi: u32) -> i32 {
    (base * dpi as i32 + 48) / 96
}

/// metrics.icon 预留槽：当前不渲染任何图标，每次会话仅提示一次（两渲染路径共用）。
pub fn log_icon_once(skin: &Skin) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    if skin.metrics.icon.is_none() {
        return;
    }
    ONCE.call_once(|| {
        crate::debug_log("skin: metrics.icon slot reserved, not yet rendered");
    });
}

/// DPI 缩放后再乘皮肤字号倍率；下限 8px 防止畸形配置把字压没。
/// D2D 后端按同一公式取 em-height → IDWriteTextFormat 字号。
pub(crate) fn font_height(base: i32, dpi: u32, font_scale: f32) -> i32 {
    ((scale(base, dpi) as f32) * font_scale).round().max(8.0) as i32
}

unsafe fn make_font(height: i32) -> HFONT {
    CreateFontW(
        -height, // 负值表示字符高度（em），排版更稳定
        0,
        0,
        0,
        FW_NORMAL.0 as i32,
        0,
        0,
        0,
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        w!("Microsoft YaHei UI"),
    )
}

unsafe fn text_width(hdc: HDC, text: &str) -> i32 {
    let wide: Vec<u16> = text.encode_utf16().collect();
    if wide.is_empty() {
        return 0;
    }
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
    size.cx
}

pub struct CandidateUi {
    hwnd: Option<HWND>,
    shadow: ShadowShell,
}

impl CandidateUi {
    pub fn new() -> Self {
        // 预热皮肤缓存：TSF DLL 部署形态下皮肤文件在 DLL 旁的 schemas 目录
        let extra = crate::dll_path()
            .parent()
            .map(|dir| dir.join("schemas").join("shurufa-skin.json"));
        let _ = skin::load_with(|| extra);
        // 预热 GPU 工厂（< 5ms）；失败则整段会话降级，不 panic
        crate::candidate_window_d2d::try_init();
        crate::candidate_window_dcomp::try_init();
        CandidateUi {
            hwnd: None,
            shadow: ShadowShell::new(),
        }
    }

    fn ensure_window(&mut self) -> Option<HWND> {
        if let Some(hwnd) = self.hwnd {
            return Some(hwnd);
        }
        unsafe {
            let hinstance = GetModuleHandleW(PCWSTR::null()).ok()?;
            CLASS_REGISTERED.with_borrow_mut(|registered| {
                if !*registered {
                    let class = WNDCLASSW {
                        style: CS_HREDRAW | CS_VREDRAW,
                        lpfnWndProc: Some(wnd_proc),
                        hInstance: hinstance.into(),
                        lpszClassName: CLASS_NAME,
                        hbrBackground: HBRUSH::default(),
                        // 不设光标会导致悬停时一直显示忙碌转圈
                        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                        ..Default::default()
                    };
                    // 同进程重复注册返回 0，忽略即可
                    RegisterClassW(&class);
                    *registered = true;
                }
            });
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!(""),
                WS_POPUP,
                0,
                0,
                BASE_MIN_WIDTH,
                BASE_ROW_HEIGHT,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
            .ok()?;
            // 现代外观：Win11 圆角 + 沉浸式深色边框 + 按皮肤透明度分层
            skin::apply_appearance(hwnd, &Skin::current());
            PAINT_HWND.with(|h| h.set(hwnd));
            self.hwnd = Some(hwnd);
            Some(hwnd)
        }
    }

    /// 用引擎上下文刷新窗口内容并显示在锚点下方。
    pub fn show(&mut self, ctx: &Context, anchor: Option<POINT>) {
        let Some(hwnd) = self.ensure_window() else {
            return;
        };
        // 每次弹出重读皮肤：文件改动/主题改动即时生效（候选文件 <128KiB，代价可忽略）
        let extra = crate::dll_path()
            .parent()
            .map(|dir| dir.join("schemas").join("shurufa-skin.json"));
        let skin = skin::load_with(|| extra);
        skin::apply_appearance(hwnd, &skin);
        log_icon_once(&skin);
        let font_scale = skin.metrics.font_scale;
        // 部分宿主（DPI 虚拟化）对弹窗返回 96 兜底值，取系统 DPI 兜底
        let dpi = unsafe { GetDpiForWindow(hwnd).max(GetDpiForSystem()) }.max(96);
        let padding = scale(BASE_PADDING, dpi);
        let item_gap = scale(BASE_ITEM_GAP, dpi);
        let label_gap = scale(BASE_LABEL_GAP, dpi);
        // 滚动条：皮肤开启且非单页时右缘预留轨道宽；单页/关闭时 0，宽度零漂移
        let page = PageInfo {
            page_no: ctx.page_no,
            is_last_page: ctx.is_last_page,
        };
        let sb_w = if skin.metrics.scrollbar && page.total_pages() > 1 {
            scale(skin::SCROLLBAR_BASE_WIDTH, dpi)
        } else {
            0
        };

        // 用与绘制一致的字体实测文本宽度，横向布槽
        let (items, preedit_w) = unsafe {
            let hdc = GetDC(Some(hwnd));
            let cand_font = make_font(font_height(BASE_FONT_HEIGHT, dpi, font_scale));
            let preedit_font = make_font(font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale));

            let old = SelectObject(hdc, HGDIOBJ(cand_font.0));
            let sub_font = make_font(font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale));
            let mut x = padding;
            let items: Vec<Item> = ctx
                .candidates
                .iter()
                .enumerate()
                .take(9)
                .map(|(i, c)| {
                    // TSF 端候选域固定 9 列；label 为 1..=9（超过 9 的索引不进此分支）
                    let label = format!("{}.", i + 1);
                    let label_w = text_width(hdc, &label);
                    let text_w = text_width(hdc, &c.text);
                    // 副标（词库附注）；只在文本不重复时展示——同字符的
                    // comment 是噪音。这里只截断长度，留待 paint 用小号字体。
                    let comment = if c.comment.is_empty() || c.comment == c.text {
                        String::new()
                    } else {
                        c.comment.chars().take(12).collect()
                    };
                    // 宽度预算给 comment：副标跟在主文本右侧，必须占住后续槽位。
                    // 与 paint 头一致：用 sub_font 实测，再加一侧间隙。
                    let comment_w = if comment.is_empty() {
                        0
                    } else {
                        SelectObject(hdc, HGDIOBJ(sub_font.0));
                        let w = text_width(hdc, &comment);
                        SelectObject(hdc, HGDIOBJ(cand_font.0));
                        w + scale(4, dpi)
                    };
                    let item = Item {
                        label,
                        text: c.text.clone(),
                        comment,
                        x,
                        label_w,
                        text_w: text_w + comment_w,
                        highlighted: i == ctx.highlighted,
                        hovered: false,
                        pure_text_w: text_w,
                    };
                    x += label_w + label_gap + text_w + comment_w + item_gap;
                    item
                })
                .collect();
            let _ = DeleteObject(HGDIOBJ(sub_font.0));

            SelectObject(hdc, HGDIOBJ(preedit_font.0));
            let preedit_w = text_width(hdc, &ctx.preedit);

            SelectObject(hdc, old);
            let _ = DeleteObject(HGDIOBJ(cand_font.0));
            let _ = DeleteObject(HGDIOBJ(preedit_font.0));
            ReleaseDC(Some(hwnd), hdc);
            (items, preedit_w)
        };

        let items_end = items
            .last()
            .map(|it| it.x + it.label_w + label_gap + it.text_w)
            .unwrap_or(padding);
        // 给模式徽标预留宽度，避免与 preedit 互相压占
        let mode_badge_hint = scale(BASE_FONT_HEIGHT, dpi) * 3 + scale(BASE_MODE_BADGE_GAP, dpi);
        let width = (items_end.max(padding + preedit_w + mode_badge_hint) + padding + sb_w)
            .max(scale(BASE_MIN_WIDTH, dpi));
        let height = scale(BASE_PREEDIT_HEIGHT, dpi) + scale(BASE_ROW_HEIGHT, dpi) + padding * 2;

        PAINT_DATA.with_borrow_mut(|data| {
            data.preedit = ctx.preedit.clone();
            data.items = items;
            data.dpi = dpi;
            data.skin = skin;
            data.is_ascii = ctx.is_ascii;
            data.is_full_shape = ctx.is_full_shape;
            data.page = page;
        });

        unsafe {
            let (mut x, mut y) = match anchor {
                Some(p) => (p.x, p.y + scale(4, dpi)),
                // 拿不到光标位置时放屏幕左下角兜底
                None => (60, GetSystemMetrics(SM_CYSCREEN) - height - 120),
            };
            // 防止超出屏幕右/下边缘
            x = x.min(GetSystemMetrics(SM_CXSCREEN) - width - 8).max(0);
            y = y.min(GetSystemMetrics(SM_CYSCREEN) - height - 8).max(0);

            let _ = MoveWindow(hwnd, x, y, width, height, true);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            // 阴影壳贴到主窗正下方（NOACTIVATE、命中穿透，不抢 IME 焦点）
            self.shadow.sync(hwnd, x, y, width, height, &skin.shadow);
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }

    pub fn hide(&mut self) {
        self.shadow.hide();
        if let Some(hwnd) = self.hwnd {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    /// 触发一次重绘：模式角标 / 长按大写提示等不发新候选、仅刷新外观的场景用。
    /// 窗口不存在（尚未 show 过）时静默 no-op —— 下帧 show 自然会带出最新状态。
    pub fn invalidate(&self) {
        if let Some(hwnd) = self.hwnd {
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
        }
    }

    pub fn destroy(&mut self) {
        self.shadow.destroy();
        if let Some(hwnd) = self.hwnd.take() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        // 释放 GPU target/brushes（HWND 已死，引用随之失效）
        crate::candidate_window_d2d::shutdown();
        crate::candidate_window_dcomp::shutdown();
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        value if value == WM_PAINT => {
            // 调度：瀑布链 DComp → D2D → GDI。当帧失败 (false) 立刻落下一级
            // 重画，画面不丢；三条路径共用同一份 thread-local PaintData 布局槽位，
            // 视觉 1:1。档位缓存在 thread-local BACKEND_KIND，不在窗口过程里
            // 反复探测硬件。
            let view = make_paint_view();
            let drawn = match backend_kind() {
                BackendKind::DComp => view
                    .as_ref()
                    .map(|v| crate::candidate_window_dcomp::paint(hwnd, &client_rect(hwnd), v))
                    .unwrap_or(false),
                BackendKind::D2D => view
                    .as_ref()
                    .map(|v| crate::candidate_window_d2d::paint(hwnd, &client_rect(hwnd), v))
                    .unwrap_or(false),
                BackendKind::Gdi => {
                    // GDI 路径：首轮 / Failed / TDR 未完成 / 任何内部错误
                    let mut ps = PAINTSTRUCT::default();
                    let hdc = BeginPaint(hwnd, &mut ps);
                    paint(hdc, &ps.rcPaint);
                    let _ = EndPaint(hwnd, &ps);
                    true
                }
            };
            if drawn && backend_kind() != BackendKind::Gdi {
                // GPU 路径已完成 BeginDraw/EndDraw；仍须 ValidateRect 清 dirty 区
                let _ = ValidateRect(Some(hwnd), None);
            } else if !drawn {
                // 当帧 GPU 故障：立刻落 GDI 重画一遍（与既有回退语义一致）
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                paint(hdc, &ps.rcPaint);
                let _ = EndPaint(hwnd, &ps);
                let _ = ValidateRect(Some(hwnd), None);
            }
            LRESULT(0)
        }
        value if value == WM_SIZE => {
            // 本帧起 swapchain/target 尺寸失配：标记失效，下一帧按
            // GetClientRect 重建（GDI 无所谓，自动按 BeginPaint 走）
            crate::candidate_window_d2d::notify_resize();
            crate::candidate_window_dcomp::notify_resize();
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        value if value == WM_DPICHANGED => {
            // DPI 失配：target 与字形都要重建/重测
            crate::candidate_window_d2d::notify_resize();
            crate::candidate_window_dcomp::notify_resize();
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        value if value == WM_DESTROY => {
            crate::candidate_window_d2d::shutdown();
            crate::candidate_window_dcomp::shutdown();
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        value if value == WM_LBUTTONDOWN => {
            select_candidate_at(lparam);
            LRESULT(0)
        }
        value if value == WM_MOUSEMOVE => {
            update_hover(lparam);
            LRESULT(0)
        }
        // WM_MOUSELEAVE = 0x02A3 = 675（windows crate 的 UI::Controls 特性未启用，用裸值）
        675 => {
            clear_hover();
            LRESULT(0)
        }
        value if value == WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as i16;
            send_virtual_key(if delta < 0 { 0x22 } else { 0x21 });
            LRESULT(0)
        }
        value if value == WM_SETTINGCHANGE => {
            // 主题热切换："ImmersiveColorSet" 到达 → 刷新皮肤缓存并重绘
            if skin::is_immersive_color_change(lparam) {
                let skin = Skin::refresh_on_setting_change();
                unsafe {
                    skin::apply_appearance(hwnd, &skin);
                    crate::candidate_window_d2d::notify_skin_changed();
                    crate::candidate_window_dcomp::notify_skin_changed();
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn client_rect(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut r);
    }
    r
}

/// 命中测试：把客户区坐标映射到当前候选序号。
/// 供 WM_MOUSEMOVE（悬停高亮）与 WM_LBUTTONDOWN（点击选词）共用，
/// 保证两处几何完全一致；None = 落在候选行之外。
fn hit_test_item(x: i32, y: i32) -> Option<usize> {
    let dpi = PAINT_DATA.with_borrow(|data| data.dpi);
    let row_top = scale(BASE_PADDING, dpi) + scale(BASE_PREEDIT_HEIGHT, dpi);
    let row_bottom = row_top + scale(BASE_ROW_HEIGHT, dpi);
    if y < row_top || y >= row_bottom {
        return None;
    }
    let label_gap = scale(BASE_LABEL_GAP, dpi);
    let item_padding = scale(BASE_HL_PAD, dpi);
    PAINT_DATA.with_borrow(|data| {
        for (index, item) in data.items.iter().enumerate() {
            let left = item.x - item_padding;
            let right = item.x + item.label_w + label_gap + item.text_w + item_padding;
            if x >= left && x <= right {
                return Some(index);
            }
        }
        None
    })
}

/// 将点击坐标映射到当前候选，并发送 Rime 已支持的数字选词键。
unsafe fn select_candidate_at(lparam: LPARAM) {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    let dpi = PAINT_DATA.with_borrow(|data| data.dpi);
    if let Some(index) = hit_test_item(x, y) {
        // 序号键 1..9 → 0x31..0x39；第 10 项按下标 0 → 0x30。
        let key = if index >= 9 { 0x30 } else { 0x31 + index as u8 };
        send_virtual_key(key);
        return;
    }
    // 落在任何 item 之外但命中滚动条轨道：纯视觉条不响应拖动，按系统习惯
    // 轨道点击 = 向上/下翻一页（PageUp/PageDown，与滚轮一致）。
    PAINT_DATA.with_borrow(|data| {
        if !data.items.is_empty() && data.skin.metrics.scrollbar && data.page.total_pages() > 1 {
            let track_w = scale(skin::SCROLLBAR_BASE_WIDTH, dpi);
            let win_right = PAINT_WIN_W.with(|w| w.get());
            if win_right > 0 && x >= win_right - track_w {
                let mid = scale(BASE_PADDING, dpi)
                    + scale(BASE_PREEDIT_HEIGHT, dpi)
                    + scale(BASE_ROW_HEIGHT, dpi) / 2;
                send_virtual_key(if y < mid { 0x21 } else { 0x22 });
            }
        }
    });
}

/// WM_MOUSEMOVE：更新悬停高亮。首次进入时注册 WM_MOUSELEAVE 跟踪，
/// 悬停项变化或首次悬停时 InvalidateRect 触发重绘。
fn update_hover(lparam: LPARAM) {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    let idx = hit_test_item(x, y);
    let changed = HOVER_INDEX.with(|h| {
        if h.get() == idx {
            false
        } else {
            h.set(idx);
            true
        }
    });
    if idx.is_some() && !HOVER_TRACKING.with(|t| t.get()) {
        unsafe {
            let mut tme = std::mem::zeroed::<TRACKMOUSEEVENT>();
            tme.cbSize = std::mem::size_of::<TRACKMOUSEEVENT>() as u32;
            tme.dwFlags = TME_LEAVE;
            tme.hwndTrack = PAINT_HWND.with(|h| h.get());
            let _ = TrackMouseEvent(&mut tme);
        }
        HOVER_TRACKING.with(|t| t.set(true));
    }
    if changed {
        let hwnd = PAINT_HWND.with(|h| h.get());
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }
}

/// WM_MOUSELEAVE：清除悬停并重绘。
fn clear_hover() {
    HOVER_TRACKING.with(|t| t.set(false));
    let changed = HOVER_INDEX.with(|h| {
        if h.get().is_none() {
            false
        } else {
            h.set(None);
            true
        }
    });
    if changed {
        let hwnd = PAINT_HWND.with(|h| h.get());
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }
}

// paint 时记录客户区宽度供命中测试使用（同 UI 线程，Cell 足够）
thread_local! {
    static PAINT_WIN_W: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

/// 无焦点候选窗将操作发送给前台编辑器，继续走 TSF 的正常按键路径。
unsafe fn send_virtual_key(vk: u8) {
    keybd_event(vk, 0, KEYBD_EVENT_FLAGS(0), 0);
    keybd_event(vk, 0, KEYEVENTF_KEYUP, 0);
}

unsafe fn paint(hdc: HDC, rc: &RECT) {
    PAINT_DATA.with_borrow(|data| {
        let dpi = data.dpi;
        let padding = scale(BASE_PADDING, dpi);
        let label_gap = scale(BASE_LABEL_GAP, dpi);
        let hl_pad = scale(BASE_HL_PAD, dpi);
        let preedit_h = scale(BASE_PREEDIT_HEIGHT, dpi);
        let row_h = scale(BASE_ROW_HEIGHT, dpi);
        let colors = data.skin.candidate;
        let font_scale = data.skin.metrics.font_scale;

        let bg = CreateSolidBrush(COLORREF(colors.background));
        FillRect(hdc, rc, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));
        SetBkMode(hdc, TRANSPARENT);
        PAINT_WIN_W.with(|w| w.set(rc.right));

        // 皮肤滚动条（GDI 路径）：右缘 4px 轨道 + 页位置 thumb；
        // 仅在开关开启且多页时绘制，纯视觉、不影响布局槽位。
        if data.skin.metrics.scrollbar && data.page.total_pages() > 1 {
            let track_w = scale(skin::SCROLLBAR_BASE_WIDTH, dpi);
            let item_w = data
                .items
                .iter()
                .map(|it| it.label_w + label_gap + it.text_w + hl_pad * 2)
                .max()
                .unwrap_or(scale(96, dpi));
            if let Some(geo) = skin::scrollbar_geo(
                rc.right,
                rc.bottom,
                item_w,
                padding,
                track_w,
                data.page.page_no,
                data.page.total_pages(),
            ) {
                let (track_c, thumb_c) = skin::scrollbar_colors(&data.skin);
                let (track_b, thumb_b) = (
                    CreateSolidBrush(COLORREF(track_c)),
                    CreateSolidBrush(COLORREF(thumb_c)),
                );
                let track_r = RECT {
                    left: geo.track[0],
                    top: geo.track[1],
                    right: geo.track[2],
                    bottom: geo.track[3],
                };
                let thumb_r = RECT {
                    left: geo.thumb[0],
                    top: geo.thumb[1],
                    right: geo.thumb[2],
                    bottom: geo.thumb[3],
                };
                FillRect(hdc, &track_r, track_b);
                FillRect(hdc, &thumb_r, thumb_b);
                let _ = DeleteObject(HGDIOBJ(track_b.0));
                let _ = DeleteObject(HGDIOBJ(thumb_b.0));
            }
        }

        // 预编辑串（小号灰字，第一行）
        let preedit_font = make_font(font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale));
        let old_font = SelectObject(hdc, HGDIOBJ(preedit_font.0));
        SetTextColor(hdc, COLORREF(colors.preedit));
        // 右上角模式角标。规则与 make_paint_view 一致（mode_badge_text），
        // caps_visual 优先；None 时不留角标宽，把空间全部交还 preedit。
        let mode_badge = mode_badge_text(data).unwrap_or("");
        let mode_badge_w = if mode_badge.is_empty() {
            0
        } else {
            text_width(hdc, mode_badge) + scale(BASE_MODE_BADGE_GAP, dpi)
        };
        let preedit_w = (rc.right - padding * 2 - mode_badge_w).max(scale(BASE_MIN_WIDTH, dpi));
        let breaks = syllable_breaks(&data.preedit);
        if breaks.is_empty() {
            draw_line(hdc, &data.preedit, padding, padding, preedit_w, preedit_h);
        } else {
            // 含音节分隔符：跳过分隔符本体、逐段交替色 + 1px 竖线占位。
            let preedit_font_h = font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale);
            draw_preedit_segmented(
                hdc,
                &data.preedit,
                &breaks,
                &colors,
                padding,
                preedit_h,
                preedit_font_h,
            );
        }
        // 角标本体：highlight 底色块 + 反色文字（skin.candidate.background 当反色，
        // 因为 highlight_background 与 background 的强对比是皮肤语义自带的）。
        if !mode_badge.is_empty() {
            let badge_left = padding + preedit_w;
            let badge_right = rc.right - padding;
            let badge_rect = RECT {
                left: badge_left,
                top: padding,
                right: badge_right,
                bottom: padding + preedit_h,
            };
            let hl_bg = CreateSolidBrush(COLORREF(colors.highlight_background));
            FillRect(hdc, &badge_rect, hl_bg);
            let _ = DeleteObject(HGDIOBJ(hl_bg.0));
            SetTextColor(hdc, COLORREF(colors.background));
            draw_line(hdc, mode_badge, badge_left, padding, badge_right, preedit_h);
        }

        // 候选行（第二行横排）
        let cand_font = make_font(font_height(BASE_FONT_HEIGHT, dpi, font_scale));
        SelectObject(hdc, HGDIOBJ(cand_font.0));
        let row_top = padding + preedit_h;
        for item in &data.items {
            let item_end = item.x + item.label_w + label_gap + item.text_w;
            if item.highlighted {
                let hl = CreateSolidBrush(COLORREF(colors.highlight_background));
                let hl_rect = RECT {
                    left: item.x - hl_pad,
                    top: row_top,
                    right: item_end + hl_pad,
                    bottom: row_top + row_h,
                };
                FillRect(hdc, &hl_rect, hl);
                let _ = DeleteObject(HGDIOBJ(hl.0));
            } else if item.hovered {
                // 悬停（非选中）：highlight 向 background 过渡 40% 的浅底，
                // 与选中态强对比区分，同时有明确的"可点"反馈。
                let hover_c = blend_colorref(colors.highlight_background, colors.background, 400);
                let hv = CreateSolidBrush(COLORREF(hover_c));
                let hv_rect = RECT {
                    left: item.x - hl_pad,
                    top: row_top,
                    right: item_end + hl_pad,
                    bottom: row_top + row_h,
                };
                FillRect(hdc, &hv_rect, hv);
                let _ = DeleteObject(HGDIOBJ(hv.0));
            }

            SetTextColor(hdc, COLORREF(colors.label));
            draw_line(
                hdc,
                &item.label,
                item.x,
                row_top,
                item.x + item.label_w,
                row_h,
            );

            SetTextColor(hdc, COLORREF(colors.text));
            draw_line(
                hdc,
                &item.text,
                item.x + item.label_w + label_gap,
                row_top,
                item_end,
                row_h,
            );

            // 副标：词库附注（emoji、近义词、词条类别等），灰字小一号，
            // 紧跟主文本右侧（与搜狗/Rime 候选副标一致），不另起一行，
            // 避免抬高候选行高。text_w 已含 comment 宽度（见 show()），
            // 这里用 cand_font 实测主文本宽度定位 comment 起点。
            if !item.comment.is_empty() {
                let pure_text_w = text_width(hdc, &item.text);
                let sub_font = make_font(font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale));
                SelectObject(hdc, HGDIOBJ(sub_font.0));
                SetTextColor(hdc, COLORREF(colors.label));
                draw_line(
                    hdc,
                    &item.comment,
                    item.x + item.label_w + label_gap + pure_text_w + scale(2, dpi),
                    row_top,
                    item_end,
                    row_h,
                );
                SelectObject(hdc, HGDIOBJ(cand_font.0));
                let _ = DeleteObject(HGDIOBJ(sub_font.0));
            }
        }

        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(cand_font.0));
        let _ = DeleteObject(HGDIOBJ(preedit_font.0));
    });
}

/// 混合 `fg` 朝 `bg` 方向 `permille` 千分比（COLORREF, 0x00BBGGRR）。
/// `permille=0` → fg 本色；`1000` → bg 本色。
/// 供音节分段派生色环：不改 skin.rs（任务约束），分隔竖线与段间交替色
/// 全部由既有 candidate 颜色派生。
pub fn blend_colorref(fg: u32, bg: u32, permille: u32) -> u32 {
    let pm = permille.min(1000);
    let ch = |shift: u32| {
        let f = (fg >> shift) & 0xff;
        let b = (bg >> shift) & 0xff;
        (f * (1000 - pm) + b * pm + 500) / 1000
    };
    ch(0) | (ch(8) << 8) | (ch(16) << 16)
}

/// 派生分段交替色环（闭循环，无皮肤侵入）：
/// - 偶数段 = preedit 本色；
/// - 奇数段 = preedit 向 text 过渡 28%（可读，不扎眼）；
/// - 分隔竖线 = preedit 本色，与偶数段合并为同色系。
///
/// 返回 [seg_even, seg_odd, separator_line]。
pub fn syllable_segment_colors(colors: &crate::skin::CandidateColors) -> [u32; 3] {
    [
        colors.preedit,
        blend_colorref(colors.preedit, colors.text, 280),
        colors.preedit,
    ]
}

/// 按 `syllable_breaks` 把 preedit 切成多段逐段绘制：断点字符本体跳过不入屏，
/// 槽位用来画 1px 竖线（水平居中于原字符宽度）；相邻段在交替色间轮换，
/// 形成 `A|B` 分段视觉。排版宽度与原文完全一致（各段 GetTextExtentPoint32W
/// 实测顺序推进），对布局槽位与窗口宽度零影响。
///
/// `hdc` 须已选入 preedit 字体；`padding`/`preedit_h`/`preedit_font_h` 由调用方
/// 按 (dpi, font_scale) 展开后传入。
unsafe fn draw_preedit_segmented(
    hdc: HDC,
    preedit: &str,
    breaks: &[u16],
    colors: &crate::skin::CandidateColors,
    padding: i32,
    preedit_h: i32,
    preedit_font_h: i32,
) {
    let wide: Vec<u16> = preedit.encode_utf16().collect();
    let n = wide.len();
    let [even_c, odd_c, sep_c] = syllable_segment_colors(colors);
    // 竖线高度 = 字形高度的 1/2 居中；上下各留 25% 空白让分隔与文字区分离。
    let line_half = (preedit_font_h / 2).max(4);
    let center_y = padding + preedit_h / 2;
    let (ly0, ly1) = (center_y - line_half, center_y + line_half);

    let mut x = padding;
    let mut seg_start = 0usize;
    for (idx, &bp) in breaks.iter().enumerate() {
        let bp = (bp as usize).min(n);
        if bp < seg_start || bp >= n {
            continue;
        }
        // 段文本：颜色按奇偶轮换（首段从 even 起）。
        if bp > seg_start {
            let seg = &wide[seg_start..bp];
            let w = utf16_width(hdc, seg);
            let seg_c = if idx % 2 == 0 { even_c } else { odd_c };
            SetTextColor(hdc, COLORREF(seg_c));
            draw_line_utf16(hdc, seg, x, padding, x + w, preedit_h);
            x += w;
        }
        // 分隔符槽位：实测原字符宽度保持基线对齐；槽位中央画 1px 竖线。
        let sep_w = utf16_width(hdc, &wide[bp..bp + 1]);
        if sep_w > 0 {
            let pen: HPEN = CreatePen(PS_SOLID, 1, COLORREF(sep_c));
            let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
            let cx = x + sep_w / 2;
            let _ = MoveToEx(hdc, cx, ly0, None);
            let _ = LineTo(hdc, cx, ly1);
            SelectObject(hdc, old_pen);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
        x += sep_w;
        seg_start = bp + 1;
    }
    // 尾段
    if seg_start < n {
        let seg = &wide[seg_start..];
        let w = utf16_width(hdc, seg);
        let seg_c = if breaks.len().is_multiple_of(2) {
            even_c
        } else {
            odd_c
        };
        SetTextColor(hdc, COLORREF(seg_c));
        draw_line_utf16(hdc, seg, x, padding, x + w, preedit_h);
    }
}

/// GetTextExtentPoint32W 直接吃 UTF-16 切片（避免 String 重编码分配）。
unsafe fn utf16_width(hdc: HDC, s: &[u16]) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, s, &mut size);
    size.cx
}

unsafe fn draw_line_utf16(hdc: HDC, wide: &[u16], left: i32, top: i32, right: i32, height: i32) {
    if wide.is_empty() {
        return;
    }
    let mut buf: Vec<u16> = wide.to_vec();
    let mut rect = RECT {
        left,
        top,
        right,
        bottom: top + height,
    };
    DrawTextW(
        hdc,
        &mut buf,
        &mut rect,
        DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_VCENTER,
    );
}

unsafe fn draw_line(hdc: HDC, text: &str, left: i32, top: i32, right: i32, height: i32) {
    if text.is_empty() {
        return;
    }
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT {
        left,
        top,
        right,
        bottom: top + height,
    };
    DrawTextW(
        hdc,
        &mut utf16,
        &mut rect,
        DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_VCENTER,
    );
}

#[cfg(test)]
mod tests {
    use super::{
        blend_colorref, mode_badge_text, syllable_breaks, syllable_segment_colors, BackendKind,
        PaintData,
    };

    /// 选路器可被显式改写并被 backend_kind() 速读出来。
    /// 用例固定 Gdi（最安全档），只验证 override 通道在。
    #[test]
    fn backend_kind_override_round_trip() {
        super::set_backend_kind_for_test(BackendKind::Gdi);
        assert_eq!(super::backend_kind(), BackendKind::Gdi);
    }

    fn paint_data(caps: bool, ascii: bool) -> PaintData {
        PaintData {
            preedit: String::new(),
            items: Vec::new(),
            dpi: 96,
            skin: super::Skin::default(),
            is_ascii: ascii,
            is_full_shape: false,
            caps_visual: caps,
            page: super::PageInfo::default(),
        }
    }

    /// mode_badge 文案：caps_visual 优先于 ascii 模式；常规态按 ascii 给 "En"/"中"。
    /// 这是候选窗右上角模式角标的唯一规则书，GDI/D2D/DComp 三路共享。
    #[test]
    fn mode_badge_text_prefers_caps_visual_over_ascii() {
        assert_eq!(mode_badge_text(&paint_data(true, false)), Some("⇪大写"));
        assert_eq!(mode_badge_text(&paint_data(true, true)), Some("⇪大写"));
        assert_eq!(mode_badge_text(&paint_data(false, false)), Some("中"));
        assert_eq!(mode_badge_text(&paint_data(false, true)), Some("En"));
    }

    /// 长按置位 → 短按清位的"两面旗"协议：set 幂等；caps_visual_active 是真值源。
    #[test]
    fn set_caps_visual_idempotent_and_query_active() {
        // 起点：两次 false。清除不清也该 false 返回。
        let _ = super::set_caps_visual(false);
        assert!(!super::caps_visual_active());
        // 置位首次返回变化 true；再次置同值 false。
        assert!(super::set_caps_visual(true));
        assert!(super::caps_visual_active());
        assert!(!super::set_caps_visual(true));
        assert!(super::caps_visual_active());
        // 清位也满足幂等。
        assert!(super::set_caps_visual(false));
        assert!(!super::caps_visual_active());
        assert!(!super::set_caps_visual(false));
    }

    /// 音节分隔符只识别空格与单引号；返回 UTF-16 码元索引。
    /// 例：`"ni hao shi"` → 空格在 UTF-16 下标 2、7，得到 `[2, 7]`。
    #[test]
    fn syllable_breaks_finds_space_and_apostrophe() {
        // 空 preedit 不该有断点
        assert!(syllable_breaks("").is_empty());
        // 整个串没有分隔符 → 空 vec
        assert!(syllable_breaks("nihao").is_empty());
        // 单个空格在 UTF-16 下标 2
        assert_eq!(syllable_breaks("ni hao"), vec![2]);
        // 单引号同样识别
        assert_eq!(syllable_breaks("ni'hao"), vec![2]);
        // 多个分隔符：空格 + 单引号 都能发现，索引按出现顺序
        assert_eq!(syllable_breaks("wo de'shi jie"), vec![2, 5, 9]);
        // 首字符即是分隔符 / 末字符即是分隔符
        assert_eq!(syllable_breaks(" ni"), vec![0]);
        assert_eq!(syllable_breaks("ni "), vec![2]);
        // 连续两个分隔符各自都需记录（绘制阶段跳过 bp < seg_start 的，但数据不能丢）
        assert_eq!(syllable_breaks("ni  hao"), vec![2, 3]);
    }

    /// syllable 分段色环：偶数段保持 preedit 本色，奇数段 = preedit → text 28% 过渡；
    /// 分隔竖线与偶数段同色系（preedit）。goldens 用具体 channel 锁差异。
    #[test]
    fn syllable_segment_colors_derive_without_skin_change() {
        let colors = crate::skin::CandidateColors::light();
        let [even, odd, sep] = syllable_segment_colors(&colors);
        // even 与 separator 严格等于 preedit 本色
        assert_eq!(even, colors.preedit);
        assert_eq!(sep, colors.preedit);
        // odd 介于 preedit 与 text 之间（各 channel 都向 text 偏移 28%）
        let pre_r = colors.preedit & 0xff;
        let text_r = colors.text & 0xff;
        let expected_r = (pre_r * 720 + text_r * 280 + 500) / 1000;
        assert_eq!(odd & 0xff, expected_r);
        // blend_colorref 边界：0% → fg，100% → bg，且 permille 上限 1000
        assert_eq!(blend_colorref(0x112233, 0xaabbcc, 0), 0x112233);
        assert_eq!(blend_colorref(0x112233, 0xaabbcc, 1000), 0xaabbcc);
        assert_eq!(blend_colorref(0x112233, 0xaabbcc, 5000), 0xaabbcc);
    }
}
