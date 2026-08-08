use canopus_bluetooth_audio_core::mp3::{
    Mp3ByteStream, Mp3Decoder, REQUIRED_SAMPLE_RATE, StreamProgress,
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
                } => {
                    assert!(consumed > 0);
                    assert_eq!(sample_rate, REQUIRED_SAMPLE_RATE);
                    assert_eq!(channels, 2);
                    assert_eq!(samples, frames * 2);
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
