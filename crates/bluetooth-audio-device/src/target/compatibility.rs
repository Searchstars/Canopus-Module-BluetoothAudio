//! Target-neutral inbound BR/EDR compatibility over an exact-target raw H4
//! receive capability. Target backends own addresses and the writable seam;
//! this module owns installation and packet policy.

use canopus_bluetooth_audio_core::compatibility::strip_l2cap_mhdt_option_any_cid;
use canopus_target_private::{bt_gap_install_receive_hook, bt_gap_stock_receive};

use super::runtime::{ERR_HCI_POLICY, FLAG_HCI_COMPAT_HIT, FLAG_HCI_COMPAT_INSTALLED, flag_set};

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
            // The compatibility policy is a property of the exact wire option,
            // not of target callback timing. Filter any dynamic-channel
            // Configuration Request carrying BES mHDT before stock dispatch.
            if let Some(new_acl_length) = strip_l2cap_mhdt_option_any_cid(&mut h4[1..]) {
                forwarded_length = (new_acl_length + 1) as i32;
                flag_set(FLAG_HCI_COMPAT_HIT, 0);
            }
        }
    }
    unsafe { bt_gap_stock_receive(state, packet, forwarded_length) }
}

/// Installs the target's raw-H4 receive seam. mHDT handling is module policy:
/// every runtime-capable target must expose the seam or fail closed.
pub fn install() -> Result<(), i32> {
    if unsafe { bt_gap_install_receive_hook(hci_receive_compatibility) } {
        flag_set(FLAG_HCI_COMPAT_INSTALLED, 0);
        Ok(())
    } else {
        Err(ERR_HCI_POLICY)
    }
}
