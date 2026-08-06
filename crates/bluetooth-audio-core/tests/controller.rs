use canopus_bluetooth_audio_core::{
    Address, ConnectionState, Controller, DiscoveredDevice, Platform,
};

#[derive(Default)]
struct Fake {
    calls: [u8; 8],
    len: usize,
    bonded: bool,
}
impl Fake {
    fn push(&mut self, value: u8) {
        self.calls[self.len] = value;
        self.len += 1;
    }
}
impl Platform for Fake {
    type Error = ();
    fn start_discovery(&mut self, _: u8) -> Result<(), Self::Error> {
        self.push(1);
        Ok(())
    }
    fn stop_discovery(&mut self) -> Result<(), Self::Error> {
        self.push(2);
        Ok(())
    }
    fn is_bonded(&mut self, _: Address) -> Result<bool, Self::Error> {
        self.push(3);
        Ok(self.bonded)
    }
    fn create_bond(&mut self, _: Address) -> Result<(), Self::Error> {
        self.push(4);
        Ok(())
    }
    fn connect_avdtp(&mut self, _: Address) -> Result<(), Self::Error> {
        self.push(5);
        Ok(())
    }
    fn disconnect_avdtp(&mut self, _: Address) -> Result<(), Self::Error> {
        self.push(6);
        Ok(())
    }
    fn play_test_tone(&mut self) -> Result<(), Self::Error> {
        self.push(7);
        Ok(())
    }
}
fn result() -> DiscoveredDevice {
    DiscoveredDevice {
        address: Address::new([1, 2, 3, 4, 5, 6]),
        rssi: -30,
        ..Default::default()
    }
}

#[test]
fn selection_stops_scan_before_pairing() {
    let mut c = Controller::new(Fake::default());
    c.start_scan().unwrap();
    c.discovery_result(result());
    c.select(0).unwrap();
    assert_eq!(c.model.connection, ConnectionState::WaitingForScanStop);
    assert_eq!(&c.platform().calls[..2], &[1, 2]);
    c.discovery_stopped().unwrap();
    assert_eq!(c.model.connection, ConnectionState::Pairing);
    assert_eq!(&c.platform().calls[..4], &[1, 2, 3, 4]);
    c.bond_complete(result().address, true).unwrap();
    assert_eq!(&c.platform().calls[..5], &[1, 2, 3, 4, 5]);
}

#[test]
fn bonded_peer_connects_without_create_bond() {
    let mut c = Controller::new(Fake {
        bonded: true,
        ..Default::default()
    });
    c.model.devices.begin_scan();
    c.discovery_result(result());
    c.select(0).unwrap();
    assert_eq!(c.model.connection, ConnectionState::Connecting);
    assert_eq!(&c.platform().calls[..2], &[3, 5]);
}

#[test]
fn stale_bond_callback_is_ignored() {
    let mut c = Controller::new(Fake::default());
    c.model.devices.begin_scan();
    c.discovery_result(result());
    c.select(0).unwrap();
    c.bond_complete(Address::new([9; 6]), true).unwrap();
    assert_eq!(c.platform().len, 2);
}
