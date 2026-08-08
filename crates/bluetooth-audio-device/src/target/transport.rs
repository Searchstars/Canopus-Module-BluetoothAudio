//! AVDTP transport: L2CAP signaling and media channels, SDP registration,
//! Bluetooth timer pacing, and the SBC test tone. This is the exact port of the
//! legacy `avdtp_transport.c` production path, driving the portable core
//! `avdtp::Source` and `media::TonePacketizer`.
//!
//! Lock discipline: every function that touches the core state machines takes a
//! `&mut Core` (or is invoked while the caller already holds the core lock).
//! Firmware callbacks enter through [`try_with_core`]; the tone action enters
//! through the blocking [`with_core`]. No transport function ever re-locks.

use core::sync::atomic::Ordering;

use canopus_target_private::*;

use canopus_bluetooth_audio_core::{
    Address, StreamState,
    avdtp::{self, State as SourceState},
    media,
};

use super::runtime::*;
use super::{audio_device, audio_stream, bluetooth};

const MEDIA_TIMER_EVENT: u8 = 9;
const MEDIA_TIMER_TAG: &[u8] = b"A2DPM\0";

// ---------------------------------------------------------------------------
// SDP registration
// ---------------------------------------------------------------------------

/// Queues SDP registration on the Bluetooth owner thread.
pub fn schedule_initialize() -> Result<(), i32> {
    let r = runtime();
    if r.transport_state.load(Ordering::Acquire) != TRANSPORT_DORMANT {
        return Err(ERR_STATE);
    }
    let owner = unsafe { bt_l2cap_owner() };
    if owner.is_null() {
        return Err(ERR_STATE);
    }
    let token = unsafe { bt_alloc(4) } as *mut u32;
    if token.is_null() {
        return Err(ERR_ALLOC);
    }
    unsafe { token.write(r.generation) };
    if r.transport_state
        .compare_exchange(
            TRANSPORT_DORMANT,
            TRANSPORT_INITIALIZING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        unsafe { bt_free(token as *mut core::ffi::c_void) };
        return Err(ERR_STATE);
    }
    let queued = unsafe {
        bt_queue_external(
            owner,
            sdp_work,
            bt_queue_free_addr(),
            token as *mut core::ffi::c_void,
            1,
        )
    };
    if queued.is_null() {
        unsafe { bt_free(token.cast()) };
        r.last_error.store(ERR_SDP, Ordering::Release);
        r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
        return Err(ERR_SDP);
    }
    Ok(())
}

extern "C" fn sdp_work(_unused: i32, event: i32, argument: *mut core::ffi::c_void) -> i32 {
    let r = runtime();
    if argument.is_null() {
        r.last_error.store(ERR_SDP, Ordering::Release);
        r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
        return 0;
    }
    let token = argument as *const u32;
    let valid = event == 1
        && r.transport_state.load(Ordering::Acquire) == TRANSPORT_INITIALIZING
        && unsafe { token.read() } == r.generation;
    if !valid {
        r.last_error.store(ERR_SDP, Ordering::Release);
        r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
        unsafe { bt_free(argument) };
        return 0;
    }
    let builder = unsafe {
        sdp_builder_create(
            0,
            SdpSourceRecord::SERVICE_UUID,
            SdpSourceRecord::PROFILE_VERSION,
            0,
            SdpSourceRecord::SERVICE_NAME.as_ptr(),
        )
    };
    if builder.is_null() {
        r.last_error.store(ERR_SDP, Ordering::Release);
        r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
        unsafe { bt_free(argument) };
        return 0;
    }
    for (id, value) in SdpSourceRecord::ATTRIBUTES {
        unsafe {
            sdp_set_raw_attribute(
                builder,
                id,
                0,
                value.len() as u16,
                value.as_ptr() as *const core::ffi::c_void,
            );
        }
    }
    let handle = unsafe { sdp_commit(builder) };
    if handle == 0 {
        r.last_error.store(ERR_SDP, Ordering::Release);
        r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
    } else {
        r.sdp_handle.store(handle, Ordering::Release);
        r.sdp_registered.store(1, Ordering::Release);
        r.transport_state.store(TRANSPORT_READY, Ordering::Release);
    }
    unsafe { bt_free(argument) };
    0
}

// ---------------------------------------------------------------------------
// Signaling channel
// ---------------------------------------------------------------------------

