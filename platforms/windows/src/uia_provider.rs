//! 候选窗 UI Automation Provider（v1.2 读屏阶段一）。
//!
//! 暴露 Name（当前候选行文本，随候选刷新更新）/ ControlType(Text) /
//! IsEnabled / IsKeyboardFocusable，NVDA / 讲述人聚焦候选窗时可朗读候选。
//! 完整 ITextProvider（逐候选范围朗读与导航）列为阶段二（评估报告口径）。
use std::sync::{Mutex, OnceLock};
use windows::core::{implement, IUnknown, Result};
use windows::Win32::Foundation::VARIANT_TRUE;
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_BSTR, VT_I4};
use windows::Win32::UI::Accessibility::{
    IRawElementProviderSimple, IRawElementProviderSimple_Impl, ProviderOptions,
    ProviderOptions_ServerSideProvider, ProviderOptions_UseComThreading, UIA_ControlTypePropertyId,
    UIA_IsEnabledPropertyId, UIA_IsKeyboardFocusablePropertyId, UIA_NamePropertyId,
    UIA_TextControlTypeId, UiaReturnRawElementProvider, UiaRootObjectId, UIA_PATTERN_ID,
    UIA_PROPERTY_ID,
};

static CANDIDATE_TEXT: OnceLock<Mutex<String>> = OnceLock::new();

fn current_text() -> String {
    CANDIDATE_TEXT
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

pub fn update_candidate_text(text: &str) {
    if let Ok(mut s) = CANDIDATE_TEXT
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
    {
        *s = text.to_owned();
    }
}

#[implement(IRawElementProviderSimple)]
struct CandidateProvider(());

impl IRawElementProviderSimple_Impl for CandidateProvider_Impl {
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
            inner.Anonymous.lVal = UIA_TextControlTypeId.0 as i32;
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
        let p: IRawElementProviderSimple = CandidateProvider(()).into();
        windows::core::Interface::into_raw(p) as usize
    });
    unsafe { windows::core::Interface::from_raw(raw as *mut core::ffi::c_void) }
}

/// 候选窗 wnd_proc 收到 WM_GETOBJECT 且 lParam == UiaRootObjectId 时调用。
pub unsafe fn on_wm_getobject(
    hwnd: windows::Win32::Foundation::HWND,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> Option<windows::Win32::Foundation::LRESULT> {
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

pub fn clear_candidate_text() {
    update_candidate_text("");
}
