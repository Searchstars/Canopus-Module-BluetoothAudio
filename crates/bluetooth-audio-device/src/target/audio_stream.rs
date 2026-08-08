//! Bluetooth-owner MP3 -> PCM -> SBC -> RTP streaming pipeline.

use core::{
    ffi::c_void,
    mem::{MaybeUninit, size_of},
    sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering},
};

use canopus_bluetooth_audio_core::{
    audio_input::{
        FORMAT_MP3, FormatV1, STATE_BUFFERING, STATE_DRAINING, STATE_PAUSED, STATE_PLAYING,
    },
    media::{FRAME_SAMPLES, StreamPacketizer},
    mp3::{Mp3ByteStream, StreamProgress},
};
use canopus_target_private::*;

use super::{
    audio_device,
    runtime::{
        Core, ERR_AUDIO_CODEC, ERR_AUDIO_DECODE, ERR_AUDIO_QUEUE, ERR_MEDIA_ALLOC, ERR_MEDIA_STATE,
        MEDIA_COMPLETE, MEDIA_STREAMING, runtime, with_core,
    },
    sbc_encoder::SbcEncoder,
    transport,
};

const AUDIO_TIMER_EVENT: u8 = 10;
const AUDIO_TIMER_TAG: &[u8] = b"A2DPA\0";
const WORK_START: u8 = 1;
const WORK_WAKE: u8 = 2;
const WORK_PAUSE: u8 = 3;
const WORK_RESUME: u8 = 4;
const WORK_STOP: u8 = 5;
const WORK_DRAIN: u8 = 6;
const WORK_FLUSH: u8 = 7;
const WORK_CLOSE: u8 = 8;
const WORK_TEST_PREPARE: u8 = 9;
const INGEST_CHUNK: usize = 512;
const PCM_FRAME_CAPACITY: usize = 1152 + FRAME_SAMPLES as usize - 1;
const PCM_SAMPLES: usize = 2 * PCM_FRAME_CAPACITY;
const LONG_TEST_AUDIO_PATH: &[u8] = b"/data/canopus/tmp_btaudio_module_long_audio_test.mp3\0";

#[repr(C)]
struct WorkToken {
    runtime_generation: u32,
    audio_generation: u32,
    command: u32,
}

#[repr(C)]
struct TimerToken {
    runtime_generation: u32,
    audio_generation: u32,
    timer_generation: u32,
}

struct Pipeline {
    stream: Mp3ByteStream,
    pcm: [i16; PCM_SAMPLES],
    pcm_frames: usize,
    pcm_offset: usize,
    ingest: [u8; INGEST_CHUNK],
    encoder: MaybeUninit<SbcEncoder>,
    encoder_ready: bool,
    packetizer: MaybeUninit<StreamPacketizer>,
    packetizer_ready: bool,
    session_generation: u32,
    marker: bool,
}

static PIPELINE: AtomicUsize = AtomicUsize::new(0);
static WAKE_QUEUED: AtomicBool = AtomicBool::new(false);
static WAKE_RUNTIME_GENERATION: AtomicU32 = AtomicU32::new(0);
static WAKE_AUDIO_GENERATION: AtomicU32 = AtomicU32::new(0);
static LONG_TEST_FD: AtomicI32 = AtomicI32::new(-1);
static LONG_TEST_OWNS_INPUT: AtomicBool = AtomicBool::new(false);

fn pipeline() -> Option<&'static mut Pipeline> {
    let pointer = PIPELINE.load(Ordering::Acquire) as *mut Pipeline;
    if pointer.is_null() {
        None
    } else {
        // SAFETY: all callers hold the module Core lock, which is also the sole
        // synchronization domain for this owner-thread pipeline.
        Some(unsafe { &mut *pointer })
    }
}

