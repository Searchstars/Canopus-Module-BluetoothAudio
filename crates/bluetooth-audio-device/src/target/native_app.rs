//! Native app registration: fixed 8-bit app id, stock launcher entry, two page
//! descriptors (overview + detail), and the page lifecycle callbacks. App/page
//! registration and Launcher publication are intentionally separate invocations
//! so miwear can process the app-registry event before Launcher persistence.

use core::sync::atomic::Ordering;

use canopus_target_private::*;

use super::runtime::*;
use super::ui_backend;

pub const APP_ID: u16 = 0x00CB;
pub const PAGE_COUNT: usize = 2;
pub const PAGE_OVERVIEW: usize = 0;
pub const PAGE_DETAIL: usize = 1;

pub const PACKAGE_NAME: &[u8] = b"com.canopus.headphones\0";
pub const DISPLAY_NAME: &[u8] = b"Headphones\0";
pub const LAUNCHER_ICON: &[u8] = b"/resource/app/launcher/flashlight.bin\0";
const PAGE_NAME_OVERVIEW: &[u8] = b"headphones\0";
const PAGE_NAME_DETAIL: &[u8] = b"headphones_detail\0";

static mut APP_DESCRIPTOR: core::mem::MaybeUninit<launcher_app_descriptor> =
    core::mem::MaybeUninit::uninit();
static mut PAGE_DESCRIPTORS: core::mem::MaybeUninit<[firmware_page_descriptor; PAGE_COUNT]> =
    core::mem::MaybeUninit::uninit();

pub fn page_descriptor_ptr(index: usize) -> *mut firmware_page_descriptor {
    // SAFETY: PAGE_DESCRIPTORS is initialized by `install` before any page
    // lifecycle callback can run; the firmware only reads these descriptors
    // after install returns. `addr_of_mut!` avoids the `static_mut_refs` deny
    // lint; the caller uses raw pointer writes.
    unsafe {
        core::ptr::addr_of_mut!(PAGE_DESCRIPTORS)
            .cast::<firmware_page_descriptor>()
            .add(index)
    }
}

extern "C" fn launcher_display_name() -> *const u8 {
    DISPLAY_NAME.as_ptr()
}

