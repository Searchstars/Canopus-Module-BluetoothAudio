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
    assert_eq!(source.selected_sbc.maximum_bitpool, 53);
    assert_eq!(out[1], 3);
    assert_eq!(n, 16);
    let n = source.receive(&[0x22, 3], &mut out).unwrap();
    assert_eq!(out[1], 6);
    assert_eq!(n, 3);
    assert_eq!(source.receive(&[0x32, 6], &mut out).unwrap(), 0);
    assert_eq!(source.state, State::Open);
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
    assert_eq!(source.selected_sbc.maximum_bitpool, 53);
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
fn rejects_malformed_or_non_sink_discovery() {
    let mut source = Source::new(1);
    let mut out = [0u8; 16];
    source.connected(&mut out).unwrap();
    assert!(source.receive(&[0x02, 1, 0x08], &mut out).is_err());
}
