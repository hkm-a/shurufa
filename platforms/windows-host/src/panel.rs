//! 剪贴板历史面板：全局热键呼出的置顶弹窗。
//!
//! 交互：Ctrl+Shift+V（冲突时回落 Alt+V）呼出 → 直接键入筛选、↑/↓ 或数字键
//! 选择 → 回车写回剪贴板并向原前台窗口模拟 Ctrl+V 完成粘贴，Esc 或失焦关闭。
//! 面板与监听器同属一条 UI 线程，状态挂 thread_local。
//!
//! 本轮改动摘要（皮肤 v2 / 现代化外观 / 主题热切换）：
//! - 删除硬编码 COLOR_* 常量；颜色统一来自共享 `skin::Skin`
//!   （与 TSF 端同一份 skin.rs，经 `#[path]` 引入，按系统 light/dark 变体）。
//! - 字号乘 `metrics.font_scale`；`metrics.opacity` < 1 时整体透明；
//!   `skin::apply_appearance` 应用 Win11 圆角 + 深色边框，`ShadowShell` 画阴影。
//! - 新增隐藏顶层"心眼"窗口 `ensure_theme_watcher` 接收 WM_SETTINGCHANGE
//!   （ImmersiveColorSet），触发 Skin 缓存刷新并使历史/AI 两面板即时换肤。

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
    GetKeyState, RegisterHotKey, SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_SHIFT, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DOWN,
    VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, GetCursorPos,
    GetForegroundWindow, GetGUIThreadInfo, GetSystemMetrics, GetWindowThreadProcessId, LoadCursorW,
    MoveWindow, RegisterClassW, SetForegroundWindow, ShowWindow, TrackPopupMenu, CS_HREDRAW,
    CS_VREDRAW, GUITHREADINFO, IDC_ARROW, MF_GRAYED, MF_STRING, SM_CXSCREEN, SM_CYSCREEN, SW_HIDE,
    SW_SHOWNA, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_TOPALIGN, WM_CHAR, WM_KEYDOWN, WM_KILLFOCUS,
    WM_PAINT, WM_RBUTTONUP, WM_SETTINGCHANGE, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

// 与 TSF 端共享同一份 skin 解析/DWM 助手源码（该文件不引用任何 crate 专属符号）。
#[path = "../../windows/src/skin.rs"]
pub(crate) mod skin;
use skin::{ShadowShell, Skin};

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

/// 面板调色板：全部来自皮肤候选窗段，任何硬编码颜色都禁止出现在绘制路径。
#[derive(Clone, Copy)]
struct Palette {
    bg: u32,
    row_hl: u32,
    text: u32,
    dim: u32,
    label: u32,
}

fn palette() -> (Palette, skin::Metrics, skin::Shadow) {
    let skin = Skin::current();
    let c = skin.candidate;
    (
        Palette {
            bg: c.background,
            row_hl: c.highlight_background,
            text: c.text,
            dim: c.preedit,
            label: c.label,
        },
        skin.metrics,
        skin.shadow,
    )
}

struct PanelState {
    hwnd: HWND,
    entries: Vec<ClipEntry>,
    query: String,
    selected: usize,
    /// 呼出面板时的前台窗口，粘贴目标
    target: HWND,
    dpi: u32,
    /// Ctrl+F 切换：true = 仅显示收藏，false = 全部
    favorites_only: bool,
}

thread_local! {
    static PANEL: RefCell<Option<PanelState>> = const { RefCell::new(None) };
    static SHADOW: RefCell<ShadowShell> = RefCell::new(ShadowShell::new());
    /// 面板窗口句柄：主题切换回调靠它找到并重绘面板。
    static PANEL_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };
}