fn allocate_pipeline() -> Result<&'static mut Pipeline, i32> {
    if let Some(existing) = pipeline() {
        return Ok(existing);
    }
    let allocation = unsafe { bt_alloc(size_of::<Pipeline>() as u32) } as *mut Pipeline;
    if allocation.is_null() {
        return Err(ERR_MEDIA_ALLOC);
    }
    // SAFETY: the allocation is fresh and correctly aligned. MaybeUninit fields,
    // numeric fields, arrays, and bool=false all accept an all-zero value; the
    // decoder workspace then receives its explicit in-place initialization.
    unsafe {
        allocation
            .cast::<u8>()
            .write_bytes(0, size_of::<Pipeline>());
        Mp3ByteStream::initialize_at(core::ptr::addr_of_mut!((*allocation).stream));
    }
    match PIPELINE.compare_exchange(0, allocation as usize, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Ok(unsafe { &mut *allocation }),
        Err(existing) => {
            unsafe { bt_free(allocation.cast()) };
            Ok(unsafe { &mut *(existing as *mut Pipeline) })
        }
    }
}

impl Pipeline {
    fn begin(&mut self, generation: u32, mtu: u16, bitpool: u8) -> Result<(), i32> {
        self.stream.reset();
        self.pcm.fill(0);
        self.pcm_frames = 0;
        self.pcm_offset = 0;
        if self.encoder_ready {
            // SAFETY: encoder_ready tracks initialization of this field.
            unsafe { self.encoder.assume_init_drop() };
            self.encoder_ready = false;
        }
        let encoder = SbcEncoder::new(bitpool)?;
        self.encoder.write(encoder);
        self.encoder_ready = true;
        let packetizer = StreamPacketizer::new(mtu, bitpool).map_err(|_| ERR_AUDIO_CODEC)?;
        self.packetizer.write(packetizer);
        self.packetizer_ready = true;
        self.session_generation = generation;
        self.marker = true;
        Ok(())
    }

    fn reset(&mut self, generation: u32) {
        self.stream.reset();
        self.pcm.fill(0);
        self.pcm_frames = 0;
        self.pcm_offset = 0;
        self.packetizer_ready = false;
        self.session_generation = generation;
        self.marker = true;
    }

    fn ready_for(&self, generation: u32) -> bool {
        self.session_generation == generation && self.encoder_ready && self.packetizer_ready
    }

    fn packetizer(&mut self) -> Result<&mut StreamPacketizer, i32> {
        if !self.packetizer_ready {
            return Err(ERR_MEDIA_STATE);
        }
        // SAFETY: packetizer_ready tracks initialization of this field.
        Ok(unsafe { self.packetizer.assume_init_mut() })
    }

    fn encoder(&mut self) -> Result<&mut SbcEncoder, i32> {
        if !self.encoder_ready {
            return Err(ERR_AUDIO_CODEC);
        }
        // SAFETY: encoder_ready tracks initialization of this field.
        Ok(unsafe { self.encoder.assume_init_mut() })
    }
}

fn queue(command: u8) -> Result<(), i32> {
    let r = runtime();
    let token = unsafe { bt_alloc(size_of::<WorkToken>() as u32) } as *mut WorkToken;
    if token.is_null() {
        return Err(ERR_MEDIA_ALLOC);
    }
    unsafe {
        token.write(WorkToken {
            runtime_generation: r.generation,
            audio_generation: audio_device::input().generation(),
            command: command as u32,
        });
    }
    let owner = unsafe { bt_l2cap_owner() };
    let queued = if owner.is_null() {
        core::ptr::null_mut()
    } else {
        unsafe {
            bt_queue_external(
                owner,
                audio_work,
                bt_queue_free_addr(),
                token.cast(),
                command,
            )
        }
    };
    if queued.is_null() {
        unsafe { bt_free(token.cast()) };
        return Err(ERR_AUDIO_QUEUE);
    }
    Ok(())
}

pub fn schedule_start() -> Result<(), i32> {
    queue(WORK_START)
}

