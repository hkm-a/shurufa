//! 候选窗 UIA Provider（hosted 模式，语义级冒烟/读屏用）。
//!
//! 只暴露 Name / ControlType(Text) / IsEnabled / IsKeyboardFocusable，
//! 供 pywinauto/FlaUI 读取当前候选文本；暂不实现 ITextProvider 全文范围。
//! 后续如需与内置候选窗同等的逐候选朗读，可再补齐 TextPattern。

use std::sync::{Mutex, OnceLock};

use windows::core::{implement, IUnknown, Result};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, VARIANT_TRUE, WPARAM};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_BSTR, VT_I4};
use windows::Win32::UI::Accessibility::{
    IRawElementProviderSimple, IRawElementProviderSimple_Impl, ProviderOptions,
    ProviderOptions_ServerSideProvider, ProviderOptions_UseComThreading, UIA_ControlTypePropertyId,
    UIA_IsEnabledPropertyId, UIA_IsKeyboardFocusablePropertyId, UIA_NamePropertyId,
    UIA_TextControlTypeId, UiaReturnRawElementProvider, UiaRootObjectId, UIA_PATTERN_ID,
    UIA_PROPERTY_ID,
};

static CAND_TEXT: OnceLock<Mutex<String>> = OnceLock::new();

fn current_text() -> String {
    CAND_TEXT
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

pub fn update_candidate_text(text: &str) {
    if let Ok(mut s) = CAND_TEXT.get_or_init(|| Mutex::new(String::new())).lock() {
        *s = text.to_owned();
    }
}

pub fn clear_candidate_text() {
    update_candidate_text("");
}

#[implement(IRawElementProviderSimple)]
struct CandProvider(());

impl IRawElementProviderSimple_Impl for CandProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, _patternid: UIA_PATTERN_ID) -> Result<IUnknown> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }

    fn GetPropertyValue(&self, propertyid: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let mut v = VARIANT::default();
        let inner = unsafe { &mut *v.Anonymous.Anonymous };
        if propertyid == UIA_NamePropertyId {
            let text: windows_core::BSTR = current_text().into();
            inner.vt = VT_BSTR;
            inner.Anonymous.bstrVal = core::mem::ManuallyDrop::new(text);
        } else if propertyid == UIA_ControlTypePropertyId {
            inner.vt = VT_I4;
            inner.Anonymous.lVal = UIA_TextControlTypeId.0;
        } else if propertyid == UIA_IsEnabledPropertyId
            || propertyid == UIA_IsKeyboardFocusablePropertyId
        {
            inner.vt = VT_BOOL;
            inner.Anonymous.boolVal = VARIANT_TRUE;
        }
        Ok(v)
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
}

static PROVIDER_RAW: OnceLock<usize> = OnceLock::new();

fn provider() -> IRawElementProviderSimple {
    let raw = *PROVIDER_RAW.get_or_init(|| {
        let p: IRawElementProviderSimple = CandProvider(()).into();
        windows::core::Interface::into_raw(p) as usize
    });
    unsafe {
        let vtbl = *(raw as *mut *const windows::core::IUnknown_Vtbl);
        ((*vtbl).AddRef)(raw as *mut core::ffi::c_void);
        windows::core::Interface::from_raw(raw as *mut core::ffi::c_void)
    }
}

/// 候选窗 wnd_proc 收到 WM_GETOBJECT 且 lParam == UiaRootObjectId 时调用。
///
/// # Safety
///
/// 必须在持有有效窗口句柄的窗口消息处理线程中调用（wnd_proc 回调上下文）。
pub unsafe fn on_wm_getobject(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if lparam.0 as i32 == UiaRootObjectId {
        Some(UiaReturnRawElementProvider(
            hwnd,
            wparam,
            lparam,
            &provider(),
        ))
    } else {
        None
    }
}
