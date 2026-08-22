//! Windows 皮肤装载与外观助手（依赖 `core/skin` 的纯模型）。
//!
//! 本 crate 负责 Windows 专属部分：
//! - 系统深色模式读取（注册表）；
//! - 皮肤文件查找、mtime 缓存与线程本地 `Skin` 装载；
//! - `Skin::current()` / `Skin::refresh_on_setting_change()` 扩展（trait `SkinExt`）；
//! - DWM 圆角 / 沉浸式深色 / 分层透明 / 阴影壳。
//!
//! 纯 JSON 模型与解析在 `core/skin`，本 crate 只做 re-export 和扩展。

use std::cell::RefCell;
use std::path::PathBuf;

pub use core_skin::{
    candidate_colors_from_json, scrollbar_colors, scrollbar_geo, CandidateColors, IconSlot,
    Metrics, ScrollbarGeo, Shadow, Skin, SCROLLBAR_BASE_WIDTH,
};

/// `core_skin::Skin` 的 Windows 装载扩展。
///
/// 因为 `Skin` 定义在 `core/skin`，无法在 `windows-skin` 里加 inherent 方法；
/// 这里用 trait 提供 `load/current/refresh_on_setting_change`，消费方需引入
/// `windows_skin::SkinExt`（`platforms/windows/src/skin.rs` 的 glob re-export
/// 会自动带上）。
pub trait SkinExt {
    /// 读皮肤文件 + 系统主题，返回并缓存 Skin（线程本地）。
    fn load() -> Skin;
    /// 线程本地缓存的当前 Skin；未初始化时按默认路径加载。
    fn current() -> Skin;
    /// WM_SETTINGCHANGE 到达后调用：重读系统主题 + 皮肤文件，刷新线程缓存。
    fn refresh_on_setting_change() -> Skin;
}

impl SkinExt for Skin {
    /// 读皮肤文件 + 系统主题，返回并缓存 Skin（线程本地）。
    #[allow(dead_code)] // host 侧与旧 API `load_candidate_colors` 使用
    fn load() -> Skin {
        load_with(|| None)
    }

    /// 线程本地缓存的当前 Skin；未初始化时按默认路径加载。
    fn current() -> Skin {
        // 缓存命中直接返回（Skin 为 Copy）。
        if let Some(skin) = SKIN_CACHE.with_borrow(|slot| slot.as_ref().map(|c| c.skin)) {
            return skin;
        }
        // 缓存为空：先释放借用再装载。load_with 内部会写入同一个线程缓存，
        // 若在 with_borrow_mut 闭包内调用会触发 RefCell 双重借用 panic
        // （2026-08-19 实机复现：host worker 启动预热 ai_panel → Skin::current()，
        // “RefCell already mutably borrowed”）。
        let skin = load_with(|| None);
        SKIN_CACHE
            .with_borrow(|slot| slot.as_ref().map(|c| c.skin))
            .unwrap_or(skin)
    }

    /// WM_SETTINGCHANGE 到达后调用：重读系统主题 + 皮肤文件，刷新线程缓存。
    fn refresh_on_setting_change() -> Skin {
        SKIN_CACHE.with_borrow_mut(|slot| {
            let source = slot.as_ref().and_then(|c| c.source.clone());
            let skin = reload_from_source(source.clone());
            let mtime = source.as_deref().and_then(file_mtime_len);
            *slot = Some(CachedSkin {
                skin,
                source,
                mtime,
            });
            skin
        })
    }
}

struct CachedSkin {
    skin: Skin,
    source: Option<PathBuf>,
    /// 皮肤源文件的 (mtime, 长度) 基线；None = 无源（默认皮肤）
    mtime: Option<(std::time::SystemTime, u64)>,
}

thread_local! {
    static SKIN_CACHE: RefCell<Option<CachedSkin>> = const { RefCell::new(None) };
}

/// 系统是否处于深色模式：HKCU\...\Themes\Personalize\SystemUsesLightTheme
/// （0=dark, 1=light，缺失按 light）。
pub fn system_dark_mode() -> bool {
    windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|key| key.get_u32("SystemUsesLightTheme"))
        .map(|value| value == 0)
        .unwrap_or(false)
}

