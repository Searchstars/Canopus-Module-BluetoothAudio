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

#[cfg(feature = "device")]
const MODULE_TARGET_ID: &[u8] = canopus_target_private::TARGET_ID.as_bytes();
#[cfg(not(feature = "device"))]
const MODULE_TARGET_ID: &[u8] = b"host-test";

#[cfg(feature = "device")]
#[repr(C)]
struct ModuleRegistrationV1 {
    magic: u32,
    descriptor: u32,
    module_id: [u8; 32],
}

#[cfg(feature = "device")]
const REGISTRATION_MAGIC: u32 = 0x3152_4d43; // "CMR1"
#[cfg(feature = "device")]
const CANOPUS_DEVICE_PATH: &[u8] = b"/dev/canopus\0";

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
    #[cfg(feature = "device")]
    RESIDENT.store(crate::target::resident(), Ordering::Release);
    if rc == 0 {
        ACTIVE.store(true, Ordering::Release);
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
    -103
}

#[unsafe(no_mangle)]
pub extern "C" fn canopus_mod_publish_native_app_stage(_ctx: *const ContextV1, stage: u32) -> i32 {
    #[cfg(feature = "device")]
    let rc = match crate::target::native_app::install_stage(stage) {
        Ok(()) => 0,
        Err(error) => error,
    };
    #[cfg(not(feature = "device"))]
    let rc = if stage == 1 || stage == 2 { 0 } else { -103 };
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
    target_id: pack(MODULE_TARGET_ID),
    prepare: Some(canopus_mod_prepare),
    activate: Some(canopus_mod_activate),
    deactivate: Some(canopus_mod_deactivate),
    stop: Some(canopus_mod_stop),
    query: Some(canopus_mod_query),
    publish_native_app: Some(canopus_mod_publish_native_app),
    publish_native_app_stage: Some(canopus_mod_publish_native_app_stage),
};

#[cfg(feature = "device")]
#[unsafe(no_mangle)]
pub extern "C" fn canopus_register_module_descriptor() -> i32 {
    if canopus_target_private::canopus_identity_guard() != 0 {
        return -1;
    }
    let registration = ModuleRegistrationV1 {
        magic: REGISTRATION_MAGIC,
        descriptor: core::ptr::addr_of!(canopus_module_descriptor) as usize as u32,
        module_id: pack(b"bluetooth_audio"),
    };
    let fd = unsafe { canopus_target_private::nuttx_open(CANOPUS_DEVICE_PATH.as_ptr(), 2) };
    if fd < 0 {
        return fd;
    }
    let written = unsafe {
        canopus_target_private::nuttx_write(
            fd,
            core::ptr::addr_of!(registration).cast(),
            core::mem::size_of::<ModuleRegistrationV1>() as u32,
        )
    };
    let close = unsafe { canopus_target_private::nuttx_close(fd) };
    if written != core::mem::size_of::<ModuleRegistrationV1>() as i32 {
        return if written < 0 { written } else { -1 };
    }
    close
}

#[unsafe(no_mangle)]
pub extern "C" fn canopus_module_descriptor_ptr() -> *const ModuleDescriptorV1 {
    &canopus_module_descriptor
}
