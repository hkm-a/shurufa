#![windows_subsystem = "windows"]

//! shurufa-ui：面板集合进程（阶段4第4项拆分）。
//!
//! 持有全部热键（历史面板 / AI 帮写 / 划词润色 / 翻译 / 语音转写）与
//! 面板窗口；崩溃只影响面板，剪贴板数据路径（shurufa-clipd）不受牵连，
//! 由 supervisor 自动重启。控制中心（悬浮条麦克风按钮）经
//! WM_APP_SPEECH_TOGGLE（WM_APP+44）对本进程窗口类 ShurufaUiHost
//! PostMessage 触发语音转写，与热键同一入口。

use shurufa_host::{init_logging, log_line, open_store, supervis};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, SetTimer,
    TranslateMessage, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_HOTKEY, WM_TIMER, WNDCLASSW,
};

/// 控制中心（悬浮条麦克风按钮）经 WM_APP 消息触发语音转写面板
/// （与 Ctrl+Shift+S 热键同一入口 speech::toggle）。
/// 值与设置页 main.rs 的 WM_APP + 44 约定保持一致。
pub const WM_APP_SPEECH_TOGGLE: u32 = WM_APP + 44;

/// 热键门控轮询定时器 id：每 2 秒按 options.json 重读
/// enable_ai_hotkey / enable_polish_hotkey，变化即重注册（见 ai_panel.rs）。
const HOTKEY_GATE_TIMER_ID: usize = 1;

/// 面板进程单实例锁名。
const UI_MUTEX: &str = "Global\\shurufa-ui";

fn main() {
    // 候选窗宿主自检：--cand-selftest 起服务→模拟客户端→断言窗口创建。
    if std::env::args().any(|a| a == "--cand-selftest") {
        std::process::exit(shurufa_host::cand_host::selftest());
    }
    init_logging();
    shurufa_host::hide_own_console();
    match supervis::acquire_singleton(UI_MUTEX) {
        Ok(None) => {
            eprintln!("已有 shurufa-ui 在运行。");
            std::process::exit(1);
        }
        Ok(Some(h)) => std::mem::forget(h),
        Err(e) => {
            eprintln!("创建面板进程单实例锁失败：{e}");
            std::process::exit(1);
        }
    }
    if let Err(e) = run() {
        eprintln!("面板进程异常退出：{e}");
        std::process::exit(1);
    }
}

fn run() -> windows::core::Result<()> {
    unsafe {
        // 高分屏下面板按真实 DPI 布局渲染，而非被系统位图拉伸
        {
            use windows::Win32::UI::HiDpi::{
                SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            };
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }

        let hinstance = windows::Win32::System::LibraryLoader::GetModuleHandleW(PCWSTR::null())?;
        let class_name = w!("ShurufaUiHost");
        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&class);
        // 不可见顶层窗口：接收线程级热键消息与外部 WM_APP 唤起
        // （设置页按类名 ShurufaUiHost 发现）。
        let _hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("shurufa-ui"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;

        let hotkey = shurufa_host::panel::register_hotkey();
        println!("历史面板热键：{hotkey}");
        log_line(&format!("历史面板热键：{hotkey}"));
        let ai_hotkey = shurufa_host::ai_panel::register_hotkey();
        println!("AI 帮写热键：{ai_hotkey}");
        log_line(&format!("AI 帮写热键：{ai_hotkey}"));
        // 候选窗宿主（阶段 6 S2）：多客户端候选窗渲染 + 命令回发。
        if !shurufa_host::cand_host::start() {
            log_line("cand_host 启动失败：hosted 模式候选窗将回退内置渲染");
        }
        let speech_hotkey = shurufa_host::speech::register_hotkey();
        println!("语音转写热键：{speech_hotkey}");
        log_line(&format!("语音转写热键：{speech_hotkey}"));
        // AI/划词润色热键门控：与设置中心开关联动（默认全开），每 2 秒轮询
        // options.json，门控变化时反注册+重注册（必须在消息循环线程执行）。
        shurufa_host::ai_panel::sync_hotkey_gate_cache();
        // M9-2：预热 AI 面板窗口，设置中心「AI 帮写」入口可随时外部唤起
        shurufa_host::ai_panel::warm_up();
        // 门控定时器挂在消息窗口上
        if let Ok(hwnd) = windows::Win32::UI::WindowsAndMessaging::FindWindowW(class_name, None) {
            let _ = SetTimer(Some(hwnd), HOTKEY_GATE_TIMER_ID, 2000, None);
        }

        let store = open_store();
        log_line("shurufa-ui 面板进程已就绪");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // 线程级热键的 WM_HOTKEY 不属于任何窗口，须在循环内截获
            if msg.message == WM_HOTKEY {
                let id = msg.wParam.0 as i32;
                if id == shurufa_host::panel::HOTKEY_ID {
                    let entries = store.list(9, 0).unwrap_or_default();
                    shurufa_host::panel::show(entries);
                    continue;
                }
                if id == shurufa_host::ai_panel::HOTKEY_ID {
                    shurufa_host::ai_panel::show();
                    continue;
                }
                if id == shurufa_host::ai_panel::POLISH_HOTKEY_ID {
                    shurufa_host::ai_panel::polish_selection();
                    continue;
                }
                if id == shurufa_host::ai_panel::TRANSLATE_HOTKEY_ID {
                    shurufa_host::ai_panel::translate_selection();
                    continue;
                }
                if id == shurufa_host::speech::HOTKEY_ID {
                    shurufa_host::speech::toggle();
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
    if msg == WM_APP_SPEECH_TOGGLE {
        // 悬浮条麦克风 → 语音转写面板（同热键入口，仅换触发方）
        shurufa_host::speech::toggle();
        return LRESULT(1);
    }
    if msg == WM_TIMER && wparam.0 == HOTKEY_GATE_TIMER_ID {
        // 热键门控热更新：设置中心开关即改即存，变化才重注册
        shurufa_host::ai_panel::refresh_hotkey_gates();
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
