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
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, GetDC,
    GetTextExtentPoint32W, InvalidateRect, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    ValidateRect, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HBRUSH, HDC, HFONT,
    HGDIOBJ, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, LoadCursorW, MoveWindow,
    RegisterClassW, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, IDC_ARROW,
    SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE,
    WM_DESTROY, WM_DPICHANGED, WM_LBUTTONDOWN, WM_MOUSEWHEEL, WM_PAINT, WM_SETTINGCHANGE, WM_SIZE,
    WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use ime_ipc::Context;

use crate::skin::{self, ShadowShell, Skin};

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
    pub mode_badge: String,
    /// 候选主字体 em-height（px）
    pub cand_font_h: i32,
    /// preedit/副标 小字体 em-height（px）
    pub sub_font_h: i32,
    /// 当前页分页快照（滚动条用）
    pub page: PageInfo,
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
        Some(PaintView {
            preedit: data.preedit.clone(),
            items: data
                .items
                .iter()
                .map(|it| ItemView {
                    label: it.label.clone(),
                    text: it.text.clone(),
                    comment: it.comment.clone(),
                    x: it.x,
                    label_w: it.label_w,
                    text_w: it.text_w,
                    pure_text_w: it.pure_text_w,
                    highlighted: it.highlighted,
                })
                .collect(),
            dpi,
            skin: data.skin,
            padding: scale(BASE_PADDING, dpi),
            preedit_h: scale(BASE_PREEDIT_HEIGHT, dpi),
            row_h: scale(BASE_ROW_HEIGHT, dpi),
            label_gap: scale(BASE_LABEL_GAP, dpi),
            hl_pad: scale(BASE_HL_PAD, dpi),
            mode_badge: format!(
                "{}{}",
                if data.is_ascii { "英" } else { "中" },
                if data.is_full_shape { "·全" } else { "" },
            ),
            cand_font_h: font_height(BASE_FONT_HEIGHT, dpi, font_scale),
            sub_font_h: font_height(BASE_PREEDIT_FONT_HEIGHT, dpi, font_scale),
            page: data.page,
        })
    })
}

