//! 剪贴板历史面板：全局热键呼出的置顶弹窗。
//!
//! 交互：Ctrl+Shift+V（冲突时回落 Alt+V）呼出 → 直接键入筛选、↑/↓ 或数字键
//! 选择 → 回车写回剪贴板并向原前台窗口模拟 Ctrl+V 完成粘贴，Esc 或失焦关闭。
//! 面板与监听器同属一条 UI 线程，状态挂 thread_local。

use std::cell::RefCell;

use clipboard_store::{ClipEntry, ClipKind};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, FW_BOLD, FW_NORMAL, HBRUSH, HDC, HFONT, HGDIOBJ, PAINTSTRUCT,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_SHIFT, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DOWN,
    VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo,
    GetSystemMetrics, GetWindowThreadProcessId, LoadCursorW, MoveWindow, RegisterClassW,
    SetForegroundWindow, ShowWindow, CS_HREDRAW, CS_VREDRAW, GUITHREADINFO, IDC_ARROW, SM_CXSCREEN,
    SM_CYSCREEN, SW_HIDE, SW_SHOWNA, WM_CHAR, WM_KEYDOWN, WM_KILLFOCUS, WM_PAINT, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

/// 面板一次展示的最大条目数（与数字键 1-9 对应）
const MAX_ROWS: usize = 9;
pub const HOTKEY_ID: i32 = 1;

// 96 DPI 基准尺寸
const BASE_WIDTH: i32 = 460;
const BASE_ROW_HEIGHT: i32 = 34;
const BASE_FOOTER_HEIGHT: i32 = 24;
const BASE_PADDING: i32 = 8;
const BASE_FONT: i32 = 16;
const BASE_SMALL_FONT: i32 = 13;

const COLOR_BG: u32 = 0x00FA_FAFA;
const COLOR_ROW_HL: u32 = 0x00F5_E6D8;
const COLOR_TEXT: u32 = 0x0020_2020;
const COLOR_DIM: u32 = 0x0090_9090;
const COLOR_LABEL: u32 = 0x00B0_6030;

struct PanelState {
    hwnd: HWND,
    entries: Vec<ClipEntry>,
    query: String,
    selected: usize,
    /// 呼出面板时的前台窗口，粘贴目标
    target: HWND,
    dpi: u32,
}

thread_local! {
    static PANEL: RefCell<Option<PanelState>> = const { RefCell::new(None) };
}

/// 注册全局热键；首选 Ctrl+Shift+V（对齐微信输入法习惯），
/// 被占用时回落 Alt+V。返回实际生效的描述。
/// 线程级注册（hwnd=None）：WM_HOTKEY 直接进线程队列，由消息循环
/// 截获，不依赖窗口消息投递路径。
pub fn register_hotkey() -> &'static str {
    unsafe {
        let which = if RegisterHotKey(None, HOTKEY_ID, MOD_CONTROL | MOD_SHIFT, 0x56).is_ok() {
            "Ctrl+Shift+V"
        } else if RegisterHotKey(None, HOTKEY_ID, MOD_ALT, 0x56).is_ok() {
            "Alt+V"
        } else {
            "（热键注册失败，面板不可用）"
        };
        crate::log_line(&format!("热键注册结果：{which}"));
        which
    }
}

