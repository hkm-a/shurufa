//! TextService：TSF 文本输入处理器主体。
//!
//! 职责：激活时挂接键盘事件 sink，把每个按键翻译为 keysym 经 IPC 客户端转发给
//! 独立算法服务（shurufa-algo），随后把引擎状态（上屏文本 / 预编辑串 / 候选）
//! 同步回文档与候选窗。引擎不在本进程内 —— 用户词库锁冲突由此消除。

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use windows::core::{implement, Interface, Ref, Result, BOOL, GUID};
use windows::Win32::Foundation::{LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse;
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfContextComposition,
    ITfInsertAtSelection, ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr,
    ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl,
    ITfThreadMgr, TfAnchor, INSERT_TEXT_AT_SELECTION_FLAGS, TF_AE_NONE, TF_ANCHOR_END,
    TF_IAS_QUERYONLY, TF_SELECTION, TF_SELECTIONSTYLE, TF_ST_CORRECTION,
};
use windows_core::IUnknownImpl;

use shurufa_options::ImeOptions;

use crate::candidate_window::{CandidatePanelMode, CandidateUi, PositionMode};
use crate::composition::edit_session;
use crate::ipc_client::ImeClient;
use crate::keys;

/// 纯判定：两份选项的 `input_scheme` 是否不同。
/// 供 refresh_options 在每次重载后比对，并供单元测试直接断言。
pub(crate) fn input_scheme_differs(a: &ImeOptions, b: &ImeOptions) -> bool {
    a.input_scheme != b.input_scheme
}

/// 纯判定：Tab/Shift+Tab 在引擎组合活着且有音节分隔时转为 XK_Right/XK_Left。
/// 与 keys.rs 的 vk_to_keysym 同返回型（Some=已决定）；返回 None 时走默认分支。
/// keysym 入参为 vk_to_keysym 已算出的 Tab keysym（0xff09）。
pub(crate) fn remap_tab_key(keysym: i32, shift: bool, has_breaks: bool) -> Option<i32> {
    if !has_breaks {
        return None;
    }
    // XK_TAB = 0xff09；左=0xff51、右=0xff53（与 keys.rs 常量一致）。
    if keysym != 0xff09 {
        return None;
    }
    Some(if shift { 0xff51 } else { 0xff53 })
}

pub struct Inner {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    /// 经 IPC 的引擎会话客户端（懒连接）。Arc<Mutex> 供候选条右键菜单
    /// 钩子与活动组合共用同一会话（新连接会建新会话，空组合无法删词）。
    client: Arc<Mutex<ImeClient>>,
    composition: Option<ITfComposition>,
    ui: CandidateUi,
    /// 仅用于排障日志：本进程是否已收到过按键
    saw_first_key: bool,
    /// 用户选项缓存（options.json；加载失败回退默认）
    opts: ImeOptions,
    /// 最近一次检查 options.json 磁盘变化的时刻
    opts_checked_at: Instant,
    /// 当前已知的 options.json 修改时间（用于热重载判定）
    opts_mtime: Option<SystemTime>,
    /// Shift 长按判定状态：按下时刻（GetTickCount64 毫秒）；None = Shift 未按住。
    /// 与既有 OnKeyDown-eat-Shift 路径不同：本字段纯粹用于 release 时区分
    /// 长/短按，不阻止既有 cn/en toggle 当松开时按短按落地。
    shift_down_at_ms: Option<u64>,
    /// "大写视觉提示"（视觉提示，不切引擎）：长按 release 后置 true；
    /// 下一次短按 Shift release 清零。不影响 IPC 上的 ascii_mode。
    caps_visual: bool,
    /// Shift 中/英切换"挂起"标记：Shift 按下时置位，等（a）松开结算、
    /// （b）下一个非 Shift 组合键结算、或（c）Shift 组合键（大写）取消。
    /// 旧实现 Shift 按下即切——英文模式打大写字母时每个 Shift+字母 都会把
    /// 模式切回中文（输入体验 bug，用户打 "Hello" 打一个 H 就变中文组字）。
    /// 改为按下仅挂起，切不切由后续按键/松开判定。
    shift_toggle_pending: bool,
    /// 输入位置缓存（weasel#1867 手段6 同类）：组合对象未变时复用上次
    /// 锚点，跳过每键 GetActiveView/GetTextExt 的 COM 往返。组合对象
    /// 每次 StartComposition 都是新指针，同一组合会话内指针不变。
    last_comp_ptr: Option<*mut core::ffi::c_void>,
    last_anchor: Option<POINT>,
    /// 按应用选项跟踪（weasel app_options）：记录当前前台应用与
    /// 进入被覆盖应用前的 ascii_mode 快照，应用切换时恢复。
    app_ascii: AppAsciiState,
    /// 「？？？」表情计数（M10 困难项替代实现，见 emoji_question.rs）。
    question_state: crate::emoji_question::QuestionState,
    /// AI 候选提交的 pending 槽：候选窗点击 AI 候选 → 写此槽 + 回发 Enter；
    /// handle_key 入口识别该键（pending 非空时消费），走编辑会话把文本落盘
    /// （不经过引擎——AI 候选不是 librime 候选，索引对不上数字选词）。
    /// 实测 chrome 只把文本相关键路由给 TSF（F9/应用键收不到 OnKeyDown），
    /// Enter 必达且正常回车不受影响（仅 pending 非空时消费）。
    pending_ai: Arc<Mutex<Option<String>>>,
    /// AI 候选 worker（懒启动：AI 开关 + 有 key + 有组合时才创建）。
    ai_worker: Option<Arc<crate::ai_candidates::AiWorker>>,
}

/// Shift 按住时长阈值：超过即视长按（→ 大写视觉提示），否则按既有的
/// 短按 → 中/英 toggle。来源：产品约定 400ms；测试用同函数以保持一致性。
pub(crate) const SHIFT_LONG_PRESS_MS: u64 = 400;

/// 纯函数：按住时长是否越过"长按"阈值。单位毫秒，跨越边界时 `>=`。
/// 测试通过它独立覆盖边界而不必起 Windows 定时器。
pub(crate) fn is_long_press(held_ms: u64) -> bool {
    held_ms >= SHIFT_LONG_PRESS_MS
}

/// Shift release 决策结果（纯函数输出；执行由 Inner::handle_shift_release 完成）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShiftReleaseAction {
    /// 按住时长 ≥ 阈值：启用大写视觉提示；不切 ascii_mode、不动组合。
    LongPressVisualCaps,
    /// 当前已处视觉提示态的短按：仅清除提示，不切中英。
    ClearVisualCaps,
    /// Shift 单独按下并松开（挂起且无组合键介入）：结算中/英切换。
    FireToggle,
    /// 短按且非挂起态（如已随 Shift+字母 取消）：不动作。
    NoToggle,
}

/// 状态机（纯函数）：根据 (held_ms, caps_visual_now, toggle_pending) 推下一步动作。
/// 规则文档化：
/// - held ≥ SHIFT_LONG_PRESS_MS ⇒ LongPressVisualCaps（无视当前 visual/pending 状态）
/// - held  < 阈值 且 caps_visual 已激活 ⇒ ClearVisualCaps
/// - held  < 阈值 且未激活 且 pending ⇒ FireToggle（Shift 单独使用）
/// - held  < 阈值 且未激活 且非 pending ⇒ NoToggle（切换已随组合键取消/结算）
pub(crate) fn decide_shift_release(
    held_ms: u64,
    caps_visual: bool,
    toggle_pending: bool,
) -> ShiftReleaseAction {
    if is_long_press(held_ms) {
        ShiftReleaseAction::LongPressVisualCaps
    } else if caps_visual {
        ShiftReleaseAction::ClearVisualCaps
    } else if toggle_pending {
        ShiftReleaseAction::FireToggle
    } else {
        ShiftReleaseAction::NoToggle
    }
}

#[implement(ITfTextInputProcessorEx, ITfKeyEventSink, ITfCompositionSink)]
pub struct TextService {
    inner: RefCell<Inner>,
}

