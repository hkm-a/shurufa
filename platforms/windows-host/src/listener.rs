//! 剪贴板监听：消息窗口接收 WM_CLIPBOARDUPDATE，读取内容归一化入库。
//!
//! 捕获优先级：图片 > 文件列表 > 文本，避免多格式图片退化为临时文件名。
//! 密码管理器等敏感来源默认不入库。

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use clipboard_store::{ClipboardStore, RetentionPolicy};
use windows::core::{w, Result, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, GetClipboardOwner,
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowThreadProcessId,
    RegisterClassW, SetTimer, TranslateMessage, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_CLIPBOARDUPDATE, WM_HOTKEY, WM_TIMER, WNDCLASSW,
};

/// 敏感来源进程名（小写）：其复制内容不入历史
const SENSITIVE_APPS: &[&str] = &[
    "keepass.exe",
    "keepassxc.exe",
    "1password.exe",
    "bitwarden.exe",
    "enpass.exe",
];

/// 每捕获多少条执行一次留存清理
const RETENTION_INTERVAL: u64 = 50;

/// 同一次复制的多事务写入去重窗口（毫秒）：内容指纹一致且间隔小于该值
/// 视为同一次复制，跳过重复处理。
const DUP_WINDOW_MS: u128 = 1500;
#[cfg(debug_assertions)]
pub const WM_TEST_SET_IMAGE: u32 = WM_APP + 41;
const WM_WRITE_CLIPBOARD: u32 = WM_APP + 42;
#[cfg(debug_assertions)]
const WM_TEST_INSPECT_IMAGE: u32 = WM_APP + 43;
/// 控制中心（悬浮条麦克风按钮）经 WM_APP 消息触发语音转写面板
/// （与 Ctrl+Shift+S 热键同一入口 speech::toggle）。
pub const WM_APP_SPEECH_TOGGLE: u32 = WM_APP + 44;
/// 热键门控轮询定时器 id：每 2 秒按 options.json 重读
/// enable_ai_hotkey / enable_polish_hotkey，变化即重注册（见 ai_panel.rs）。
const HOTKEY_GATE_TIMER_ID: usize = 1;

enum ClipboardWrite {
    Text(String),
    Image(Vec<u8>),
    Files(String),
}

static WRITE_QUEUE: OnceLock<Mutex<VecDeque<ClipboardWrite>>> = OnceLock::new();
static LISTENER_HWND: AtomicIsize = AtomicIsize::new(0);

struct ListenerState {
    store: ClipboardStore,
    last_sequence: u32,
    captured: u64,
    /// 最近一次入库：(条目 id, 规范化内容, 时刻)。
    /// 资源管理器等来源一次复制会连续多次更新剪贴板（先文本路径
    /// 再文件对象），短时间窗内内容一致时合并为一条。
    last_insert: Option<(i64, String, std::time::Instant)>,
    /// 最近一次广播内容指纹：(指纹, 时刻)。浏览器/Office/微信等会把
    /// 一次复制拆成多个格式、多次事务写入剪贴板，序列号各不相同但内容
    /// 相同；短时间窗内指纹一致时整体跳过，避免重复入库与重复广播
    /// （2026-08-19 实机：.NET SetText ×2 / SetImage、SetFileDropList ×3）。
    last_broadcast: Option<(String, std::time::Instant)>,
}

// 消息窗口回调无法携带参数，监听进程单实例，用静态槽转交状态
static mut STATE: Option<ListenerState> = None;

