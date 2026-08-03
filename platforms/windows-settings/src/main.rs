//! Shurufa Windows 控制中心。
//!
//! 保持轻量 Win32 原生运行时，但以自绘导航、状态卡片和统一操作按钮提供完整桌面
//! 入口。配置仍由 sync-core 持久化，后台任务仍由同目录 shurufa-host 执行。

#![cfg(windows)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_BOLD, FW_NORMAL, HDC, HFONT, HGDIOBJ, PAINTSTRUCT,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::DRAWITEMSTRUCT;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, LoadCursorW,
    MessageBoxW, MoveWindow, PostQuitMessage, RegisterClassW, SetWindowTextW, ShowWindow,
    TranslateMessage, BS_OWNERDRAW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, HMENU, IDC_ARROW,
    MB_ICONERROR, MB_ICONINFORMATION, MSG, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DRAWITEM, WM_ERASEBKGND, WM_LBUTTONUP, WM_PAINT, WM_SIZE,
    WNDCLASSW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

const WINDOW_WIDTH: i32 = 1080;
const WINDOW_HEIGHT: i32 = 700;
const SIDEBAR_WIDTH: i32 = 224;
const CONTENT_MARGIN: i32 = 48;
const NAV_TOP: i32 = 170;
const NAV_ROW_HEIGHT: i32 = 48;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const COLOR_CANVAS: u32 = 0x00F6_F7_F8;
const COLOR_SIDEBAR: u32 = 0x0027_2B_20;
const COLOR_SIDEBAR_TEXT: u32 = 0x00EB_F0_EB;
const COLOR_NAV_ACTIVE: u32 = 0x0046_81_31;
const COLOR_SURFACE: u32 = 0x00FF_FF_FF;
const COLOR_TEXT: u32 = 0x0028_2B_25;
const COLOR_MUTED: u32 = 0x007B_81_7B;
const COLOR_TEAL: u32 = 0x0078_6F_1C;
const COLOR_TEAL_DARK: u32 = 0x005A_51_14;
const COLOR_CORAL: u32 = 0x005C_6B_E5;
const COLOR_BLUE: u32 = 0x00C7_76_3F;

const EDIT_STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | 0x0080);
const BUTTON_STYLE: WINDOW_STYLE =
    WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | BS_OWNERDRAW as u32 | 0x0001_0000);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Dictionary,
    Sync,
    System,
}

impl Page {
    const ALL: [Self; 4] = [Self::Overview, Self::Dictionary, Self::Sync, Self::System];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "概览",
            Self::Dictionary => "词库",
            Self::Sync => "同步",
            Self::System => "设置",
        }
    }

    fn from_nav_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
}

struct Controls {
    relay: HWND,
    save_relay: HWND,
    start_service: HWND,
    update_dictionary: HWND,
    open_windows_settings: HWND,
    page: Page,
}

thread_local! {
    static CONTROLS: RefCell<Option<Controls>> = const { RefCell::new(None) };
}

fn app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
}

fn sync_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("SHURUFA_SYNC_DIR") {
        PathBuf::from(path)
    } else {
        app_data_dir().join("sync")
    }
}

fn sibling_exe(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn relay_text() -> String {
    sync_core::load_relay_addr(&sync_dir()).unwrap_or_default()
}

fn service_status() -> &'static str {
    let state = std::fs::read_to_string(app_data_dir().join("daemon.state")).unwrap_or_default();
    if state.contains("status=running") {
        "运行中"
    } else {
        "待启动"
    }
}