/// 徽标宽度（与 GDI paint 内 text_width + BASE_MODE_BADGE_GAP 同公式；
/// GDI 实际测宽结果 ∈ [3*cand_h, 3.5*cand_h]，用近似值差 1px 视觉无感）。
pub fn mode_badge_width(view: &PaintView) -> i32 {
    view.cand_font_h * 3 + scale(BASE_MODE_BADGE_GAP, view.dpi)
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
        page: PageInfo::default(),
    });
    static CLASS_REGISTERED: RefCell<bool> = const { RefCell::new(false) };
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
        // 预热 D2D 工厂（< 5ms）；失败则本进程整段会话走 GDI 路径
        crate::candidate_window_d2d::try_init();
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

    pub fn destroy(&mut self) {
        self.shadow.destroy();
        if let Some(hwnd) = self.hwnd.take() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        // 释放 D2D target/brushes（HWND 已死，引用随之失效）
        crate::candidate_window_d2d::shutdown();
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
            // 调度：D2D 就绪则走 GPU 帧；失败/未就绪当帧落 GDI。
            // 两条路径共用同一份 thread-local PaintData 布局槽位，视觉 1:1。
            let try_d2d = make_paint_view()
                .map(|view| crate::candidate_window_d2d::paint(
                    hwnd,
                    &client_rect(hwnd),
                    &view,
                ))
                .unwrap_or(false);
            if !try_d2d {
                // GDI 路径（首轮 / Failed / TDR 未完成 / 任何内部错误）
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                paint(hdc, &ps.rcPaint);
                let _ = EndPaint(hwnd, &ps);
            } else {
                // D2D 已完成 BeginDraw/EndDraw；仍须 ValidateRect 清 dirty 区
                let _ = ValidateRect(Some(hwnd), None);
            }
            LRESULT(0)
        }
        value if value == WM_SIZE => {
            // 本帧起 target 尺寸失配：标记失效，下一帧按 GetClientRect 重建
            crate::candidate_window_d2d::notify_resize();
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        value if value == WM_DPICHANGED => {
            crate::candidate_window_d2d::notify_resize(); // DPI 失配在 paint 内 SetDpi
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        value if value == WM_DESTROY => {
            crate::candidate_window_d2d::shutdown();
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        value if value == WM_LBUTTONDOWN => {
            select_candidate_at(lparam);
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

/// 将点击坐标映射到当前候选，并发送 Rime 已支持的数字选词键。
unsafe fn select_candidate_at(lparam: LPARAM) {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    let dpi = PAINT_DATA.with_borrow(|data| data.dpi);
    let row_top = scale(BASE_PADDING, dpi) + scale(BASE_PREEDIT_HEIGHT, dpi);
    let row_bottom = row_top + scale(BASE_ROW_HEIGHT, dpi);
    if y < row_top || y >= row_bottom {
        return;
    }

    let label_gap = scale(BASE_LABEL_GAP, dpi);
    let item_padding = scale(BASE_HL_PAD, dpi);
    PAINT_DATA.with_borrow(|data| {
        for (index, item) in data.items.iter().enumerate() {
            let left = item.x - item_padding;
            let right = item.x + item.label_w + label_gap + item.text_w + item_padding;
            if x >= left && x <= right {
                // 序号键 1..9 → 0x31..0x39；第 10 项按下标 0 → 0x30。
                let key = if index >= 9 { 0x30 } else { 0x31 + index as u8 };
                send_virtual_key(key);
                return;
            }
        }
        // 落在任何 item 之外但命中滚动条轨道：纯视觉条不响应拖动，按系统习惯
        // 轨道点击 = 向上/下翻一页（PageUp/PageDown，与滚轮一致）。
        if !data.items.is_empty() && data.skin.metrics.scrollbar && data.page.total_pages() > 1 {
            let track_w = scale(skin::SCROLLBAR_BASE_WIDTH, dpi);
            let win_right = PAINT_WIN_W.with(|w| w.get());
            if win_right > 0 && x >= win_right - track_w {
                let mid = scale(BASE_PADDING, dpi) + scale(BASE_PREEDIT_HEIGHT, dpi)
                    + scale(BASE_ROW_HEIGHT, dpi) / 2;
                send_virtual_key(if y < mid { 0x21 } else { 0x22 });
            }
        }
    });
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
                let (track_b, thumb_b) =
                    (CreateSolidBrush(COLORREF(track_c)), CreateSolidBrush(COLORREF(thumb_c)));
                let track_r = RECT { left: geo.track[0], top: geo.track[1], right: geo.track[2], bottom: geo.track[3] };
                let thumb_r = RECT { left: geo.thumb[0], top: geo.thumb[1], right: geo.thumb[2], bottom: geo.thumb[3] };
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
        // 模式徽标（中/英 + 全/半角），与搜狗主流视觉一致；仅占 preedit 行尾部少量宽度。
        let mode_badge = format!(
            "{}{}",
            if data.is_ascii { "英" } else { "中" },
            if data.is_full_shape { "·全" } else { "" },
        );
        let mode_badge_w = text_width(hdc, &mode_badge) + scale(BASE_MODE_BADGE_GAP, dpi);
        let preedit_w = (rc.right - padding * 2 - mode_badge_w).max(scale(BASE_MIN_WIDTH, dpi));
        draw_line(
            hdc,
            &data.preedit,
            padding,
            padding,
            preedit_w,
            preedit_h,
        );
        SetTextColor(hdc, COLORREF(colors.label));
        draw_line(
            hdc,
            &mode_badge,
            padding + preedit_w,
            padding,
            rc.right - padding,
            preedit_h,
        );

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
