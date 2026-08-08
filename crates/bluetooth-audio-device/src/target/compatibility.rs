//! Exact-target BR/EDR compatibility filters installed in writable stock
//! callback slots. The module is boot-resident before any installed callback
//! can outlive its code.

use canopus_target_private::{
    apply_l2cap_information_compatibility, bt_gap_install_send_hook, bt_gap_stock_send,
};

use super::runtime::{
    ERR_HCI_POLICY, FLAG_HCI_SEND_FILTER_HIT, FLAG_HCI_SEND_FILTER_INSTALLED, flag_set,
};

const HCI_PACKET_ACL: u8 = 2;
const MAX_H4_PACKET: usize = 4097;

extern "C" fn hci_send_compatibility(
    state: *mut core::ffi::c_void,
    packet: *mut u8,
    packet_length: i32,
) -> i32 {
    if !packet.is_null() && packet_length > 1 && packet_length as usize <= MAX_H4_PACKET {
        let h4 = unsafe { core::slice::from_raw_parts_mut(packet, packet_length as usize) };
        if h4[0] == HCI_PACKET_ACL && apply_l2cap_information_compatibility(&mut h4[1..]) {
            flag_set(FLAG_HCI_SEND_FILTER_HIT, 0);
        }
    }
    unsafe { bt_gap_stock_send(state, packet, packet_length) }
}

/// Installs the compare-before-write GAP transport send filter. This is the
/// final fallible activation step: after it succeeds the module must remain
/// resident until reboot.
pub fn install() -> Result<(), i32> {
    if unsafe { bt_gap_install_send_hook(hci_send_compatibility) } {
        flag_set(FLAG_HCI_SEND_FILTER_INSTALLED, 0);
        Ok(())
    } else {
        Err(ERR_HCI_POLICY)
    }
}