fn edit_text(hwnd: HWND) -> String {
    let mut buffer = [0u16; 512];
    let length =
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

fn show_message(hwnd: HWND, text: &str, error: bool) {
    unsafe {
        let _ = MessageBoxW(
            Some(hwnd),
            &HSTRING::from(text),
            w!("Shurufa 控制中心"),
            if error {
                MB_ICONERROR
            } else {
                MB_ICONINFORMATION
            },
        );
    }
}

fn launch_host(args: &[&str]) -> Result<(), String> {
    Command::new(sibling_exe("shurufa-host.exe"))
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn open_windows_settings() -> Result<(), String> {
    Command::new("cmd.exe")
        .args(["/c", "start", "", "ms-settings:regionlanguage"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn create_control(
    class: PCWSTR,
    title: PCWSTR,
    style: WINDOW_STYLE,
    parent: HWND,
    instance: HINSTANCE,
) -> HWND {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            title,
            style,
            0,
            0,
            1,
            1,
            Some(parent),
            None::<HMENU>,
            Some(instance),
            None,
        )
        .unwrap_or_default()
    }
}

fn create_button(title: PCWSTR, parent: HWND, instance: HINSTANCE) -> HWND {
    create_control(w!("BUTTON"), title, BUTTON_STYLE, parent, instance)
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

unsafe fn fill(hdc: HDC, rect: &RECT, color: u32) {
    let brush = CreateSolidBrush(COLORREF(color));
    FillRect(hdc, rect, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

unsafe fn draw_text(
    hdc: HDC,
    text: &str,
    rect: RECT,
    color: u32,
    flags: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT,
) {
    if text.is_empty() {
        return;
    }
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    SetTextColor(hdc, COLORREF(color));
    let mut rect = rect;
    DrawTextW(hdc, &mut utf16, &mut rect, flags);
}

unsafe fn card(hdc: HDC, left: i32, top: i32, width: i32, height: i32) {
    let rect = RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    };
    fill(hdc, &rect, COLOR_SURFACE);
}

unsafe fn draw_nav(hdc: HDC, page: Page) {
    let title_font = make_font(28, FW_BOLD.0 as i32);
    let body_font = make_font(14, FW_NORMAL.0 as i32);
    let nav_font = make_font(16, FW_NORMAL.0 as i32);
    let old = SelectObject(hdc, HGDIOBJ(title_font.0));
    draw_text(
        hdc,
        "Shurufa",
        RECT {
            left: 28,
            top: 42,
            right: SIDEBAR_WIDTH - 24,
            bottom: 82,
        },
        COLOR_SIDEBAR_TEXT,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    SelectObject(hdc, HGDIOBJ(body_font.0));
    draw_text(
        hdc,
        "输入与剪贴板控制中心",
        RECT {
            left: 29,
            top: 84,
            right: SIDEBAR_WIDTH - 20,
            bottom: 110,
        },
        0x00B5_BF_B2,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
    );
    SelectObject(hdc, HGDIOBJ(nav_font.0));
    for (index, item) in Page::ALL.iter().enumerate() {
        let top = NAV_TOP + index as i32 * NAV_ROW_HEIGHT;
        if *item == page {
            fill(
                hdc,
                &RECT {
                    left: 16,
                    top,
                    right: SIDEBAR_WIDTH - 16,
                    bottom: top + 40,
                },
                COLOR_NAV_ACTIVE,
            );
        }
        draw_text(
            hdc,
            item.label(),
            RECT {
                left: 36,
                top,
                right: SIDEBAR_WIDTH - 24,
                bottom: top + 40,
            },
            COLOR_SIDEBAR_TEXT,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
    }
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(title_font.0));
    let _ = DeleteObject(HGDIOBJ(body_font.0));
    let _ = DeleteObject(HGDIOBJ(nav_font.0));
}

unsafe fn draw_overview(hdc: HDC, left: i32, width: i32) {
    draw_header(
        hdc,
        "工作台",
        "输入、词库与跨设备剪贴板都在这里",
        left,
        width,
    );
    let status = service_status();
    card(hdc, left, 168, width, 112);
    let title = make_font(17, FW_BOLD.0 as i32);
    let body = make_font(14, FW_NORMAL.0 as i32);
    let old = SelectObject(hdc, HGDIOBJ(title.0));
    draw_text(
        hdc,
        "后台服务",
        RECT {
            left: left + 24,
            top: 190,
            right: left + 240,
            bottom: 220,
        },
        COLOR_TEXT,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    SelectObject(hdc, HGDIOBJ(body.0));
    draw_text(
        hdc,
        status,
        RECT {
            left: left + 24,
            top: 228,
            right: left + 180,
            bottom: 252,
        },
        COLOR_TEAL,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    let card_width = (width - 32) / 3;
    let cards = [
        ("剪贴板历史", "Ctrl+Shift+V", COLOR_BLUE),
        ("输入方案", "雾凇拼音", COLOR_TEAL),
        ("热门词库", "rime-ice", COLOR_CORAL),
    ];
    for (index, (title_text, value, color)) in cards.iter().enumerate() {
        let x = left + index as i32 * (card_width + 16);
        card(hdc, x, 304, card_width, 136);
        SelectObject(hdc, HGDIOBJ(body.0));
        draw_text(
            hdc,
            title_text,
            RECT {
                left: x + 20,
                top: 326,
                right: x + card_width - 20,
                bottom: 350,
            },
            COLOR_MUTED,
            DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS | DT_VCENTER | DT_NOPREFIX,
        );
        SelectObject(hdc, HGDIOBJ(title.0));
        draw_text(
            hdc,
            value,
            RECT {
                left: x + 20,
                top: 366,
                right: x + card_width - 20,
                bottom: 398,
            },
            *color,
            DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS | DT_VCENTER | DT_NOPREFIX,
        );
    }
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(title.0));
    let _ = DeleteObject(HGDIOBJ(body.0));
}

unsafe fn draw_dictionary(hdc: HDC, left: i32, width: i32) {
    draw_header(hdc, "词库", "让常用表达保持在输入法里", left, width);
    card(hdc, left, 168, width, 210);
    let title = make_font(20, FW_BOLD.0 as i32);
    let body = make_font(14, FW_NORMAL.0 as i32);
    let old = SelectObject(hdc, HGDIOBJ(title.0));
    draw_text(
        hdc,
        "雾凇拼音",
        RECT {
            left: left + 28,
            top: 196,
            right: left + 320,
            bottom: 230,
        },
        COLOR_TEXT,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    SelectObject(hdc, HGDIOBJ(body.0));
    draw_text(
        hdc,
        "热门云词库：rime-ice",
        RECT {
            left: left + 28,
            top: 244,
            right: left + 400,
            bottom: 270,
        },
        COLOR_MUTED,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    draw_text(
        hdc,
        "更新完成后重启输入法即可生效。",
        RECT {
            left: left + 28,
            top: 278,
            right: left + 460,
            bottom: 304,
        },
        COLOR_MUTED,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(title.0));
    let _ = DeleteObject(HGDIOBJ(body.0));
}

unsafe fn draw_sync(hdc: HDC, left: i32, width: i32) {
    draw_header(hdc, "剪贴板同步", "设备间内容保持连贯", left, width);
    card(hdc, left, 168, width, 240);
    let title = make_font(18, FW_BOLD.0 as i32);
    let body = make_font(14, FW_NORMAL.0 as i32);
    let old = SelectObject(hdc, HGDIOBJ(title.0));
    draw_text(
        hdc,
        "自托管中继",
        RECT {
            left: left + 28,
            top: 194,
            right: left + 350,
            bottom: 226,
        },
        COLOR_TEXT,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    SelectObject(hdc, HGDIOBJ(body.0));
    draw_text(
        hdc,
        "留空即可关闭；修改将在后台服务下次启动时生效。",
        RECT {
            left: left + 28,
            top: 236,
            right: left + width - 28,
            bottom: 260,
        },
        COLOR_MUTED,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
    );
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(title.0));
    let _ = DeleteObject(HGDIOBJ(body.0));
}

unsafe fn draw_system(hdc: HDC, left: i32, width: i32) {
    draw_header(hdc, "设置", "管理 Windows 输入体验", left, width);
    card(hdc, left, 168, width, 190);
    let title = make_font(18, FW_BOLD.0 as i32);
    let body = make_font(14, FW_NORMAL.0 as i32);
    let old = SelectObject(hdc, HGDIOBJ(title.0));
    draw_text(
        hdc,
        "系统输入法",
        RECT {
            left: left + 28,
            top: 196,
            right: left + 400,
            bottom: 228,
        },
        COLOR_TEXT,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    SelectObject(hdc, HGDIOBJ(body.0));
    draw_text(
        hdc,
        "在 Windows 设置中管理语言、输入法与默认输入法。",
        RECT {
            left: left + 28,
            top: 240,
            right: left + width - 28,
            bottom: 266,
        },
        COLOR_MUTED,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
    );
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(title.0));
    let _ = DeleteObject(HGDIOBJ(body.0));
}

unsafe fn draw_header(hdc: HDC, title: &str, subtitle: &str, left: i32, width: i32) {
    let title_font = make_font(30, FW_BOLD.0 as i32);
    let body_font = make_font(15, FW_NORMAL.0 as i32);
    let old = SelectObject(hdc, HGDIOBJ(title_font.0));
    draw_text(
        hdc,
        title,
        RECT {
            left,
            top: 54,
            right: left + width,
            bottom: 96,
        },
        COLOR_TEXT,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    SelectObject(hdc, HGDIOBJ(body_font.0));
    draw_text(
        hdc,
        subtitle,
        RECT {
            left,
            top: 106,
            right: left + width,
            bottom: 132,
        },
        COLOR_MUTED,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
    );
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(title_font.0));
    let _ = DeleteObject(HGDIOBJ(body_font.0));
}

unsafe fn paint(hwnd: HWND, hdc: HDC) {
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    fill(hdc, &client, COLOR_CANVAS);
    fill(
        hdc,
        &RECT {
            left: 0,
            top: 0,
            right: SIDEBAR_WIDTH,
            bottom: client.bottom,
        },
        COLOR_SIDEBAR,
    );
    SetBkMode(hdc, TRANSPARENT);
    let page = CONTROLS.with_borrow(|state| state.as_ref().map(|state| state.page));
    let Some(page) = page else {
        return;
    };
    draw_nav(hdc, page);
    let left = SIDEBAR_WIDTH + CONTENT_MARGIN;
    let width = (client.right - left - CONTENT_MARGIN).max(300);
    match page {
        Page::Overview => draw_overview(hdc, left, width),
        Page::Dictionary => draw_dictionary(hdc, left, width),
        Page::Sync => draw_sync(hdc, left, width),
        Page::System => draw_system(hdc, left, width),
    }
}

unsafe fn draw_button(item: &DRAWITEMSTRUCT) {
    let (title, base_color) = CONTROLS.with_borrow(|state| {
        let Some(controls) = state.as_ref() else {
            return ("", COLOR_TEAL);
        };
        if item.hwndItem == controls.update_dictionary {
            ("更新热门云词库", COLOR_CORAL)
        } else if item.hwndItem == controls.save_relay {
            ("保存中继", COLOR_TEAL)
        } else if item.hwndItem == controls.open_windows_settings {
            ("打开系统输入法设置", COLOR_BLUE)
        } else {
            ("启动后台服务", COLOR_TEAL)
        }
    });
    let pressed = item.itemState.0 & 0x0001 != 0;
    let color = if pressed { COLOR_TEAL_DARK } else { base_color };
    fill(item.hDC, &item.rcItem, color);
    let font = make_font(14, FW_BOLD.0 as i32);
    let old = SelectObject(item.hDC, HGDIOBJ(font.0));
    draw_text(
        item.hDC,
        title,
        item.rcItem,
        COLOR_SURFACE,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
    );
    SelectObject(item.hDC, old);
    let _ = DeleteObject(HGDIOBJ(font.0));
}

unsafe fn update_layout(hwnd: HWND) {
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let content_left = SIDEBAR_WIDTH + CONTENT_MARGIN;
    let content_width = (client.right - content_left - CONTENT_MARGIN).max(300);
    CONTROLS.with_borrow(|state| {
        let Some(controls) = state.as_ref() else {
            return;
        };
        let _ = MoveWindow(
            controls.start_service,
            content_left + content_width - 190,
            198,
            160,
            42,
            true,
        );
        let _ = MoveWindow(
            controls.update_dictionary,
            content_left + 28,
            320,
            180,
            42,
            true,
        );
        let _ = MoveWindow(
            controls.relay,
            content_left + 28,
            282,
            (content_width - 210).max(160),
            38,
            true,
        );
        let _ = MoveWindow(
            controls.save_relay,
            content_left + content_width - 166,
            282,
            138,
            38,
            true,
        );
        let _ = MoveWindow(
            controls.open_windows_settings,
            content_left + 28,
            292,
            190,
            42,
            true,
        );
        let _ = ShowWindow(
            controls.start_service,
            if controls.page == Page::Overview {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        let _ = ShowWindow(
            controls.update_dictionary,
            if controls.page == Page::Dictionary {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        let sync_visible = controls.page == Page::Sync;
        let _ = ShowWindow(controls.relay, if sync_visible { SW_SHOW } else { SW_HIDE });
        let _ = ShowWindow(
            controls.save_relay,
            if sync_visible { SW_SHOW } else { SW_HIDE },
        );
        let _ = ShowWindow(
            controls.open_windows_settings,
            if controls.page == Page::System {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
    });
    let _ = InvalidateRect(Some(hwnd), None, true);
}

fn switch_page(hwnd: HWND, page: Page) {
    CONTROLS.with_borrow_mut(|state| {
        if let Some(controls) = state.as_mut() {
            controls.page = page;
        }
    });
    unsafe { update_layout(hwnd) };
}

fn point_from_lparam(lparam: LPARAM) -> (i32, i32) {
    let value = lparam.0 as i32;
    (value as i16 as i32, (value >> 16) as i16 as i32)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    _wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            let relay = create_control(
                w!("EDIT"),
                PCWSTR::null(),
                EDIT_STYLE,
                hwnd,
                instance.into(),
            );
            let _ = SetWindowTextW(relay, &HSTRING::from(relay_text()));
            let save_relay = create_button(w!("保存中继"), hwnd, instance.into());
            let start_service = create_button(w!("启动后台服务"), hwnd, instance.into());
            let update_dictionary = create_button(w!("更新热门云词库"), hwnd, instance.into());
            let open_windows_settings =
                create_button(w!("打开系统输入法设置"), hwnd, instance.into());
            CONTROLS.with_borrow_mut(|state| {
                *state = Some(Controls {
                    relay,
                    save_relay,
                    start_service,
                    update_dictionary,
                    open_windows_settings,
                    page: Page::Overview,
                });
            });
            update_layout(hwnd);
            LRESULT(0)
        }
        WM_COMMAND => {
            let source = HWND(lparam.0 as *mut c_void);
            CONTROLS.with_borrow(|state| {
                let Some(controls) = state.as_ref() else {
                    return;
                };
                if source == controls.save_relay {
                    let value = edit_text(controls.relay);
                    let relay = (!value.trim().is_empty()).then_some(value.trim());
                    match sync_core::save_relay_addr(&sync_dir(), relay) {
                        Ok(()) => show_message(hwnd, "中继设置已保存。重启后台服务后生效。", false),
                        Err(error) => {
                            show_message(hwnd, &format!("保存中继设置失败：{error}"), true)
                        }
                    }
                } else if source == controls.start_service {
                    match launch_host(&["supervise"]) {
                        Ok(()) => {
                            show_message(hwnd, "后台服务已在后台启动。", false);
                            let _ = InvalidateRect(Some(hwnd), None, true);
                        }
                        Err(error) => {
                            show_message(hwnd, &format!("启动后台服务失败：{error}"), true)
                        }
                    }
                } else if source == controls.update_dictionary {
                    match launch_host(&["dict-update", "rime-ice"]) {
                        Ok(()) => {
                            show_message(hwnd, "词库更新已在后台启动。完成后请重启输入法。", false)
                        }
                        Err(error) => {
                            show_message(hwnd, &format!("启动词库更新失败：{error}"), true)
                        }
                    }
                } else if source == controls.open_windows_settings {
                    if let Err(error) = open_windows_settings() {
                        show_message(hwnd, &format!("打开系统设置失败：{error}"), true);
                    }
                }
            });
            LRESULT(0)
        }
        WM_DRAWITEM => {
            let item = &*(lparam.0 as *const DRAWITEMSTRUCT);
            draw_button(item);
            LRESULT(1)
        }
        WM_LBUTTONUP => {
            let (x, y) = point_from_lparam(lparam);
            if x < SIDEBAR_WIDTH && y >= NAV_TOP && y < NAV_TOP + NAV_ROW_HEIGHT * 4 {
                let index = ((y - NAV_TOP) / NAV_ROW_HEIGHT) as usize;
                if let Some(page) = Page::from_nav_index(index) {
                    switch_page(hwnd, page);
                }
            }
            LRESULT(0)
        }
        WM_SIZE => {
            update_layout(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut paint_struct = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint_struct);
            paint(hwnd, hdc);
            let _ = EndPaint(hwnd, &paint_struct);
            LRESULT(0)
        }
        WM_DESTROY => {
            CONTROLS.with_borrow_mut(|state| *state = None);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, _wparam, lparam),
    }
}

fn main() {
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null()).expect("获取模块句柄失败");
        let class = w!("ShurufaControlCenterWindow");
        let window_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&window_class);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Shurufa 控制中心"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("创建控制中心窗口失败");
        let _ = ShowWindow(hwnd, SW_SHOW);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{sync_dir, Page};

    #[test]
    fn 默认同步目录位于应用数据目录下() {
        assert!(sync_dir().ends_with("shurufa\\sync"));
    }

    #[test]
    fn 导航包含四个完整页面() {
        assert_eq!(Page::ALL.len(), 4);
        assert_eq!(Page::Dictionary.label(), "词库");
    }
}
