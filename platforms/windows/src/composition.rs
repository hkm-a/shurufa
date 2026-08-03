//! ITfEditSession 的闭包封装：TSF 的文档修改都必须在编辑会话内进行。

use std::cell::RefCell;

use windows::core::{implement, Result};
use windows::Win32::UI::TextServices::{
    ITfContext, ITfEditSession, ITfEditSession_Impl, TF_ES_READWRITE, TF_ES_SYNC,
};

type Action = Box<dyn FnMut(u32) -> Result<()>>;

#[implement(ITfEditSession)]
struct ClosureEditSession {
    action: RefCell<Action>,
}

impl ITfEditSession_Impl for ClosureEditSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        (self.action.borrow_mut())(ec)
    }
}

/// 在同步读写编辑会话中执行 `action`（参数为 edit cookie）。
/// 只允许在键事件回调等 TSF 认可的同步时机调用。
pub fn edit_session(
    client_id: u32,
    context: &ITfContext,
    action: impl FnMut(u32) -> Result<()>,
) -> Result<()> {
    // 安全性：TF_ES_SYNC 保证 DoEditSession 在 RequestEditSession
    // 返回前同步执行完毕，闭包不会在借用结束后被调用，
    // 因此将局部借用擦除为 'static 是受控的。
    let action: Box<dyn FnMut(u32) -> Result<()> + '_> = Box::new(action);
    let action: Action = unsafe { std::mem::transmute(action) };
    let session: ITfEditSession = ClosureEditSession {
        action: RefCell::new(action),
    }
    .into();
    let hr =
        unsafe { context.RequestEditSession(client_id, &session, TF_ES_SYNC | TF_ES_READWRITE) }?;
    hr.ok()
}
