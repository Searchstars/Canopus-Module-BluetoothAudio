//! Target-neutral inbound BR/EDR compatibility over an exact-target raw H4
//! receive capability. Target backends own addresses and installation policy;
//! this module owns packet semantics.

use core::sync::atomic::Ordering;

use canopus_target_private::{
    HCI_RECEIVE_HOOK_REQUIRED, bt_gap_install_receive_hook, bt_gap_stock_receive,
};

use super::runtime::{
    ERR_HCI_POLICY, FLAG_HCI_COMPAT_HIT, FLAG_HCI_COMPAT_INSTALLED, MEDIA_CONNECTED,
    MEDIA_CONNECTING, TRANSPORT_CONNECTED, TRANSPORT_CONNECTING, flag_set, runtime,
};

const HCI_PACKET_ACL: u8 = 2;
const MAX_H4_PACKET: usize = 4097;

extern "C" fn hci_receive_compatibility(
    state: *mut core::ffi::c_void,
    packet: *mut u8,
    packet_length: i32,
) -> i32 {
    let mut forwarded_length = packet_length;
    if !packet.is_null() && packet_length > 1 && packet_length as usize <= MAX_H4_PACKET {
        let h4 = unsafe { core::slice::from_raw_parts_mut(packet, packet_length as usize) };
        if h4[0] == HCI_PACKET_ACL {
            let r = runtime();
            let transport_state = r.transport_state.load(Ordering::Acquire);
            let media_state = r.media_state.load(Ordering::Acquire);
            let signaling_cid = r.signaling_cid.load(Ordering::Acquire) as u16;
            let media_cid = r.media_cid.load(Ordering::Acquire) as u16;
            let mut new_acl_length = None;
            if matches!(transport_state, TRANSPORT_CONNECTING | TRANSPORT_CONNECTED)
                && signaling_cid > 0x3F
            {
                new_acl_length = strip_l2cap_mhdt_option(&mut h4[1..], signaling_cid);
            }
            if new_acl_length.is_none()
                && matches!(media_state, MEDIA_CONNECTING | MEDIA_CONNECTED)
                && media_cid > 0x3F
            {
                new_acl_length = strip_l2cap_mhdt_option(&mut h4[1..], media_cid);
            }
            if let Some(new_acl_length) = new_acl_length {
                forwarded_length = (new_acl_length + 1) as i32;
                flag_set(FLAG_HCI_COMPAT_HIT, 0);
            }
        }
    }
    unsafe { bt_gap_stock_receive(state, packet, forwarded_length) }
}

/// Installs the exact-target raw-H4 receive capability when required. Every
/// target follows this path; targets whose Bluetooth stack does not require the
/// BES workaround declare that policy in `canopus-target-private`.
pub fn install() -> Result<(), i32> {
    if !HCI_RECEIVE_HOOK_REQUIRED {
        return Ok(());
    }
    if unsafe { bt_gap_install_receive_hook(hci_receive_compatibility) } {
        flag_set(FLAG_HCI_COMPAT_INSTALLED, 0);
        Ok(())
    } else {
        Err(ERR_HCI_POLICY)
    }
}

/// Removes the exact BES mHDT capability option (`7F 01 01`) from an inbound
/// Configuration Request for `local_cid`, leaving all standard options intact.
fn strip_l2cap_mhdt_option(payload: &mut [u8], local_cid: u16) -> Option<usize> {
    const ACL_HEADER_SIZE: usize = 4;
    const L2CAP_HEADER_SIZE: usize = 4;
    const SIGNALING_CID: u16 = 1;
    const CONFIGURATION_REQUEST: u8 = 0x04;
    const MHDT_TYPE: u8 = 0x7F;
    const MHDT_LENGTH: u8 = 1;
    const MHDT_SUPPORTED: u8 = 1;
    const MHDT_OPTION_SIZE: usize = 3;

    if local_cid <= 0x3F || payload.len() < ACL_HEADER_SIZE + L2CAP_HEADER_SIZE {
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

#[cfg(test)]
mod tests {
    use super::strip_l2cap_mhdt_option;

    #[test]
    fn strips_bes_mhdt_from_matching_configuration_request() {
        let mut packet = [
            0x01, 0x20, 0x13, 0x00, 0x0f, 0x00, 0x01, 0x00, 0x04, 0x01, 0x0b, 0x00, 0x41, 0x00,
            0x00, 0x00, 0x01, 0x02, 0x00, 0x04, 0x7f, 0x01, 0x01,
        ];
        assert_eq!(strip_l2cap_mhdt_option(&mut packet, 0x0041), Some(20));
        assert_eq!(&packet[2..4], &[0x10, 0x00]);
        assert_eq!(&packet[4..6], &[0x0c, 0x00]);
        assert_eq!(&packet[10..12], &[0x08, 0x00]);
        assert_eq!(&packet[20..23], &[0, 0, 0]);
    }
}
