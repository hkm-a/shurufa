//! 候选窗：GDI 绘制的置顶弹窗，显示预编辑串与编号候选。
//!
//! 窗口在宿主应用的 UI 线程内创建（TSF 单元线程模型），
//! 不抢焦点（WS_EX_NOACTIVATE），随组合文本位置移动。

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    SelectObject, SetBkMode, SetTextColor, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, FW_NORMAL,
    HBRUSH, HDC, HFONT, HGDIOBJ, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, MoveWindow,
    RegisterClassW, ShowWindow, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, SM_CXSCREEN, SM_CYSCREEN,
    SW_HIDE, SW_SHOWNOACTIVATE, SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

use ime_bridge::Context;

const CLASS_NAME: PCWSTR = w!("ShurufaCandidateWindow");
const LINE_HEIGHT: i32 = 26;
const PADDING: i32 = 8;
const WIDTH: i32 = 220;
const FONT_HEIGHT: i32 = 18;

// 绘制数据挂在线程本地：窗口与 TSF 回调同属宿主 UI 线程
thread_local! {
    static PAINT_DATA: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static CLASS_REGISTERED: RefCell<bool> = const { RefCell::new(false) };
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
                WIDTH,
                LINE_HEIGHT + PADDING * 2,
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
        let mut lines = vec![ctx.preedit.clone()];
        for (i, c) in ctx.candidates.iter().enumerate().take(5) {
            let marker = if i == ctx.highlighted { "▸" } else { " " };
            lines.push(format!("{}{}. {}", marker, i + 1, c.text));
        }
        let height = lines.len() as i32 * LINE_HEIGHT + PADDING * 2;
        PAINT_DATA.with_borrow_mut(|data| *data = lines);

        unsafe {
            let (mut x, mut y) = match anchor {
                Some(p) => (p.x, p.y + 4),
                // 拿不到光标位置时放屏幕左下角兜底
                None => (60, GetSystemMetrics(SM_CYSCREEN) - height - 80),
            };
            // 防止超出屏幕右/下边缘
            x = x.min(GetSystemMetrics(SM_CXSCREEN) - WIDTH - 8).max(0);
            y = y.min(GetSystemMetrics(SM_CYSCREEN) - height - 8).max(0);

            let _ = MoveWindow(hwnd, x, y, WIDTH, height, true);
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
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, true);
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

unsafe fn paint(hdc: HDC, rc: &RECT) {
    // 背景：白底
    let bg = CreateSolidBrush(COLORREF(0x00FF_FFFF));
    FillRect(hdc, rc, bg);
    let _ = DeleteObject(HGDIOBJ(bg.0));

    let font: HFONT = CreateFontW(
        FONT_HEIGHT,
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
    );
    let old_font = SelectObject(hdc, HGDIOBJ(font.0));
    SetBkMode(hdc, TRANSPARENT);

    PAINT_DATA.with_borrow(|lines| {
        for (i, line) in lines.iter().enumerate() {
            // 首行为拼音预编辑串，用灰色；候选行用黑色
            SetTextColor(
                hdc,
                COLORREF(if i == 0 { 0x0088_8888 } else { 0x0000_0000 }),
            );
            let mut utf16: Vec<u16> = line.encode_utf16().collect();
            let mut rect = RECT {
                left: PADDING,
                top: PADDING + i as i32 * LINE_HEIGHT,
                right: WIDTH - PADDING,
                bottom: PADDING + (i as i32 + 1) * LINE_HEIGHT,
            };
            DrawTextW(
                hdc,
                &mut utf16,
                &mut rect,
                DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
    });

    SelectObject(hdc, old_font);
    let _ = DeleteObject(HGDIOBJ(font.0));
}
