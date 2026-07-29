//! 粘贴回填：把历史条目重新写入系统剪贴板。
//!
//! M2 覆盖文本与文件列表（写回为文本路径）；图片回填依赖 BMP→DIB
//! 逆转换，与截图模块（M5）一并实现。
//!
//! 写回会触发一次 WM_CLIPBOARDUPDATE，但内容哈希与原条目一致，
//! 入库路径命中去重只刷新时间戳，因此无需自我标记格式。

use clipboard_store::{ClipEntry, ClipKind};
use windows::core::Result;
use windows::Win32::Foundation::{E_FAIL, HANDLE};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// 把条目内容写回剪贴板；返回是否受支持并成功。
pub fn copy_entry_to_clipboard(entry: &ClipEntry) -> Result<bool> {
    match entry.kind {
        ClipKind::Text | ClipKind::Files => set_clipboard_text(&entry.text).map(|_| true),
        ClipKind::Image => Ok(false),
    }
}

fn set_clipboard_text(text: &str) -> Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        // 所有者传 None：由系统代管数据生命周期
        OpenClipboard(None)?;
        let result = (|| -> Result<()> {
            EmptyClipboard()?;
            let bytes = wide.len() * 2;
            let hglobal = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() {
                return Err(windows::core::Error::from_hresult(E_FAIL));
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr as *mut u16, wide.len());
            let _ = GlobalUnlock(hglobal);
            // 成功后剪贴板接管内存，不得再释放
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(hglobal.0)))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}
