//! Fixed-profile software SBC encoder backed by the vendored BlueZ codec.

use core::ffi::c_void;

use canopus_target_private::{bt_alloc, bt_free};

unsafe extern "C" {
    fn canopus_sbc_encoder_size() -> u32;
    fn canopus_sbc_encoder_init(memory: *mut c_void, size: u32, bitpool: u8) -> *mut c_void;
    fn canopus_sbc_encoder_encode(
        context: *mut c_void,
        pcm: *const i16,
        pcm_frames: u32,
        output: *mut u8,
        output_capacity: u32,
    ) -> i32;
}

pub struct SbcEncoder {
    allocation: *mut c_void,
    context: *mut c_void,
    bitpool: u8,
}

impl SbcEncoder {
    pub fn new(bitpool: u8) -> Result<Self, i32> {
        let size = unsafe { canopus_sbc_encoder_size() };
        if size == 0 {
            return Err(-5);
        }
        let allocation = unsafe { bt_alloc(size) };
        if allocation.is_null() {
            return Err(-12);
        }
        let context = unsafe { canopus_sbc_encoder_init(allocation, size, bitpool) };
        if context.is_null() {
            unsafe { bt_free(allocation) };
            return Err(-22);
        }
        Ok(Self {
            allocation,
            context,
            bitpool,
        })
    }

    pub fn bitpool(&self) -> u8 {
        self.bitpool
    }

    pub fn encode(&mut self, pcm: &[i16; 256], output: &mut [u8]) -> Result<usize, i32> {
        let written = unsafe {
            canopus_sbc_encoder_encode(
                self.context,
                pcm.as_ptr(),
                128,
                output.as_mut_ptr(),
                output.len() as u32,
            )
        };
        if written > 0 {
            Ok(written as usize)
        } else {
            Err(written)
        }
    }
}

impl Drop for SbcEncoder {
    fn drop(&mut self) {
        if !self.allocation.is_null() {
            unsafe { bt_free(self.allocation) };
            self.allocation = core::ptr::null_mut();
            self.context = core::ptr::null_mut();
        }
    }
}