/// 前台是否处于安全桌面（锁屏 / UAC consent / 登录）。
///
/// 判定方法二选一即为安全桌面：
/// - GetForegroundWindow 为空（桌面切换中、锁屏触发的瞬间）；
/// - 前台窗口所在进程名属于安全外壳集合（LogonUI / consent / SecureFolder 等）。
///
/// 任何一环 API 失败都按"安全"处理（保守错杀），避免在未知状态下吞按键。
fn is_secure_desktop() -> bool {
    match foreground_path() {
        None => true,
        Some(path) => {
            let lower = path.to_ascii_lowercase();
            const SECURE: &[&str] = &[
                "\\logonui.exe",
                "\\consent.exe",
                "\\securesystemfolder",
                "\\credentialuibroker.exe",
            ];
            SECURE.iter().any(|p| lower.ends_with(p))
        }
    }
}

/// 前台窗口所在进程的完整路径（小写）。
fn foreground_path() -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let Ok(proc_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return None;
        };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let got = QueryFullProcessImageNameW(
            proc_handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(proc_handle);
        if got.is_err() {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Some(path.to_ascii_lowercase())
    }
}

/// 前台窗口所在进程的可执行文件名（小写，如 "windowsterminal.exe"）。
/// 用于按应用选项（app_options）的前台应用判定。任何一步 API 失败
/// 返回 None（保守：宁可不触发覆盖，也不误判）。
fn foreground_app_name() -> Option<String> {
    foreground_path().map(|path| path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned())
}

/// 按应用选项（weasel app_options）的前台应用跟踪状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppAsciiState {
    /// 最近一次识别到的前台应用（小写文件名）；None = 尚未识别。
    current_app: Option<String>,
    /// 进入"被覆盖应用"那一刻的全局 ascii_mode 快照；离开该应用时恢复。
    /// None = 当前应用没有被覆盖（或快照尚未建立）。
    snapshot: Option<bool>,
}

impl AppAsciiState {
    pub(crate) fn new() -> Self {
        Self {
            current_app: None,
            snapshot: None,
        }
    }
}

/// 按应用 ascii_mode 覆盖的决策结果（纯函数输出；执行由 Inner 完成）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppAsciiAction {
    /// 切到被覆盖的应用：把引擎 ascii_mode 设为覆盖值，并记录进入前快照。
    ApplyOverride { value: bool },
    /// 离开被覆盖的应用：恢复进入前快照（若快照存在）。
    RestoreSnapshot { value: bool },
    /// 应用没变或没有覆盖：不动作。
    NoAction,
}

/// 纯函数：根据前台应用变化与覆盖表，推下一步按应用 ascii_mode 动作。
///
/// 规则（weasel app_options 语义）：
/// - 应用没变（old == new）⇒ NoAction（不打扰同应用内的手动切换）；
/// - 新应用有覆盖 ⇒ ApplyOverride（快照在调用方保存，见下方返回语义）；
/// - 新应用无覆盖 且 旧应用有覆盖（即刚离开被覆盖应用）⇒ RestoreSnapshot；
/// - 新应用无覆盖 且 旧应用也无覆盖 ⇒ NoAction（纯切换，不动作）。
///
/// `overrides` 是 app_options 映射（进程名小写 → 选项）；`had_snapshot` 表示
/// 调用方是否在旧应用上保存过快照（= 旧应用曾被覆盖）。返回 `ApplyOverride`
/// 时调用方必须保存当前 ascii_mode 为快照；返回 `RestoreSnapshot` 时调用方
/// 必须用返回的 value 恢复并清空快照。
pub(crate) fn decide_app_ascii(
    old_app: &Option<String>,
    new_app: &Option<String>,
    overrides: &std::collections::BTreeMap<String, shurufa_options::AppOption>,
    had_snapshot: bool,
) -> AppAsciiAction {
    if old_app == new_app || new_app.is_none() {
        return AppAsciiAction::NoAction;
    }
    let new_override = new_app
        .as_ref()
        .and_then(|n| overrides.get(n))
        .and_then(|o| o.ascii_mode);
    match new_override {
        Some(value) => AppAsciiAction::ApplyOverride { value },
        None if had_snapshot => AppAsciiAction::RestoreSnapshot { value: false },
        None => AppAsciiAction::NoAction,
    }
}

/// 仅测试可见：直接解析前台进程名（不依赖真实前台窗口）。
#[cfg(test)]
fn app_name_from_path(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    Some(name.to_ascii_lowercase())
}

/// vim 的"回 normal 模式键"（weasel RimeWithWeasel.cpp:274-287 同款）：
/// Esc，或 Ctrl+C / Ctrl+[。这些键按下时，vim_mode 应用应切回英文直输，
/// 让 vim/emacs 拿到按键（否则输入法会留在中文态吃掉后续 normal 键）。
pub(crate) fn is_vim_normal_mode_key(vk: u32, ctrl: bool) -> bool {
    const VK_ESCAPE: u32 = 0x1B;
    const VK_C: u32 = 0x43;
    const VK_OPEN_BRACKET: u32 = 0xDB;
    vk == VK_ESCAPE || (ctrl && (vk == VK_C || vk == VK_OPEN_BRACKET))
}

/// 纯函数：当前应用是否启用了 vim_mode（weasel app_options vim_mode）。
/// `overrides` 为 app_options 映射；当前应用无条目或字段为 None 时返回
/// false（跟随全局，不启用）。
pub(crate) fn app_vim_mode_enabled(
    app: &Option<String>,
    overrides: &std::collections::BTreeMap<String, shurufa_options::AppOption>,
) -> bool {
    app.as_ref()
        .and_then(|name| overrides.get(name))
        .and_then(|opt| opt.vim_mode)
        .unwrap_or(false)
}

impl TextService {
    pub fn new() -> Self {
        let client = Arc::new(Mutex::new(ImeClient::new()));
        // 候选条右键菜单的引擎动作钩子：与活动组合共用同一会话客户端。
        // 闭包持有 Arc，服务实例生命周期内有效；同一 TSF UI 线程上
        // wnd_proc 与 handle_key 顺序执行，锁零争用。
        {
            let shared = Arc::clone(&client);
            crate::candidate_window::set_engine_simulate(Box::new(move |keys| {
                shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .simulate(keys)
            }));
        }
        // AI 候选提交钩子（2026-08-20）：候选窗点击 AI 候选时写入 pending
        // 槽并回发 Enter，由 handle_key 入口在编辑会话内提交（OnKeyDown
        // 的 context 可靠指向焦点文档；直接提交经 GetTop 在组合被 chrome
        // 终止后插入位置错误，实测 textarea 为空）。
        let pending_ai = Arc::new(Mutex::new(None::<String>));
        {
            let slot = Arc::clone(&pending_ai);
            crate::candidate_window::set_ai_commit(Box::new(move |text| {
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_owned());
                false
            }));
        }
        TextService {
            inner: RefCell::new(Inner {
                thread_mgr: None,
                client_id: 0,
                client,
                composition: None,
                ui: CandidateUi::new(),
                saw_first_key: false,
                opts: shurufa_options::load(),
                opts_checked_at: Instant::now(),
                opts_mtime: None,
                shift_down_at_ms: None,
                caps_visual: false,
                shift_toggle_pending: false,
                last_comp_ptr: None,
                last_anchor: None,
                app_ascii: AppAsciiState::new(),
                question_state: crate::emoji_question::QuestionState::default(),
                pending_ai,
                ai_worker: None,
            }),
        }
    }
}

impl Inner {
    /// AI 候选预测请求（2026-08-20，见 docs/AI候选预测方案.md）：开关开 +
    /// 有 AGNES_API_KEY + 中文态 + 有组合时，把当前 preedit 投递给 worker
    ///（缓冲 1，快打丢旧保新；worker 800ms 停顿后调 agnès，结果经
    /// PostMessage 回候选窗刷新）。失败/超时/无 key 一律静默，绝不影响输入。
    fn maybe_request_ai(&mut self, ctx: &ime_ipc::Context) {
        if !self.opts.ai_candidates || crate::ai_candidates::api_key().is_none() {
            return;
        }
        let preedit = ctx.preedit.trim().to_owned();
        if preedit.is_empty() || ctx.is_ascii {
            return;
        }
        if self.ai_worker.is_none() {
            crate::debug_log("AI 候选 worker 首次启用");
            self.ai_worker = Some(crate::ai_candidates::AiWorker::spawn());
        }
        let hwnd = crate::candidate_window::current_hwnd()
            .map(|h| h.0 as usize)
            .unwrap_or(0);
        self.ai_worker
            .as_ref()
            .expect("worker 已创建")
            .request(preedit, hwnd);
    }

