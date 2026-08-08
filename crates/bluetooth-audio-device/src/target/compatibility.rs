//! Exact-target inbound BR/EDR compatibility installed in the writable stock
//! receive callback slot. The module is boot-resident before the callback can
//! outlive its code.

use core::sync::atomic::Ordering;

use canopus_target_private::{
    bt_gap_install_receive_hook, bt_gap_stock_receive, strip_l2cap_mhdt_option,
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

/// Installs the compare-before-write GAP host receive filter. This is the final
/// fallible activation step: after it succeeds the module must remain resident
/// until reboot.
pub fn install() -> Result<(), i32> {
    if unsafe { bt_gap_install_receive_hook(hci_receive_compatibility) } {
        flag_set(FLAG_HCI_COMPAT_INSTALLED, 0);
        Ok(())
    } else {
        Err(ERR_HCI_POLICY)
    }
}
