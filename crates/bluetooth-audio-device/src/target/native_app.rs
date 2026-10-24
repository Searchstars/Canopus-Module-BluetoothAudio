//! Native app registration: fixed 8-bit app id, stock launcher entry, two page
//! descriptors (overview + detail), and the page lifecycle callbacks. App/page
//! registration and Launcher publication are intentionally separate invocations
//! so miwear can process the app-registry event before Launcher persistence.

use core::sync::atomic::{AtomicBool, Ordering};

use canopus_target_private::*;

use super::runtime::*;
use super::ui_backend;

pub const APP_ID: u16 = 0x00CB;
pub const PAGE_COUNT: usize = 3;
pub const PAGE_OVERVIEW: usize = 0;
pub const PAGE_DETAIL: usize = 1;
pub const PAGE_DIAGNOSTICS: usize = 2;

pub const PACKAGE_NAME: &[u8] = b"com.canopus.headphones\0";
pub const DISPLAY_NAME: &[u8] = &[0xE8, 0x80, 0xB3, 0xE6, 0x9C, 0xBA, 0];
/// The headset artwork, which is also the launcher icon: the stock launcher
/// reads it by path as a 117x117 LVGL v9 ARGB8888 image and leaves the app out
/// when the file is missing. The installer watchfaces stage this same file;
/// a package install does not, so the module writes it itself.
const ICON_PATH: &[u8] = b"/data/canopus/appicon_headphones.bin\0";
const ICON_TEMP_PATH: &[u8] = b"/data/canopus/appicon_headphones.tmp\0";
const PAGE_NAME_OVERVIEW: &[u8] = b"headphones\0";
const PAGE_NAME_DETAIL: &[u8] = b"headphones_detail\0";
const PAGE_NAME_DIAGNOSTICS: &[u8] = b"headphones_diagnostics\0";
const O_WRONLY: i32 = 2;
const O_CREAT: i32 = 4;
const FILE_MODE: u32 = 0o644;

/// Keeps the embedded icon word aligned for the verifier's word scan.
#[repr(C)]
struct Aligned<Align, Bytes: ?Sized> {
    _align: [Align; 0],
    bytes: Bytes,
}

/// The icon as its LVGL header, a 256-entry ARGB8888 palette and one index
/// per pixel (`scripts/encode-icon-palette.py`): 15 KB carried instead of 54.
static ICON: &Aligned<u32, [u8]> = &Aligned {
    _align: [],
    bytes: *include_bytes!("../../assets/appicon_headphones.pal"),
};
const ICON_HEADER: usize = 12;
const ICON_PALETTE: usize = 256 * 4;
const ICON_SIDE: usize = 117;
const ICON_ROW: usize = ICON_SIDE * 4;

static ICON_READY: AtomicBool = AtomicBool::new(false);

/// Row `y` of the icon in ARGB8888.
fn icon_row(y: usize, out: &mut [u8; ICON_ROW]) {
    let icon = &ICON.bytes;
    let indices = &icon[ICON_HEADER + ICON_PALETTE + y * ICON_SIDE..][..ICON_SIDE];
    for (pixel, &index) in out.chunks_exact_mut(4).zip(indices) {
        let entry = ICON_HEADER + usize::from(index) * 4;
        pixel.copy_from_slice(&icon[entry..entry + 4]);
    }
}

/// Whether the file at [`ICON_PATH`] already holds exactly this icon.
fn icon_on_disk() -> bool {
    let fd = unsafe { nuttx_open(ICON_PATH.as_ptr(), O_RDONLY) };
    if fd < 0 {
        return false;
    }
    let mut expected = [0u8; ICON_ROW];
    let mut actual = [0u8; ICON_ROW];
    let read = |buffer: &mut [u8]| unsafe {
        nuttx_read(fd, buffer.as_mut_ptr().cast(), buffer.len() as u32)
    };
    let mut same = read(&mut actual[..ICON_HEADER]) == ICON_HEADER as i32
        && actual[..ICON_HEADER] == ICON.bytes[..ICON_HEADER];
    for y in 0..ICON_SIDE {
        if !same {
            break;
        }
        icon_row(y, &mut expected);
        same = read(&mut actual) == ICON_ROW as i32 && actual == expected;
    }
    // Nothing may follow the image.
    same = same && read(&mut actual[..1]) == 0;
    unsafe { nuttx_close(fd) };
    same
}