    /// 至多每 2 秒检查一次 options.json 的修改时间，变了就重载。
    /// 不主动向引擎推状态（全角/标点默认由 schema 决定），只影响快捷键行为。
    fn refresh_options(&mut self) {
        if self.opts_checked_at.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.opts_checked_at = Instant::now();
        let mtime = std::fs::metadata(shurufa_options::path())
            .and_then(|m| m.modified())
            .ok();
        if mtime != self.opts_mtime {
            self.opts_mtime = mtime;
            let previous = std::mem::replace(&mut self.opts, shurufa_options::load());
            crate::debug_log(&format!("选项已重载：{:?}", self.opts));
            // wave 4：方案切换仅记录日志，不做热交换。
            // 真正的 librime schema redeploy 由 shurufa-algo 侧的 watcher 接管（wave 5）。
            if input_scheme_differs(&previous, &self.opts) {
                crate::debug_log(&format!(
                    "input scheme change detected: {} → {}（当前版本需要重启输入法生效）",
                    previous.input_scheme, self.opts.input_scheme
                ));
            }
        }
    }

    /// 按应用选项（weasel app_options）：前台应用变化时，按覆盖表应用
    /// ascii_mode 覆盖（进入被覆盖应用）或恢复快照（离开被覆盖应用）。
    ///
    /// 只在应用真的变化时动作（decide_app_ascii 判定）；同应用内用户手动
    /// Shift 切换不受打扰。读取引擎当前 ascii_mode 走 IPC（get_option），
    /// 覆盖/恢复走 set_option。
    fn apply_app_override(&mut self) {
        let Some(new_app) = foreground_app_name() else {
            return;
        };
        let had_snapshot = self.app_ascii.snapshot.is_some();
        let action = decide_app_ascii(
            &self.app_ascii.current_app,
            &Some(new_app.clone()),
            &self.opts.app_options,
            had_snapshot,
        );
        match action {
            AppAsciiAction::ApplyOverride { value } => {
                // 进入被覆盖应用：记录进入前的 ascii_mode 为快照，再应用覆盖。
                let before = self
                    .client
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_option("ascii_mode")
                    .unwrap_or(false);
                self.app_ascii.snapshot = Some(before);
                self.app_ascii.current_app = Some(new_app.clone());
                if self
                    .client
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .set_option("ascii_mode", value)
                {
                    crate::debug_log(&format!(
                        "按应用选项：{new_app} 应用 ascii_mode={value}（快照 {before}）"
                    ));
                }
            }
            AppAsciiAction::RestoreSnapshot { value } => {
                // 离开被覆盖应用：恢复进入前的快照，并清空状态。
                let was = self.app_ascii.snapshot.take();
                self.app_ascii.current_app = Some(new_app.clone());
                if let Some(snap) = was {
                    if self
                        .client
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .set_option("ascii_mode", snap)
                    {
                        crate::debug_log(&format!(
                            "按应用选项：离开被覆盖应用，恢复 ascii_mode={snap}（原覆盖值 {value}）"
                        ));
                    }
                }
            }
            AppAsciiAction::NoAction => {
                self.app_ascii.current_app = Some(new_app);
            }
        }
    }

    /// 结束文档侧残留的 TSF 组合（切换中英文/标点/全角前的收尾，
    /// 否则残留组合会把后续按键吃进去）。
    fn end_pending_composition(&mut self, context: &ITfContext) {
        if let Some(comp) = self.composition.take() {
            let client_id = self.client_id;
            if let Err(e) = edit_session(client_id, context, |ec| unsafe {
                set_composition_text(&comp, ec, "", 0)?;
                comp.EndComposition(ec)
            }) {
                crate::debug_log(&format!("结束残留组合失败：{e:?}"));
            }
        }
    }

    /// 模式切换反馈：在输入锚点附近弹出 toast（候选窗不可见时的反馈通道，
    /// 微信/搜狗模式提示同类；无锚点落主屏底部居中）。
    fn toast_mode(&self, text: &str) {
        crate::toast::show(text, self.last_anchor);
    }