pub fn schedule_wake() -> Result<(), i32> {
    if WAKE_QUEUED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(());
    }
    let r = runtime();
    WAKE_RUNTIME_GENERATION.store(r.generation, Ordering::Release);
    WAKE_AUDIO_GENERATION.store(audio_device::input().generation(), Ordering::Release);
    let owner = unsafe { bt_l2cap_owner() };
    let queued = if owner.is_null() {
        core::ptr::null_mut()
    } else {
        unsafe {
            bt_queue_external(
                owner,
                audio_wake_work,
                audio_wake_cancel as *const () as *mut c_void,
                core::ptr::dangling_mut::<c_void>(),
                WORK_WAKE,
            )
        }
    };
    if queued.is_null() {
        WAKE_QUEUED.store(false, Ordering::Release);
        return Err(ERR_AUDIO_QUEUE);
    }
    Ok(())
}

pub fn schedule_pause() -> Result<(), i32> {
    queue(WORK_PAUSE)
}

pub fn schedule_resume() -> Result<(), i32> {
    queue(WORK_RESUME)
}

pub fn schedule_stop() -> Result<(), i32> {
    queue(WORK_STOP)
}

pub fn schedule_drain() -> Result<(), i32> {
    queue(WORK_DRAIN)
}

pub fn schedule_flush() -> Result<(), i32> {
    queue(WORK_FLUSH)
}

pub fn schedule_close() -> Result<(), i32> {
    queue(WORK_CLOSE)
}

pub fn start_long_test() -> Result<(), i32> {
    if LONG_TEST_OWNS_INPUT.load(Ordering::Acquire) {
        return Err(-16);
    }
    let input = audio_device::input();
    if let Err(error) = input.open() {
        return Err(error.errno());
    }
    let format = FormatV1 {
        struct_size: size_of::<FormatV1>() as u32,
        format: FORMAT_MP3,
        sample_rate_hint: 0,
        channels_hint: 0,
        flags: 0,
        reserved: [0; 3],
    };
    if let Err(error) = input.set_format(&format) {
        let _ = input.close();
        return Err(error.errno());
    }
    LONG_TEST_OWNS_INPUT.store(true, Ordering::Release);
    if let Err(error) = queue(WORK_TEST_PREPARE) {
        release_long_test_input();
        return Err(error);
    }
    Ok(())
}

fn release_long_test_input() {
    let fd = LONG_TEST_FD.swap(-1, Ordering::AcqRel);
    if fd >= 0 {
        unsafe { nuttx_close(fd) };
    }
    if LONG_TEST_OWNS_INPUT.swap(false, Ordering::AcqRel) {
        let _ = audio_device::input().close();
    }
}

fn prepare_long_test() -> i32 {
    if !LONG_TEST_OWNS_INPUT.load(Ordering::Acquire) {
        return 0;
    }
    let fd = unsafe { nuttx_open(LONG_TEST_AUDIO_PATH.as_ptr(), O_RDONLY) };
    if fd < 0 {
        return ERR_AUDIO_DECODE;
    }
    LONG_TEST_FD.store(fd, Ordering::Release);
    let mut scratch = [0u8; INGEST_CHUNK];
    let input = audio_device::input();
    let mut buffered = 0usize;
    while buffered < 4096 {
        let count = scratch.len().min(input.ring.free());
        if count == 0 {
            break;
        }
        let read = unsafe { nuttx_read(fd, scratch.as_mut_ptr().cast(), count as u32) };
        if read <= 0 {
            return ERR_AUDIO_DECODE;
        }
        match input.write(&scratch[..read as usize]) {
            Ok(written) if written == read as usize => buffered += written,
            Ok(_) => return ERR_AUDIO_QUEUE,
            Err(error) => return error.errno(),
        }
    }
    if buffered < 2048 {
        return ERR_AUDIO_DECODE;
    }
    if let Err(error) = input.start() {
        return error.errno();
    }
    match schedule_start() {
        Ok(()) => 0,
        Err(error) => error,
    }
}

