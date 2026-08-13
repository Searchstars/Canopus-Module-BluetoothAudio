//! Fixed-size per-headset absolute-volume persistence.

use canopus_target_private::{
    O_RDONLY, nuttx_close, nuttx_create, nuttx_open, nuttx_read, nuttx_rename, nuttx_unlink,
    nuttx_write,
};

use super::runtime::{target_load, target_matches};

const PATH: &[u8] = b"/data/canopus_btaudio_volume.bin\0";
const TEMP_PATH: &[u8] = b"/data/canopus_btaudio_volume.tmp\0";
const MAGIC: [u8; 4] = *b"BAV2";
const RECORDS: usize = 8;
const HEADER_SIZE: usize = 8;
const RECORD_SIZE: usize = 8;
const FILE_SIZE: usize = HEADER_SIZE + RECORDS * RECORD_SIZE + 4;
const O_WRONLY: i32 = 2;
const O_CREAT: i32 = 4;
const FILE_MODE: u32 = 0o600;

static mut SELECTED_ADDRESS: [u8; 6] = [0; 6];
static mut SELECTED_VOLUME: u8 = 0x7f;
static mut SELECTED_VALID: bool = false;
static mut PENDING_ADDRESS: [u8; 6] = [0; 6];
static mut PENDING_VOLUME: u8 = 0;
static mut PENDING_DIRTY: bool = false;

fn checksum(bytes: &[u8]) -> u32 {
    let mut value = 0x811c_9dc5u32;
    for byte in bytes {
        value ^= u32::from(*byte);
        value = value.wrapping_mul(0x0100_0193);
    }
    value
}

fn read_file(out: &mut [u8; FILE_SIZE]) -> bool {
    let fd = unsafe { nuttx_open(PATH.as_ptr(), O_RDONLY) };
    if fd < 0 {
        return false;
    }
    let read = unsafe { nuttx_read(fd, out.as_mut_ptr().cast(), FILE_SIZE as u32) };
    unsafe { nuttx_close(fd) };
    if read != FILE_SIZE as i32 || out[..4] != MAGIC || out[4] != 2 {
        return false;
    }
    let stored = u32::from_le_bytes(out[FILE_SIZE - 4..].try_into().unwrap_or([0; 4]));
    stored == checksum(&out[..FILE_SIZE - 4])
}

pub fn select(address: [u8; 6], fallback: u8) -> u8 {
    let volume = load(address).unwrap_or(fallback.min(0x7f));
    unsafe {
        SELECTED_ADDRESS = address;
        SELECTED_VOLUME = volume;
        SELECTED_VALID = true;
    }
    volume
}

pub fn selected(address: [u8; 6], fallback: u8) -> u8 {
    unsafe {
        if SELECTED_VALID && SELECTED_ADDRESS == address {
            SELECTED_VOLUME
        } else {
            fallback.min(0x7f)
        }
    }
}

pub fn load(address: [u8; 6]) -> Option<u8> {
    let mut file = [0u8; FILE_SIZE];
    if !read_file(&mut file) {
        return None;
    }
    let count = usize::from(file[5]).min(RECORDS);
    for index in 0..count {
        let offset = HEADER_SIZE + index * RECORD_SIZE;
        if file[offset..offset + 6] == address && file[offset + 6] <= 0x7f {
            return Some(file[offset + 6]);
        }
    }
    None
}

/// Records a callback-produced update without performing file I/O in the
/// Bluetooth owner. The page-owner refresh later calls [`flush_pending`].
pub fn mark_pending(address: [u8; 6], volume: u8) {
    // This module has one target connection. Callback and page-owner accesses are
    // serialized by the core lock before entering these functions.
    unsafe {
        SELECTED_ADDRESS = address;
        SELECTED_VOLUME = volume.min(0x7f);
        SELECTED_VALID = true;
        PENDING_ADDRESS = address;
        PENDING_VOLUME = volume.min(0x7f);
        PENDING_DIRTY = true;
    }
}

pub fn mark_target_pending(volume: u8) {
    if let Some(address) = target_load() {
        mark_pending(address, volume);
    }
}

pub fn flush_pending() -> bool {
    let (address, volume) = unsafe {
        if !PENDING_DIRTY {
            return true;
        }
        (PENDING_ADDRESS, PENDING_VOLUME)
    };
    if !target_matches(address) || !store(address, volume) {
        return false;
    }
    unsafe {
        if PENDING_ADDRESS == address && PENDING_VOLUME == volume {
            PENDING_DIRTY = false;
        }
    }
    true
}

fn store(address: [u8; 6], volume: u8) -> bool {
    let mut old = [0u8; FILE_SIZE];
    let valid = read_file(&mut old);
    let old_count = if valid {
        usize::from(old[5]).min(RECORDS)
    } else {
        0
    };
    let mut file = [0u8; FILE_SIZE];
    file[..4].copy_from_slice(&MAGIC);
    file[4] = 2;
    let mut count = 1usize;
    file[HEADER_SIZE..HEADER_SIZE + 6].copy_from_slice(&address);
    file[HEADER_SIZE + 6] = volume.min(0x7f);
    for index in 0..old_count {
        if count == RECORDS {
            break;
        }
        let source = HEADER_SIZE + index * RECORD_SIZE;
        if old[source..source + 6] == address {
            continue;
        }
        let destination = HEADER_SIZE + count * RECORD_SIZE;
        file[destination..destination + RECORD_SIZE]
            .copy_from_slice(&old[source..source + RECORD_SIZE]);
        count += 1;
    }
    file[5] = count as u8;
    let sum = checksum(&file[..FILE_SIZE - 4]);
    file[FILE_SIZE - 4..].copy_from_slice(&sum.to_le_bytes());

    unsafe { nuttx_unlink(TEMP_PATH.as_ptr()) };
    let fd = unsafe { nuttx_create(TEMP_PATH.as_ptr(), O_WRONLY | O_CREAT, FILE_MODE) };
    if fd < 0 {
        return false;
    }
    let written = unsafe { nuttx_write(fd, file.as_ptr().cast(), FILE_SIZE as u32) };
    let closed = unsafe { nuttx_close(fd) };
    if written != FILE_SIZE as i32 || closed != 0 {
        unsafe { nuttx_unlink(TEMP_PATH.as_ptr()) };
        return false;
    }
    if unsafe { nuttx_rename(TEMP_PATH.as_ptr(), PATH.as_ptr()) } != 0 {
        unsafe { nuttx_unlink(TEMP_PATH.as_ptr()) };
        return false;
    }
    true
}
