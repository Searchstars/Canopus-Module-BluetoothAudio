use crate::{Address, ConnectionState, DiscoveredDevice, Model, Peer, ScanState};

pub trait Platform {
    type Error;
    fn start_discovery(&mut self, timeout_seconds: u8) -> Result<(), Self::Error>;
    fn stop_discovery(&mut self) -> Result<(), Self::Error>;
    /// Queries both target bond records and removes any local Classic record.
    /// Returns `true` when pairing must wait for a removal callback.
    fn prepare_bond(&mut self, address: Address) -> Result<bool, Self::Error>;
    fn create_bond(&mut self, address: Address) -> Result<(), Self::Error>;
    fn connect_avdtp(&mut self, address: Address) -> Result<(), Self::Error>;
    fn disconnect_avdtp(&mut self, address: Address) -> Result<(), Self::Error>;
    fn play_test_tone(&mut self) -> Result<(), Self::Error>;
}

pub struct Controller<P> {
    pub model: Model,
    platform: P,
}

impl<P: Platform> Controller<P> {
    pub fn new(platform: P) -> Self {
        Self {
            model: Model::default(),
            platform,
        }
    }
    pub fn platform(&self) -> &P {
        &self.platform
    }
    pub fn platform_mut(&mut self) -> &mut P {
        &mut self.platform
    }

    pub fn start_scan(&mut self) -> Result<(), P::Error> {
        self.model.devices.begin_scan();
        self.model.scan = ScanState::Starting;
        self.model.last_error = 0;
        self.model.touch();
        match self.platform.start_discovery(20) {
            Ok(()) => {
                self.model.scan = ScanState::Scanning;
                self.model.touch();
                Ok(())
            }
            Err(error) => {
                self.model.scan = ScanState::Failed;
                self.model.touch();
                Err(error)
            }
        }
    }

    pub fn stop_scan(&mut self) -> Result<(), P::Error> {
        if !matches!(self.model.scan, ScanState::Scanning | ScanState::Starting) {
            return Ok(());
        }
        self.model.scan = ScanState::Stopping;
        self.model.touch();
        match self.platform.stop_discovery() {
            Ok(()) => Ok(()),
            Err(error) => {
                self.model.scan = ScanState::Scanning;
                self.model.touch();
                Err(error)
            }
        }
    }

    pub fn discovery_result(&mut self, device: DiscoveredDevice) {
        self.model.devices.upsert(device);
        self.model.touch();
    }

    pub fn discovery_stopped(&mut self) -> Result<(), P::Error> {
        self.model.scan = ScanState::Idle;
        if self.model.connection == ConnectionState::WaitingForScanStop {
            self.begin_bond_or_connect()?;
        }
        self.model.touch();
        Ok(())
    }

    pub fn select(&mut self, index: usize) -> Result<(), P::Error> {
        let Some(device) = self.model.devices.get(index).copied() else {
            return Ok(());
        };
        let peer = Peer {
            address: device.address,
            name: device.name,
            rssi: device.rssi,
            class_of_device: device.class_of_device,
            bond: Default::default(),
        };
        self.model.selected = Some(peer);
        self.model.last_error = 0;
        if matches!(self.model.scan, ScanState::Scanning | ScanState::Starting) {
            self.model.connection = ConnectionState::WaitingForScanStop;
            self.model.scan = ScanState::Stopping;
            self.model.touch();
            match self.platform.stop_discovery() {
                Ok(()) => Ok(()),
                Err(error) => {
                    self.model.scan = ScanState::Scanning;
                    self.model.connection = ConnectionState::Failed;
                    self.model.touch();
                    Err(error)
                }
            }
        } else {
            self.begin_bond_or_connect()
        }
    }

    fn begin_bond_or_connect(&mut self) -> Result<(), P::Error> {
        let address = match self.model.selected {
            Some(peer) => peer.address,
            None => return Ok(()),
        };
        self.model.connection = ConnectionState::CheckingBond;
        self.model.touch();

        // The exact target cannot safely infer that a retained local record is
        // usable by the remote peer. Always query and remove it, then begin a
        // fresh stock Classic bond. The NONE callback is the removal commit.
        self.model.connection = ConnectionState::RemovingBond;
        self.model.touch();
        match self.platform.prepare_bond(address) {
            Ok(true) => Ok(()),
            Ok(false) => self.begin_pair(address),
            Err(error) => {
                self.model.connection = ConnectionState::Failed;
                self.model.touch();
                Err(error)
            }
        }
    }

    fn begin_pair(&mut self, address: Address) -> Result<(), P::Error> {
        self.model.connection = ConnectionState::Pairing;
        self.model.touch();
        match self.platform.create_bond(address) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.model.connection = ConnectionState::Failed;
                self.model.touch();
                Err(error)
            }
        }
    }

    pub fn bond_removed(&mut self, address: Address, success: bool) -> Result<(), P::Error> {
        if self.model.selected.map(|peer| peer.address) != Some(address)
            || self.model.connection != ConnectionState::RemovingBond
        {
            return Ok(());
        }
        if !success {
            self.model.connection = ConnectionState::Failed;
            self.model.touch();
            return Ok(());
        }
        self.begin_pair(address)
    }

    pub fn bond_complete(&mut self, address: Address, success: bool) -> Result<(), P::Error> {
        if self.model.selected.map(|p| p.address) != Some(address)
            || self.model.connection != ConnectionState::Pairing
        {
            return Ok(());
        }
        if !success {
            self.model.connection = ConnectionState::Failed;
            self.model.touch();
            return Ok(());
        }
        self.model.connection = ConnectionState::Connecting;
        self.model.touch();
        match self.platform.connect_avdtp(address) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.model.connection = ConnectionState::Failed;
                self.model.touch();
                Err(error)
            }
        }
    }

    pub fn connected(&mut self, address: Address) {
        if self.model.selected.map(|p| p.address) != Some(address) {
            return;
        }
        self.model.connected = self.model.selected;
        self.model.connection = ConnectionState::Configuring;
        self.model.touch();
    }

    pub fn stream_ready(&mut self) {
        self.model.connection = ConnectionState::Ready;
        self.model.stream = crate::StreamState::Open;
        self.model.touch();
    }

    pub fn disconnect(&mut self) -> Result<(), P::Error> {
        let Some(peer) = self.model.connected else {
            return Ok(());
        };
        self.model.connection = ConnectionState::Disconnecting;
        self.model.touch();
        self.platform.disconnect_avdtp(peer.address)
    }

    pub fn disconnected(&mut self, address: Address) {
        if self.model.connected.map(|p| p.address) != Some(address) {
            return;
        }
        self.model.connected = None;
        self.model.selected = None;
        self.model.connection = ConnectionState::Disconnected;
        self.model.stream = crate::StreamState::Idle;
        self.model.details = Default::default();
        self.model.touch();
    }

    pub fn play_test_tone(&mut self) -> Result<(), P::Error> {
        if self.model.connection != ConnectionState::Ready
            || self.model.stream != crate::StreamState::Open
        {
            return Ok(());
        }
        self.platform.play_test_tone()
    }
}
