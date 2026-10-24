use canopus_bluetooth_audio_core::avdtp::{Source, State};

#[test]
fn negotiates_audio_sink_and_sbc_441_stereo() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    assert_eq!(source.connected(&mut out).unwrap(), 2);
    assert_eq!(&out[..2], &[0, 1]);
    // DISCOVER accept, transaction 0; SEID 2, audio sink, available.
    let n = source.receive(&[0x02, 0x01, 0x08, 0x08], &mut out).unwrap();
    assert_eq!(&out[..n], &[0x10, 0x0c, 0x08]);
    // GET_ALL_CAPABILITIES accept with media transport + SBC + delay reporting.
    let packet = [0x12, 0x0c, 1, 0, 7, 6, 0, 0, 0x22, 0x15, 2, 53, 8, 0];
    let n = source.receive(&packet, &mut out).unwrap();
    assert_eq!(source.state, State::Configuring);
    assert_eq!(source.selected_sbc.maximum_bitpool, 39);
    assert_eq!(out[1], 3);
    assert_eq!(n, 16);
    let n = source.receive(&[0x22, 3], &mut out).unwrap();
    assert_eq!(out[1], 6);
    assert_eq!(n, 3);
    assert_eq!(source.receive(&[0x32, 6], &mut out).unwrap(), 0);
    assert_eq!(source.state, State::Open);
}

#[test]
fn negotiates_real_peer_bitpool_39_and_emits_set_configuration() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    assert_eq!(source.connected(&mut out).unwrap(), 2);
    assert_eq!(
        source
            .receive(
                &[
                    0x02, 0x01, 0x04, 0x08, 0x08, 0x08, 0x0c, 0x08, 0x10, 0x08, 0x14, 0x08
                ],
                &mut out,
            )
            .unwrap(),
        3
    );
    assert_eq!(&out[..3], &[0x10, 0x0c, 0x04]);

    let packet = [
        0x12, 0x0c, 0x01, 0x00, 0x07, 0x06, 0x00, 0x00, 0x3f, 0xff, 0x02, 0x27, 0x08, 0x00,
    ];
    let n = source.receive(&packet, &mut out).unwrap();
    assert_eq!(source.state, State::Configuring);
    assert_eq!(source.remote_seid, 1);
    assert_eq!(source.local_seid, 1);
    assert_eq!(source.selected_sbc.frequency_channel, 0x22);
    assert_eq!(source.selected_sbc.blocks_subbands_allocation, 0x15);
    assert_eq!(source.selected_sbc.minimum_bitpool, 27);
    assert_eq!(source.selected_sbc.maximum_bitpool, 39);
    assert_eq!(n, 16);
    assert_eq!(
        &out[..n],
        &[
            0x20, 0x03, 0x04, 0x04, 0x01, 0x00, 0x07, 0x06, 0x00, 0x00, 0x22, 0x15, 27, 39, 0x08,
            0x00,
        ]
    );

    let n = source.receive(&[0x22, 0x03], &mut out).unwrap();
    assert_eq!(source.state, State::Opening);
    assert_eq!(&out[..n], &[0x30, 0x06, 0x04]);
    assert_eq!(source.receive(&[0x32, 0x06], &mut out).unwrap(), 0);
    assert_eq!(source.state, State::Open);
}

#[test]
fn records_sink_delay_and_restarts_after_suspend() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    source.connected(&mut out).unwrap();
    source.receive(&[0x02, 0x01, 0x08, 0x08], &mut out).unwrap();
    source
        .receive(
            &[0x12, 0x0c, 1, 0, 7, 6, 0, 0, 0x22, 0x15, 27, 53, 8, 0],
            &mut out,
        )
        .unwrap();
    source.receive(&[0x22, 0x03], &mut out).unwrap();
    source.receive(&[0x32, 0x06], &mut out).unwrap();
    assert_eq!(source.state, State::Open);

    let n = source
        .receive(&[0x70, 0x0d, 0x04, 0x05, 0xdc], &mut out)
        .unwrap();
    assert_eq!(&out[..n], &[0x72, 0x0d]);
    assert_eq!(source.reported_delay_100us, 1_500);

    source.media_connected = true;
    let n = source.start(&mut out).unwrap();
    let transaction = out[0] >> 4;
    assert_eq!(out[1], 0x07);
    assert!(n > 0);
    source
        .receive(&[(transaction << 4) | 0x02, 0x07], &mut out)
        .unwrap();
    assert_eq!(source.state, State::Streaming);

    source.suspend(&mut out).unwrap();
    let transaction = out[0] >> 4;
    source
        .receive(&[(transaction << 4) | 0x02, 0x09], &mut out)
        .unwrap();
    assert_eq!(source.state, State::Open);
    assert!(source.start(&mut out).is_ok());
    assert_eq!(source.state, State::Starting);
}

