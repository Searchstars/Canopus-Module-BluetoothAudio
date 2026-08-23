//! Target-independent BR/EDR wire compatibility transforms.

const ACL_HEADER_SIZE: usize = 4;
const L2CAP_HEADER_SIZE: usize = 4;
const SIGNALING_CID: u16 = 1;

/// Removes the exact BES mHDT capability option (`7F 01 01`) from an inbound
/// Configuration Request for `local_cid`, leaving all standard options intact.
/// The input excludes the H4 type byte. On success, all enclosing lengths are
/// repaired and the new ACL packet length is returned.
pub fn strip_l2cap_mhdt_option(payload: &mut [u8], local_cid: u16) -> Option<usize> {
    strip_l2cap_mhdt_option_matching(payload, Some(local_cid))
}

/// Removes the exact BES mHDT capability option from any inbound Configuration
/// Request for a dynamic destination CID. This is the target-neutral receive-hook
/// policy: it does not depend on platform callback ordering or module-owned
/// channel state, both of which can lag the wire packet being filtered.
pub fn strip_l2cap_mhdt_option_any_cid(payload: &mut [u8]) -> Option<usize> {
    strip_l2cap_mhdt_option_matching(payload, None)
}

fn strip_l2cap_mhdt_option_matching(payload: &mut [u8], local_cid: Option<u16>) -> Option<usize> {
    const CONFIGURATION_REQUEST: u8 = 0x04;
    const MHDT_TYPE: u8 = 0x7f;
    const MHDT_LENGTH: u8 = 1;
    const MHDT_SUPPORTED: u8 = 1;
    const MHDT_OPTION_SIZE: usize = 3;

    if local_cid.is_some_and(|cid| cid <= 0x3f) {
        return None;
    }
    let acl_length =
        u16::from_le_bytes([payload.get(2).copied()?, payload.get(3).copied()?]) as usize;
    let l2cap_length =
        u16::from_le_bytes([payload.get(4).copied()?, payload.get(5).copied()?]) as usize;
    let (mut command, l2cap_end, acl_end) = signaling_bounds(payload)?;

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
            if destination_cid > 0x3f
                && local_cid.is_none_or(|expected| destination_cid == expected)
                && flags == 0
            {
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

fn signaling_bounds(payload: &[u8]) -> Option<(usize, usize, usize)> {
    if payload.len() < ACL_HEADER_SIZE + L2CAP_HEADER_SIZE {
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
    Some((ACL_HEADER_SIZE + L2CAP_HEADER_SIZE, l2cap_end, acl_end))
}
