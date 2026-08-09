//! TextService：TSF 文本输入处理器主体。
//!
//! 职责：激活时挂接键盘事件 sink，把每个按键翻译为 keysym 经 IPC 客户端转发给
//! 独立算法服务（shurufa-algo），随后把引擎状态（上屏文本 / 预编辑串 / 候选）
//! 同步回文档与候选窗。引擎不在本进程内 —— 用户词库锁冲突由此消除。

use std::cell::RefCell;
use std::time::{Duration, Instant, SystemTime};

use windows::core::{implement, Interface, Ref, Result, BOOL, GUID};
use windows::Win32::Foundation::{LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfContextComposition,
    ITfInsertAtSelection, ITfKeyEventSink, ITfKeyEventSink_Impl, ITfKeystrokeMgr,
    ITfTextInputProcessorEx, ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl,
    ITfThreadMgr, TfAnchor, INSERT_TEXT_AT_SELECTION_FLAGS, TF_AE_NONE, TF_ANCHOR_END,
    TF_IAS_QUERYONLY, TF_SELECTION, TF_SELECTIONSTYLE, TF_ST_CORRECTION,
};
use windows::Win32::UI::Input::KeyboardAndMouse;
use windows_core::IUnknownImpl;

use shurufa_options::ImeOptions;

use crate::candidate_window::CandidateUi;
use crate::composition::edit_session;
use crate::ipc_client::ImeClient;
use crate::keys;

pub struct Inner {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    /// 经 IPC 的引擎会话客户端（懒连接）。
    client: ImeClient,
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
}

#[implement(ITfTextInputProcessorEx, ITfKeyEventSink, ITfCompositionSink)]
pub struct TextService {
    inner: RefCell<Inner>,
}

impl TextService {
    pub fn new() -> Self {
        TextService {
            inner: RefCell::new(Inner {
                thread_mgr: None,
                client_id: 0,
                client: ImeClient::new(),
                composition: None,
                ui: CandidateUi::new(),
                saw_first_key: false,
                opts: shurufa_options::load(),
                opts_checked_at: Instant::now(),
                opts_mtime: None,
            }),
        }
    }
}

