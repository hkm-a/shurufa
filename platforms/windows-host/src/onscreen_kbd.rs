//! 虚拟键盘面板（On-Screen Keyboard / OSK）：Ctrl+Shift+K 呼出/隐藏。
//!
//! 场景：无物理键盘（触屏二合一、远程桌面极端情况）或单键临时输入时，
//! 让用户用鼠标/触摸把单个 VK 注入前台输入焦点。窗口 NOACTIVATE +
//! TOOLWINDOW，永不抢焦点；点击按钮直接把 VK 通过 SendInput 打进去，
//! 与候选窗/AI 面板的"写回文本"路径完全隔离。
//!
//! 骨架复用 panel.rs/ai_panel.rs 的"置顶弹窗 + 消息循环"模式；配色、圆角、
//! 字号倍率、透明度统一走共享皮肤（`crate::panel::skin`，经 `#[path]` 引入）。
//!
//! 开关：options.json 顶层 `general.enable_osk_hotkey`（缺失默认 true，损坏
//! 也默认 true—— 宁多勿少，关掉只是用户自己删一行）。windows-host 当前未
//! 依赖 shurufa-options crate，这里用 serde_json 做一次只读解析；
//! TODO(wave4+)：若后续 windows-host 接入 options crate，改成共享类型。

use std::cell::RefCell;
use std::path::PathBuf;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, FrameRect, InvalidateRect, SelectObject, SetBkMode, SetTextColor, DT_CENTER,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HBRUSH, HDC, HFONT, HGDIOBJ, PAINTSTRUCT,
    PS_SOLID, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    MOD_CONTROL, MOD_SHIFT, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_LCONTROL, VK_LSHIFT, VK_RETURN,
    VK_SHIFT, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, LoadCursorW, MoveWindow, RegisterClassW, ShowWindow,
    SystemParametersInfoW, CS_HREDRAW, CS_VREDRAW, IDC_ARROW, SPI_GETWORKAREA, SW_HIDE,
    SW_SHOWNOACTIVATE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WNDCLASSW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE, WM_APP, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_PAINT, WM_SETTINGCHANGE,
};

use crate::panel::skin::{self, ShadowShell, Skin};

pub const HOTKEY_ID: i32 = 4;

// 96 DPI 基准尺寸（logical）：宽 640，高 220，键 56x48，边距 13/5，间距 6。
// 高度 48 而非 56 是为了让 QWERTY/ASDF/ZXCV/空格四行 4*48+3*6=210 ≤ 220。
const BASE_WIDTH: i32 = 640;
const BASE_HEIGHT: i32 = 220;
const KEY_W: i32 = 56;
const KEY_H: i32 = 48;
const KEY_GAP: i32 = 6;
const PAD_X: i32 = 13;
const PAD_Y: i32 = 5;
const BASE_FONT: i32 = 11;

