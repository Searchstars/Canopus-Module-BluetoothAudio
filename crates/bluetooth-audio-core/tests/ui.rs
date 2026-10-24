use canopus_bluetooth_audio_core::ui::{self, key};
use canopus_bluetooth_audio_core::{
    Address, ConnectionState, DeviceName, DiscoveredDevice, Model, PAIR_DIAG_DISPLAY,
    PAIR_DIAG_FILTER_HIT, PAIR_DIAG_MHDT_FIXED, PAIR_DIAG_MHDT_INSTALLED,
    PAIR_DIAG_REMOVE_CONFIRMED, PAIR_DIAG_REQUEST, Peer, StreamState,
};
use canopus_ui_core::{ColorRole, ComponentVariant, NodeKind, Snapshot};

fn text(snapshot: &Snapshot, key: u32) -> &str {
    snapshot.primary(snapshot.find_by_key(key).unwrap())
}

fn style(snapshot: &Snapshot, key: u32) -> canopus_ui_core::Style {
    let index = snapshot.nodes[..snapshot.node_count as usize]
        .iter()
        .position(|node| node.key == key)
        .unwrap();
    *snapshot.style(index).unwrap()
}

fn connected(name: &[u8]) -> Model {
    let peer = Peer {
        address: Address::new([0xF6, 0x6F, 0x9B, 0x84, 0x5A, 0xBE]),
        name: DeviceName::from_bytes(name),
        ..Default::default()
    };
    Model {
        selected: Some(peer),
        connected: Some(peer),
        connection: ConnectionState::Ready,
        stream: StreamState::Open,
        ..Default::default()
    }
}

#[test]
fn overview_invites_pairing_when_nothing_is_connected() {
    let snapshot = ui::overview(&Model::default()).unwrap();
    assert_eq!(text(&snapshot, key::HEADLINE), "未连接耳机");
    assert_eq!(text(&snapshot, key::STATUS), "未连接");
    assert!(snapshot.find_by_key(key::DETAIL).is_none());
    let scan = snapshot.find_by_key(key::SCAN).unwrap();
    assert_eq!(scan.kind(), Some(NodeKind::Button));
    assert!(scan.enabled());
    assert_eq!(snapshot.primary(scan), "搜索耳机");
    assert_eq!(
        style(&snapshot, key::SCAN).variant,
        ComponentVariant::Filled as u16
    );
    assert_eq!(text(&snapshot, 22), "请先让耳机进入配对模式");
    assert!(snapshot.find_by_key(key::SUMMARY).is_none());
    let icon = snapshot.find_by_key(key::ICON).unwrap();
    assert_eq!(icon.kind(), Some(NodeKind::Image));
    assert_eq!(snapshot.primary(icon), ui::ICON_RESOURCE);
}

#[test]
fn overview_shows_a_connected_headset_with_its_detail_row() {
    let snapshot = ui::overview(&connected(b"AirPods Pro")).unwrap();
    assert_eq!(text(&snapshot, key::HEADLINE), "AirPods Pro");
    assert_eq!(text(&snapshot, key::STATUS), "已连接");
    let pill = style(&snapshot, key::STATUS);
    assert_eq!(pill.variant, ComponentVariant::Tonal as u16);
    assert_eq!(pill.foreground, ColorRole::Success as u16);
    let detail = snapshot.find_by_key(key::DETAIL).unwrap();
    assert_eq!(detail.event_id, ui::EVENT_CONNECTED_DETAIL);
    assert!(snapshot.find_by_key(key::DIAGNOSTICS).is_some());
}

#[test]
fn overview_explains_an_unsupported_codec() {
    let mut model = connected(b"AirPods Pro");
    model.connection = ConnectionState::Configuring;
    model.stream = StreamState::Failed;
    model.last_error = -1107;
    let snapshot = ui::overview(&model).unwrap();
    assert_eq!(text(&snapshot, key::STATUS), "连接失败");
    assert_eq!(
        style(&snapshot, key::STATUS).foreground,
        ColorRole::Danger as u16
    );
    assert_eq!(
        text(&snapshot, key::SUMMARY),
        "耳机不支持 SBC 音频格式\n错误 -1107"
    );
}

