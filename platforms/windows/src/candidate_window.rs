//! 候选窗：GDI 绘制的置顶弹窗，横向排列编号候选（主流输入法布局）。
//!
//! 窗口在宿主应用的 UI 线程内创建（TSF 单元线程模型），
//! 不抢焦点（WS_EX_NOACTIVATE），随组合文本位置移动，
//! 宽度按候选文本实测宽度自适应，所有尺寸按窗口 DPI 缩放。

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, GetDC,
    GetTextExtentPoint32W, InvalidateRect, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HBRUSH, HDC, HFONT, HGDIOBJ,
    PAINTSTRUCT, TRANSPARENT,
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
    WM_LBUTTONDOWN, WM_MOUSEWHEEL, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

use ime_ipc::Context;

use crate::skin::{load_candidate_colors, CandidateColors};

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
    x: i32,
    label_w: i32,
    text_w: i32,
    highlighted: bool,
}

struct PaintData {
    preedit: String,
    items: Vec<Item>,
    dpi: u32,
    colors: CandidateColors,
    is_ascii: bool,
    is_full_shape: bool,
}

// 绘制数据挂在线程本地：窗口与 TSF 回调同属宿主 UI 线程
thread_local! {
    static PAINT_DATA: RefCell<PaintData> = RefCell::new(PaintData {
        preedit: String::new(),
        items: Vec::new(),
        dpi: 96,
        colors: CandidateColors::default(),
        is_ascii: false,
        is_full_shape: false,
    });
    static CLASS_REGISTERED: RefCell<bool> = const { RefCell::new(false) };
}

fn scale(base: i32, dpi: u32) -> i32 {
    (base * dpi as i32 + 48) / 96
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
    colors: CandidateColors,
}

impl CandidateUi {
    pub fn new() -> Self {
        CandidateUi {
            hwnd: None,
            colors: load_candidate_colors(),
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
            self.hwnd = Some(hwnd);
            Some(hwnd)
        }
    }

    /// 用引擎上下文刷新窗口内容并显示在锚点下方。
    pub fn show(&mut self, ctx: &Context, anchor: Option<POINT>) {
        let Some(hwnd) = self.ensure_window() else {
            return;
        };
        // 部分宿主（DPI 虚拟化）对弹窗返回 96 兜底值，取系统 DPI 兜底
        let dpi = unsafe { GetDpiForWindow(hwnd).max(GetDpiForSystem()) }.max(96);
        let padding = scale(BASE_PADDING, dpi);
        let item_gap = scale(BASE_ITEM_GAP, dpi);
        let label_gap = scale(BASE_LABEL_GAP, dpi);

        // 用与绘制一致的字体实测文本宽度，横向布槽
        let (items, preedit_w) = unsafe {
            let hdc = GetDC(Some(hwnd));
            let cand_font = make_font(scale(BASE_FONT_HEIGHT, dpi));
            let preedit_font = make_font(scale(BASE_PREEDIT_FONT_HEIGHT, dpi));

            let old = SelectObject(hdc, HGDIOBJ(cand_font.0));
            let mut x = padding;
            let items: Vec<Item> = ctx
                .candidates
                .iter()
                .enumerate()
                .take(9)
                .map(|(i, c)| {
                    let label = format!("{}.", i + 1);
                    let label_w = text_width(hdc, &label);
                    let text_w = text_width(hdc, &c.text);
                    let item = Item {
                        label,
                        text: c.text.clone(),
                        x,
                        label_w,
                        text_w,
                        highlighted: i == ctx.highlighted,
                    };
                    x += label_w + label_gap + text_w + item_gap;
                    item
                })
                .collect();

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
        let width = (items_end.max(padding + preedit_w + mode_badge_hint) + padding)
            .max(scale(BASE_MIN_WIDTH, dpi));
        let height = scale(BASE_PREEDIT_HEIGHT, dpi) + scale(BASE_ROW_HEIGHT, dpi) + padding * 2;

        PAINT_DATA.with_borrow_mut(|data| {
            data.preedit = ctx.preedit.clone();
            data.items = items;
            data.dpi = dpi;
            data.colors = self.colors;
            data.is_ascii = ctx.is_ascii;
            data.is_full_shape = ctx.is_full_shape;
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
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }

    pub fn hide(&mut self) {
        if let Some(hwnd) = self.hwnd {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    pub fn destroy(&mut self) {
        if let Some(hwnd) = self.hwnd.take() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
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
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint(hdc, &ps.rcPaint);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
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
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
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
                break;
            }
        }
    });
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
        let colors = data.colors;

        let bg = CreateSolidBrush(COLORREF(colors.background));
        FillRect(hdc, rc, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));
        SetBkMode(hdc, TRANSPARENT);

        // 预编辑串（小号灰字，第一行）
        let preedit_font = make_font(scale(BASE_PREEDIT_FONT_HEIGHT, dpi));
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
        let cand_font = make_font(scale(BASE_FONT_HEIGHT, dpi));
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
