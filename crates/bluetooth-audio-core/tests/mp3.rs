use canopus_bluetooth_audio_core::mp3::{
    Mp3ByteStream, Mp3Decoder, REQUIRED_INPUT_WINDOW, REQUIRED_SAMPLE_RATE, StreamProgress,
};

const MP3: &[u8] = include_bytes!("fixtures/tone-44100-stereo.mp3");
const MP3_48K: &[u8] = include_bytes!("fixtures/tone-48000-stereo.mp3");

#[test]
fn decodes_mp3_frames_and_converts_to_stereo_s16() {
    let mut decoder = Mp3Decoder::new();
    let mut offset = 0usize;
    let mut decoded_frames = 0usize;
    let mut pcm = [0i16; 2304];
    while offset < MP3.len() {
        let frame = match decoder.decode(&MP3[offset..]) {
            Ok(frame) => frame,
            Err(_) => break,
        };
        assert!(frame.consumed > 0);
        offset += frame.consumed;
        if frame.frames == 0 {
            continue;
        }
        assert_eq!(frame.sample_rate, REQUIRED_SAMPLE_RATE);
        assert_eq!(frame.channels, 2);
        let samples = decoder.write_stereo_s16(frame, 100, &mut pcm).unwrap();
        assert_eq!(samples, frame.frames * 2);
        assert!(pcm[..samples].iter().any(|sample| *sample != 0));
        decoded_frames += frame.frames;
    }
    assert!(decoded_frames >= 4 * 1152);
}

#[test]
fn initializes_large_stream_workspace_in_place() {
    let mut storage = Box::new(core::mem::MaybeUninit::<Mp3ByteStream>::uninit());
    unsafe { Mp3ByteStream::initialize_at(storage.as_mut_ptr()) };
    let mut stream = unsafe { storage.assume_init() };
    assert_eq!(stream.pending(), 0);
    assert_eq!(stream.push(MP3), MP3.len());

    let mut pcm = [0i16; 2304];
    let mut decoded_frames = 0usize;
    loop {
        match stream.decode_next(100, &mut pcm, true).unwrap() {
            StreamProgress::NeedInput => break,
            StreamProgress::Skipped { .. } => {}
            StreamProgress::Frame { frames, .. } => decoded_frames += frames,
        }
    }
    assert!(decoded_frames >= 4 * 1152);
}

#[test]
fn decodes_frames_split_across_arbitrary_writes() {
    let mut stream = Mp3ByteStream::new();
    let mut cursor = 0usize;
    let mut chunk_index = 0usize;
    let mut decoded_frames = 0usize;
    let mut pcm = [0i16; 2304];
    let chunk_sizes = [1usize, 7, 31, 509, 3, 1024, 19];

    while cursor < MP3.len() {
        let requested = chunk_sizes[chunk_index % chunk_sizes.len()];
        chunk_index += 1;
        let end = (cursor + requested).min(MP3.len());
        let pushed = stream.push(&MP3[cursor..end]);
        assert!(pushed > 0);
        cursor += pushed;

        loop {
            match stream.decode_next(42, &mut pcm, false).unwrap() {
                StreamProgress::NeedInput => break,
                StreamProgress::Skipped { consumed } => assert!(consumed > 0),
                StreamProgress::Frame {
                    consumed,
                    frames,
                    samples,
                    sample_rate,
                    channels,
                    source_frames,
                } => {
                    assert!(consumed > 0);
                    assert_eq!(sample_rate, REQUIRED_SAMPLE_RATE);
                    assert_eq!(channels, 2);
                    assert_eq!(samples, frames * 2);
                    assert_eq!(source_frames, frames);
                    assert!(pcm[..samples].iter().any(|sample| *sample != 0));
                    decoded_frames += frames;
                }
            }
        }
    }

    loop {
        match stream.decode_next(42, &mut pcm, true).unwrap() {
            StreamProgress::NeedInput => break,
            StreamProgress::Skipped { .. } => {}
            StreamProgress::Frame { frames, .. } => decoded_frames += frames,
        }
    }
    assert!(decoded_frames >= 4 * 1152);
    assert!(stream.discard_incomplete() < 2_000);
    assert_eq!(stream.pending(), 0);
}

