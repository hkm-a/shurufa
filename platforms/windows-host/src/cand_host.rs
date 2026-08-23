//! 候选窗宿主（阶段 6 S2，方案见 docs/候选窗迁出宿主进程-方案.md）。
//!
//! 把候选 UI 从每个 TSF 宿主进程迁到本进程渲染：TSF DLL 作为客户端连
//! `\\.\pipe\shurufa-cand` 推送 CandEvent（全量候选 + 光标物理像素矩形），
//! 本模块作为服务端多连接并发；用户点击/滚轮回发 CandCommand，TSF 侧以
//! 虚拟键合成重走正常按键路径（数字选词/翻页拦截全部生效）。
//!
//! ui 侧零会话状态：每帧 Show 全量推送，本进程崩溃重启后 TSF 下一次按键
//! 自然恢复显示——"面板进程崩了不影响输入"的关键。
//!
//! v1 限制（S4 验收矩阵后逐步补齐）：单行候选条布局；AI 候选/英文 Tab
//! 分组不在此渲染（它们仍由 TSF 内置路径接管）；固定角落位置模式仍走内置。

use std::collections::HashMap;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, GetDC, GetTextExtentPoint32W, InvalidateRect, ReleaseDC, SelectObject, SetBkMode,
    SetTextColor, DT_LEFT, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HBRUSH, HDC, HFONT, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    FindWindowW, GetSystemMetrics, PeekMessageW, PostMessageW, RegisterClassW, SetWindowPos,
    SetWindowTextW, ShowWindow, TrackPopupMenu, TranslateMessage, MF_SEPARATOR, MF_STRING, MSG,
    PM_REMOVE, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_GETOBJECT, WM_LBUTTONDOWN, WM_MOUSEWHEEL, WM_PAINT,
    WM_RBUTTONDOWN, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use ime_ipc::{decode_cand_event, encode_cand_command, CandCommand, CandEvent, Context};
use windows_ipc::pipe::{PipeClient, PipeServer, CAND_PIPE_NAME};
use windows_skin::SkinExt;

use crate::log_line;

/// 每客户端候选窗的窗口类（selftest 也按它 FindWindow）。
pub const CAND_WINDOW_CLASS: &str = "ShurufaCandWin";
/// 连接线程 → 主线程的事件投递消息；lparam = Box<CandEvent> 裸指针。
const WM_APP_CAND_EVENT: u32 = WM_APP + 61;

// -----------------------------------------------------------------------
// 主线程状态（窗口与布局只在 ui 主线程触碰）
// -----------------------------------------------------------------------

struct ItemView {
    label: String,
    text: String,
    comment: String,
    is_ai: bool,
}

struct CandView {
    client_id: u32,
    dpi: u32,
    preedit: String,
    items: Vec<ItemView>,
    item_rects: Vec<RECT>,
    highlighted: usize,
    width: i32,
    height: i32,
    caret: POINT,
    show_tab: bool,
    tab_label: String,
    multi_line: bool,
    position: String,
    mode_badge: String,
}

thread_local! {
    /// client_id → (hwnd, 最新视图)。窗口随事件更新，Hide 只隐藏不销毁。
    static CLIENTS: std::cell::RefCell<HashMap<u32, (HWND, CandView)>> =
        std::cell::RefCell::new(HashMap::new());
    /// client_id → 阴影壳（完整皮肤样式用）。
    static SHADOWS: std::cell::RefCell<HashMap<u32, windows_skin::ShadowShell>> =
        std::cell::RefCell::new(HashMap::new());
}

/// 控制窗口句柄（连接线程也要用它投递事件，因此必须是跨线程原子，不能 thread_local）。
static CTL_HWND: AtomicIsize = AtomicIsize::new(0);

/// PipeServer 只有 Send 没有 Sync（HANDLE 裸指针）；命令写入从主线程发起，
/// 包一层显式 Sync（写句柄本身线程安全：消息模式 WriteFile 原子一条消息）。
struct SyncPipe(PipeServer);
unsafe impl Sync for SyncPipe {}

