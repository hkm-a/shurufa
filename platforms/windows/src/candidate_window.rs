//! 候选窗 hosted 模式（S5 默认，内置绘制路径已删除）。
//!
//! TSF 侧不再创建/绘制候选窗口，只负责：
//! - 把 `CandEvent::Show/Hide` 推给 `shurufa-ui` 的 `cand_host`；
//! - 读取 `CandCommand`（点击/滚轮）并合成虚拟键回走 TSF 正常按键路径；
//! - 管道连接/写入失败时静默降级（不弹内置窗，输入本身不受影响）。

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetForegroundWindow, GetSystemMetrics, GetWindowRect,
    PostMessageW, RegisterClassW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, WM_APP, WNDCLASSW,
};

use ime_ipc::{Candidate, Context};

use crate::cand_client::CandClient;

// ---------------------------------------------------------------------------
// 兼容旧 service.rs 注册的引擎动作钩子（builtin 菜单/AI 点击已删除，
// 保留 setter 避免改动 service.rs；实际 hosted 模式不使用）。
// ---------------------------------------------------------------------------

type EngineSimulateFn = Box<dyn Fn(&str) -> bool>;
thread_local! {
    static ENGINE_SIMULATE: RefCell<Option<EngineSimulateFn>> =
        const { RefCell::new(None) };
}

pub fn set_engine_simulate(f: EngineSimulateFn) {
    ENGINE_SIMULATE.with(|slot| *slot.borrow_mut() = Some(f));
}

fn engine_simulate(keys: &str) -> bool {
    ENGINE_SIMULATE.with(|slot| slot.borrow().as_ref().map(|f| f(keys)).unwrap_or(false))
}

/// 右键菜单动作分发：由 hosted 通过 CandCommand::MenuAction 回传。
pub fn dispatch_menu_action(action: &str, index: usize) {
    match action {
        "Drop" => {
            move_highlight_for_menu(index);
            engine_simulate("{Control+d}");
        }
        "Demote" => {
            move_highlight_for_menu(index);
            engine_simulate("{Control+j}");
        }
        "Hide" => {
            move_highlight_for_menu(index);
            engine_simulate("{Control+x}");
        }
        _ => {}
    }
}

fn move_highlight_for_menu(index: usize) {
    for _ in 0..index {
        engine_simulate("{Down}");
    }
}

// ---------------------------------------------------------------------------
// AI 候选消息常量与提交钩子（builtin 渲染已删除；hosted 下 AI 结果到达后
// 由隐藏窗口消息触发重推一帧）。
// ---------------------------------------------------------------------------

/// AI 结果到达后触发 hosted 主动刷新的隐藏窗口消息。
const WM_APP_AI_REFRESH: u32 = WM_APP + 82;
const AI_REFRESH_CLASS: PCWSTR = w!("ShurufaAiRefreshHost");
static AI_REFRESH_HWND: AtomicIsize = AtomicIsize::new(0);
static CURRENT_UI: AtomicIsize = AtomicIsize::new(0);

// TSF 侧长按 Shift 的大写视觉提示位；由 set_caps_visual 维护，
// build_view_ctx 每次推帧时写入 CandEvent Context。
thread_local! {
    static CAPS_VISUAL: Cell<bool> = const { Cell::new(false) };
}

type AiCommitFn = Box<dyn Fn(&str) -> bool>;
thread_local! {
    static AI_COMMIT: RefCell<Option<AiCommitFn>> = const { RefCell::new(None) };
}

pub fn set_ai_commit(f: AiCommitFn) {
    AI_COMMIT.with(|slot| *slot.borrow_mut() = Some(f));
}

// ---------------------------------------------------------------------------
// 供 service.rs 读取的 Context 快照。
// ---------------------------------------------------------------------------

thread_local! {
    static LAST_CTX: RefCell<Option<Context>> = const { RefCell::new(None) };
}

pub fn last_ctx_clone() -> Option<Context> {
    LAST_CTX.with(|c| c.borrow().clone())
}

// ---------------------------------------------------------------------------
// 通用工具函数（toast.rs 等仍引用）。
// ---------------------------------------------------------------------------

pub(crate) fn scale(base: i32, dpi: u32) -> i32 {
    (base * dpi as i32 + 48) / 96
}

pub(crate) fn logical_screen_dim(physical: i32, dpi: u32) -> i32 {
    (physical * 96 / dpi as i32).max(1)
}