fn submit_connect(address: &[u8; 6], expected: u32) -> Result<(), i32> {
    let r = runtime();
    if r.sdp_registered.load(Ordering::Acquire) == 0 {
        return Err(ERR_STATE);
    }
    let request = unsafe { bt_alloc(CONNECT_REQUEST_SIZE as u32) } as *mut u8;
    if request.is_null() {
        return Err(ERR_ALLOC);
    }
    unsafe {
        core::ptr::write_bytes(request, 0, CONNECT_REQUEST_SIZE);
        configure_avdtp_connect_request(request);
        // flags at CONNECT_FLAGS_OFFSET stay 0.
        core::ptr::write_unaligned(
            request.add(CONNECT_CALLBACK_OFFSET) as *mut u32,
            l2cap_callback as *const () as usize as u32,
        );
        core::ptr::copy_nonoverlapping(address.as_ptr(), request.add(CONNECT_ADDRESS_OFFSET), 6);
    }
    if r.transport_state
        .compare_exchange(
            expected,
            TRANSPORT_CONNECTING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        unsafe { bt_free(request as *mut core::ffi::c_void) };
        return Err(ERR_STATE);
    }
    r.signaling_cid.store(0, Ordering::Release);
    r.signaling_mtu.store(0, Ordering::Release);
    r.last_error.store(0, Ordering::Release);
    let queued = unsafe { bt_l2cap_connect(request as *mut core::ffi::c_void) };
    if queued == 0 {
        r.last_error.store(ERR_STATE, Ordering::Release);
        let _ = r.transport_state.compare_exchange(
            TRANSPORT_CONNECTING,
            expected,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        return Err(ERR_STATE);
    }
    Ok(())
}

/// Continues the AVDTP connection for `address` from READY (bonded) or from
/// WAIT_BOND (bond just completed).
pub fn connect(address: Address) -> Result<(), i32> {
    let r = runtime();
    if r.media_state.load(Ordering::Acquire) != MEDIA_IDLE {
        return Err(ERR_MEDIA_STATE);
    }
    let state = r.transport_state.load(Ordering::Acquire);
    match state {
        TRANSPORT_READY => submit_connect(&address.0, TRANSPORT_READY),
        TRANSPORT_WAIT_BOND if target_matches(address.0) => {
            submit_connect(&address.0, TRANSPORT_WAIT_BOND)
        }
        _ => Err(ERR_STATE),
    }
}

/// Holds the L2CAP connect until the bond completes. The address keeps the
/// signature symmetric with [`connect`]; the bond callbacks validate it.
pub fn arm_bond_wait(_address: Address) -> Result<(), i32> {
    let r = runtime();
    if r.sdp_registered.load(Ordering::Acquire) == 0
        || r.transport_state
            .compare_exchange(
                TRANSPORT_READY,
                TRANSPORT_WAIT_BOND,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
    {
        return Err(ERR_STATE);
    }
    r.signaling_cid.store(0, Ordering::Release);
    r.signaling_mtu.store(0, Ordering::Release);
    r.last_error.store(0, Ordering::Release);
    Ok(())
}

/// Returns from WAIT_BOND to READY, recording `error` when nonzero.
pub fn bond_failed(address: [u8; 6], error: i32) {
    let r = runtime();
    if r.transport_state.load(Ordering::Acquire) != TRANSPORT_WAIT_BOND || !target_matches(address)
    {
        return;
    }
    if r.transport_state
        .compare_exchange(
            TRANSPORT_WAIT_BOND,
            TRANSPORT_READY,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
        && error != 0
    {
        r.last_error.store(error, Ordering::Release);
    }
}

pub fn disconnect() -> Result<(), i32> {
    let r = runtime();
    let state = r.transport_state.load(Ordering::Acquire);
    if state == TRANSPORT_WAIT_BOND {
        if r.transport_state
            .compare_exchange(
                TRANSPORT_WAIT_BOND,
                TRANSPORT_READY,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return Err(ERRNO_EBUSY);
        }
        if let Some(address) = target_load() {
            bluetooth::cancel_connect_pairing(address);
        }
        return Ok(());
    }
    if (state != TRANSPORT_CONNECTING && state != TRANSPORT_CONNECTED)
        || r.signaling_cid.load(Ordering::Acquire) <= 0x3f
    {
        return Err(ERR_STATE);
    }
    let media_state = r.media_state.load(Ordering::Acquire);
    if r.media_cid.load(Ordering::Acquire) > 0x3f && media_state != MEDIA_DISCONNECTING {
        media_disconnect()?;
    }
    let request = unsafe { bt_alloc(4) } as *mut DisconnectRequest;
    if request.is_null() {
        return Err(ERR_ALLOC);
    }
    unsafe {
        (*request).private_cid = r.signaling_cid.load(Ordering::Acquire) as u16;
        (*request).caller_tag = 0;
    }
    if r.transport_state
        .compare_exchange(
            state,
            TRANSPORT_DISCONNECTING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        unsafe { bt_free(request as *mut core::ffi::c_void) };
        return Err(ERR_STATE);
    }
    unsafe { bt_l2cap_disconnect(request) };
    Ok(())
}

unsafe fn u16_at(p: *const u8, off: usize) -> u16 {
    unsafe { core::ptr::read_unaligned(p.add(off) as *const u16) }
}

fn send_signaling(sdu: &[u8]) -> i32 {
    let r = runtime();
    let cid = r.signaling_cid.load(Ordering::Acquire) as u16;
    if sdu.is_empty() || cid <= 0x3f {
        return ERR_STATE;
    }
    let buffer = unsafe { bt_buffer_new(sdu.len() as u16, 12) };
    if buffer.is_null() {
        return ERR_ALLOC;
    }
    unsafe {
        (*buffer).type_ = 1;
        let payload = stock_buffer_payload_mut(buffer);
        core::ptr::copy_nonoverlapping(sdu.as_ptr(), payload, sdu.len());
        bt_l2cap_submit_cid(buffer, cid);
    }
    0
}

fn send_media(sdu: &[u8]) -> i32 {
    let r = runtime();
    let cid = r.media_cid.load(Ordering::Acquire) as u16;
    if sdu.is_empty() || cid <= 0x3f {
        return ERR_MEDIA_STATE;
    }
    let buffer = unsafe { bt_buffer_new(sdu.len() as u16, 12) };
    if buffer.is_null() {
        return ERR_MEDIA_ALLOC;
    }
    unsafe {
        (*buffer).type_ = 1;
        let payload = stock_buffer_payload_mut(buffer);
        core::ptr::copy_nonoverlapping(sdu.as_ptr(), payload, sdu.len());
        bt_l2cap_submit_cid(buffer, cid);
    }
    0
}

pub(super) fn send_audio_media(sdu: &[u8]) -> i32 {
    send_media(sdu)
}

/// Maps the AVDTP source state into the UI model's stream state.
fn sync_stream_state(core: &mut Core, state: SourceState) {
    core.controller.model.stream = match state {
        SourceState::Discovering => StreamState::Discovering,
        SourceState::ReadingCapabilities => StreamState::ReadingCapabilities,
        SourceState::Configuring => StreamState::Configuring,
        SourceState::Opening => StreamState::Opening,
        SourceState::Open => StreamState::Open,
        SourceState::Starting => StreamState::Starting,
        SourceState::Streaming => StreamState::Streaming,
        SourceState::Suspending => StreamState::Suspending,
        SourceState::Failed => StreamState::Failed,
        SourceState::Idle => StreamState::Idle,
    };
    core.controller.model.touch();
}

/// Handles the AVDTP stream transitions that drive the media channel.
fn handle_stream_transition(core: &mut Core, before: SourceState, after: SourceState) -> i32 {
    let r = runtime();
    match (before, after) {
        (SourceState::Opening, SourceState::Open) => media_submit_connect(),
        (SourceState::Starting, SourceState::Streaming) => {
            if r.media_flags.load(Ordering::Acquire) & MEDIA_FLAG_EXTERNAL_STREAM != 0 {
                audio_stream::begin(core, audio_device::input().generation())
            } else {
                media_begin_tone(core)
            }
        }
        (SourceState::Open, SourceState::Streaming) => {
            // Remote START. Ensure the media channel is ready, then begin.
            let media_state = r.media_state.load(Ordering::Acquire);
            if media_state != MEDIA_CONNECTED && media_state != MEDIA_COMPLETE {
                return ERR_MEDIA_STATE;
            }
            r.media_state.store(MEDIA_STARTING, Ordering::Release);
            if r.media_flags.load(Ordering::Acquire) & MEDIA_FLAG_EXTERNAL_STREAM != 0 {
                audio_stream::begin(core, audio_device::input().generation())
            } else {
                media_begin_tone(core)
            }
        }
        (SourceState::Streaming, SourceState::Open) => {
            // SUSPEND accepted; the producer will issue START again when data is
            // available. Both media timers are generation-cancelled here.
            let external = r.media_flags.load(Ordering::Acquire) & MEDIA_FLAG_EXTERNAL_STREAM != 0;
            media_cancel_timer();
            audio_stream::cancel_timer();
            if external {
                r.media_flags
                    .fetch_or(MEDIA_FLAG_EXTERNAL_STREAM, Ordering::AcqRel);
                let input = audio_device::input();
                input.mark_underrun(input.generation());
                let _ = audio_stream::schedule_wake();
            }
            r.media_state.store(MEDIA_COMPLETE, Ordering::Release);
            0
        }
        (_, SourceState::Idle) => {
            let media_cid = r.media_cid.load(Ordering::Acquire);
            if media_cid > 0x3f {
                if media_disconnect().is_err() {
                    media_cancel_timer();
                    audio_stream::cancel_timer();
                    r.media_generation.fetch_add(1, Ordering::AcqRel);
                    r.media_cid.store(0, Ordering::Release);
                    r.media_mtu.store(0, Ordering::Release);
                    r.media_state.store(MEDIA_IDLE, Ordering::Release);
                }
            } else if r.media_state.load(Ordering::Acquire) != MEDIA_IDLE {
                // CLOSE/ABORT can arrive while the second L2CAP request is still
                // connecting and therefore has no CID to disconnect. Invalidate
                // that request's callback generation before allowing reconnect.
                media_cancel_timer();
                audio_stream::cancel_timer();
                r.media_generation.fetch_add(1, Ordering::AcqRel);
                r.media_mtu.store(0, Ordering::Release);
                r.media_state.store(MEDIA_IDLE, Ordering::Release);
            }
            0
        }
        _ => 0,
    }
}

fn queue_owned_callback(run: QueueWork, event: u8, argument: *mut core::ffi::c_void) -> bool {
    let owner = unsafe { bt_l2cap_owner() };
    if owner.is_null() {
        return false;
    }
    let queued = unsafe { bt_queue_external(owner, run, bt_queue_free_addr(), argument, event) };
    !queued.is_null()
}

pub(super) fn queue_audio_retry(
    run: QueueWork,
    event: u8,
    argument: *mut core::ffi::c_void,
) -> bool {
    queue_owned_callback(run, event, argument)
}

extern "C" fn signaling_retry_work(
    _owner_valid: i32,
    event: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    l2cap_callback_impl(event as u32, argument, true)
}

extern "C" fn l2cap_callback(event: u32, argument: *mut core::ffi::c_void) -> i32 {
    l2cap_callback_impl(event, argument, false)
}

fn l2cap_callback_impl(event: u32, argument: *mut core::ffi::c_void, blocking: bool) -> i32 {
    let r = runtime();
    if argument.is_null() {
        return 0;
    }
    // Initial firmware callbacks never block the Bluetooth stack. If the core
    // is busy, ownership moves to queued Bluetooth-owner work, where a short
    // blocking acquisition guarantees the event is not discarded.
    let dispatch = |core: &mut Core| {
        let packet = argument as *const u8;
        let result = (|| match event {
            EVENT_CONNECTION_CONFIRM => {
                let cid = unsafe { u16_at(packet, 0) };
                let state = r.transport_state.load(Ordering::Acquire);
                if state != TRANSPORT_CONNECTING || cid <= 0x3f {
                    return ERR_STATE;
                }
                r.signaling_cid.store(cid as u32, Ordering::Release);
                0
            }
            EVENT_CONNECTION_COMPLETE => {
                let cid = unsafe { u16_at(packet, EVENT_COMPLETE_CID_OFFSET) };
                let state = r.transport_state.load(Ordering::Acquire);
                if state != TRANSPORT_CONNECTING
                    || cid != r.signaling_cid.load(Ordering::Acquire) as u16
                {
                    return ERR_STATE;
                }
                let mtu = unsafe { u16_at(packet, EVENT_COMPLETE_MTU_OFFSET) };
                r.signaling_mtu.store(mtu as u32, Ordering::Release);
                r.transport_state
                    .store(TRANSPORT_CONNECTED, Ordering::Release);
                if let Some(address) = target_load() {
                    core.controller.connected(Address(address));
                }
                let result = core.source.connected(&mut core.signaling_out);
                sync_stream_state(core, core.source.state);
                match result {
                    Ok(len) if len > 0 => send_signaling(&core.signaling_out[..len]),
                    Ok(_) => 0,
                    Err(_) => ERR_PACKET,
                }
            }
            EVENT_DATA => {
                let total = unsafe { u16_at(packet, 0) } as usize;
                let offset = unsafe { u16_at(packet, 2) } as usize;
                let cid = unsafe { u16_at(packet, 4) };
                let state = r.transport_state.load(Ordering::Acquire);
                if state != TRANSPORT_CONNECTED
                    || cid != r.signaling_cid.load(Ordering::Acquire) as u16
                    || total < offset
                {
                    return ERR_PACKET;
                }
                let input =
                    unsafe { core::slice::from_raw_parts(packet.add(4 + offset), total - offset) };
                let before = core.source.state;
                let result = core.source.receive(input, &mut core.signaling_out);
                let after = core.source.state;
                sync_stream_state(core, after);
                match result {
                    Ok(len) if len > 0 => {
                        let send = send_signaling(&core.signaling_out[..len]);
                        if send != 0 {
                            return send;
                        }
                    }
                    Err(error) => {
                        let failure = match error {
                            avdtp::Error::Rejected(_) => ERR_REMOTE,
                            avdtp::Error::Unsupported => ERR_CODEC_UNSUPPORTED,
                            avdtp::Error::State => ERR_STATE,
                            avdtp::Error::Packet | avdtp::Error::Overflow => ERR_PACKET,
                        };
                        core.source.state = SourceState::Failed;
                        sync_stream_state(core, SourceState::Failed);
                        r.last_error.store(failure, Ordering::Release);
                        r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
                        return failure;
                    }
                    Ok(_) => {}
                }
                handle_stream_transition(core, before, after)
            }
            EVENT_CHANNEL_STATUS_4 | EVENT_CHANNEL_STATUS_5 | EVENT_FLOW_STATUS => 0,
            EVENT_DISCONNECTION_COMPLETE => {
                let cid = unsafe { u16_at(packet, 0) };
                let mtu = unsafe { u16_at(packet, 2) };
                let state = r.transport_state.load(Ordering::Acquire);
                if (state != TRANSPORT_CONNECTING
                    && state != TRANSPORT_CONNECTED
                    && state != TRANSPORT_DISCONNECTING)
                    || r.signaling_cid.load(Ordering::Acquire) <= 0x3f
                    || cid != r.signaling_cid.load(Ordering::Acquire) as u16
                {
                    return ERR_STATE;
                }
                r.signaling_cid.store(0, Ordering::Release);
                r.signaling_mtu.store(mtu as u32, Ordering::Release);
                if mtu != 0 {
                    r.last_error.store(ERR_REMOTE, Ordering::Release);
                }
                media_cancel_timer();
                audio_stream::transport_lost(ERR_MEDIA_REMOTE);
                let media_state = r.media_state.load(Ordering::Acquire);
                let media_cid = r.media_cid.load(Ordering::Acquire);
                let awaiting_media_disconnect = if media_cid > 0x3f {
                    if media_state == MEDIA_DISCONNECTING {
                        true
                    } else {
                        media_disconnect().is_ok()
                    }
                } else {
                    false
                };
                if !awaiting_media_disconnect {
                    // No live CID can produce a completion callback. Invalidate
                    // any in-flight connect callback and finish teardown here.
                    if media_state != MEDIA_IDLE {
                        r.media_generation.fetch_add(1, Ordering::AcqRel);
                    }
                    r.media_cid.store(0, Ordering::Release);
                    r.media_mtu.store(0, Ordering::Release);
                    r.media_state.store(MEDIA_IDLE, Ordering::Release);
                }
                core.source.media_connected = false;
                core.source = avdtp::Source::new(r.generation);
                r.transport_state.store(TRANSPORT_READY, Ordering::Release);
                if let Some(address) = target_load() {
                    bluetooth::cancel_connect_pairing(address);
                    core.controller.disconnected(Address(address));
                }
                0
            }
            _ => ERR_STATE,
        })();
        if result != 0 {
            r.last_error.store(result, Ordering::Release);
            r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
            core.source.state = SourceState::Failed;
            sync_stream_state(core, SourceState::Failed);
        }
        result
    };
    let handled = if blocking {
        Some(with_core(dispatch))
    } else {
        try_with_core(dispatch)
    };
    if handled.is_some() {
        unsafe { bt_free(argument) };
        return 0;
    }
    if event <= u8::MAX as u32 && queue_owned_callback(signaling_retry_work, event as u8, argument)
    {
        return 0;
    }
    r.last_error.store(ERRNO_EBUSY, Ordering::Release);
    r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
    unsafe { bt_free(argument) };
    0
}

// ---------------------------------------------------------------------------
// Media channel
// ---------------------------------------------------------------------------

fn media_submit_connect() -> i32 {
    let r = runtime();
    if r.transport_state.load(Ordering::Acquire) != TRANSPORT_CONNECTED {
        return ERR_MEDIA_STATE;
    }
    if r.media_state
        .compare_exchange(
            MEDIA_IDLE,
            MEDIA_CONNECTING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return ERR_MEDIA_STATE;
    }
    let request = unsafe { bt_alloc(CONNECT_REQUEST_SIZE as u32) } as *mut u8;
    if request.is_null() {
        r.media_state.store(MEDIA_IDLE, Ordering::Release);
        return ERR_MEDIA_ALLOC;
    }
    let Some(address) = target_load() else {
        unsafe { bt_free(request as *mut core::ffi::c_void) };
        r.media_state.store(MEDIA_IDLE, Ordering::Release);
        return ERR_MEDIA_STATE;
    };
    let media_generation = r.media_generation.fetch_add(1, Ordering::AcqRel) + 1;
    let callback = if media_generation & 1 == 0 {
        media_l2cap_callback_even
    } else {
        media_l2cap_callback_odd
    };
    unsafe {
        core::ptr::write_bytes(request, 0, CONNECT_REQUEST_SIZE);
        configure_avdtp_connect_request(request);
        core::ptr::write_unaligned(
            request.add(CONNECT_CALLBACK_OFFSET) as *mut u32,
            callback as *const () as usize as u32,
        );
        core::ptr::copy_nonoverlapping(address.as_ptr(), request.add(CONNECT_ADDRESS_OFFSET), 6);
    }
    r.media_cid.store(0, Ordering::Release);
    r.media_mtu.store(0, Ordering::Release);
    let queued = unsafe { bt_l2cap_connect(request as *mut core::ffi::c_void) };
    if queued == 0 {
        r.last_error.store(ERR_MEDIA_STATE, Ordering::Release);
        r.media_state.store(MEDIA_IDLE, Ordering::Release);
        return ERR_MEDIA_STATE;
    }
    0
}

extern "C" fn media_l2cap_callback_even(event: u32, argument: *mut core::ffi::c_void) -> i32 {
    media_l2cap_callback_impl(event, argument, 0, false)
}

extern "C" fn media_l2cap_callback_odd(event: u32, argument: *mut core::ffi::c_void) -> i32 {
    media_l2cap_callback_impl(event, argument, 1, false)
}

extern "C" fn media_retry_even(
    _owner_valid: i32,
    event: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    media_l2cap_callback_impl(event as u32, argument, 0, true)
}

extern "C" fn media_retry_odd(
    _owner_valid: i32,
    event: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    media_l2cap_callback_impl(event as u32, argument, 1, true)
}

fn media_l2cap_callback_impl(
    event: u32,
    argument: *mut core::ffi::c_void,
    generation_parity: u32,
    blocking: bool,
) -> i32 {
    let r = runtime();
    if argument.is_null() {
        return 0;
    }
    if r.media_generation.load(Ordering::Acquire) & 1 != generation_parity {
        unsafe { bt_free(argument) };
        return 0;
    }
    let dispatch = |core: &mut Core| {
        let packet = argument as *const u8;
        let media_state = r.media_state.load(Ordering::Acquire);
        let result = (|| match event {
            EVENT_CONNECTION_CONFIRM => {
                let cid = unsafe { u16_at(packet, 0) };
                if media_state != MEDIA_CONNECTING || cid <= 0x3f {
                    return ERR_MEDIA_STATE;
                }
                r.media_cid.store(cid as u32, Ordering::Release);
                0
            }
            EVENT_CONNECTION_COMPLETE => {
                let cid = unsafe { u16_at(packet, EVENT_COMPLETE_CID_OFFSET) };
                if cid != r.media_cid.load(Ordering::Acquire) as u16
                    || (media_state != MEDIA_CONNECTING && media_state != MEDIA_DISCONNECTING)
                {
                    return ERR_MEDIA_STATE;
                }
                if media_state == MEDIA_CONNECTING {
                    let mtu = unsafe { u16_at(packet, EVENT_COMPLETE_MTU_OFFSET) };
                    r.media_mtu.store(mtu as u32, Ordering::Release);
                    r.media_state.store(MEDIA_CONNECTED, Ordering::Release);
                    core.source.media_connected = true;
                    let start_pending = r
                        .media_flags
                        .fetch_and(!MEDIA_FLAG_START_WHEN_CONNECTED, Ordering::AcqRel)
                        & MEDIA_FLAG_START_WHEN_CONNECTED
                        != 0;
                    if start_pending {
                        match core.source.state {
                            SourceState::Open => {
                                if let Err(error) =
                                    submit_tone_start(&mut core.source, &mut core.signaling_out)
                                {
                                    return error;
                                }
                            }
                            SourceState::Streaming => {
                                r.media_state.store(MEDIA_STARTING, Ordering::Release);
                                let result = if r.media_flags.load(Ordering::Acquire)
                                    & MEDIA_FLAG_EXTERNAL_STREAM
                                    != 0
                                {
                                    audio_stream::begin(core, audio_device::input().generation())
                                } else {
                                    media_begin_tone(core)
                                };
                                if result != 0 {
                                    return result;
                                }
                                core.controller.model.stream = StreamState::Streaming;
                                core.controller.model.touch();
                            }
                            _ => return ERR_MEDIA_STATE,
                        }
                    } else if core.source.state == SourceState::Open {
                        core.controller.stream_ready();
                    }
                }
                0
            }
            EVENT_CHANNEL_STATUS_4 | EVENT_CHANNEL_STATUS_5 | EVENT_FLOW_STATUS | EVENT_DATA => 0,
            EVENT_DISCONNECTION_COMPLETE => {
                let cid = unsafe { u16_at(packet, 0) };
                let reason = unsafe { u16_at(packet, 2) };
                if cid != r.media_cid.load(Ordering::Acquire) as u16 {
                    return ERR_MEDIA_STATE;
                }
                media_cancel_timer();
                audio_stream::transport_lost(ERR_MEDIA_REMOTE);
                core.source.media_connected = false;
                r.media_cid.store(0, Ordering::Release);
                r.media_mtu.store(0, Ordering::Release);
                r.last_error.store(
                    if reason == 0 { 0 } else { ERR_MEDIA_REMOTE },
                    Ordering::Release,
                );
                r.media_state.store(MEDIA_IDLE, Ordering::Release);
                0
            }
            _ => ERR_MEDIA_STATE,
        })();
        if result != 0 {
            r.last_error.store(result, Ordering::Release);
            r.media_state.store(MEDIA_FAILED, Ordering::Release);
            core.source.media_connected = false;
        }
        result
    };
    let handled = if blocking {
        Some(with_core(dispatch))
    } else {
        try_with_core(dispatch)
    };
    if handled.is_some() {
        unsafe { bt_free(argument) };
        return 0;
    }
    let retry = if generation_parity == 0 {
        media_retry_even
    } else {
        media_retry_odd
    };
    if event <= u8::MAX as u32 && queue_owned_callback(retry, event as u8, argument) {
        return 0;
    }
    r.last_error.store(ERRNO_EBUSY, Ordering::Release);
    r.media_state.store(MEDIA_FAILED, Ordering::Release);
    unsafe { bt_free(argument) };
    0
}

fn media_disconnect() -> Result<(), i32> {
    let r = runtime();
    if r.media_cid.load(Ordering::Acquire) <= 0x3f
        || r.media_state.load(Ordering::Acquire) == MEDIA_DISCONNECTING
    {
        return Err(ERR_MEDIA_STATE);
    }
    let request = unsafe { bt_alloc(4) } as *mut DisconnectRequest;
    if request.is_null() {
        return Err(ERR_MEDIA_ALLOC);
    }
    media_cancel_timer();
    audio_stream::cancel_timer();
    unsafe {
        (*request).private_cid = r.media_cid.load(Ordering::Acquire) as u16;
        (*request).caller_tag = 0;
    }
    r.media_state.store(MEDIA_DISCONNECTING, Ordering::Release);
    unsafe { bt_l2cap_disconnect(request) };
    Ok(())
}

fn media_cancel_timer() {
    let r = runtime();
    let _ = r.media_timer_generation.fetch_add(1, Ordering::AcqRel);
    let mut handle = r.media_timer_handle.swap(0, Ordering::AcqRel);
    if handle != 0 {
        unsafe { bt_timer_cancel(&mut handle) };
    }
    r.media_flags.store(0, Ordering::Release);
}

// ---------------------------------------------------------------------------
// External audio stream
// ---------------------------------------------------------------------------

pub fn start_audio(core: &mut Core, generation: u32) -> Result<(), i32> {
    let r = runtime();
    if audio_device::input().generation() != generation
        || r.media_timer_handle.load(Ordering::Acquire) != 0
    {
        return Err(ERR_MEDIA_STATE);
    }
    let source_state = core.source.state;
    if source_state != SourceState::Open && source_state != SourceState::Streaming {
        return Err(ERR_MEDIA_STATE);
    }
    r.media_flags
        .fetch_or(MEDIA_FLAG_EXTERNAL_STREAM, Ordering::AcqRel);
    match r.media_state.load(Ordering::Acquire) {
        MEDIA_CONNECTED | MEDIA_COMPLETE if source_state == SourceState::Open => {
            submit_tone_start(&mut core.source, &mut core.signaling_out)
        }
        MEDIA_CONNECTED | MEDIA_COMPLETE | MEDIA_STREAMING
            if source_state == SourceState::Streaming =>
        {
            r.media_state.store(MEDIA_STREAMING, Ordering::Release);
            let result = audio_stream::begin(core, generation);
            if result == 0 { Ok(()) } else { Err(result) }
        }
        MEDIA_IDLE => {
            r.media_flags
                .fetch_or(MEDIA_FLAG_START_WHEN_CONNECTED, Ordering::AcqRel);
            let result = media_submit_connect();
            if result != 0 {
                r.media_flags.fetch_and(
                    !(MEDIA_FLAG_START_WHEN_CONNECTED | MEDIA_FLAG_EXTERNAL_STREAM),
                    Ordering::AcqRel,
                );
                return Err(result);
            }
            Ok(())
        }
        MEDIA_CONNECTING | MEDIA_STARTING => {
            r.media_flags
                .fetch_or(MEDIA_FLAG_START_WHEN_CONNECTED, Ordering::AcqRel);
            Ok(())
        }
        _ => {
            r.media_flags
                .fetch_and(!MEDIA_FLAG_EXTERNAL_STREAM, Ordering::AcqRel);
            Err(ERR_MEDIA_STATE)
        }
    }
}

pub fn complete_audio(core: &mut Core) {
    let r = runtime();
    audio_stream::cancel_timer();
    r.media_flags.fetch_and(
        !(MEDIA_FLAG_EXTERNAL_STREAM | MEDIA_FLAG_START_WHEN_CONNECTED),
        Ordering::AcqRel,
    );
    if core.source.state == SourceState::Streaming {
        r.media_state.store(MEDIA_COMPLETE, Ordering::Release);
        core.controller.model.stream = StreamState::Open;
        core.controller.model.touch();
    }
}

pub fn audio_failed(_generation: u32) {
    let r = runtime();
    audio_stream::cancel_timer();
    r.media_flags.fetch_and(
        !(MEDIA_FLAG_EXTERNAL_STREAM | MEDIA_FLAG_START_WHEN_CONNECTED),
        Ordering::AcqRel,
    );
    if r.media_state.load(Ordering::Acquire) == MEDIA_STREAMING {
        r.media_state.store(MEDIA_COMPLETE, Ordering::Release);
    }
    with_core(|core| {
        if core.source.state == SourceState::Open || core.source.state == SourceState::Streaming {
            core.controller.model.stream = StreamState::Open;
            core.controller.model.touch();
        }
    });
}

// ---------------------------------------------------------------------------
// Test tone
// ---------------------------------------------------------------------------

/// Starts immediately when the media channel is live, or reconnects a media
/// channel that the peer closed after the previous tone. After the first
/// accepted START, later tones continue on that live AVDTP stream rather than
/// depending on a peer-specific SUSPEND/START round trip.
pub fn play_tone(core: &mut Core) -> Result<(), i32> {
    let r = runtime();
    if r.media_timer_handle.load(Ordering::Acquire) != 0
        || r.media_flags.load(Ordering::Acquire) & MEDIA_FLAG_EXTERNAL_STREAM != 0
    {
        return Err(ERR_MEDIA_STATE);
    }
    let source_state = core.source.state;
    if source_state != SourceState::Open && source_state != SourceState::Streaming {
        return Err(ERR_MEDIA_STATE);
    }
    match r.media_state.load(Ordering::Acquire) {
        MEDIA_CONNECTED | MEDIA_COMPLETE if source_state == SourceState::Open => {
            submit_tone_start(&mut core.source, &mut core.signaling_out)
        }
        MEDIA_CONNECTED | MEDIA_COMPLETE => {
            r.media_state.store(MEDIA_STARTING, Ordering::Release);
            let result = media_begin_tone(core);
            if result == 0 { Ok(()) } else { Err(result) }
        }
        MEDIA_IDLE => {
            r.media_flags
                .fetch_or(MEDIA_FLAG_START_WHEN_CONNECTED, Ordering::AcqRel);
            let result = media_submit_connect();
            if result != 0 {
                r.media_flags
                    .fetch_and(!MEDIA_FLAG_START_WHEN_CONNECTED, Ordering::AcqRel);
                return Err(result);
            }
            Ok(())
        }
        MEDIA_CONNECTING => {
            r.media_flags
                .fetch_or(MEDIA_FLAG_START_WHEN_CONNECTED, Ordering::AcqRel);
            Ok(())
        }
        _ => Err(ERR_MEDIA_STATE),
    }
}

fn submit_tone_start(source: &mut avdtp::Source, out: &mut [u8]) -> Result<(), i32> {
    let r = runtime();
    let media_state = r.media_state.load(Ordering::Acquire);
    if (media_state != MEDIA_CONNECTED && media_state != MEDIA_COMPLETE)
        || r.media_cid.load(Ordering::Acquire) <= 0x3f
    {
        return Err(ERR_MEDIA_STATE);
    }
    let len = source.start(out).map_err(|_| ERR_MEDIA_STATE)?;
    if len == 0 {
        return Err(ERR_MEDIA_STATE);
    }
    r.media_state.store(MEDIA_STARTING, Ordering::Release);
    let send = send_signaling(&out[..len]);
    if send != 0 {
        r.media_state.store(MEDIA_FAILED, Ordering::Release);
        r.last_error.store(send, Ordering::Release);
        return Err(send);
    }
    Ok(())
}

/// Starts the media stream and the tone once START has been accepted. Runs
/// under the core lock held by the caller.
fn media_begin_tone(core: &mut Core) -> i32 {
    let r = runtime();
    if r.media_state.load(Ordering::Acquire) != MEDIA_STARTING {
        return ERR_MEDIA_STATE;
    }
    // Validate that the negotiated SBC configuration has a matching resident
    // frame. The source selects one fixed bitpool inside the peer's range.
    let sbc = &core.source.selected_sbc;
    if sbc.frequency_channel != 0x22
        || sbc.blocks_subbands_allocation != 0x15
        || sbc.minimum_bitpool > sbc.maximum_bitpool
    {
        r.media_state.store(MEDIA_FAILED, Ordering::Release);
        return ERR_MEDIA_STATE;
    }
    let mtu = r.media_mtu.load(Ordering::Acquire) as u16;
    let packetizer = match media::TonePacketizer::new(mtu, sbc.maximum_bitpool) {
        Ok(packetizer) => packetizer,
        Err(_) => {
            r.media_state.store(MEDIA_FAILED, Ordering::Release);
            return ERR_MEDIA_PACKET;
        }
    };
    core.packetizer = Some(packetizer);
    r.media_state.store(MEDIA_STREAMING, Ordering::Release);
    let startup_packets = core
        .packetizer
        .as_ref()
        .map(|packetizer| packetizer.startup_packets(core.source.reported_delay_100us))
        .unwrap_or(1);
    let result = {
        for _ in 0..startup_packets {
            let out = core.media_out.as_mut_slice();
            let packet_len = match core.packetizer.as_mut() {
                Some(packetizer) => match packetizer.write_packet(out) {
                    Ok(len) => len,
                    Err(_) => return ERR_MEDIA_PACKET,
                },
                None => return ERR_MEDIA_STATE,
            };
            let send = send_media(&out[..packet_len]);
            if send != 0 {
                return send;
            }
            if core.packetizer.as_ref().is_some_and(|p| p.is_complete()) {
                break;
            }
        }
        let complete = core.packetizer.as_ref().is_some_and(|p| p.is_complete());
        sync_media_counters(core, r);
        if complete {
            media_schedule_finish(core)
        } else {
            let delay_ms = core
                .packetizer
                .as_mut()
                .map(|packetizer| packetizer.startup_catchup_delay_ms(startup_packets))
                .unwrap_or(1);
            media_schedule_packet_after(delay_ms)
        }
    };
    if result != 0 {
        r.media_state.store(MEDIA_FAILED, Ordering::Release);
        r.last_error.store(result, Ordering::Release);
    }
    result
}

fn sync_media_counters(core: &mut Core, r: &Runtime) {
    if let Some(packetizer) = core.packetizer.as_ref() {
        r.media_packets_sent
            .store(packetizer.packets_sent, Ordering::Release);
        r.media_frames_sent
            .store(packetizer.frames_sent, Ordering::Release);
        r.media_packets_target
            .store(packetizer.packets_target, Ordering::Release);
        r.media_frames_per_packet
            .store(packetizer.frames_per_packet as u32, Ordering::Release);
        core.controller.model.details.packets_sent = packetizer.packets_sent;
        core.controller.model.details.frames_sent = packetizer.frames_sent;
        core.controller.model.details.bitpool = packetizer.bitpool;
        core.controller.model.details.sbc_frequency_channel = 0x22;
        core.controller.model.details.sbc_blocks_subbands_allocation = 0x15;
    }
}

fn media_schedule_packet(core: &mut Core) -> i32 {
    let Some(packetizer) = core.packetizer.as_mut() else {
        return ERR_MEDIA_STATE;
    };
    media_schedule_packet_after(packetizer.next_delay_ms())
}

fn media_schedule_finish(core: &mut Core) -> i32 {
    let r = runtime();
    let Some(packetizer) = core.packetizer.as_mut() else {
        return ERR_MEDIA_STATE;
    };
    let delay_ms = packetizer.presentation_drain_delay_ms(core.source.reported_delay_100us);
    r.media_flags
        .fetch_or(MEDIA_FLAG_FINISH_ON_TIMER, Ordering::AcqRel);
    let result = media_schedule_packet_after(delay_ms);
    if result != 0 {
        r.media_flags
            .fetch_and(!MEDIA_FLAG_FINISH_ON_TIMER, Ordering::AcqRel);
    }
    result
}

fn media_schedule_packet_after(delay_ms: u32) -> i32 {
    let r = runtime();
    if r.media_timer_handle.load(Ordering::Acquire) != 0
        || r.media_state.load(Ordering::Acquire) != MEDIA_STREAMING
    {
        return ERR_MEDIA_STATE;
    }
    let owner = unsafe { bt_l2cap_owner() };
    if owner.is_null() {
        return ERR_MEDIA_TIMER;
    }
    let token =
        unsafe { bt_alloc(core::mem::size_of::<MediaTimerToken>() as u32) } as *mut MediaTimerToken;
    if token.is_null() {
        return ERR_MEDIA_ALLOC;
    }
    unsafe {
        token.write(MediaTimerToken {
            generation: r.generation,
            timer_generation: r.media_timer_generation.load(Ordering::Acquire),
        });
    }
    let handle = unsafe {
        bt_timer_add(
            owner,
            delay_ms.max(1),
            MEDIA_TIMER_EVENT,
            media_timer_callback as *const () as *mut core::ffi::c_void,
            token as *mut core::ffi::c_void,
            MEDIA_TIMER_TAG.as_ptr(),
            1,
        )
    };
    if handle == 0 {
        unsafe { bt_free(token as *mut core::ffi::c_void) };
        return ERR_MEDIA_TIMER;
    }
    r.media_timer_handle.store(handle, Ordering::Release);
    0
}

extern "C" fn media_timer_retry_work(
    _owner_valid: i32,
    event: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    media_timer_callback_impl(1, event, argument, true)
}

extern "C" fn media_timer_callback(
    owner_valid: i32,
    event: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    media_timer_callback_impl(owner_valid, event, argument, false)
}

fn media_timer_callback_impl(
    owner_valid: i32,
    event: i32,
    argument: *mut core::ffi::c_void,
    blocking: bool,
) -> i32 {
    let r = runtime();
    if argument.is_null() {
        r.media_state.store(MEDIA_FAILED, Ordering::Release);
        return 0;
    }
    let token = argument as *const MediaTimerToken;
    if unsafe { (*token).generation } != r.generation
        || unsafe { (*token).timer_generation } != r.media_timer_generation.load(Ordering::Acquire)
    {
        unsafe { bt_free(argument) };
        return 0;
    }
    let dispatch = |core: &mut Core| {
        r.media_timer_handle.store(0, Ordering::Release);
        if owner_valid == 0
            || event != MEDIA_TIMER_EVENT as i32
            || r.media_state.load(Ordering::Acquire) != MEDIA_STREAMING
            || core.source.state != SourceState::Streaming
        {
            return ERR_MEDIA_STATE;
        }
        if r.media_flags
            .fetch_and(!MEDIA_FLAG_FINISH_ON_TIMER, Ordering::AcqRel)
            & MEDIA_FLAG_FINISH_ON_TIMER
            != 0
        {
            return media_complete_tone(core);
        }
        let out = core.media_out.as_mut_slice();
        let packet_len = match core.packetizer.as_mut() {
            Some(packetizer) => match packetizer.write_packet(out) {
                Ok(len) => len,
                Err(_) => return ERR_MEDIA_PACKET,
            },
            None => return ERR_MEDIA_STATE,
        };
        let send = send_media(&out[..packet_len]);
        if send != 0 {
            return send;
        }
        let complete = core.packetizer.as_ref().is_some_and(|p| p.is_complete());
        sync_media_counters(core, r);
        if complete {
            media_schedule_finish(core)
        } else {
            media_schedule_packet(core)
        }
    };
    let result = if blocking {
        Some(with_core(dispatch))
    } else {
        try_with_core(dispatch)
    };
    if let Some(result) = result {
        unsafe { bt_free(argument) };
        if result != 0 {
            r.media_state.store(MEDIA_FAILED, Ordering::Release);
        }
        return 0;
    }
    if owner_valid != 0 && queue_owned_callback(media_timer_retry_work, MEDIA_TIMER_EVENT, argument)
    {
        return 0;
    }
    unsafe { bt_free(argument) };
    r.last_error.store(ERRNO_EBUSY, Ordering::Release);
    r.media_state.store(MEDIA_FAILED, Ordering::Release);
    0
}

/// Completes one tone while leaving the accepted AVDTP stream active. Reusing
/// the stream avoids depending on peer-specific SUSPEND response timing and is
/// the same lifecycle needed by a later continuous music producer.
fn media_complete_tone(core: &mut Core) -> i32 {
    let r = runtime();
    core.packetizer = None;
    r.media_state.store(MEDIA_COMPLETE, Ordering::Release);
    core.controller.model.stream = StreamState::Open;
    core.controller.model.touch();
    0
}