/// 注册全局热键；首选 Ctrl+Shift+V（对齐微信输入法习惯），
/// 被占用时回落 Alt+V。返回实际生效的描述。
/// 线程级注册（hwnd=None）：WM_HOTKEY 直接进线程队列，由消息循环
/// 截获，不依赖窗口消息投递路径。
/// 同时创建隐藏的主题监听窗口（WM_SETTINGCHANGE 只广播到顶层窗口，
/// 消息专用窗口收不到，所以这里用一个永不显示的顶层窗口）。
pub fn register_hotkey() -> &'static str {
    ensure_theme_watcher();
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
    // 每次弹出都按当前皮肤重设外观（圆角/深边框/透明度）
    let (_, _, shadow) = palette();
    let skin = Skin::current();
    skin::apply_appearance(hwnd, &skin);
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
            favorites_only: false,
        });
    });

    unsafe {
        let _ = MoveWindow(hwnd, x, y, width, height, true);
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        // 阴影壳：主窗下一层的半透明黑圆角壳
        SHADOW.with_borrow_mut(|shell| shell.sync(hwnd, x, y, width, height, &shadow));
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
    SHADOW.with_borrow_mut(|shell| shell.hide());
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
        skin::apply_appearance(hwnd, &Skin::current());
        CACHED_HWND.with_borrow_mut(|h| *h = Some(hwnd));
        register_panel_hwnd(hwnd);
        Some(hwnd)
    }
}

