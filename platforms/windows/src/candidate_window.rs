//! 候选窗 hosted 模式（S5 默认，内置绘制路径已删除）。
//!
//! TSF 侧不再创建/绘制候选窗口，只负责：
//! - 把 `CandEvent::Show/Hide` 推给 `shurufa-ui` 的 `cand_host`；
//! - 读取 `CandCommand`（点击/滚轮）并合成虚拟键回走 TSF 正常按键路径；
//! - 管道连接/写入失败时静默降级（不弹内置窗，输入本身不受影响）。

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetSystemMetrics, GetWindowRect, WM_APP, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
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

// ---------------------------------------------------------------------------
// AI 候选消息常量与提交钩子（builtin 渲染已删除；常量保留供 ai_candidates
// 编译，实际 hosted 下 AI 候选展示由 shurufa-ui 后续版本接管）。
// ---------------------------------------------------------------------------

pub(crate) const WM_AI_CANDIDATES_READY: u32 = WM_APP + 81;

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

/// 长按大写视觉提示：builtin 已删除，hosted 下暂不支持，恒返回 false。
pub fn set_caps_visual(_active: bool) -> bool {
    false
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

// ---------------------------------------------------------------------------
// CandidateUi：hosted-only。
// ---------------------------------------------------------------------------

pub struct CandidateUi {
    cand_client: Option<CandClient>,
    client_id: u32,
    visible: bool,
}

impl CandidateUi {
    pub fn new() -> Self {
        CandidateUi {
            cand_client: None,
            client_id: std::process::id(),
            visible: false,
        }
    }

    /// S3/S5 兼容入口：现在恒为 hosted，忽略参数。
    pub fn set_hosted(&mut self, _hosted: bool) {}

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
        let mut view_ctx = ctx.clone();
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
        // hosted 窗口按 1..9/0 编号，最多显示 10 项
        view_ctx.candidates.truncate(10);

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
            if client
                .show(self.client_id, &view_ctx, caret, dpi, multi_line)
                .is_ok()
            {
                self.visible = true;
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