/// 按默认规则装载皮肤并写入线程缓存；`extra` 允许调用方注入额外的
/// 候选文件路径（TSF DLL 用它指向 DLL 旁的 schemas 目录）。
///
/// 历史坑（2026-08-16 实机反馈"打字莫名卡顿"）：candidate_window::show()
/// 每次按键都调本函数，旧实现无条件读文件 + JSON 解析 + 系统主题查询，
/// 实测 ~1ms/次 同步磁盘 I/O，叠加热路径上的 debug_log 写文件导致每键
/// ~3ms 阻塞。现改为**带 mtime/长度校验的缓存**：缓存存在且源文件未变化时
/// 直接返回，热切换皮肤仍生效（文件改动才重读），每键零 I/O。
pub fn load_with(extra: impl FnOnce() -> Option<PathBuf>) -> Skin {
    let source = resolved_skin_path(extra());
    // 缓存命中判定：源路径一致，且文件 mtime/长度未变（或双方都无源）
    let reuse = SKIN_CACHE.with_borrow(|slot| match slot {
        Some(cached) if cached.source == source => match (&source, &cached.mtime) {
            (None, _) => true, // 无源（默认皮肤）→ 复用
            (Some(path), Some(baseline)) => {
                file_mtime_len(path).is_some_and(|current| current == *baseline)
            }
            (Some(_), None) => false, // 有源但缓存无基线 → 重载
        },
        _ => false,
    });
    if reuse {
        return SKIN_CACHE.with_borrow(|slot| slot.as_ref().expect("命中缓存").skin);
    }
    let skin = reload_from_source(source.clone());
    let mtime = source.as_deref().and_then(file_mtime_len);
    SKIN_CACHE.with_borrow_mut(|slot| {
        *slot = Some(CachedSkin {
            skin,
            source,
            mtime,
        });
    });
    skin
}

/// 取文件 (mtime, 长度)；失败返回 None。
fn file_mtime_len(path: &std::path::Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    Some((mtime, meta.len()))
}

fn reload_from_source(source: Option<PathBuf>) -> Skin {
    match source.and_then(read_skin_text) {
        Some(text) => Skin::from_json(&text, system_dark_mode()),
        None => Skin::from_json("", system_dark_mode()),
    }
}