fn write_icon() -> bool {
    unsafe { nuttx_unlink(ICON_TEMP_PATH.as_ptr()) };
    let fd = unsafe { nuttx_create(ICON_TEMP_PATH.as_ptr(), O_WRONLY | O_CREAT, FILE_MODE) };
    if fd < 0 {
        return false;
    }
    let write = |buffer: &[u8]| unsafe {
        nuttx_write(fd, buffer.as_ptr().cast(), buffer.len() as u32) == buffer.len() as i32
    };
    let mut row = [0u8; ICON_ROW];
    let mut written = write(&ICON.bytes[..ICON_HEADER]);
    for y in 0..ICON_SIDE {
        if !written {
            break;
        }
        icon_row(y, &mut row);
        written = write(&row);
    }
    let closed = unsafe { nuttx_close(fd) } == 0;
    if !written
        || !closed
        || unsafe { nuttx_rename(ICON_TEMP_PATH.as_ptr(), ICON_PATH.as_ptr()) } != 0
    {
        unsafe { nuttx_unlink(ICON_TEMP_PATH.as_ptr()) };
        return false;
    }
    true
}

/// The NUL-terminated path of the headset artwork once it is on disk, writing
/// it first when missing or different.
pub fn icon_path() -> Option<&'static [u8]> {
    if !ICON_READY.load(Ordering::Acquire) {
        if !icon_on_disk() && !write_icon() {
            return None;
        }
        ICON_READY.store(true, Ordering::Release);
    }
    Some(ICON_PATH)
}

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
        app.launcher_icon_resource =
            icon_path().unwrap_or(ICON_PATH).as_ptr() as *mut core::ffi::c_void;
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
        #[cfg(not(any(
            feature = "target-xiaomi-band-11-4-100-139",
            feature = "target-xiaomi-band-11-4-100-155",
            feature = "target-xiaomi-band-11-4-100-179"
        )))]
        {
            (*descriptor).on_destroy = page_on_destroy as *const () as *mut core::ffi::c_void;
        }
        #[cfg(any(
            feature = "target-xiaomi-band-11-4-100-139",
            feature = "target-xiaomi-band-11-4-100-155",
            feature = "target-xiaomi-band-11-4-100-179"
        ))]
        {
            (*descriptor).on_ui_destroy = page_on_destroy as *const () as *mut core::ffi::c_void;
        }
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
            // Read the package field using the exact target descriptor layout.
            let package: *const u8 = unsafe {
                core::ptr::read(existing.cast::<u8>().add(APP_PACKAGE_OFFSET) as *const *const u8)
            };
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
        descriptor_init(
            PAGE_DIAGNOSTICS,
            PAGE_NAME_DIAGNOSTICS,
            PAGE_DIAGNOSTICS as u16,
        );

        // SAFETY: descriptors are zeroed and fully initialized above; app_install
        // consumes the local pointer array synchronously and retains the descriptors.
        let pages: [*mut firmware_page_descriptor; PAGE_COUNT] = [
            page_descriptor_ptr(PAGE_OVERVIEW),
            page_descriptor_ptr(PAGE_DETAIL),
            page_descriptor_ptr(PAGE_DIAGNOSTICS),
        ];
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
        let package: *const u8 = unsafe {
            core::ptr::read(installed.cast::<u8>().add(APP_PACKAGE_OFFSET) as *const *const u8)
        };
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
        let package: *const u8 = unsafe {
            core::ptr::read(existing.cast::<u8>().add(APP_PACKAGE_OFFSET) as *const *const u8)
        };
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