impl Inner {
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
            self.opts = shurufa_options::load();
            crate::debug_log(&format!("选项已重载：{:?}", self.opts));
        }
    }

    /// 结束文档侧残留的 TSF 组合（切换中英文/标点/全角前的收尾，
    /// 否则残留组合会把后续按键吃进去）。
    fn end_pending_composition(&mut self, context: &ITfContext) {
        if let Some(comp) = self.composition.take() {
            let client_id = self.client_id;
            if let Err(e) = edit_session(client_id, context, |ec| {
                unsafe {
                    set_composition_text(&comp, ec, "", 0)?;
                    comp.EndComposition(ec)
                }
            }) {
                crate::debug_log(&format!("结束残留组合失败：{e:?}"));
            }
        }
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

        // Shift 单独按下：切换中英文（主流行为）。ToggleAscii 是唯一写路径，
        // 避免 OnTestKeyDown 只试不写导致的状态漂移。
        // 切换前必须把已有组合收尾，否则残留组合会把后续的英文/拼音都吃进去。
        if vk == KeyboardAndMouse::VK_SHIFT.0 as u32 {
            if !self.opts.shift_switch_cn_en {
                return false;
            }
            self.end_pending_composition(context);
            if let Some(is_ascii) = self.client.toggle_ascii() {
                crate::debug_log(&format!("Shift 切换中英文：ascii={is_ascii}"));
                return true;
            }
            return false;
        }
        // CapsLock：开启选项时切到英文直输（只进不出，回中文用 Shift）。
        // 吃掉该键后系统不再翻转大写灯（OnTestKeyDown 已声明接管）。
        if vk == KeyboardAndMouse::VK_CAPITAL.0 as u32 && self.opts.capslock_to_english {
            self.end_pending_composition(context);
            if self.client.get_option("ascii_mode") == Some(false) {
                let is_ascii = self.client.toggle_ascii().unwrap_or(false);
                crate::debug_log(&format!("CapsLock 切英文直输：ascii={is_ascii}"));
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
            let current = self.client.get_option("full_shape").unwrap_or(false);
            let next = !current;
            let ok = self.client.set_option("full_shape", next);
            crate::debug_log(&format!("Shift+Space 切换全/半角：full_shape={next} ok={ok}"));
            return true;
        }
        // Ctrl+.：切换中/英标点（ascii_punct）。必须放在 Ctrl/Alt 直通判断之前。
        if vk == 0xBE && ctrl && !alt && self.opts.ctrl_period_ascii_punct {
            self.end_pending_composition(context);
            let current = self.client.get_option("ascii_punct").unwrap_or(false);
            let next = !current;
            let ok = self.client.set_option("ascii_punct", next);
            crate::debug_log(&format!("Ctrl+. 切换中/英标点：ascii_punct={next} ok={ok}"));
            return true;
        }
        // Ctrl/Alt 组合键与不认识的键一律放行
        if modifiers & (keys::MASK_CONTROL | keys::MASK_ALT) != 0 {
            return false;
        }
        let Some(keysym) = keys::vk_to_keysym(vk, shift) else {
            // 引擎连接失败：把当前按键作为原字符落入文档（中文兜底），
            // 避免“只能输入英文”。
            let _ = self.fallback_commit(context, vk, shift);
            return false;
        };
        let Some((eaten, commit, ctx)) = self.client.process_key(keysym, modifiers) else {
            // 引擎连接失败：把当前按键作为原字符落入文档，避免“只能输入英文”。
            let _ = self.fallback_commit(context, vk, shift);
            crate::debug_log("引擎 IPC 不可用，按键直通");
            return false;
        };

        crate::debug_log(&format!(
            "键 vk=0x{:X} keysym=0x{:X} eaten={} commit={:?} preedit={:?}",
            wparam.0, keysym, eaten, commit, ctx.preedit
        ));

        let has_preedit = !ctx.preedit.is_empty();
        let client_id = self.client_id;

        // 文档更新必须进入编辑会话
        let composition_slot = &mut self.composition;
        let ui = &mut self.ui;
        let edit_result = edit_session(client_id, context, |ec| {
            unsafe {
                // 1. 上屏文本：结束组合并以最终文本落盘
                if let Some(text) = commit.as_deref() {
                    if let Some(comp) = composition_slot.take() {
                        set_composition_text(&comp, ec, text, text.encode_utf16().count())?;
                        comp.EndComposition(ec)?;
                    } else {
                        insert_text(context, ec, text)?;
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

                // 3. 候选窗：跟随组合文本位置
                if has_preedit && !ctx.candidates.is_empty() {
                    let anchor = composition_slot
                        .as_ref()
                        .and_then(|comp| composition_anchor(context, comp, ec));
                    ui.show(&ctx, anchor);
                } else {
                    ui.hide();
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
        self.client.simulate("{Escape}");
        self.composition = None;
        self.ui.hide();
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
        edit_session(client_id, context, |ec| unsafe { insert_text(context, ec, &text) })
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
    let result = context.SetSelection(ec, &[selection.clone()]);
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
        Ok(())
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    fn ActivateEx(&self, ptim: Ref<'_, ITfThreadMgr>, tid: u32, _flags: u32) -> Result<()> {
        ITfTextInputProcessor_Impl::Activate(self, ptim, tid)
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, _foreground: BOOL) -> Result<()> {
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
        // CapsLock 接管与否取决于选项缓存（不读文件，只有 handle_key 周期性重载）。
        let caps_managed = self.inner.borrow().opts.capslock_to_english;
        Ok(keys::is_ime_key(wparam.0 as u32, keys::current_modifiers(), caps_managed).into())
    }

    fn OnKeyDown(&self, pic: Ref<'_, ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
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
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        Ok(false.into())
    }

    fn OnKeyUp(&self, _pic: Ref<'_, ITfContext>, _wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        Ok(false.into())
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
