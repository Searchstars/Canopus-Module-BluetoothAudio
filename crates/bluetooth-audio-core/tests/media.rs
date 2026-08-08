use canopus_bluetooth_audio_core::media::{MAX_FRAME_LENGTH, MAX_PACKET, TonePacketizer};

#[test]
fn packetizes_tone_with_rtp_progression() {
    let mut p = TonePacketizer::new(672, 53).unwrap();
    let mut packet = [0u8; MAX_PACKET];
    let n = p.write_packet(&mut packet).unwrap();
    assert_eq!(n, 13 + 5 * MAX_FRAME_LENGTH);
    assert_eq!(packet[1] & 0x80, 0x80);
    assert_eq!(&packet[8..12], &0x4254_5036u32.to_be_bytes());
    assert_eq!(&packet[13..16], &[0x9c, 0xb9, 53]);
    assert_eq!(p.sequence, 2);
    assert_eq!(p.timestamp, 640);
    assert!(matches!(p.next_delay_ms(), 14 | 15));
}

#[test]
fn packetizes_peer_limited_bitpool_39() {
    let mut p = TonePacketizer::new(672, 39).unwrap();
    assert_eq!(p.frame_length, 90);
    let mut packet = [0u8; MAX_PACKET];
    let n = p.write_packet(&mut packet).unwrap();
    assert_eq!(n, 13 + 5 * 90);
    assert_eq!(&packet[13..16], &[0x9c, 0xb9, 39]);
}

#[test]
fn honors_small_mtu_and_exact_duration_ceiling() {
    let frame_length = 90u16;
    let mut p = TonePacketizer::new(13 + 2 * frame_length, 39).unwrap();
    assert_eq!(p.frames_per_packet, 2);
    let mut packet = [0u8; MAX_PACKET];
    while !p.is_complete() {
        p.write_packet(&mut packet).unwrap();
    }
    assert_eq!(p.packets_sent, p.packets_target);
    assert!(p.frames_sent * 128 >= 5 * 44100);
}

#[test]
fn every_supported_stereo_bitpool_has_a_matching_frame() {
    let mut packet = [0u8; MAX_PACKET];
    for bitpool in 27..=53 {
        let mut p = TonePacketizer::new(672, bitpool).unwrap();
        let n = p.write_packet(&mut packet).unwrap();
        let frame_length = 12 + 2 * usize::from(bitpool);
        assert_eq!(p.frame_length as usize, frame_length);
        assert_eq!(n, 13 + 5 * frame_length);
        assert_eq!(&packet[13..16], &[0x9c, 0xb9, bitpool]);
    }
}

#[test]
fn rejects_unsupported_bitpool_or_small_mtu() {
    assert!(TonePacketizer::new(672, 26).is_err());
    assert!(TonePacketizer::new(102, 53).is_err());
}
