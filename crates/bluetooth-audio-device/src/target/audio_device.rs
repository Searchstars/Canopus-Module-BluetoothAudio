//! `/dev/canopus_audio` single-writer compressed-audio endpoint.

use core::{
    ffi::c_void,
    mem::{MaybeUninit, size_of},
    sync::atomic::{AtomicI32, AtomicU32, AtomicUsize, Ordering},
};

use canopus_bluetooth_audio_core::audio_input::{
    ABI_VERSION, AudioInput, FormatV1, InputError, StatusV1,
};
#[cfg(not(feature = "target-xiaomi-band-9-pro-3-1-175"))]
use canopus_target_private::canopus_fw_register_driver;
use canopus_target_private::{
    O_RDWR, bt_alloc, bt_free, file_operations, get_errno, nuttx_close, nuttx_ioctl, nuttx_open,
};

use super::{audio_stream, transport};

const DEVICE_PATH: &[u8] = b"/dev/canopus_audio\0";
#[cfg(not(feature = "target-xiaomi-band-9-pro-3-1-175"))]
const DEVICE_MODE: u32 = 0o666;
pub const INPUT_CAPACITY: usize = 16 * 1024;

const IOC_GET_ABI: u32 = 0x304;
const IOC_SET_FORMAT: u32 = 0x305;
const IOC_START: u32 = 0x306;
const IOC_PAUSE: u32 = 0x307;
const IOC_RESUME: u32 = 0x308;
const IOC_STOP: u32 = 0x309;
const IOC_DRAIN: u32 = 0x30d;
const IOC_FLUSH: u32 = 0x30e;
const IOC_GET_STATUS: u32 = 0x30f;
const IOC_SET_VOLUME: u32 = 0x310;
const IOC_GET_VOLUME: u32 = 0x311;

static AUDIO_INPUT_PTR: AtomicUsize = AtomicUsize::new(0);
static mut FILE_OPERATIONS: MaybeUninit<file_operations> = MaybeUninit::uninit();
static AUDIO_REGISTER_RESULT: AtomicI32 = AtomicI32::new(-1);
static AUDIO_PROBE_RESULT: AtomicI32 = AtomicI32::new(-1);
static AUDIO_PROBE_ABI: AtomicU32 = AtomicU32::new(0);
static AUDIO_LAST_COMMAND: AtomicU32 = AtomicU32::new(0);

pub fn diagnostics() -> (i32, i32, u32, u32) {
    (
        AUDIO_REGISTER_RESULT.load(Ordering::Acquire),
        AUDIO_PROBE_RESULT.load(Ordering::Acquire),
        AUDIO_PROBE_ABI.load(Ordering::Acquire),
        AUDIO_LAST_COMMAND.load(Ordering::Acquire),
    )
}

fn normalize_result(result: i32) -> i32 {
    if result == -1 {
        let errno = unsafe { get_errno() };
        if errno > 0 {
            return -errno;
        }
    }
    result
}

fn probe_endpoint() -> (i32, u32) {
    let fd = unsafe { nuttx_open(DEVICE_PATH.as_ptr(), O_RDWR) };
    if fd < 0 {
        return (normalize_result(fd), 0);
    }
    let mut abi = 0u32;
    AUDIO_LAST_COMMAND.store(IOC_GET_ABI, Ordering::Release);
    let result = normalize_result(unsafe {
        nuttx_ioctl(fd, IOC_GET_ABI, core::ptr::addr_of_mut!(abi) as usize)
    });
    let _ = unsafe { nuttx_close(fd) };
    if result == 0 && abi != ABI_VERSION {
        return (-22, abi);
    }
    (result, abi)
}

pub fn status() -> Option<StatusV1> {
    let pointer = AUDIO_INPUT_PTR.load(Ordering::Acquire) as *const AudioInput<INPUT_CAPACITY>;
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { (&*pointer).status() })
    }
}

pub fn input() -> &'static AudioInput<INPUT_CAPACITY> {
    let pointer = AUDIO_INPUT_PTR.load(Ordering::Acquire) as *const AudioInput<INPUT_CAPACITY>;
    // SAFETY: register initializes and publishes this resident allocation before
    // the driver or any external-stream transport path can call input().
    unsafe { &*pointer }
}

