use nanomp3::{Decoder, FrameInfo, MAX_SAMPLES_PER_FRAME};

pub const REQUIRED_INPUT_WINDOW: usize = 16 * 1024;
pub const MIN_DECODE_INPUT: usize = 2 * 1024;
pub const REQUIRED_SAMPLE_RATE: u32 = 44_100;
pub const RESAMPLED_SAMPLE_RATE_24K: u32 = 24_000;
pub const RESAMPLED_SAMPLE_RATE_48K: u32 = 48_000;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StreamProgress {
    NeedInput,
    Skipped {
        consumed: usize,
    },
    Frame {
        consumed: usize,
        frames: usize,
        samples: usize,
        sample_rate: u32,
        channels: u8,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mp3Error {
    NeedInput,
    UnsupportedRate,
    UnsupportedChannels,
    Output,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DecodedFrame {
    pub consumed: usize,
    pub sample_rate: u32,
    pub channels: u8,
    /// Number of sample frames (one sample per channel) in this MP3 frame.
    pub frames: usize,
}

/// Allocation-free nanomp3 wrapper with resident decode output. `nanomp3`
/// exposes f32 PCM, which is converted to interleaved stereo S16 for SBC.
pub struct Mp3Decoder {
    decoder: Decoder,
    pcm: [f32; MAX_SAMPLES_PER_FRAME],
}

impl Mp3Decoder {
    pub const fn new() -> Self {
        Self {
            decoder: Decoder::new(),
            pcm: [0.0; MAX_SAMPLES_PER_FRAME],
        }
    }

    /// Initializes heap-backed decoder storage without a large stack temporary.
    ///
    /// # Safety
    ///
    /// `destination` must identify aligned, writable, uninitialized storage for
    /// one `Mp3Decoder` and must not currently hold a live value.
    pub unsafe fn initialize_at(destination: *mut Self) {
        let decoder = unsafe { core::ptr::addr_of_mut!((*destination).decoder) };
        unsafe { Decoder::initialize_at(decoder) };
        let pcm = unsafe { core::ptr::addr_of_mut!((*destination).pcm) };
        unsafe { pcm.write_bytes(0, 1) };
    }

    pub fn reset(&mut self) {
        self.decoder.reset();
        // The decoder overwrites every sample reported through `frames`; stale
        // PCM beyond that range is never observed by conversion.
    }

    pub fn decode(&mut self, input: &[u8]) -> Result<DecodedFrame, Mp3Error> {
        let (consumed, info) = self.decoder.decode(input, &mut self.pcm);
        let Some(FrameInfo {
            samples_produced,
            channels,
            sample_rate,
            ..
        }) = info
        else {
            return if consumed == 0 {
                Err(Mp3Error::NeedInput)
            } else {
                Ok(DecodedFrame {
                    consumed,
                    sample_rate: 0,
                    channels: 0,
                    frames: 0,
                })
            };
        };
        if sample_rate != REQUIRED_SAMPLE_RATE
            && sample_rate != RESAMPLED_SAMPLE_RATE_24K
            && sample_rate != RESAMPLED_SAMPLE_RATE_48K
        {
            return Err(Mp3Error::UnsupportedRate);
        }
        let channels = channels.num();
        if channels != 1 && channels != 2 {
            return Err(Mp3Error::UnsupportedChannels);
        }
        let sample_count = samples_produced
            .checked_mul(channels as usize)
            .ok_or(Mp3Error::Output)?;
        if sample_count > self.pcm.len() {
            return Err(Mp3Error::Output);
        }
        Ok(DecodedFrame {
            consumed,
            sample_rate,
            channels,
            frames: samples_produced,
        })
    }

    pub fn write_stereo_s16(
        &self,
        frame: DecodedFrame,
        volume_percent: u32,
        output: &mut [i16],
    ) -> Result<usize, Mp3Error> {
        let needed = frame.frames.checked_mul(2).ok_or(Mp3Error::Output)?;
        if output.len() < needed
            || volume_percent > 100
            || (frame.channels != 1 && frame.channels != 2)
        {
            return Err(Mp3Error::Output);
        }
        if frame.channels == 1 {
            for (sample, pair) in self.pcm[..frame.frames]
                .iter()
                .copied()
                .zip(output[..needed].chunks_exact_mut(2))
            {
                let sample = apply_volume(float_to_s16(sample), volume_percent);
                pair[0] = sample;
                pair[1] = sample;
            }
        } else {
            for (sample, slot) in self.pcm[..needed]
                .iter()
                .copied()
                .zip(output[..needed].iter_mut())
            {
                *slot = apply_volume(float_to_s16(sample), volume_percent);
            }
        }
        Ok(needed)
    }

    fn write_stereo_s16_resampled(
        &self,
        frame: DecodedFrame,
        volume_percent: u32,
        output: &mut [i16],
        phase: &mut u32,
    ) -> Result<usize, Mp3Error> {
        if frame.sample_rate == REQUIRED_SAMPLE_RATE {
            *phase = 0;
            return self.write_stereo_s16(frame, volume_percent, output);
        }
        if (frame.sample_rate != RESAMPLED_SAMPLE_RATE_24K
            && frame.sample_rate != RESAMPLED_SAMPLE_RATE_48K)
            || volume_percent > 100
            || (frame.channels != 1 && frame.channels != 2)
        {
            return Err(Mp3Error::UnsupportedRate);
        }
        let span = (frame.frames as u32)
            .checked_mul(REQUIRED_SAMPLE_RATE)
            .ok_or(Mp3Error::Output)?;
        if *phase >= span {
            return Err(Mp3Error::Output);
        }
        let output_frames = (span - *phase).div_ceil(frame.sample_rate) as usize;
        let needed = output_frames.checked_mul(2).ok_or(Mp3Error::Output)?;
        if output.len() < needed {
            return Err(Mp3Error::Output);
        }

        let mut index = (*phase / REQUIRED_SAMPLE_RATE) as usize;
        let mut fraction = *phase % REQUIRED_SAMPLE_RATE;
        for pair in output[..needed].chunks_exact_mut(2) {
            let next = (index + 1).min(frame.frames - 1);
            let blend = fraction as f32 * (1.0 / REQUIRED_SAMPLE_RATE as f32);
            for (channel, slot) in pair.iter_mut().enumerate() {
                let source_channel = if frame.channels == 1 { 0 } else { channel };
                let first = self.pcm[index * frame.channels as usize + source_channel];
                let second = self.pcm[next * frame.channels as usize + source_channel];
                let sample = first + (second - first) * blend;
                *slot = apply_volume(float_to_s16(sample), volume_percent);
            }
            fraction += frame.sample_rate;
            while fraction >= REQUIRED_SAMPLE_RATE {
                fraction -= REQUIRED_SAMPLE_RATE;
                index += 1;
            }
        }
        *phase = index as u32 * REQUIRED_SAMPLE_RATE + fraction - span;
        Ok(needed)
    }
}

impl Default for Mp3Decoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Incremental MP3 frame reader for a byte stream split across arbitrary
/// writes. Compressed bytes stay resident until nanomp3 reports how much input
/// it consumed, so a frame crossing ring-buffer writes is never discarded.
pub struct Mp3ByteStream {
    decoder: Mp3Decoder,
    input: [u8; REQUIRED_INPUT_WINDOW],
    input_len: usize,
    resample_rate: u32,
    resample_phase: u32,
}

impl Mp3ByteStream {
    pub const fn new() -> Self {
        Self {
            decoder: Mp3Decoder::new(),
            input: [0; REQUIRED_INPUT_WINDOW],
            input_len: 0,
            resample_rate: 0,
            resample_phase: 0,
        }
    }

    /// Initializes a heap-backed stream workspace without materializing the
    /// decoder and 16 KiB input window on the caller's stack.
    ///
    /// # Safety
    ///
    /// `destination` must identify aligned, writable, uninitialized storage for
    /// one `Mp3ByteStream` and must not currently hold a live value.
    pub unsafe fn initialize_at(destination: *mut Self) {
        let decoder = unsafe { core::ptr::addr_of_mut!((*destination).decoder) };
        unsafe { Mp3Decoder::initialize_at(decoder) };
        let input = unsafe { core::ptr::addr_of_mut!((*destination).input) };
        unsafe { input.write_bytes(0, 1) };
        let input_len = unsafe { core::ptr::addr_of_mut!((*destination).input_len) };
        unsafe { input_len.write(0) };
        let resample_rate = unsafe { core::ptr::addr_of_mut!((*destination).resample_rate) };
        unsafe { resample_rate.write(0) };
        let resample_phase = unsafe { core::ptr::addr_of_mut!((*destination).resample_phase) };
        unsafe { resample_phase.write(0) };
    }

    pub fn reset(&mut self) {
        self.decoder.reset();
        self.input_len = 0;
        self.resample_rate = 0;
        self.resample_phase = 0;
    }

    pub fn pending(&self) -> usize {
        self.input_len
    }

    pub fn free(&self) -> usize {
        self.input.len() - self.input_len
    }

    /// Appends as much compressed input as fits in the resident decode window.
    pub fn push(&mut self, input: &[u8]) -> usize {
        let count = input.len().min(self.free());
        self.input[self.input_len..self.input_len + count].copy_from_slice(&input[..count]);
        self.input_len += count;
        count
    }

    /// Attempts one bounded decode step. A successful frame is converted to
    /// interleaved stereo S16 in `output`; metadata-only input such as ID3 or a
    /// resynchronization prefix is reported as `Skipped`. `end_of_input` allows
    /// the decoder to inspect the final short window after DRAIN.
    pub fn decode_next(
        &mut self,
        volume_percent: u32,
        output: &mut [i16],
        end_of_input: bool,
    ) -> Result<StreamProgress, Mp3Error> {
        if self.input_len < MIN_DECODE_INPUT && !end_of_input {
            return Ok(StreamProgress::NeedInput);
        }
        let frame = match self.decoder.decode(&self.input[..self.input_len]) {
            Ok(frame) => frame,
            Err(Mp3Error::NeedInput) => return Ok(StreamProgress::NeedInput),
            Err(error) => return Err(error),
        };
        if frame.consumed == 0 || frame.consumed > self.input_len {
            return Err(Mp3Error::Output);
        }
        let progress = if frame.frames == 0 {
            StreamProgress::Skipped {
                consumed: frame.consumed,
            }
        } else {
            if self.resample_rate != frame.sample_rate {
                self.resample_rate = frame.sample_rate;
                self.resample_phase = 0;
            }
            let samples = self.decoder.write_stereo_s16_resampled(
                frame,
                volume_percent,
                output,
                &mut self.resample_phase,
            )?;
            StreamProgress::Frame {
                consumed: frame.consumed,
                frames: samples / 2,
                samples,
                sample_rate: frame.sample_rate,
                channels: frame.channels,
            }
        };
        self.input.copy_within(frame.consumed..self.input_len, 0);
        self.input_len -= frame.consumed;
        Ok(progress)
    }

    /// Drops an incomplete trailing frame after the producer has requested
    /// DRAIN and supplied all bytes. Ordinary underruns must retain it instead.
    pub fn discard_incomplete(&mut self) -> usize {
        let discarded = self.input_len;
        self.input_len = 0;
        discarded
    }
}

impl Default for Mp3ByteStream {
    fn default() -> Self {
        Self::new()
    }
}

fn apply_volume(sample: i16, volume_percent: u32) -> i16 {
    (i32::from(sample) * volume_percent as i32 / 100) as i16
}

fn float_to_s16(sample: f32) -> i16 {
    let scaled = sample.clamp(-1.0, 1.0) * 32_767.0;
    if scaled >= 0.0 {
        (scaled + 0.5) as i16
    } else {
        (scaled - 0.5) as i16
    }
}