    /// 喂键给引擎并同步文档/候选窗；返回该键是否被输入法吃掉。
    fn handle_key(
        &mut self,
        sink: &ITfCompositionSink,
        context: &ITfContext,
        wparam: WPARAM,
    ) -> bool {
        let vk = wparam.0 as u32;
        let modifiers = keys::current_modifiers();
        let shift = modifiers & keys::MASK_SHIFT != 0;
        let ctrl = modifiers & keys::MASK_CONTROL != 0;
        let alt = modifiers & keys::MASK_ALT != 0;

        // AI 候选提交（Enter 触发，2026-08-20）：候选窗点击 AI 候选时回发
        // Enter，本入口识别后直接把文本落盘（结束组合 + 插入文档；不经过
        // 引擎数字选词——AI 候选不是 librime 候选，索引对不上）。chrome 只
        // 把文本相关键路由给 TSF，Enter 必达；仅 pending_ai 非空时消费，
        // 正常回车不受影响。
        if vk == 0x0D {
            if let Some(text) = self
                .pending_ai
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
            {
                let text = text.trim_start_matches('🤖').to_owned();
                let client_id = self.client_id;
                let composition_slot = &mut self.composition;
                let edit_result = edit_session(client_id, context, |ec| {
                    unsafe {
                        if let Some(comp) = composition_slot.take() {
                            set_composition_text(&comp, ec, &text, text.encode_utf16().count())?;
                            comp.EndComposition(ec)?;
                        } else {
                            insert_text(context, ec, &text)?;
                        }
                    }
                    Ok(())
                });

                if let Err(e) = &edit_result {
                    crate::debug_log(&format!("AI 候选提交失败：{e:?}"));
                } else {
                    crate::debug_log(&format!("AI 候选提交：{text:?}"));
                }
                self.ui.hide();
                self.last_comp_ptr = None;
                self.last_anchor = None;
            }
            return true;
        }

        // 「？？？」表情（M10）：非斜杠键重置连续问号计数。
        self.question_state.reset();

        // 按应用选项（weasel app_options）：前台应用变化时按覆盖表应用
        // ascii_mode 覆盖 / 恢复快照。只在应用真的变化时动作，同应用内的
        // 手动 Shift 切换不受打扰。
        self.apply_app_override();

        // vim 模式（weasel app_options vim_mode 同款，2026-08-18 引入）：
        // 该应用配置 vim_mode 后，无组合时按 vim 的"回 normal 模式键"
        // （Esc / Ctrl+C / Ctrl+[）自动切英文直输——vim/emacs/终端才能
        // 拿到这些键进入 normal 模式（否则输入法留在中文态吃掉后续
        // j/k/l 等 normal 键）。有组合时跳过：Esc 由引擎取消组合（与
        // weasel 的 !handled 判定等价），不抢不切。
        if self.composition.is_none()
            && is_vim_normal_mode_key(vk, ctrl)
            && app_vim_mode_enabled(&self.app_ascii.current_app.clone(), &self.opts.app_options)
            && self
                .client
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_option("ascii_mode")
                == Some(false)
            && self
                .client
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .set_option("ascii_mode", true)
        {
            crate::debug_log("vim_mode：Esc/Ctrl+C/Ctrl+[ 回 normal 模式，切英文直输");
            self.toast_mode("英文直输");
        }

        // Shift 单独按下：挂起中/英切换（不立即切），收尾残留组合，并记录
        // 按下时刻。吃掉该键，否则系统会走默认中英文切换，导致双触发。
        //
        // 为什么"挂起"而非按下即切：按下即切会让英文模式下打大写字母
        // （Shift+字母）把模式误切回中文——用户打 "Hello" 第一个 H 就被切回
        // 中文组字（输入体验 bug）。挂起后由三种路径结算：
        //   - 松开（有 TestKeyUp 的宿主）：短按 → 切换落地；
        //   - 下一个不带 Shift 的键（无 TestKeyUp 的宿主兜底）：切换落地；
        //   - 下一个带 Shift 的键（大写/上档符号）：取消挂起，不切换。
        if vk == KeyboardAndMouse::VK_SHIFT.0 as u32 {
            if !self.opts.shift_switch_cn_en {
                return false;
            }
            self.shift_down_at_ms =
                Some(unsafe { windows::Win32::System::SystemInformation::GetTickCount64() });
            self.shift_toggle_pending = true;
            // 收尾残留组合（主流输入法一致：Shift 提交拼音）
            self.end_pending_composition(context);
            crate::debug_log("Shift 按下：挂起中英文切换，待松开/组合键结算");
            return true;
        }

        // ---- Shift 挂起结算：按下 Shift 后的第一个按键决定切换是否落地 ----
        // - 该键带 Shift（大写/上档符号/Shift+方向选择）：Shift 被用作组合键，
        //   取消挂起（这次 Shift 不是切换意图）；
        // - 该键不带 Shift（Shift 已松开）：Shift 被单独使用 → 此刻结算切换。
        //   （无 TestKeyUp 的宿主收不到 release，这里兜底；有 TestKeyUp 的宿主
        //   handle_shift_release 已优先结算并清掉挂起，走到这里已是 false。）
        if self.shift_toggle_pending {
            self.shift_toggle_pending = false;
            if !shift {
                self.shift_down_at_ms = None;
                if let Some(is_ascii) = self
                    .client
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .toggle_ascii()
                {
                    crate::debug_log(&format!(
                        "Shift 挂起结算（后续按键无 Shift）：ascii={is_ascii}"
                    ));
                    self.toast_mode(if is_ascii {
                        "英文直输"
                    } else {
                        "中文输入"
                    });
                }
            } else {
                crate::debug_log("Shift 挂起取消（后续按键带 Shift，视为组合键）");
            }
        }
        // CapsLock：开启选项时切到英文直输（只进不出，回中文用 Shift）。
        // 吃掉该键后系统不再翻转大写灯（OnTestKeyDown 已声明接管）。
        if vk == KeyboardAndMouse::VK_CAPITAL.0 as u32 && self.opts.capslock_to_english {
            self.end_pending_composition(context);
            if self
                .client
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_option("ascii_mode")
                == Some(false)
            {
                let is_ascii = self
                    .client
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .toggle_ascii()
                    .unwrap_or(false);
                crate::debug_log(&format!("CapsLock 切英文直输：ascii={is_ascii}"));
                self.toast_mode("英文直输");
            }
            return true;
        }
        // Shift+Space：无组合时切换全/半角；有组合时按普通空格交给引擎。
        if vk == KeyboardAndMouse::VK_SPACE.0 as u32
            && shift
            && !ctrl
            && !alt
            && self.opts.shift_space_full_shape
            && self.composition.is_none()
        {
            let current = self
                .client
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_option("full_shape")
                .unwrap_or(false);
            let next = !current;
            let ok = self
                .client
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .set_option("full_shape", next);
            crate::debug_log(&format!(
                "Shift+Space 切换全/半角：full_shape={next} ok={ok}"
            ));
            self.toast_mode(if next { "全角" } else { "半角" });
            return true;
        }
        // Ctrl+.：切换中/英标点（ascii_punct）。必须放在 Ctrl/Alt 直通判断之前。
        if vk == 0xBE && ctrl && !alt && self.opts.ctrl_period_ascii_punct {
            self.end_pending_composition(context);
            let current = self
                .client
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_option("ascii_punct")
                .unwrap_or(false);
            let next = !current;
            let ok = self
                .client
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .set_option("ascii_punct", next);
            crate::debug_log(&format!("Ctrl+. 切换中/英标点：ascii_punct={next} ok={ok}"));
            self.toast_mode(if next { "英文标点" } else { "中文标点" });
            return true;
        }
        // 「？？？」表情（M10 困难项替代实现，emoji_question.rs）：中文态、
        // 无组合的 Shift+/（0xBF）由 TSF 接管——前两个上屏全角「？」（顺带
        // 修正中文态 Shift+/ 被既有直通分支上成半角 "?" 的瑕疵），第三个上屏
        // 🤔。零回退风险：librime 组合无法累积标点（实测 / 上屏「、」、
        // 无组合），TSF 自建组合会与引擎抢 composition。
        if vk == 0xBF
            && shift
            && !ctrl
            && !alt
            && self.composition.is_none()
            && self
                .client
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_option("ascii_mode")
                == Some(false)
        {
            let action = self.question_state.on_slash();
            let text = match action {
                crate::emoji_question::QuestionAction::EmitQuestion => "？",
                crate::emoji_question::QuestionAction::EmitEmoji => crate::emoji_question::EMOJI,
            };
            let client_id = self.client_id;
            let _ = edit_session(client_id, context, |ec| unsafe {
                insert_text(context, ec, text)
            });
            crate::debug_log(&format!("？？？表情：上屏 {text}"));
            return true;
        }

        // 符号配对（微信输入法同类，默认关）：中文态、无组合时按配对键 →
        // 插入配对符号并把光标居中（`(` → `()` 光标中间）。必须放在
        // Shift+可打印键分支之前——US 键盘上 ( [ { 是 Shift+数字/括号，
        // 否则会被那个分支截胡只插入单个字符。默认关避免与 IDE 自动补全
        // 冲突；仅在 ascii_mode=false（中文态）时生效。
        if self.opts.symbol_pairing
            && self.composition.is_none()
            && !ctrl
            && !alt
            && self
                .client
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_option("ascii_mode")
                == Some(false)
        {
            if let Some(keysym) = keys::vk_to_keysym(vk, shift) {
                if let Some((pair, cursor)) = symbol_pair_for(keysym) {
                    let client_id = self.client_id;
                    let _ = edit_session(client_id, context, |ec| unsafe {
                        insert_paired(context, ec, pair, cursor)
                    });
                    crate::debug_log(&format!(
                        "符号配对：{} → {}（光标 {cursor}）",
                        char::from_u32(keysym as u32).unwrap_or('?'),
                        pair
                    ));
                    return true;
                }
            }
        }
        // Shift+可打印字符（大写字母/上档符号）：直接上屏，不进 rime。
        // 中文态打 "Hello 世界" 的 H（Shift+H）必须立即上屏，否则 rime 会把它
        // 收进组字串（实测 preedit='H'，无候选、不自动提交）；英文态同理避免
        // 误触发组字。纯 TSF 落盘，引擎挂掉也能用。Shift 已在挂起结算分支
        // 按组合键取消，此路径不再涉及中英切换。
        if shift
            && !ctrl
            && !alt
            && self.composition.is_none()
            && (0x20..=0x7E).contains(&keys::vk_to_keysym(vk, true).unwrap_or(0))
        {
            if let Some(keysym) = keys::vk_to_keysym(vk, true) {
                if let Some(ch) = char::from_u32(keysym as u32) {
                    let text = ch.to_string();
                    let client_id = self.client_id;
                    let _ = edit_session(client_id, context, |ec| unsafe {
                        insert_text(context, ec, &text)
                    });
                    crate::debug_log(&format!("Shift+可打印键直接上屏：{ch:?}"));
                    return true;
                }
            }
        }
        // Ctrl/Alt 组合键与不认识的键一律放行
        if modifiers & (keys::MASK_CONTROL | keys::MASK_ALT) != 0 {
            return false;
        }
        // Tab/Shift+Tab 音节光标导航：composition 活着且引擎实时 preedit 有
        // 音节分隔符时把 Tab 重映射为 XK_Left/XK_Right，让引擎光标按音节步进；
        // 否则透传 Tab。0 新 IPC message：仅换 keysym，引擎侧走既有 librime
        // cursor 处理。
        //
        // 关键：**不能依赖渲染快照**（PAINT_DATA 是上一帧 show() 的快照，按键时
        // 可能已过期）。这里实时向引擎查一次 context 取 preedit 的分隔符。
        let tab_remap_candidates =
            if vk == KeyboardAndMouse::VK_TAB.0 as u32 && self.composition.is_some() {
                let live_breaks = self
                    .client
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .context()
                    .map(|ctx| !crate::candidate_window::syllable_breaks(&ctx.preedit).is_empty())
                    .unwrap_or(false);
                remap_tab_key(0xff09, shift, live_breaks)
            } else {
                None
            };
        let Some(keysym) = tab_remap_candidates.or_else(|| keys::vk_to_keysym(vk, shift)) else {
            // 引擎连接失败：把当前按键作为原字符落入文档（中文兜底），
            // 避免“只能输入英文”。
            let _ = self.fallback_commit(context, vk, shift);
            return false;
        };
        // R2.1 打点起点：必须在 process_key 之前（含引擎 IPC + 算法 + commit 全程），
        // 否则量到的只是"写文本"一段，对于 commit 路径（快路径）误差大。
        // 2026-08-16 起默认开启（debug_log 已改内存缓冲，零 I/O）；卡顿排查时
        // 直接分析日志 LAT 行。设 SHURUFA_DISABLE_LATENCY_LOG=1 可关闭。
        let probe_q0 = if std::env::var_os("SHURUFA_DISABLE_LATENCY_LOG").is_none() {
            let mut q = 0i64;
            unsafe {
                let _ = windows::Win32::System::Performance::QueryPerformanceCounter(&mut q);
            }
            Some(q as u64)
        } else {
            None
        };

        let Some((eaten, commit, ctx)) = self
            .client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .process_key(keysym, modifiers)
        else {
            // 引擎连接失败：把当前按键作为原字符落入文档，避免“只能输入英文”。
            let _ = self.fallback_commit(context, vk, shift);
            crate::debug_log("引擎 IPC 不可用，按键直通");
            return false;
        };
        // 防御双字：引擎返回了上屏文本却声称未吃掉（异常/边界态）时，只要落了盘
        // 就必须吃掉该键，否则应用会再处理一次 → 双字（曾现 "你好你好" 类问题）。
        let eaten = eaten || commit.is_some();

        crate::debug_log(&format!(
            "键 vk=0x{:X} keysym=0x{:X} eaten={} commit={:?} preedit={:?}",
            wparam.0, keysym, eaten, commit, ctx.preedit
        ));

        // AI 候选预测（2026-08-20）：候选刷新时把 preedit 投递给 worker
        //（800ms 停顿后调 agnès；开关关/无 key/无组合/英文直输一律跳过）。
        self.maybe_request_ai(&ctx);

        let has_preedit = !ctx.preedit.is_empty();
        let client_id = self.client_id;

        // 文档更新必须进入编辑会话
        let composition_slot = &mut self.composition;
        let ui = &mut self.ui;
        let edit_result = edit_session(client_id, context, |ec| {
            unsafe {
                // 1. 上屏文本：结束组合并以最终文本落盘（M8-4 直达候选除外）
                if let Some(text) = commit.as_deref() {
                    if let Some(shortcut) = crate::direct_launch::resolve_commit(text) {
                        // 应用/网站直达：清空组合但不落文本，启动目标。
                        crate::debug_log(&format!(
                            "直达候选提交（不落盘）：{} → {}",
                            shortcut.label, shortcut.target
                        ));
                        if let Some(comp) = composition_slot.take() {
                            set_composition_text(&comp, ec, "", 0)?;
                            comp.EndComposition(ec)?;
                        }
                        if let Err(e) = crate::direct_launch::spawn_target(&shortcut) {
                            crate::debug_log(&format!("直达启动失败：{e}"));
                        }
                    } else if let Some(comp) = composition_slot.take() {
                        set_composition_text(&comp, ec, text, text.encode_utf16().count())?;
                        comp.EndComposition(ec)?;
                    } else {
                        insert_text(context, ec, text)?;
                    }
                    // R2.1：编辑会话内完成落盘的时点打点（之上往下 1 步即可与
                    // sender 时间对应）
                    if let Some(q0) = probe_q0 {
                        let mut q1: i64 = 0;
                        let _ =
                            windows::Win32::System::Performance::QueryPerformanceCounter(&mut q1);
                        let mut freq: i64 = 0;
                        let _ = windows::Win32::System::Performance::QueryPerformanceFrequency(
                            &mut freq,
                        );
                        let elapsed_us = if freq > 0 {
                            (q1 - q0 as i64) * 1_000_000 / freq
                        } else {
                            0
                        };
                        crate::debug_log(&format!(
                            "LAT commit keysym=0x{:X} chars={} elapsed_us={} q0={} q1={}",
                            keysym,
                            text.chars().count(),
                            elapsed_us,
                            q0,
                            q1
                        ));
                    }
                }

                // 2. 预编辑串：保证组合存在并刷新内容
                if has_preedit {
                    if composition_slot.is_none() {
                        *composition_slot = Some(start_composition(context, ec, sink)?);
                    }
                    if let Some(comp) = composition_slot.as_ref() {
                        set_composition_text(comp, ec, &ctx.preedit, ctx.cursor_pos)?;
                    }
                } else if let Some(comp) = composition_slot.take() {
                    // 引擎已无组合（如 Esc 清空），结束并清除文档中的预编辑
                    set_composition_text(&comp, ec, "", 0)?;
                    comp.EndComposition(ec)?;
                }

                // 3. 候选窗：跟随组合文本位置（或按位置策略固定）
                if has_preedit && !ctx.candidates.is_empty() {
                    let position = PositionMode::from_option(&self.opts.candidate_position);
                    let panel_mode =
                        CandidatePanelMode::from_option(&self.opts.candidate_panel_mode);
                    // 固定模式无需锚点：直接复用缓存或清空，跳过 COM 往返。
                    let anchor = if position == PositionMode::Follow {
                        match composition_slot.as_ref() {
                            Some(comp) => {
                                let ptr = Interface::as_raw(comp);
                                if self.last_comp_ptr == Some(ptr) {
                                    self.last_anchor
                                } else {
                                    let anchor = composition_anchor(context, comp, ec);
                                    self.last_comp_ptr = Some(ptr);
                                    self.last_anchor = anchor;
                                    anchor
                                }
                            }
                            None => {
                                self.last_comp_ptr = None;
                                self.last_anchor = None;
                                None
                            }
                        }
                    } else {
                        // 固定模式：不需要组合位置，清空缓存避免误复用
                        self.last_comp_ptr = None;
                        self.last_anchor = None;
                        None
                    };
                    ui.show(&ctx, anchor, position, panel_mode);
                } else {
                    ui.hide();
                    self.last_comp_ptr = None;
                    self.last_anchor = None;
                }
                Ok(())
            }
        });
        if let Err(e) = &edit_result {
            crate::debug_log(&format!("编辑会话失败：{e:?}"));
        }
        eaten
    }

