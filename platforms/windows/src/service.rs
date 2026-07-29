//! TextService：TSF 文本输入处理器主体。
//!
//! 职责：激活时挂接键盘事件 sink 并建立 librime 会话；每个按键翻译为
//! keysym 喂给引擎，随后把引擎状态（上屏文本 / 预编辑串 / 候选）
//! 同步回文档与候选窗。

use std::cell::RefCell;

use windows::core::{implement, Interface, Ref, Result, BOOL, GUID};
use windows_core::IUnknownImpl;
use windows::Win32::Foundation::{LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext,
    ITfContextComposition, ITfInsertAtSelection, ITfKeyEventSink, ITfKeyEventSink_Impl,
    ITfKeystrokeMgr, ITfTextInputProcessorEx,
    ITfTextInputProcessorEx_Impl, ITfTextInputProcessor_Impl, ITfThreadMgr,
    INSERT_TEXT_AT_SELECTION_FLAGS, TF_ANCHOR_END, TF_IAS_QUERYONLY, TF_SELECTION,
    TF_SELECTIONSTYLE, TF_AE_NONE, TF_ST_CORRECTION,
};

use ime_bridge::Session;

use crate::candidate_window::CandidateUi;
use crate::composition::edit_session;
use crate::keys;

pub struct Inner {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    session: Option<Session<'static>>,
    composition: Option<ITfComposition>,
    ui: CandidateUi,
    /// OnTestKeyDown 已处理的键及其结论，供紧随其后的 OnKeyDown 复用
    pending_key: Option<(u32, bool)>,
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
                session: None,
                composition: None,
                ui: CandidateUi::new(),
                pending_key: None,
            }),
        }
    }
}

impl Inner {
    /// 懒建引擎会话：激活阶段绝不碰引擎，宿主加载失败的代价必须最小化；
    /// 引擎不可用时输入法退化为按键直通。
    fn ensure_session(&mut self) -> Option<&Session<'static>> {
        if self.session.is_none() {
            match crate::engine() {
                Ok(engine) => match engine.create_session() {
                    Ok(s) => self.session = Some(s),
                    Err(e) => {
                        crate::debug_log(&format!("创建引擎会话失败：{e}"));
                        return None;
                    }
                },
                Err(_) => return None,
            }
        }
        self.session.as_ref()
    }

    /// 喂键给引擎并同步文档/候选窗；返回该键是否被输入法吃掉。
    fn handle_key(
        &mut self,
        sink: &ITfCompositionSink,
        context: &ITfContext,
        wparam: WPARAM,
    ) -> bool {
        let modifiers = keys::current_modifiers();
        let shift = modifiers & keys::MASK_SHIFT != 0;
        // Ctrl/Alt 组合键与不认识的键一律放行
        if modifiers & (keys::MASK_CONTROL | keys::MASK_ALT) != 0 {
            return false;
        }
        let Some(keysym) = keys::vk_to_keysym(wparam.0 as u32, shift) else {
            return false;
        };
        if self.ensure_session().is_none() {
            return false;
        }
        let session = self.session.as_ref().expect("ensure_session 已保证存在");

        let eaten = session.process_key(keysym, modifiers);

        // 引擎可能产生上屏文本（如空格确认候选、顶字上屏）
        let commit = session.commit();
        let ctx_snapshot = session.context();

        let has_preedit = !ctx_snapshot.preedit.is_empty();
        let client_id = self.client_id;

        // 文档更新必须进入编辑会话
        let composition_slot = &mut self.composition;
        let ui = &mut self.ui;
        let _ = edit_session(client_id, context, |ec| {
            unsafe {
                // 1. 上屏文本：结束组合并以最终文本落盘
                if let Some(text) = commit.as_deref() {
                    if let Some(comp) = composition_slot.take() {
                        set_composition_text(&comp, ec, text)?;
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
                        set_composition_text(comp, ec, &ctx_snapshot.preedit)?;
                    }
                } else if let Some(comp) = composition_slot.take() {
                    // 引擎已无组合（如 Esc 清空），结束并清除文档中的预编辑
                    set_composition_text(&comp, ec, "")?;
                    comp.EndComposition(ec)?;
                }

                // 3. 候选窗：跟随组合文本位置
                if has_preedit && !ctx_snapshot.candidates.is_empty() {
                    let anchor = composition_slot
                        .as_ref()
                        .and_then(|comp| composition_anchor(context, comp, ec));
                    ui.show(&ctx_snapshot, anchor);
                } else {
                    ui.hide();
                }
                Ok(())
            }
        });

        eaten
    }

    fn abort_composition(&mut self) {
        if let Some(session) = self.session.as_ref() {
            // 清空引擎侧组合状态；文档侧组合由 TSF 生命周期回调负责
            session.simulate("{Escape}");
        }
        self.composition = None;
        self.ui.hide();
        self.pending_key = None;
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

/// 用 `text` 替换组合范围内容，并把光标放到末尾。
unsafe fn set_composition_text(comp: &ITfComposition, ec: u32, text: &str) -> Result<()> {
    let range = comp.GetRange()?;
    let utf16: Vec<u16> = text.encode_utf16().collect();
    range.SetText(ec, TF_ST_CORRECTION, &utf16)?;
    let end = range.Clone()?;
    end.Collapse(ec, TF_ANCHOR_END)?;
    let context = range.GetContext()?;
    set_selection(&context, ec, &end)?;
    Ok(())
}

unsafe fn set_selection(context: &ITfContext, ec: u32, range: &windows::Win32::UI::TextServices::ITfRange) -> Result<()> {
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

        // 只挂接键盘 sink。引擎初始化推迟到首个按键：激活路径上的任何
        // 失败都会让 TSF 禁用本输入法，代价过高。
        let key_sink: ITfKeyEventSink = self.to_interface();
        let keystroke_mgr: ITfKeystrokeMgr = thread_mgr.cast()?;
        unsafe { keystroke_mgr.AdviseKeyEventSink(tid, &key_sink, true)? };

        let mut inner = self.inner.borrow_mut();
        inner.thread_mgr = Some(thread_mgr);
        inner.client_id = tid;
        inner.session = None;
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
        inner.session = None;
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
        pic: Ref<'_, ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        let context = pic.ok()?;
        let sink: ITfCompositionSink = self.to_interface();
        let mut inner = self.inner.borrow_mut();
        let eaten = inner.handle_key(&sink, context, wparam);
        // 记录结论：应用随后会调用 OnKeyDown，不能重复喂引擎
        inner.pending_key = Some((wparam.0 as u32, eaten));
        Ok(eaten.into())
    }

    fn OnKeyDown(
        &self,
        pic: Ref<'_, ITfContext>,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
        let mut inner = self.inner.borrow_mut();
        if let Some((vk, eaten)) = inner.pending_key.take() {
            if vk == wparam.0 as u32 {
                return Ok(eaten.into());
            }
        }
        // 应用跳过了 OnTestKeyDown，直接处理
        let context = pic.ok()?;
        let sink: ITfCompositionSink = self.to_interface();
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

    fn OnKeyUp(
        &self,
        _pic: Ref<'_, ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Result<BOOL> {
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
