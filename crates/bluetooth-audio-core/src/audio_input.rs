use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
};

pub const ABI_VERSION: u32 = 1;
pub const FORMAT_MP3: u32 = 1;
pub const FORMAT_PCM_S16LE: u32 = 2;
pub const VOLUME_DEFAULT: u32 = 100;
pub const VOLUME_MAX: u32 = 100;

pub const STATE_CLOSED: u32 = 0;
pub const STATE_IDLE: u32 = 1;
pub const STATE_CONFIGURED: u32 = 2;
pub const STATE_BUFFERING: u32 = 3;
pub const STATE_PLAYING: u32 = 4;
pub const STATE_PAUSED: u32 = 5;
pub const STATE_DRAINING: u32 = 6;
pub const STATE_STOPPED: u32 = 7;
pub const STATE_ERROR: u32 = 8;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum InputError {
    Busy,
    Invalid,
    Again,
    Pipe,
    Io,
}

impl InputError {
    pub const fn errno(self) -> i32 {
        match self {
            Self::Busy => -16,
            Self::Invalid => -22,
            Self::Again => -11,
            Self::Pipe => -32,
            Self::Io => -5,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FormatV1 {
    pub struct_size: u32,
    pub format: u32,
    pub sample_rate_hint: u32,
    pub channels_hint: u32,
    pub flags: u32,
    pub reserved: [u32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusV1 {
    pub struct_size: u32,
    pub abi_version: u32,
    pub state: u32,
    pub last_error: i32,
    pub format: u32,
    pub input_capacity: u32,
    pub input_used: u32,
    pub input_free: u32,
    pub decoded_sample_rate: u32,
    pub decoded_channels: u32,
    pub negotiated_bitpool: u32,
    pub bytes_accepted: u32,
    pub bytes_consumed: u32,
    pub pcm_frames: u32,
    pub rtp_packets: u32,
    pub underruns: u32,
    pub generation: u32,
    pub volume_percent: u32,
}

/// Lock-free single-producer/single-consumer byte ring. The device's exclusive
/// open establishes one writer; Bluetooth-owner work is the only consumer.
pub struct InputRing<const N: usize> {
    bytes: UnsafeCell<[u8; N]>,
    head: AtomicU32,
    tail: AtomicU32,
}

// SAFETY: only the exclusive file writer mutates producer slots and only the
// Bluetooth owner mutates consumer slots. Release/Acquire publishes contents.
unsafe impl<const N: usize> Sync for InputRing<N> {}

impl<const N: usize> InputRing<N> {
    pub const fn new() -> Self {
        assert!(N > 0 && N.is_power_of_two() && N <= u32::MAX as usize);
        Self {
            bytes: UnsafeCell::new([0; N]),
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
        }
    }

    pub fn used(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head.wrapping_sub(tail).min(N as u32) as usize
    }

    pub fn free(&self) -> usize {
        N - self.used()
    }

    pub fn write(&self, input: &[u8]) -> usize {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Relaxed);
        let free = N - head.wrapping_sub(tail).min(N as u32) as usize;
        let count = input.len().min(free);
        let mask = N - 1;
        for (offset, byte) in input[..count].iter().copied().enumerate() {
            let index = (head as usize + offset) & mask;
            // SAFETY: the producer owns all slots from head up to tail+N.
            unsafe { (*self.bytes.get())[index] = byte };
        }
        self.head
            .store(head.wrapping_add(count as u32), Ordering::Release);
        count
    }

    pub fn read(&self, output: &mut [u8]) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        let count = output
            .len()
            .min(head.wrapping_sub(tail).min(N as u32) as usize);
        let mask = N - 1;
        for (offset, byte) in output[..count].iter_mut().enumerate() {
            let index = (tail as usize + offset) & mask;
            // SAFETY: the consumer owns all published slots before head.
            *byte = unsafe { (*self.bytes.get())[index] };
        }
        self.tail
            .store(tail.wrapping_add(count as u32), Ordering::Release);
        count
    }

    /// Discards all published bytes. Call only after the session generation has
    /// invalidated producer/consumer work.
    pub fn reset(&self) {
        let head = self.head.load(Ordering::Acquire);
        self.tail.store(head, Ordering::Release);
    }
}

impl<const N: usize> Default for InputRing<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AudioInput<const N: usize> {
    pub ring: InputRing<N>,
    opened: AtomicBool,
    control_lock: AtomicBool,
    state: AtomicU32,
    format: AtomicU32,
    sample_rate_hint: AtomicU32,
    channels_hint: AtomicU32,
    last_error: AtomicI32,
    generation: AtomicU32,
    decoded_sample_rate: AtomicU32,
    decoded_channels: AtomicU32,
    negotiated_bitpool: AtomicU32,
    bytes_accepted: AtomicU32,
    bytes_consumed: AtomicU32,
    pcm_frames: AtomicU32,
    rtp_packets: AtomicU32,
    underruns: AtomicU32,
    volume_percent: AtomicU32,
}

impl<const N: usize> AudioInput<N> {
    pub const fn new() -> Self {
        Self {
            ring: InputRing::new(),
            opened: AtomicBool::new(false),
            control_lock: AtomicBool::new(false),
            state: AtomicU32::new(STATE_CLOSED),
            format: AtomicU32::new(0),
            sample_rate_hint: AtomicU32::new(0),
            channels_hint: AtomicU32::new(0),
            last_error: AtomicI32::new(0),
            generation: AtomicU32::new(1),
            decoded_sample_rate: AtomicU32::new(0),
            decoded_channels: AtomicU32::new(0),
            negotiated_bitpool: AtomicU32::new(0),
            bytes_accepted: AtomicU32::new(0),
            bytes_consumed: AtomicU32::new(0),
            pcm_frames: AtomicU32::new(0),
            rtp_packets: AtomicU32::new(0),
            underruns: AtomicU32::new(0),
            volume_percent: AtomicU32::new(VOLUME_DEFAULT),
        }
    }

    /// Initializes heap-backed endpoint storage without placing the ring on the
    /// caller's stack.
    ///
    /// # Safety
    ///
    /// `destination` must identify aligned, writable, uninitialized storage for
    /// one `AudioInput` and must not currently hold a live value.
    pub unsafe fn initialize_at(destination: *mut Self) {
        // InputRing storage, integer atomics, and false AtomicBool values all
        // have valid all-zero representations. Establish the two nonzero
        // constructor values explicitly after clearing the aggregate.
        unsafe {
            destination
                .cast::<u8>()
                .write_bytes(0, core::mem::size_of::<Self>());
            core::ptr::addr_of_mut!((*destination).generation).write(AtomicU32::new(1));
            core::ptr::addr_of_mut!((*destination).volume_percent)
                .write(AtomicU32::new(VOLUME_DEFAULT));
        }
    }

    fn try_control(&self) -> Result<ControlGuard<'_, N>, InputError> {
        self.control_lock
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ControlGuard(self))
            .map_err(|_| InputError::Busy)
    }

    pub fn open(&self) -> Result<(), InputError> {
        self.opened
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| InputError::Busy)?;
        let _guard = match self.try_control() {
            Ok(guard) => guard,
            Err(error) => {
                self.opened.store(false, Ordering::Release);
                return Err(error);
            }
        };
        self.format.store(0, Ordering::Release);
        self.sample_rate_hint.store(0, Ordering::Release);
        self.channels_hint.store(0, Ordering::Release);
        self.volume_percent.store(VOLUME_DEFAULT, Ordering::Release);
        self.reset_counters();
        self.invalidate_and_reset(STATE_IDLE);
        Ok(())
    }