fn refill_long_test(
    input: &canopus_bluetooth_audio_core::audio_input::AudioInput<{ audio_device::INPUT_CAPACITY }>,
    scratch: &mut [u8],
) -> i32 {
    if !LONG_TEST_OWNS_INPUT.load(Ordering::Acquire) {
        return 0;
    }
    let fd = LONG_TEST_FD.load(Ordering::Acquire);
    if fd < 0 {
        return 0;
    }
    let mut total = 0usize;
    while total < 4096 {
        let count = scratch.len().min(input.ring.free());
        if count == 0 {
            return 0;
        }
        let read = unsafe { nuttx_read(fd, scratch.as_mut_ptr().cast(), count as u32) };
        if read < 0 {
            return read;
        }
        if read == 0 {
            let closed = LONG_TEST_FD.swap(-1, Ordering::AcqRel);
            if closed >= 0 {
                unsafe { nuttx_close(closed) };
            }
            return match input.drain() {
                Ok(()) => 0,
                Err(error) => error.errno(),
            };
        }
        match input.write(&scratch[..read as usize]) {
            Ok(written) if written == read as usize => total += written,
            Ok(_) => return ERR_AUDIO_QUEUE,
            Err(error) => return error.errno(),
        }
    }
    0
}

extern "C" fn audio_wake_cancel(_owner_valid: i32, _event: i32, _argument: *mut c_void) -> i32 {
    WAKE_QUEUED.store(false, Ordering::Release);
    0
}

extern "C" fn audio_wake_work(_owner_valid: i32, event: i32, _argument: *mut c_void) -> i32 {
    WAKE_QUEUED.store(false, Ordering::Release);
    let r = runtime();
    let generation = WAKE_AUDIO_GENERATION.load(Ordering::Acquire);
    if event != WORK_WAKE as i32
        || WAKE_RUNTIME_GENERATION.load(Ordering::Acquire) != r.generation
        || generation != audio_device::input().generation()
    {
        return 0;
    }
    let result = with_core(|core| dispatch(core, WORK_WAKE, generation));
    if result != 0 {
        audio_device::input().fail(generation, result);
        r.last_error.store(result, Ordering::Release);
        transport::audio_failed(generation);
        if LONG_TEST_OWNS_INPUT.load(Ordering::Acquire) {
            release_long_test_input();
        }
    }
    0
}

extern "C" fn audio_work(_owner_valid: i32, event: i32, argument: *mut c_void) -> i32 {
    if argument.is_null() {
        return 0;
    }
    let token = unsafe { (argument as *const WorkToken).read() };
    unsafe { bt_free(argument) };
    let r = runtime();
    let input = audio_device::input();
    if token.runtime_generation != r.generation
        || token.audio_generation != input.generation()
        || token.command != event as u32
    {
        return 0;
    }
    let result = with_core(|core| dispatch(core, token.command as u8, token.audio_generation));
    if result != 0 {
        input.fail(token.audio_generation, result);
        r.last_error.store(result, Ordering::Release);
        transport::audio_failed(token.audio_generation);
        if LONG_TEST_OWNS_INPUT.load(Ordering::Acquire) {
            release_long_test_input();
        }
    }
    0
}

fn dispatch(core: &mut Core, command: u8, generation: u32) -> i32 {
    match command {
        WORK_START => match transport::start_audio(core, generation) {
            Ok(()) => 0,
            Err(error) => error,
        },
        WORK_WAKE => {
            if audio_device::input().state() == STATE_PAUSED {
                return 0;
            }
            if runtime().media_state.load(Ordering::Acquire) == MEDIA_STREAMING {
                pump(core, generation)
            } else if runtime().media_state.load(Ordering::Acquire) == MEDIA_COMPLETE {
                match transport::start_audio(core, generation) {
                    Ok(()) => 0,
                    Err(error) => error,
                }
            } else {
                0
            }
        }
        WORK_PAUSE => {
            cancel_timer();
            0
        }
        WORK_RESUME => {
            if runtime().media_state.load(Ordering::Acquire) == MEDIA_STREAMING {
                pump(core, generation)
            } else {
                match transport::start_audio(core, generation) {
                    Ok(()) => 0,
                    Err(error) => error,
                }
            }
        }
        WORK_STOP | WORK_FLUSH | WORK_CLOSE => {
            cancel_timer();
            if let Some(pipeline) = pipeline() {
                pipeline.reset(generation);
            }
            transport::complete_audio(core);
            0
        }
        WORK_DRAIN => pump(core, generation),
        WORK_TEST_PREPARE => prepare_long_test(),
        _ => ERR_AUDIO_QUEUE,
    }
}

