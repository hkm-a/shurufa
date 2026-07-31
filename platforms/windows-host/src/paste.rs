//! 粘贴回填：把历史条目重新写入系统剪贴板。
//!
//! 文本与文件列表写回为文本；图片由存储的 BMP 剥掉文件头还原为
//! CF_DIB。写回会触发一次 WM_CLIPBOARDUPDATE，但内容哈希与原条目
//! 一致，入库路径命中去重只刷新时间戳（条目自然浮到最新），
//! 因此无需自我标记格式。

use clipboard_store::{ClipEntry, ClipKind, ClipboardStore};
use windows::core::Result;
use windows::Win32::Foundation::{E_FAIL, HANDLE, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};

/// 本程序回填时写入的私有格式；监听器据此跳过自身写回，避免图片的
/// CF_HDROP 临时 PNG 被再次作为文件条目捕获并同步。
pub fn owned_clipboard_format() -> u32 {
    unsafe { RegisterClipboardFormatW(windows::core::w!("ShurufaHistoryPaste")) }
}

/// 把条目内容写回剪贴板；返回是否受支持并成功。
pub fn copy_entry_to_clipboard(store: &ClipboardStore, entry: &ClipEntry) -> Result<bool> {
    match entry.kind {
        ClipKind::Text => set_clipboard_text(&entry.text).map(|_| true),
        // 文件条目双格式写回：资源管理器粘贴得到文件本体（CF_HDROP），
        // 文本框粘贴得到路径文本
        ClipKind::Files => set_clipboard_files(&entry.text).map(|_| true),
        ClipKind::Image => {
            let blob = store.image_data(entry.id).unwrap_or(None);
            match blob {
                // 同时写入位图与临时 PNG 文件：富文本程序取 CF_DIB，桌面和
                // 资源管理器取 CF_HDROP，两个目标都能直接 Ctrl+V。
                Some(bmp) if bmp.len() > 14 => set_clipboard_image(&bmp).map(|_| true),
                _ => Ok(false),
            }
        }
    }
}

/// 在剪贴板打开状态下写入一块全局内存数据。
unsafe fn set_clipboard_bytes(format: u32, bytes: &[u8]) -> Result<()> {
    let hglobal = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
    let ptr = GlobalLock(hglobal);
    if ptr.is_null() {
        return Err(windows::core::Error::from_hresult(E_FAIL));
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
    let _ = GlobalUnlock(hglobal);
    // 成功后剪贴板接管内存，不得再释放
    SetClipboardData(format, Some(HANDLE(hglobal.0)))?;
    Ok(())
}

fn with_open_clipboard(owner: Option<HWND>, f: impl FnOnce() -> Result<()>) -> Result<()> {
    unsafe {
        // 所有者传 None：由系统代管数据生命周期
        let mut opened = false;
        let mut last_error = None;
        for _ in 0..10 {
            match OpenClipboard(owner) {
                Ok(()) => {
                    opened = true;
                    break;
                }
                Err(e) => {
                    last_error = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }
        if !opened {
            return Err(last_error.unwrap_or_else(|| windows::core::Error::from_hresult(E_FAIL)));
        }
        let result = (|| -> Result<()> {
            EmptyClipboard()?;
            f()
        })();
        let _ = CloseClipboard();
        result
    }
}

pub(crate) fn set_clipboard_text(text: &str) -> Result<()> {
    set_clipboard_text_with_owner(text, None)
}

pub(crate) fn set_clipboard_text_with_owner(text: &str, owner: Option<HWND>) -> Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: Vec<u8> = wide.iter().flat_map(|w| w.to_le_bytes()).collect();
    with_open_clipboard(owner, || unsafe {
        set_clipboard_bytes(CF_UNICODETEXT.0 as u32, &bytes)
    })
}

pub(crate) fn set_clipboard_image(bmp: &[u8]) -> Result<()> {
    set_clipboard_image_with_owner(bmp, None)
}

pub(crate) fn set_clipboard_image_with_owner(bmp: &[u8], owner: Option<HWND>) -> Result<()> {
    let png_path = export_png(bmp)?;
    let hdrop = hdrop_bytes(&png_path.to_string_lossy());
    with_open_clipboard(owner, || unsafe {
        set_clipboard_bytes(CF_DIB.0 as u32, &bmp[14..])?;
        set_clipboard_bytes(CF_HDROP.0 as u32, &hdrop)?;
        set_clipboard_bytes(owned_clipboard_format(), &[1])
    })
}

#[cfg(debug_assertions)]
pub(crate) fn set_test_clipboard_image_with_owner(
    width: u32,
    height: u32,
    owner: Option<HWND>,
) -> Result<()> {
    let mut image = image::RgbaImage::new(width.max(1), height.max(1));
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = image::Rgba([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8, 255]);
    }
    let mut bmp = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bmp, image::ImageFormat::Bmp)
        .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?;
    let bmp = bmp.into_inner();
    with_open_clipboard(owner, || unsafe {
        set_clipboard_bytes(CF_DIB.0 as u32, &bmp[14..])
    })
}