fn c_str_equal(a: *const u8, expected: &[u8]) -> bool {
    if a.is_null() || expected.last() != Some(&0) {
        return false;
    }
    let mut i = 0usize;
    while i < expected.len() {
        if unsafe { *a.add(i) } != expected[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn app_descriptor_init() {
    // SAFETY: APP_DESCRIPTOR is a module-private static initialized exactly
    // once here, before `app_install` publishes it to the firmware.
    unsafe {
        core::ptr::write_bytes(
            core::ptr::addr_of_mut!(APP_DESCRIPTOR).cast::<u8>(),
            0,
            core::mem::size_of::<launcher_app_descriptor>(),
        );
    }
    // SAFETY: writes through the freshly zeroed static.
    unsafe {
        let app = &mut *core::ptr::addr_of_mut!(APP_DESCRIPTOR).cast::<launcher_app_descriptor>();
        app.package_name = PACKAGE_NAME.as_ptr() as *mut core::ffi::c_void;
        app.launcher_icon_resource = LAUNCHER_ICON.as_ptr() as *mut core::ffi::c_void;
        app.app_id = APP_ID;
        app.launcher_metadata_callback =
            launcher_display_name as *const () as *mut core::ffi::c_void;
    }
}

fn descriptor_init(index: usize, name: &[u8], page_id: u16) {
    let descriptor = page_descriptor_ptr(index);
    // SAFETY: descriptor points at a zero-valid region of the static array;
    // the firmware does not inspect it until `app_install` is called. Cast to
    // bytes because `write_bytes` counts elements, not bytes.
    unsafe {
        core::ptr::write_bytes(
            descriptor.cast::<u8>(),
            0,
            core::mem::size_of::<firmware_page_descriptor>(),
        );
        (*descriptor).page_name = name.as_ptr() as *mut core::ffi::c_void;
        (*descriptor).page_id = page_id;
        (*descriptor).app_id = APP_ID;
        (*descriptor).on_signal = page_on_signal as *const () as *mut core::ffi::c_void;
        (*descriptor).on_create = page_on_create as *const () as *mut core::ffi::c_void;
        (*descriptor).on_resume = page_on_resume as *const () as *mut core::ffi::c_void;
        (*descriptor).on_pause = page_on_pause as *const () as *mut core::ffi::c_void;
        (*descriptor).on_destroy = page_on_destroy as *const () as *mut core::ffi::c_void;
    }
}

/// Executes one native-app publication stage. Stage 1 registers the app and
/// pages, while stage 2 adds its Launcher entry after miwear has processed the
/// app-registry event.
pub fn install_stage(stage: u32) -> Result<(), i32> {
    let r = runtime();
    let existing = unsafe { app_lookup(APP_ID) };

    if stage == 1 {
        if !existing.is_null() {
            // The installed app object carries its package name pointer at +0x8.
            let package: *const u8 =
                unsafe { core::ptr::read(existing.cast::<u8>().add(8) as *const *const u8) };
            if !c_str_equal(package, PACKAGE_NAME) {
                r.app_error.store(-101, Ordering::Release);
                r.app_state.store(APP_FAILED, Ordering::Release);
                return Err(-101);
            }
            r.app_state.store(APP_REGISTERED, Ordering::Release);
            r.app_error.store(0, Ordering::Release);
            return Ok(());
        }

        app_descriptor_init();
        descriptor_init(PAGE_OVERVIEW, PAGE_NAME_OVERVIEW, PAGE_OVERVIEW as u16);
        descriptor_init(PAGE_DETAIL, PAGE_NAME_DETAIL, PAGE_DETAIL as u16);

        // SAFETY: descriptors are zeroed and fully initialized above; app_install
        // consumes the local pointer array synchronously and retains the descriptors.
        let pages: [*mut firmware_page_descriptor; PAGE_COUNT] =
            [page_descriptor_ptr(0), page_descriptor_ptr(1)];
        let install_result = unsafe {
            app_install(
                core::ptr::addr_of_mut!(APP_DESCRIPTOR).cast::<launcher_app_descriptor>(),
                pages.as_ptr(),
                PAGE_COUNT as u32,
            )
        };
        r.app_install_result
            .store(install_result, Ordering::Release);
        let installed = unsafe { app_lookup(APP_ID) };
        if installed.is_null() {
            r.app_error.store(-100, Ordering::Release);
            r.app_state.store(APP_FAILED, Ordering::Release);
            return Err(-100);
        }
        let package: *const u8 =
            unsafe { core::ptr::read(installed.cast::<u8>().add(8) as *const *const u8) };
        if !c_str_equal(package, PACKAGE_NAME) {
            r.app_error.store(-101, Ordering::Release);
            r.app_state.store(APP_FAILED, Ordering::Release);
            return Err(-101);
        }
        r.app_state.store(APP_REGISTERED, Ordering::Release);
        r.app_error.store(0, Ordering::Release);
        return Ok(());
    }

    if stage == 2 {
        if existing.is_null() {
            r.app_error.store(-102, Ordering::Release);
            r.app_state.store(APP_FAILED, Ordering::Release);
            return Err(-102);
        }
        let package: *const u8 =
            unsafe { core::ptr::read(existing.cast::<u8>().add(8) as *const *const u8) };
        if !c_str_equal(package, PACKAGE_NAME) {
            r.app_error.store(-101, Ordering::Release);
            r.app_state.store(APP_FAILED, Ordering::Release);
            return Err(-101);
        }
        if r.app_state.load(Ordering::Acquire) == APP_OK {
            return Ok(());
        }
        // `app_launcher_add` returns an implementation-defined launcher
        // bookkeeping result, not a zero-on-success status. The installed app
        // lookup above is the validity gate; retain the raw result for status
        // diagnostics and treat publication as complete once the call returns.
        let launcher_result = unsafe { launcher_add(APP_ID) };
        r.launcher_add_result
            .store(launcher_result, Ordering::Release);
        r.app_state.store(APP_OK, Ordering::Release);
        r.app_error.store(0, Ordering::Release);
        return Ok(());
    }

    Err(-103)
}

// ---------------------------------------------------------------------------
// Page lifecycle (firmware -> module). Renders happen only from the page
// owner thread; LVX is never touched from Bluetooth/timer callbacks.
// ---------------------------------------------------------------------------

fn page_id_of(page: *mut firmware_page_descriptor) -> usize {
    if page.is_null() {
        return usize::MAX;
    }
    // SAFETY: the firmware passes one of the descriptors we registered.
    usize::from(unsafe { (*page).page_id })
}

extern "C" fn page_on_signal(
    _page: *mut firmware_page_descriptor,
    _event: u32,
    _payload: *mut core::ffi::c_void,
) -> i32 {
    0
}

extern "C" fn page_on_create(
    page: *mut firmware_page_descriptor,
    root: *mut core::ffi::c_void,
    _start_data: *mut core::ffi::c_void,
) -> i32 {
    let index = page_id_of(page);
    if index >= PAGE_COUNT {
        return -1;
    }
    ui_backend::page_create(index, root)
}

extern "C" fn page_on_resume(page: *mut firmware_page_descriptor) -> i32 {
    let index = page_id_of(page);
    if index >= PAGE_COUNT {
        return -1;
    }
    ui_backend::page_resume(index)
}

extern "C" fn page_on_pause(page: *mut firmware_page_descriptor) -> i32 {
    let index = page_id_of(page);
    if index >= PAGE_COUNT {
        return -1;
    }
    ui_backend::page_pause(index)
}

extern "C" fn page_on_destroy(page: *mut firmware_page_descriptor) -> i32 {
    let index = page_id_of(page);
    if index >= PAGE_COUNT {
        return -1;
    }
    ui_backend::page_destroy(index)
}
