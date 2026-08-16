use crate::{Address, DeviceName, DiscoveryTable};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ScanState {
    #[default]
    Idle,
    Starting,
    Scanning,
    Stopping,
    Failed,
}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ConnectionState {
    #[default]
    Disconnected,
    WaitingForScanStop,
    CheckingBond,
    RemovingBond,
    Pairing,
    Connecting,
    Configuring,
    Ready,
    Disconnecting,
    Failed,
}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum BondState {
    #[default]
    Unknown,
    NotBonded,
    Bonding,
    Bonded,
    Failed,
}
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum StreamState {
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
pub struct Peer {
    pub address: Address,
    pub name: DeviceName,
    pub rssi: i32,
    pub class_of_device: u32,
    pub bond: BondState,
}

pub const PAIR_DIAG_FILTER_HIT: u8 = 1 << 0;
pub const PAIR_DIAG_REQUEST: u8 = 1 << 1;
pub const PAIR_DIAG_DISPLAY: u8 = 1 << 2;
pub const PAIR_DIAG_BONDED: u8 = 1 << 3;
pub const PAIR_DIAG_REMOVE_PENDING: u8 = 1 << 4;
pub const PAIR_DIAG_REMOVE_CONFIRMED: u8 = 1 << 5;
pub const PAIR_DIAG_MHDT_FIXED: u8 = 1 << 6;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkDetails {
    pub signaling_cid: u16,
    pub signaling_mtu: u16,
    pub media_cid: u16,
    pub media_mtu: u16,
    pub avrcp_cid: u16,
    pub avrcp_mtu: u16,
    pub avrcp_state: u8,
    pub avrcp_volume: u8,
    pub avrcp_packets_sent: u32,
    pub avrcp_packets_received: u32,
    pub avrcp_last_event: u32,
    pub avrcp_rx_header: u32,
    pub avrcp_rx_length: u16,
    pub avrcp_error: i32,
    pub stock_bond_state: u8,
    pub device_bond_state: u8,
    pub pairing_flags: u8,
    pub sbc_frequency_channel: u8,
    pub sbc_blocks_subbands_allocation: u8,
    pub bitpool: u8,
    pub packets_sent: u32,
    pub frames_sent: u32,
    pub audio_state: u8,
    pub audio_stage: u8,
    pub decoded_channels: u8,
    pub decoded_sample_rate: u32,
    pub input_used: u32,
    pub pcm_frames: u32,
    pub audio_rtp_packets: u32,
    pub media_packets_queued: u32,
    pub media_flow_events: u32,
    pub media_tx_outstanding: u32,
    pub startup_silence_packets: u32,
    pub underruns: u32,
    pub audio_elapsed_ms: u32,
    pub decode_cpu_ms: u32,
    pub startup_ms: u32,
    pub startup_queue_ms: u32,
    pub startup_prepare_ms: u32,
    pub startup_avdtp_ms: u32,
    pub audio_error: i32,
    pub audio_register_result: i32,
    pub audio_probe_result: i32,
    pub audio_probe_abi: u32,
    pub audio_last_command: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub generation: u32,
    pub scan: ScanState,
    pub connection: ConnectionState,
    pub stream: StreamState,
    pub devices: DiscoveryTable,
    pub selected: Option<Peer>,
    pub connected: Option<Peer>,
    pub details: LinkDetails,
    pub last_error: i32,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            generation: 1,
            scan: ScanState::Idle,
            connection: ConnectionState::Disconnected,
            stream: StreamState::Idle,
            devices: DiscoveryTable::default(),
            selected: None,
            connected: None,
            details: LinkDetails::default(),
            last_error: 0,
        }
    }
}

impl Model {
    pub fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
    }
}
