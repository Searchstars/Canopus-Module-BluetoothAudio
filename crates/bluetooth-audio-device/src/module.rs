use canopus_abi::*;
use canopus_runtime::{status_put_u32, status_writer_publish};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const MAGIC: u32 = 0x4241_5531; // "BAU1"
static ACTIVE: AtomicBool = AtomicBool::new(false);
static RESIDENT: AtomicBool = AtomicBool::new(false);
static LAST_ERROR: AtomicU32 = AtomicU32::new(0);

const fn pack<const N: usize>(value: &[u8]) -> [u8; N] {
    let mut out = [0; N];
    let mut i = 0;
    while i < value.len() && i < N {
        out[i] = value[i];
        i += 1;
    }
    out
}

#[unsafe(no_mangle)]
pub extern "C" fn canopus_mod_prepare(_ctx: *const ContextV1) -> i32 {
    ACTIVE.store(false, Ordering::Release);
    RESIDENT.store(false, Ordering::Release);
    LAST_ERROR.store(0, Ordering::Release);
    #[cfg(feature = "device")]
    crate::target::prepare(1);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn canopus_mod_activate(_ctx: *const ContextV1) -> i32 {
    // The target backend performs the identity guard, adapter registration and
    // queued SDP initialization. Native app publication is a separate miwear
    // bootstrap operation. Once Bluetooth callbacks are published, unload is
    // irreversible until reboot.
    #[cfg(feature = "device")]
    let rc = crate::target::activate();
    #[cfg(not(feature = "device"))]
    let rc = 0;
    if rc == 0 {
        ACTIVE.store(true, Ordering::Release);
        #[cfg(feature = "device")]
        RESIDENT.store(crate::target::resident(), Ordering::Release);
        0
    } else {
        LAST_ERROR.store(rc as u32, Ordering::Release);
        rc
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn canopus_mod_deactivate(_ctx: *const ContextV1) -> i32 {
    if RESIDENT.load(Ordering::Acquire) {
        return RESULT_REBOOT_REQUIRED as i32;
    }
    ACTIVE.store(false, Ordering::Release);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn canopus_mod_stop(ctx: *const ContextV1) -> i32 {
    canopus_mod_deactivate(ctx)
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn canopus_mod_query(writer: *mut StatusWriterV1) -> i32 {
    if writer.is_null() {
        return -1;
    }
    let writer = unsafe { &mut *writer };
    unsafe {
        if !status_put_u32(writer, MAGIC)
            || !status_put_u32(writer, ACTIVE.load(Ordering::Acquire) as u32)
            || !status_put_u32(writer, RESIDENT.load(Ordering::Acquire) as u32)
            || !status_put_u32(writer, LAST_ERROR.load(Ordering::Acquire))
        {
            return -1;
        }
    }
    #[cfg(feature = "device")]
    {
        for value in crate::target::query_status() {
            if !unsafe { status_put_u32(writer, value) } {
                return -1;
            }
        }
    }
    status_writer_publish(writer);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn canopus_mod_publish_native_app(_ctx: *const ContextV1) -> i32 {
    #[cfg(feature = "device")]
    let rc = match crate::target::native_app::install() {
        Ok(()) => 0,
        Err(error) => error,
    };
    #[cfg(not(feature = "device"))]
    let rc = 0;
    if rc != 0 {
        LAST_ERROR.store(rc as u32, Ordering::Release);
    }
    rc
}

#[unsafe(no_mangle)]
pub static canopus_module_descriptor: ModuleDescriptorV1 = ModuleDescriptorV1 {
    struct_size: core::mem::size_of::<ModuleDescriptorV1>() as u32,
    abi_major: ABI_MAJOR,
    abi_minor: ABI_MINOR,
    flags: FLAG_HAS_NATIVE_APP
        | FLAG_NATIVE_APP_STANDALONE
        | FLAG_REGISTERS_LAUNCHER_ENTRY
        | FLAG_REQUIRES_UI_DISPATCHER
        | FLAG_APP_UNREGISTER_REBOOT_REQUIRED,
    module_id: pack(b"bluetooth_audio"),
    module_version: pack(b"0.1.0"),
    build_id: pack(b"bluetooth-audio-0.1.0"),
    target_id: pack(b"xiaomi-band-10-pro-3.101.030"),
    prepare: Some(canopus_mod_prepare),
    activate: Some(canopus_mod_activate),
    deactivate: Some(canopus_mod_deactivate),
    stop: Some(canopus_mod_stop),
    query: Some(canopus_mod_query),
    publish_native_app: Some(canopus_mod_publish_native_app),
};

#[unsafe(no_mangle)]
pub extern "C" fn canopus_module_descriptor_ptr() -> *const ModuleDescriptorV1 {
    &canopus_module_descriptor
}
