//! Last-headset metadata. Link keys remain in the firmware's Classic bond store.

use crate::{Address, BondState, DeviceName, Peer, discovery::DEVICE_NAME_CAPACITY};

pub const RECORD_SIZE: usize = 84;
const MAGIC: &[u8; 4] = b"BAP1";

fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

pub fn encode(peer: &Peer) -> [u8; RECORD_SIZE] {
    let mut record = [0u8; RECORD_SIZE];
    record[..4].copy_from_slice(MAGIC);
    record[4..10].copy_from_slice(&peer.address.0);
    let name = peer.name.as_str().as_bytes();
    record[10] = name.len() as u8;
    record[12..16].copy_from_slice(&peer.class_of_device.to_le_bytes());
    record[16..16 + name.len()].copy_from_slice(name);
    let sum = checksum(&record[..80]);
    record[80..].copy_from_slice(&sum.to_le_bytes());
    record
}

pub fn decode(record: &[u8]) -> Option<Peer> {
    if record.len() != RECORD_SIZE || &record[..4] != MAGIC {
        return None;
    }
    let len = usize::from(record[10]);
    if len > DEVICE_NAME_CAPACITY
        || record[4..10] == [0; 6]
        || u32::from_le_bytes(record[80..].try_into().ok()?) != checksum(&record[..80])
        || core::str::from_utf8(&record[16..16 + len]).is_err()
    {
        return None;
    }
    Some(Peer {
        address: Address(record[4..10].try_into().ok()?),
        name: DeviceName::from_bytes(&record[16..16 + len]),
        class_of_device: u32::from_le_bytes(record[12..16].try_into().ok()?),
        bond: BondState::Bonded,
        ..Peer::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembered_headset_survives_restart_and_rejects_damaged_records() {
        let peer = Peer {
            address: Address([1, 2, 3, 4, 5, 6]),
            name: DeviceName::from_bytes("小米耳机 Buds".as_bytes()),
            class_of_device: 0x240404,
            bond: BondState::Bonded,
            ..Peer::default()
        };
        let mut record = encode(&peer);
        assert_eq!(decode(&record), Some(peer));
        assert_eq!(decode(&record[..83]), None);
        record[20] ^= 1;
        assert_eq!(decode(&record), None);
    }
}