pub fn register() -> Result<(), i32> {
    AUDIO_REGISTER_RESULT.store(-1, Ordering::Release);
    AUDIO_PROBE_RESULT.store(-1, Ordering::Release);
    AUDIO_PROBE_ABI.store(0, Ordering::Release);
    let allocation = unsafe { bt_alloc(size_of::<AudioInput<INPUT_CAPACITY>>() as u32) }
        as *mut AudioInput<INPUT_CAPACITY>;
    if allocation.is_null() {
        AUDIO_REGISTER_RESULT.store(-12, Ordering::Release);
        return Err(-12);
    }
    // SAFETY: this fresh allocation is unpublished and correctly aligned.
    unsafe { AudioInput::initialize_at(allocation) };
    if AUDIO_INPUT_PTR
        .compare_exchange(0, allocation as usize, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        unsafe { bt_free(allocation.cast()) };
        AUDIO_REGISTER_RESULT.store(-16, Ordering::Release);
        return Err(-16);
    }
    #[cfg(not(feature = "device"))]
    let operations = file_operations {
        open: audio_open as *const () as *mut c_void,
        close: audio_close as *const () as *mut c_void,
        read: audio_read as *const () as *mut c_void,
        write: audio_write as *const () as *mut c_void,
        _pad_10: [0; 4],
        ioctl: audio_ioctl as *const () as *mut c_void,
        _tail: [0; 24],
    };
    #[cfg(any(
        feature = "target-xiaomi-band-10-pro-3-101-030",
        feature = "target-xiaomi-band-10-pro-3-101-036"
    ))]
    let operations = file_operations {
        open: audio_open as *const () as *mut c_void,
        close: audio_close as *const () as *mut c_void,
        read: audio_read as *const () as *mut c_void,
        write: audio_write as *const () as *mut c_void,
        lseek: core::ptr::null_mut(),
        ioctl: audio_ioctl as *const () as *mut c_void,
        _tail: [0; 24],
    };
    #[cfg(feature = "target-xiaomi-band-9-pro-3-1-175")]
    let operations = file_operations {
        open: audio_open as *const () as *mut c_void,
        close: audio_close as *const () as *mut c_void,
        read: audio_read as *const () as *mut c_void,
        write: audio_write as *const () as *mut c_void,
        lseek: core::ptr::null_mut(),
        ioctl: audio_ioctl as *const () as *mut c_void,
        _pad_18: [0; 8],
        fsync: core::ptr::null_mut(),
        _tail: [0; 12],
    };
    let operations_ptr = core::ptr::addr_of_mut!(FILE_OPERATIONS).cast::<file_operations>();
    // SAFETY: activation is single-threaded and this resident table is written
    // once before register_driver publishes its address.
    unsafe { operations_ptr.write(operations) };
    #[cfg(feature = "target-xiaomi-band-9-pro-3-1-175")]
    let result = unsafe {
        // Band-9 register_driver is 3-arg (no mode_t); the driver is registered
        // on the band-9 NuttX variant with a NULL private argument.
        canopus_target_private::canopus_fw_register_driver_b9(
            DEVICE_PATH.as_ptr(),
            operations_ptr.cast(),
            core::ptr::null_mut(),
        )
    };
    #[cfg(not(feature = "target-xiaomi-band-9-pro-3-1-175"))]
    let result = unsafe {
        canopus_fw_register_driver(
            DEVICE_PATH.as_ptr(),
            operations_ptr.cast(),
            DEVICE_MODE,
            core::ptr::null_mut(),
        )
    };
    if result == 0 {
        AUDIO_REGISTER_RESULT.store(0, Ordering::Release);
        let (probe_result, abi) = probe_endpoint();
        AUDIO_PROBE_RESULT.store(probe_result, Ordering::Release);
        AUDIO_PROBE_ABI.store(abi, Ordering::Release);
        Ok(())
    } else {
        AUDIO_REGISTER_RESULT.store(result, Ordering::Release);
        AUDIO_INPUT_PTR.store(0, Ordering::Release);
        unsafe { bt_free(allocation.cast()) };
        Err(result)
    }
}

fn errno(result: Result<(), InputError>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) => error.errno(),
    }
}

