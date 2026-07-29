//! 剪贴板监听：消息窗口接收 WM_CLIPBOARDUPDATE，读取内容归一化入库。
//!
//! 捕获优先级：文件列表 > 文本 > 图片（与用户感知的"复制了什么"一致）。
//! 密码管理器等敏感来源默认不入库。

use std::path::Path;

use clipboard_store::{ClipboardStore, RetentionPolicy};
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, GetClipboardOwner,
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowThreadProcessId,
    RegisterClassW, TranslateMessage, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CLIPBOARDUPDATE, WNDCLASSW,
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

struct ListenerState {
    store: ClipboardStore,
    last_sequence: u32,
    captured: u64,
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
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            // 消息专用窗口：不可见，仅收消息
            Some(HWND_MESSAGE),
            None,
            Some(hinstance.into()),
            None,
        )?;

        STATE = Some(ListenerState {
            store,
            last_sequence: GetClipboardSequenceNumber(),
            captured: 0,
        });
        AddClipboardFormatListener(hwnd)?;

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
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
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

impl ListenerState {
    fn on_clipboard_update(&mut self, hwnd: HWND) {
        // 同一次复制可能触发多次通知，用序列号去重
        let seq = unsafe { GetClipboardSequenceNumber() };
        if seq == self.last_sequence {
            return;
        }
        self.last_sequence = seq;

        let source = clipboard_source_app();
        if SENSITIVE_APPS.contains(&source.as_str()) {
            return;
        }

        if let Some(capture) = read_clipboard(hwnd) {
            let result = match &capture {
                Capture::Files(paths) => self.store.insert_files(paths, &source),
                Capture::Text(text) => self.store.insert_text(text, &source),
                Capture::Image(bmp) => self.store.insert_image(bmp, &source),
            };
            match result {
                Ok(Some(_)) => {
                    self.captured += 1;
                    if self.captured % RETENTION_INTERVAL == 0 {
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
    if IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() {
        if let Some(files) = read_files() {
            return Some(Capture::Files(files));
        }
    }
    if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok() {
        if let Some(text) = read_text() {
            return Some(Capture::Text(text));
        }
    }
    if IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok() {
        if let Some(bmp) = read_dib_as_bmp() {
            return Some(Capture::Image(bmp));
        }
    }
    None
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
    let dib = std::slice::from_raw_parts(ptr as *const u8, size);
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
    let masks = if compression == 3 && header_size == 40 { 12 } else { 0 };
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
