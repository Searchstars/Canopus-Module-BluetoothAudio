//! Allocation-free AVDTP Source signaling state machine.

const MSG_COMMAND: u8 = 0;
const MSG_GENERAL_REJECT: u8 = 1;
const MSG_ACCEPT: u8 = 2;
const MSG_REJECT: u8 = 3;
const SIGNAL_DISCOVER: u8 = 0x01;
const SIGNAL_GET_CAPABILITIES: u8 = 0x02;
const SIGNAL_SET_CONFIGURATION: u8 = 0x03;
const SIGNAL_GET_CONFIGURATION: u8 = 0x04;
const SIGNAL_RECONFIGURE: u8 = 0x05;
const SIGNAL_OPEN: u8 = 0x06;
const SIGNAL_START: u8 = 0x07;
const SIGNAL_CLOSE: u8 = 0x08;
const SIGNAL_SUSPEND: u8 = 0x09;
const SIGNAL_ABORT: u8 = 0x0a;
const SIGNAL_SECURITY_CONTROL: u8 = 0x0b;
const SIGNAL_GET_ALL_CAPABILITIES: u8 = 0x0c;
const SIGNAL_DELAY_REPORT: u8 = 0x0d;
const CATEGORY_MEDIA_TRANSPORT: u8 = 0x01;
const CATEGORY_MEDIA_CODEC: u8 = 0x07;
const CATEGORY_DELAY_REPORTING: u8 = 0x08;
const ERROR_BAD_ACP_SEID: u8 = 0x12;
const ERROR_BAD_PAYLOAD_FORMAT: u8 = 0x18;
const ERROR_BAD_STATE: u8 = 0x31;
const MAX_SDU: usize = 128;
const FRAGMENT_CAPACITY: usize = MAX_SDU - 2;
const PACKET_SINGLE: u8 = 0;
const PACKET_START: u8 = 1;
const PACKET_CONTINUE: u8 = 2;
const PACKET_END: u8 = 3;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum LinkState {
    #[default]
    Down,
    Connected,
}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Discovering,
    ReadingCapabilities,
    Configuring,
    Opening,
    Open,
    Starting,
    Streaming,
    Suspending,
    Failed,
}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SbcConfig {
    pub frequency_channel: u8,
    pub blocks_subbands_allocation: u8,
    pub minimum_bitpool: u8,
    pub maximum_bitpool: u8,
}
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Error {
    State,
    Packet,
    Overflow,
    Unsupported,
    Rejected(u8),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub generation: u32,
    pub link: LinkState,
    pub state: State,
    pub local_seid: u8,
    pub remote_seid: u8,
    pub selected_sbc: SbcConfig,
    pub delay_reporting: bool,
    pub local_in_use: bool,
    pub media_connected: bool,
    next_transaction: u8,
    pending_transaction: u8,
    pending_signal: u8,
    fragment_active: bool,
    fragment_length: u8,
    fragment_transaction: u8,
    fragment_message: u8,
    fragment_signal: u8,
    fragment_expected_packets: u8,
    fragment_received_packets: u8,
    fragment_payload: [u8; FRAGMENT_CAPACITY],
}

impl Source {
    pub const fn new(generation: u32) -> Self {
        Self {
            generation,
            link: LinkState::Down,
            state: State::Idle,
            local_seid: 1,
            remote_seid: 0,
            selected_sbc: SbcConfig {
                frequency_channel: 0,
                blocks_subbands_allocation: 0,
                minimum_bitpool: 0,
                maximum_bitpool: 0,
            },
            delay_reporting: false,
            local_in_use: false,
            media_connected: false,
            next_transaction: 0,
            pending_transaction: 0,
            pending_signal: 0,
            fragment_active: false,
            fragment_length: 0,
            fragment_transaction: 0,
            fragment_message: 0,
            fragment_signal: 0,
            fragment_expected_packets: 0,
            fragment_received_packets: 0,
            fragment_payload: [0; FRAGMENT_CAPACITY],
        }
    }