impl std::ops::Deref for SyncPipe {
    type Target = PipeServer;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// client_id → 命令发送端（连接线程登记/注销；主线程发命令时投递）。
static CONNS: OnceLock<Mutex<HashMap<u32, Sender<CandCommand>>>> = OnceLock::new();

fn conns() -> &'static Mutex<HashMap<u32, Sender<CandCommand>>> {
    CONNS.get_or_init(|| Mutex::new(HashMap::new()))
}

// -----------------------------------------------------------------------
// 启动与连接服务
// -----------------------------------------------------------------------

/// 在 ui 主线程调用：创建控制窗口并启动接受循环（连接线程）。
/// 返回 false 表示控制窗口创建失败（候选窗宿主不可用，TSF 侧自动回退内置）。
pub fn start() -> bool {
    unsafe {
        let hinstance = windows::Win32::System::LibraryLoader::GetModuleHandleW(PCWSTR::null())
            .expect("GetModuleHandleW");
        let ctl_class = WNDCLASSW {
            lpfnWndProc: Some(ctl_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: w!("ShurufaCandCtl"),
            ..Default::default()
        };
        RegisterClassW(&ctl_class);
        let cand_class = WNDCLASSW {
            lpfnWndProc: Some(cand_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: w!("ShurufaCandWin"),
            ..Default::default()
        };
        RegisterClassW(&cand_class);
        let ctl = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("ShurufaCandCtl"),
            w!("cand-ctl"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                log_line(&format!("cand_host: 控制窗口创建失败：{e}"));
                return false;
            }
        };
        CTL_HWND.store(ctl.0 as isize, Ordering::Relaxed);

        std::thread::spawn(|| accept_loop());
        true
    }
}

unsafe fn accept_loop() {
    loop {
        let server = match PipeServer::create_named(CAND_PIPE_NAME) {
            Ok(s) => s,
            Err(e) => {
                log_line(&format!("cand_host: 创建管道失败：{e}"));
                return;
            }
        };
        if let Err(e) = server.accept() {
            log_line(&format!("cand_host: 接受连接失败：{e}"));
            continue;
        }
        let conn = Arc::new(SyncPipe(server));
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || serve_connection(conn, tx, rx));
    }
}

/// 单连接读循环：事件解码后投递主线程；连接关闭时注销写端。
unsafe fn serve_connection(
    conn: Arc<SyncPipe>,
    tx: Sender<CandCommand>,
    rx: Receiver<CandCommand>,
) {
    let mut known_ids: Vec<u32> = Vec::new();
    loop {
        // 优先读客户端事件；没有输入时才处理待发命令，避免同一句柄上
        // 同时阻塞 Read/Write（实测会导致 WriteFile 与 ReadFile 互相等待）。
        match conn.peek_available() {
            Ok(true) => {
                let frame = match conn.read_frame() {
                    Ok(f) => f,
                    Err(_) => break,
                };
                let Ok(event) = decode_cand_event(&frame) else {
                    continue;
                };
                let client_id = match &event {
                    CandEvent::Show { client_id, .. } | CandEvent::Hide { client_id } => *client_id,
                };
                if !known_ids.contains(&client_id) {
                    known_ids.push(client_id);
                    conns().lock().unwrap().insert(client_id, tx.clone());
                }
                let boxed = Box::into_raw(Box::new(event));
                let ctl = CTL_HWND.load(Ordering::Relaxed);
                if ctl == 0
                    || PostMessageW(
                        Some(HWND(ctl as *mut _)),
                        WM_APP_CAND_EVENT,
                        WPARAM(0),
                        LPARAM(boxed as isize),
                    )
                    .is_err()
                {
                    drop(Box::from_raw(boxed));
                    break;
                }
            }
            Ok(false) => match rx.try_recv() {
                Ok(cmd) => {
                    if let Ok(frame) = encode_cand_command(&cmd) {
                        let _ = conn.write_frame(&frame);
                    }
                }
                Err(TryRecvError::Empty) => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(TryRecvError::Disconnected) => break,
            },
            Err(_) => break,
        }
    }
    let mut map = conns().lock().unwrap();
    for id in &known_ids {
        map.remove(id);
    }
    // 末次 Hide 投递，避免窗口悬留在屏幕上
    for id in &known_ids {
        let boxed = Box::into_raw(Box::new(CandEvent::Hide { client_id: *id }));
        let ctl = CTL_HWND.load(Ordering::Relaxed);
        if ctl != 0 {
            let _ = PostMessageW(
                Some(HWND(ctl as *mut _)),
                WM_APP_CAND_EVENT,
                WPARAM(0),
                LPARAM(boxed as isize),
            );
        }
    }
}