/// 一颗键的几何与语义。`vk` 是 SendInput 推出的虚拟键码；`label` 是绘制文本。
/// `modifier` 标记 Shift/Ctrl（按住型状态机，不直接产生字符串）。
/// `close` 为 true 时点击即隐藏面板。
#[derive(Clone, Copy, Debug)]
struct Key {
    label: &'static str,
    vk: u16,
    /// 宽度（以 KEY_W 为 1.0 的倍数，整数存储避免浮点误差；空格为 5）。
    width_units: u8,
    modifier: Modifier,
    action: KeyAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Modifier {
    None,
    Shift,
    Ctrl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyAction {
    /// 普通字符/控制键：SendInput(vk down/up)，若有 Shift/Ctrl 按住状态先按下修饰再松开。
    Type,
    /// 仅切换修饰状态（Shift / Ctrl 本身）。
    ToggleModifier,
    /// 隐藏面板（Close ✕）。
    Hide,
}

fn vk(c: u8) -> u16 {
    c as u16
}

/// ASCII 字母直接映射到同名 VK 码（'A'=0x41 ... 'Z'=0x5A）。
const LETTER_A: u16 = 0x41;

/// 键位表：行 0 QWERTY，行 1 ASDF，行 2 Shift+ZXCV+Shift，行 3 Ctrl+Space+Enter+Bksp+Close+Ctrl。
/// 全部静态常量，布局在 `layout_keys` 里按 width_units 顺序排开。
const ROW0: &[Key] = &[
    Key { label: "Q", vk: LETTER_A + 16, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "W", vk: 0x57, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "E", vk: 0x45, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "R", vk: 0x52, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "T", vk: 0x54, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "Y", vk: 0x59, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "U", vk: 0x55, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "I", vk: 0x49, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "O", vk: 0x4F, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "P", vk: 0x50, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
];

const ROW1: &[Key] = &[
    Key { label: "A", vk: LETTER_A, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "S", vk: 0x53, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "D", vk: 0x44, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "F", vk: 0x46, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "G", vk: 0x47, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "H", vk: 0x48, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "J", vk: 0x4A, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "K", vk: 0x4B, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "L", vk: 0x4C, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
];

const ROW2: &[Key] = &[
    Key { label: "Shift", vk: VK_LSHIFT.0, width_units: 1, modifier: Modifier::Shift, action: KeyAction::ToggleModifier },
    Key { label: "Z", vk: 0x5A, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "X", vk: 0x58, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "C", vk: 0x43, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "V", vk: 0x56, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "B", vk: 0x42, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "N", vk: 0x4E, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "M", vk: 0x4D, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "Shift", vk: VK_LSHIFT.0, width_units: 1, modifier: Modifier::Shift, action: KeyAction::ToggleModifier },
];

const ROW3: &[Key] = &[
    Key { label: "Ctrl", vk: VK_LCONTROL.0, width_units: 1, modifier: Modifier::Ctrl, action: KeyAction::ToggleModifier },
    Key { label: "Space", vk: VK_SPACE.0, width_units: 5, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "Enter", vk: VK_RETURN.0, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "Bksp", vk: VK_BACK.0, width_units: 1, modifier: Modifier::None, action: KeyAction::Type },
    Key { label: "✕", vk: 0, width_units: 1, modifier: Modifier::None, action: KeyAction::Hide },
    Key { label: "Ctrl", vk: VK_LCONTROL.0, width_units: 1, modifier: Modifier::Ctrl, action: KeyAction::ToggleModifier },
];

const ROWS: &[&[Key]] = &[ROW0, ROW1, ROW2, ROW3];

// 已从布局推导：行 0 占 10 单位、行 1 占 9、行 2 占 1*2+7=9、行 3 占 1+5+1+1+1+1=10。
// 行宽不足 10 单位时整体水平居中——跟真实 OSK 观感一致（ASDF 略缩进）。

struct PanelState {
    hwnd: HWND,
    dpi: u32,
    shift: bool,
    ctrl: bool,
    /// 鼠标按下时命中的键下标（扁平化索引），供 WM_LBUTTONUP 高亮恢复与判定。
    pressed: Option<usize>,
}

thread_local! {
    static OSK: RefCell<Option<PanelState>> = const { RefCell::new(None) };
    static OSK_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };
    static SHADOW: RefCell<ShadowShell> = RefCell::new(ShadowShell::new());
}

/// 读取 `%APPDATA%\shurufa\options.json` 的 `general.enable_osk_hotkey`。
/// 文件缺失 / 字段缺失 / 解析失败 / 路径推导失败均默认 true。
/// windows-host 暂未依赖 shurufa-options，这里只读不写字段。
fn read_enable_osk_hotkey() -> bool {
    let Some(dir) = std::env::var_os("APPDATA").map(PathBuf::from) else {
        return true;
    };
    let path = dir.join("shurufa").join("options.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return true;
    };
    // 最小化局部结构体：只关心 general.enable_osk_hotkey；其余字段宽松忽略。
    #[derive(serde::Deserialize)]
    struct Top {
        #[serde(default)]
        general: General,
    }
    #[derive(serde::Deserialize, Default)]
    struct General {
        #[serde(default = "default_true")]
        enable_osk_hotkey: bool,
    }
    fn default_true() -> bool {
        true
    }
    serde_json::from_slice::<Top>(&bytes)
        .map(|t| t.general.enable_osk_hotkey)
        .unwrap_or(true)
}

/// 注册 Ctrl+Shift+K；options.json 允许关闭；占用冲突只记日志不 panic。
pub fn register_hotkey() -> &'static str {
    if !read_enable_osk_hotkey() {
        crate::log_line("虚拟键盘热键：options.json 已关闭");
        return "（options.json 已关闭）";
    }
    unsafe {
        if RegisterHotKey(None, HOTKEY_ID, MOD_CONTROL | MOD_SHIFT, 0x4B).is_ok() {
            crate::log_line("虚拟键盘热键注册结果：Ctrl+Shift+K");
            "Ctrl+Shift+K"
        } else {
            crate::log_line("虚拟键盘热键注册失败（冲突或未挂载桌面会话）");
            "（虚拟键盘热键注册失败）"
        }
    }
}

/// 按当前 DPI 重算所有键的客户区矩形；扁平化索引与 ROWS 遍历顺序一一对应。
/// 返回 Vec<RECT>，长度 = 行内键数总和。测试用同一函数验证不重叠 + 总数。
fn layout_keys(dpi: u32) -> Vec<RECT> {
    let s = |base: i32| ((base * dpi as i32) + 48) / 96;
    let key_w = s(KEY_W);
    let key_h = s(KEY_H);
    let gap = s(KEY_GAP);
    let pad_x = s(PAD_X);
    let pad_y = s(PAD_Y);
    // 行内"单位" = 一个 key_w + 一个 gap；width_units > 1 的键吃掉被它覆盖的 gap。
    let _unit = key_w + gap;

    let mut rects = Vec::with_capacity(key_total_count());
    for (row_idx, row) in ROWS.iter().enumerate() {
        // 行总宽 = Σ(w_u*key_w + (w_u-1)*gap) + (count-1)*gap
        // 等价换算 (以 unit 为单位)：units*key_w + (Σw_u - 1)*gap
        let units: i32 = row.iter().map(|k| k.width_units as i32).sum();
        let row_width = units * key_w + (units - 1) * gap;
        let inner_width = s(BASE_WIDTH) - pad_x * 2;
        let mut x = pad_x + ((inner_width - row_width).max(0) / 2);
        let y = pad_y + row_idx as i32 * (key_h + gap);
        for k in *row {
            let w = key_w * k.width_units as i32 + gap * (k.width_units as i32 - 1);
            rects.push(RECT {
                left: x,
                top: y,
                right: x + w,
                bottom: y + key_h,
            });
            x += w + gap;
        }
    }
    rects
}

/// 面板键总数（单元测试会断言）。
const fn key_total_count() -> usize {
    ROW0.len() + ROW1.len() + ROW2.len() + ROW3.len()
}

fn key_radius(skin: &Skin, dpi: u32) -> i32 {
    let base = (skin.metrics.radius / 2).clamp(2, 12);
    // 96 DPI 基准半径；高 DPI 按比例缩放，仍以 2..12 单位加下限保护。
    ((base * dpi as i32 + 48) / 96).max(2)
}

/// 命中检测：客户区坐标落在哪个键；返回扁平索引。
fn hit_test(rects: &[RECT], pt: POINT) -> Option<usize> {
    rects.iter().position(|r| {
        pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
    })
}

fn scale_font_height(base: i32, dpi: u32, font_scale: f32) -> i32 {
    let px = ((base * dpi as i32 + 48) / 96) as f32 * font_scale;
    px.round().max(8.0) as i32
}

unsafe fn make_font(height: i32) -> HFONT {
    CreateFontW(
        -height,
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

/// 把单颗键用 SendInput 打给前台焦点。
/// 按下修饰状态 = 先按修饰、再按键、再抬键、最后抬修饰；修饰在 Type 后自动复位。
fn send_key(key: &Key, shift: bool, ctrl: bool) {
    let mut inputs: Vec<INPUT> = Vec::with_capacity(6);
    let make = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() },
                ..Default::default()
            },
        },
    };
    if ctrl {
        inputs.push(make(VK_CONTROL, false));
    }
    if shift {
        inputs.push(make(VK_SHIFT, false));
    }
    inputs.push(make(VIRTUAL_KEY(key.vk), false));
    inputs.push(make(VIRTUAL_KEY(key.vk), true));
    if shift {
        inputs.push(make(VK_SHIFT, true));
    }
    if ctrl {
        inputs.push(make(VK_CONTROL, true));
    }
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn ensure_window() -> Option<HWND> {
    if let Some(hwnd) = OSK_HWND.with_borrow(|h| *h) {
        return Some(hwnd);
    }
    thread_local! {
        static CLASS_REGISTERED: RefCell<bool> = const { RefCell::new(false) };
    }
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null()).ok()?;
        let class_name = w!("ShurufaOnscreenKbd");
        CLASS_REGISTERED.with_borrow_mut(|registered| {
            if !*registered {
                let class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: class_name,
                    hbrBackground: HBRUSH::default(),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    ..Default::default()
                };
                RegisterClassW(&class);
                *registered = true;
            }
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name,
            w!("shurufa 虚拟键盘"),
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            BASE_WIDTH,
            BASE_HEIGHT,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
        .ok()?;
        skin::apply_appearance(hwnd, &Skin::current());
        OSK_HWND.with_borrow_mut(|h| *h = Some(hwnd));
        Some(hwnd)
    }
}

