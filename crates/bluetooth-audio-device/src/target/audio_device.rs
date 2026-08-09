//! `/dev/canopus_audio` single-writer compressed-audio endpoint.

use core::{
    ffi::c_void,
    mem::{MaybeUninit, size_of},
    sync::atomic::{AtomicUsize, Ordering},
};

use canopus_bluetooth_audio_core::audio_input::{
    ABI_VERSION, AudioInput, FormatV1, InputError, StatusV1,
};
use canopus_target_private::{bt_alloc, bt_free, canopus_fw_register_driver, file_operations};

use super::audio_stream;

const DEVICE_PATH: &[u8] = b"/dev/canopus_audio\0";
const DEVICE_MODE: u32 = 0o666;
pub const INPUT_CAPACITY: usize = 16 * 1024;

const IOC_GET_ABI: u32 = 0x4341_0001;
const IOC_SET_FORMAT: u32 = 0x4341_0002;
const IOC_START: u32 = 0x4341_0003;
const IOC_PAUSE: u32 = 0x4341_0004;
const IOC_RESUME: u32 = 0x4341_0005;
const IOC_STOP: u32 = 0x4341_0006;
const IOC_DRAIN: u32 = 0x4341_0007;
const IOC_FLUSH: u32 = 0x4341_0008;
const IOC_GET_STATUS: u32 = 0x4341_0009;
const IOC_SET_VOLUME: u32 = 0x4341_000a;
const IOC_GET_VOLUME: u32 = 0x4341_000b;

static AUDIO_INPUT_PTR: AtomicUsize = AtomicUsize::new(0);
static mut FILE_OPERATIONS: MaybeUninit<file_operations> = MaybeUninit::uninit();

pub fn input() -> &'static AudioInput<INPUT_CAPACITY> {
    let pointer = AUDIO_INPUT_PTR.load(Ordering::Acquire) as *const AudioInput<INPUT_CAPACITY>;
    // SAFETY: register initializes and publishes this resident allocation before
    // the driver or any external-stream transport path can call input().
    unsafe { &*pointer }
}

pub fn register() -> Result<(), i32> {
    let allocation = unsafe { bt_alloc(size_of::<AudioInput<INPUT_CAPACITY>>() as u32) }
        as *mut AudioInput<INPUT_CAPACITY>;
    if allocation.is_null() {
        return Err(-12);
    }
    // SAFETY: this fresh allocation is unpublished and correctly aligned.
    unsafe { AudioInput::initialize_at(allocation) };
    if AUDIO_INPUT_PTR
        .compare_exchange(0, allocation as usize, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        unsafe { bt_free(allocation.cast()) };
        return Err(-16);
    }
    let operations = file_operations {
        open: audio_open as *const () as *mut c_void,
        close: audio_close as *const () as *mut c_void,
        read: audio_read as *const () as *mut c_void,
        write: audio_write as *const () as *mut c_void,
        _pad_10: [0; 4],
        ioctl: audio_ioctl as *const () as *mut c_void,
        _tail: [0; 24],
    };
    let operations_ptr = core::ptr::addr_of_mut!(FILE_OPERATIONS).cast::<file_operations>();
    // SAFETY: activation is single-threaded and this resident table is written
    // once before register_driver publishes its address.
    unsafe { operations_ptr.write(operations) };
    let result = unsafe {
        canopus_fw_register_driver(
            DEVICE_PATH.as_ptr(),
            operations_ptr.cast(),
            DEVICE_MODE,
            core::ptr::null_mut(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
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
            errno(input().set_volume(volume))
        }
        IOC_GET_VOLUME => {
            if argument == 0 {
                return InputError::Invalid.errno();
            }
            // SAFETY: ioctl ABI requires a writable u32 percentage.
            unsafe { (argument as *mut u32).write_unaligned(input().volume()) };
            0
        }
        _ => InputError::Invalid.errno(),
    }
}