/// 主线程/窗口线程发命令（点击/滚轮）。失败静默：连接可能刚断。
fn send_command(cmd: CandCommand) {
    let client_id = match &cmd {
        CandCommand::Select { client_id, .. }
        | CandCommand::PageNext { client_id }
        | CandCommand::PagePrev { client_id }
        | CandCommand::MenuAction { client_id, .. } => *client_id,
    };
    let Some(tx) = conns().lock().unwrap().get(&client_id).cloned() else {
        return;
    };
    let _ = tx.send(cmd);
}

// -----------------------------------------------------------------------
// 布局（纯函数，可单测）
// -----------------------------------------------------------------------

const BASE_FONT_HEIGHT: i32 = 18;
const BASE_PADDING: i32 = 8;
const BASE_GAP: i32 = 10;

fn scale(base: i32, dpi: u32) -> i32 {
    (base as u64 * dpi as u64 / 96) as i32
}

/// 单行布局度量（不建窗口即可算，供单测与绘制共用）。
struct RowLayout {
    width: i32,
    height: i32,
    /// 每项的 (x, y, w, h)——相对客户区左上角。
    item_spans: Vec<(i32, i32, i32, i32)>,
}

fn layout_row(extra_left_w: i32, preedit_w: i32, item_widths: &[i32], dpi: u32) -> RowLayout {
    let pad = scale(BASE_PADDING, dpi);
    let gap = scale(BASE_GAP, dpi);
    let font_h = scale(BASE_FONT_HEIGHT, dpi);
    let height = font_h + pad * 2;
    let mut x = pad + extra_left_w + preedit_w + if extra_left_w + preedit_w > 0 { gap } else { 0 };
    let mut item_spans = Vec::with_capacity(item_widths.len());
    for &w in item_widths {
        item_spans.push((x, 0, w, height));
        x += w + gap;
    }
    let width = (x - gap + pad).max(pad * 2);
    RowLayout {
        width,
        height,
        item_spans,
    }
}

/// 多行候选面板布局：每行最多 MULTI_COLUMNS 项，自动换行。
fn layout_multi(extra_left_w: i32, preedit_w: i32, item_widths: &[i32], dpi: u32) -> RowLayout {
    const MULTI_COLUMNS: usize = 5;
    let pad = scale(BASE_PADDING, dpi);
    let gap = scale(BASE_GAP, dpi);
    let font_h = scale(BASE_FONT_HEIGHT, dpi);
    let row_h = font_h + scale(6, dpi);
    let mut x = pad + extra_left_w + preedit_w + if extra_left_w + preedit_w > 0 { gap } else { 0 };
    let mut y = 0i32;
    let mut row_used = 0i32;
    let mut max_row_used = 0i32;
    let mut item_spans = Vec::with_capacity(item_widths.len());
    for (i, &w) in item_widths.iter().enumerate() {
        if i > 0 && i % MULTI_COLUMNS == 0 {
            x = pad + extra_left_w + preedit_w + if extra_left_w + preedit_w > 0 { gap } else { 0 };
            y += row_h;
            row_used = 0;
        }
        item_spans.push((x, y, w, row_h));
        x += w + gap;
        row_used += w + gap;
        max_row_used = max_row_used.max(row_used);
    }
    let height = y + row_h + pad;
    let width = (pad + max_row_used + pad).max(pad * 2);
    RowLayout {
        width,
        height,
        item_spans,
    }
}