    fn abort_composition(&mut self) {
        // 清空引擎侧组合状态；文档侧组合由 TSF 生命周期回调负责
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .simulate("{Escape}");
        self.composition = None;
        self.ui.hide();
    }

    /// Shift release 分流：长按 → 视觉大写角标；短按 + 挂起 → 结算中/英切换；
    /// 短按 + 视觉态 → 解除角标。返回 true = 已吃掉（按键路径上不再交还系统）。
    ///
    /// 实现约定：
    /// - 未在按下时记录时刻（None）说明 Shift 是被选项关着/直接跳过的，
    ///   release 直通不动作。
    /// - **切换结算**：切换在 Shift 单独使用时才落地（按下时只挂起，见 handle_key）。
    ///   若按下后跟了 Shift 组合键（大写），挂起已被取消，这里走 NoToggle 不切。
    ///   无 TestKeyUp 的宿主（多数应用）收不到本回调，由 handle_key 的
    ///   "挂起结算"（下一个非 Shift 键）兜底，两种宿主行为一致。
    /// - 长按路径**不**调 ToggleAscii、**不**改组合：仅设 caps_visual + 触发候选窗重绘。
    fn handle_shift_release(&mut self, sink: &ITfCompositionSink, context: &ITfContext) -> bool {
        let Some(down_at) = self.shift_down_at_ms.take() else {
            return false;
        };
        let held = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
            .saturating_sub(down_at);
        match decide_shift_release(held, self.caps_visual, self.shift_toggle_pending) {
            ShiftReleaseAction::LongPressVisualCaps => {
                // 长按：视觉大写提示，不切引擎；挂起一并取消（长按不是切换意图）
                self.shift_toggle_pending = false;
                crate::debug_log(&format!(
                    "Shift 长按 {}ms ≥ {}ms：设大写视觉提示",
                    held, SHIFT_LONG_PRESS_MS
                ));
                self.caps_visual = true;
                if crate::candidate_window::set_caps_visual(true) {
                    self.ui.invalidate();
                }
                true
            }
            ShiftReleaseAction::ClearVisualCaps => {
                self.shift_toggle_pending = false;
                crate::debug_log("Shift 短按：清除大写视觉提示");
                self.caps_visual = false;
                if crate::candidate_window::set_caps_visual(false) {
                    self.ui.invalidate();
                }
                let _ = sink;
                let _ = context;
                true
            }
            ShiftReleaseAction::FireToggle => {
                // Shift 单独按下并松开（短按，中间无其它键）：此刻结算中/英切换。
                // 按下时已收尾残留组合，这里只切换引擎态。
                self.shift_toggle_pending = false;
                crate::debug_log(&format!("Shift 短按松开：结算中英文切换（held={held}ms）"));
                if let Some(is_ascii) = self
                    .client
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .toggle_ascii()
                {
                    crate::debug_log(&format!("  ascii={is_ascii}"));
                    self.toast_mode(if is_ascii {
                        "英文直输"
                    } else {
                        "中文输入"
                    });
                }
                let _ = sink;
                let _ = context;
                true
            }
            ShiftReleaseAction::NoToggle => {
                // 短按 + 非挂起（如已随 Shift+字母 取消）：不动作
                crate::debug_log(&format!("Shift 短按松开（无挂起切换）held={held}ms"));
                let _ = sink;
                let _ = context;
                true
            }
        }
    }