#[test]
fn overview_lists_scan_results_with_signal_and_address_tail() {
    let mut model = Model::default();
    model.devices.begin_scan();
    model.scan = canopus_bluetooth_audio_core::ScanState::Scanning;
    let address = Address::new([1, 2, 3, 4, 0x5A, 0xBE]);
    model.devices.upsert(DiscoveredDevice {
        address,
        name: DeviceName::from_bytes(b"AirPods Pro"),
        rssi: -52,
        class_of_device: 0x240404,
        last_seen_epoch: 0,
    });
    let snapshot = ui::overview(&model).unwrap();
    assert_eq!(text(&snapshot, key::SCAN), "停止搜索");
    let row = snapshot.find_by_key(ui::device_key(address)).unwrap();
    assert_eq!(snapshot.primary(row), "AirPods Pro");
    assert_eq!(snapshot.secondary(row), "信号强");
    assert_eq!(row.event_id, ui::EVENT_DEVICE_BASE);
    assert!(row.enabled());
    assert_eq!(text(&snapshot, 22), "点按耳机即可连接");

    let twin = Address::new([1, 2, 3, 4, 0x77, 0x01]);
    model.devices.upsert(DiscoveredDevice {
        address: twin,
        name: DeviceName::from_bytes(b"AirPods Pro"),
        rssi: -70,
        class_of_device: 0x240404,
        last_seen_epoch: 0,
    });
    let snapshot = ui::overview(&model).unwrap();
    let row = |address| snapshot.secondary(snapshot.find_by_key(ui::device_key(address)).unwrap());
    assert_eq!(row(address), "信号强 · 尾号 5ABE");
    assert_eq!(row(twin), "信号中 · 尾号 7701");

    model.scan = canopus_bluetooth_audio_core::ScanState::Idle;
    model.connection = ConnectionState::Pairing;
    let snapshot = ui::overview(&model).unwrap();
    assert!(
        !snapshot
            .find_by_key(ui::device_key(address))
            .unwrap()
            .enabled()
    );
    let scan = snapshot.find_by_key(key::SCAN).unwrap();
    assert!(!scan.enabled());
    assert_eq!(
        style(&snapshot, key::SCAN).variant,
        ComponentVariant::Default as u16
    );
    assert!(snapshot.find_by_key(22).is_none());
}