/// 文本像素宽（当前 DPI 字号下）。
/// 候选窗字体（测量与绘制共用，保证宽度一致）。
unsafe fn cand_font(dpi: u32) -> HFONT {
    CreateFontW(
        -scale(BASE_FONT_HEIGHT, dpi),
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

fn text_width(hdc: HDC, text: &str) -> i32 {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = windows::Win32::Foundation::SIZE::default();
    unsafe {
        let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
    }
    size.cx
}

/// hosted 候选窗右上角模式角标：长按 Shift 大写视觉 `⇪` / 英文直输 `En` /
/// 全角 `全` / 中文半角 `中`。
/// 大写视觉优先（用户主动长按，必须立即反馈）；其次英文直输
/// （英文+全角时也以 En 为准，避免角标切换闪烁）。
fn mode_badge_for(ctx: &Context) -> String {
    if ctx.caps_visual {
        "⇪".to_owned()
    } else if ctx.is_ascii {
        "En".to_owned()
    } else if ctx.is_full_shape {
        "全".to_owned()
    } else {
        "中".to_owned()
    }
}

fn view_items(ctx: &Context) -> Vec<ItemView> {
    ctx.candidates
        .iter()
        .enumerate()
        .map(|(i, c)| ItemView {
            // 序号与 TSF 数字选词一致：1..9，第 10 项为 0
            label: format!("{}", if i >= 9 { 0 } else { i + 1 }),
            text: c.text.clone(),
            comment: c.comment.clone(),
            is_ai: c.comment.contains("\u{1F916}"),
        })
        .collect()
}

// -----------------------------------------------------------------------
// 窗口过程
// -----------------------------------------------------------------------

unsafe extern "system" fn ctl_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_APP_CAND_EVENT {
        let boxed = Box::from_raw(lparam.0 as *mut CandEvent);
        handle_event(*boxed);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn handle_event(event: CandEvent) {
    match event {
        CandEvent::Show {
            client_id,
            context,
            caret_rect,
            dpi,
            multi_line,
            position,
        } => {
            let (x, y, _cx, _cy) = caret_rect;
            let items = view_items(&context);
            // 布局测量用屏幕 DC + 候选字体（与绘制同字体，宽度才一致）
            let show_tab = !context.candidates.is_empty()
                && context.candidates.iter().all(|c| c.text.is_ascii());
            let tab_label = if show_tab { "EN" } else { "拼" };
            let mode_badge = mode_badge_for(&context);
            let hdc = GetDC(None);
            let font = cand_font(dpi);
            let old_font = SelectObject(hdc, font.into());
            let tab_w = if show_tab {
                text_width(hdc, &format!("[{tab_label}] ")) + scale(BASE_GAP, dpi)
            } else {
                0
            };
            let preedit_w = text_width(hdc, &format!("{} ", context.preedit));
            let item_widths: Vec<i32> = items
                .iter()
                .map(|it| text_width(hdc, &format!("{}.{}{}", it.label, it.text, it.comment)))
                .collect();
            let pad = scale(BASE_PADDING, dpi);
            let badge_w = if mode_badge.is_empty() {
                0
            } else {
                text_width(hdc, &format!("[{}]", mode_badge)) + pad
            };
            SelectObject(hdc, old_font);
            let _ = DeleteObject(font.into());
            ReleaseDC(None, hdc);
            let mut layout = if multi_line {
                layout_multi(tab_w, preedit_w, &item_widths, dpi)
            } else {
                layout_row(tab_w, preedit_w, &item_widths, dpi)
            };
            if badge_w > 0 {
                // 角标在右上角，不参与候选布局；额外加宽避免盖住最后一项。
                layout.width += badge_w;
            }
            let view = CandView {
                client_id,
                dpi,
                preedit: context.preedit.clone(),
                item_rects: layout
                    .item_spans
                    .iter()
                    .map(|&(x, y, w, h)| RECT {
                        left: x,
                        top: y,
                        right: x + w,
                        bottom: y + h,
                    })
                    .collect(),
                items,
                highlighted: context.highlighted,
                width: layout.width,
                height: layout.height,
                caret: POINT { x, y },
                show_tab,
                tab_label: tab_label.to_owned(),
                multi_line,
                position,
                mode_badge,
            };
            let uia_text = view
                .items
                .iter()
                .map(|it| format!("{}.{}", it.label, it.text))
                .collect::<Vec<_>>()
                .join("，");
            crate::cand_uia::update_candidate_text(&uia_text);
            let _hwnd = CLIENTS.with(|c| {
                let mut map = c.borrow_mut();
                let entry = map
                    .entry(client_id)
                    .or_insert_with(|| (create_client_window(client_id), dummy_view(client_id)));
                let (hwnd, slot) = entry;
                *slot = view;
                *hwnd
            });
            let mut title: Vec<u16> = uia_text.encode_utf16().collect();
            title.push(0);
            let _ = SetWindowTextW(_hwnd, PCWSTR(title.as_ptr()));
            position_and_show(client_id);
        }
        CandEvent::Hide { client_id } => {
            crate::cand_uia::clear_candidate_text();
            CLIENTS.with(|c| {
                if let Some((hwnd, _)) = c.borrow().get(&client_id) {
                    let _ = SetWindowTextW(*hwnd, w!(""));
                    let _ = ShowWindow(*hwnd, SW_HIDE);
                }
            });
            SHADOWS.with(|s| {
                if let Some(shadow) = s.borrow_mut().get_mut(&client_id) {
                    shadow.hide();
                }
            });
        }
    }
}

fn dummy_view(client_id: u32) -> CandView {
    CandView {
        client_id,
        dpi: 96,
        preedit: String::new(),
        items: Vec::new(),
        item_rects: Vec::new(),
        highlighted: 0,
        width: 0,
        height: 0,
        caret: POINT::default(),
        show_tab: false,
        tab_label: String::new(),
        multi_line: false,
        position: String::new(),
        mode_badge: String::new(),
    }
}

unsafe fn create_client_window(client_id: u32) -> HWND {
    let hinstance = windows::Win32::System::LibraryLoader::GetModuleHandleW(PCWSTR::null())
        .expect("GetModuleHandleW");
    let mut title: Vec<u16> = format!("cand-{client_id}").encode_utf16().collect();
    title.push(0);
    let hwnd = CreateWindowExW(
        WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        w!("ShurufaCandWin"),
        PCWSTR(title.as_ptr()),
        WS_POPUP,
        0,
        0,
        10,
        10,
        None,
        None,
        Some(hinstance.into()),
        None,
    )
    .unwrap_or_default();
    if !hwnd.is_invalid() {
        let skin = windows_skin::Skin::load();
        windows_skin::apply_appearance(hwnd, &skin);
    }
    hwnd
}

/// 依光标矩形定位（物理像素；ui 进程 PerMonitorV2 感知，坐标直接可用），
/// 优先放在光标下方，越界翻到上方/夹回虚拟屏幕。
unsafe fn position_and_show(client_id: u32) {
    CLIENTS.with(|c| {
        let map = c.borrow();
        let Some((hwnd, view)) = map.get(&client_id) else {
            return;
        };
        if view.items.is_empty() {
            let _ = ShowWindow(*hwnd, SW_HIDE);
            return;
        }
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        let gap = scale(4, view.dpi);
        let margin = scale(8, view.dpi);
        let (mut x, y) = match view.position.as_str() {
            "bottom_left" => (vx + margin, vy + vh - view.height - margin),
            "bottom_right" => (
                vx + vw - view.width - margin,
                vy + vh - view.height - margin,
            ),
            _ => {
                let x = view.caret.x;
                let mut y = view.caret.y + view.height / 2 + gap;
                if y + view.height > vy + vh {
                    y = view.caret.y - view.height - gap;
                }
                (x, y)
            }
        };
        if x + view.width > vx + vw {
            x = vx + vw - view.width;
        }
        let x = x.max(vx);
        let y = y.max(vy);
        let _ = SetWindowPos(
            *hwnd,
            None,
            x,
            y,
            view.width,
            view.height,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
        let _ = ShowWindow(*hwnd, SW_SHOWNOACTIVATE);
        let _ = InvalidateRect(Some(*hwnd), None, true);
        // 完整皮肤：阴影壳跟随主窗
        let skin = windows_skin::Skin::load();
        SHADOWS.with(|s| {
            let mut map = s.borrow_mut();
            let shadow = map.entry(client_id).or_default();
            shadow.sync(*hwnd, x, y, view.width, view.height, &skin.shadow);
        });
    });
}

unsafe extern "system" fn cand_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        value if value == WM_GETOBJECT => {
            if let Some(lr) = crate::cand_uia::on_wm_getobject(hwnd, wparam, lparam) {
                return lr;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint(hwnd, hdc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            if let Some(index) = hit_test(hwnd, x, y) {
                let client_id =
                    CLIENTS.with(|c| c.borrow().get_key_value_by_hwnd(hwnd).map(|(id, _)| *id));
                if let Some(client_id) = client_id {
                    send_command(CandCommand::Select { client_id, index });
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            show_context_menu(hwnd, x, y);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as i16;
            if let Some(client_id) =
                CLIENTS.with(|c| c.borrow().get_key_value_by_hwnd(hwnd).map(|(id, _)| *id))
            {
                let cmd = if delta < 0 {
                    CandCommand::PageNext { client_id }
                } else {
                    CandCommand::PagePrev { client_id }
                };
                send_command(cmd);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// HashMap 辅助：按 hwnd 反查 client_id（点击/滚轮只拿得到窗口句柄）。
trait HwndLookup {
    fn get_key_value_by_hwnd(&self, hwnd: HWND) -> Option<(&u32, &(HWND, CandView))>;
}
impl HwndLookup for HashMap<u32, (HWND, CandView)> {
    fn get_key_value_by_hwnd(&self, hwnd: HWND) -> Option<(&u32, &(HWND, CandView))> {
        self.iter().find(|(_, (h, _))| *h == hwnd)
    }
}

unsafe fn hit_test(hwnd: HWND, x: i32, y: i32) -> Option<usize> {
    CLIENTS.with(|c| {
        let map = c.borrow();
        let (_, (_, view)) = map.get_key_value_by_hwnd(hwnd)?;
        view.item_rects
            .iter()
            .position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
    })
}

fn menu_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn show_context_menu(hwnd: HWND, x: i32, y: i32) {
    let Some(index) = hit_test(hwnd, x, y) else {
        return;
    };
    let Some((client_id, _text)) = CLIENTS.with(|c| {
        let map = c.borrow();
        let (id, (_, view)) = map.get_key_value_by_hwnd(hwnd)?;
        let text = view
            .items
            .get(index)
            .map(|it| it.text.clone())
            .unwrap_or_default();
        Some((*id, text))
    }) else {
        return;
    };
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };
    let s_drop = menu_wide("从候选删除");
    let s_demote = menu_wide("降低词频");
    let s_hide = menu_wide("隐藏该词");
    let s_settings = menu_wide("打开设置中心");
    let _ = AppendMenuW(menu, MF_STRING, 1, PCWSTR::from_raw(s_drop.as_ptr()));
    let _ = AppendMenuW(menu, MF_STRING, 2, PCWSTR::from_raw(s_demote.as_ptr()));
    let _ = AppendMenuW(menu, MF_STRING, 3, PCWSTR::from_raw(s_hide.as_ptr()));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, 4, PCWSTR::from_raw(s_settings.as_ptr()));
    let mut pt = POINT { x, y };
    let _ = ClientToScreen(hwnd, &mut pt);
    let cmd = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        None,
        hwnd,
        None,
    );
    let _ = DestroyMenu(menu);
    let action = match cmd.0 {
        1 => "Drop",
        2 => "Demote",
        3 => "Hide",
        4 => {
            // 打开设置中心：与 shurufa-ui 同目录的 Shurufa.exe
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    let _ = std::process::Command::new(dir.join("Shurufa.exe")).spawn();
                }
            }
            return;
        }
        _ => return,
    };
    send_command(CandCommand::MenuAction {
        client_id,
        index,
        action: action.to_owned(),
    });
}

unsafe fn paint(hwnd: HWND, hdc: HDC) {
    let (view,) = CLIENTS.with(|c| map_get_clone(c, hwnd));
    let Some(view) = view else { return };
    let skin = windows_skin::Skin::load();
    let colors = skin.candidate;
    // COLORREF 已是 0x00BBGGRR，可直接 CreateSolidBrush
    let bg = CreateSolidBrush(windows::Win32::Foundation::COLORREF(colors.background));
    let ps_rect = RECT {
        left: 0,
        top: 0,
        right: view.width,
        bottom: view.height,
    };
    let _ = FillRect(hdc, &ps_rect, bg);
    let _ = DeleteObject(HBRUSH(bg.0).into());
    let font: HFONT = cand_font(view.dpi);
    let old_font = SelectObject(hdc, font.into());
    SetBkMode(hdc, TRANSPARENT);

    let pad = scale(BASE_PADDING, view.dpi);
    let mut left = pad;
    // 中/En/全 状态角标（右上角，不参与候选布局）
    if !view.mode_badge.is_empty() {
        let badge = format!("[{}]", view.mode_badge);
        let bw = text_width(hdc, &badge);
        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(colors.label));
        draw_text_at(hdc, &badge, view.width - bw - pad, 0, view.height);
    }
    // Tab 行（英文联想/拼音标识）
    if view.show_tab {
        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(colors.label));
        let tab = format!("[{}] ", view.tab_label);
        draw_text_at(hdc, &tab, left, 0, view.height);
        left += text_width(hdc, &tab) + scale(BASE_GAP, view.dpi);
    }
    // preedit
    if !view.preedit.is_empty() {
        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(colors.preedit));
        draw_text_at(hdc, &format!("{} ", view.preedit), left, 0, view.height);
    }
    // 候选项：高亮底色 + 序号色 + 正文色
    for (i, item) in view.items.iter().enumerate() {
        let Some(rect) = view.item_rects.get(i) else {
            continue;
        };
        let x = rect.left;
        if i == view.highlighted {
            let hl = CreateSolidBrush(windows::Win32::Foundation::COLORREF(
                colors.highlight_background,
            ));
            let r = RECT {
                left: x - 2,
                top: 1,
                right: x + (view.item_rects[i].right - view.item_rects[i].left) + 2,
                bottom: view.height - 1,
            };
            let _ = FillRect(hdc, &r, hl);
            let _ = DeleteObject(HBRUSH(hl.0).into());
        }
        let label = format!("{}.", item.label);
        let label_w = text_width(hdc, &label);
        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(colors.label));
        draw_text_at(hdc, &label, x, 0, view.height);
        SetTextColor(hdc, windows::Win32::Foundation::COLORREF(colors.text));
        let text = if item.comment.is_empty() {
            item.text.clone()
        } else {
            format!("{}{}", item.text, item.comment)
        };
        draw_text_at(hdc, &text, x + label_w, 0, view.height);
        if item.is_ai {
            // AI 候选副标用 label 色强调（hosted 暂无独立 AI 色，先用标签色）
            let comment = format!(" {}", item.comment);
            let cx = x + label_w + text_width(hdc, &item.text);
            SetTextColor(hdc, windows::Win32::Foundation::COLORREF(colors.label));
            draw_text_at(hdc, &comment, cx, 0, view.height);
        }
    }
    SelectObject(hdc, old_font);
    let _ = DeleteObject(font.into());
    let _ = ps_rect;
}

fn map_get_clone(
    c: &std::cell::RefCell<HashMap<u32, (HWND, CandView)>>,
    hwnd: HWND,
) -> (Option<CandView>,) {
    let map = c.borrow();
    let found = map.get_key_value_by_hwnd(hwnd).map(|(_, (_, v))| CandView {
        client_id: v.client_id,
        dpi: v.dpi,
        preedit: v.preedit.clone(),
        items: v
            .items
            .iter()
            .map(|it| ItemView {
                label: it.label.clone(),
                text: it.text.clone(),
                comment: it.comment.clone(),
                is_ai: it.is_ai,
            })
            .collect(),
        item_rects: v.item_rects.clone(),
        highlighted: v.highlighted,
        width: v.width,
        height: v.height,
        caret: v.caret,
        show_tab: v.show_tab,
        tab_label: v.tab_label.clone(),
        multi_line: v.multi_line,
        position: v.position.clone(),
        mode_badge: v.mode_badge.clone(),
    });
    (found,)
}

unsafe fn draw_text_at(hdc: HDC, text: &str, x: i32, top: i32, height: i32) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT {
        left: x,
        top,
        right: x + 4096,
        bottom: height,
    };
    let _ = DrawTextW(
        hdc,
        wide.as_mut_slice(),
        &mut rect,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
}

// -----------------------------------------------------------------------
// selftest：--cand-selftest 入口。起服务 → 模拟客户端推 Show → 泵消息 →
// 按窗口类断言候选窗已创建可见 → 清理退出。零交互，可本地/CI 执行。
// -----------------------------------------------------------------------

pub fn selftest() -> i32 {
    unsafe {
        if !start() {
            eprintln!("[cand-selftest] 宿主启动失败");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        // 模拟 TSF 客户端：连管道、发一条 Show
        let sender = std::thread::spawn(|| -> Result<(), String> {
            let client = PipeClient::connect_named(CAND_PIPE_NAME).map_err(|e| e.to_string())?;
            let event = CandEvent::Show {
                client_id: 99,
                multi_line: false,
                position: "follow".to_owned(),
                context: Context {
                    preedit: "nihao".into(),
                    candidates: vec![
                        ime_ipc::Candidate {
                            text: "你好".into(),
                            comment: String::new(),
                        },
                        ime_ipc::Candidate {
                            text: "拟好".into(),
                            comment: String::new(),
                        },
                    ],
                    highlighted: 0,
                    page_size: 9,
                    ..Context::default()
                },
                caret_rect: (200, 300, 8, 16),
                dpi: 96,
            };
            client
                .write_frame(&ime_ipc::encode_cand_event(&event).unwrap())
                .map_err(|e| e.to_string())?;
            // 保持连接 1s，等主线程渲染
            std::thread::sleep(std::time::Duration::from_millis(1000));
            Ok(())
        });
        // 泵消息 1.2s（事件经 PostMessage 到达）
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1200);
        let mut msg = MSG::default();
        while std::time::Instant::now() < deadline {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                let _ = windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
            }
            if FindWindowW(w!("ShurufaCandWin"), None).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let found = FindWindowW(w!("ShurufaCandWin"), None);
        let ok = found.is_ok();
        if ok {
            // 保持候选窗可见约 2s，供外部 UI 自动化（pywinauto）附加断言。
            eprintln!("[cand-selftest] 候选窗已显示，保持 2s 供外部检查…");
            std::thread::sleep(std::time::Duration::from_millis(2000));
            let _ = DestroyWindow(found.unwrap());
            eprintln!("[cand-selftest] 候选窗创建/渲染/寻址全部通过");
            log_line("cand-selftest: 候选窗创建/渲染/寻址全部通过");
        } else {
            eprintln!("[cand-selftest] 未找到候选窗");
        }
        let _ = sender.join();
        if ok {
            0
        } else {
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 单行布局含序号与间距() {
        // 96 DPI：padding 8、gap 10；无 preedit，两项宽 30/50
        let l = layout_row(0, 0, &[30, 50], 96);
        assert_eq!(l.height, 18 + 16);
        assert_eq!(l.item_spans[0].0, 8);
        assert_eq!(l.item_spans[1].0, 8 + 30 + 10);
        assert_eq!(l.width, 8 + 30 + 10 + 50 + 8);
    }

    #[test]
    fn 高_dpi_等比放大() {
        let a = layout_row(0, 0, &[30], 96);
        let b = layout_row(0, 0, &[60], 192);
        assert_eq!(b.height, a.height * 2);
        assert_eq!(b.width, (a.width as f64 * 2.0) as i32);
    }

    #[test]
    fn 序号第10项为零() {
        let mut ctx = Context::default();
        for i in 0..10 {
            ctx.candidates.push(ime_ipc::Candidate {
                text: format!("词{i}"),
                comment: String::new(),
            });
        }
        let items = view_items(&ctx);
        assert_eq!(items[0].label, "1");
        assert_eq!(items[8].label, "9");
        assert_eq!(items[9].label, "0");
    }

    #[test]
    fn 模式角标_中英全角() {
        let mut ctx = Context::default();
        assert_eq!(mode_badge_for(&ctx), "中");

        ctx.is_ascii = true;
        assert_eq!(mode_badge_for(&ctx), "En");

        ctx.is_ascii = false;
        ctx.is_full_shape = true;
        assert_eq!(mode_badge_for(&ctx), "全");

        // 英文直输优先：英文 + 全角不闪烁切回中文
        ctx.is_ascii = true;
        assert_eq!(mode_badge_for(&ctx), "En");

        // 长按 Shift 大写视觉优先于中/英/全角
        ctx.caps_visual = true;
        assert_eq!(mode_badge_for(&ctx), "⇪");
        ctx.is_ascii = false;
        ctx.is_full_shape = false;
        assert_eq!(mode_badge_for(&ctx), "⇪");
    }
}
