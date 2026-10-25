//! Native page descriptors and lifecycle callbacks. The Supervisor owns app
//! identity, firmware installation and launcher publication. Opening pages
//! remains a launcher or in-app user action.

use canopus_abi::{AppEntryV1, ContextV1};
use core::sync::atomic::{AtomicBool, Ordering};

use canopus_target_private::*;

use super::runtime::*;
use super::ui_backend;

pub const PAGE_COUNT: usize = 3;
pub const PAGE_OVERVIEW: usize = 0;
pub const PAGE_DETAIL: usize = 1;
pub const PAGE_DIAGNOSTICS: usize = 2;

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

static mut PAGE_DESCRIPTORS: core::mem::MaybeUninit<[firmware_page_descriptor; PAGE_COUNT]> =
    core::mem::MaybeUninit::uninit();

pub fn page_descriptor_ptr(index: usize) -> *mut firmware_page_descriptor {
    // SAFETY: PAGE_DESCRIPTORS is initialized by `install_stage` before any page
    // lifecycle callback can run; the Supervisor retains these descriptors
    // until reboot. `addr_of_mut!` avoids the `static_mut_refs` deny
    // lint; the caller uses raw pointer writes.
    unsafe {
        core::ptr::addr_of_mut!(PAGE_DESCRIPTORS)
            .cast::<firmware_page_descriptor>()
            .add(index)
    }
}

fn descriptor_init(index: usize, name: &[u8], page_id: u16) {
    let descriptor = page_descriptor_ptr(index);
    // SAFETY: descriptor points at a zero-valid region of the static array;
    // the firmware does not inspect it until the Supervisor publishes it. Cast to
    // bytes because `write_bytes` counts elements, not bytes.
    unsafe {
        core::ptr::write_bytes(
            descriptor.cast::<u8>(),
            0,
            core::mem::size_of::<firmware_page_descriptor>(),
        );
        (*descriptor).page_name = name.as_ptr() as *mut core::ffi::c_void;
        (*descriptor).page_id = page_id;
        // Supervisor writes app_id with the allocated identity during publication.
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

/// Queues native pages at stage 1 and confirms registration at stage 2.
/// Supervisor defers stage 2 until app installation and launcher publication
/// succeed. Publication does not open the app.
///
/// # Safety
/// `ctx` must be the live context supplied on the serialized module/UI owner.
pub unsafe fn install_stage(ctx: *const ContextV1, stage: u32) -> Result<(), i32> {
    let registration = &runtime().app;
    if stage == 1 {
        // Keep descriptors already retained by the Supervisor intact.
        if registration.id() != 0 {
            return Ok(());
        }
        descriptor_init(PAGE_OVERVIEW, PAGE_NAME_OVERVIEW, PAGE_OVERVIEW as u16);
        descriptor_init(PAGE_DETAIL, PAGE_NAME_DETAIL, PAGE_DETAIL as u16);
        descriptor_init(
            PAGE_DIAGNOSTICS,
            PAGE_NAME_DIAGNOSTICS,
            PAGE_DIAGNOSTICS as u16,
        );
        let pages = [
            page_descriptor_ptr(PAGE_OVERVIEW).cast(),
            page_descriptor_ptr(PAGE_DETAIL).cast(),
            page_descriptor_ptr(PAGE_DIAGNOSTICS).cast(),
        ];
        let entry = AppEntryV1 {
            struct_size: core::mem::size_of::<AppEntryV1>() as u32,
            label: DISPLAY_NAME.as_ptr().cast(),
            icon: icon_path().unwrap_or(ICON_PATH).as_ptr().cast(),
            pages: pages.as_ptr(),
            page_count: PAGE_COUNT as u32,
        };
        // The Supervisor copies this stack array; descriptors and callbacks remain
        // in module-owned storage until reboot.
        return unsafe { registration.register(ctx, &entry) };
    }
    if stage == 2 {
        registration.page_key(PAGE_OVERVIEW).ok_or(-16)?;
        return Ok(());
    }
    Err(-22)
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
