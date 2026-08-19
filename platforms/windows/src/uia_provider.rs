//! 候选窗 UI Automation Provider（v1.2 读屏）。
//!
//! 阶段一：Name（当前候选行文本，随候选刷新更新）/ ControlType(Text) /
//! IsEnabled / IsKeyboardFocusable，NVDA / 讲述人聚焦候选窗时可朗读候选。
//! 阶段二：ITextProvider / ITextRangeProvider 只读全文范围（DocumentRange /
//! GetVisibleRanges / GetText(maxlength)），逐候选偏移与编辑能力按 UIA 规范
//! 返回 E_NOTIMPL / 0；运行时探针单测走读屏器同款客户端路径验证。
use std::sync::{Mutex, OnceLock};
use windows::core::{implement, IUnknown, Interface, Result};
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

    fn GetPatternProvider(&self, patternid: UIA_PATTERN_ID) -> Result<IUnknown> {
        if patternid == windows::Win32::UI::Accessibility::UIA_TextPatternId {
            // v1.2 阶段二：文本模式（全文范围，NVDA/讲述人可逐字朗读候选）
            let tp: ITextProvider = CandidateTextProvider(()).into();
            Ok(tp.cast()?)
        } else {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
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

/// 进程级单例 Provider：UIA 会持有该指针，必须与进程同生命周期。
/// 初始化时泄漏一个引用计数由静态永久持有；每次 from_raw 只借用另一份
/// 引用，首个 WM_GETOBJECT 后对象不会因包装释放而消亡（若每次新建，
/// 读屏器将拿到悬垂指针 → 堆损坏）。
static PROVIDER_RAW: OnceLock<usize> = OnceLock::new();

fn provider() -> IRawElementProviderSimple {
    let raw = *PROVIDER_RAW.get_or_init(|| {
        let p: IRawElementProviderSimple = CandidateProvider(()).into();
        std::mem::forget(p.clone());
        windows::core::Interface::into_raw(p) as usize
    });
    unsafe { windows::core::Interface::from_raw(raw as *mut core::ffi::c_void) }
}

/// 候选窗 wnd_proc 收到 WM_GETOBJECT 且 lParam == UiaRootObjectId 时调用。
///
/// # Safety
///
/// 必须在持有有效窗口句柄的窗口消息处理线程中调用（wnd_proc 回调上下文）。
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

// ================= 阶段二：ITextProvider / ITextRangeProvider =================
// 只读"全文范围"：DocumentRange/GetVisibleRanges 覆盖整条候选行文本，
// 逐候选偏移与编辑能力（Move/Select 等）按 UIA 规范返回 E_NOTIMPL / 空值。

use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayCreateVector, SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::VT_UNKNOWN;
use windows::Win32::UI::Accessibility::{
    ITextProvider, ITextProvider_Impl, ITextRangeProvider, ITextRangeProvider_Impl,
    SupportedTextSelection, SupportedTextSelection_None, TextPatternRangeEndpoint,
    TextPatternRangeEndpoint_Start, TextUnit, UiaPoint, UIA_TEXTATTRIBUTE_ID,
};

/// 候选行只读范围：始终代表整条候选文本（start=0, end=当前长度）。
#[implement(ITextRangeProvider)]
struct CandidateRange(());

impl ITextRangeProvider_Impl for CandidateRange_Impl {
    fn Clone(&self) -> Result<ITextRangeProvider> {
        Ok(CandidateRange(()).into())
    }
    fn Compare(&self, _range: windows_core::Ref<ITextRangeProvider>) -> Result<windows_core::BOOL> {
        Ok(windows_core::BOOL(1)) // 同为全文范围
    }
    fn CompareEndpoints(
        &self,
        endpoint: TextPatternRangeEndpoint,
        _targetrange: windows_core::Ref<ITextRangeProvider>,
        targetendpoint: TextPatternRangeEndpoint,
    ) -> Result<i32> {
        // 全文范围：Start 端点 0，End 端点 = 文本长度
        let len = current_text().chars().count() as i32;
        if endpoint == TextPatternRangeEndpoint_Start
            && targetendpoint == TextPatternRangeEndpoint_Start
        {
            return Ok(0);
        }
        if endpoint == TextPatternRangeEndpoint_Start {
            return Ok(-len);
        }
        if targetendpoint == TextPatternRangeEndpoint_Start {
            return Ok(len);
        }
        Ok(0)
    }
    fn ExpandToEnclosingUnit(&self, _unit: TextUnit) -> Result<()> {
        Ok(()) // 全文范围已是最外层
    }
    fn FindAttribute(
        &self,
        _attributeid: UIA_TEXTATTRIBUTE_ID,
        _val: &VARIANT,
        _backward: windows_core::BOOL,
    ) -> Result<ITextRangeProvider> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn FindText(
        &self,
        _text: &windows_core::BSTR,
        _backward: windows_core::BOOL,
        _ignorecase: windows_core::BOOL,
    ) -> Result<ITextRangeProvider> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn GetAttributeValue(&self, _attributeid: UIA_TEXTATTRIBUTE_ID) -> Result<VARIANT> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn GetBoundingRectangles(&self) -> Result<*mut SAFEARRAY> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn GetEnclosingElement(&self) -> Result<IRawElementProviderSimple> {
        Ok(provider())
    }
    fn GetText(&self, maxlength: i32) -> Result<windows_core::BSTR> {
        let text = current_text();
        let sliced: String = if maxlength >= 0 {
            text.chars().take(maxlength as usize).collect()
        } else {
            text
        };
        Ok(sliced.into())
    }
    fn Move(&self, _unit: TextUnit, _count: i32) -> Result<i32> {
        Ok(0)
    }
    fn MoveEndpointByUnit(
        &self,
        _endpoint: TextPatternRangeEndpoint,
        _unit: TextUnit,
        _count: i32,
    ) -> Result<i32> {
        Ok(0)
    }
    fn MoveEndpointByRange(
        &self,
        _endpoint: TextPatternRangeEndpoint,
        _targetrange: windows_core::Ref<ITextRangeProvider>,
        _targetendpoint: TextPatternRangeEndpoint,
    ) -> Result<()> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn Select(&self) -> Result<()> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn AddToSelection(&self) -> Result<()> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn RemoveFromSelection(&self) -> Result<()> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn ScrollIntoView(&self, _aligntotop: windows_core::BOOL) -> Result<()> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn GetChildren(&self) -> Result<*mut SAFEARRAY> {
        Ok(make_empty_array())
    }
}

#[implement(ITextProvider)]
struct CandidateTextProvider(());

impl ITextProvider_Impl for CandidateTextProvider_Impl {
    fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
        Ok(make_empty_array()) // 只读，无选区
    }
    fn GetVisibleRanges(&self) -> Result<*mut SAFEARRAY> {
        let range: ITextRangeProvider = CandidateRange(()).into();
        let unknown: windows_core::IUnknown = range.cast()?;
        Ok(make_unknown_array(&[unknown]))
    }
    fn RangeFromChild(
        &self,
        _childelement: windows_core::Ref<IRawElementProviderSimple>,
    ) -> Result<ITextRangeProvider> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn RangeFromPoint(&self, _point: &UiaPoint) -> Result<ITextRangeProvider> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }
    fn DocumentRange(&self) -> Result<ITextRangeProvider> {
        Ok(CandidateRange(()).into())
    }
    fn SupportedTextSelection(&self) -> Result<SupportedTextSelection> {
        Ok(SupportedTextSelection_None)
    }
}

