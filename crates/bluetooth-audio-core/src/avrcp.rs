//! Allocation-free AVRCP Controller absolute-volume protocol.
//!
//! The target transport owns AVCTP/L2CAP. This state machine emits and consumes
//! complete single-packet AVCTP AV/C vendor-dependent messages.

const AVCTP_PROFILE_AVRCP: u16 = 0x110e;
const AVCTP_COMMAND: u8 = 0;
const AVCTP_RESPONSE: u8 = 1;
const AVC_CONTROL: u8 = 0x00;
const AVC_NOTIFY: u8 = 0x03;
const AVC_NOT_IMPLEMENTED: u8 = 0x08;
const AVC_ACCEPTED: u8 = 0x09;
const AVC_REJECTED: u8 = 0x0a;
const AVC_STABLE: u8 = 0x0c;
const AVC_CHANGED: u8 = 0x0d;
const AVC_INTERIM: u8 = 0x0f;
const AVC_SUBUNIT_PANEL: u8 = 0x48;
const AVC_OPCODE_VENDOR_DEPENDENT: u8 = 0x00;
const AVC_OPCODE_PASS_THROUGH: u8 = 0x7c;
const AVRCP_OPERATION_RELEASED: u8 = 0x80;
const AVRCP_OPERATION_MASK: u8 = 0x7f;
const AVRCP_OPERATION_PLAY: u8 = 0x44;
const AVRCP_OPERATION_PAUSE: u8 = 0x46;
const AVRCP_OPERATION_FORWARD: u8 = 0x4b;
const AVRCP_OPERATION_BACKWARD: u8 = 0x4c;
const BLUETOOTH_SIG_COMPANY_ID: [u8; 3] = [0x00, 0x19, 0x58];
const PDU_GET_CAPABILITIES: u8 = 0x10;
const PDU_REGISTER_NOTIFICATION: u8 = 0x31;
const PDU_SET_ABSOLUTE_VOLUME: u8 = 0x50;
const CAPABILITY_COMPANY_ID: u8 = 0x02;
const CAPABILITY_EVENTS_SUPPORTED: u8 = 0x03;
const EVENT_VOLUME_CHANGED: u8 = 0x0d;
const NO_TRANSACTION: u8 = 0xff;