    pub fn connected(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        if self.link != LinkState::Down {
            return Err(Error::State);
        }
        let len = self.command(SIGNAL_DISCOVER, &[], out)?;
        self.link = LinkState::Connected;
        self.state = State::Discovering;
        Ok(len)
    }

    pub fn start(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        if self.link != LinkState::Connected
            || self.state != State::Open
            || self.remote_seid == 0
            || self.pending_signal != 0
        {
            return Err(Error::State);
        }
        let payload = [self.remote_seid << 2];
        let len = self.command(SIGNAL_START, &payload, out)?;
        self.state = State::Starting;
        Ok(len)
    }

    pub fn suspend(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        if self.link != LinkState::Connected
            || self.state != State::Streaming
            || self.remote_seid == 0
            || self.pending_signal != 0
        {
            return Err(Error::State);
        }
        let payload = [self.remote_seid << 2];
        let len = self.command(SIGNAL_SUSPEND, &payload, out)?;
        self.state = State::Suspending;
        Ok(len)
    }

    pub fn receive(&mut self, packet: &[u8], out: &mut [u8]) -> Result<usize, Error> {
        if self.link != LinkState::Connected || packet.is_empty() || packet.len() > MAX_SDU {
            self.reset_fragment();
            return Err(Error::Packet);
        }
        let transaction = packet[0] >> 4;
        let packet_type = (packet[0] >> 2) & 3;
        let message = packet[0] & 3;

        if packet_type == PACKET_SINGLE {
            if self.fragment_active {
                self.reset_fragment();
                return Err(Error::Packet);
            }
            if packet.len() < 2 || packet[1] & 0xc0 != 0 {
                return Err(Error::Packet);
            }
            return self.dispatch(transaction, message, packet[1] & 0x3f, &packet[2..], out);
        }

        if packet_type == PACKET_START {
            if self.fragment_active {
                self.reset_fragment();
                return Err(Error::Packet);
            }
            if packet.len() < 3 || packet[1] < 2 || packet[2] & 0xc0 != 0 {
                return Err(Error::Packet);
            }
            let payload = &packet[3..];
            if payload.len() > FRAGMENT_CAPACITY {
                return Err(Error::Overflow);
            }
            self.fragment_payload[..payload.len()].copy_from_slice(payload);
            self.fragment_active = true;
            self.fragment_length = payload.len() as u8;
            self.fragment_transaction = transaction;
            self.fragment_message = message;
            self.fragment_signal = packet[2] & 0x3f;
            self.fragment_expected_packets = packet[1];
            self.fragment_received_packets = 1;
            return Ok(0);
        }

        if !self.fragment_active
            || transaction != self.fragment_transaction
            || message != self.fragment_message
        {
            self.reset_fragment();
            return Err(Error::Packet);
        }
        let next_packet = self.fragment_received_packets as u16 + 1;
        if (packet_type == PACKET_CONTINUE && next_packet >= self.fragment_expected_packets as u16)
            || (packet_type == PACKET_END && next_packet != self.fragment_expected_packets as u16)
        {
            self.reset_fragment();
            return Err(Error::Packet);
        }
        let fragment = &packet[1..];
        let old_length = self.fragment_length as usize;
        let new_length = old_length + fragment.len();
        if new_length > FRAGMENT_CAPACITY {
            self.reset_fragment();
            return Err(Error::Overflow);
        }
        self.fragment_payload[old_length..new_length].copy_from_slice(fragment);
        self.fragment_length = new_length as u8;
        self.fragment_received_packets = next_packet as u8;
        if packet_type == PACKET_CONTINUE {
            return Ok(0);
        }
        if packet_type != PACKET_END {
            self.reset_fragment();
            return Err(Error::Packet);
        }

        let assembled = self.fragment_payload;
        let assembled_length = self.fragment_length as usize;
        let signal = self.fragment_signal;
        let assembled_transaction = self.fragment_transaction;
        let assembled_message = self.fragment_message;
        self.reset_fragment();
        self.dispatch(
            assembled_transaction,
            assembled_message,
            signal,
            &assembled[..assembled_length],
            out,
        )
    }

