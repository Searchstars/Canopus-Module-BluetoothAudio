use canopus_bluetooth_audio_core::compatibility::{
    strip_l2cap_mhdt_option, strip_l2cap_mhdt_option_any_cid,
};

#[test]
fn strips_bes_mhdt_from_matching_configuration_request() {
    let original = [
        0x01, 0x20, 0x13, 0x00, 0x0f, 0x00, 0x01, 0x00, 0x04, 0x01, 0x0b, 0x00, 0x41, 0x00, 0x00,
        0x00, 0x01, 0x02, 0x00, 0x04, 0x7f, 0x01, 0x01,
    ];

    let mut wrong_cid = original;
    assert_eq!(strip_l2cap_mhdt_option(&mut wrong_cid, 0x0042), None);
    assert_eq!(wrong_cid, original);

    let mut packet = original;
    assert_eq!(strip_l2cap_mhdt_option(&mut packet, 0x0041), Some(20));
    assert_eq!(&packet[2..4], &[0x10, 0x00]);
    assert_eq!(&packet[4..6], &[0x0c, 0x00]);
    assert_eq!(&packet[10..12], &[0x08, 0x00]);
    assert_eq!(&packet[20..23], &[0, 0, 0]);
}

#[test]
fn ignores_acl_continuations_and_malformed_lengths() {
    let mut continuation = [
        0x01, 0x10, 0x13, 0x00, 0x0f, 0x00, 0x01, 0x00, 0x04, 0x01, 0x0b, 0x00, 0x41, 0x00, 0x00,
        0x00, 0x01, 0x02, 0x00, 0x04, 0x7f, 0x01, 0x01,
    ];
    assert_eq!(strip_l2cap_mhdt_option(&mut continuation, 0x0041), None);

    let mut truncated = continuation;
    truncated[1] = 0x20;
    truncated[2] = 0xff;
    assert_eq!(strip_l2cap_mhdt_option(&mut truncated, 0x0041), None);
}

#[test]
fn target_neutral_hook_filters_without_runtime_cid_state() {
    let original = [
        0x81, 0x20, 0x13, 0x00, 0x0f, 0x00, 0x01, 0x00, 0x04, 0x25, 0x0b, 0x00, 0x77, 0x00, 0x00,
        0x00, 0x01, 0x02, 0x04, 0x0b, 0x7f, 0x01, 0x01,
    ];

    let mut packet = original;
    assert_eq!(strip_l2cap_mhdt_option_any_cid(&mut packet), Some(20));
    assert_eq!(&packet[2..4], &[0x10, 0x00]);
    assert_eq!(&packet[4..6], &[0x0c, 0x00]);
    assert_eq!(&packet[10..12], &[0x08, 0x00]);
    assert_eq!(&packet[16..20], &[0x01, 0x02, 0x04, 0x0b]);
    assert_eq!(&packet[20..23], &[0, 0, 0]);

    let mut fixed_channel = original;
    fixed_channel[12] = 0x3f;
    fixed_channel[13] = 0;
    let fixed_original = fixed_channel;
    assert_eq!(strip_l2cap_mhdt_option_any_cid(&mut fixed_channel), None);
    assert_eq!(fixed_channel, fixed_original);
}

#[test]
fn portable_transform_is_byte_exact_with_pre_migration_logic() {
    let original = [
        0x81, 0x20, 0x13, 0x00, 0x0f, 0x00, 0x01, 0x00, 0x04, 0x25, 0x0b, 0x00, 0x41, 0x00, 0x00,
        0x00, 0x01, 0x02, 0x04, 0x0b, 0x7f, 0x01, 0x01,
    ];

    for local_cid in [0x003f, 0x0040, 0x0041, 0x0042, 0xffff] {
        let mut legacy = original;
        let mut portable = original;
        assert_eq!(
            legacy_strip_l2cap_mhdt_option(&mut legacy, local_cid),
            strip_l2cap_mhdt_option(&mut portable, local_cid)
        );
        assert_eq!(legacy, portable);

        for index in 0..original.len() {
            for value in 0u16..=u8::MAX as u16 {
                let mut legacy = original;
                let mut portable = original;
                legacy[index] = value as u8;
                portable[index] = value as u8;
                assert_eq!(
                    legacy_strip_l2cap_mhdt_option(&mut legacy, local_cid),
                    strip_l2cap_mhdt_option(&mut portable, local_cid),
                    "result differs at byte {index}, value {value:#04x}, cid {local_cid:#06x}"
                );
                assert_eq!(
                    legacy, portable,
                    "bytes differ at byte {index}, value {value:#04x}, cid {local_cid:#06x}"
                );
            }
        }
    }
}