fn make_empty_array() -> *mut SAFEARRAY {
    unsafe { SafeArrayCreateVector(VT_UNKNOWN, 0, 0) }
}

fn make_unknown_array(items: &[windows_core::IUnknown]) -> *mut SAFEARRAY {
    unsafe {
        let arr = SafeArrayCreateVector(VT_UNKNOWN, 0, items.len() as u32);
        if arr.is_null() {
            return std::ptr::null_mut();
        }
        let mut data: *mut *mut core::ffi::c_void = std::ptr::null_mut();
        if SafeArrayAccessData(arr, &mut data as *mut _ as *mut *mut core::ffi::c_void).is_err() {
            return arr;
        }
        for (i, item) in items.iter().enumerate() {
            *data.add(i) = windows::core::Interface::into_raw(item.clone());
        }
        let _ = SafeArrayUnaccessData(arr);
        arr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_text(t: &str) {
        update_candidate_text(t);
    }

    #[test]
    fn document_range_returns_full_text() {
        set_text("1.你好，2.世界");
        let tp: ITextProvider = CandidateTextProvider(()).into();
        let doc = unsafe { tp.DocumentRange() }.unwrap();
        assert_eq!(
            unsafe { doc.GetText(-1) }.unwrap().to_string(),
            "1.你好，2.世界"
        );
    }

    #[test]
    fn get_text_honors_max_length() {
        set_text("1.你好，2.世界");
        let range: ITextRangeProvider = CandidateRange(()).into();
        assert_eq!(unsafe { range.GetText(3) }.unwrap().to_string(), "1.你");
    }

    #[test]
    fn visible_ranges_hold_one_range() {
        set_text("1.X，2.Y");
        let tp: ITextProvider = CandidateTextProvider(()).into();
        let arr = unsafe { tp.GetVisibleRanges() }.unwrap();
        assert!(!arr.is_null());
        let mut data: *mut *mut core::ffi::c_void = std::ptr::null_mut();
        assert!(unsafe {
            SafeArrayAccessData(arr, &mut data as *mut _ as *mut *mut core::ffi::c_void)
        }
        .is_ok());
        let unknown: windows_core::IUnknown = unsafe { windows::core::Interface::from_raw(*data) };
        let _ = unsafe { SafeArrayUnaccessData(arr) };
        let _ = unknown.cast::<ITextRangeProvider>().unwrap();
    }

    #[test]
    fn selection_is_empty_array() {
        let tp: ITextProvider = CandidateTextProvider(()).into();
        assert!(!unsafe { tp.GetSelection() }.unwrap().is_null());
    }

    #[test]
    fn provider_exposes_text_pattern() {
        let p: IRawElementProviderSimple = CandidateProvider(()).into();
        let unknown =
            unsafe { p.GetPatternProvider(windows::Win32::UI::Accessibility::UIA_TextPatternId) }
                .unwrap();
        let _ = unknown.cast::<ITextProvider>().unwrap();
    }

    /// 端到端探针：真实窗口 + WM_GETOBJECT → UIA 客户端查询 Name 与 TextPattern，
    /// 等价于 NVDA / 讲述人读取候选窗的协议链路（不含读屏器本身）。
    #[test]
    fn uia_runtime_probe_roundtrip() {
        use windows::core::{w, PCWSTR};
        use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
        use windows::Win32::Graphics::Gdi::HBRUSH;
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation, UIA_TextPatternId};
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, PostQuitMessage, RegisterClassW,
            ShowWindow, CS_HREDRAW, CS_VREDRAW, SW_SHOW, WM_DESTROY, WM_GETOBJECT, WNDCLASSW,
        };

        const PROBE_CLASS: PCWSTR = w!("ShurufaUiaProbeWindow");

        unsafe extern "system" fn probe_wnd_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            if msg == WM_GETOBJECT {
                if let Some(lr) = on_wm_getobject(hwnd, wparam, lparam) {
                    return lr;
                }
            }
            if msg == WM_DESTROY {
                PostQuitMessage(0);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let hinstance = unsafe { GetModuleHandleW(PCWSTR::null()) }.unwrap();
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(probe_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PROBE_CLASS,
            hbrBackground: HBRUSH::default(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) };
        let hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                PROBE_CLASS,
                w!(""),
                Default::default(),
                0,
                0,
                320,
                48,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .unwrap();
        let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };

        update_candidate_text("1.你好，2.世界");

        let uia: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
                .expect("创建 CUIAutomation 失败");
        let el = unsafe { uia.ElementFromHandle(hwnd) }.expect("ElementFromHandle 失败");
        let name = unsafe { el.CurrentName() }.unwrap().to_string();
        assert_eq!(name, "1.你好，2.世界", "Name 属性应暴露候选行文本");

        // 读屏器同款路径：客户端模式对象（IUIAutomationTextPattern）→ DocumentRange → GetText
        let tp_client: windows::Win32::UI::Accessibility::IUIAutomationTextPattern =
            unsafe { el.GetCurrentPatternAs(UIA_TextPatternId) }.expect("候选窗应支持 TextPattern");
        let range = unsafe { tp_client.DocumentRange() }.expect("DocumentRange 失败");
        let text = unsafe { range.GetText(-1) }.unwrap().to_string();
        assert_eq!(text, "1.你好，2.世界", "DocumentRange 应覆盖整条候选行");

        unsafe {
            let _ = DestroyWindow(hwnd);
            CoUninitialize();
        }
    }
}
