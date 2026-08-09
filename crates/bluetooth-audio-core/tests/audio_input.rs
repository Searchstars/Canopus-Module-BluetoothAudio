use canopus_bluetooth_audio_core::audio_input::{
    AudioInput, FORMAT_MP3, FormatV1, InputError, InputRing, STATE_BUFFERING, STATE_CONFIGURED,
    STATE_DRAINING, STATE_PAUSED, STATE_STOPPED,
};

fn mp3_format() -> FormatV1 {
    FormatV1 {
        struct_size: core::mem::size_of::<FormatV1>() as u32,
        format: FORMAT_MP3,
        sample_rate_hint: 44_100,
        channels_hint: 2,
        flags: 0,
        reserved: [0; 3],
    }
}

#[test]
fn ring_wraps_and_applies_backpressure_without_loss() {
    let input = AudioInput::<8>::new();
    input.open().unwrap();
    input.set_format(&mp3_format()).unwrap();
    assert_eq!(input.write(&[0, 1, 2, 3, 4, 5]).unwrap(), 6);
    let mut first = [0u8; 4];
    assert_eq!(input.consume(&mut first), 4);
    assert_eq!(first, [0, 1, 2, 3]);
    assert_eq!(input.write(&[6, 7, 8, 9, 10, 11]).unwrap(), 6);
    assert_eq!(input.write(&[12]), Err(InputError::Again));
    let mut rest = [0u8; 8];
    assert_eq!(input.consume(&mut rest), 8);
    assert_eq!(rest, [4, 5, 6, 7, 8, 9, 10, 11]);
}

#[test]
fn ring_reset_invalidates_old_cursors_and_preserves_new_data() {
    let ring = InputRing::<8>::new();
    for epoch in 0..1024u32 {
        let old = [epoch as u8; 6];
        assert_eq!(ring.write(&old), old.len());
        let mut prefix = [0u8; 2];
        assert_eq!(ring.read(&mut prefix), prefix.len());
        ring.reset();
        assert_eq!(ring.used(), 0);

        let new = [epoch.wrapping_add(1) as u8; 8];
        assert_eq!(ring.write(&new), new.len());
        let mut output = [0u8; 8];
        assert_eq!(ring.read(&mut output), output.len());
        assert_eq!(output, new);
        assert_eq!(ring.used(), 0);
    }
}

#[test]
fn exclusive_open_and_control_lifecycle_are_enforced() {
    let input = AudioInput::<16>::new();
    input.open().unwrap();
    assert_eq!(input.volume(), 100);
    input.set_volume(42).unwrap();
    assert_eq!(input.status().volume_percent, 42);
    assert_eq!(input.set_volume(101), Err(InputError::Invalid));
    assert_eq!(input.open(), Err(InputError::Busy));
    input.set_format(&mp3_format()).unwrap();
    assert_eq!(input.state(), STATE_CONFIGURED);
    assert!(input.decoded_format_matches(44_100, 2));
    assert!(!input.decoded_format_matches(48_000, 2));
    let configured_generation = input.generation();
    input.start().unwrap();
    assert!(input.generation() > configured_generation);
    assert_eq!(input.state(), STATE_BUFFERING);
    input.pause().unwrap();
    assert_eq!(input.state(), STATE_PAUSED);
    input.resume().unwrap();
    assert_eq!(input.state(), STATE_BUFFERING);
    input.write(&[1, 2, 3]).unwrap();
    input.drain().unwrap();
    assert_eq!(input.state(), STATE_DRAINING);
    assert!(input.mark_playing(input.generation(), 44_100, 2, 39));
    assert_eq!(input.state(), STATE_DRAINING);
    assert_eq!(input.write(&[4]), Err(InputError::Pipe));
    let mut bytes = [0; 3];
    assert_eq!(input.consume(&mut bytes), 3);
    input.mark_drained(input.generation(), true);
    assert_eq!(input.state(), STATE_STOPPED);
    input.close().unwrap();
    input.open().unwrap();
}

#[test]
fn stop_and_flush_invalidate_stale_work_and_clear_ring() {
    let input = AudioInput::<16>::new();
    input.open().unwrap();
    input.set_format(&mp3_format()).unwrap();
    input.write(&[1, 2, 3, 4]).unwrap();
    let configured_generation = input.generation();
    input.start().unwrap();
    input.stop().unwrap();
    assert!(input.generation() > configured_generation);
    assert_eq!(input.status().input_used, 0);
    input.write(&[5, 6]).unwrap();
    input.flush().unwrap();
    assert_eq!(input.state(), STATE_CONFIGURED);
    assert_eq!(input.status().input_used, 0);
}

#[test]
fn format_layout_and_reserved_fields_are_validated() {
    let input = AudioInput::<16>::new();
    input.open().unwrap();
    let mut format = mp3_format();
    format.reserved[1] = 1;
    assert_eq!(input.set_format(&format), Err(InputError::Invalid));
    format = mp3_format();
    format.struct_size -= 4;
    assert_eq!(input.set_format(&format), Err(InputError::Invalid));
}
