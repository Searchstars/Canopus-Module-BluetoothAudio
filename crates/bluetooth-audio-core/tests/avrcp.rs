use canopus_bluetooth_audio_core::avrcp::{
    Controller, DEFAULT_VOLUME, Event, State, absolute_to_percent, percent_to_absolute,
};

#[test]
fn sets_volume_then_registers_and_reregisters_notification() {
    let mut controller = Controller::new();
    let mut out = [0u8; 32];
    let n = controller.connected(DEFAULT_VOLUME, &mut out).unwrap();
    assert_eq!(DEFAULT_VOLUME, 64);
    assert_eq!(controller.state, State::WaitingSetVolume);
    assert_eq!(
        &out[..n],
        &[
            0x00, 0x11, 0x0e, 0x00, 0x48, 0x00, 0x00, 0x19, 0x58, 0x50, 0, 0, 1, 64
        ]
    );

    assert_eq!(
        controller
            .receive(
                &[
                    0x02, 0x11, 0x0e, 0x09, 0x48, 0x00, 0, 0x19, 0x58, 0x50, 0, 0, 1, 127,
                ],
                &mut out
            )
            .unwrap(),
        Event::Volume(127)
    );
    let n = controller.register_volume_notification(&mut out).unwrap();
    assert_eq!(
        &out[..n],
        &[
            0x10, 0x11, 0x0e, 0x03, 0x48, 0x00, 0, 0x19, 0x58, 0x31, 0, 0, 5, 0x0d, 0, 0, 0, 0
        ]
    );
    assert_eq!(
        controller
            .receive(
                &[
                    0x12, 0x11, 0x0e, 0x0f, 0x48, 0x00, 0, 0x19, 0x58, 0x31, 0, 0, 2, 0x0d, 127,
                ],
                &mut out
            )
            .unwrap(),
        Event::Volume(127)
    );
    assert_eq!(controller.state, State::Registered);
    assert_eq!(
        controller
            .receive(
                &[
                    0x12, 0x11, 0x0e, 0x0d, 0x48, 0x00, 0, 0x19, 0x58, 0x31, 0, 0, 2, 0x0d, 80,
                ],
                &mut out
            )
            .unwrap(),
        Event::Reregister
    );
    assert_eq!(controller.volume, 80);
    assert_eq!(controller.state, State::Ready);
}

#[test]
fn tracks_control_and_notification_transactions_independently() {
    let mut controller = Controller::new();
    let mut out = [0u8; 32];
    controller.connected(100, &mut out).unwrap();
    controller
        .receive(
            &[
                0x02, 0x11, 0x0e, 0x09, 0x48, 0x00, 0, 0x19, 0x58, 0x50, 0, 0, 1, 100,
            ],
            &mut out,
        )
        .unwrap();
    controller.register_volume_notification(&mut out).unwrap();
    controller
        .receive(
            &[
                0x12, 0x11, 0x0e, 0x0f, 0x48, 0x00, 0, 0x19, 0x58, 0x31, 0, 0, 2, 0x0d, 100,
            ],
            &mut out,
        )
        .unwrap();

    let n = controller.set_absolute_volume(90, &mut out).unwrap();
    assert_eq!(out[0], 0x20);
    assert_eq!(n, 14);
    assert_eq!(controller.state, State::WaitingSetVolume);
    assert_eq!(
        controller
            .receive(
                &[
                    0x22, 0x11, 0x0e, 0x09, 0x48, 0x00, 0, 0x19, 0x58, 0x50, 0, 0, 1, 90,
                ],
                &mut out
            )
            .unwrap(),
        Event::Volume(90)
    );
    assert_eq!(controller.state, State::Registered);
    assert_eq!(
        controller
            .receive(
                &[
                    0x12, 0x11, 0x0e, 0x0d, 0x48, 0x00, 0, 0x19, 0x58, 0x31, 0, 0, 2, 0x0d, 70,
                ],
                &mut out
            )
            .unwrap(),
        Event::Reregister
    );
}

#[test]
fn rejects_changed_with_the_wrong_transaction_label() {
    let mut controller = Controller::new();
    let mut out = [0u8; 32];
    controller.connected(100, &mut out).unwrap();
    controller
        .receive(
            &[
                0x02, 0x11, 0x0e, 0x09, 0x48, 0x00, 0, 0x19, 0x58, 0x50, 0, 0, 1, 100,
            ],
            &mut out,
        )
        .unwrap();
    controller.register_volume_notification(&mut out).unwrap();
    controller
        .receive(
            &[
                0x12, 0x11, 0x0e, 0x0f, 0x48, 0x00, 0, 0x19, 0x58, 0x31, 0, 0, 2, 0x0d, 100,
            ],
            &mut out,
        )
        .unwrap();
    assert_eq!(
        controller
            .receive(
                &[
                    0x22, 0x11, 0x0e, 0x0d, 0x48, 0x00, 0, 0x19, 0x58, 0x31, 0, 0, 2, 0x0d, 80,
                ],
                &mut out
            )
            .unwrap(),
        Event::None
    );
    assert_eq!(controller.volume, 100);
    assert_eq!(controller.state, State::Registered);
}

