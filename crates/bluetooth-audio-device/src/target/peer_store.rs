//! Metadata for the reconnect button. All file I/O runs on the module/page owner.

use canopus_abi::ContextV1;
use canopus_bluetooth_audio_core::{Peer, peer};
use canopus_target_private::{
    O_RDONLY, nuttx_close, nuttx_create, nuttx_open, nuttx_read, nuttx_rename, nuttx_unlink,
    nuttx_write,
};
use core::{
    ffi::c_void,
    sync::atomic::{AtomicPtr, Ordering},
};

const PATH: &[u8] = b"/data/canopus_btaudio_peer.bin\0";
const TEMP_PATH: &[u8] = b"/data/canopus_btaudio_peer.tmp\0";
static mut SAVED: Option<[u8; peer::RECORD_SIZE]> = None;
static CONTEXT: AtomicPtr<ContextV1> = AtomicPtr::new(core::ptr::null_mut());

pub fn bind_context(ctx: *const ContextV1) {
    CONTEXT.store(ctx.cast_mut(), Ordering::Release);
}

/// Pairing may finish after the user leaves the page. Queue persistence on
/// the UI owner so it does not depend on the headphones page staying open.
pub fn schedule_flush() {
    let ctx = CONTEXT.load(Ordering::Acquire);
    let _ = canopus_runtime::ui::post(ctx.cast(), flush_task, core::ptr::null_mut());
}

unsafe extern "C" fn flush_task(_ctx: *const ContextV1, _argument: *mut c_void) {
    let Some(remembered) = super::runtime::try_with_core(|core| core.controller.model.remembered) else {
        schedule_flush();
        return;
    };
    if !flush(remembered) {
        super::runtime::runtime()
            .last_error
            .store(super::runtime::ERRNO_EIO, Ordering::Release);
    }
}

pub fn load() -> Option<Peer> {
    unsafe { SAVED = None };
    let fd = unsafe { nuttx_open(PATH.as_ptr(), O_RDONLY) };
    if fd < 0 {
        return None;
    }
    let mut bytes = [0u8; peer::RECORD_SIZE];
    let read = unsafe { nuttx_read(fd, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    unsafe { nuttx_close(fd) };
    if read != bytes.len() as i32 {
        return None;
    }
    let remembered = peer::decode(&bytes)?;
    unsafe { SAVED = Some(bytes) };
    Some(remembered)
}

pub fn flush(remembered: Option<Peer>) -> bool {
    let Some(remembered) = remembered else {
        return true;
    };
    let bytes = peer::encode(&remembered);
    if unsafe { SAVED == Some(bytes) } {
        return true;
    }
    unsafe { nuttx_unlink(TEMP_PATH.as_ptr()) };
    // Same NuttX open flags used by the volume store: O_WRONLY | O_CREAT.
    let fd = unsafe { nuttx_create(TEMP_PATH.as_ptr(), 2 | 4, 0o600) };
    if fd < 0 {
        return false;
    }
    let written = unsafe { nuttx_write(fd, bytes.as_ptr().cast(), bytes.len() as u32) };
    let closed = unsafe { nuttx_close(fd) };
    if written != bytes.len() as i32
        || closed != 0
        || unsafe { nuttx_rename(TEMP_PATH.as_ptr(), PATH.as_ptr()) } != 0
    {
        unsafe { nuttx_unlink(TEMP_PATH.as_ptr()) };
        return false;
    }
    unsafe { SAVED = Some(bytes) };
    true
}
