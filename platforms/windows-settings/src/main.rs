//! Shurufa Windows 设置页。
//!
//! 这个轻量原生窗口只管理应用已有的配置和操作入口：中继地址持久化、守护
//! 进程启动、固定热门词库更新，以及跳转系统的输入法设置。避免再引入第二套
//! 配置格式或 Web 运行时。

#![cfg(windows)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::process::Command;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, LoadCursorW, MessageBoxW,
    PostQuitMessage, RegisterClassW, SetWindowTextW, ShowWindow, TranslateMessage, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, HMENU, IDC_ARROW, MB_ICONERROR, MB_ICONINFORMATION, MSG, SW_SHOW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_CREATE, WM_DESTROY, WNDCLASSW, WS_BORDER,
    WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

const WIDTH: i32 = 520;
const HEIGHT: i32 = 260;
const EDIT_STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | 0x0080);
const BUTTON_STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | 0x0001);

struct Controls {
    relay: HWND,
    save_relay: HWND,
    start_service: HWND,
    update_dictionary: HWND,
    open_windows_settings: HWND,
}

thread_local! {
    static CONTROLS: RefCell<Option<Controls>> = const { RefCell::new(None) };
}

fn app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("shurufa")
}

fn sync_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("SHURUFA_SYNC_DIR") {
        PathBuf::from(path)
    } else {
        app_data_dir().join("sync")
    }
}

fn sibling_exe(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn relay_text() -> String {
    sync_core::load_relay_addr(&sync_dir()).unwrap_or_default()
}

fn edit_text(hwnd: HWND) -> String {
    let mut buffer = [0u16; 512];
    let length =
        unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

fn show_message(hwnd: HWND, text: &str, error: bool) {
    unsafe {
        let _ = MessageBoxW(
            Some(hwnd),
            &HSTRING::from(text),
            w!("Shurufa 设置"),
            if error {
                MB_ICONERROR
            } else {
                MB_ICONINFORMATION
            },
        );
    }
}

fn create_control(
    class: PCWSTR,
    title: PCWSTR,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    parent: HWND,
    instance: HINSTANCE,
) -> HWND {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            title,
            style,
            x,
            y,
            width,
            height,
            Some(parent),
            None::<HMENU>,
            Some(instance),
            None,
        )
        .unwrap_or_default()
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    _wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            let _ = create_control(
                w!("STATIC"),
                w!("自托管同步中继（留空则关闭）："),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
                20,
                20,
                440,
                24,
                hwnd,
                instance.into(),
            );
            let relay = create_control(
                w!("EDIT"),
                PCWSTR::null(),
                EDIT_STYLE,
                20,
                48,
                330,
                28,
                hwnd,
                instance.into(),
            );
            let _ = SetWindowTextW(relay, &HSTRING::from(relay_text()));
            let save_relay = create_control(
                w!("BUTTON"),
                w!("保存中继"),
                BUTTON_STYLE,
                364,
                48,
                120,
                28,
                hwnd,
                instance.into(),
            );
            let start_service = create_control(
                w!("BUTTON"),
                w!("启动后台服务"),
                BUTTON_STYLE,
                20,
                104,
                150,
                32,
                hwnd,
                instance.into(),
            );
            let update_dictionary = create_control(
                w!("BUTTON"),
                w!("更新热门云词库"),
                BUTTON_STYLE,
                182,
                104,
                150,
                32,
                hwnd,
                instance.into(),
            );
            let open_windows_settings = create_control(
                w!("BUTTON"),
                w!("打开系统输入法设置"),
                BUTTON_STYLE,
                344,
                104,
                140,
                32,
                hwnd,
                instance.into(),
            );
            let _ = create_control(
                w!("STATIC"),
                w!("中继设置将在后台服务下次启动时生效；词库更新完成后请重启输入法。"),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
                20,
                166,
                460,
                40,
                hwnd,
                instance.into(),
            );
            CONTROLS.with_borrow_mut(|slot| {
                *slot = Some(Controls {
                    relay,
                    save_relay,
                    start_service,
                    update_dictionary,
                    open_windows_settings,
                });
            });
            LRESULT(0)
        }
        WM_COMMAND => {
            let source = HWND(lparam.0 as *mut c_void);
            CONTROLS.with_borrow(|slot| {
                let Some(controls) = slot.as_ref() else {
                    return;
                };
                if source == controls.save_relay {
                    let value = edit_text(controls.relay);
                    let relay = (!value.trim().is_empty()).then_some(value.trim());
                    match sync_core::save_relay_addr(&sync_dir(), relay) {
                        Ok(()) => show_message(hwnd, "中继设置已保存。重启后台服务后生效。", false),
                        Err(error) => {
                            show_message(hwnd, &format!("保存中继设置失败：{error}"), true)
                        }
                    }
                } else if source == controls.start_service {
                    match Command::new(sibling_exe("shurufa-host.exe"))
                        .arg("supervise")
                        .spawn()
                    {
                        Ok(_) => show_message(hwnd, "后台服务已启动或已在运行。", false),
                        Err(error) => {
                            show_message(hwnd, &format!("启动后台服务失败：{error}"), true)
                        }
                    }
                } else if source == controls.update_dictionary {
                    match Command::new(sibling_exe("shurufa-host.exe"))
                        .args(["dict-update", "rime-ice"])
                        .spawn()
                    {
                        Ok(_) => {
                            show_message(hwnd, "词库更新已在后台启动。完成后请重启输入法。", false)
                        }
                        Err(error) => {
                            show_message(hwnd, &format!("启动词库更新失败：{error}"), true)
                        }
                    }
                } else if source == controls.open_windows_settings {
                    match Command::new("cmd.exe")
                        .args(["/c", "start", "", "ms-settings:regionlanguage"])
                        .spawn()
                    {
                        Ok(_) => {}
                        Err(error) => {
                            show_message(hwnd, &format!("打开系统设置失败：{error}"), true)
                        }
                    }
                }
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            CONTROLS.with_borrow_mut(|slot| *slot = None);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, _wparam, lparam),
    }
}

fn main() {
    unsafe {
        let instance = GetModuleHandleW(PCWSTR::null()).expect("获取模块句柄失败");
        let class = w!("ShurufaSettingsWindow");
        let window_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&window_class);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Shurufa 设置"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WIDTH,
            HEIGHT,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("创建设置窗口失败");
        let _ = ShowWindow(hwnd, SW_SHOW);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sync_dir;

    #[test]
    fn 默认同步目录位于应用数据目录下() {
        assert!(sync_dir().ends_with("shurufa\\sync"));
    }
}