    /// 引擎服务不可用时，把当前按键作为原字符落入文档（中文兜底）。
    /// 这样即使算法服务崩溃，用户也能继续输入中文而非被迫切回英文。
    fn fallback_commit(&mut self, context: &ITfContext, vk: u32, shift: bool) -> Result<()> {
        let ch: char = match vk {
            0x41..=0x5A => char::from_u32(vk + if shift { 0 } else { 0x20 }).unwrap_or('a'),
            0x30..=0x39 => char::from_u32(vk).unwrap_or('0'),
            _ => ' ',
        };
        let text = ch.to_string();
        let client_id = self.client_id;
        edit_session(client_id, context, |ec| unsafe {
            insert_text(context, ec, &text)
        })
    }
}

/// 在当前选区插入文本（无组合时的直接上屏路径）。
unsafe fn insert_text(context: &ITfContext, ec: u32, text: &str) -> Result<()> {
    let insert: ITfInsertAtSelection = context.cast()?;
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let range = insert.InsertTextAtSelection(ec, INSERT_TEXT_AT_SELECTION_FLAGS(0), &utf16)?;
    // 光标移到插入文本之后
    range.Collapse(ec, TF_ANCHOR_END)?;
    set_selection(context, ec, &range)?;
    Ok(())
}

/// 符号配对表：keysym → (配对文本, 光标位置 UTF-16 码元数)。
/// 覆盖常见开括号/引号；光标居中（如 "()" 光标在 1 处）。
const SYMBOL_PAIRS: &[(i32, (&str, usize))] = &[
    // '(' U+0028 / ')' U+0029
    (0x0028, ("()", 1)),
    // '[' U+005B / ']' U+005D
    (0x005B, ("[]", 1)),
    // '{' U+007B / '}' U+007D
    (0x007B, ("{}", 1)),
    // '《' U+300A / '》' U+300B
    (0x300A, ("《》", 1)),
];

/// 插入配对符号并把光标居中（微信输入法"符号配对"同款）。
/// 只插入一对文本 + SetSelection 光标到中间，纯 TSF 落盘，无引擎交互。
unsafe fn insert_paired(context: &ITfContext, ec: u32, pair: &str, cursor: usize) -> Result<()> {
    let insert: ITfInsertAtSelection = context.cast()?;
    let utf16: Vec<u16> = pair.encode_utf16().collect();
    let range = insert.InsertTextAtSelection(ec, INSERT_TEXT_AT_SELECTION_FLAGS(0), &utf16)?;
    // 光标移到 pair 的 cursor 位置（UTF-16 码元数）：先从起点（TF_ANCHOR_START）
    // 向右移 cursor 个码元。与 set_composition_text 的 ShiftStart 同法。
    range.Collapse(ec, TfAnchor(0))?;
    let mut actual = 0i32;
    let haltcond = windows::Win32::UI::TextServices::TF_HALTCOND::default();
    range.ShiftEnd(ec, cursor as i32, &mut actual, &haltcond)?;
    set_selection(context, ec, &range)?;
    Ok(())
}

/// 纯函数：按 keysym 查符号配对；命中返回 (配对文本, 光标位置)。
pub(crate) fn symbol_pair_for(keysym: i32) -> Option<(&'static str, usize)> {
    SYMBOL_PAIRS
        .iter()
        .find(|(k, _)| *k == keysym)
        .map(|(_, pair)| *pair)
}

/// 在插入点建立新组合。
unsafe fn start_composition(
    context: &ITfContext,
    ec: u32,
    sink: &ITfCompositionSink,
) -> Result<ITfComposition> {
    let insert: ITfInsertAtSelection = context.cast()?;
    let range = insert.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])?;
    let composition_ctx: ITfContextComposition = context.cast()?;
    composition_ctx.StartComposition(ec, &range, sink)
}