    fn dispatch(
        &mut self,
        transaction: u8,
        message: u8,
        signal: u8,
        payload: &[u8],
        out: &mut [u8],
    ) -> Result<usize, Error> {
        match message {
            MSG_ACCEPT => self.accept(transaction, signal, payload, out),
            MSG_REJECT | MSG_GENERAL_REJECT => self.reject(transaction, signal, payload, out),
            MSG_COMMAND => self.remote_command(transaction, signal, payload, out),
            _ => Err(Error::Packet),
        }
    }

    fn reject(
        &mut self,
        transaction: u8,
        signal: u8,
        payload: &[u8],
        out: &mut [u8],
    ) -> Result<usize, Error> {
        if signal != self.pending_signal || transaction != self.pending_transaction {
            return Err(Error::Packet);
        }
        self.pending_signal = 0;
        if signal == SIGNAL_GET_ALL_CAPABILITIES {
            return self.command(SIGNAL_GET_CAPABILITIES, &[self.remote_seid << 2], out);
        }
        match signal {
            SIGNAL_OPEN => self.state = State::Configuring,
            SIGNAL_START => self.state = State::Open,
            SIGNAL_SUSPEND => self.state = State::Streaming,
            SIGNAL_DISCOVER | SIGNAL_GET_CAPABILITIES | SIGNAL_SET_CONFIGURATION => {
                self.state = State::Idle;
                self.remote_seid = 0;
                self.local_in_use = false;
                self.delay_reporting = false;
                self.selected_sbc = SbcConfig::default();
            }
            _ => {}
        }
        Err(Error::Rejected(payload.first().copied().unwrap_or(0)))
    }

    fn reset_fragment(&mut self) {
        self.fragment_active = false;
        self.fragment_length = 0;
        self.fragment_transaction = 0;
        self.fragment_message = 0;
        self.fragment_signal = 0;
        self.fragment_expected_packets = 0;
        self.fragment_received_packets = 0;
    }

    fn accept(
        &mut self,
        transaction: u8,
        signal: u8,
        payload: &[u8],
        out: &mut [u8],
    ) -> Result<usize, Error> {
        if signal != self.pending_signal || transaction != self.pending_transaction {
            return Err(Error::Packet);
        }
        self.pending_signal = 0;
        match signal {
            SIGNAL_DISCOVER => {
                if payload.is_empty() || payload.len() & 1 != 0 {
                    return Err(Error::Packet);
                }
                let seid = payload
                    .chunks_exact(2)
                    .find_map(|item| {
                        let candidate = item[0] >> 2;
                        let in_use = (item[0] >> 1) & 1;
                        let media_type = item[1] >> 4;
                        let sink = (item[1] >> 3) & 1;
                        (candidate > 0
                            && candidate <= 0x3e
                            && item[0] & 1 == 0
                            && item[1] & 7 == 0
                            && in_use == 0
                            && media_type == 0
                            && sink == 1)
                            .then_some(candidate)
                    })
                    .ok_or(Error::Unsupported)?;
                self.remote_seid = seid;
                self.state = State::ReadingCapabilities;
                self.command(SIGNAL_GET_ALL_CAPABILITIES, &[seid << 2], out)
            }
            SIGNAL_GET_ALL_CAPABILITIES | SIGNAL_GET_CAPABILITIES => self.choose_sbc(payload, out),
            SIGNAL_SET_CONFIGURATION => {
                if !payload.is_empty() {
                    return Err(Error::Packet);
                }
                self.local_in_use = true;
                self.state = State::Opening;
                self.command(SIGNAL_OPEN, &[self.remote_seid << 2], out)
            }
            SIGNAL_OPEN => {
                if !payload.is_empty() {
                    return Err(Error::Packet);
                }
                self.state = State::Open;
                Ok(0)
            }
            SIGNAL_START => {
                if self.state != State::Starting || !payload.is_empty() {
                    return Err(Error::Packet);
                }
                self.state = State::Streaming;
                Ok(0)
            }
            SIGNAL_SUSPEND => {
                if self.state != State::Suspending || !payload.is_empty() {
                    return Err(Error::Packet);
                }
                self.state = State::Open;
                Ok(0)
            }
            _ => Err(Error::Packet),
        }
    }

