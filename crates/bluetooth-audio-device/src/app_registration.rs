//! Application identity supplied by the Supervisor, independent of firmware ABI.

use canopus_abi::{AppEntryV1, ContextV1};
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

pub struct AppRegistration {
    id: AtomicU32,
    error: AtomicI32,
}

impl AppRegistration {
    pub const fn new() -> Self {
        Self {
            id: AtomicU32::new(0),
            error: AtomicI32::new(0),
        }
    }

    pub fn id(&self) -> u32 {
        self.id.load(Ordering::Acquire)
    }

    pub fn error(&self) -> i32 {
        self.error.load(Ordering::Acquire)
    }

    /// # Safety
    /// The context, definition and call owner must satisfy `launcher::register_app`.
    pub unsafe fn register(&self, ctx: *const ContextV1, entry: &AppEntryV1) -> Result<(), i32> {
        if self.id() != 0 {
            return Ok(());
        }
        match unsafe { canopus_runtime::launcher::register_app(ctx.cast(), entry) } {
            Ok(id) if id != 0 => {
                self.id.store(u32::from(id), Ordering::Release);
                self.error.store(0, Ordering::Release);
                Ok(())
            }
            result => {
                let error = result.err().unwrap_or(-22);
                self.error.store(error, Ordering::Release);
                Err(error)
            }
        }
    }

    pub fn page_key(&self, page: usize) -> Option<u32> {
        let id = self.id();
        (id != 0 && page <= u16::MAX as usize).then_some((id << 16) | page as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canopus_abi::*;
    use core::ffi::CStr;

    unsafe extern "C" fn register(ctx: *const ContextV1, entry: *const AppEntryV1) -> i32 {
        let ctx = unsafe { &*ctx.cast::<CanopusContext>() };
        let entry = unsafe { &*entry };
        assert_eq!(
            entry.struct_size as usize,
            core::mem::size_of::<AppEntryV1>()
        );
        assert_eq!(
            unsafe { CStr::from_ptr(entry.label) }.to_bytes(),
            b"Headphones"
        );
        assert_eq!(entry.page_count, 3);
        let pages = unsafe { core::slice::from_raw_parts(entry.pages, 3) };
        assert!(pages.iter().all(|page| !page.is_null()));
        // Use the context's module index to independently model success/failure.
        if ctx.slot == 0 { -12 } else { 0xD7 }
    }

    #[test]
    fn assigned_identity_survives_repeated_publication_and_errors_can_retry() {
        let registration = AppRegistration::new();
        assert_eq!(registration.page_key(0), None);
        let object = CanopusLauncherObj {
            register_app: register,
        };
        let mut services = ServicesV1Head {
            struct_size: core::mem::size_of::<ServicesV1Head>() as u32,
            group_version: [0; SVC_GROUP_COUNT as usize],
        };
        services.group_version[SVC_LAUNCHER as usize] = 1;
        let mut ctx: CanopusContext = unsafe { core::mem::zeroed() };
        ctx.struct_size = core::mem::size_of::<CanopusContext>() as u32;
        ctx.services = core::ptr::addr_of!(services).cast();
        ctx.launcher = &object;
        let mut descriptors = [0u32; 3];
        let pages = descriptors.each_mut().map(|page| (page as *mut u32).cast());
        let entry = AppEntryV1 {
            struct_size: core::mem::size_of::<AppEntryV1>() as u32,
            label: c"Headphones".as_ptr(),
            icon: core::ptr::null(),
            pages: pages.as_ptr(),
            page_count: 3,
        };
        assert_eq!(
            unsafe { registration.register(core::ptr::addr_of!(ctx).cast(), &entry) },
            Err(-12)
        );
        assert_eq!(registration.error(), -12);
        assert_eq!(registration.page_key(2), None);
        ctx.slot = 1;
        assert_eq!(
            unsafe { registration.register(core::ptr::addr_of!(ctx).cast(), &entry) },
            Ok(())
        );
        assert_eq!(registration.error(), 0);
        assert_eq!(registration.page_key(0), Some(0xD70000));
        assert_eq!(registration.page_key(2), Some(0xD70002));
        assert_eq!(registration.page_key(0x10000), None);
        // A later failing context must not register again or replace the ID.
        ctx.slot = 0;
        assert_eq!(
            unsafe { registration.register(core::ptr::addr_of!(ctx).cast(), &entry) },
            Ok(())
        );
        assert_eq!(registration.id(), 0xD7);
    }

    #[test]
    fn missing_service_leaves_navigation_unavailable() {
        let registration = AppRegistration::new();
        let entry: AppEntryV1 = unsafe { core::mem::zeroed() };
        assert_eq!(
            unsafe { registration.register(core::ptr::null(), &entry) },
            Err(-22)
        );
        let mut ctx: CanopusContext = unsafe { core::mem::zeroed() };
        ctx.struct_size = core::mem::size_of::<CanopusContext>() as u32;
        assert_eq!(
            unsafe { registration.register(core::ptr::addr_of!(ctx).cast(), &entry) },
            Err(-38)
        );
        assert_eq!(registration.id(), 0);
        assert_eq!(registration.page_key(1), None);
    }
}