/// 用 `text` 替换组合范围内容，并把光标放到 `cursor_pos` 处。
unsafe fn set_composition_text(
    comp: &ITfComposition,
    ec: u32,
    text: &str,
    cursor_pos: usize,
) -> Result<()> {
    let range = comp.GetRange()?;
    let utf16: Vec<u16> = text.encode_utf16().collect();
    range.SetText(ec, TF_ST_CORRECTION, &utf16)?;
    // 把光标放到 cursor_pos 处（UTF-16 码元数），而非总是末尾。
    let cursor = range.Clone()?;
    // TfAnchor(0) = TF_ANCHOR_START
    cursor.Collapse(ec, TfAnchor(0))?;
    let mut actual = 0i32;
    let haltcond = windows::Win32::UI::TextServices::TF_HALTCOND::default();
    cursor.ShiftStart(ec, cursor_pos as i32, &mut actual, &haltcond)?;
    let ctx = range.GetContext()?;
    set_selection(&ctx, ec, &cursor)?;
    Ok(())
}
/// 把编辑器选区设为给定范围，避免组合更新后系统仍把光标留在旧位置。
unsafe fn set_selection(
    context: &ITfContext,
    ec: u32,
    range: &windows::Win32::UI::TextServices::ITfRange,
) -> Result<()> {
    let selection = TF_SELECTION {
        range: std::mem::ManuallyDrop::new(Some(range.clone())),
        style: TF_SELECTIONSTYLE {
            ase: TF_AE_NONE,
            fInterimChar: false.into(),
        },
    };
    let result = context.SetSelection(ec, std::slice::from_ref(&selection));
    let mut selection = selection;
    std::mem::ManuallyDrop::drop(&mut selection.range);
    result
}
/// 组合文本末端在屏幕上的位置，作为候选窗锚点。
unsafe fn composition_anchor(
    context: &ITfContext,
    comp: &ITfComposition,
    ec: u32,
) -> Option<POINT> {
    let view = context.GetActiveView().ok()?;
    let range = comp.GetRange().ok()?;
    let mut rect = RECT::default();
    let mut clipped = BOOL::default();
    view.GetTextExt(ec, &range, &mut rect, &mut clipped).ok()?;
    Some(POINT {
        x: rect.left,
        y: rect.bottom,
    })
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<'_, ITfThreadMgr>, tid: u32) -> Result<()> {
        crate::debug_log("Activate");
        let thread_mgr = ptim.ok()?.clone();

        // 只挂接键盘 sink。引擎/服务连接推迟到首个按键：激活路径上的任何
        // 失败都会让 TSF 禁用本输入法，代价过高。
        let key_sink: ITfKeyEventSink = self.to_interface();
        let keystroke_mgr: ITfKeystrokeMgr = thread_mgr.cast()?;
        unsafe { keystroke_mgr.AdviseKeyEventSink(tid, &key_sink, true)? };

        let mut inner = self.inner.borrow_mut();
        inner.thread_mgr = Some(thread_mgr);
        inner.client_id = tid;
        Ok(())
    }

    fn Deactivate(&self) -> Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.abort_composition();
        if let (Some(tm), tid) = (inner.thread_mgr.take(), inner.client_id) {
            let keystroke_mgr: Result<ITfKeystrokeMgr> = tm.cast();
            if let Ok(mgr) = keystroke_mgr {
                unsafe {
                    let _ = mgr.UnadviseKeyEventSink(tid);
                }
            }
        }
        inner.ui.destroy();
        crate::toast::destroy();
        Ok(())
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    fn ActivateEx(&self, ptim: Ref<'_, ITfThreadMgr>, tid: u32, _flags: u32) -> Result<()> {
        ITfTextInputProcessor_Impl::Activate(self, ptim, tid)
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, foreground: BOOL) -> Result<()> {
        // 焦点离开应用时收起 toast，避免提示残留在其它应用上
        if !foreground.as_bool() {
            crate::toast::hide();
        }
        Ok(())
    }

    fn OnTestKeyDown(
        &self,
        _pic: Ref<'_, ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        // TSF 会先试探再投递实际按键。这里绝不能向引擎喂键或写文档，
        // 否则只调用试探回调的宿主会丢失中文输入并退化成英文直通。
        // 安全桌面守卫：前台是锁屏/UAC/登录时一律不接管按键，全部交还系统
        //（Shift/CapsLock/Ctrl+. 在 LogonUI 上绝不能落 IME 状态，否则会造成
        // 锁屏提权类隐患，搜狗先例 2024.8）。
        if is_secure_desktop() {
            return Ok(false.into());
        }
        // CapsLock 接管与否取决于选项缓存（不读文件，只有 handle_key 周期性重载）。
        let caps_managed = self.inner.borrow().opts.capslock_to_english;
        // Shift：选项开启时主动接管（既有的"Shift 切中英"逻辑只在 handle_key 里跑，
        // 若 OnTestKeyDown 拒绝，TSF 永远不会投递 OnKeyDown）。shift_switch_cn_en
        // 关掉时 Shift 保持旧行为交还系统。
        if wparam.0 as u32 == KeyboardAndMouse::VK_SHIFT.0 as u32 {
            return Ok(self.inner.borrow().opts.shift_switch_cn_en.into());
        }
        // Ctrl+.（中/英标点）：is_ime_key 对带 Ctrl 的键一律放行（直通），但
        // handle_key 里的标点切换分支需要先收到 OnKeyDown —— 必须在此主动接管，
        // 否则该功能永远不可达（"Ctrl+. 切换标点"选项是摆设，2026-08-14 发现）。
        if wparam.0 as u32 == 0xBE {
            let mods = keys::current_modifiers();
            if mods & keys::MASK_CONTROL != 0
                && mods & keys::MASK_ALT == 0
                && self.inner.borrow().opts.ctrl_period_ascii_punct
            {
                return Ok(true.into());
            }
        }
        Ok(keys::is_ime_key(wparam.0 as u32, keys::current_modifiers(), caps_managed).into())
    }

    fn OnKeyDown(&self, pic: Ref<'_, ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        if is_secure_desktop() {
            return Ok(false.into());
        }
        let mut inner = self.inner.borrow_mut();
        let context = pic.ok()?;
        let sink: ITfCompositionSink = self.to_interface();
        if !inner.saw_first_key {
            inner.saw_first_key = true;
            crate::debug_log(&format!("首个按键到达（vk=0x{:X}）", wparam.0));
        }
        inner.refresh_options();
        let eaten = inner.handle_key(&sink, context, wparam);
        Ok(eaten.into())
    }

    fn OnTestKeyUp(
        &self,
        _pic: Ref<'_, ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        // Shift release：若我们上一个 keydown 记录了按下时刻，就接管这次 release
        // 以在 OnKeyUp 里决定走长按视觉提示还是短按切换。其他键的 up 一律放行。
        let is_shift = wparam.0 as u32 == KeyboardAndMouse::VK_SHIFT.0 as u32;
        if !is_shift {
            return Ok(false.into());
        }
        let we_own_this_press = self.inner.borrow().shift_down_at_ms.is_some();
        Ok(we_own_this_press.into())
    }

    fn OnKeyUp(&self, pic: Ref<'_, ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        // 仅 Shift 的 release 有意义；其他 key up 保持旧行为（放行）。
        if wparam.0 as u32 != KeyboardAndMouse::VK_SHIFT.0 as u32 {
            return Ok(false.into());
        }
        if is_secure_desktop() {
            return Ok(false.into());
        }
        let mut inner = self.inner.borrow_mut();
        if inner.shift_down_at_ms.is_none() {
            return Ok(false.into());
        }
        let context = pic.ok()?;
        let sink: ITfCompositionSink = self.to_interface();
        let eaten = inner.handle_shift_release(&sink, context);
        Ok(eaten.into())
    }

    fn OnPreservedKey(&self, _pic: Ref<'_, ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        Ok(false.into())
    }
}

impl ITfCompositionSink_Impl for TextService_Impl {
    fn OnCompositionTerminated(
        &self,
        _ecwrite: u32,
        _pcomposition: Ref<'_, ITfComposition>,
    ) -> Result<()> {
        // 宿主应用强制终止了组合（如点击文档其他位置）
        self.inner.borrow_mut().abort_composition();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        app_name_from_path, app_vim_mode_enabled, decide_app_ascii, decide_shift_release,
        input_scheme_differs, is_long_press, is_vim_normal_mode_key, remap_tab_key,
        symbol_pair_for, AppAsciiAction, ShiftReleaseAction, SHIFT_LONG_PRESS_MS,
    };

    /// watcher 唯一的条件判定：只在 input_scheme 真正变化时才打"方案变化"日志；
    /// 其他字段（热键开关 / general 子字段）翻转不应触发。wave 4 仅为日志，
    /// wave 5 才做 redeploy，因此这个判定就是"要不要打扰用户"的开关。
    #[test]
    fn 方案差异判定_只在_input_scheme_变化时返回true() {
        let base = shurufa_options::ImeOptions::default();
        let mut same_payload = base.clone();
        same_payload.shift_switch_cn_en = !same_payload.shift_switch_cn_en;
        same_payload.general.history_max_entries += 100;
        assert!(
            !input_scheme_differs(&base, &same_payload),
            "其它字段变化不应触发方案变更日志"
        );
        let changed = shurufa_options::ImeOptions {
            input_scheme: "wubi".to_owned(),
            ..base.clone()
        };
        assert!(
            input_scheme_differs(&base, &changed),
            "input_scheme 由 pinyin 切到 wubi 必须被识别"
        );
    }

    /// Shift 长按判定：阈值是 SHIFT_LONG_PRESS_MS，采用 `>=` 闭区间。
    /// - 严格小于阈值 → 短按
    /// - 恰好阈值 → 长按（不能给用户"按了 400ms 却没生效"的错觉）
    /// - 远大于阈值 → 长按
    #[test]
    fn shift_long_press_time_window_at_threshold() {
        assert!(!is_long_press(0));
        assert!(!is_long_press(SHIFT_LONG_PRESS_MS - 1));
        assert!(is_long_press(SHIFT_LONG_PRESS_MS));
        assert!(is_long_press(SHIFT_LONG_PRESS_MS + 1));
        assert!(is_long_press(60_000));
    }

    /// caps_visual/pending 状态机：decide_shift_release 四分支全覆盖。
    /// - 长按 → 进入视觉提示（无论此前状态，幂等），挂起由调用方清除
    /// - 短按 + 已激活 → 清除提示（不切中英）
    /// - 短按 + 未激活 + 挂起 → 结算中/英切换
    /// - 短按 + 未激活 + 无挂起 → 不动作
    #[test]
    fn caps_visual_state_machine_branches() {
        assert_eq!(
            decide_shift_release(SHIFT_LONG_PRESS_MS, false, true),
            ShiftReleaseAction::LongPressVisualCaps,
            "长按 + 未激活 → 进入视觉提示"
        );
        assert_eq!(
            decide_shift_release(SHIFT_LONG_PRESS_MS, true, false),
            ShiftReleaseAction::LongPressVisualCaps,
            "长按 + 已激活 → 视觉提示幂等保持"
        );
        assert_eq!(
            decide_shift_release(SHIFT_LONG_PRESS_MS - 1, true, true),
            ShiftReleaseAction::ClearVisualCaps,
            "短按 + 已激活 → 清除提示，不切中英"
        );
        assert_eq!(
            decide_shift_release(SHIFT_LONG_PRESS_MS - 1, false, true),
            ShiftReleaseAction::FireToggle,
            "短按 + 挂起 → 结算中英切换"
        );
        assert_eq!(
            decide_shift_release(SHIFT_LONG_PRESS_MS - 1, false, false),
            ShiftReleaseAction::NoToggle,
            "短按 + 无挂起 → 不动作（切换已随组合键取消/结算）"
        );
    }