/// 热键触发：记录当前前台窗口并弹出面板。
pub fn show(entries: Vec<ClipEntry>) {
    let target = unsafe { GetForegroundWindow() };
    let Some(hwnd) = ensure_window() else {
        crate::log_line("面板窗口创建失败");
        return;
    };
    crate::log_line(&format!("面板弹出，条目数 {}", entries.len()));
    let dpi = unsafe { GetDpiForWindow(hwnd).max(GetDpiForSystem()) }.max(96);

    let row_count = entries.len().max(1) as i32;
    let width = scale(BASE_WIDTH, dpi);
    let height = scale(BASE_PADDING, dpi) * 2
        + row_count * scale(BASE_ROW_HEIGHT, dpi)
        + scale(BASE_FOOTER_HEIGHT, dpi);

    let anchor = caret_or_cursor_pos(target);
    let (mut x, mut y) = (anchor.x, anchor.y + scale(6, dpi));
    unsafe {
        x = x.min(GetSystemMetrics(SM_CXSCREEN) - width - 8).max(0);
        y = y.min(GetSystemMetrics(SM_CYSCREEN) - height - 8).max(0);
    }

    PANEL.with_borrow_mut(|slot| {
        *slot = Some(PanelState {
            hwnd,
            entries,
            query: String::new(),
            selected: 0,
            target,
            dpi,
        });
    });

    unsafe {
        let _ = MoveWindow(hwnd, x, y, width, height, true);
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        // 热键按下让本线程获得前台权限，此处可以合法抢焦点收键盘
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

fn hide() {
    // 先取出状态并结束借用，再调 ShowWindow：隐藏持焦点的窗口会
    // 同步派发 WM_KILLFOCUS，回调里会再次进入 hide()，借用期间
    // 重入会触发 RefCell 双重借用 panic 并拖垮整个进程。
    let hwnd = PANEL.with_borrow_mut(|slot| slot.take().map(|s| s.hwnd));
    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// 确认选择：写回剪贴板 → 归还前台 → 模拟 Ctrl+V。
fn commit(index: usize) {
    let Some((entry, target)) = PANEL.with_borrow(|slot| {
        slot.as_ref()
            .and_then(|s| s.entries.get(index).map(|e| (e.clone(), s.target)))
    }) else {
        return;
    };
    hide();
    crate::log_line(&format!("选择条目 id={}", entry.id));

    let store = crate::open_store();
    match crate::paste::copy_entry_to_clipboard(&store, &entry) {
        Ok(true) => unsafe {
            if !target.is_invalid() {
                let _ = SetForegroundWindow(target);
                // 给目标窗口留出重新拿焦点的时间
                std::thread::sleep(std::time::Duration::from_millis(80));
                send_ctrl_v();
            }
        },
        Ok(false) => eprintln!("条目数据缺失，无法粘贴"),
        Err(e) => eprintln!("写回剪贴板失败：{e}"),
    }
}

/// 模拟 Ctrl+V。先补发 Shift 抬起：用户呼出面板的 Ctrl+Shift 可能
/// 还没松开，避免目标应用收到 Ctrl+Shift+V（浏览器的无格式粘贴）。
unsafe fn send_ctrl_v() {
    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                    ..Default::default()
                },
            },
        }
    }
    let inputs = [
        key(VK_SHIFT, true),
        key(VK_CONTROL, false),
        key(VIRTUAL_KEY(0x56), false),
        key(VIRTUAL_KEY(0x56), true),
        key(VK_CONTROL, true),
    ];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

/// 粘贴目标的文本光标位置；拿不到时用鼠标位置。
fn caret_or_cursor_pos(target: HWND) -> POINT {
    unsafe {
        if !target.is_invalid() {
            let thread = GetWindowThreadProcessId(target, None);
            let mut info = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            if GetGUIThreadInfo(thread, &mut info).is_ok()
                && !info.hwndCaret.is_invalid()
                && info.rcCaret.bottom > info.rcCaret.top
            {
                let mut p = POINT {
                    x: info.rcCaret.left,
                    y: info.rcCaret.bottom,
                };
                let _ = ClientToScreen(info.hwndCaret, &mut p);
                return p;
            }
        }
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        p
    }
}

fn scale(base: i32, dpi: u32) -> i32 {
    (base * dpi as i32 + 48) / 96
}