/// Initializes a new external playback generation after the AVDTP stream has
/// entered Streaming. Called by the transport while holding the Core lock.
pub fn begin(core: &mut Core, generation: u32) -> i32 {
    cancel_timer();
    let input = audio_device::input();
    if input.generation() != generation {
        return 0;
    }
    if let Some(pipeline) = pipeline()
        && pipeline.ready_for(generation)
    {
        pipeline.marker = true;
        runtime()
            .media_state
            .store(MEDIA_STREAMING, Ordering::Release);
        return pump(core, generation);
    }
    let sbc = &core.source.selected_sbc;
    if sbc.frequency_channel != 0x22
        || sbc.blocks_subbands_allocation != 0x15
        || sbc.minimum_bitpool > sbc.maximum_bitpool
    {
        return ERR_MEDIA_STATE;
    }
    let mtu = runtime().media_mtu.load(Ordering::Acquire) as u16;
    let pipeline = match allocate_pipeline() {
        Ok(pipeline) => pipeline,
        Err(error) => return error,
    };
    if let Err(error) = pipeline.begin(generation, mtu, sbc.maximum_bitpool) {
        return error;
    }
    runtime()
        .media_state
        .store(MEDIA_STREAMING, Ordering::Release);
    pump(core, generation)
}

fn pump(core: &mut Core, generation: u32) -> i32 {
    let input = audio_device::input();
    if input.generation() != generation {
        return 0;
    }
    if runtime().audio_timer_handle.load(Ordering::Acquire) != 0 {
        return 0;
    }
    let mut state = input.state();
    if state == STATE_PAUSED {
        return 0;
    }
    if state != STATE_BUFFERING && state != STATE_PLAYING && state != STATE_DRAINING {
        return 0;
    }
    let pipeline = match pipeline() {
        Some(pipeline) if pipeline.session_generation == generation => pipeline,
        _ => return 0,
    };
    let refill = refill_long_test(input, &mut pipeline.ingest);
    if refill != 0 {
        return refill;
    }
    state = input.state();

    while pipeline.stream.free() != 0 && input.ring.used() != 0 {
        let count = pipeline
            .ingest
            .len()
            .min(pipeline.stream.free())
            .min(input.ring.used());
        let consumed = input.consume(&mut pipeline.ingest[..count]);
        if consumed == 0 {
            break;
        }
        let pushed = pipeline.stream.push(&pipeline.ingest[..consumed]);
        if pushed != consumed {
            return ERR_AUDIO_DECODE;
        }
    }

    if pipeline.pcm_frames - pipeline.pcm_offset < FRAME_SAMPLES as usize {
        let carry = pipeline.pcm_frames - pipeline.pcm_offset;
        if carry != 0 {
            pipeline
                .pcm
                .copy_within(pipeline.pcm_offset * 2..pipeline.pcm_frames * 2, 0);
        }
        pipeline.pcm_offset = 0;
        pipeline.pcm_frames = carry;
        let end_of_input = state == STATE_DRAINING && input.ring.used() == 0;
        match pipeline.stream.decode_next(
            input.volume(),
            &mut pipeline.pcm[carry * 2..],
            end_of_input,
        ) {
            Ok(StreamProgress::Frame {
                frames,
                sample_rate,
                channels,
                ..
            }) => {
                if !input.decoded_format_matches(sample_rate, channels as u32)
                    || carry + frames > PCM_FRAME_CAPACITY
                {
                    return ERR_AUDIO_DECODE;
                }
                pipeline.pcm_frames = carry + frames;
                input.add_pcm_frames(generation, frames as u32);
                let bitpool = match pipeline.packetizer() {
                    Ok(packetizer) => packetizer.bitpool as u32,
                    Err(error) => return error,
                };
                input.mark_playing(generation, sample_rate, channels as u32, bitpool);
            }
            Ok(StreamProgress::Skipped { .. }) => return schedule_timer(1, generation),
            Ok(StreamProgress::NeedInput) if end_of_input && carry != 0 => {
                pipeline.pcm[carry * 2..FRAME_SAMPLES as usize * 2].fill(0);
                pipeline.pcm_frames = FRAME_SAMPLES as usize;
            }
            Ok(StreamProgress::NeedInput) if end_of_input => {
                pipeline.stream.discard_incomplete();
                input.mark_drained(generation, true);
                transport::complete_audio(core);
                if LONG_TEST_OWNS_INPUT.load(Ordering::Acquire) {
                    release_long_test_input();
                }
                return 0;
            }
            Ok(StreamProgress::NeedInput) => {
                if state == STATE_PLAYING {
                    input.mark_underrun(generation);
                }
                return 0;
            }
            Err(_) => return ERR_AUDIO_DECODE,
        }
    }

    let remaining_frames = pipeline.pcm_frames - pipeline.pcm_offset;
    let frames_per_packet = match pipeline.packetizer() {
        Ok(packetizer) => packetizer.frames_per_packet as usize,
        Err(error) => return error,
    };
    let packet_frames = (remaining_frames / FRAME_SAMPLES as usize).min(frames_per_packet) as u8;
    if packet_frames == 0 {
        return ERR_AUDIO_DECODE;
    }
    let (frame_length, packet_length) = match pipeline.packetizer() {
        Ok(packetizer) => match packetizer.packet_length(packet_frames) {
            Ok(length) => (packetizer.frame_length as usize, length),
            Err(_) => return ERR_AUDIO_CODEC,
        },
        Err(error) => return error,
    };
    let marker = pipeline.marker;
    let payload = match pipeline.packetizer() {
        Ok(packetizer) => match packetizer.write_header(
            &mut core.media_out[..packet_length],
            packet_frames,
            marker,
        ) {
            Ok(payload) => payload,
            Err(_) => return ERR_AUDIO_CODEC,
        },
        Err(error) => return error,
    };
    pipeline.marker = false;

    for frame_index in 0..packet_frames as usize {
        let pcm_begin = (pipeline.pcm_offset + frame_index * FRAME_SAMPLES as usize) * 2;
        let pcm_end = pcm_begin + FRAME_SAMPLES as usize * 2;
        if pcm_end > pipeline.pcm_frames * 2 {
            return ERR_AUDIO_DECODE;
        }
        // SAFETY: the bounds check above proves this exact 256-sample SBC
        // window lies inside the initialized PCM frame.
        let pcm = unsafe { &*pipeline.pcm.as_ptr().add(pcm_begin).cast::<[i16; 256]>() };
        let frame_begin = payload + frame_index * frame_length;
        let frame_end = frame_begin + frame_length;
        let encoded = match pipeline.encoder() {
            Ok(encoder) => match encoder.encode(pcm, &mut core.media_out[frame_begin..frame_end]) {
                Ok(encoded) => encoded,
                Err(error) => return error,
            },
            Err(error) => return error,
        };
        if encoded != frame_length {
            return ERR_AUDIO_CODEC;
        }
    }
    if input.generation() != generation {
        return 0;
    }
    let send = transport::send_audio_media(&core.media_out[..packet_length]);
    if send != 0 {
        return send;
    }
    pipeline.pcm_offset += packet_frames as usize * FRAME_SAMPLES as usize;
    input.add_rtp_packets(generation, 1);
    let delay = match pipeline.packetizer() {
        Ok(packetizer) => packetizer.next_delay_ms(packet_frames),
        Err(error) => return error,
    };
    schedule_timer(delay, generation)
}