fn assert_resampled_stream(mp3: &[u8], expected_rate: u32) {
    let mut stream = Mp3ByteStream::new();
    let mut cursor = 0usize;
    let mut decoded_frames = 0usize;
    let mut pcm = [0i16; 2304];

    while cursor < mp3.len() {
        let count = stream.free().min(mp3.len() - cursor);
        cursor += stream.push(&mp3[cursor..cursor + count]);
        loop {
            match stream.decode_next(100, &mut pcm, false).unwrap() {
                StreamProgress::NeedInput => break,
                StreamProgress::Skipped { consumed } => assert!(consumed > 0),
                StreamProgress::Frame {
                    frames,
                    samples,
                    sample_rate,
                    channels,
                    ..
                } => {
                    assert_eq!(sample_rate, expected_rate);
                    assert_eq!(channels, 2);
                    assert!(matches!(frames, 1058 | 1059));
                    assert_eq!(samples, frames * 2);
                    decoded_frames += frames;
                    if pcm[..samples].iter().any(|sample| *sample != 0) {
                        return;
                    }
                }
            }
        }
    }
    panic!("{expected_rate}-Hz MP3 produced {decoded_frames} silent resampled frames");
}

fn assert_complete_resampled_packetization(
    mp3: &[u8],
    expected_rate: u32,
    source_frames_per_mp3_frame: usize,
) {
    const SBC_FRAMES: usize = 128;
    const SBC_FRAMES_PER_PACKET: usize = 5;

    let mut stream = Mp3ByteStream::new();
    let mut cursor = 0usize;
    let mut decoded_mp3_frames = 0usize;
    let mut output_frames = 0usize;
    let mut media_frames = 0usize;
    let mut carry = 0usize;
    let mut packets = 0usize;
    let mut sbc_frames_encoded = 0usize;
    let mut pcm = [0i16; 2304];
    let chunks = [17usize, 509, 2048, 73, 4096, 3, 997];
    let mut chunk_index = 0usize;

    while cursor < mp3.len() {
        let requested = chunks[chunk_index % chunks.len()];
        chunk_index += 1;
        let count = requested.min(stream.free()).min(mp3.len() - cursor);
        if count != 0 {
            cursor += stream.push(&mp3[cursor..cursor + count]);
        }
        loop {
            match stream.decode_next(100, &mut pcm, false).unwrap() {
                StreamProgress::NeedInput => break,
                StreamProgress::Skipped { consumed } => assert!(consumed > 0),
                StreamProgress::Frame {
                    frames,
                    samples,
                    sample_rate,
                    channels,
                    source_frames,
                    ..
                } => {
                    assert_eq!(sample_rate, expected_rate);
                    assert_eq!(channels, 2);
                    assert_eq!(samples, frames * 2);
                    decoded_mp3_frames += 1;
                    output_frames += frames;
                    media_frames += source_frames;
                    carry += frames;
                    while carry >= SBC_FRAMES {
                        let encoded = (carry / SBC_FRAMES).min(SBC_FRAMES_PER_PACKET);
                        assert!((1..=SBC_FRAMES_PER_PACKET).contains(&encoded));
                        carry -= encoded * SBC_FRAMES;
                        sbc_frames_encoded += encoded;
                        packets += 1;
                    }
                    assert!(carry < SBC_FRAMES);
                }
            }
        }
    }

    loop {
        match stream.decode_next(100, &mut pcm, true).unwrap() {
            StreamProgress::NeedInput => break,
            StreamProgress::Skipped { consumed } => assert!(consumed > 0),
            StreamProgress::Frame {
                frames,
                sample_rate,
                channels,
                source_frames,
                ..
            } => {
                assert_eq!(sample_rate, expected_rate);
                assert_eq!(channels, 2);
                decoded_mp3_frames += 1;
                output_frames += frames;
                media_frames += source_frames;
                carry += frames;
                while carry >= SBC_FRAMES {
                    let encoded = (carry / SBC_FRAMES).min(SBC_FRAMES_PER_PACKET);
                    carry -= encoded * SBC_FRAMES;
                    sbc_frames_encoded += encoded;
                    packets += 1;
                }
                assert!(carry < SBC_FRAMES);
            }
        }
    }

    let expected_output = (decoded_mp3_frames * source_frames_per_mp3_frame * 44_100)
        .div_ceil(expected_rate as usize);
    assert_eq!(output_frames, expected_output);
    // A clock that divides by the MP3's own rate needs the MP3's own frame count; dividing the
    // 44.1-kHz output count by 48 kHz ran an 8 % slow clock.
    assert_eq!(media_frames, decoded_mp3_frames * source_frames_per_mp3_frame);
    assert!(packets > 0);
    assert_eq!(sbc_frames_encoded, output_frames / SBC_FRAMES);
    assert!(carry < SBC_FRAMES);
    if carry != 0 {
        sbc_frames_encoded += 1;
        packets += 1;
    }
    assert_eq!(sbc_frames_encoded, output_frames.div_ceil(SBC_FRAMES));
    assert!(packets <= sbc_frames_encoded);
    assert!(stream.discard_incomplete() < 2_000);
}

