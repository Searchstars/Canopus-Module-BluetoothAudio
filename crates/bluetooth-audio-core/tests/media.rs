use canopus_bluetooth_audio_core::media::{
    FRAME_LENGTH, MAX_PACKET, TEST_TONE_FRAME, TonePacketizer,
};

#[test]
fn packetizes_tone_with_rtp_progression() {
    let mut p = TonePacketizer::new(672).unwrap();
    let mut packet = [0u8; MAX_PACKET];
    let n = p.write_packet(&mut packet).unwrap();
    assert_eq!(n, 13 + 5 * FRAME_LENGTH);
    assert_eq!(packet[1] & 0x80, 0x80);
    assert_eq!(&packet[8..12], &0x4254_5036u32.to_be_bytes());
    assert_eq!(&packet[13..13 + FRAME_LENGTH], &TEST_TONE_FRAME);
    assert_eq!(p.sequence, 2);
    assert_eq!(p.timestamp, 640);
    assert!(matches!(p.next_delay_ms(), 14 | 15));
}
#[test]
fn honors_small_mtu_and_exact_duration_ceiling() {
    let mut p = TonePacketizer::new(13 + 2 * FRAME_LENGTH as u16).unwrap();
    assert_eq!(p.frames_per_packet, 2);
    let mut packet = [0u8; MAX_PACKET];
    while !p.is_complete() {
        p.write_packet(&mut packet).unwrap();
    }
    assert_eq!(p.packets_sent, p.packets_target);
    assert!(p.frames_sent * 128 >= 5 * 44100);
}
#[test]
fn rejects_mtu_smaller_than_one_frame() {
    assert!(TonePacketizer::new(130).is_err());
}
