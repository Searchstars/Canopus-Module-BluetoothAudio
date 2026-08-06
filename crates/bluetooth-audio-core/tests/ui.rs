use canopus_bluetooth_audio_core::ui;
use canopus_bluetooth_audio_core::{
    Address, ConnectionState, DeviceName, DiscoveredDevice, Model, StreamState,
};

#[test]
fn overview_always_has_connected_headset_row() {
    let model = Model::default();
    let snapshot = ui::overview(&model).unwrap();
    let row = snapshot.find_by_key(11).unwrap();
    assert_eq!(snapshot.primary(row), "Headset");
    assert_eq!(snapshot.secondary(row), "Not connected");
}
#[test]
fn overview_renders_all_retained_results_within_capacity() {
    let mut model = Model::default();
    model.devices.begin_scan();
    for i in 0..12 {
        model.devices.upsert(DiscoveredDevice {
            address: Address::new([0, 0, 0, 0, 0, i + 1]),
            name: DeviceName::from_bytes(b"Audio"),
            rssi: -30,
            class_of_device: 0x400,
            last_seen_epoch: 0,
        });
    }
    let snapshot = ui::overview(&model).unwrap();
    assert!(snapshot.node_count as usize <= canopus_ui_core::MAX_NODES);
    assert!(snapshot.string_used as usize <= canopus_ui_core::STRING_CAPACITY);
}
#[test]
fn detail_enables_tone_only_when_stream_ready() {
    let mut model = Model {
        connected: Some(Default::default()),
        ..Default::default()
    };
    let first = ui::detail(&model).unwrap();
    assert!(!first.find_by_key(34).unwrap().enabled());
    model.connection = ConnectionState::Ready;
    model.stream = StreamState::Open;
    let ready = ui::detail(&model).unwrap();
    assert!(ready.find_by_key(34).unwrap().enabled());
}