fn ensure_window() -> Option<HWND> {
    if let Some(hwnd) = PANEL.with_borrow(|s| s.as_ref().map(|s| s.hwnd)) {
        return Some(hwnd);
    }
    thread_local! {
        static CACHED_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };
        static CLASS_REGISTERED: RefCell<bool> = const { RefCell::new(false) };
    }
    if let Some(hwnd) = CACHED_HWND.with_borrow(|h| *h) {
        return Some(hwnd);
    }
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null()).ok()?;
        let class_name = w!("ShurufaClipboardPanel");
        CLASS_REGISTERED.with_borrow_mut(|registered| {
            if !*registered {
                let class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: class_name,
                    hbrBackground: HBRUSH::default(),
                    // 不设光标会导致悬停时一直显示忙碌转圈
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    ..Default::default()
                };
                RegisterClassW(&class);
                *registered = true;
            }
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            w!("剪贴板历史"),
            WS_POPUP,
            0,
            0,
            BASE_WIDTH,
            BASE_ROW_HEIGHT,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
        .ok()?;
        CACHED_HWND.with_borrow_mut(|h| *h = Some(hwnd));
        Some(hwnd)
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint(hdc, &ps.rcPaint);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            on_key(hwnd, VIRTUAL_KEY(wparam.0 as u16));
            LRESULT(0)
        }
        WM_CHAR => {
            on_char(hwnd, wparam.0 as u32);
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            hide();
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn on_key(hwnd: HWND, vk: VIRTUAL_KEY) {
    let handled = PANEL.with_borrow_mut(|slot| {
        let Some(state) = slot.as_mut() else {
            return None;
        };
        let count = state.entries.len();
        match vk {
            VK_ESCAPE => Some(Action::Close),
            VK_RETURN => Some(Action::Commit(state.selected)),
            VK_UP if count > 0 => {
                state.selected = state.selected.checked_sub(1).unwrap_or(count - 1);
                Some(Action::Repaint)
            }
            VK_DOWN if count > 0 => {
                state.selected = (state.selected + 1) % count;
                Some(Action::Repaint)
            }
            VK_BACK if !state.query.is_empty() => {
                state.query.pop();
                state.entries = entries_for_query(&state.query);
                state.selected = 0;
                Some(Action::Repaint)
            }
            // 数字键 1-9 直接选择
            VIRTUAL_KEY(code @ 0x31..=0x39) => {
                let index = (code - 0x31) as usize;
                (index < count).then_some(Action::Commit(index))
            }
            _ => None,
        }
    });
    match handled {
        Some(Action::Close) => hide(),
        Some(Action::Commit(i)) => commit(i),
        Some(Action::Repaint) => unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        },
        None => {}
    }
}

fn on_char(hwnd: HWND, code: u32) {
    let Some(character) = char::from_u32(code) else {
        return;
    };
    let changed = PANEL.with_borrow_mut(|slot| {
        let Some(state) = slot.as_mut() else {
            return false;
        };
        if !append_filter_character(&mut state.query, character) {
            return false;
        }
        state.entries = entries_for_query(&state.query);
        state.selected = 0;
        true
    });
    if changed {
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }
}

fn append_filter_character(query: &mut String, character: char) -> bool {
    if character.is_control() {
        return false;
    }
    query.push(character);
    true
}

fn entries_for_query(query: &str) -> Vec<ClipEntry> {
    let store = crate::open_store();
    if query.is_empty() {
        store.list(MAX_ROWS as u32, 0).unwrap_or_default()
    } else {
        store.search(query, MAX_ROWS as u32).unwrap_or_default()
    }
}

enum Action {
    Close,
    Commit(usize),
    Repaint,
}

