//! 仅 Debug 的原生编辑控件 TSF 集成验收。
//!
//! 不依赖桌面自动化的文本注入：创建系统 EDIT 控件、确认它已取得前台焦点，
//! 再通过系统键盘输入走 TSF。此模块不参与 Release 产品。

use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_1, VK_BACK, VK_ESCAPE, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, FindWindowW, GetForegroundWindow,
    GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, PeekMessageW,
    SetForegroundWindow, SetWindowTextW, ShowWindow, TranslateMessage, MSG, PM_REMOVE, SW_SHOW,
    WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

const EXPECTED_TEXT: &str = "你好";
const CANDIDATE_WINDOW_CLASS: PCWSTR = w!("ShurufaCandidateWindow");

pub fn run() -> Result<String, String> {
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null())
            .map_err(|error| format!("读取模块句柄失败：{error}"))?;
        let window = CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("Shurufa TSF 原生验收"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            40,
            40,
            360,
            120,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|error| format!("创建验收窗口失败：{error}"))?;

        let edit = CreateWindowExW(
            Default::default(),
            w!("EDIT"),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_BORDER,
            16,
            24,
            320,
            32,
            Some(window),
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|error| {
            let _ = DestroyWindow(window);
            format!("创建原生编辑控件失败：{error}")
        })?;

        let previous_foreground = GetForegroundWindow();
        let attachment = ForegroundInputAttachment::attach(previous_foreground)?;
        let _ = ShowWindow(window, SW_SHOW);
        let _ = SetForegroundWindow(window);
        if let Err(error) = SetFocus(Some(edit)) {
            let _ = DestroyWindow(window);
            return Err(format!("验收编辑控件未取得焦点：{error}"));
        }
        if GetForegroundWindow() != window {
            let _ = DestroyWindow(window);
            return Err("验收窗口未取得前台焦点，已取消键盘输入".to_owned());
        }

        let result = run_scenarios(edit);
        let _ = DestroyWindow(window);
        drop(attachment);
        if !previous_foreground.is_invalid() {
            let _ = SetForegroundWindow(previous_foreground);
        }
        result
    }
}

struct ForegroundInputAttachment {
    current_thread: u32,
    foreground_thread: u32,
}

impl ForegroundInputAttachment {
    unsafe fn attach(foreground: HWND) -> Result<Option<Self>, String> {
        if foreground.is_invalid() {
            return Ok(None);
        }
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let current_thread = GetCurrentThreadId();
        if foreground_thread == 0 || foreground_thread == current_thread {
            return Ok(None);
        }
        if !AttachThreadInput(current_thread, foreground_thread, true).as_bool() {
            return Err("无法附加当前前台窗口的输入线程，已取消键盘输入".to_owned());
        }
        Ok(Some(Self {
            current_thread,
            foreground_thread,
        }))
    }
}

impl Drop for ForegroundInputAttachment {
    fn drop(&mut self) {
        unsafe {
            let _ = AttachThreadInput(self.current_thread, self.foreground_thread, false);
        }
    }
}

unsafe fn run_scenarios(edit: HWND) -> Result<String, String> {
    // 预编辑与候选窗必须同时出现，并且候选窗不能飘离当前编辑控件。
    send_pinyin()?;
    let candidate = wait_for_visible_candidate()?;
    assert_candidate_is_near_edit(candidate, edit)?;

    // 数字键选择首候选是独立于空格上屏的常用路径。
    send_keys(&[VK_1.0])?;
    wait_for_text(edit, EXPECTED_TEXT)?;
    wait_for_hidden_candidate()?;

    reset_edit(edit);
    send_pinyin()?;
    let candidate = wait_for_visible_candidate()?;
    assert_candidate_is_near_edit(candidate, edit)?;
    send_keys(&[VK_SPACE.0])?;
    wait_for_text(edit, EXPECTED_TEXT)?;
    wait_for_hidden_candidate()?;

    // 回删至空与 Esc 取消都不得在文档中遗留拼音或中文半成品。
    reset_edit(edit);
    send_pinyin()?;
    wait_for_visible_candidate()?;
    send_keys(&[VK_BACK.0; 5])?;
    wait_for_text(edit, "")?;
    wait_for_hidden_candidate()?;

    reset_edit(edit);
    send_pinyin()?;
    wait_for_visible_candidate()?;
    send_keys(&[VK_ESCAPE.0])?;
    wait_for_text(edit, "")?;
    wait_for_hidden_candidate()?;

    Ok(EXPECTED_TEXT.to_owned())
}