#[test]
fn reassembles_fragmented_capabilities() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    source.connected(&mut out).unwrap();
    source.receive(&[0x02, 0x01, 0x08, 0x08], &mut out).unwrap();

    // Transaction 1, accept, two packets. START carries signal then payload.
    assert_eq!(
        source
            .receive(&[0x16, 2, 0x0c, 1, 0, 7, 6, 0], &mut out)
            .unwrap(),
        0
    );
    let n = source
        .receive(&[0x1e, 0, 0x22, 0x15, 2, 53, 8, 0], &mut out)
        .unwrap();
    assert_eq!(source.state, State::Configuring);
    assert_eq!(source.selected_sbc.maximum_bitpool, 39);
    assert_eq!(n, 16);
}

#[test]
fn rejects_invalid_fragment_sequence_and_recovers() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    source.connected(&mut out).unwrap();
    source.receive(&[0x02, 0x01, 0x08, 0x08], &mut out).unwrap();

    source.receive(&[0x16, 3, 0x0c, 1, 0], &mut out).unwrap();
    assert!(source.receive(&[0x1e, 7, 6], &mut out).is_err());

    let packet = [0x12, 0x0c, 1, 0, 7, 6, 0, 0, 0x22, 0x15, 2, 53];
    assert!(source.receive(&packet, &mut out).is_ok());
}

#[test]
fn falls_back_to_get_capabilities_after_get_all_reject() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    source.connected(&mut out).unwrap();
    source.receive(&[0x02, 0x01, 0x08, 0x08], &mut out).unwrap();

    let n = source.receive(&[0x13, 0x0c, 0x31], &mut out).unwrap();
    assert_eq!(&out[..n], &[0x20, 0x02, 0x08]);
    let packet = [0x22, 0x02, 1, 0, 7, 6, 0, 0, 0x22, 0x15, 2, 53];
    let n = source.receive(&packet, &mut out).unwrap();
    assert_eq!(source.state, State::Configuring);
    assert_eq!(out[1], 3);
    assert_eq!(n, 14);
}

#[test]
fn rejects_remote_start_until_media_channel_is_connected() {
    let mut source = Source::new(1);
    let mut out = [0u8; 16];
    source.connected(&mut out).unwrap();
    source.state = State::Open;
    source.local_in_use = true;

    let n = source.receive(&[0x40, 0x07, 0x04], &mut out).unwrap();
    assert_eq!(&out[..n], &[0x43, 0x07, 0x04, 0x31]);
    assert_eq!(source.state, State::Open);

    source.media_connected = true;
    let n = source.receive(&[0x50, 0x07, 0x04], &mut out).unwrap();
    assert_eq!(&out[..n], &[0x52, 0x07]);
    assert_eq!(source.state, State::Streaming);
}

#[test]
fn rejects_malformed_or_non_sink_discovery() {
    let mut source = Source::new(1);
    let mut out = [0u8; 16];
    source.connected(&mut out).unwrap();
    assert!(source.receive(&[0x02, 1, 0x08], &mut out).is_err());

    let mut source = Source::new(1);
    source.connected(&mut out).unwrap();
    assert!(source.receive(&[0x02, 1, 0x08, 0x08, 0], &mut out).is_err());

    let mut source = Source::new(1);
    source.connected(&mut out).unwrap();
    assert!(source.receive(&[0x02, 1, 0x09, 0x08], &mut out).is_err());
}

#[test]
fn suspend_requires_a_live_link_and_remote_sep() {
    let mut source = Source::new(1);
    let mut out = [0u8; 16];
    source.state = State::Streaming;
    source.remote_seid = 2;
    assert!(source.suspend(&mut out).is_err());

    source.connected(&mut out).unwrap();
    source.state = State::Streaming;
    source.remote_seid = 0;
    assert!(source.suspend(&mut out).is_err());
}