#[test]
fn rejects_unsupported_peer_commands_without_failing_controller_transactions() {
    let mut controller = Controller::new();
    let mut out = [0u8; 32];
    controller.connected(127, &mut out).unwrap();

    // Transaction 2, AVCTP command, AVRCP profile, 14-byte AV/C status frame.
    // Android sends this independently of our outstanding SetAbsoluteVolume.
    assert_eq!(
        controller
            .receive(
                &[
                    0x20, 0x11, 0x0e, 0x01, 0x48, 0x00, 0x00, 0x19, 0x58, 0x10, 0, 0, 1, 0,
                ],
                &mut out
            )
            .unwrap(),
        Event::PeerCommand(14)
    );
    assert_eq!(
        &out[..14],
        &[
            0x22, 0x11, 0x0e, 0x0a, 0x48, 0x00, 0x00, 0x19, 0x58, 0x10, 0, 0, 1, 0
        ]
    );
    assert_eq!(controller.state, State::WaitingSetVolume);

    assert_eq!(
        controller
            .receive(
                &[
                    0x02, 0x11, 0x0e, 0x09, 0x48, 0x00, 0, 0x19, 0x58, 0x50, 0, 0, 1, 127,
                ],
                &mut out
            )
            .unwrap(),
        Event::Volume(127)
    );
}

#[test]
fn dual_role_answers_valid_peer_get_capabilities_without_disturbing_controller_transactions() {
    let mut controller = Controller::new();
    let mut out = [0u8; 32];
    // Controller: SetAbsoluteVolume(transaction 0) + RegisterNotification(txn 1).
    let n = controller.connected(64, &mut out).unwrap();
    assert_eq!(n, 14);
    controller.register_volume_notification(&mut out).unwrap();
    assert_eq!(controller.state, State::WaitingSetVolume);

    // Target: a peer sends a VALID GetCapabilities(events_supported) command on
    // its own transaction label 2 while both of our commands are outstanding.
    assert_eq!(
        controller
            .receive(
                &[
                    0x20, 0x11, 0x0e, 0x01, 0x48, 0x00, 0x00, 0x19, 0x58, 0x10, 0, 0, 1, 0x03,
                ],
                &mut out
            )
            .unwrap(),
        Event::PeerCommand(16)
    );
    assert_eq!(
        &out[..16],
        &[
            0x22, 0x11, 0x0e, 0x0c, 0x48, 0x00, 0x00, 0x19, 0x58, 0x10, 0, 0, 3, 0x03, 1, 0x0d,
        ]
    );
    // The peer command must not have moved our own transactions or state.
    assert_eq!(controller.state, State::WaitingSetVolume);

    // Controller responses still match their own transactions afterward.
    assert_eq!(
        controller
            .receive(
                &[
                    0x02, 0x11, 0x0e, 0x09, 0x48, 0x00, 0, 0x19, 0x58, 0x50, 0, 0, 1, 64,
                ],
                &mut out
            )
            .unwrap(),
        Event::Volume(64)
    );
    assert_eq!(
        controller
            .receive(
                &[
                    0x12, 0x11, 0x0e, 0x0f, 0x48, 0x00, 0, 0x19, 0x58, 0x31, 0, 0, 2, 0x0d, 64,
                ],
                &mut out
            )
            .unwrap(),
        Event::Volume(64)
    );
    assert_eq!(controller.state, State::Registered);
}

#[test]
fn supersedes_unanswered_volume_and_ignores_late_response() {
    let mut controller = Controller::new();
    let mut out = [0u8; 32];
    controller.connected(64, &mut out).unwrap();

    // The second slider update supersedes transaction 0 instead of returning
    // State forever when the peer delays or drops the first response.
    controller.set_absolute_volume(80, &mut out).unwrap();
    assert_eq!(out[0], 0x10);
    assert_eq!(controller.volume, 80);

    assert_eq!(
        controller
            .receive(
                &[
                    0x02, 0x11, 0x0e, 0x09, 0x48, 0x00, 0, 0x19, 0x58, 0x50, 0, 0, 1, 64,
                ],
                &mut out,
            )
            .unwrap(),
        Event::None
    );
    assert_eq!(controller.volume, 80);
    assert_eq!(
        controller
            .receive(
                &[
                    0x12, 0x11, 0x0e, 0x09, 0x48, 0x00, 0, 0x19, 0x58, 0x50, 0, 0, 1, 80,
                ],
                &mut out,
            )
            .unwrap(),
        Event::Volume(80)
    );
}

#[test]
fn rejected_control_transaction_recovers_without_failing_channel() {
    let mut controller = Controller::new();
    let mut out = [0u8; 32];
    controller.connected(64, &mut out).unwrap();
    assert!(
        controller
            .receive(
                &[
                    0x02, 0x11, 0x0e, 0x0a, 0x48, 0x00, 0, 0x19, 0x58, 0x50, 0, 0, 0,
                ],
                &mut out,
            )
            .is_err()
    );
    assert_eq!(controller.state, State::Ready);
    controller.set_absolute_volume(70, &mut out).unwrap();
}

#[test]
fn maps_percent_and_absolute_volume_with_endpoints() {
    assert_eq!(percent_to_absolute(0), 0);
    assert_eq!(percent_to_absolute(100), 127);
    assert_eq!(percent_to_absolute(200), 127);
    assert_eq!(absolute_to_percent(0), 0);
    assert_eq!(absolute_to_percent(127), 100);
    assert_eq!(absolute_to_percent(100), 79);
}