pub(crate) fn font_height(base: i32, dpi: u32, font_scale: f32) -> i32 {
    ((scale(base, dpi) as f32) * font_scale).round().max(8.0) as i32
}

/// 预编辑串内的音节分隔符位置（UTF-16 码元索引），service.rs Tab 重映射用。
pub fn syllable_breaks(preedit: &str) -> Vec<u16> {
    let mut out = Vec::new();
    for (i, u) in preedit.encode_utf16().enumerate() {
        if u == b' ' as u16 || u == b'\'' as u16 {
            out.push(i as u16);
        }
    }
    out
}

/// 长按大写视觉提示：写入 TSF 线程状态，并在候选窗可见时立即重推一帧，
/// 让 hosted 候选窗的右上角角标同步为 `⇪` / 中 / En。
pub fn set_caps_visual(active: bool) -> bool {
    CAPS_VISUAL.with(|c| c.set(active));
    unsafe {
        refresh_ai_current();
    }
    true
}

/// 当前候选窗 HWND：builtin 已删除，hosted 无 TSF 侧窗口，恒 None。
pub fn current_hwnd() -> Option<HWND> {
    None
}

// ---------------------------------------------------------------------------
// 枚举（service.rs 仍使用）。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionMode {
    Follow,
    FixedBottomRight,
    FixedBottomLeft,
}

impl PositionMode {
    pub fn from_option(value: &str) -> Self {
        match value {
            "bottom_right" => PositionMode::FixedBottomRight,
            "bottom_left" => PositionMode::FixedBottomLeft,
            _ => PositionMode::Follow,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidatePanelMode {
    Single,
    Multi,
}

impl CandidatePanelMode {
    pub fn from_option(value: &str) -> Self {
        match value {
            "multi" => CandidatePanelMode::Multi,
            _ => CandidatePanelMode::Single,
        }
    }
}

fn is_foreground_fullscreen() -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return false;
        }
        let mut rect = RECT::default();
        if GetWindowRect(fg, &mut rect).is_err() {
            return false;
        }
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if vw <= 0 || vh <= 0 {
            return false;
        }
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        w >= vw * 95 / 100
            && h >= vh * 95 / 100
            && rect.left <= vx + vw / 100
            && rect.top <= vy + vh / 100
            && rect.right >= vx + vw - vw / 100
            && rect.bottom >= vy + vh - vh / 100
    }
}

fn build_view_ctx(ctx: &Context) -> Context {
    let mut view_ctx = ctx.clone();
    CAPS_VISUAL.with(|c| view_ctx.caps_visual = c.get());
    if view_ctx.candidates.is_empty() {
        let english = crate::english_candidates::suggest(&view_ctx.preedit);
        if !english.is_empty() {
            view_ctx.candidates = english
                .into_iter()
                .map(|t| Candidate {
                    text: t,
                    comment: String::new(),
                })
                .collect();
        }
    }
    let ai = crate::ai_candidates::cached(&view_ctx.preedit);
    if !ai.is_empty() {
        view_ctx.candidates.extend(
            ai.into_iter()
                .map(|t| Candidate {
                    text: t,
                    comment: "\u{1F916}".to_owned(),
                })
                .take(crate::ai_candidates::MAX_CANDIDATES),
        );
    }
    view_ctx.candidates.truncate(10);
    view_ctx
}

pub(crate) fn notify_ai_ready() {
    let hwnd = AI_REFRESH_HWND.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd as *mut _)),
                WM_APP_AI_REFRESH,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

unsafe fn refresh_ai_current() {
    let ptr = CURRENT_UI.load(Ordering::Relaxed);
    if ptr != 0 {
        let ui = &mut *(ptr as *mut CandidateUi);
        ui.refresh_ai();
    }
}

unsafe extern "system" fn ai_refresh_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_APP_AI_REFRESH {
        refresh_ai_current();
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn create_ai_refresh_window() -> Option<HWND> {
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null()).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(ai_refresh_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: AI_REFRESH_CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
            AI_REFRESH_CLASS,
            w!(""),
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(0),
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
        AI_REFRESH_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        Some(hwnd)
    }
}

// ---------------------------------------------------------------------------
// CandidateUi：hosted-only。
// ---------------------------------------------------------------------------

pub struct CandidateUi {
    cand_client: Option<CandClient>,
    client_id: u32,
    visible: bool,
    last_caret: (i32, i32, i32, i32),
    last_dpi: u32,
    last_multi_line: bool,
    last_position: String,
    inline_preedit: bool,
}

