use crate::sbc_tone_frames;

pub const SAMPLE_RATE: u32 = 44_100;
pub const FRAME_SAMPLES: u32 = 128;
pub const MAX_FRAME_LENGTH: usize = 118;
pub const RTP_HEADER: usize = 12;
pub const SBC_HEADER: usize = 1;
pub const MAX_FRAMES_PER_PACKET: u8 = 5;
/// Proven safe transmit SDU ceiling for the exact-target stock L2CAP path.
/// The peer may advertise a larger MTU, but the recovered production sender
/// never submits more than 672 bytes on this firmware.
pub const MAX_TX_SDU: usize = 672;
const MAX_TONE_FRAMES_PER_PACKET: u8 = 5;
pub const DEFAULT_SINK_DELAY_100US: u16 = 1_500;
pub const MAX_STARTUP_PACKETS: u8 = 16;
pub const MAX_PACKET: usize =
    RTP_HEADER + SBC_HEADER + MAX_FRAMES_PER_PACKET as usize * MAX_FRAME_LENGTH;
pub const TONE_SECONDS: u32 = 5;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MediaError {
    Mtu,
    Complete,
    Buffer,
    Codec,
}

pub struct StreamPacketizer {
    pub sequence: u16,
    pub timestamp: u32,
    pub packets_sent: u32,
    pub frames_sent: u32,
    pub frames_per_packet: u8,
    pub bitpool: u8,
    pub frame_length: u16,
    pace_remainder: u32,
}

impl StreamPacketizer {
    pub fn new(mtu: u16, bitpool: u8) -> Result<Self, MediaError> {
        let frame_length = sbc_tone_frames::frame_length(bitpool).ok_or(MediaError::Codec)?;
        let limit = usize::from(mtu).min(MAX_TX_SDU);
        let mut frames = MAX_FRAMES_PER_PACKET;
        while frames > 1 && RTP_HEADER + SBC_HEADER + frames as usize * frame_length > limit {
            frames -= 1;
        }
        if RTP_HEADER + SBC_HEADER + frame_length > limit {
            return Err(MediaError::Mtu);
        }
        Ok(Self {
            sequence: 1,
            timestamp: 0,
            packets_sent: 0,
            frames_sent: 0,
            frames_per_packet: frames,
            bitpool,
            frame_length: frame_length as u16,
            pace_remainder: 0,
        })
    }

    pub fn packet_length(&self, frames: u8) -> Result<usize, MediaError> {
        if frames == 0 || frames > self.frames_per_packet {
            return Err(MediaError::Buffer);
        }
        Ok(RTP_HEADER + SBC_HEADER + frames as usize * usize::from(self.frame_length))
    }