/// 启动监听并进入消息循环（阻塞直至窗口销毁）。
pub fn run(store: ClipboardStore) -> Result<()> {
    unsafe {
        let hinstance = windows::Win32::System::LibraryLoader::GetModuleHandleW(PCWSTR::null())?;
        let class_name = w!("ShurufaClipboardListener");
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&class);
        let window_title = listener_window_title();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            &window_title,
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            // 不可见顶层窗口：接收剪贴板消息，也允许 Debug 控制进程按类名发现。
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;
        LISTENER_HWND.store(hwnd.0 as isize, Ordering::Release);

        STATE = Some(ListenerState {
            store,
            last_sequence: GetClipboardSequenceNumber(),
            captured: 0,
            last_insert: None,
            last_broadcast: None,
        });
        AddClipboardFormatListener(hwnd)?;
        let hotkey = crate::panel::register_hotkey();
        println!("历史面板热键：{hotkey}");
        crate::log_line(&format!("历史面板热键：{hotkey}"));
        let ai_hotkey = crate::ai_panel::register_hotkey();
        println!("AI 帮写热键：{ai_hotkey}");
        crate::log_line(&format!("AI 帮写热键：{ai_hotkey}"));
        let speech_hotkey = crate::speech::register_hotkey();
        println!("语音转写热键：{speech_hotkey}");
        crate::log_line(&format!("语音转写热键：{speech_hotkey}"));
        // AI/划词润色热键门控：与设置中心开关联动（默认全开），每 2 秒轮询
        // options.json，门控变化时反注册+重注册（必须在消息循环线程执行）。
        crate::ai_panel::sync_hotkey_gate_cache();
        // M9-2：预热 AI 面板窗口，设置中心「AI 帮写」入口可随时外部唤起
        crate::ai_panel::warm_up();
        let _ = SetTimer(Some(hwnd), HOTKEY_GATE_TIMER_ID, 2000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // 线程级热键的 WM_HOTKEY 不属于任何窗口，须在循环内截获
            if msg.message == WM_HOTKEY {
                let id = msg.wParam.0 as i32;
                if id == crate::panel::HOTKEY_ID {
                    #[allow(static_mut_refs)]
                    if let Some(state) = STATE.as_ref() {
                        let entries = state.store.list(9, 0).unwrap_or_default();
                        crate::panel::show(entries);
                    }
                    continue;
                }
                if id == crate::ai_panel::HOTKEY_ID {
                    crate::ai_panel::show();
                    continue;
                }
                if id == crate::ai_panel::POLISH_HOTKEY_ID {
                    crate::ai_panel::polish_selection();
                    continue;
                }
                if id == crate::ai_panel::TRANSLATE_HOTKEY_ID {
                    crate::ai_panel::translate_selection();
                    continue;
                }
                if id == crate::speech::HOTKEY_ID {
                    crate::speech::toggle();
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        #[allow(static_mut_refs)]
        if let Some(state) = STATE.as_mut() {
            state.on_clipboard_update(hwnd);
        }
        return LRESULT(0);
    }
    #[cfg(debug_assertions)]
    if msg == WM_TEST_SET_IMAGE {
        let result = crate::paste::make_test_bmp(wparam.0 as u32, lparam.0 as u32);
        // 自动化会话可能不拥有系统剪贴板。测试源直接进入正式广播队列，
        // Android→Windows 方向仍由脚本验证真实系统剪贴板写入。
        return match result {
            Ok(bmp) => {
                crate::sync::broadcast_image(&bmp);
                LRESULT(1)
            }
            Err(error) => {
                crate::log_line(&format!("测试图片写入剪贴板失败：{error}"));
                LRESULT(0)
            }
        };
    }
    if msg == WM_WRITE_CLIPBOARD {
        let command = WRITE_QUEUE
            .get_or_init(|| Mutex::new(VecDeque::new()))
            .lock()
            .expect("剪贴板写入队列锁不可恢复")
            .pop_front();
        let ok = command
            .map(|item| write_clipboard(hwnd, item))
            .unwrap_or(false);
        return LRESULT(if ok { 1 } else { 0 });
    }
    if msg == WM_APP_SPEECH_TOGGLE {
        // 悬浮条麦克风 → 语音转写面板（同热键入口，仅换触发方）
        crate::speech::toggle();
        return LRESULT(1);
    }
    if msg == WM_TIMER && wparam.0 == HOTKEY_GATE_TIMER_ID {
        // 热键门控热更新：设置中心开关即改即存，变化才重注册
        crate::ai_panel::refresh_hotkey_gates();
        return LRESULT(0);
    }
    #[cfg(debug_assertions)]
    if msg == WM_TEST_INSPECT_IMAGE {
        return match crate::paste::inspect_test_clipboard_image_with_owner(Some(hwnd)) {
            Ok((width, height, _, _)) if width > 0 && height > 0 => {
                LRESULT((((width as u32) << 16) | height as u32) as isize)
            }
            _ => LRESULT(0),
        };
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn write_clipboard(hwnd: HWND, command: ClipboardWrite) -> bool {
    let result = match command {
        ClipboardWrite::Text(text) => {
            crate::paste::set_clipboard_text_with_owner(&text, Some(hwnd))
        }
        ClipboardWrite::Image(bmp) => {
            crate::paste::set_clipboard_image_with_owner(&bmp, Some(hwnd))
        }
        ClipboardWrite::Files(paths) => {
            crate::paste::set_clipboard_files_with_owner(&paths, Some(hwnd))
        }
    };
    result.is_ok()
}

fn listener_window() -> Option<HWND> {
    let raw = LISTENER_HWND.load(Ordering::Acquire);
    if raw != 0 {
        return Some(HWND(raw as *mut _));
    }

    #[cfg(debug_assertions)]
    {
        use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

        let title = listener_window_title();
        let title = if title.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(title.as_ptr())
        };
        unsafe { FindWindowW(w!("ShurufaClipboardListener"), title).ok() }
    }

    #[cfg(not(debug_assertions))]
    {
        None
    }
}

fn listener_window_title() -> HSTRING {
    #[cfg(any(debug_assertions, test))]
    {
        HSTRING::from(debug_listener_title(
            std::env::var("SHURUFA_TEST_LISTENER_TITLE").ok(),
        ))
    }

    #[cfg(not(any(debug_assertions, test)))]
    HSTRING::new()
}

#[cfg(any(debug_assertions, test))]
fn debug_listener_title(value: Option<String>) -> String {
    value
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_default()
}

fn request_write(command: ClipboardWrite) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

    let Some(hwnd) = listener_window() else {
        return false;
    };
    WRITE_QUEUE
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .expect("剪贴板写入队列锁不可恢复")
        .push_back(command);
    unsafe { SendMessageW(hwnd, WM_WRITE_CLIPBOARD, None, None).0 == 1 }
}

pub fn write_remote_text(text: String) -> bool {
    request_write(ClipboardWrite::Text(text))
}

pub fn write_remote_image(bmp: Vec<u8>) -> bool {
    request_write(ClipboardWrite::Image(bmp))
}

pub fn write_remote_files(paths: String) -> bool {
    request_write(ClipboardWrite::Files(paths))
}

#[cfg(debug_assertions)]
pub fn request_test_image(width: u32, height: u32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

    unsafe {
        let Some(hwnd) = listener_window() else {
            return false;
        };
        SendMessageW(
            hwnd,
            WM_TEST_SET_IMAGE,
            Some(WPARAM(width as usize)),
            Some(LPARAM(height as isize)),
        )
        .0 == 1
    }
}

#[cfg(debug_assertions)]
pub fn inspect_test_image() -> Option<(u32, u32)> {
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

    let hwnd = listener_window()?;
    let packed = unsafe { SendMessageW(hwnd, WM_TEST_INSPECT_IMAGE, None, None).0 as u32 };
    if packed == 0 {
        None
    } else {
        Some((packed >> 16, packed & 0xffff))
    }
}

impl ListenerState {
    fn on_clipboard_update(&mut self, hwnd: HWND) {
        // 同一次复制可能触发多次通知，用序列号去重
        let seq = unsafe { GetClipboardSequenceNumber() };
        if seq == self.last_sequence {
            return;
        }
        self.last_sequence = seq;

        // 历史面板回填的图片同时含 CF_DIB 和 CF_HDROP；跳过私有标记，
        // 防止临时 PNG 反向变成文件历史并被再次同步。
        if unsafe { IsClipboardFormatAvailable(crate::paste::owned_clipboard_format()) }.is_ok() {
            return;
        }

        let source = clipboard_source_app();
        if SENSITIVE_APPS.contains(&source.as_str()) {
            return;
        }

        if let Some(capture) = read_clipboard(hwnd) {
            // 内容指纹覆盖全部类型（图片也纳入）：同一次复制拆成多次
            // 事务写剪贴板时序列号各不相同，但内容一致——短窗口内整体跳过，
            // 避免重复入库/重复广播（2026-08-19 实机复现 ×2/×3 投递）。
            let fingerprint = match &capture {
                Capture::Files(paths) => paths.join(
                    "
",
                ),
                Capture::Text(text) => text.clone(),
                Capture::Image(bmp) => {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    bmp.hash(&mut h);
                    format!("img:{:016x}", h.finish())
                }
            };
            if let Some((last_fp, at)) = &self.last_broadcast {
                if *last_fp == fingerprint && at.elapsed().as_millis() < DUP_WINDOW_MS {
                    return;
                }
            }

            // 同一次复制的多重更新：与上一条内容一致且间隔极短时，
            // 删除旧条目改存新条目（文件对象优先于纯文本路径）
            let normalized = match &capture {
                Capture::Files(paths) => paths.join(
                    "
",
                ),
                Capture::Text(text) => text.clone(),
                Capture::Image(_) => String::new(),
            };
            if !normalized.is_empty() {
                if let Some((last_id, last_norm, at)) = &self.last_insert {
                    if *last_norm == normalized && at.elapsed().as_millis() < 1500 {
                        let _ = self.store.delete(*last_id);
                    }
                }
            }

            let result = match &capture {
                Capture::Files(paths) => self.store.insert_files(paths, &source),
                Capture::Text(text) => self.store.insert_text(text, &source),
                Capture::Image(bmp) => self.store.insert_image(bmp, &source),
            };
            match result {
                Ok(Some(id)) => {
                    // 本机复制的文本、图片和上限内单文件均推送给已配对设备。
                    match &capture {
                        Capture::Text(text) => crate::sync::broadcast_text(text),
                        Capture::Image(bmp) => crate::sync::broadcast_image(bmp),
                        Capture::Files(paths) => {
                            if paths.len() == 1 {
                                crate::sync::broadcast_file(Path::new(&paths[0]));
                            }
                        }
                    }
                    if !normalized.is_empty() {
                        self.last_insert = Some((id, normalized, std::time::Instant::now()));
                    }
                    self.last_broadcast = Some((fingerprint, std::time::Instant::now()));
                    self.captured += 1;
                    if self.captured.is_multiple_of(RETENTION_INTERVAL) {
                        let _ = self.store.apply_retention(&RetentionPolicy::default());
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("历史入库失败：{e}"),
            }
        }
    }
}

enum Capture {
    Text(String),
    Image(Vec<u8>),
    Files(Vec<String>),
}

/// 打开剪贴板读取内容；剪贴板被他人占用时短暂重试。
fn read_clipboard(hwnd: HWND) -> Option<Capture> {
    unsafe {
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(Some(hwnd)).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !opened {
            return None;
        }
        let result = read_open_clipboard();
        let _ = CloseClipboard();
        result
    }
}

unsafe fn read_open_clipboard() -> Option<Capture> {
    let format = preferred_format(
        IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok(),
        IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok(),
        IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok(),
    );
    match format {
        Some(PreferredFormat::Image) => read_dib_as_bmp().map(Capture::Image),
        Some(PreferredFormat::Files) => read_files().map(Capture::Files),
        Some(PreferredFormat::Text) => read_text().map(Capture::Text),
        None => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreferredFormat {
    Image,
    Files,
    Text,
}

/// 图片应用常同时提供 CF_DIB、CF_HDROP 和文件名文本；位图必须优先，
/// 否则跨设备同步会退化成临时文件或纯文件名。
fn preferred_format(has_image: bool, has_files: bool, has_text: bool) -> Option<PreferredFormat> {
    if has_image {
        Some(PreferredFormat::Image)
    } else if has_files {
        Some(PreferredFormat::Files)
    } else if has_text {
        Some(PreferredFormat::Text)
    } else {
        None
    }
}

unsafe fn read_text() -> Option<String> {
    let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
    let (ptr, size) = lock_global(handle)?;
    let wide = std::slice::from_raw_parts(ptr as *const u16, size / 2);
    let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    let text = String::from_utf16_lossy(&wide[..len]);
    unlock_global(handle);
    Some(text)
}

unsafe fn read_files() -> Option<Vec<String>> {
    let handle = GetClipboardData(CF_HDROP.0 as u32).ok()?;
    let hdrop = HDROP(handle.0);
    let count = DragQueryFileW(hdrop, u32::MAX, None);
    let mut paths = Vec::with_capacity(count as usize);
    let mut buf = [0u16; 1024];
    for i in 0..count {
        let len = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
        if len > 0 {
            paths.push(String::from_utf16_lossy(&buf[..len]));
        }
    }
    Some(paths)
}

/// 把 CF_DIB 包装成自包含 BMP 文件字节（补 BITMAPFILEHEADER）。
unsafe fn read_dib_as_bmp() -> Option<Vec<u8>> {
    let handle = GetClipboardData(CF_DIB.0 as u32).ok()?;
    let (ptr, size) = lock_global(handle)?;
    let dib = std::slice::from_raw_parts(ptr, size);
    let bmp = wrap_dib_in_bmp(dib);
    unlock_global(handle);
    bmp
}

unsafe fn lock_global(handle: HANDLE) -> Option<(*const u8, usize)> {
    let hglobal = HGLOBAL(handle.0);
    let ptr = GlobalLock(hglobal);
    if ptr.is_null() {
        return None;
    }
    Some((ptr as *const u8, GlobalSize(hglobal)))
}

unsafe fn unlock_global(handle: HANDLE) {
    let _ = GlobalUnlock(HGLOBAL(handle.0));
}

/// 依据 DIB 头计算像素数据偏移并拼出 BMP 文件。
/// 参考 BITMAPINFOHEADER 布局：biSize/biBitCount/biCompression/biClrUsed。
fn wrap_dib_in_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 {
        return None;
    }
    let u32_at = |off: usize| u32::from_le_bytes(dib[off..off + 4].try_into().unwrap());
    let u16_at = |off: usize| u16::from_le_bytes(dib[off..off + 2].try_into().unwrap());

    let header_size = u32_at(0) as usize;
    let bit_count = u16_at(14) as usize;
    let compression = u32_at(16);
    let clr_used = u32_at(32) as usize;

    let palette_entries = if clr_used > 0 {
        clr_used
    } else if bit_count <= 8 {
        1usize << bit_count
    } else {
        0
    };
    // BI_BITFIELDS(3) 且经典 40 字节头时附带 3 个颜色掩码
    let masks = if compression == 3 && header_size == 40 {
        12
    } else {
        0
    };
    let pixel_offset = 14 + header_size + masks + palette_entries * 4;

    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&(pixel_offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

/// 剪贴板所有者的进程名（小写文件名）；取不到时返回空串。
fn clipboard_source_app() -> String {
    unsafe {
        let Ok(owner) = GetClipboardOwner() else {
            return String::new();
        };
        let mut pid = 0u32;
        GetWindowThreadProcessId(owner, Some(&mut pid));
        if pid == 0 {
            return String::new();
        }
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let name = if QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok()
        {
            let full = String::from_utf16_lossy(&buf[..len as usize]);
            Path::new(&full)
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let _ = windows::Win32::Foundation::CloseHandle(process);
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 图片格式优先于文件和文本() {
        assert_eq!(
            preferred_format(true, true, true),
            Some(PreferredFormat::Image)
        );
        assert_eq!(
            preferred_format(false, true, true),
            Some(PreferredFormat::Files)
        );
        assert_eq!(
            preferred_format(false, false, true),
            Some(PreferredFormat::Text)
        );
    }

    #[test]
    fn 调试监听窗口标题仅接受非空隔离标识() {
        assert_eq!(debug_listener_title(None), "");
        assert_eq!(debug_listener_title(Some("   ".to_owned())), "");
        assert_eq!(
            debug_listener_title(Some("background-sync-48634".to_owned())),
            "background-sync-48634"
        );
    }
}