fn schedule_control(result: Result<(), InputError>, schedule: fn() -> Result<(), i32>) -> i32 {
    match result {
        Ok(()) => match schedule() {
            Ok(()) => 0,
            Err(error) => {
                input().fail(input().generation(), error);
                error
            }
        },
        Err(error) => error.errno(),
    }
}

fn set_local_volume(volume: u32) -> i32 {
    // The A2DP source streams full-scale PCM; the sink (headset) is the volume
    // authority. Local volume changes are forwarded to the headset over AVRCP
    // absolute volume and never attenuate the source's own PCM gain.
    if volume > canopus_bluetooth_audio_core::audio_input::VOLUME_MAX {
        return InputError::Invalid.errno();
    }
    match transport::set_absolute_volume(volume) {
        Ok(()) => 0,
        Err(error) => error,
    }
}

extern "C" fn audio_open(_file: *mut c_void) -> i32 {
    errno(input().open())
}

extern "C" fn audio_close(_file: *mut c_void) -> i32 {
    match input().close() {
        Ok(()) => match audio_stream::schedule_close() {
            Ok(()) => 0,
            Err(error) => error,
        },
        Err(error) => error.errno(),
    }
}

extern "C" fn audio_read(_file: *mut c_void, _buffer: *mut c_void, _count: u32) -> i32 {
    -38
}

extern "C" fn audio_write(_file: *mut c_void, buffer: *const c_void, count: u32) -> i32 {
    if count == 0 {
        return 0;
    }
    if buffer.is_null() {
        return InputError::Invalid.errno();
    }
    // The compressed ring is SPSC. The built-in long-test feeder is its sole
    // producer while active, so an external writer must not enter concurrently.
    if audio_stream::long_test_owns_input() {
        return -16;
    }
    // SAFETY: NuttX supplies the caller buffer for the duration of write.
    let bytes = unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), count as usize) };
    match input().write(bytes) {
        Ok(written) => {
            if let Err(error) = audio_stream::schedule_wake() {
                input().fail(input().generation(), error);
            }
            written as i32
        }
        Err(error) => error.errno(),
    }
}

extern "C" fn audio_ioctl(_file: *mut c_void, command: i32, argument: usize) -> i32 {
    AUDIO_LAST_COMMAND.store(command as u32, Ordering::Release);
    match command as u32 {
        IOC_GET_ABI => {
            if argument == 0 {
                return InputError::Invalid.errno();
            }
            // SAFETY: ioctl ABI requires a writable u32 argument.
            unsafe { (argument as *mut u32).write_unaligned(ABI_VERSION) };
            0
        }
        IOC_SET_FORMAT => {
            if argument == 0 {
                return InputError::Invalid.errno();
            }
            // SAFETY: ioctl ABI requires a readable FormatV1 argument.
            let format = unsafe { (argument as *const FormatV1).read_unaligned() };
            schedule_control(input().set_format(&format), audio_stream::schedule_flush)
        }
        IOC_START => schedule_control(input().start(), audio_stream::schedule_start),
        IOC_PAUSE => schedule_control(input().pause(), audio_stream::schedule_pause),
        IOC_RESUME => schedule_control(input().resume(), audio_stream::schedule_resume),
        IOC_STOP => schedule_control(input().stop(), audio_stream::schedule_stop),
        IOC_DRAIN => schedule_control(input().drain(), audio_stream::schedule_drain),
        IOC_FLUSH => schedule_control(input().flush(), audio_stream::schedule_flush),
        IOC_GET_STATUS => {
            if argument == 0 {
                return InputError::Invalid.errno();
            }
            let status: StatusV1 = input().status();
            // SAFETY: ioctl ABI requires a writable StatusV1 argument.
            unsafe { (argument as *mut StatusV1).write_unaligned(status) };
            0
        }
        IOC_SET_VOLUME => {
            if argument == 0 {
                return InputError::Invalid.errno();
            }
            // SAFETY: ioctl ABI requires a readable u32 percentage.
            let volume = unsafe { (argument as *const u32).read_unaligned() };
            set_local_volume(volume)
        }
        IOC_GET_VOLUME => {
            if argument == 0 {
                return InputError::Invalid.errno();
            }
            // SAFETY: ioctl ABI requires a writable u32 percentage.
            unsafe { (argument as *mut u32).write_unaligned(transport::absolute_volume_percent()) };
            0
        }
        _ => InputError::Invalid.errno(),
    }
}