#[cfg(debug_assertions)]
pub(crate) fn inspect_test_clipboard_image_with_owner(
    owner: Option<HWND>,
) -> Result<(i32, i32, bool, bool)> {
    use windows::Win32::System::DataExchange::{GetClipboardData, IsClipboardFormatAvailable};
    use windows::Win32::System::Memory::GlobalSize;

    unsafe {
        OpenClipboard(owner)?;
        let result = (|| -> Result<(i32, i32, bool, bool)> {
            let handle = GetClipboardData(CF_DIB.0 as u32)?;
            let hglobal = windows::Win32::Foundation::HGLOBAL(handle.0);
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() || GlobalSize(hglobal) < 12 {
                return Err(windows::core::Error::from_hresult(E_FAIL));
            }
            let bytes = std::slice::from_raw_parts(ptr as *const u8, 12);
            let width = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
            let height = i32::from_le_bytes(bytes[8..12].try_into().unwrap()).abs();
            let _ = GlobalUnlock(hglobal);
            Ok((
                width,
                height,
                IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok(),
                IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok(),
            ))
        })();
        let _ = CloseClipboard();
        result
    }
}

fn export_png(bmp: &[u8]) -> Result<std::path::PathBuf> {
    let image = image::load_from_memory_with_format(bmp, image::ImageFormat::Bmp)
        .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?;
    let dir = std::env::temp_dir().join("shurufa-paste");
    std::fs::create_dir_all(&dir).map_err(|_| windows::core::Error::from_hresult(E_FAIL))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?
        .as_millis();
    let path = dir.join(format!("image-{stamp}.png"));
    image
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?;
    Ok(path)
}

/// 文件条目写回：CF_HDROP（DROPFILES + 宽字符路径表）与路径文本并存。
/// `paths_text` 为换行分隔的绝对路径（入库时的存储格式）。
pub(crate) fn set_clipboard_files(paths_text: &str) -> Result<()> {
    set_clipboard_files_with_owner(paths_text, None)
}

pub(crate) fn set_clipboard_files_with_owner(paths_text: &str, owner: Option<HWND>) -> Result<()> {
    let hdrop = hdrop_bytes(paths_text);
    let text: Vec<u8> = paths_text
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|w| w.to_le_bytes())
        .collect();

    with_open_clipboard(owner, || unsafe {
        set_clipboard_bytes(CF_HDROP.0 as u32, &hdrop)?;
        set_clipboard_bytes(CF_UNICODETEXT.0 as u32, &text)?;
        set_clipboard_bytes(owned_clipboard_format(), &[1])
    })
}

fn hdrop_bytes(paths_text: &str) -> Vec<u8> {
    // DROPFILES 头：pFiles(4)=20, pt(8), fNC(4)=0, fWide(4)=1
    let mut hdrop = Vec::with_capacity(20 + paths_text.len() * 2 + 4);
    hdrop.extend_from_slice(&20u32.to_le_bytes());
    hdrop.extend_from_slice(&[0u8; 12]);
    hdrop.extend_from_slice(&1u32.to_le_bytes());
    for path in paths_text.lines().filter(|l| !l.is_empty()) {
        for unit in path.encode_utf16() {
            hdrop.extend_from_slice(&unit.to_le_bytes());
        }
        hdrop.extend_from_slice(&[0, 0]);
    }
    hdrop.extend_from_slice(&[0, 0]);
    hdrop
}