unsafe fn send_pinyin() -> Result<(), String> {
    send_keys(&[0x4E, 0x49, 0x48, 0x41, 0x4F])
}

unsafe fn send_keys(keys: &[u16]) -> Result<(), String> {
    let mut inputs = Vec::with_capacity(keys.len() * 2);
    for &vk in keys {
        inputs.push(key_input(vk, false));
        inputs.push(key_input(vk, true));
    }
    if SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) != inputs.len() as u32 {
        return Err("系统未完整接收验收键盘输入".to_owned());
    }
    Ok(())
}

unsafe fn wait_for_text(edit: HWND, expected: &str) -> Result<(), String> {
    wait_for(&format!("文本“{expected}”"), || {
        control_text(edit) == expected
    })
}

unsafe fn wait_for_visible_candidate() -> Result<HWND, String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        pump_messages();
        let candidate = candidate_window();
        if !candidate.is_invalid() && IsWindowVisible(candidate).as_bool() {
            return Ok(candidate);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err("三秒内未显示 Shurufa 候选窗".to_owned())
}

unsafe fn wait_for_hidden_candidate() -> Result<(), String> {
    wait_for("候选窗隐藏", || {
        let candidate = candidate_window();
        candidate.is_invalid() || !IsWindowVisible(candidate).as_bool()
    })
}

unsafe fn candidate_window() -> HWND {
    FindWindowW(CANDIDATE_WINDOW_CLASS, PCWSTR::null()).unwrap_or_default()
}

unsafe fn wait_for(label: &str, mut condition: impl FnMut() -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        pump_messages();
        if condition() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(format!("三秒内未满足验收条件：{label}"))
}

unsafe fn assert_candidate_is_near_edit(candidate: HWND, edit: HWND) -> Result<(), String> {
    let mut candidate_rect = RECT::default();
    let mut edit_rect = RECT::default();
    GetWindowRect(candidate, &mut candidate_rect)
        .map_err(|error| format!("读取候选窗位置失败：{error}"))?;
    GetWindowRect(edit, &mut edit_rect)
        .map_err(|error| format!("读取编辑控件位置失败：{error}"))?;
    if candidate_is_near_edit(candidate_rect, edit_rect) {
        Ok(())
    } else {
        Err(format!(
            "候选窗未贴近编辑控件：候选={candidate_rect:?}，编辑={edit_rect:?}"
        ))
    }
}

fn candidate_is_near_edit(candidate: RECT, edit: RECT) -> bool {
    let overlaps_horizontally = candidate.left < edit.right && candidate.right > edit.left;
    let is_below_or_aligned = candidate.top >= edit.top - 16;
    let has_area = candidate.right > candidate.left && candidate.bottom > candidate.top;
    overlaps_horizontally && is_below_or_aligned && has_area
}

unsafe fn reset_edit(edit: HWND) {
    let _ = SetWindowTextW(edit, w!(""));
}

fn key_input(vk: u16, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                dwFlags: if key_up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                ..Default::default()
            },
        },
    }
}

unsafe fn pump_messages() {
    let mut message = MSG::default();
    while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
}

unsafe fn control_text(edit: HWND) -> String {
    let mut buffer = [0u16; 128];
    let length = GetWindowTextW(edit, &mut buffer).max(0) as usize;
    String::from_utf16_lossy(&buffer[..length])
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::RECT;

    use super::{candidate_is_near_edit, EXPECTED_TEXT};

    #[test]
    fn 验收契约固定为首选中文提交() {
        assert_eq!(EXPECTED_TEXT, "你好");
    }

    #[test]
    fn 候选窗必须贴近当前编辑控件() {
        let edit = RECT {
            left: 40,
            top: 80,
            right: 300,
            bottom: 112,
        };
        assert!(candidate_is_near_edit(
            RECT {
                left: 40,
                top: 116,
                right: 240,
                bottom: 196,
            },
            edit,
        ));
        assert!(!candidate_is_near_edit(
            RECT {
                left: 500,
                top: 500,
                right: 700,
                bottom: 580,
            },
            edit,
        ));
    }
}