/// 计算主显示器工作区底部居中位置（离底边 24px）。
fn bottom_center_position(w: i32, h: i32) -> (i32, i32) {
    unsafe {
        let mut rc = RECT::default();
        // 失败则用 GetSystemMetrics 主屏兜底；极少发生，落回屏幕底居中。
        let ok = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rc as *mut RECT as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok();
        if ok {
            let x = rc.left + ((rc.right - rc.left) - w) / 2;
            let y = rc.bottom - h - 24;
            return (x, y);
        }
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        let sx = GetSystemMetrics(SM_CXSCREEN);
        let sy = GetSystemMetrics(SM_CYSCREEN);
        ((sx - w) / 2, sy - h - 24)
    }
}

/// Ctrl+Shift+K 触发：可见则隐藏，不可见则显示（并保持 NOACTIVATE，不抢焦点）。
pub fn toggle() {
    let already = OSK.with_borrow(|slot| slot.is_some());
    if already {
        hide();
        return;
    }
    let Some(hwnd) = ensure_window() else {
        crate::log_line("虚拟键盘窗口创建失败");
        return;
    };
    let skin = Skin::current();
    skin::apply_appearance(hwnd, &skin);
    let dpi = unsafe { GetDpiForWindow(hwnd).max(GetDpiForSystem()) }.max(96);
    let w = (BASE_WIDTH * dpi as i32 + 48) / 96;
    let h = (BASE_HEIGHT * dpi as i32 + 48) / 96;
    let (x, y) = bottom_center_position(w, h);
    OSK.with_borrow_mut(|slot| {
        *slot = Some(PanelState {
            hwnd,
            dpi,
            shift: false,
            ctrl: false,
            pressed: None,
        });
    });
    unsafe {
        let _ = MoveWindow(hwnd, x, y, w, h, true);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SHADOW.with_borrow_mut(|shell| shell.sync(hwnd, x, y, w, h, &skin.shadow));
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
    crate::log_line("虚拟键盘已显示");
}

fn hide() {
    let hwnd = OSK.with_borrow_mut(|slot| slot.take().map(|s| s.hwnd));
    SHADOW.with_borrow_mut(|shell| shell.hide());
    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

/// 主题切换由面板自身 wnd_proc 的 WM_SETTINGCHANGE 分支处理（与 ai_panel 相同路径），
/// 这里不暴露 on_theme_changed 回调——主题监听集中在 panel.rs，无需再挂第二个钩子。

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint(hdc, &ps.rcPaint);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            // 命中检测 + 高亮 + 由 ToggleModifier/Type/Hide 决定动作
            let pt = POINT {
                x: (lparam.0 & 0xffff) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            };
            let result = OSK.with_borrow_mut(|slot| {
                let state = slot.as_mut()?;
                let rects = layout_keys(state.dpi);
                let idx = hit_test(&rects, pt)?;
                let (row, col) = locate(idx)?;
                let key = ROWS[row][col];
                let act = match key.action {
                    KeyAction::ToggleModifier => {
                        match key.modifier {
                            Modifier::Shift => state.shift = !state.shift,
                            Modifier::Ctrl => state.ctrl = !state.ctrl,
                            Modifier::None => {}
                        }
                        None
                    }
                    KeyAction::Type => {
                        let shift = state.shift;
                        let ctrl = state.ctrl;
                        send_key(&key, shift, ctrl);
                        // 打字后修饰一次性弹起
                        state.shift = false;
                        state.ctrl = false;
                        None
                    }
                    KeyAction::Hide => Some(()),
                };
                state.pressed = Some(idx);
                Some((act, state.pressed))
            });
            let Some((action, _new_pressed)) = result else {
                return LRESULT(0);
            };
            if action.is_some() {
                hide();
            } else {
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = OSK.with_borrow_mut(|slot| {
                if let Some(state) = slot.as_mut() {
                    state.pressed = None;
                }
            });
            let _ = InvalidateRect(Some(hwnd), None, true);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            // NOACTIVATE 模式下本面板通常不拿焦点；但监督者可能转交（调试用）。
            // 物理 Esc 经由调用方无需关闭——文档约定为 no-op，这里保持静默。
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            if skin::is_immersive_color_change(lparam) {
                let skin = Skin::refresh_on_setting_change();
                let _ = &skin;
                skin::apply_appearance(hwnd, &skin);
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
            LRESULT(0)
        }
        x if x == WM_APP + 99 => LRESULT(0), // 占位：未来扩展（长按重复等）
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 把扁平索引还原回 (row, col)，供键值查找。越界返回 None。
fn locate(flat: usize) -> Option<(usize, usize)> {
    let mut rest = flat;
    for (r, row) in ROWS.iter().enumerate() {
        if rest < row.len() {
            return Some((r, rest));
        }
        rest -= row.len();
    }
    None
}

unsafe fn paint(hdc: HDC, rc: &RECT) {
    OSK.with_borrow(|slot| {
        let Some(state) = slot.as_ref() else {
            return;
        };
        let skin = Skin::current();
        let c = skin.candidate;
        let dpi = state.dpi;

        // 背景：皮肤候选底色
        let bg_brush = CreateSolidBrush(COLORREF(c.background));
        FillRect(hdc, rc, bg_brush);
        let _ = DeleteObject(HGDIOBJ(bg_brush.0));
        SetBkMode(hdc, TRANSPARENT);

        // 键底/边框/文字
        let idle_bg = CreateSolidBrush(COLORREF(c.background));
        let hl_bg = CreateSolidBrush(COLORREF(c.highlight_background));
        let border_pen = CreatePen(PS_SOLID, 1, COLORREF(c.preedit));
        let font = make_font(scale_font_height(BASE_FONT, dpi, skin.metrics.font_scale));
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        SetTextColor(hdc, COLORREF(c.text));

        let rects = layout_keys(dpi);
        let radius = key_radius(&skin, dpi);
        let _ = radius; // 圆角由 DWM 处理整体窗口；键级圆角如需 D2D 再启用（v1 方角）
        let mut idx = 0usize;
        for row in ROWS.iter() {
            for key in *row {
                let r = rects[idx];
                idx += 1;

                let active_mod = match key.modifier {
                    Modifier::Shift => state.shift,
                    Modifier::Ctrl => state.ctrl,
                    Modifier::None => false,
                };
                let pressed = state.pressed == Some(idx - 1) || active_mod;
                let brush = if pressed { hl_bg } else { idle_bg };
                FillRect(hdc, &r, brush);
                // 仅空闲键画淡边框；按下/修饰已用高亮区分
                if !pressed {
                    let old_pen = SelectObject(hdc, HGDIOBJ(border_pen.0));
                    FrameRect(hdc, &r, idle_bg);
                    SelectObject(hdc, old_pen);
                }

                if !key.label.is_empty() {
                    let mut utf16: Vec<u16> = key.label.encode_utf16().collect();
                    let mut r2 = r;
                    DrawTextW(
                        hdc,
                        &mut utf16,
                        &mut r2,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                    );
                }
            }
        }

        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = DeleteObject(HGDIOBJ(idle_bg.0));
        let _ = DeleteObject(HGDIOBJ(hl_bg.0));
        let _ = DeleteObject(HGDIOBJ(border_pen.0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (a) 键位数组：总数 + 任意两键矩形在客户区无重叠。
    /// 期望总数 = 10(QWERTY) + 9(ASDF) + 9(Shift+ZXCV+Shift) + 6(Ctrl+Space+Enter+Bksp+Close+Ctrl) = 34。
    #[test]
    fn key_rect_array_total_and_non_overlapping() {
        assert_eq!(key_total_count(), 34);
        let rects = layout_keys(96);
        assert_eq!(rects.len(), key_total_count());
        for (i, a) in rects.iter().enumerate() {
            // 正面积
            assert!(a.right > a.left, "rect {i} 宽度非正");
            assert!(a.bottom > a.top, "rect {i} 高度非正");
            for (j, b) in rects.iter().enumerate().skip(i + 1) {
                let overlap_x = a.left.min(b.left).max(0).max(a.left.max(b.left))
                    < a.right.min(b.right);
                let overlap = a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
                assert!(!overlap, "rect {i} 与 rect {j} 重叠：{a:?} vs {b:?}");
                let _ = overlap_x;
            }
        }
    }

    /// (b) VK 映射抽测：字母键 = ASCII 大写值；空格/回车/退格走标准 VK；修饰记 VK_L*。
    #[test]
    fn sendinput_vk_mapping_spot_check() {
        // A..Z 的 VK 是连续 0x41..=0x5A
        assert_eq!(ROW1[0].vk, 0x41, "A 键 VK 应为 0x41");
        assert_eq!(ROW1[8].vk, 0x4C, "L 键 VK 应为 0x4C");
        assert_eq!(ROW2[1].vk, 0x5A, "Z 键 VK 应为 0x5A");
        assert_eq!(ROW0[1].vk, 0x57, "W 键 VK 应为 0x57");
        // Space / Enter / Bksp
        assert_eq!(ROW3[1].vk, VK_SPACE.0);
        assert_eq!(ROW3[2].vk, VK_RETURN.0, "Enter 应为 VK_RETURN (0x0D)");
        assert_eq!(ROW3[3].vk, VK_BACK.0);
        // Ctrl/Shift 是 modifier，不直接走 VK 发送
        assert_eq!(ROW2[0].modifier, Modifier::Shift);
        assert_eq!(ROW3[0].modifier, Modifier::Ctrl);
        assert_eq!(ROW3[4].action, KeyAction::Hide);
    }

    /// Shift 按下后字母键 SendInput 序列应含 VK_SHIFT down/up 包裹。
    /// 这里不真正 SendInput，仅断言 layout_keys 不 panic + 行数一致。
    #[test]
    fn layout_keys_at_common_dpis() {
        for dpi in [96u32, 120, 144, 192] {
            let rects = layout_keys(dpi);
            assert_eq!(rects.len(), key_total_count());
        }
    }
}