fn read_skin_text(path: PathBuf) -> Option<String> {
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.len() > 128 * 1024 {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// 皮肤文件查找链（命中即止）：SHURUFA_SKIN_PATH → %APPDATA%\shurufa →
/// 当前 exe 旁的 schemas 目录（host 部署形态） → 调用方注入路径（TSF DLL 旁）。
pub fn resolved_skin_path(extra: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SHURUFA_SKIN_PATH").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    let user = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|root| root.join("shurufa").join("shurufa-skin.json"));
    if user.as_ref().is_some_and(|path| path.is_file()) {
        return user;
    }
    let exe_sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        .map(|dir| dir.join("schemas").join("shurufa-skin.json"));
    if exe_sibling.as_ref().is_some_and(|path| path.is_file()) {
        return exe_sibling;
    }
    extra.filter(|path| path.is_file())
}

/// WM_SETTINGCHANGE 的 lparam 是否意味着主题变化（"ImmersiveColorSet"）；
/// lparam 为空时是泛泛的设置广播，也按主题可能已变处理（保守刷新）。
/// 备注：Windows 目标之外恒为 true，方便纯逻辑测试。
pub fn is_immersive_color_change(lparam: windows::Win32::Foundation::LPARAM) -> bool {
    if lparam.0 == 0 {
        return true;
    }
    #[cfg(windows)]
    {
        unsafe {
            windows::core::PCWSTR(lparam.0 as *const u16)
                .to_string()
                .map(|s| s == "ImmersiveColorSet")
                .unwrap_or(false)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = lparam;
        true
    }
}

#[cfg(windows)]
mod dwm_impl {
    use super::{Shadow, Skin};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HBRUSH, HGDIOBJ,
        PAINTSTRUCT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, LoadCursorW,
        RegisterClassW, SetLayeredWindowAttributes, SetWindowLongPtrW, ShowWindow, CS_HREDRAW,
        CS_VREDRAW, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE,
        WM_NCHITTEST, WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_POPUP,
    };

    // DWM 属性裸值（DWMWA_WINDOW_CORNER_PREFERENCE / DWMWA_USE_IMMERSIVE_DARK_MODE），
    // 用数值构造避开 windows crate 版本间符号出现/改名差异。
    const DWMWA_WINDOW_CORNER_PREFERENCE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(33);
    const DWMWA_USE_IMMERSIVE_DARK_MODE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(20);
    const DWMWCP_ROUND: u32 = 2;

    const SHELL_CLASS: PCWSTR = w!("ShurufaShadowShell");
    /// SetWindowLongPtr 的索引常量（GWL_EXSTYLE）。
    const GWL_EXSTYLE_INDEX: i32 = -20;

    /// 对一个自绘弹窗统一应用皮肤外观：Win11 圆角、沉浸式深色边框、
    /// metrics.opacity < 1 时启用分层窗口整体透明。
    /// 全部调用失败静默——Win10 没有圆角属性，落回直角也不破坏功能。
    pub fn apply_appearance(hwnd: HWND, skin: &Skin) {
        unsafe {
            let corner: u32 = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const u32 as *const core::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            );
            let dark: u32 = u32::from(skin.dark_mode);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const u32 as *const core::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            );
            apply_opacity(hwnd, skin.metrics.opacity);
        }
    }

    fn apply_opacity(hwnd: HWND, opacity: f32) {
        if opacity >= 0.999 {
            return;
        }
        let alpha = (opacity.clamp(0.05, 1.0) * 255.0).round() as u8;
        unsafe {
            let style = GetWindowLongPtrW(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX(GWL_EXSTYLE_INDEX),
            );
            SetWindowLongPtrW(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX(GWL_EXSTYLE_INDEX),
                style | WS_EX_LAYERED.0 as isize,
            );
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        }
    }

    /// 阴影壳：主窗下层的一个半透明黑色 WS_POPUP。
    /// 不抢焦点（WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW）、不挡点击
    /// （WS_EX_LAYERED + WM_NCHITTEST 返回 HTTRANSPARENT），z 序紧贴主窗下方。
    pub struct ShadowShell {
        hwnd: Option<HWND>,
    }

    impl Default for ShadowShell {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ShadowShell {
        pub fn new() -> Self {
            ShadowShell { hwnd: None }
        }

        fn ensure_window(&mut self) -> Option<HWND> {
            if let Some(hwnd) = self.hwnd {
                return Some(hwnd);
            }
            unsafe {
                let hinstance = GetModuleHandleW(PCWSTR::null()).ok()?;
                // 重复注册返回 0，忽略即可
                let class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(shell_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: SHELL_CLASS,
                    hbrBackground: HBRUSH::default(),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    ..Default::default()
                };
                RegisterClassW(&class);
                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                    SHELL_CLASS,
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(hinstance.into()),
                    None,
                )
                .ok()?;
                // 壳自身也拿 Win11 圆角，阴影轮廓跟主窗一致
                let corner: u32 = DWMWCP_ROUND;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &corner as *const u32 as *const core::ffi::c_void,
                    std::mem::size_of::<u32>() as u32,
                );
                self.hwnd = Some(hwnd);
                Some(hwnd)
            }
        }

        /// 让阴影壳贴着主窗 (x,y,w,h)：向外扩 radius/2，向下偏 radius/6，
        /// 然后调整 z 序为 壳 → 主窗（SetWindowPos 以壳为 insert-after）。
        pub fn sync(&mut self, owner: HWND, x: i32, y: i32, w: i32, h: i32, shadow: &Shadow) {
            if !shadow.enabled {
                self.hide();
                return;
            }
            let Some(shell) = self.ensure_window() else {
                return;
            };
            let pad = (shadow.radius / 2).max(2);
            let drop = (shadow.radius / 6).max(1);
            let alpha = shadow.alpha;
            unsafe {
                let _ = SetLayeredWindowAttributes(shell, COLORREF(0), alpha, LWA_ALPHA);
                let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                    shell,
                    Some(HWND_TOPMOST),
                    x - pad,
                    y - pad + drop,
                    w + pad * 2,
                    h + pad * 2,
                    SWP_NOACTIVATE,
                );
                let _ = ShowWindow(shell, SW_SHOWNOACTIVATE);
                // 主窗紧贴壳之上（保持 NOACTIVATE 语义）
                let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                    owner,
                    Some(shell),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE,
                );
            }
        }

        pub fn hide(&mut self) {
            if let Some(hwnd) = self.hwnd {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
        }

        #[allow(dead_code)] // candidates 用它；host 面板常驻不销毁壳
        pub fn destroy(&mut self) {
            if let Some(hwnd) = self.hwnd.take() {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
    }

    unsafe extern "system" fn shell_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            // 命中测试透明：鼠标事件穿透到下层窗口，壳绝不拦截候选窗/面板交互
            value if value == WM_NCHITTEST => LRESULT(-1), // HTTRANSPARENT
            value if value == WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                // 纯黑填充，整体透明度由 LWA_ALPHA 控制
                let black = CreateSolidBrush(COLORREF(0));
                FillRect(hdc, &ps.rcPaint, black);
                let _ = DeleteObject(HGDIOBJ(black.0));
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(windows)]
pub use dwm_impl::{apply_appearance, ShadowShell};

/// 读取用户覆盖、开发覆盖或部署的默认皮肤（亮色；主题感知请用 `Skin::load`）。
#[allow(dead_code)] // 向后兼容保留；窗口代码已改用 Skin
pub fn load_candidate_colors() -> CandidateColors {
    Skin::load().candidate
}

#[cfg(test)]
mod tests {
    use super::{file_mtime_len, load_with, SKIN_CACHE};
    use std::io::Write;

    /// 皮肤缓存行为（2026-08-16 卡顿修复新增）：mtime/长度未变时复用缓存
    /// （每键零 I/O），文件改动后才重读——热切换皮肤语义保持不变。
    #[test]
    fn load_with_reuses_cache_until_file_changes() {
        let dir = std::env::temp_dir().join(format!("shurufa-skin-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("skin.json");
        let write = |text: &str| {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(text.as_bytes()).unwrap();
        };
        let json_a = r##"{"version":2,"light":{"candidate":{"background":"#111111"}},"dark":{"candidate":{"background":"#222222"}}}"##;
        let json_b = r##"{"version":2,"light":{"candidate":{"background":"#FF333333"}},"dark":{"candidate":{"background":"#444444"}}}"##;
        write(json_a);
        SKIN_CACHE.with(|c| *c.borrow_mut() = None);
        let extra = || Some(path.clone());
        let s1 = load_with(extra);
        let bg_a = s1.candidate.background;
        let s2 = load_with(extra);
        assert_eq!(
            s2.candidate.background, bg_a,
            "缓存命中：内容应与首次加载一致"
        );
        write(json_b);
        let s3 = load_with(extra);
        assert_ne!(s3.candidate.background, bg_a, "文件改动后应重读 json_b");
        let s4 = load_with(extra);
        assert_eq!(s4.candidate.background, s3.candidate.background);
        let _ = std::fs::remove_dir_all(&dir);
        SKIN_CACHE.with(|c| *c.borrow_mut() = None);
    }

    /// file_mtime_len：正常返回 (mtime, len)，缺文件返回 None。
    #[test]
    fn file_mtime_len_reports_metadata() {
        let dir = std::env::temp_dir().join(format!("shurufa-mtime-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.json");
        std::fs::write(&p, "hello").unwrap();
        let got = file_mtime_len(&p);
        assert!(got.is_some(), "存在的文件应返回 (mtime, len)");
        assert_eq!(got.unwrap().1, 5, "长度应为 5 字节");
        assert!(file_mtime_len(&dir.join("missing.json")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