fn legacy_strip_l2cap_mhdt_option(payload: &mut [u8], local_cid: u16) -> Option<usize> {
    const ACL_HEADER_SIZE: usize = 4;
    const L2CAP_HEADER_SIZE: usize = 4;
    const SIGNALING_CID: u16 = 1;
    const CONFIGURATION_REQUEST: u8 = 0x04;
    const MHDT_TYPE: u8 = 0x7f;
    const MHDT_LENGTH: u8 = 1;
    const MHDT_SUPPORTED: u8 = 1;
    const MHDT_OPTION_SIZE: usize = 3;

    if local_cid <= 0x3f || payload.len() < ACL_HEADER_SIZE + L2CAP_HEADER_SIZE {
        return None;
    }
    let packet_boundary = (u16::from_le_bytes([payload[0], payload[1]]) >> 12) & 0x3;
    if !matches!(packet_boundary, 0 | 2) {
        return None;
    }
    let acl_length = u16::from_le_bytes([payload[2], payload[3]]) as usize;
    let acl_end = ACL_HEADER_SIZE.checked_add(acl_length)?;
    if acl_end > payload.len() || acl_length < L2CAP_HEADER_SIZE {
        return None;
    }
    let l2cap_length = u16::from_le_bytes([payload[4], payload[5]]) as usize;
    let l2cap_end = (ACL_HEADER_SIZE + L2CAP_HEADER_SIZE).checked_add(l2cap_length)?;
    if u16::from_le_bytes([payload[6], payload[7]]) != SIGNALING_CID || l2cap_end > acl_end {
        return None;
    }

    let mut command = ACL_HEADER_SIZE + L2CAP_HEADER_SIZE;
    while command + 4 <= l2cap_end {
        let command_length =
            u16::from_le_bytes([payload[command + 2], payload[command + 3]]) as usize;
        let command_end = command.checked_add(4 + command_length)?;
        if command_end > l2cap_end {
            return None;
        }
        if payload[command] == CONFIGURATION_REQUEST && command_length >= 4 {
            let destination_cid = u16::from_le_bytes([payload[command + 4], payload[command + 5]]);
            let flags = u16::from_le_bytes([payload[command + 6], payload[command + 7]]);
            if destination_cid == local_cid && flags == 0 {
                let mut option = command + 8;
                while option + 2 <= command_end {
                    let option_length = payload[option + 1] as usize;
                    let option_end = option.checked_add(2 + option_length)?;
                    if option_end > command_end {
                        return None;
                    }
                    if payload[option] == MHDT_TYPE
                        && payload[option + 1] == MHDT_LENGTH
                        && payload[option + 2] == MHDT_SUPPORTED
                    {
                        payload.copy_within(option_end..acl_end, option);
                        payload[acl_end - MHDT_OPTION_SIZE..acl_end].fill(0);
                        let new_command_length = command_length - MHDT_OPTION_SIZE;
                        let new_l2cap_length = l2cap_length - MHDT_OPTION_SIZE;
                        let new_acl_length = acl_length - MHDT_OPTION_SIZE;
                        payload[command + 2..command + 4]
                            .copy_from_slice(&(new_command_length as u16).to_le_bytes());
                        payload[4..6].copy_from_slice(&(new_l2cap_length as u16).to_le_bytes());
                        payload[2..4].copy_from_slice(&(new_acl_length as u16).to_le_bytes());
                        return Some(acl_end - MHDT_OPTION_SIZE);
                    }
                    option = option_end;
                }
            }
        }
        command = command_end;
    }
    None
}