    fn choose_sbc(&mut self, payload: &[u8], out: &mut [u8]) -> Result<usize, Error> {
        let mut offset = 0;
        let mut found = None;
        let mut delay = false;
        while offset + 2 <= payload.len() {
            let category = payload[offset];
            let length = payload[offset + 1] as usize;
            offset += 2;
            if offset + length > payload.len() {
                return Err(Error::Packet);
            }
            let value = &payload[offset..offset + length];
            if category == CATEGORY_DELAY_REPORTING && length == 0 {
                delay = true;
            }
            if category == CATEGORY_MEDIA_CODEC
                && length == 6
                && value[0] >> 4 == 0
                && value[1] == 0
            {
                let frequency_channel = value[2] & 0x22;
                let shape = value[3] & 0x15;
                let minimum_bitpool = value[4].max(crate::sbc_tone_frames::MIN_BITPOOL);
                let maximum_bitpool = value[5].min(crate::sbc_tone_frames::MAX_BITPOOL);
                if frequency_channel == 0x22 && shape == 0x15 && minimum_bitpool <= maximum_bitpool
                {
                    found = Some(SbcConfig {
                        frequency_channel: 0x22,
                        blocks_subbands_allocation: 0x15,
                        minimum_bitpool,
                        maximum_bitpool,
                    });
                }
            }
            offset += length;
        }
        if offset != payload.len() {
            return Err(Error::Packet);
        }
        let config = found.ok_or(Error::Unsupported)?;
        self.selected_sbc = config;
        self.delay_reporting = delay;
        let mut bytes = [0u8; 14];
        bytes[..12].copy_from_slice(&[
            self.remote_seid << 2,
            self.local_seid << 2,
            1,
            0,
            7,
            6,
            0,
            0,
            0x22,
            0x15,
            config.minimum_bitpool,
            config.maximum_bitpool,
        ]);
        let len = if delay {
            bytes[12] = 8;
            bytes[13] = 0;
            14
        } else {
            12
        };
        self.state = State::Configuring;
        self.command(SIGNAL_SET_CONFIGURATION, &bytes[..len], out)
    }

    fn valid_local_seid(&self, payload: &[u8]) -> bool {
        payload.len() == 1 && payload[0] & 3 == 0 && payload[0] >> 2 == self.local_seid
    }