#[test]
fn walks_to_the_sbc_sink_sep_when_the_first_one_is_aac() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    // DISCOVER returns sink SEID 2 (AAC only) and sink SEID 3 (SBC).
    assert_eq!(source.connected(&mut out).unwrap(), 2);
    let n = source
        .receive(&[0x02, 0x01, 0x08, 0x08, 0x0c, 0x08], &mut out)
        .unwrap();
    // GET_ALL_CAPABILITIES is issued for the first sink SEP (SEID 2).
    assert_eq!(&out[..n], &[0x10, 0x0c, 0x08]);
    // SEID 2 answers with an AAC media-codec category (codec type 2).
    let aac = [
        0x12, 0x0c, 1, 0, 7, 6, 0x02, 0, 0x80, 0x0c, 0x60, 0xff, 8, 0,
    ];
    let n = source.receive(&aac, &mut out).unwrap();
    // Negotiation advances to SEID 3 instead of failing with Unsupported.
    assert_eq!(source.state, State::ReadingCapabilities);
    assert_eq!(source.remote_seid, 3);
    assert_eq!(&out[..n], &[0x20, 0x0c, 0x0c]);
    // SEID 3 answers with a valid SBC configuration.
    let sbc = [0x22, 0x0c, 1, 0, 7, 6, 0, 0, 0x22, 0x15, 2, 53, 8, 0];
    let _ = source.receive(&sbc, &mut out).unwrap();
    assert_eq!(source.state, State::Configuring);
    assert_eq!(source.remote_seid, 3);
    assert_eq!(source.selected_sbc.maximum_bitpool, 39);
    assert_eq!(out[1], 3);
    source.receive(&[0x32, 3], &mut out).unwrap();
    assert_eq!(out[1], 6);
    assert_eq!(source.receive(&[0x42, 6], &mut out).unwrap(), 0);
    assert_eq!(source.state, State::Open);
}

#[test]
fn reports_unsupported_once_every_sink_sep_lacks_sbc() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    source.connected(&mut out).unwrap();
    source
        .receive(&[0x02, 0x01, 0x08, 0x08, 0x0c, 0x08], &mut out)
        .unwrap();
    // Both SEPs answer with the AAC codec category.
    let aac = [
        0x12, 0x0c, 1, 0, 7, 6, 0x02, 0, 0x80, 0x0c, 0x60, 0xff, 8, 0,
    ];
    let n = source.receive(&aac, &mut out).unwrap();
    assert_eq!(&out[..n], &[0x20, 0x0c, 0x0c]);
    let aac = [
        0x22, 0x0c, 1, 0, 7, 6, 0x02, 0, 0x80, 0x0c, 0x60, 0xff, 8, 0,
    ];
    assert!(matches!(
        source.receive(&aac, &mut out),
        Err(canopus_bluetooth_audio_core::avdtp::Error::Unsupported)
    ));
}

#[test]
fn falls_back_to_joint_stereo_when_stereo_is_unavailable() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    assert_eq!(source.connected(&mut out).unwrap(), 2);
    // Single sink SEP advertising 44.1 kHz + joint stereo only.
    let n = source.receive(&[0x02, 0x01, 0x08, 0x08], &mut out).unwrap();
    assert_eq!(&out[..n], &[0x10, 0x0c, 0x08]);
    let joint = [0x12, 0x0c, 1, 0, 7, 6, 0, 0, 0x21, 0x15, 2, 53, 8, 0];
    let _ = source.receive(&joint, &mut out).unwrap();
    assert_eq!(source.state, State::Configuring);
    assert_eq!(source.selected_sbc.frequency_channel, 0x21);
    assert!(source.selected_sbc.joint());
    // SET_CONFIGURATION carries the negotiated joint-stereo channel byte.
    assert_eq!(out[10], 0x21);
    assert_eq!(out[12], 27);
    assert_eq!(out[13], 39);
}

