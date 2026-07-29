//! COM 类工厂：宿主进程通过它实例化 TextService。

use std::ffi::c_void;

use windows::core::{implement, IUnknown, Interface, Ref, Result, GUID, BOOL};
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};

use crate::service::TextService;

#[implement(IClassFactory)]
pub struct ClassFactory;

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> Result<()> {
        unsafe {
            if ppvobject.is_null() {
                return Err(E_POINTER.into());
            }
            *ppvobject = std::ptr::null_mut();
            if punkouter.is_some() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let service: IUnknown = TextService::new().into();
            service.query(riid, ppvobject).ok()
        }
    }

    fn LockServer(&self, _flock: BOOL) -> Result<()> {
        Ok(())
    }
}