    /// Writes the RTP/SBC payload header and advances transport counters. The
    /// caller writes exactly `frames` encoded SBC frames after the returned
    /// offset; an encoder failure is terminal for that playback generation.
    pub fn write_header(
        &mut self,
        out: &mut [u8],
        frames: u8,
        marker: bool,
    ) -> Result<usize, MediaError> {
        let length = self.packet_length(frames)?;
        if out.len() < length {
            return Err(MediaError::Buffer);
        }
        out[..RTP_HEADER + SBC_HEADER].fill(0);
        out[0] = 0x80;
        out[1] = 96 | if marker { 0x80 } else { 0 };
        out[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        out[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        out[8..12].copy_from_slice(&0x4254_5036u32.to_be_bytes());
        out[12] = frames;
        self.packets_sent = self.packets_sent.wrapping_add(1);
        self.frames_sent = self.frames_sent.wrapping_add(frames as u32);
        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(frames as u32 * FRAME_SAMPLES);
        Ok(RTP_HEADER + SBC_HEADER)
    }

    pub fn next_delay_ms(&mut self, frames: u8) -> u32 {
        let timing = self.pace_remainder + frames as u32 * FRAME_SAMPLES * 1000;
        let delay = (timing / SAMPLE_RATE).max(1);
        self.pace_remainder = timing % SAMPLE_RATE;
        delay
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TonePacketizer {
    pub sequence: u16,
    pub timestamp: u32,
    pub packets_sent: u32,
    pub frames_sent: u32,
    pub packets_target: u32,
    pub frames_per_packet: u8,
    pub bitpool: u8,
    pub frame_length: u16,
    pace_remainder: u32,
}

impl TonePacketizer {
    pub fn new(mtu: u16, bitpool: u8) -> Result<Self, MediaError> {
        let frame_length = sbc_tone_frames::frame_length(bitpool).ok_or(MediaError::Codec)?;
        let limit = usize::from(mtu).min(MAX_TX_SDU);
        let mut frames = MAX_TONE_FRAMES_PER_PACKET;
        while frames > 1 && RTP_HEADER + SBC_HEADER + frames as usize * frame_length > limit {
            frames -= 1;
        }
        if RTP_HEADER + SBC_HEADER + frame_length > limit {
            return Err(MediaError::Mtu);
        }
        let packet_samples = frames as u32 * FRAME_SAMPLES;
        let packets_target = (TONE_SECONDS * SAMPLE_RATE).div_ceil(packet_samples);
        Ok(Self {
            sequence: 1,
            timestamp: 0,
            packets_sent: 0,
            frames_sent: 0,
            packets_target,
            frames_per_packet: frames,
            bitpool,
            frame_length: frame_length as u16,
            pace_remainder: 0,
        })
    }

    pub fn is_complete(&self) -> bool {
        self.packets_sent >= self.packets_target
    }

    pub fn write_packet(&mut self, out: &mut [u8]) -> Result<usize, MediaError> {
        if self.is_complete() {
            return Err(MediaError::Complete);
        }
        let frame_length = usize::from(self.frame_length);
        let frames = self.frames_per_packet as usize;
        let length = RTP_HEADER + SBC_HEADER + frames * frame_length;
        if out.len() < length {
            return Err(MediaError::Buffer);
        }
        out[..length].fill(0);
        out[0] = 0x80;
        out[1] = 96 | if self.packets_sent == 0 { 0x80 } else { 0 };
        out[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        out[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        out[8..12].copy_from_slice(&0x4254_5036u32.to_be_bytes());
        out[12] = self.frames_per_packet;
        for chunk in out[13..length].chunks_exact_mut(frame_length) {
            if sbc_tone_frames::write_frame(self.bitpool, chunk) != Some(frame_length) {
                return Err(MediaError::Codec);
            }
        }
        self.packets_sent += 1;
        self.frames_sent += self.frames_per_packet as u32;
        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self
            .timestamp
            .wrapping_add(self.frames_per_packet as u32 * FRAME_SAMPLES);
        Ok(length)
    }

    pub fn next_delay_ms(&mut self) -> u32 {
        let timing = self.pace_remainder + self.frames_per_packet as u32 * FRAME_SAMPLES * 1000;
        let delay = (timing / SAMPLE_RATE).max(1);
        self.pace_remainder = timing % SAMPLE_RATE;
        delay
    }

    /// Number of packets to queue at START so a TWS sink can establish its
    /// presentation buffer before either ear begins rendering. A sink that did
    /// not send Delay Report uses the 150 ms value observed from the reference
    /// peer; the cap prevents a malformed report from creating a large burst.
    pub fn startup_packets(&self, reported_delay_100us: u16) -> u8 {
        let delay = if reported_delay_100us == 0 {
            DEFAULT_SINK_DELAY_100US
        } else {
            reported_delay_100us
        } as u32;
        let packet_samples = self.frames_per_packet as u32 * FRAME_SAMPLES;
        let packet_100us = (packet_samples * 10_000).div_ceil(SAMPLE_RATE).max(1);
        delay
            .div_ceil(packet_100us)
            .clamp(1, MAX_STARTUP_PACKETS as u32) as u8
    }

    /// Pauses after the START burst until all but one packet interval has
    /// elapsed. This preserves one packet of headroom without shortening the
    /// five-second presentation or sending SUSPEND ahead of queued audio.
    pub fn startup_catchup_delay_ms(&mut self, packets: u8) -> u32 {
        if packets <= 1 {
            return self.next_delay_ms();
        }
        let mut delay = 0;
        for _ in 1..packets {
            delay += self.next_delay_ms();
        }
        delay
    }

    /// Waits for the sink's reported presentation delay plus the duration of
    /// the final RTP packet before declaring the tone complete, so queued audio
    /// is not replaced while either TWS ear is still rendering it.
    pub fn presentation_drain_delay_ms(&mut self, reported_delay_100us: u16) -> u32 {
        let delay = if reported_delay_100us == 0 {
            DEFAULT_SINK_DELAY_100US
        } else {
            reported_delay_100us
        } as u32;
        delay.min(5_000).div_ceil(10) + self.next_delay_ms()
    }
}
