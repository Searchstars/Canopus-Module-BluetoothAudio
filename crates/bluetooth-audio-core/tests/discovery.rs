use canopus_bluetooth_audio_core::{
    Address, DISCOVERY_CAPACITY, DeviceName, DiscoveredDevice, DiscoveryTable,
};

fn device(id: u8, rssi: i32, cod: u32) -> DiscoveredDevice {
    DiscoveredDevice {
        address: Address::new([0, 1, 2, 3, 4, id]),
        name: DeviceName::from_bytes(b"Headset"),
        rssi,
        class_of_device: cod,
        last_seen_epoch: 0,
    }
}

#[test]
fn deduplicates_and_refreshes_results() {
    let mut table = DiscoveryTable::default();
    let epoch = table.begin_scan();
    assert!(table.upsert(device(1, -70, 0)));
    assert!(table.upsert(device(1, -40, 0x0400)));
    assert_eq!(table.len(), 1);
    assert_eq!(table.entries()[0].rssi, -40);
    assert_eq!(table.entries()[0].last_seen_epoch, epoch);
}

#[test]
fn audio_likely_devices_rank_before_stronger_ambiguous_devices() {
    let mut table = DiscoveryTable::default();
    table.begin_scan();
    table.upsert(device(1, -20, 0));
    table.upsert(device(2, -70, 0x0400));
    assert_eq!(table.entries()[0].address, device(2, 0, 0).address);
}

#[test]
fn capacity_is_explicit_and_scan_reset_is_bounded() {
    let mut table = DiscoveryTable::default();
    table.begin_scan();
    for id in 0..DISCOVERY_CAPACITY as u8 {
        assert!(table.upsert(device(id + 1, -50, 0)));
    }
    assert!(!table.upsert(device(99, -10, 0)));
    assert_eq!(table.dropped(), 1);
    table.begin_scan();
    assert!(table.is_empty());
    assert_eq!(table.dropped(), 0);
}