unsafe fn make_font(height: i32, weight: i32) -> HFONT {
    CreateFontW(
        -height,
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
    PANEL.with_borrow(|slot| {
        let Some(state) = slot.as_ref() else {
            return;
        };
        let dpi = state.dpi;
        let padding = scale(BASE_PADDING, dpi);
        let row_h = scale(BASE_ROW_HEIGHT, dpi);
        let width = scale(BASE_WIDTH, dpi);

        let bg = CreateSolidBrush(COLORREF(COLOR_BG));
        FillRect(hdc, rc, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));
        SetBkMode(hdc, TRANSPARENT);

        let font = make_font(scale(BASE_FONT, dpi), FW_NORMAL.0 as i32);
        let bold = make_font(scale(BASE_FONT, dpi), FW_BOLD.0 as i32);
        let small = make_font(scale(BASE_SMALL_FONT, dpi), FW_NORMAL.0 as i32);
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));

        if state.entries.is_empty() {
            SetTextColor(hdc, COLORREF(COLOR_DIM));
            draw_line(
                hdc,
                if state.query.is_empty() {
                    "（历史为空）"
                } else {
                    "（无匹配条目）"
                },
                padding,
                padding,
                width - padding * 2,
                row_h,
            );
        }

        for (i, entry) in state.entries.iter().enumerate().take(MAX_ROWS) {
            let top = padding + i as i32 * row_h;
            if i == state.selected {
                let hl = CreateSolidBrush(COLORREF(COLOR_ROW_HL));
                let row_rect = RECT {
                    left: scale(4, dpi),
                    top,
                    right: width - scale(4, dpi),
                    bottom: top + row_h,
                };
                FillRect(hdc, &row_rect, hl);
                let _ = DeleteObject(HGDIOBJ(hl.0));
            }

            // 序号
            SelectObject(hdc, HGDIOBJ(bold.0));
            SetTextColor(hdc, COLORREF(COLOR_LABEL));
            draw_line(
                hdc,
                &format!("{}", i + 1),
                padding,
                top,
                scale(18, dpi),
                row_h,
            );

            // 类型标记 + 置顶星标
            SelectObject(hdc, HGDIOBJ(small.0));
            SetTextColor(hdc, COLORREF(COLOR_DIM));
            let tag = match (entry.kind, entry.pinned) {
                (ClipKind::Image, p) => {
                    if p {
                        "图★"
                    } else {
                        "图"
                    }
                }
                (ClipKind::Files, p) => {
                    if p {
                        "件★"
                    } else {
                        "件"
                    }
                }
                (ClipKind::Text, true) => "★",
                (ClipKind::Text, false) => "",
            };
            draw_line(
                hdc,
                tag,
                padding + scale(20, dpi),
                top,
                scale(30, dpi),
                row_h,
            );

            // 内容预览
            SelectObject(hdc, HGDIOBJ(font.0));
            SetTextColor(hdc, COLORREF(COLOR_TEXT));
            let preview = match entry.kind {
                ClipKind::Image => format!("[图片 {} KB]", (entry.data_size / 1024).max(1)),
                _ => crate::single_line_preview(&entry.text, 60),
            };
            draw_line(
                hdc,
                &preview,
                padding + scale(54, dpi),
                top,
                width - padding * 2 - scale(54, dpi),
                row_h,
            );
        }

        // 底部操作提示
        let footer_top = padding + state.entries.len().max(1) as i32 * row_h;
        SelectObject(hdc, HGDIOBJ(small.0));
        SetTextColor(hdc, COLORREF(COLOR_DIM));
        let footer = if state.query.is_empty() {
            "直接键入筛选 · 回车/数字 粘贴 · Esc 关闭".to_owned()
        } else {
            format!(
                "筛选：{} · 退格修改 · 回车/数字 粘贴 · Esc 关闭",
                crate::single_line_preview(&state.query, 18)
            )
        };
        draw_line(
            hdc,
            &footer,
            padding,
            footer_top,
            width - padding * 2,
            scale(BASE_FOOTER_HEIGHT, dpi),
        );

        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = DeleteObject(HGDIOBJ(bold.0));
        let _ = DeleteObject(HGDIOBJ(small.0));
    });
}

#[cfg(test)]
mod tests {
    use super::append_filter_character;

    #[test]
    fn 筛选条件接受_unicode_并忽略控制字符() {
        let mut query = "会议".to_owned();
        assert!(append_filter_character(&mut query, '纪'));
        assert!(append_filter_character(&mut query, '要'));
        assert!(!append_filter_character(&mut query, '\u{8}'));
        assert_eq!(query, "会议纪要");
    }
}

unsafe fn draw_line(hdc: HDC, text: &str, x: i32, y: i32, w: i32, h: i32) {
    // 空串必须跳过：空 Vec 的悬垂指针传入 DrawTextW 会在 user32
    // 内触发访问违例（0xc0000005），整个进程随之崩溃
    if text.is_empty() {
        return;
    }
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    DrawTextW(
        hdc,
        &mut utf16,
        &mut rect,
        DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
    );
}