/// 主题（或皮肤文件）变化后由主题监听窗口调用：重设外观并重绘可见面板。
pub fn on_theme_changed() {
    let hwnd = PANEL_HWND.with_borrow(|h| *h);
    if let Some(hwnd) = hwnd {
        let skin = Skin::current();
        unsafe {
            skin::apply_appearance(hwnd, &skin);
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }
}

/// 登记面板窗口句柄，供主题切换回调找到它。
fn register_panel_hwnd(hwnd: HWND) {
    PANEL_HWND.with_borrow_mut(|h| *h = Some(hwnd));
}

/// 心眼窗口：永不显示的顶层窗口，只为接收系统广播 WM_SETTINGCHANGE。
/// （HWND_MESSAGE 消息专用窗口收不到广播，所以必须是真顶层窗口。）
fn ensure_theme_watcher() {
    thread_local! {
        static WATCHER_CREATED: RefCell<bool> = const { RefCell::new(false) };
    }
    if WATCHER_CREATED.with_borrow(|b| *b) {
        return;
    }
    unsafe {
        let Ok(hinstance) = GetModuleHandleW(PCWSTR::null()) else {
            return;
        };
        let class_name = w!("ShurufaThemeWatcher");
        let class = WNDCLASSW {
            lpfnWndProc: Some(theme_watcher_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            Default::default(),
            class_name,
            w!(""),
            Default::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        );
        if hwnd.is_ok() {
            WATCHER_CREATED.with_borrow_mut(|b| *b = true);
        }
    }
}

unsafe extern "system" fn theme_watcher_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_SETTINGCHANGE && skin::is_immersive_color_change(lparam) {
        // 刷新共享皮肤缓存，随后让两个面板各自按新皮肤重设外观+重绘
        let _ = Skin::refresh_on_setting_change();
        crate::panel::on_theme_changed();
        crate::ai_panel::on_theme_changed();
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
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
        WM_RBUTTONUP => {
            on_right_click(hwnd, lparam);
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            hide();
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            // 面板自身也收到广播：隐藏状态下冷启动时按新皮肤画
            if skin::is_immersive_color_change(lparam) {
                let skin = Skin::refresh_on_setting_change();
                unsafe {
                    skin::apply_appearance(hwnd, &skin);
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn on_key(hwnd: HWND, vk: VIRTUAL_KEY) {
    // Ctrl+F：切换"⭐ 收藏"过滤；与 spec 一致，不引入其他键盘快捷键。
    if vk == VIRTUAL_KEY(0x46) && is_ctrl_down() {
        let next_mode = PANEL.with_borrow_mut(|slot| {
            let state = slot.as_mut()?;
            state.favorites_only = !state.favorites_only;
            // 切换时重置选择并重拉列表（收藏视图从收藏项回源剪贴板历史）
            let all = entries_for_query(&state.query);
            state.entries = if state.favorites_only {
                filter_by_favorites(all)
            } else {
                all
            };
            if !state.entries.is_empty() && state.selected >= state.entries.len() {
                state.selected = 0;
            }
            Some(state.favorites_only)
        });
        if let Some(mode) = next_mode {
            crate::log_line(&format!(
                "历史面板过滤切换：{}",
                if mode { "⭐ 收藏" } else { "全部" }
            ));
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
        }
        return;
    }
    let handled = PANEL.with_borrow_mut(|slot| {
        let state = slot.as_mut()?;
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
                let all = entries_for_query(&state.query);
                state.entries = if state.favorites_only {
                    filter_by_favorites(all)
                } else {
                    all
                };
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

/// 键盘 Ctrl 当前是否按下；用 GetKeyState 高位判定（Win32 惯例）。
fn is_ctrl_down() -> bool {
    unsafe { (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0 }
}

/// 右键：在 point 处弹上下文菜单。菜单 id 用于 TrackPopupMenu 返回值分支。
/// 菜单项与 spec 一致：
///   - 收藏 ★ / 取消收藏 ☆（当前条目的 pinned_at_ms 符号翻转）
///   - 以文件形式转发（仅图片/文件条目可用）
///   - ⭐ 收藏过滤 = 收藏夹视图（等价于 Ctrl+F）
///
/// 注意：菜单 id ${id..} 仅为本面板内部使用，不与系统菜单冲突。
const CTX_TOGGLE_FAVORITE: u16 = 1;
const CTX_FORWARD_AS_FILE: u16 = 2;

fn on_right_click(hwnd: HWND, lparam: LPARAM) {
    // lparam 是屏幕坐标（右键消息约定），低位 x / 高位 y（可为负，禁截位）
    let x = (lparam.0 as i32 & 0xFFFF) as i16 as i32;
    let y = ((lparam.0 >> 16) as i16) as i32;
    let (selected_kind, is_favorited_now) = PANEL.with_borrow(|slot| {
        slot.as_ref()
            .and_then(|s| {
                let entry = s.entries.get(s.selected)?;
                let favs = shurufa_options::favorites::load_favorites();
                let fav = is_favorited(&favs, entry);
                Some((entry.kind, fav))
            })
            .unwrap_or((ClipKind::Text, false))
    });
    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };
        let fav_label = if is_favorited_now {
            w!("取消收藏 ☆")
        } else {
            w!("收藏 ★")
        };
        let _ = AppendMenuW(menu, MF_STRING, CTX_TOGGLE_FAVORITE as usize, fav_label);
        // 仅图片/文件可转发；文本条目灰显
        let forward_flags = if matches!(selected_kind, ClipKind::Image | ClipKind::Files) {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        let _ = AppendMenuW(
            menu,
            forward_flags,
            CTX_FORWARD_AS_FILE as usize,
            w!("以文件形式转发"),
        );
        let _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RETURNCMD,
            x,
            y,
            Some(0),
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        match picked.0 as u16 {
            CTX_TOGGLE_FAVORITE => {
                let _ = toggle_favorite_on_selected();
                // 收藏过滤打开时，取消收藏应立刻让条目消失：重拉列表并重绘
                PANEL.with_borrow_mut(|slot| {
                    if let Some(state) = slot.as_mut() {
                        let all = entries_for_query(&state.query);
                        state.entries = if state.favorites_only {
                            filter_by_favorites(all)
                        } else {
                            all
                        };
                        if !state.entries.is_empty() && state.selected >= state.entries.len() {
                            state.selected = state.entries.len() - 1;
                        }
                    }
                });
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
            CTX_FORWARD_AS_FILE => {
                forward_selected_as_file();
            }
            _ => {}
        }
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
        let all = entries_for_query(&state.query);
        state.entries = if state.favorites_only {
            filter_by_favorites(all)
        } else {
            all
        };
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

/// 收藏过滤：把候选条目按收藏 id 集（pinned_at_ms > 0 视为已收藏）过滤。
/// 合并 clip-favorites.json 中的 source_peer 供绘制层标注"来自对端"。
fn filter_by_favorites(entries: Vec<ClipEntry>) -> Vec<ClipEntry> {
    let favs = shurufa_options::favorites::load_favorites();
    if favs.entries.is_empty() {
        return Vec::new();
    }
    entries
        .into_iter()
        .filter(|entry| is_favorited(&favs, entry))
        .collect()
}

fn is_favorited(favs: &shurufa_options::ClipFavorites, entry: &ClipEntry) -> bool {
    favs.entries
        .iter()
        .any(|fav| fav.pinned_at_ms > 0 && favorite_matches_entry(fav, entry))
}

/// 匹配规则：以 kind + 内容指纹近似。文本用 content_text 完全相等；图片/文件
/// 用 path 与 entry.text 的首行/完整内容比较。历史 id 不写入收藏，避免
/// 用户清理历史后收藏孤儿化；代价是同内容的重复条目会同时被视为收藏。
fn favorite_matches_entry(fav: &shurufa_options::ClipFavorite, entry: &ClipEntry) -> bool {
    use shurufa_options::ClipFavoriteKind as FK;
    match (fav.kind, entry.kind) {
        (FK::Text, ClipKind::Text) => fav
            .content_text
            .as_deref()
            .map(|t| t == entry.text)
            .unwrap_or(false),
        (FK::Image, ClipKind::Image) => fav
            .path
            .as_deref()
            .map(|p| p == entry.text)
            .unwrap_or(false),
        (FK::File, ClipKind::Files) => fav
            .path
            .as_deref()
            .map(|p| p == entry.text)
            .unwrap_or(false),
        _ => false,
    }
}

/// 对当前选中项做"收藏 / 取消收藏"切换；返回新的收藏状态（None 表示面板不可用）。
fn toggle_favorite_on_selected() -> Option<bool> {
    let entry = PANEL.with_borrow(|slot| {
        slot.as_ref()
            .and_then(|s| s.entries.get(s.selected).cloned())
    })?;
    use shurufa_options::{ClipFavorite, ClipFavoriteKind};
    let mut favs = shurufa_options::favorites::load_favorites();
    // 已收藏 → 切换为取消收藏（符号翻转语义，见 options::favorites::toggle_pin_favorite）
    if let Some(existing) = favs
        .entries
        .iter()
        .find(|f| f.pinned_at_ms > 0 && favorite_matches_entry(f, &entry))
        .map(|f| f.id)
    {
        match shurufa_options::favorites::toggle_pin_favorite(existing) {
            Ok(new_state) => {
                crate::log_line(&format!("取消收藏 id={existing}"));
                return new_state;
            }
            Err(e) => {
                crate::log_line(&format!("取消收藏失败：{e}"));
                return None;
            }
        }
    }
    // 未收藏 → 追加新条目
    let (kind, content_text, path) = match entry.kind {
        ClipKind::Text => (ClipFavoriteKind::Text, Some(entry.text.clone()), None),
        ClipKind::Image => (ClipFavoriteKind::Image, None, Some(entry.text.clone())),
        ClipKind::Files => (ClipFavoriteKind::File, None, Some(entry.text.clone())),
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let fav = ClipFavorite {
        id: 0, // 实际 id 由 add_favorite 分配（next_id 单调）
        kind,
        content_text,
        path,
        pinned_at_ms: now_ms,
        source_peer: Some(entry.source_app.clone()).filter(|s| !s.is_empty()),
    };
    match shurufa_options::favorites::add_favorite(fav) {
        Ok(saved) => {
            crate::log_line(&format!("已收藏 id={} kind={:?}", saved.id, saved.kind));
            favs.entries.push(saved);
            Some(true)
        }
        Err(e) => {
            crate::log_line(&format!("收藏失败：{e}"));
            None
        }
    }
}

/// Image/File 条目：以文件形式转发给所有已配对设备。
/// 路径取自 entry.text（文件列表的首行就是磁盘路径）。
/// 委托给 `crate::sync::send_file_to_all`——本函数仅做调用点封装（sync.rs 未动）。
fn forward_selected_as_file() {
    let Some((kind, text)) = PANEL.with_borrow(|slot| {
        slot.as_ref()
            .and_then(|s| s.entries.get(s.selected).map(|e| (e.kind, e.text.clone())))
    }) else {
        return;
    };
    if matches!(kind, ClipKind::Text) {
        crate::log_line("转发仅对图片/文件条目生效（文本请直接粘贴）");
        return;
    }
    let first_line = text.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        crate::log_line("转发失败：条目未携带文件路径");
        return;
    }
    let path = std::path::Path::new(first_line);
    if !path.exists() {
        crate::log_line(&format!("转发失败：文件不存在 {first_line}"));
        return;
    }
    crate::log_line(&format!("以文件形式转发：{first_line}"));
    crate::sync::send_file_to_all(path);
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

/// DPI 缩放后乘皮肤字号倍率。
fn scaled_font(base: i32, dpi: u32, font_scale: f32) -> i32 {
    ((scale(base, dpi) as f32) * font_scale).round().max(8.0) as i32
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
        let (colors, metrics, _) = palette();
        let fs = metrics.font_scale;

        let bg = CreateSolidBrush(COLORREF(colors.bg));
        FillRect(hdc, rc, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));
        SetBkMode(hdc, TRANSPARENT);

        let font = make_font(scaled_font(BASE_FONT, dpi, fs), FW_NORMAL.0 as i32);
        let bold = make_font(scaled_font(BASE_FONT, dpi, fs), FW_BOLD.0 as i32);
        let small = make_font(scaled_font(BASE_SMALL_FONT, dpi, fs), FW_NORMAL.0 as i32);
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));

        if state.entries.is_empty() {
            SetTextColor(hdc, COLORREF(colors.dim));
            let empty_hint = if state.favorites_only {
                "（⭐ 收藏为空 · Ctrl+F 返回全部）"
            } else if state.query.is_empty() {
                "（历史为空）"
            } else {
                "（无匹配条目）"
            };
            draw_line(
                hdc,
                empty_hint,
                padding,
                padding,
                width - padding * 2,
                row_h,
            );
        }

        // 收藏匹配集合在绘制循环外取一次，避免每行文件 IO
        let favs = shurufa_options::favorites::load_favorites();

        for (i, entry) in state.entries.iter().enumerate().take(MAX_ROWS) {
            let top = padding + i as i32 * row_h;
            if i == state.selected {
                let hl = CreateSolidBrush(COLORREF(colors.row_hl));
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
            SetTextColor(hdc, COLORREF(colors.label));
            draw_line(
                hdc,
                &format!("{}", i + 1),
                padding,
                top,
                scale(18, dpi),
                row_h,
            );

            // 类型标记 + 置顶星标；已收藏额外追加 ☆ 角标
            SelectObject(hdc, HGDIOBJ(small.0));
            SetTextColor(hdc, COLORREF(colors.dim));
            let is_fav = is_favorited(&favs, entry);
            let base_tag = match (entry.kind, entry.pinned) {
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
            let tag: String = if is_fav {
                format!("{base_tag}☆")
            } else {
                base_tag.to_owned()
            };
            draw_line(
                hdc,
                &tag,
                padding + scale(20, dpi),
                top,
                scale(30, dpi),
                row_h,
            );

            // 内容预览
            SelectObject(hdc, HGDIOBJ(font.0));
            SetTextColor(hdc, COLORREF(colors.text));
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
        SetTextColor(hdc, COLORREF(colors.dim));
        let footer = if state.favorites_only {
            if state.query.is_empty() {
                "⭐ 收藏 · Ctrl+F 回全部 · 右键 收藏/转发 · Esc 关闭".to_owned()
            } else {
                format!(
                    "⭐ 收藏 · 筛选：{} · Ctrl+F 回全部 · Esc 关闭",
                    crate::single_line_preview(&state.query, 14)
                )
            }
        } else if state.query.is_empty() {
            "直接键入筛选 · 回车/数字 粘贴 · Ctrl+F ⭐收藏 · 右键 菜单 · Esc 关闭".to_owned()
        } else {
            format!(
                "筛选：{} · 退格修改 · 回车/数字 粘贴 · Ctrl+F ⭐收藏 · Esc 关闭",
                crate::single_line_preview(&state.query, 14)
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