pub const MAX_VOLUME: u8 = 0x7f;
/// Fallback until the headset reports its current absolute volume.
pub const DEFAULT_VOLUME: u8 = 0x20;

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum State {
    #[default]
    Disconnected,
    Ready,
    WaitingSetVolume,
    WaitingInterim,
    Registered,
    Failed,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    State,
    Packet,
    Rejected,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MediaControl {
    Play,
    Pause,
    Next,
    Previous,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Event {
    None,
    Volume(u8),
    Reregister,
    PeerCommand(usize),
    PeerVolume {
        volume: u8,
        response_len: usize,
    },
    PeerControl {
        control: MediaControl,
        response_len: usize,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Controller {
    pub state: State,
    pub volume: u8,
    next_transaction: u8,
    control_transaction: u8,
    notification_transaction: u8,
    notification_registered: bool,
    target_notification_transaction: u8,
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller {
    pub const fn new() -> Self {
        Self {
            state: State::Disconnected,
            volume: DEFAULT_VOLUME,
            next_transaction: 0,
            control_transaction: NO_TRANSACTION,
            notification_transaction: NO_TRANSACTION,
            notification_registered: false,
            target_notification_transaction: NO_TRANSACTION,
        }
    }

    pub fn connected(&mut self, volume: u8, out: &mut [u8]) -> Result<usize, Error> {
        if self.state != State::Disconnected || volume > MAX_VOLUME {
            return Err(Error::State);
        }
        self.state = State::Ready;
        self.volume = volume;
        self.set_absolute_volume(volume, out)
    }

    /// Starts the controller without writing a persisted/default volume to the
    /// peer. The first `REGISTER_NOTIFICATION` interim response becomes the
    /// source of truth for the headset's current volume.
    pub fn connected_for_volume_sync(&mut self, volume: u8) -> Result<(), Error> {
        if self.state != State::Disconnected || volume > MAX_VOLUME {
            return Err(Error::State);
        }
        self.state = State::Ready;
        self.volume = volume;
        Ok(())
    }

    pub fn target_connected(&mut self, volume: u8) -> Result<(), Error> {
        if self.state != State::Disconnected || volume > MAX_VOLUME {
            return Err(Error::State);
        }
        self.state = State::Ready;
        self.volume = volume;
        Ok(())
    }

    pub fn target_volume_changed(&mut self, volume: u8, out: &mut [u8]) -> Result<usize, Error> {
        if self.state == State::Disconnected || volume > MAX_VOLUME {
            return Err(Error::State);
        }
        self.volume = volume;
        if self.target_notification_transaction == NO_TRANSACTION {
            return Ok(0);
        }
        let transaction = self.target_notification_transaction;
        self.target_notification_transaction = NO_TRANSACTION;
        self.response(
            transaction,
            AVC_CHANGED,
            PDU_REGISTER_NOTIFICATION,
            &[EVENT_VOLUME_CHANGED, volume],
            out,
        )
    }

    pub fn disconnected(&mut self) {
        *self = Self::new();
    }

    pub fn set_absolute_volume(&mut self, volume: u8, out: &mut [u8]) -> Result<usize, Error> {
        if matches!(self.state, State::Disconnected | State::Failed) || volume > MAX_VOLUME {
            return Err(Error::State);
        }
        // A volume slider can advance again before the peer answers the previous
        // command. Supersede that transaction; a late response is ignored by
        // receive(), while the newest requested volume remains authoritative.
        let (len, transaction) =
            self.command(AVC_CONTROL, PDU_SET_ABSOLUTE_VOLUME, &[volume], out)?;
        self.volume = volume;
        self.control_transaction = transaction;
        self.refresh_state();
        Ok(len)
    }

    pub fn register_volume_notification(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        if matches!(self.state, State::Disconnected | State::Failed)
            || self.notification_transaction != NO_TRANSACTION
        {
            return Err(Error::State);
        }
        let (len, transaction) = self.command(
            AVC_NOTIFY,
            PDU_REGISTER_NOTIFICATION,
            &[EVENT_VOLUME_CHANGED, 0, 0, 0, 0],
            out,
        )?;
        self.notification_transaction = transaction;
        self.notification_registered = false;
        self.refresh_state();
        Ok(len)
    }

    pub fn receive(&mut self, packet: &[u8], out: &mut [u8]) -> Result<Event, Error> {
        if packet.len() < 6 || self.state == State::Disconnected {
            return Err(Error::Packet);
        }
        let transaction = packet[0] >> 4;
        let packet_type = (packet[0] >> 2) & 3;
        let response = (packet[0] >> 1) & 1;
        let ipid = packet[0] & 1;
        if packet_type != 0
            || ipid != 0
            || u16::from_be_bytes([packet[1], packet[2]]) != AVCTP_PROFILE_AVRCP
            || packet[4] != AVC_SUBUNIT_PANEL
        {
            return Err(Error::Packet);
        }
        let ctype = packet[3] & 0x0f;
        if packet[5] == AVC_OPCODE_PASS_THROUGH {
            if response == AVCTP_COMMAND {
                return self.receive_pass_through(transaction, ctype, packet, out);
            }
            return if response == AVCTP_RESPONSE {
                Ok(Event::None)
            } else {
                Err(Error::Packet)
            };
        }
        if packet.len() < 13 || packet[5] != AVC_OPCODE_VENDOR_DEPENDENT {
            return Err(Error::Packet);
        }
        let pdu = packet[9];
        let parameter_length = u16::from_be_bytes([packet[11], packet[12]]) as usize;
        if packet.len() != 13 + parameter_length {
            return Err(Error::Packet);
        }
        let parameters = &packet[13..];

        if packet[6..9] != BLUETOOTH_SIG_COMPANY_ID || packet[10] & 3 != 0 {
            return Err(Error::Packet);
        }
        if response == AVCTP_COMMAND {
            return self.receive_command(transaction, ctype, pdu, parameters, out);
        }
        if response != AVCTP_RESPONSE {
            return Err(Error::Packet);
        }

        if pdu == PDU_REGISTER_NOTIFICATION
            && transaction == self.notification_transaction
            && parameters.len() == 2
            && parameters[0] == EVENT_VOLUME_CHANGED
        {
            let volume = parameters[1] & MAX_VOLUME;
            self.volume = volume;
            return match ctype {
                AVC_INTERIM if !self.notification_registered => {
                    self.notification_registered = true;
                    self.refresh_state();
                    Ok(Event::Volume(volume))
                }
                AVC_CHANGED if self.notification_registered => {
                    self.notification_transaction = NO_TRANSACTION;
                    self.notification_registered = false;
                    self.refresh_state();
                    Ok(Event::Reregister)
                }
                _ => Err(Error::Rejected),
            };
        }

        if pdu == PDU_REGISTER_NOTIFICATION
            && transaction == self.notification_transaction
            && matches!(ctype, AVC_REJECTED | AVC_NOT_IMPLEMENTED)
        {
            self.notification_transaction = NO_TRANSACTION;
            self.notification_registered = false;
            self.refresh_state();
            return Err(Error::Rejected);
        }

        if pdu == PDU_SET_ABSOLUTE_VOLUME && transaction == self.control_transaction {
            return match (ctype, parameters) {
                (AVC_ACCEPTED, [volume]) if *volume <= MAX_VOLUME => {
                    self.volume = *volume;
                    self.control_transaction = NO_TRANSACTION;
                    self.refresh_state();
                    Ok(Event::Volume(self.volume))
                }
                (AVC_REJECTED | AVC_NOT_IMPLEMENTED, _) => {
                    self.control_transaction = NO_TRANSACTION;
                    self.refresh_state();
                    Err(Error::Rejected)
                }
                _ => Err(Error::Rejected),
            };
        }
        // Unknown, stale, duplicate, and unrelated responses are legal on a
        // long-lived dual-role AVCTP channel. They must not poison current
        // transactions or make A2DP unusable.
        Ok(Event::None)
    }

    fn receive_pass_through(
        &self,
        transaction: u8,
        ctype: u8,
        packet: &[u8],
        out: &mut [u8],
    ) -> Result<Event, Error> {
        if ctype != AVC_CONTROL || packet.len() < 8 {
            return Err(Error::Packet);
        }
        let operation_data_length = packet[7] as usize;
        if packet.len() != 8 + operation_data_length {
            return Err(Error::Packet);
        }
        let released = packet[6] & AVRCP_OPERATION_RELEASED != 0;
        let control = match packet[6] & AVRCP_OPERATION_MASK {
            AVRCP_OPERATION_PLAY => Some(MediaControl::Play),
            AVRCP_OPERATION_PAUSE => Some(MediaControl::Pause),
            AVRCP_OPERATION_FORWARD => Some(MediaControl::Next),
            AVRCP_OPERATION_BACKWARD => Some(MediaControl::Previous),
            _ => None,
        };
        let response = if control.is_some() {
            AVC_ACCEPTED
        } else {
            AVC_NOT_IMPLEMENTED
        };
        let response_len = Self::pass_through_response(transaction, response, packet, out)?;
        match (control, released) {
            (Some(control), false) => Ok(Event::PeerControl {
                control,
                response_len,
            }),
            _ => Ok(Event::PeerCommand(response_len)),
        }
    }

    fn pass_through_response(
        transaction: u8,
        ctype: u8,
        packet: &[u8],
        out: &mut [u8],
    ) -> Result<usize, Error> {
        if out.len() < packet.len() {
            return Err(Error::Packet);
        }
        out[..packet.len()].copy_from_slice(packet);
        out[0] = transaction << 4 | AVCTP_RESPONSE << 1;
        out[3] = ctype;
        Ok(packet.len())
    }

    fn receive_command(
        &mut self,
        transaction: u8,
        ctype: u8,
        pdu: u8,
        parameters: &[u8],
        out: &mut [u8],
    ) -> Result<Event, Error> {
        match (ctype, pdu, parameters) {
            (0x01, PDU_GET_CAPABILITIES, [CAPABILITY_COMPANY_ID]) => {
                let len = self.response(
                    transaction,
                    AVC_STABLE,
                    pdu,
                    &[CAPABILITY_COMPANY_ID, 1, 0x00, 0x19, 0x58],
                    out,
                )?;
                Ok(Event::PeerCommand(len))
            }
            (0x01, PDU_GET_CAPABILITIES, [CAPABILITY_EVENTS_SUPPORTED]) => {
                let len = self.response(
                    transaction,
                    AVC_STABLE,
                    pdu,
                    &[CAPABILITY_EVENTS_SUPPORTED, 1, EVENT_VOLUME_CHANGED],
                    out,
                )?;
                Ok(Event::PeerCommand(len))
            }
            (AVC_CONTROL, PDU_SET_ABSOLUTE_VOLUME, [volume]) if *volume <= MAX_VOLUME => {
                self.volume = *volume;
                let len = self.response(transaction, AVC_ACCEPTED, pdu, &[*volume], out)?;
                Ok(Event::PeerVolume {
                    volume: *volume,
                    response_len: len,
                })
            }
            (AVC_NOTIFY, PDU_REGISTER_NOTIFICATION, [EVENT_VOLUME_CHANGED, _, _, _, _]) => {
                self.target_notification_transaction = transaction;
                let len = self.response(
                    transaction,
                    AVC_INTERIM,
                    pdu,
                    &[EVENT_VOLUME_CHANGED, self.volume],
                    out,
                )?;
                Ok(Event::PeerCommand(len))
            }
            _ => {
                let response = if pdu == PDU_GET_CAPABILITIES
                    || pdu == PDU_SET_ABSOLUTE_VOLUME
                    || pdu == PDU_REGISTER_NOTIFICATION
                {
                    AVC_REJECTED
                } else {
                    AVC_NOT_IMPLEMENTED
                };
                let len = self.response(transaction, response, pdu, parameters, out)?;
                Ok(Event::PeerCommand(len))
            }
        }
    }

    fn response(
        &self,
        transaction: u8,
        ctype: u8,
        pdu: u8,
        parameters: &[u8],
        out: &mut [u8],
    ) -> Result<usize, Error> {
        let length = 13 + parameters.len();
        if out.len() < length || parameters.len() > u16::MAX as usize {
            return Err(Error::Packet);
        }
        out[..length].fill(0);
        out[0] = transaction << 4 | AVCTP_RESPONSE << 1;
        out[1..3].copy_from_slice(&AVCTP_PROFILE_AVRCP.to_be_bytes());
        out[3] = ctype;
        out[4] = AVC_SUBUNIT_PANEL;
        out[5] = AVC_OPCODE_VENDOR_DEPENDENT;
        out[6..9].copy_from_slice(&BLUETOOTH_SIG_COMPANY_ID);
        out[9] = pdu;
        out[11..13].copy_from_slice(&(parameters.len() as u16).to_be_bytes());
        out[13..length].copy_from_slice(parameters);
        Ok(length)
    }

    fn command(
        &mut self,
        ctype: u8,
        pdu: u8,
        parameters: &[u8],
        out: &mut [u8],
    ) -> Result<(usize, u8), Error> {
        let length = 13 + parameters.len();
        if out.len() < length || parameters.len() > u16::MAX as usize {
            return Err(Error::Packet);
        }
        let transaction = self.allocate_transaction();
        out[..length].fill(0);
        out[0] = transaction << 4 | AVCTP_COMMAND << 1;
        out[1..3].copy_from_slice(&AVCTP_PROFILE_AVRCP.to_be_bytes());
        out[3] = ctype;
        out[4] = AVC_SUBUNIT_PANEL;
        out[5] = AVC_OPCODE_VENDOR_DEPENDENT;
        out[6..9].copy_from_slice(&BLUETOOTH_SIG_COMPANY_ID);
        out[9] = pdu;
        out[10] = 0;
        out[11..13].copy_from_slice(&(parameters.len() as u16).to_be_bytes());
        out[13..length].copy_from_slice(parameters);
        Ok((length, transaction))
    }

    fn allocate_transaction(&mut self) -> u8 {
        // At most two locally initiated transactions are live. Skip both labels
        // so wraparound cannot make a control response look like a notification
        // response (or vice versa).
        for _ in 0..16 {
            let transaction = self.next_transaction & 0x0f;
            self.next_transaction = self.next_transaction.wrapping_add(1) & 0x0f;
            if transaction != self.control_transaction
                && transaction != self.notification_transaction
            {
                return transaction;
            }
        }
        // Two occupied labels cannot exhaust a 16-label namespace.
        unreachable!()
    }

    fn refresh_state(&mut self) {
        self.state = if self.control_transaction != NO_TRANSACTION {
            State::WaitingSetVolume
        } else if self.notification_transaction == NO_TRANSACTION {
            State::Ready
        } else if self.notification_registered {
            State::Registered
        } else {
            State::WaitingInterim
        };
    }
}

pub fn percent_to_absolute(percent: u32) -> u8 {
    percent
        .min(100)
        .saturating_mul(MAX_VOLUME as u32)
        .div_ceil(100) as u8
}

pub fn absolute_to_percent(volume: u8) -> u32 {
    u32::from(volume.min(MAX_VOLUME))
        .saturating_mul(100)
        .div_ceil(MAX_VOLUME as u32)
}
