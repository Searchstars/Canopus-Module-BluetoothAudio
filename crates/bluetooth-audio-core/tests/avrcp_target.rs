use canopus_bluetooth_audio_core::avrcp::{Controller, Event, MediaControl, State};

#[test]
fn target_reports_volume_capability_and_accepts_peer_volume() {
    let mut target = Controller::new();
    let mut out = [0u8; 32];
    target.target_connected(64).unwrap();

    let event = target
        .receive(
            &[
                0x20, 0x11, 0x0e, 0x01, 0x48, 0x00, 0x00, 0x19, 0x58, 0x10, 0, 0, 1, 0x03,
            ],
            &mut out,
        )
        .unwrap();
    assert_eq!(event, Event::PeerCommand(16));
    assert_eq!(
        &out[..16],
        &[
            0x22, 0x11, 0x0e, 0x0c, 0x48, 0x00, 0x00, 0x19, 0x58, 0x10, 0, 0, 3, 0x03, 1, 0x0d,
        ]
    );

    let event = target
        .receive(
            &[
                0x30, 0x11, 0x0e, 0x00, 0x48, 0x00, 0x00, 0x19, 0x58, 0x50, 0, 0, 1, 80,
            ],
            &mut out,
        )
        .unwrap();
    assert_eq!(
        event,
        Event::PeerVolume {
            volume: 80,
            response_len: 14,
        }
    );
    assert_eq!(out[0], 0x32);
    assert_eq!(out[3], 0x09);
    assert_eq!(out[13], 80);
    assert_eq!(target.volume, 80);
    assert_eq!(target.state, State::Ready);
}

#[test]
fn target_accepts_pass_through_press_and_ignores_release() {
    let mut target = Controller::new();
    let mut out = [0u8; 32];
    target.target_connected(64).unwrap();

    for (operation, control) in [
        (0x44, MediaControl::Play),
        (0x46, MediaControl::Pause),
        (0x4b, MediaControl::Next),
        (0x4c, MediaControl::Previous),
    ] {
        let packet = [0x50, 0x11, 0x0e, 0x00, 0x48, 0x7c, operation, 0];
        assert_eq!(
            target.receive(&packet, &mut out).unwrap(),
            Event::PeerControl {
                control,
                response_len: packet.len(),
            }
        );
        assert_eq!(
            &out[..packet.len()],
            &[0x52, 0x11, 0x0e, 0x09, 0x48, 0x7c, operation, 0]
        );

        let release = [0x50, 0x11, 0x0e, 0x00, 0x48, 0x7c, operation | 0x80, 0];
        assert_eq!(
            target.receive(&release, &mut out).unwrap(),
            Event::PeerCommand(release.len())
        );
        assert_eq!(out[3], 0x09);
    }
}

#[test]
fn target_rejects_unknown_and_malformed_pass_through_without_state_loss() {
    let mut target = Controller::new();
    let mut out = [0u8; 32];
    target.target_connected(64).unwrap();

    let unknown = [0x60, 0x11, 0x0e, 0x00, 0x48, 0x7c, 0x01, 0];
    assert_eq!(
        target.receive(&unknown, &mut out).unwrap(),
        Event::PeerCommand(unknown.len())
    );
    assert_eq!(out[3], 0x08);
    assert_eq!(target.state, State::Ready);

    let truncated = [0x60, 0x11, 0x0e, 0x00, 0x48, 0x7c, 0x44];
    assert_eq!(
        target.receive(&truncated, &mut out),
        Err(canopus_bluetooth_audio_core::avrcp::Error::Packet)
    );
    assert_eq!(target.state, State::Ready);
}

#[test]
fn target_interim_and_changed_preserve_peer_transaction() {
    let mut target = Controller::new();
    let mut out = [0u8; 32];
    target.target_connected(64).unwrap();

    let event = target
        .receive(
            &[
                0x40, 0x11, 0x0e, 0x03, 0x48, 0x00, 0x00, 0x19, 0x58, 0x31, 0, 0, 5, 0x0d, 0, 0, 0,
                0,
            ],
            &mut out,
        )
        .unwrap();
    assert_eq!(event, Event::PeerCommand(15));
    assert_eq!(out[0], 0x42);
    assert_eq!(out[3], 0x0f);
    assert_eq!(&out[13..15], &[0x0d, 64]);

    let len = target.target_volume_changed(72, &mut out).unwrap();
    assert_eq!(len, 15);
    assert_eq!(out[0], 0x42);
    assert_eq!(out[3], 0x0d);
    assert_eq!(&out[13..15], &[0x0d, 72]);
    assert_eq!(target.target_volume_changed(73, &mut out).unwrap(), 0);
}