    pub fn close(&self) -> Result<(), InputError> {
        let _guard = self.try_control()?;
        self.invalidate_and_reset(STATE_CLOSED);
        self.format.store(0, Ordering::Release);
        self.sample_rate_hint.store(0, Ordering::Release);
        self.channels_hint.store(0, Ordering::Release);
        self.opened.store(false, Ordering::Release);
        Ok(())
    }

    pub fn set_format(&self, format: &FormatV1) -> Result<(), InputError> {
        let _guard = self.try_control()?;
        if !self.opened.load(Ordering::Acquire)
            || format.struct_size as usize != core::mem::size_of::<FormatV1>()
            || format.format != FORMAT_MP3
            || (format.channels_hint != 0 && format.channels_hint != 1 && format.channels_hint != 2)
            || format.flags != 0
            || format.reserved != [0; 3]
        {
            return Err(InputError::Invalid);
        }
        let state = self.state.load(Ordering::Acquire);
        if state != STATE_IDLE && state != STATE_CONFIGURED && state != STATE_STOPPED {
            return Err(InputError::Busy);
        }
        self.reset_counters();
        self.invalidate_and_reset(STATE_CONFIGURED);
        self.format.store(format.format, Ordering::Release);
        self.sample_rate_hint
            .store(format.sample_rate_hint, Ordering::Release);
        self.channels_hint
            .store(format.channels_hint, Ordering::Release);
        Ok(())
    }

