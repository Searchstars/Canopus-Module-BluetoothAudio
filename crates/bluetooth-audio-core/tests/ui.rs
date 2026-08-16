use canopus_bluetooth_audio_core::ui;
use canopus_bluetooth_audio_core::{
    Address, ConnectionState, DeviceName, DiscoveredDevice, Model, PAIR_DIAG_DISPLAY,
    PAIR_DIAG_FILTER_HIT, PAIR_DIAG_MHDT_FIXED, PAIR_DIAG_REMOVE_CONFIRMED, PAIR_DIAG_REQUEST,
    StreamState,
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
fn device_rows_use_stable_address_keys() {
    let first = Address::new([1, 2, 3, 4, 5, 6]);
    let second = Address::new([1, 2, 3, 4, 5, 7]);
    assert_eq!(ui::device_key(first), ui::device_key(first));
    assert_ne!(ui::device_key(first), ui::device_key(second));
}

#[test]
fn overview_exposes_pairing_milestones_without_extra_rows() {
    let mut model = Model {
        selected: Some(Default::default()),
        ..Default::default()
    };
    model.details.stock_bond_state = 3;
    model.details.device_bond_state = 2;
    model.details.pairing_flags = PAIR_DIAG_REMOVE_CONFIRMED
        | PAIR_DIAG_FILTER_HIT
        | PAIR_DIAG_MHDT_FIXED
        | PAIR_DIAG_REQUEST
        | PAIR_DIAG_DISPLAY;
    let snapshot = ui::overview(&model).unwrap();
    let diagnostic = snapshot.find_by_key(13).unwrap();
    assert_eq!(snapshot.primary(diagnostic), "Bond 3/2 · 67");
}

#[test]
fn detail_keeps_audio_diagnostics_topology_stable() {
    let mut model = Model {
        connected: Some(Default::default()),
        connection: ConnectionState::Ready,
        stream: StreamState::Open,
        ..Default::default()
    };
    let before = ui::detail(&model).unwrap();

    model.details.audio_state = 4;
    model.details.audio_stage = 7;
    model.details.decoded_sample_rate = 24_000;
    model.details.decoded_channels = 2;
    model.details.input_used = 3072;
    model.details.pcm_frames = 1058;
    model.details.audio_rtp_packets = 2;
    model.details.media_packets_queued = 5;
    model.details.media_flow_events = 3;
    model.details.media_tx_outstanding = 2;
    model.details.startup_silence_packets = 3;
    model.details.underruns = 1;
    model.details.audio_error = -1206;
    model.details.avrcp_state = 2;
    model.details.avrcp_cid = 65;
    model.details.avrcp_mtu = 512;
    model.details.avrcp_volume = 96;
    model.details.avrcp_packets_sent = 3;
    model.details.avrcp_packets_received = 2;
    model.details.avrcp_last_event = 7;
    model.details.avrcp_rx_header = 0x0048_0E12;
    model.details.avrcp_rx_length = 14;
    model.details.avrcp_error = -1303;
    let after = ui::detail(&model).unwrap();

    assert_eq!(before.node_count, after.node_count);
    for index in 0..before.node_count as usize {
        assert_eq!(before.nodes[index].key, after.nodes[index].key);
        assert_eq!(before.nodes[index].kind(), after.nodes[index].kind());
    }
    assert_eq!(after.secondary(after.find_by_key(37).unwrap()), "RTP sent");
    assert_eq!(
        after.secondary(after.find_by_key(38).unwrap()),
        "Playing / 3072 B"
    );
    assert_eq!(
        after.secondary(after.find_by_key(39).unwrap()),
        "24000 Hz / 2 ch"
    );
    assert_eq!(after.secondary(after.find_by_key(40).unwrap()), "1058");
    assert_eq!(after.secondary(after.find_by_key(41).unwrap()), "2");
    assert_eq!(
        after.secondary(after.find_by_key(49).unwrap()),
        "Connected c65 v96 3/2 ev7 e-1303 h00480E12/14"
    );
    assert_eq!(after.secondary(after.find_by_key(42).unwrap()), "1");
    assert_eq!(after.secondary(after.find_by_key(43).unwrap()), "-1206");
    assert_eq!(
        after.secondary(after.find_by_key(31).unwrap()),
        "media 0 / ctl 512"
    );
}

#[test]
fn detail_handles_maximum_peer_text_with_avrcp_diagnostics() {
    let name = DeviceName::from_bytes(&[b'X'; 64]);
    let mut model = Model {
        connected: Some(canopus_bluetooth_audio_core::Peer {
            name,
            ..Default::default()
        }),
        connection: ConnectionState::Ready,
        stream: StreamState::Open,
        ..Default::default()
    };
    model.details.avrcp_state = 4;
    model.details.avrcp_cid = u16::MAX;
    model.details.avrcp_mtu = u16::MAX;
    model.details.avrcp_volume = 127;
    model.details.avrcp_packets_sent = u32::MAX;
    model.details.avrcp_packets_received = u32::MAX;
    model.details.avrcp_last_event = u32::MAX;
    model.details.avrcp_rx_header = u32::MAX;
    model.details.avrcp_rx_length = u16::MAX;
    model.details.avrcp_error = i32::MIN;
    assert!(ui::detail(&model).is_ok());
}

#[cfg(not(feature = "production"))]
#[test]
fn detail_enables_audio_tests_only_when_stream_ready() {
    let mut model = Model {
        connected: Some(Default::default()),
        ..Default::default()
    };
    let first = ui::detail(&model).unwrap();
    assert!(!first.find_by_key(34).unwrap().enabled());
    assert!(!first.find_by_key(35).unwrap().enabled());
    assert!(!first.find_by_key(44).unwrap().enabled());
    model.connection = ConnectionState::Ready;
    model.stream = StreamState::Open;
    let ready = ui::detail(&model).unwrap();
    assert!(ready.find_by_key(34).unwrap().enabled());
    assert!(ready.find_by_key(35).unwrap().enabled());
    assert!(ready.find_by_key(44).unwrap().enabled());
}

#[cfg(feature = "production")]
#[test]
fn detail_omits_audio_test_actions_in_production() {
    let model = Model {
        connected: Some(Default::default()),
        connection: ConnectionState::Ready,
        stream: StreamState::Open,
        ..Default::default()
    };
    let snapshot = ui::detail(&model).unwrap();
    assert!(snapshot.find_by_key(34).is_none());
    assert!(snapshot.find_by_key(35).is_none());
    assert!(snapshot.find_by_key(44).is_none());
    assert!(snapshot.find_by_key(36).is_some());
}