    /// Tab routing：composition 有 breaks 且 keysym = Tab 时，无 Shift → XK_Right，
    /// 有 Shift → XK_Left；keysym 不是 Tab 或没 breaks 一律 None（透传默认路径）。
    /// 这是"分词视觉"与"光标跳音节"之间的纯逻辑接缝。
    #[test]
    fn tab_remap_only_when_breaks_present() {
        const XK_TAB: i32 = 0xff09;
        const XK_LEFT: i32 = 0xff51;
        const XK_RIGHT: i32 = 0xff53;
        // 无断点：不动 Tab
        assert_eq!(remap_tab_key(XK_TAB, false, false), None);
        assert_eq!(remap_tab_key(XK_TAB, true, false), None);
        // 非 Tab keysym：不动（即使 has_breaks=true 也不该乱改其它键）
        assert_eq!(remap_tab_key(0x41, false, true), None);
        assert_eq!(remap_tab_key(0x20, true, true), None);
        // 有断点：Tab → Right，Shift+Tab → Left（1 char/音节步进取自 Rime cursor）
        assert_eq!(remap_tab_key(XK_TAB, false, true), Some(XK_RIGHT));
        assert_eq!(remap_tab_key(XK_TAB, true, true), Some(XK_LEFT));
    }

    /// 前台进程名解析：取路径最后一段、小写化（与 options.json 键一致）。
    #[test]
    fn app_name_from_path_取文件名并小写() {
        assert_eq!(
            app_name_from_path(r"C:\Windows\System32\WindowsTerminal.exe"),
            Some("windowsterminal.exe".to_owned())
        );
        assert_eq!(
            app_name_from_path(r"/usr/bin/code"),
            Some("code".to_owned())
        );
        assert_eq!(
            app_name_from_path("notepad.exe"),
            Some("notepad.exe".to_owned())
        );
    }

    /// decide_app_ascii 状态机：应用变化 + 覆盖表 → 动作。
    /// - 应用没变 → NoAction（不打扰同应用手动切换）
    /// - 切到有覆盖的应用 → ApplyOverride
    /// - 离开被覆盖应用（有快照）→ RestoreSnapshot
    /// - 切到无覆盖应用且无快照 → NoAction（纯切换）
    #[test]
    fn app_ascii_decision_全覆盖语义() {
        use shurufa_options::AppOption;
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert(
            "windowsterminal.exe".to_owned(),
            AppOption {
                ascii_mode: Some(true),
                vim_mode: None,
            },
        );
        overrides.insert(
            "code.exe".to_owned(),
            AppOption {
                ascii_mode: Some(false),
                vim_mode: None,
            },
        );

        let none = None;
        let terminal = Some("windowsterminal.exe".to_owned());
        let code = Some("code.exe".to_owned());
        let chrome = Some("chrome.exe".to_owned());

        // 1) 应用没变 → 不动作
        assert_eq!(
            decide_app_ascii(&terminal, &terminal, &overrides, false),
            AppAsciiAction::NoAction
        );
        // 2) 首次识别（old=None）到被覆盖应用 → 应用覆盖
        assert_eq!(
            decide_app_ascii(&none, &terminal, &overrides, false),
            AppAsciiAction::ApplyOverride { value: true }
        );
        // 3) 有覆盖应用之间切换：以新应用覆盖为准
        assert_eq!(
            decide_app_ascii(&terminal, &code, &overrides, true),
            AppAsciiAction::ApplyOverride { value: false }
        );
        // 4) 离开被覆盖应用（有快照）→ 恢复快照
        assert_eq!(
            decide_app_ascii(&code, &chrome, &overrides, true),
            AppAsciiAction::RestoreSnapshot { value: false }
        );
        // 5) 无覆盖应用之间切换（无快照）→ 不动作
        assert_eq!(
            decide_app_ascii(&chrome, &Some("explorer.exe".to_owned()), &overrides, false),
            AppAsciiAction::NoAction
        );
        // 6) 前台识别失败（new=None）→ 不动作（宁可不触发）
        assert_eq!(
            decide_app_ascii(&terminal, &none, &overrides, true),
            AppAsciiAction::NoAction
        );
        // 7) 覆盖表里该应用无 ascii_mode（None 字段）→ 视同无覆盖
        let mut empty_override = std::collections::BTreeMap::new();
        empty_override.insert(
            "a.exe".to_owned(),
            AppOption {
                ascii_mode: None,
                vim_mode: None,
            },
        );
        assert_eq!(
            decide_app_ascii(&none, &Some("a.exe".to_owned()), &empty_override, false),
            AppAsciiAction::NoAction
        );
    }

    /// vim 模式回 normal 模式键（weasel RimeWithWeasel.cpp 同款）：Esc /
    /// Ctrl+C / Ctrl+[ 命中；其余键（含 Ctrl+V、单独 C、Ctrl+Shift+C）不命中。
    #[test]
    fn vim_normal_mode_key_判定() {
        assert!(is_vim_normal_mode_key(0x1B, false)); // Esc
        assert!(is_vim_normal_mode_key(0x43, true)); // Ctrl+C
        assert!(is_vim_normal_mode_key(0xDB, true)); // Ctrl+[
        assert!(!is_vim_normal_mode_key(0x43, false)); // 单独 C
        assert!(!is_vim_normal_mode_key(0x56, true)); // Ctrl+V
        assert!(!is_vim_normal_mode_key(0xDB, false)); // 单独 [
        assert!(!is_vim_normal_mode_key(0x0D, false)); // Enter
    }

    /// app_vim_mode_enabled：仅当前应用在覆盖表且 vim_mode=true 时启用；
    /// 无条目 / vim_mode=None / 未识别前台应用一律 false（跟随全局）。
    #[test]
    fn app_vim_mode_覆盖语义() {
        use shurufa_options::AppOption;
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert(
            "vim.exe".to_owned(),
            AppOption {
                ascii_mode: Some(true),
                vim_mode: Some(true),
            },
        );
        overrides.insert(
            "chrome.exe".to_owned(),
            AppOption {
                ascii_mode: Some(true),
                vim_mode: None,
            },
        );

        let vim = Some("vim.exe".to_owned());
        let chrome = Some("chrome.exe".to_owned());
        let other = Some("notepad.exe".to_owned());
        let none = None;

        // 1) 匹配且 vim_mode=true → 启用
        assert!(app_vim_mode_enabled(&vim, &overrides));
        // 2) 匹配但 vim_mode=None → 不启用（只配置了 ascii_mode 的应用不受影响）
        assert!(!app_vim_mode_enabled(&chrome, &overrides));
        // 3) 无覆盖条目 → 不启用
        assert!(!app_vim_mode_enabled(&other, &overrides));
        // 4) 前台识别失败 → 不启用（宁可不触发）
        assert!(!app_vim_mode_enabled(&none, &overrides));
        // 5) 空覆盖表 → 不启用
        assert!(!app_vim_mode_enabled(
            &vim,
            &std::collections::BTreeMap::new()
        ));
    }

    /// 符号配对表（微信输入法同类）：四个开括号 → (配对文本, 光标位置)。
    /// 光标位置 = 1（UTF-16 码元，落在两个字符之间）。
    #[test]
    fn symbol_pair_lookup_覆盖常见开括号() {
        assert_eq!(symbol_pair_for(0x0028), Some(("()", 1))); // (
        assert_eq!(symbol_pair_for(0x005B), Some(("[]", 1))); // [
        assert_eq!(symbol_pair_for(0x007B), Some(("{}", 1))); // {
        assert_eq!(symbol_pair_for(0x300A), Some(("《》", 1))); // 《
                                                                // 不配对的键：普通字母/闭合括号/其它符号一律 None（透传默认路径）
        assert_eq!(symbol_pair_for(0x0041), None); // A
        assert_eq!(symbol_pair_for(0x0029), None); // )
        assert_eq!(symbol_pair_for(0x0020), None); // 空格
        assert_eq!(symbol_pair_for(0x300B), None); // 》
        assert_eq!(symbol_pair_for(0xFFFF), None); // 未知 keysym
    }
}
