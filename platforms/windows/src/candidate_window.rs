//! 候选窗：GDI 绘制的置顶弹窗，显示预编辑串与编号候选。
//!
//! 窗口在宿主应用的 UI 线程内创建（TSF 单元线程模型），
//! 不抢焦点（WS_EX_NOACTIVATE），随组合文本位置移动，
//! 所有尺寸按窗口 DPI 缩放。

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    FW_NORMAL, HBRUSH, HDC, HFONT, HGDIOBJ, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, MoveWindow, RegisterClassW,
    SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, SM_CXSCREEN, SM_CYSCREEN,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, WM_PAINT, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use ime_bridge::Context;

const CLASS_NAME: PCWSTR = w!("ShurufaCandidateWindow");

// 96 DPI 下的基准尺寸，运行期按窗口实际 DPI 缩放
const BASE_LINE_HEIGHT: i32 = 36;
const BASE_PADDING: i32 = 10;
const BASE_WIDTH: i32 = 300;
const BASE_FONT_HEIGHT: i32 = 22;
const BASE_PREEDIT_FONT_HEIGHT: i32 = 17;

// 配色（COLORREF 为 0x00BBGGRR）
const COLOR_BG: u32 = 0x00FA_FAFA;
const COLOR_HIGHLIGHT_BG: u32 = 0x00F5_E6D8; // 选中行淡蓝底
const COLOR_TEXT: u32 = 0x0020_2020;
const COLOR_PREEDIT: u32 = 0x0088_8888;
const COLOR_LABEL: u32 = 0x00B0_6030; // 序号用重点色

struct PaintData {
    preedit: String,
    /// (序号起始文本, 候选文本, 是否选中)
    rows: Vec<(String, String, bool)>,
    dpi: u32,
}

// 绘制数据挂在线程本地：窗口与 TSF 回调同属宿主 UI 线程
thread_local! {
    static PAINT_DATA: RefCell<PaintData> = RefCell::new(PaintData {
        preedit: String::new(),
        rows: Vec::new(),
        dpi: 96,
    });
    static CLASS_REGISTERED: RefCell<bool> = const { RefCell::new(false) };
}

fn scale(base: i32, dpi: u32) -> i32 {
    (base * dpi as i32 + 48) / 96
}

pub struct CandidateUi {
    hwnd: Option<HWND>,
}

impl CandidateUi {
    pub fn new() -> Self {
        CandidateUi { hwnd: None }
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
                BASE_WIDTH,
                BASE_LINE_HEIGHT,
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
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let rows: Vec<(String, String, bool)> = ctx
            .candidates
            .iter()
            .enumerate()
            .take(9)
            .map(|(i, c)| (format!("{}", i + 1), c.text.clone(), i == ctx.highlighted))
            .collect();
        let row_count = rows.len() as i32;
        PAINT_DATA.with_borrow_mut(|data| {
            data.preedit = ctx.preedit.clone();
            data.rows = rows;
            data.dpi = dpi;
        });

        let line_h = scale(BASE_LINE_HEIGHT, dpi);
        let preedit_h = scale(BASE_LINE_HEIGHT * 3 / 4, dpi);
        let padding = scale(BASE_PADDING, dpi);
        let width = scale(BASE_WIDTH, dpi);
        let height = preedit_h + row_count * line_h + padding * 2;

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
    if msg == WM_PAINT {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        paint(hdc, &ps.rcPaint);
        let _ = EndPaint(hwnd, &ps);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn make_font(height: i32, weight: i32) -> HFONT {
    CreateFontW(
        -height, // 负值表示字符高度（em），排版更稳定
        0,
        0,
        0,
        weight,
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

unsafe fn paint(hdc: HDC, rc: &RECT) {
    PAINT_DATA.with_borrow(|data| {
        let dpi = data.dpi;
        let line_h = scale(BASE_LINE_HEIGHT, dpi);
        let preedit_h = scale(BASE_LINE_HEIGHT * 3 / 4, dpi);
        let padding = scale(BASE_PADDING, dpi);
        let width = scale(BASE_WIDTH, dpi);

        let bg = CreateSolidBrush(COLORREF(COLOR_BG));
        FillRect(hdc, rc, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));
        SetBkMode(hdc, TRANSPARENT);

        // 预编辑串（小号灰字）
        let preedit_font = make_font(scale(BASE_PREEDIT_FONT_HEIGHT, dpi), FW_NORMAL.0 as i32);
        let old_font = SelectObject(hdc, HGDIOBJ(preedit_font.0));
        SetTextColor(hdc, COLORREF(COLOR_PREEDIT));
        let mut utf16: Vec<u16> = data.preedit.encode_utf16().collect();
        let mut rect = RECT {
            left: padding,
            top: padding,
            right: width - padding,
            bottom: padding + preedit_h,
        };
        DrawTextW(hdc, &mut utf16, &mut rect, DT_LEFT | DT_SINGLELINE | DT_NOPREFIX);

        // 候选行（大号字，选中行整行高亮）
        let cand_font = make_font(scale(BASE_FONT_HEIGHT, dpi), FW_NORMAL.0 as i32);
        SelectObject(hdc, HGDIOBJ(cand_font.0));
        for (i, (label, text, highlighted)) in data.rows.iter().enumerate() {
            let top = padding + preedit_h + i as i32 * line_h;
            if *highlighted {
                let hl = CreateSolidBrush(COLORREF(COLOR_HIGHLIGHT_BG));
                let row_rect = RECT {
                    left: scale(4, dpi),
                    top,
                    right: width - scale(4, dpi),
                    bottom: top + line_h,
                };
                FillRect(hdc, &row_rect, hl);
                let _ = DeleteObject(HGDIOBJ(hl.0));
            }

            // 序号
            SetTextColor(hdc, COLORREF(COLOR_LABEL));
            let mut label_utf16: Vec<u16> = format!("{label}.").encode_utf16().collect();
            let mut label_rect = RECT {
                left: padding,
                top,
                right: padding + scale(26, dpi),
                bottom: top + line_h,
            };
            DrawTextW(hdc, &mut label_utf16, &mut label_rect, DT_LEFT | DT_SINGLELINE | DT_NOPREFIX);

            // 候选文本
            SetTextColor(hdc, COLORREF(COLOR_TEXT));
            let mut text_utf16: Vec<u16> = text.encode_utf16().collect();
            let mut text_rect = RECT {
                left: padding + scale(32, dpi),
                top,
                right: width - padding,
                bottom: top + line_h,
            };
            DrawTextW(hdc, &mut text_utf16, &mut text_rect, DT_LEFT | DT_SINGLELINE | DT_NOPREFIX);
        }

        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(cand_font.0));
        let _ = DeleteObject(HGDIOBJ(preedit_font.0));
    });
}