    pub fn start(&self) -> Result<(), InputError> {
        let _guard = self.try_control()?;
        let state = self.state.load(Ordering::Acquire);
        if self.format.load(Ordering::Acquire) != FORMAT_MP3
            || (state != STATE_CONFIGURED && state != STATE_STOPPED)
        {
            return Err(InputError::Invalid);
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.last_error.store(0, Ordering::Release);
        self.decoded_sample_rate.store(0, Ordering::Release);
        self.decoded_channels.store(0, Ordering::Release);
        self.negotiated_bitpool.store(0, Ordering::Release);
        self.state.store(STATE_BUFFERING, Ordering::Release);
        Ok(())
    }

    pub fn pause(&self) -> Result<(), InputError> {
        let _guard = self.try_control()?;
        let state = self.state.load(Ordering::Acquire);
        if state != STATE_BUFFERING && state != STATE_PLAYING {
            return Err(InputError::Invalid);
        }
        self.state.store(STATE_PAUSED, Ordering::Release);
        Ok(())
    }

    pub fn resume(&self) -> Result<(), InputError> {
        let _guard = self.try_control()?;
        if self.state.load(Ordering::Acquire) != STATE_PAUSED {
            return Err(InputError::Invalid);
        }
        self.state.store(STATE_BUFFERING, Ordering::Release);
        Ok(())
    }

    pub fn stop(&self) -> Result<(), InputError> {
        let _guard = self.try_control()?;
        if !self.opened.load(Ordering::Acquire) {
            return Err(InputError::Pipe);
        }
        self.invalidate_and_reset(STATE_STOPPED);
        Ok(())
    }

    pub fn flush(&self) -> Result<(), InputError> {
        let _guard = self.try_control()?;
        if self.format.load(Ordering::Acquire) == 0 {
            return Err(InputError::Pipe);
        }
        self.invalidate_and_reset(STATE_CONFIGURED);
        Ok(())
    }

    pub fn drain(&self) -> Result<(), InputError> {
        let _guard = self.try_control()?;
        let state = self.state.load(Ordering::Acquire);
        if state != STATE_BUFFERING && state != STATE_PLAYING && state != STATE_PAUSED {
            return Err(InputError::Invalid);
        }
        self.state.store(STATE_DRAINING, Ordering::Release);
        Ok(())
    }

    pub fn write(&self, input: &[u8]) -> Result<usize, InputError> {
        if !self.opened.load(Ordering::Acquire) || self.format.load(Ordering::Acquire) == 0 {
            return Err(InputError::Pipe);
        }
        let state = self.state.load(Ordering::Acquire);
        if state == STATE_DRAINING || state == STATE_CLOSED || state == STATE_IDLE {
            return Err(InputError::Pipe);
        }
        if state == STATE_ERROR {
            return Err(InputError::Io);
        }
        let count = self.ring.write(input);
        if count == 0 && !input.is_empty() {
            return Err(InputError::Again);
        }
        self.bytes_accepted
            .fetch_add(count as u32, Ordering::Relaxed);
        Ok(count)
    }

    pub fn state(&self) -> u32 {
        self.state.load(Ordering::Acquire)
    }

    pub fn generation(&self) -> u32 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn status(&self) -> StatusV1 {
        let used = self.ring.used() as u32;
        StatusV1 {
            struct_size: core::mem::size_of::<StatusV1>() as u32,
            abi_version: ABI_VERSION,
            state: self.state(),
            last_error: self.last_error.load(Ordering::Acquire),
            format: self.format.load(Ordering::Acquire),
            input_capacity: N as u32,
            input_used: used,
            input_free: N as u32 - used,
            decoded_sample_rate: self.decoded_sample_rate.load(Ordering::Acquire),
            decoded_channels: self.decoded_channels.load(Ordering::Acquire),
            negotiated_bitpool: self.negotiated_bitpool.load(Ordering::Acquire),
            bytes_accepted: self.bytes_accepted.load(Ordering::Acquire),
            bytes_consumed: self.bytes_consumed.load(Ordering::Acquire),
            pcm_frames: self.pcm_frames.load(Ordering::Acquire),
            rtp_packets: self.rtp_packets.load(Ordering::Acquire),
            underruns: self.underruns.load(Ordering::Acquire),
            generation: self.generation(),
            volume_percent: self.volume(),
        }
    }

    pub fn set_volume(&self, volume_percent: u32) -> Result<(), InputError> {
        if !self.opened.load(Ordering::Acquire) || volume_percent > VOLUME_MAX {
            return Err(InputError::Invalid);
        }
        self.volume_percent.store(volume_percent, Ordering::Release);
        Ok(())
    }

    pub fn volume(&self) -> u32 {
        self.volume_percent.load(Ordering::Acquire)
    }

    pub fn decoded_format_matches(&self, sample_rate: u32, channels: u32) -> bool {
        let rate_hint = self.sample_rate_hint.load(Ordering::Acquire);
        let channels_hint = self.channels_hint.load(Ordering::Acquire);
        (rate_hint == 0 || rate_hint == sample_rate)
            && (channels_hint == 0 || channels_hint == channels)
    }

    pub fn consume(&self, output: &mut [u8]) -> usize {
        let count = self.ring.read(output);
        self.bytes_consumed
            .fetch_add(count as u32, Ordering::Relaxed);
        count
    }

    pub fn mark_playing(
        &self,
        generation: u32,
        sample_rate: u32,
        channels: u32,
        bitpool: u32,
    ) -> bool {
        if self.generation() != generation {
            return false;
        }
        self.decoded_sample_rate
            .store(sample_rate, Ordering::Release);
        self.decoded_channels.store(channels, Ordering::Release);
        self.negotiated_bitpool.store(bitpool, Ordering::Release);
        if self.state.load(Ordering::Acquire) != STATE_DRAINING {
            self.state.store(STATE_PLAYING, Ordering::Release);
        }
        true
    }

    pub fn add_pcm_frames(&self, generation: u32, frames: u32) {
        if self.generation() == generation {
            self.pcm_frames.fetch_add(frames, Ordering::Relaxed);
        }
    }

    pub fn add_rtp_packets(&self, generation: u32, packets: u32) {
        if self.generation() == generation {
            self.rtp_packets.fetch_add(packets, Ordering::Relaxed);
        }
    }

    pub fn mark_underrun(&self, generation: u32) {
        if self.generation() == generation {
            self.underruns.fetch_add(1, Ordering::Relaxed);
            if self.state.load(Ordering::Acquire) != STATE_DRAINING {
                self.state.store(STATE_BUFFERING, Ordering::Release);
            }
        }
    }

    pub fn mark_drained(&self, generation: u32, pipeline_empty: bool) {
        if self.generation() == generation
            && pipeline_empty
            && self.state.load(Ordering::Acquire) == STATE_DRAINING
            && self.ring.used() == 0
        {
            self.state.store(STATE_STOPPED, Ordering::Release);
        }
    }

    pub fn fail(&self, generation: u32, error: i32) {
        if self.generation() == generation {
            self.last_error.store(error, Ordering::Release);
            self.state.store(STATE_ERROR, Ordering::Release);
        }
    }

    fn reset_counters(&self) {
        self.bytes_accepted.store(0, Ordering::Release);
        self.bytes_consumed.store(0, Ordering::Release);
        self.pcm_frames.store(0, Ordering::Release);
        self.rtp_packets.store(0, Ordering::Release);
        self.underruns.store(0, Ordering::Release);
    }

    fn invalidate_and_reset(&self, next_state: u32) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.ring.reset();
        self.last_error.store(0, Ordering::Release);
        self.decoded_sample_rate.store(0, Ordering::Release);
        self.decoded_channels.store(0, Ordering::Release);
        self.negotiated_bitpool.store(0, Ordering::Release);
        self.state.store(next_state, Ordering::Release);
    }
}

impl<const N: usize> Default for AudioInput<N> {
    fn default() -> Self {
        Self::new()
    }
}

struct ControlGuard<'a, const N: usize>(&'a AudioInput<N>);

impl<const N: usize> Drop for ControlGuard<'_, N> {
    fn drop(&mut self) {
        self.0.control_lock.store(false, Ordering::Release);
    }
}
