//! 粘贴回填：把历史条目重新写入系统剪贴板。
//!
//! 文本与文件列表写回为文本；图片由存储的 BMP 剥掉文件头还原为
//! CF_DIB。写回会触发一次 WM_CLIPBOARDUPDATE，但内容哈希与原条目
//! 一致，入库路径命中去重只刷新时间戳（条目自然浮到最新），
//! 因此无需自我标记格式。

use clipboard_store::{ClipEntry, ClipKind, ClipboardStore};
use windows::core::Result;
use windows::Win32::Foundation::{E_FAIL, HANDLE};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};

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
                // 存储格式为自包含 BMP：14 字节 BITMAPFILEHEADER + DIB
                Some(bmp) if bmp.len() > 14 => set_clipboard_dib(&bmp[14..]).map(|_| true),
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

fn with_open_clipboard(f: impl FnOnce() -> Result<()>) -> Result<()> {
    unsafe {
        // 所有者传 None：由系统代管数据生命周期
        OpenClipboard(None)?;
        let result = (|| -> Result<()> {
            EmptyClipboard()?;
            f()
        })();
        let _ = CloseClipboard();
        result
    }
}

fn set_clipboard_text(text: &str) -> Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: Vec<u8> = wide.iter().flat_map(|w| w.to_le_bytes()).collect();
    with_open_clipboard(|| unsafe { set_clipboard_bytes(CF_UNICODETEXT.0 as u32, &bytes) })
}

fn set_clipboard_dib(dib: &[u8]) -> Result<()> {
    with_open_clipboard(|| unsafe { set_clipboard_bytes(CF_DIB.0 as u32, dib) })
}

/// 文件条目写回：CF_HDROP（DROPFILES + 宽字符路径表）与路径文本并存。
/// `paths_text` 为换行分隔的绝对路径（入库时的存储格式）。
fn set_clipboard_files(paths_text: &str) -> Result<()> {
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

    let text: Vec<u8> = paths_text
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|w| w.to_le_bytes())
        .collect();

    with_open_clipboard(|| unsafe {
        set_clipboard_bytes(CF_HDROP.0 as u32, &hdrop)?;
        set_clipboard_bytes(CF_UNICODETEXT.0 as u32, &text)
    })
}