    fn remote_command(
        &mut self,
        transaction: u8,
        signal: u8,
        payload: &[u8],
        out: &mut [u8],
    ) -> Result<usize, Error> {
        let local = self.local_seid << 2;
        match signal {
            SIGNAL_DISCOVER => {
                if !payload.is_empty() {
                    return response(
                        transaction,
                        signal,
                        MSG_REJECT,
                        &[ERROR_BAD_PAYLOAD_FORMAT],
                        out,
                    );
                }
                response(
                    transaction,
                    signal,
                    MSG_ACCEPT,
                    &[local | if self.local_in_use { 2 } else { 0 }, 0],
                    out,
                )
            }
            SIGNAL_GET_CAPABILITIES | SIGNAL_GET_ALL_CAPABILITIES => {
                if !self.valid_local_seid(payload) {
                    return response(transaction, signal, MSG_REJECT, &[ERROR_BAD_ACP_SEID], out);
                }
                let capabilities = [
                    CATEGORY_MEDIA_TRANSPORT,
                    0,
                    CATEGORY_MEDIA_CODEC,
                    6,
                    0,
                    0,
                    0x22,
                    0x15,
                    crate::sbc_tone_frames::MIN_BITPOOL,
                    crate::sbc_tone_frames::MAX_BITPOOL,
                    CATEGORY_DELAY_REPORTING,
                    0,
                ];
                let len = if signal == SIGNAL_GET_ALL_CAPABILITIES {
                    capabilities.len()
                } else {
                    capabilities.len() - 2
                };
                response(transaction, signal, MSG_ACCEPT, &capabilities[..len], out)
            }
            SIGNAL_GET_CONFIGURATION => {
                if !self.valid_local_seid(payload) {
                    return response(transaction, signal, MSG_REJECT, &[ERROR_BAD_ACP_SEID], out);
                }
                if !self.local_in_use {
                    return response(transaction, signal, MSG_REJECT, &[ERROR_BAD_STATE], out);
                }
                let mut configuration = [
                    CATEGORY_MEDIA_TRANSPORT,
                    0,
                    CATEGORY_MEDIA_CODEC,
                    6,
                    0,
                    0,
                    self.selected_sbc.frequency_channel,
                    self.selected_sbc.blocks_subbands_allocation,
                    self.selected_sbc.minimum_bitpool,
                    self.selected_sbc.maximum_bitpool,
                    0,
                    0,
                ];
                let len = if self.delay_reporting {
                    configuration[10] = CATEGORY_DELAY_REPORTING;
                    12
                } else {
                    10
                };
                response(transaction, signal, MSG_ACCEPT, &configuration[..len], out)
            }
            SIGNAL_CLOSE | SIGNAL_ABORT => {
                if !self.valid_local_seid(payload) {
                    return response(transaction, signal, MSG_REJECT, &[ERROR_BAD_ACP_SEID], out);
                }
                let len = response(transaction, signal, MSG_ACCEPT, &[], out)?;
                self.state = State::Idle;
                self.local_in_use = false;
                self.remote_seid = 0;
                self.pending_signal = 0;
                self.delay_reporting = false;
                self.media_connected = false;
                Ok(len)
            }
            SIGNAL_DELAY_REPORT => {
                if payload.len() != 3 || payload[0] != local {
                    return response(transaction, signal, MSG_REJECT, &[ERROR_BAD_ACP_SEID], out);
                }
                if !self.local_in_use || !self.delay_reporting {
                    return response(transaction, signal, MSG_REJECT, &[ERROR_BAD_STATE], out);
                }
                let len = response(transaction, signal, MSG_ACCEPT, &[], out)?;
                Ok(len)
            }
            SIGNAL_START if self.valid_local_seid(payload) => {
                if self.state != State::Open || !self.media_connected {
                    return response(transaction, signal, MSG_REJECT, &[ERROR_BAD_STATE], out);
                }
                let len = response(transaction, signal, MSG_ACCEPT, &[], out)?;
                self.state = State::Streaming;
                Ok(len)
            }
            SIGNAL_SUSPEND if self.valid_local_seid(payload) => {
                if self.state != State::Streaming {
                    return response(transaction, signal, MSG_REJECT, &[ERROR_BAD_STATE], out);
                }
                let len = response(transaction, signal, MSG_ACCEPT, &[], out)?;
                self.state = State::Open;
                Ok(len)
            }
            SIGNAL_START | SIGNAL_SUSPEND => {
                response(transaction, signal, MSG_REJECT, &[ERROR_BAD_ACP_SEID], out)
            }
            SIGNAL_RECONFIGURE => {
                response(transaction, signal, MSG_REJECT, &[ERROR_BAD_STATE], out)
            }
            SIGNAL_SECURITY_CONTROL | SIGNAL_SET_CONFIGURATION | SIGNAL_OPEN => {
                response(transaction, signal, MSG_GENERAL_REJECT, &[], out)
            }
            _ => response(transaction, signal, MSG_GENERAL_REJECT, &[], out),
        }
    }

    fn command(&mut self, signal: u8, payload: &[u8], out: &mut [u8]) -> Result<usize, Error> {
        let transaction = self.next_transaction & 0x0f;
        self.next_transaction = (transaction + 1) & 0x0f;
        let len = response(transaction, signal, MSG_COMMAND, payload, out)?;
        self.pending_transaction = transaction;
        self.pending_signal = signal;
        Ok(len)
    }
}

fn response(
    transaction: u8,
    signal: u8,
    message: u8,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, Error> {
    let len = 2 + payload.len();
    if out.len() < len {
        return Err(Error::Overflow);
    }
    out[0] = (transaction << 4) | message;
    out[1] = signal;
    out[2..len].copy_from_slice(payload);
    Ok(len)
}
