use canopus_bluetooth_audio_core::media::{
    MAX_FRAME_LENGTH, MAX_PACKET, StreamPacketizer, TonePacketizer,
};

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
fn derives_bounded_startup_prebuffer_from_sink_delay() {
    let mut p = TonePacketizer::new(672, 39).unwrap();
    assert_eq!(p.startup_packets(1_500), 11);
    assert_eq!(p.startup_packets(0), 11);
    assert_eq!(p.startup_packets(u16::MAX), 16);
    assert!(matches!(p.startup_catchup_delay_ms(11), 144 | 145));
    let mut drain = TonePacketizer::new(672, 39).unwrap();
    assert!(matches!(
        drain.presentation_drain_delay_ms(1_500),
        164 | 165
    ));
    assert!(drain.presentation_drain_delay_ms(u16::MAX) <= 515);
}

#[test]
fn packetizes_variable_length_stream_packets() {
    let mut p = StreamPacketizer::new(2_820, 39, false).unwrap();
    assert_eq!(p.frames_per_packet, 5);
    let mut packet = [0u8; MAX_PACKET];
    let payload = p.write_header(&mut packet, 5, true).unwrap();
    assert_eq!(payload, 13);
    assert_eq!(p.packet_length(5).unwrap(), 13 + 5 * 90);
    assert_eq!(packet[1], 96 | 0x80);
    assert_eq!(packet[12], 5);
    assert_eq!(p.sequence, 2);
    assert_eq!(p.timestamp, 640);
    assert!(matches!(p.next_delay_ms(5), 14 | 15));
    assert_eq!(p.startup_packets(1_500), 11);
    assert_eq!(p.startup_packets(0), 11);
    assert_eq!(p.startup_packets(u16::MAX), 16);

    p.write_header(&mut packet, 2, false).unwrap();
    assert_eq!(packet[1], 96);
    assert_eq!(packet[12], 2);
    assert_eq!(p.frames_sent, 7);
    assert_eq!(p.timestamp, 896);
    assert!(p.write_header(&mut packet, 0, false).is_err());
    assert!(p.write_header(&mut packet, 6, false).is_err());
}

#[test]
fn rejects_unsupported_bitpool_or_small_mtu() {
    assert!(TonePacketizer::new(672, 26).is_err());
    assert!(TonePacketizer::new(102, 53).is_err());
}

use canopus_bluetooth_audio_core::media::{Pace, PacketClock};

/// 640 samples: one five-frame SBC packet, 14.51 ms at 44.1 kHz.
const PACKET: u32 = 640;

#[test]
fn packet_clock_keeps_real_time_on_a_punctual_thread() {
    let mut clock = PacketClock::new();
    // Nothing is owed before the first packet, however long the stream waited for it.
    assert_eq!(clock.poll(1_000), Pace::Send);
    assert_eq!(clock.poll(1_900), Pace::Send);
    clock.sent(1_000, PACKET);
    assert_eq!(clock.poll(1_000), Pace::Wait(15));
    assert_eq!(clock.poll(1_014), Pace::Wait(1));
    assert_eq!(clock.poll(1_015), Pace::Send);
    clock.sent(1_015, PACKET);
    assert_eq!(clock.poll(1_015), Pace::Wait(15));
}

#[test]
fn packet_clock_owes_the_packets_a_stall_held_back() {
    let mut clock = PacketClock::new();
    let mut now = 0u32;
    let mut sent = 0u32;
    // 200 ms in which the owner thread never ran: every packet of it is still owed.
    assert_eq!(clock.poll(now), Pace::Send);
    clock.sent(now, PACKET);
    sent += 1;
    now += 200;
    while clock.poll(now) == Pace::Send {
        clock.sent(now, PACKET);
        sent += 1;
    }
    // 200 ms of audio is 13.8 packets; the stream is back on time, not 200 ms short.
    assert_eq!(sent, 14);
    assert!(clock.take_worst_lag_ms() >= 185);
    assert_eq!(clock.take_worst_lag_ms(), 0);
    // A long run on a thread that only gets to run every 50 ms still delivers real time.
    for _ in 0..200 {
        now += 50;
        while clock.poll(now) == Pace::Send {
            clock.sent(now, PACKET);
            sent += 1;
        }
    }
    let audio_ms = u64::from(sent) * u64::from(PACKET) * 1_000 / 44_100;
    assert!(audio_ms >= u64::from(now) && audio_ms <= u64::from(now) + 15, "{audio_ms} vs {now}");
}

#[test]
fn packet_clock_writes_off_a_stall_the_sink_cannot_have_survived() {
    let mut clock = PacketClock::new();
    assert_eq!(clock.poll(0), Pace::Send);
    clock.sent(0, PACKET);
    let late = PacketClock::CATCH_UP_LIMIT_MS + 500;
    assert_eq!(clock.poll(late), Pace::Send);
    clock.sent(late, PACKET);
    // Re-anchored at `late`: one packet ahead, not 0.9 s of packets behind.
    assert_eq!(clock.poll(late), Pace::Wait(15));
    // A reset (resume after pause) starts over without a debt as well.
    clock.reset();
    assert_eq!(clock.poll(late + 10_000), Pace::Send);
    clock.sent(late + 10_000, PACKET);
    assert_eq!(clock.poll(late + 10_000), Pace::Wait(15));
}

#[test]
fn packet_clock_survives_monotonic_wrap() {
    let mut clock = PacketClock::new();
    let start = u32::MAX - 5;
    assert_eq!(clock.poll(start), Pace::Send);
    clock.sent(start, PACKET);
    assert_eq!(clock.poll(start.wrapping_add(15)), Pace::Send);
}