/// Drives a source through DISCOVER (AAC sink SEID 1, SBC sink SEID 2), the
/// AAC capabilities, and the full SBC capability set AirPods-class sinks
/// advertise, up to Open.
fn open_with_aac_first_sink(source: &mut Source, out: &mut [u8]) {
    source.connected(out).unwrap();
    let n = source
        .receive(&[0x02, 0x01, 0x04, 0x08, 0x08, 0x08], out)
        .unwrap();
    assert_eq!(&out[..n], &[0x10, 0x0c, 0x04]);
    // SEID 1: media transport, AAC, content protection (SCMS-T), delay reporting.
    let aac = [
        0x12, 0x0c, 1, 0, 7, 8, 0x00, 0x02, 0x80, 0x01, 0x8c, 0x84, 0xe2, 0x00, 4, 2, 2, 0, 8, 0,
    ];
    let n = source.receive(&aac, out).unwrap();
    assert_eq!(&out[..n], &[0x20, 0x0c, 0x08]);
    // SEID 2: every SBC option, bitpool 2..53, content protection, delay reporting.
    let sbc = [
        0x22, 0x0c, 1, 0, 7, 6, 0, 0, 0xff, 0xff, 2, 53, 4, 2, 2, 0, 8, 0,
    ];
    let n = source.receive(&sbc, out).unwrap();
    assert_eq!(source.state, State::Configuring);
    assert_eq!(source.remote_seid, 2);
    assert_eq!(source.selected_sbc.frequency_channel, 0x22);
    assert_eq!(source.selected_sbc.maximum_bitpool, 39);
    assert!(source.delay_reporting);
    // SET_CONFIGURATION: ACP 2, INT 1, transport, SBC 0x22/0x15/27/39, delay reporting.
    assert_eq!(
        &out[..n],
        &[
            0x30, 0x03, 0x08, 0x04, 1, 0, 7, 6, 0, 0, 0x22, 0x15, 27, 39, 8, 0
        ]
    );
    let n = source.receive(&[0x32, 0x03], out).unwrap();
    assert_eq!(&out[..n], &[0x40, 0x06, 0x08]);
    assert_eq!(source.receive(&[0x42, 0x06], out).unwrap(), 0);
    assert_eq!(source.state, State::Open);
}

#[test]
fn negotiates_sbc_behind_an_aac_first_sink() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    open_with_aac_first_sink(&mut source, &mut out);
}

#[test]
fn refuses_a_sink_initiated_configuration_without_failing() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    source.connected(&mut out).unwrap();
    // The sink discovers us and tries to configure our SEP before our DISCOVER is answered.
    let n = source.receive(&[0x50, 0x01], &mut out).unwrap();
    assert_eq!(&out[..n], &[0x52, 0x01, 0x04, 0x00]);
    let set = [0x60, 0x03, 0x04, 0x08, 1, 0, 7, 6, 0, 0, 0x21, 0x15, 2, 53];
    let n = source.receive(&set, &mut out).unwrap();
    assert_eq!(&out[..n], &[0x63, 0x03, 0x00, 0x13]);
    let n = source.receive(&[0x70, 0x06, 0x04], &mut out).unwrap();
    assert_eq!(&out[..n], &[0x73, 0x06, 0x31]);
    // Our own negotiation carries on.
    assert_eq!(source.state, State::Discovering);
    let n = source.receive(&[0x02, 0x01, 0x08, 0x08], &mut out).unwrap();
    assert_eq!(&out[..n], &[0x10, 0x0c, 0x08]);
}

#[test]
fn ignores_stale_responses() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    open_with_aac_first_sink(&mut source, &mut out);
    // A second accept for the OPEN that already completed, and an accept with a
    // transaction label nothing is waiting on.
    assert_eq!(source.receive(&[0x42, 0x06], &mut out).unwrap(), 0);
    assert_eq!(source.receive(&[0x92, 0x07], &mut out).unwrap(), 0);
    assert_eq!(
        source.receive(&[0x93, 0x07, 0x08, 0x31], &mut out).unwrap(),
        0
    );
    assert_eq!(source.state, State::Open);
    assert_eq!(source.last_reject, (0, 0));
}

#[test]
fn a_refused_start_leaves_the_stream_open() {
    let mut source = Source::new(1);
    let mut out = [0u8; 64];
    open_with_aac_first_sink(&mut source, &mut out);
    source.media_connected = true;
    source.start(&mut out).unwrap();
    let transaction = out[0] >> 4;
    assert_eq!(source.state, State::Starting);
    // START reject: ACP SEID 2, BAD_STATE.
    let n = source
        .receive(&[(transaction << 4) | 0x03, 0x07, 0x08, 0x31], &mut out)
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(source.state, State::Open);
    assert_eq!(source.last_reject, (0x07, 0x31));
    // A later START goes out normally.
    assert!(source.start(&mut out).is_ok());
}
