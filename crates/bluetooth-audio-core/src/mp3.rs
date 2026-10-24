use nanomp3::{Decoder, FrameInfo, MAX_SAMPLES_PER_FRAME};

pub const REQUIRED_INPUT_WINDOW: usize = 16 * 1024;
pub const MIN_DECODE_INPUT: usize = 2 * 1024;
pub const REQUIRED_SAMPLE_RATE: u32 = 44_100;
pub const RESAMPLED_SAMPLE_RATE_24K: u32 = 24_000;
pub const RESAMPLED_SAMPLE_RATE_48K: u32 = 48_000;
/// Consumed input is reclaimed only after it reaches half of the window. A
/// refill then shifts at most half a window, instead of moving the whole
/// window after every decoded frame.
const COMPACT_THRESHOLD: usize = REQUIRED_INPUT_WINDOW / 2;
const ID3V2_HEADER: usize = 10;
/// Maps a resampler phase remainder in `0..REQUIRED_SAMPLE_RATE` to a Q15
/// interpolation weight after a 16-bit shift.
const BLEND_Q15_SCALE: u32 = ((1u64 << 31) / REQUIRED_SAMPLE_RATE as u64) as u32;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StreamProgress {
    NeedInput,
    Skipped {
        consumed: usize,
    },
    Frame {
        consumed: usize,
        /// Stereo frames written to the output, at [`REQUIRED_SAMPLE_RATE`].
        frames: usize,
        samples: usize,
        /// The MP3's own rate; differs from the output's when the frame was resampled.
        sample_rate: u32,
        channels: u8,
        /// Frames the MP3 frame held at `sample_rate`: media time is
        /// `source_frames / sample_rate`, whatever the resampler made of it.
        source_frames: usize,
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
/// emits S16 PCM, which is expanded to interleaved stereo 44.1 kHz for SBC.
pub struct Mp3Decoder {
    decoder: Decoder,
    pcm: [i16; MAX_SAMPLES_PER_FRAME],
}

impl Mp3Decoder {
    pub const fn new() -> Self {
        Self {
            decoder: Decoder::new(),
            pcm: [0; MAX_SAMPLES_PER_FRAME],
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
        let output = &mut output[..needed];
        if frame.channels == 1 {
            for (sample, pair) in self.pcm[..frame.frames]
                .iter()
                .zip(output.chunks_exact_mut(2))
            {
                pair[0] = *sample;
                pair[1] = *sample;
            }
        } else {
            output.copy_from_slice(&self.pcm[..needed]);
        }
        scale_volume(output, volume_percent);
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
        let output = &mut output[..needed];

        let source = &self.pcm[..frame.frames * frame.channels as usize];
        let mut index = (*phase / REQUIRED_SAMPLE_RATE) as usize;
        let mut fraction = *phase % REQUIRED_SAMPLE_RATE;
        // SAFETY: output frame `k` starts at `phase + k * sample_rate`, which
        // is below `span` for every `k < output_frames`, so each source index
        // is below `frame.frames`; `source` holds that many frames.
        unsafe {
            if frame.channels == 1 {
                resample::<1>(source, frame.sample_rate, &mut index, &mut fraction, output);
            } else {
                resample::<2>(source, frame.sample_rate, &mut index, &mut fraction, output);
            }
        }
        scale_volume(output, volume_percent);
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
    /// Pending input is `input[input_start..input_end]`.
    input_start: usize,
    input_end: usize,
    /// Bytes of an ID3v2 tag that have not been buffered yet.
    tag_remaining: usize,
    resample_rate: u32,
    resample_phase: u32,
}

impl Mp3ByteStream {
    pub const fn new() -> Self {
        Self {
            decoder: Mp3Decoder::new(),
            input: [0; REQUIRED_INPUT_WINDOW],
            input_start: 0,
            input_end: 0,
            tag_remaining: 0,
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
        let input_start = unsafe { core::ptr::addr_of_mut!((*destination).input_start) };
        unsafe { input_start.write(0) };
        let input_end = unsafe { core::ptr::addr_of_mut!((*destination).input_end) };
        unsafe { input_end.write(0) };
        let tag_remaining = unsafe { core::ptr::addr_of_mut!((*destination).tag_remaining) };
        unsafe { tag_remaining.write(0) };
        let resample_rate = unsafe { core::ptr::addr_of_mut!((*destination).resample_rate) };
        unsafe { resample_rate.write(0) };
        let resample_phase = unsafe { core::ptr::addr_of_mut!((*destination).resample_phase) };
        unsafe { resample_phase.write(0) };
    }

    pub fn reset(&mut self) {
        self.decoder.reset();
        self.input_start = 0;
        self.input_end = 0;
        self.tag_remaining = 0;
        self.resample_rate = 0;
        self.resample_phase = 0;
    }

    pub fn pending(&self) -> usize {
        self.input_end - self.input_start
    }

    /// Bytes `push` accepts now. Space before `input_start` is offered only
    /// once it reaches `COMPACT_THRESHOLD`, so a full window keeps decoding
    /// from its tail instead of being shifted after every frame.
    pub fn free(&self) -> usize {
        if self.input_start >= COMPACT_THRESHOLD {
            self.input.len() - self.pending()
        } else {
            self.input.len() - self.input_end
        }
    }

    /// Appends as much compressed input as fits in the resident decode window.
    pub fn push(&mut self, input: &[u8]) -> usize {
        let count = input.len().min(self.free());
        if count > self.input.len() - self.input_end {
            self.compact();
        }
        self.input[self.input_end..self.input_end + count].copy_from_slice(&input[..count]);
        self.input_end += count;
        count
    }

    fn compact(&mut self) {
        if self.input_start != 0 {
            self.input.copy_within(self.input_start..self.input_end, 0);
            self.input_end -= self.input_start;
            self.input_start = 0;
        }
    }

    fn consume(&mut self, count: usize) {
        self.input_start += count;
        if self.input_start == self.input_end {
            self.input_start = 0;
            self.input_end = 0;
        }
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
        let pending = self.pending();
        if self.tag_remaining != 0 && pending != 0 {
            let consumed = pending.min(self.tag_remaining);
            self.tag_remaining -= consumed;
            self.consume(consumed);
            return Ok(StreamProgress::Skipped { consumed });
        }
        if pending < MIN_DECODE_INPUT && !end_of_input {
            // Offer the whole window to the next refill.
            self.compact();
            return Ok(StreamProgress::NeedInput);
        }
        let window = &self.input[self.input_start..self.input_end];
        if let Some(tag) = id3v2_length(window) {
            // nanomp3 has no ID3 parser and would otherwise scan embedded
            // artwork byte by byte for a frame sync.
            let consumed = tag.min(pending);
            self.tag_remaining = tag - consumed;
            self.consume(consumed);
            return Ok(StreamProgress::Skipped { consumed });
        }
        let frame = match self.decoder.decode(window) {
            Ok(frame) => frame,
            Err(Mp3Error::NeedInput) => {
                self.compact();
                return Ok(StreamProgress::NeedInput);
            }
            Err(error) => return Err(error),
        };
        if frame.consumed == 0 || frame.consumed > pending {
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
                source_frames: frame.frames,
            }
        };
        self.consume(frame.consumed);
        Ok(progress)
    }

    /// Drops an incomplete trailing frame after the producer has requested
    /// DRAIN and supplied all bytes. Ordinary underruns must retain it instead.
    pub fn discard_incomplete(&mut self) -> usize {
        let discarded = self.pending();
        self.input_start = 0;
        self.input_end = 0;
        self.tag_remaining = 0;
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

fn scale_volume(samples: &mut [i16], volume_percent: u32) {
    if volume_percent != 100 {
        for sample in samples {
            *sample = apply_volume(*sample, volume_percent);
        }
    }
}

/// Linearly resamples interleaved `CHANNELS`-channel `source` frames to
/// stereo 44.1 kHz `output`, starting at source frame `index` plus
/// `fraction / REQUIRED_SAMPLE_RATE`, and advances that position past the
/// last output frame. Mono input is written to both output channels.
///
/// # Safety
///
/// `source.len()` must be a nonzero multiple of `CHANNELS`, and every output
/// frame's start position must lie before the end of `source`: for
/// `k < output.len() / 2`, `index + (fraction + k * sample_rate) /
/// REQUIRED_SAMPLE_RATE` must be below `source.len() / CHANNELS`.
unsafe fn resample<const CHANNELS: usize>(
    source: &[i16],
    sample_rate: u32,
    index: &mut usize,
    fraction: &mut u32,
    output: &mut [i16],
) {
    let last = source.len() / CHANNELS - 1;
    let step_whole = (sample_rate / REQUIRED_SAMPLE_RATE) as usize;
    let step_fraction = sample_rate % REQUIRED_SAMPLE_RATE;
    let mut position = *index;
    let mut remainder = *fraction;
    for pair in output.chunks_exact_mut(2) {
        debug_assert!(position <= last);
        let first = position * CHANNELS;
        let second = if position < last {
            first + CHANNELS
        } else {
            first
        };
        let blend = blend_q15(remainder);
        // SAFETY: the caller guarantees `position <= last`, so both frames lie
        // inside `source`.
        unsafe {
            let left = interpolate(
                *source.get_unchecked(first),
                *source.get_unchecked(second),
                blend,
            );
            pair[0] = left;
            pair[1] = if CHANNELS == 1 {
                left
            } else {
                interpolate(
                    *source.get_unchecked(first + 1),
                    *source.get_unchecked(second + 1),
                    blend,
                )
            };
        }
        position += step_whole;
        remainder += step_fraction;
        if remainder >= REQUIRED_SAMPLE_RATE {
            remainder -= REQUIRED_SAMPLE_RATE;
            position += 1;
        }
    }
    *index = position;
    *fraction = remainder;
}

#[inline(always)]
fn blend_q15(fraction: u32) -> i32 {
    ((fraction * BLEND_Q15_SCALE) >> 16) as i32
}

/// Rounded Q15 linear interpolation. `(second - first) * blend` stays below
/// 65_535 * 32_767, so the rounded product fits in i32, and the result lies
/// between the two samples.
#[inline(always)]
fn interpolate(first: i16, second: i16, blend: i32) -> i16 {
    let first = i32::from(first);
    let delta = i32::from(second) - first;
    (first + ((delta * blend + (1 << 14)) >> 15)) as i16
}

/// Complete length of an ID3v2 tag at the start of `bytes`, if one is there.
fn id3v2_length(bytes: &[u8]) -> Option<usize> {
    let header = bytes.get(..ID3V2_HEADER)?;
    if &header[..3] != b"ID3"
        || header[3] == 0xff
        || header[4] == 0xff
        || header[6..].iter().any(|byte| byte & 0x80 != 0)
    {
        return None;
    }
    let size = header[6..]
        .iter()
        .fold(0usize, |size, byte| size << 7 | usize::from(*byte));
    let footer = if header[5] & 0x10 != 0 {
        ID3V2_HEADER
    } else {
        0
    };
    Some(ID3V2_HEADER + size + footer)
}