#[test]
fn packetizes_complete_packaged_24khz_stream_without_losing_pcm_carry() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../watchfaces/bluetooth-audio/long_test_audio.bin");
    let mp3 = std::fs::read(path).unwrap();
    assert_complete_resampled_packetization(&mp3, 24_000, 576);
}

#[test]
fn packetizes_complete_48khz_stream_without_losing_pcm_carry() {
    assert_complete_resampled_packetization(MP3_48K, 48_000, 1152);
}

#[test]
fn resamples_packaged_24khz_mp3_for_44100hz_sbc() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../watchfaces/bluetooth-audio/long_test_audio.bin");
    let mp3 = std::fs::read(path).unwrap();
    assert_resampled_stream(&mp3, 24_000);
}

#[test]
fn resamples_48khz_mp3_for_44100hz_sbc() {
    assert_resampled_stream(MP3_48K, 48_000);
}

fn packaged_mp3() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../watchfaces/bluetooth-audio/long_test_audio.bin");
    std::fs::read(path).unwrap()
}

/// Decodes `mp3` through `Mp3ByteStream`, offering at most `chunk` bytes per
/// push, and returns the S16 output plus the number of skipped bytes.
fn decode_stream(mp3: &[u8], chunk: usize) -> (Vec<i16>, usize) {
    let mut stream = Mp3ByteStream::new();
    let mut cursor = 0usize;
    let mut output = Vec::new();
    let mut skipped = 0usize;
    let mut pcm = [0i16; 2304];
    loop {
        // Refill the way the device pump does: until the stream stops
        // offering space or the producer has nothing left.
        while stream.free() != 0 && cursor < mp3.len() {
            let count = chunk.min(stream.free()).min(mp3.len() - cursor);
            assert_eq!(stream.push(&mp3[cursor..cursor + count]), count);
            cursor += count;
        }
        let end_of_input = cursor == mp3.len();
        match stream.decode_next(100, &mut pcm, end_of_input).unwrap() {
            StreamProgress::NeedInput if end_of_input => break,
            StreamProgress::NeedInput => assert!(stream.free() != 0),
            StreamProgress::Skipped { consumed } => {
                assert!(consumed > 0);
                skipped += consumed;
            }
            StreamProgress::Frame { samples, .. } => {
                output.extend_from_slice(&pcm[..samples]);
            }
        }
    }
    stream.discard_incomplete();
    (output, skipped)
}

#[test]
fn window_refills_do_not_change_decoded_pcm() {
    let mp3 = packaged_mp3();
    let (pump_sized, pump_skipped) = decode_stream(&mp3, 512);
    let (odd_sized, odd_skipped) = decode_stream(&mp3, 97);
    assert!(pump_sized.len() > 44_100 * 2 * 60);
    assert_eq!(pump_skipped, odd_skipped);
    assert!(pump_sized == odd_sized);
}

#[test]
fn skips_id3v2_tag_without_scanning_its_payload() {
    let mp3 = packaged_mp3();
    assert_eq!(&mp3[..3], b"ID3");
    let tag = 10
        + mp3[6..10]
            .iter()
            .fold(0usize, |size, byte| size << 7 | usize::from(*byte));
    assert!(tag > REQUIRED_INPUT_WINDOW);

    // The packaged file starts with a large ID3v2 tag carrying cover art.
    let audio = &mp3[tag..tag + 64 * 1024];
    let (tagged, skipped) = decode_stream(&mp3[..tag + audio.len()], 512);
    assert!(!tagged.is_empty());
    let (_, untagged_skipped) = decode_stream(audio, 512);
    assert_eq!(skipped, tag + untagged_skipped);

    // A tag whose payload looks like MP3 frames is still skipped as a whole.
    let mut spoofed = Vec::new();
    spoofed.extend_from_slice(b"ID3\x04\x00\x10");
    let payload = 3 * MP3.len();
    spoofed.extend((0..4).map(|shift| ((payload >> (21 - 7 * shift)) & 0x7f) as u8));
    for _ in 0..3 {
        spoofed.extend_from_slice(MP3);
    }
    spoofed.extend_from_slice(b"3DI\x04\x00\x00\x00\x00\x00\x00");
    spoofed.extend_from_slice(audio);
    let (spoofed_pcm, spoofed_skipped) = decode_stream(&spoofed, 509);
    assert_eq!(spoofed_skipped, 10 + payload + 10 + untagged_skipped);
    assert!(spoofed_pcm == tagged);
}
