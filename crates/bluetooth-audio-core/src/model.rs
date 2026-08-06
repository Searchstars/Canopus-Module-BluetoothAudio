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

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkDetails {
    pub signaling_cid: u16,
    pub signaling_mtu: u16,
    pub media_cid: u16,
    pub media_mtu: u16,
    pub sbc_frequency_channel: u8,
    pub sbc_blocks_subbands_allocation: u8,
    pub bitpool: u8,
    pub packets_sent: u32,
    pub frames_sent: u32,
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