#[test]
fn overview_renders_all_retained_results_within_capacity() {
    let mut model = connected(&[b'X'; 64]);
    model.connection = ConnectionState::Failed;
    model.last_error = -1105;
    model.devices.begin_scan();
    for i in 0..12 {
        model.devices.upsert(DiscoveredDevice {
            address: Address::new([0, 0, 0, 0, 0, i + 1]),
            name: DeviceName::from_bytes(&[b'Y'; 64]),
            rssi: -80,
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
fn detail_describes_the_negotiated_stream() {
    let mut model = connected(b"AirPods Pro");
    model.details.bitpool = 39;
    model.details.sbc_frequency_channel = 0x22;
    model.details.avrcp_state = 2;
    model.details.avrcp_volume = 0x7f;
    let snapshot = ui::detail(&model).unwrap();
    let row = |key| snapshot.secondary(snapshot.find_by_key(key).unwrap());
    assert_eq!(row(11), "SBC 44.1 kHz · 立体声");
    assert_eq!(row(12), "248 kbps");
    assert_eq!(row(13), "100%");
    assert_eq!(row(14), "F6:6F:9B:84:5A:BE");
    let disconnect = snapshot.find_by_key(key::DISCONNECT).unwrap();
    assert!(disconnect.enabled());
    assert_eq!(
        style(&snapshot, key::DISCONNECT).variant,
        ComponentVariant::Destructive as u16
    );

    model.details.sbc_frequency_channel = 0x21;
    model.details.avrcp_state = 0;
    let snapshot = ui::detail(&model).unwrap();
    let row = |key| snapshot.secondary(snapshot.find_by_key(key).unwrap());
    assert_eq!(row(11), "SBC 44.1 kHz · 联合立体声");
    assert_eq!(row(12), "250 kbps");
    assert_eq!(row(13), "在耳机上调节");
}

#[test]
fn detail_lets_a_failed_audio_setup_be_disconnected() {
    let mut model = connected(b"AirPods Pro");
    model.connection = ConnectionState::Configuring;
    model.stream = StreamState::Failed;
    let snapshot = ui::detail(&model).unwrap();
    assert!(snapshot.find_by_key(key::DISCONNECT).unwrap().enabled());
}

#[test]
fn diagnostics_keep_their_topology_while_counters_move() {
    let mut model = connected(b"AirPods Pro");
    let before = ui::diagnostics(&model).unwrap();

    let d = &mut model.details;
    d.audio_state = 4;
    d.audio_stage = 7;
    d.decoded_sample_rate = 24_000;
    d.decoded_channels = 2;
    d.input_used = 3072;
    d.pcm_frames = 1058;
    d.audio_rtp_packets = 2;
    d.media_packets_queued = 5;
    d.media_flow_events = 3;
    d.media_tx_outstanding = 2;
    d.media_flow_reclaims = 1;
    d.startup_silence_packets = 3;
    d.underruns = 1;
    d.audio_error = -1206;
    d.avdtp_reject = 0x0731;
    d.remote_seid = 2;
    d.bitpool = 39;
    d.sbc_frequency_channel = 0x22;
    d.avrcp_state = 2;
    d.avrcp_cid = 65;
    d.avrcp_mtu = 512;
    d.avrcp_volume = 96;
    d.avrcp_packets_sent = 3;
    d.avrcp_packets_received = 2;
    d.avrcp_last_event = 7;
    d.avrcp_rx_header = 0x0048_0E12;
    d.avrcp_rx_length = 14;
    d.avrcp_error = -1303;
    model.last_error = -1105;
    let after = ui::diagnostics(&model).unwrap();

    assert_eq!(before.node_count, after.node_count);
    for index in 0..before.node_count as usize {
        assert_eq!(before.nodes[index].key, after.nodes[index].key);
        assert_eq!(before.nodes[index].kind(), after.nodes[index].kind());
    }
    let row = |key| after.secondary(after.find_by_key(key).unwrap());
    assert_eq!(text(&after, 2), "C7S5 E-1105 A07/31 B39 Q5/3/2/1 U1");
    assert_eq!(row(13), "SEID 2 · 22 bp39 · rej 07/31");
    assert_eq!(row(14), "sig 0 · media 0 · ctl 512");
    assert_eq!(row(15), "Connected c65 v96 3/2 ev7 e-1303 h00480E12/14");
    assert_eq!(row(51), "RTP sent");
    assert_eq!(row(52), "Playing / 3072 B");
    assert_eq!(row(53), "24000 Hz / 2 ch");
    assert_eq!(row(54), "PCM 1058 · RTP 2");
    assert_eq!(row(55), "q5 c3 o2 r1 s3");
    assert_eq!(row(56), "1");
    assert_eq!(row(57), "-1206");

    model.details.pacing_lag_ms = u32::MAX;
    let late = ui::diagnostics(&model).unwrap();
    assert_eq!(late.node_count, after.node_count);
    assert_eq!(
        late.secondary(late.find_by_key(56).unwrap()),
        "1 / late 4294967295 ms"
    );
}

#[test]
fn diagnostics_show_the_codec_bytes_of_an_unsupported_sink() {
    let mut model = connected(b"AirPods Pro");
    model.details.sbc_caps = [0x00, 0x02, 0x80, 0x01, 0x8c, 0x84];
    let snapshot = ui::diagnostics(&model).unwrap();
    assert_eq!(
        snapshot.secondary(snapshot.find_by_key(13).unwrap()),
        "caps 00 02 80 01 8c 84"
    );
}

#[test]
fn diagnostics_keep_the_pairing_milestones() {
    let mut model = connected(b"AirPods Pro");
    model.details.stock_bond_state = 3;
    model.details.device_bond_state = 2;
    model.details.pairing_flags = PAIR_DIAG_REMOVE_CONFIRMED
        | PAIR_DIAG_FILTER_HIT
        | PAIR_DIAG_MHDT_INSTALLED
        | PAIR_DIAG_MHDT_FIXED
        | PAIR_DIAG_REQUEST
        | PAIR_DIAG_DISPLAY;
    model.details.audio_probe_result = -25;
    model.details.audio_last_command = 0x304;
    let snapshot = ui::diagnostics(&model).unwrap();
    let row = |key| snapshot.secondary(snapshot.find_by_key(key).unwrap());
    assert_eq!(row(16), "Bond 3/2 · E7");
    assert_eq!(row(17), "reg 0 probe -25 abi 0 cmd 0x304");
}

#[test]
fn pages_handle_maximum_peer_text_and_counters() {
    let mut model = connected(&[b'X'; 64]);
    let d = &mut model.details;
    d.avrcp_state = 4;
    d.avrcp_cid = u16::MAX;
    d.avrcp_mtu = u16::MAX;
    d.avrcp_volume = 127;
    d.avrcp_packets_sent = u32::MAX;
    d.avrcp_packets_received = u32::MAX;
    d.avrcp_last_event = u32::MAX;
    d.avrcp_rx_header = u32::MAX;
    d.avrcp_rx_length = u16::MAX;
    d.avrcp_error = i32::MIN;
    d.media_packets_queued = u32::MAX;
    d.media_flow_events = u32::MAX;
    d.media_flow_reclaims = u32::MAX;
    d.avdtp_reject = u16::MAX;
    model.last_error = i32::MIN;
    model.stream = StreamState::Failed;
    assert!(ui::overview(&model).is_ok());
    assert!(ui::detail(&model).is_ok());
    assert!(ui::diagnostics(&model).is_ok());
}

#[cfg(not(feature = "production"))]
#[test]
fn detail_enables_audio_tests_only_when_stream_ready() {
    let mut model = connected(b"Headset");
    model.connection = ConnectionState::Configuring;
    let first = ui::detail(&model).unwrap();
    assert!(!first.find_by_key(key::TEST_TONE).unwrap().enabled());
    assert!(!first.find_by_key(key::LONG_MP3).unwrap().enabled());
    assert!(
        !first
            .find_by_key(key::LONG_MP3_DECODE_ONLY)
            .unwrap()
            .enabled()
    );
    model.connection = ConnectionState::Ready;
    let ready = ui::detail(&model).unwrap();
    assert!(ready.find_by_key(key::TEST_TONE).unwrap().enabled());
    assert!(ready.find_by_key(key::LONG_MP3).unwrap().enabled());
    assert!(
        ready
            .find_by_key(key::LONG_MP3_DECODE_ONLY)
            .unwrap()
            .enabled()
    );
}

#[cfg(feature = "production")]
#[test]
fn detail_omits_audio_test_actions_in_production() {
    let snapshot = ui::detail(&connected(b"Headset")).unwrap();
    assert!(snapshot.find_by_key(key::TEST_TONE).is_none());
    assert!(snapshot.find_by_key(key::LONG_MP3).is_none());
    assert!(snapshot.find_by_key(key::LONG_MP3_DECODE_ONLY).is_none());
    assert!(snapshot.find_by_key(key::DISCONNECT).is_some());
}
