pub const SAMPLE_RATE: u32 = 44_100;
pub const FRAME_SAMPLES: u32 = 128;
pub const FRAME_LENGTH: usize = 118;
pub const RTP_HEADER: usize = 12;
pub const SBC_HEADER: usize = 1;
pub const MAX_FRAMES_PER_PACKET: u8 = 5;
pub const MAX_PACKET: usize =
    RTP_HEADER + SBC_HEADER + MAX_FRAMES_PER_PACKET as usize * FRAME_LENGTH;
pub const TONE_SECONDS: u32 = 5;

pub const TEST_TONE_FRAME: [u8; FRAME_LENGTH] = [
    0x9c, 0xb9, 0x35, 0xe2, 0xc2, 0, 0, 0, 0xc2, 0, 0, 0, 0xc7, 0x02, 0xd4, 0x8c, 0x70, 0x1a, 0x97,
    0x53, 0x82, 0xb2, 0xf5, 0x37, 0x56, 0x4e, 0x3f, 0x0a, 0x88, 0xe3, 0xf1, 0x50, 0x2b, 0x1d, 0x50,
    0xa2, 0xb1, 0xea, 0x1c, 0x70, 0x2d, 0x48, 0xc7, 0x01, 0xa9, 0x75, 0x38, 0x2b, 0x2f, 0x53, 0x75,
    0x64, 0xe3, 0xf0, 0xa8, 0x8e, 0x3f, 0x15, 0x02, 0xb1, 0xd5, 0x0a, 0x2b, 0x1e, 0xa1, 0xc7, 0x02,
    0xd4, 0x8c, 0x70, 0x1a, 0x97, 0x53, 0x82, 0xb2, 0xf5, 0x37, 0x56, 0x4e, 0x3f, 0x0a, 0x88, 0xe3,
    0xf1, 0x50, 0x2b, 0x1d, 0x50, 0xa2, 0xb1, 0xea, 0x1c, 0x70, 0x2d, 0x48, 0xc7, 0x01, 0xa9, 0x75,
    0x38, 0x2b, 0x2f, 0x53, 0x75, 0x64, 0xe3, 0xf0, 0xa8, 0x8e, 0x3f, 0x15, 0x02, 0xb1, 0xd5, 0x0a,
    0x2b, 0x1e, 0xa1,
];

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MediaError {
    Mtu,
    Complete,
    Buffer,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TonePacketizer {
    pub sequence: u16,
    pub timestamp: u32,
    pub packets_sent: u32,
    pub frames_sent: u32,
    pub packets_target: u32,
    pub frames_per_packet: u8,
    pace_remainder: u32,
}

impl TonePacketizer {
    pub fn new(mtu: u16) -> Result<Self, MediaError> {
        let limit = usize::from(mtu).min(672);
        let mut frames = MAX_FRAMES_PER_PACKET;
        while frames > 1 && RTP_HEADER + SBC_HEADER + frames as usize * FRAME_LENGTH > limit {
            frames -= 1;
        }
        if RTP_HEADER + SBC_HEADER + FRAME_LENGTH > limit {
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
        let frames = self.frames_per_packet as usize;
        let length = RTP_HEADER + SBC_HEADER + frames * FRAME_LENGTH;
        if out.len() < length {
            return Err(MediaError::Buffer);
        }
        out[..length].fill(0);
        out[0] = 0x80;
        out[1] = 96 | if self.packets_sent == 0 { 0x80 } else { 0 };
        out[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        out[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        out[8..12].copy_from_slice(&0x4341_4e50u32.to_be_bytes());
        out[12] = self.frames_per_packet;
        for chunk in out[13..length].chunks_exact_mut(FRAME_LENGTH) {
            chunk.copy_from_slice(&TEST_TONE_FRAME);
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
}