fn schedule_timer(delay_ms: u32, audio_generation: u32) -> i32 {
    let r = runtime();
    if r.audio_timer_handle.load(Ordering::Acquire) != 0 {
        return ERR_MEDIA_STATE;
    }
    let owner = unsafe { bt_l2cap_owner() };
    if owner.is_null() {
        return ERR_AUDIO_QUEUE;
    }
    let token = unsafe { bt_alloc(size_of::<TimerToken>() as u32) } as *mut TimerToken;
    if token.is_null() {
        return ERR_MEDIA_ALLOC;
    }
    unsafe {
        token.write(TimerToken {
            runtime_generation: r.generation,
            audio_generation,
            timer_generation: r.audio_timer_generation.load(Ordering::Acquire),
        });
    }
    let handle = unsafe {
        bt_timer_add(
            owner,
            delay_ms.max(1),
            AUDIO_TIMER_EVENT,
            audio_timer_callback as *const () as *mut c_void,
            token.cast(),
            AUDIO_TIMER_TAG.as_ptr(),
            1,
        )
    };
    if handle == 0 {
        unsafe { bt_free(token.cast()) };
        return ERR_AUDIO_QUEUE;
    }
    r.audio_timer_handle.store(handle, Ordering::Release);
    0
}

extern "C" fn audio_timer_retry(_owner_valid: i32, event: i32, argument: *mut c_void) -> i32 {
    audio_timer_callback_impl(1, event, argument, true)
}