impl CandidateUi {
    pub fn new() -> Self {
        let ui = CandidateUi {
            cand_client: None,
            client_id: std::process::id(),
            visible: false,
            last_caret: (0, 0, 0, 0),
            last_dpi: 96,
            last_multi_line: false,
            last_position: "follow".to_owned(),
            inline_preedit: false,
        };
        CURRENT_UI.store(
            &ui as *const CandidateUi as *mut CandidateUi as isize,
            Ordering::Relaxed,
        );
        let _ = create_ai_refresh_window();
        ui
    }

    /// S3/S5 兼容入口：现在恒为 hosted，忽略参数。
    pub fn set_hosted(&mut self, _hosted: bool) {}

    /// 按应用 inline_preedit：true 时候选窗不重复绘制 preedit（应用内联显示）。
    pub fn set_inline_preedit(&mut self, enabled: bool) {
        self.inline_preedit = enabled;
    }

    pub fn show(
        &mut self,
        ctx: &Context,
        anchor: Option<POINT>,
        _position: PositionMode,
        _panel_mode: CandidatePanelMode,
    ) {
        // 保留 Context 快照：service.rs 简拼提交/提交拦截仍依赖它。
        LAST_CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));

        // 全屏/无边框最大化时不推 hosted（避免被游戏/全屏应用遮挡）；
        // 内置 fallback 已删除，因此全屏下暂不显示候选。
        if is_foreground_fullscreen() {
            self.visible = false;
            return;
        }

        // 迁入 hosted：引擎无候选时补英文联想；AI 候选结果从共享缓存合并。
        let view_ctx = build_view_ctx(ctx);

        if self.cand_client.is_none() {
            self.cand_client = CandClient::connect().ok();
        }
        if let Some(client) = &self.cand_client {
            let dpi = unsafe { GetDpiForSystem().max(96) }.max(96);
            let caret = match anchor {
                Some(p) => (p.x, p.y, 0, 0),
                None => (0, 0, 0, 0),
            };
            let multi_line = _panel_mode == CandidatePanelMode::Multi;
            let position = match _position {
                PositionMode::FixedBottomRight => "bottom_right",
                PositionMode::FixedBottomLeft => "bottom_left",
                PositionMode::Follow => "follow",
            };
            if client
                .show(
                    self.client_id,
                    &view_ctx,
                    caret,
                    dpi,
                    multi_line,
                    position,
                    self.inline_preedit,
                )
                .is_ok()
            {
                self.visible = true;
                self.last_caret = caret;
                self.last_dpi = dpi;
                self.last_multi_line = multi_line;
                self.last_position = position.to_owned();
                return;
            }
            // 写失败说明管道已失效：丢弃旧客户端，下帧重连。
            self.cand_client = None;
        }
        self.visible = false;
        crate::debug_log("hosted cand unavailable; builtin fallback removed");
    }

    pub fn hide(&mut self) {
        if let Some(client) = &self.cand_client {
            if client.hide(self.client_id).is_err() {
                self.cand_client = None;
            }
        }
        self.visible = false;
        crate::uia_provider::clear_candidate_text();
        LAST_CTX.with(|c| *c.borrow_mut() = None);
    }

    /// AI 结果到达后由隐藏窗口消息触发：用最新缓存重推一帧。
    fn refresh_ai(&mut self) {
        if !self.visible {
            return;
        }
        let Some(ctx) = last_ctx_clone() else {
            return;
        };
        let view_ctx = build_view_ctx(&ctx);
        if let Some(client) = &self.cand_client {
            if client
                .show(
                    self.client_id,
                    &view_ctx,
                    self.last_caret,
                    self.last_dpi,
                    self.last_multi_line,
                    &self.last_position,
                    self.inline_preedit,
                )
                .is_ok()
            {
                return;
            }
            self.cand_client = None;
        }
    }

    pub fn invalidate(&self) {
        // hosted 无本地窗口，无需重绘。
    }

    pub fn destroy(&mut self) {
        self.cand_client = None;
    }
}

// 保留 engine_simulate/ai_commit 的引用，避免 dead_code 警告（未来 hosted
// 的 AI/菜单能力可能经管道回发实现）。
#[allow(dead_code)]
fn _keep_compat(_: &str) -> bool {
    engine_simulate("") || AI_COMMIT.with(|c| c.borrow().is_some())
}
