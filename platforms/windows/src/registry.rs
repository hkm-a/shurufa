//! TSF 注册与反注册：CLSID 注册表项 + 输入处理器语言配置 + 类别声明。

use windows::core::{Interface, Result, GUID};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, ITfCategoryMgr,
    ITfInputProcessorProfileMgr, ITfInputProcessorProfiles, GUID_TFCAT_TIPCAP_COMLESS,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT, GUID_TFCAT_TIPCAP_SECUREMODE,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT, GUID_TFCAT_TIP_KEYBOARD,
    TF_IPPMF_DONTCARECURRENTINPUTLANGUAGE, TF_IPPMF_FORSESSION, TF_PROFILETYPE_INPUTPROCESSOR,
};

use crate::{dll_path, CLSID_SHURUFA, GUID_PROFILE, IME_NAME};

/// 简体中文（中国大陆）LANGID
const LANGID_ZH_CN: u16 = 0x0804;

const CATEGORIES: &[GUID] = &[
    GUID_TFCAT_TIP_KEYBOARD,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    // 安全桌面（UAC、锁屏输入框）与免 COM 激活场景，对齐系统输入法的声明
    GUID_TFCAT_TIPCAP_SECUREMODE,
    GUID_TFCAT_TIPCAP_COMLESS,
];

fn clsid_key_path() -> String {
    format!("CLSID\\{{{:?}}}", CLSID_SHURUFA)
}

/// 在 COM 单元内执行注册动作（regsvr32 调用时未必已初始化 COM）。
fn with_com<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let need_uninit = hr.is_ok();
        let result = f();
        if need_uninit {
            CoUninitialize();
        }
        result
    }
}

pub fn register() -> Result<()> {
    // 1. COM 类注册（HKCR 需要管理员权限）
    let key = windows_registry::CLASSES_ROOT.create(clsid_key_path())?;
    key.set_string("", IME_NAME)?;
    let inproc = key.create("InprocServer32")?;
    inproc.set_string("", dll_path().to_string_lossy().as_ref())?;
    inproc.set_string("ThreadingModel", "Apartment")?;

    // 2. TSF 输入处理器与语言配置注册。
    // 档案注册走 ITfInputProcessorProfileMgr（Win8+ 官方推荐，
    // SampleIME 同款）：旧的 AddLanguageProfile 注册出的档案
    // 在 Win11 输入切换器上表现异常（无法激活、从列表消失）。
    with_com(|| unsafe {
        let profiles: ITfInputProcessorProfiles =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        profiles.Register(&CLSID_SHURUFA)?;
        let profile_mgr: ITfInputProcessorProfileMgr = profiles.cast()?;
        let desc: Vec<u16> = IME_NAME.encode_utf16().collect();
        profile_mgr.RegisterProfile(
            &CLSID_SHURUFA,
            LANGID_ZH_CN,
            &GUID_PROFILE,
            &desc,
            &[],
            0,
            HKL::default(),
            0,
            true,
            0,
        )?;
        // 注册后立即在当前会话激活，避免要求用户注销或手动切换输入法。
        profile_mgr.ActivateProfile(
            TF_PROFILETYPE_INPUTPROCESSOR,
            LANGID_ZH_CN,
            &CLSID_SHURUFA,
            &GUID_PROFILE,
            HKL::default(),
            TF_IPPMF_FORSESSION | TF_IPPMF_DONTCARECURRENTINPUTLANGUAGE,
        )?;

        // 3. 类别声明：键盘类 TIP，支持沉浸式应用与系统托盘
        let categories: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        for cat in CATEGORIES {
            categories.RegisterCategory(&CLSID_SHURUFA, cat, &CLSID_SHURUFA)?;
        }
        Ok(())
    })
}

pub fn unregister() -> Result<()> {
    with_com(|| unsafe {
        let categories: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        for cat in CATEGORIES {
            let _ = categories.UnregisterCategory(&CLSID_SHURUFA, cat, &CLSID_SHURUFA);
        }
        let profiles: ITfInputProcessorProfiles =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        if let Ok(profile_mgr) = profiles.cast::<ITfInputProcessorProfileMgr>() {
            let _ = profile_mgr.UnregisterProfile(&CLSID_SHURUFA, LANGID_ZH_CN, &GUID_PROFILE, 0);
        }
        let _ = profiles.Unregister(&CLSID_SHURUFA);
        Ok(())
    })?;
    windows_registry::CLASSES_ROOT.remove_tree(clsid_key_path())?;
    Ok(())
}