extern "C" fn audio_timer_callback(owner_valid: i32, event: i32, argument: *mut c_void) -> i32 {
    audio_timer_callback_impl(owner_valid, event, argument, false)
}

fn audio_timer_callback_impl(
    owner_valid: i32,
    event: i32,
    argument: *mut c_void,
    blocking: bool,
) -> i32 {
    if argument.is_null() {
        return 0;
    }
    let token = unsafe { &*(argument as *const TimerToken) };
    let r = runtime();
    let timer_generation = r.audio_timer_generation.load(Ordering::Acquire);
    if token.runtime_generation != r.generation || token.timer_generation != timer_generation {
        unsafe { bt_free(argument) };
        return 0;
    }
    if token.audio_generation != audio_device::input().generation() {
        r.audio_timer_handle.store(0, Ordering::Release);
        unsafe { bt_free(argument) };
        return 0;
    }
    let dispatch = |core: &mut Core| {
        r.audio_timer_handle.store(0, Ordering::Release);
        if owner_valid == 0 || event != AUDIO_TIMER_EVENT as i32 {
            ERR_AUDIO_QUEUE
        } else {
            pump(core, token.audio_generation)
        }
    };
    let result = if blocking {
        Some(with_core(dispatch))
    } else {
        super::runtime::try_with_core(dispatch)
    };
    if let Some(result) = result {
        let generation = token.audio_generation;
        unsafe { bt_free(argument) };
        if result != 0 {
            audio_device::input().fail(generation, result);
            r.last_error.store(result, Ordering::Release);
            transport::audio_failed(generation);
            if LONG_TEST_OWNS_INPUT.load(Ordering::Acquire) {
                release_long_test_input();
            }
        }
        return 0;
    }
    if owner_valid != 0
        && transport::queue_audio_retry(audio_timer_retry, AUDIO_TIMER_EVENT, argument)
    {
        return 0;
    }
    let generation = token.audio_generation;
    unsafe { bt_free(argument) };
    audio_device::input().fail(generation, ERR_AUDIO_QUEUE);
    r.last_error.store(ERR_AUDIO_QUEUE, Ordering::Release);
    transport::audio_failed(generation);
    if LONG_TEST_OWNS_INPUT.load(Ordering::Acquire) {
        release_long_test_input();
    }
    0
}

pub fn transport_lost(error: i32) {
    cancel_timer();
    let input = audio_device::input();
    let state = input.state();
    if state == STATE_BUFFERING
        || state == STATE_PLAYING
        || state == STATE_PAUSED
        || state == STATE_DRAINING
    {
        input.fail(input.generation(), error);
    }
    if LONG_TEST_OWNS_INPUT.load(Ordering::Acquire) {
        release_long_test_input();
    }
}

pub fn cancel_timer() {
    let r = runtime();
    r.audio_timer_generation.fetch_add(1, Ordering::AcqRel);
    let mut handle = r.audio_timer_handle.swap(0, Ordering::AcqRel);
    if handle != 0 {
        unsafe { bt_timer_cancel(&mut handle) };
    }
}
