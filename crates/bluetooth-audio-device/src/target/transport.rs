//! AVDTP transport: L2CAP signaling and media channels, SDP registration,
//! Bluetooth timer pacing, and the SBC test tone. This is the exact port of the
//! legacy `avdtp_transport.c` production path, driving the portable core
//! `avdtp::Source` and `media::TonePacketizer`.
//!
//! Lock discipline: every function that touches the core state machines takes a
//! `&mut Core` (or is invoked while the caller already holds the core lock).
//! Firmware callbacks and timer-dispatched actions enter the core through
//! [`try_with_core`]. No transport function ever re-locks.

use core::sync::atomic::Ordering;

use canopus_target_private::*;

use canopus_bluetooth_audio_core::{
    Address, StreamState,
    avdtp::{self, State as SourceState},
    avrcp::{self, Event as AvrcpEvent},
    media,
};

use super::runtime::*;
use super::{audio_device, audio_stream, bluetooth, volume_store};

const MEDIA_TIMER_EVENT: u8 = 9;
const MEDIA_TIMER_TAG: &[u8] = b"A2DPM\0";
const DISCONNECT_TIMER_EVENT: u8 = 11;
const TONE_TIMER_EVENT: u8 = 12;
const CALLBACK_RETRY_TAG: &[u8] = b"A2DPR\0";
const CALLBACK_RETRY_DELAY_MS: u32 = 5;
/// Keep at most two media SDUs inside the stock L2CAP queue. Queue acceptance
/// transfers buffer ownership but is not proof that the controller consumed it;
/// event 8 releases one flow credit.
const MAX_MEDIA_TX_OUTSTANDING: u32 = 2;

#[repr(C)]
struct MediaRetryToken {
    generation: u32,
    event: u32,
    argument: *mut core::ffi::c_void,
}

// ---------------------------------------------------------------------------
// SDP registration
// ---------------------------------------------------------------------------

/// Queues SDP registration once the Bluetooth owner exists. A null owner is a
/// transient power-state condition: the adapter ON callback or the next user
/// operation retries without making module activation fail.
pub fn schedule_initialize_if_ready() -> Result<bool, i32> {
    let r = runtime();
    if !flag(FLAG_ADAPTER_ON) {
        return Ok(false);
    }
    match r.transport_state.load(Ordering::Acquire) {
        TRANSPORT_INITIALIZING | TRANSPORT_READY => return Ok(false),
        TRANSPORT_DORMANT => {}
        _ => return Err(ERR_TRANSPORT_STATE),
    }
    let owner = unsafe { bt_l2cap_owner() };
    if owner.is_null() {
        return Ok(false);
    }
    let token = unsafe { bt_alloc(4) } as *mut u32;
    if token.is_null() {
        return Err(ERR_ALLOC);
    }
    unsafe { token.write(r.generation) };
    match r.transport_state.compare_exchange(
        TRANSPORT_DORMANT,
        TRANSPORT_INITIALIZING,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => {}
        Err(TRANSPORT_INITIALIZING | TRANSPORT_READY) => {
            unsafe { bt_free(token.cast()) };
            return Ok(false);
        }
        Err(_) => {
            unsafe { bt_free(token.cast()) };
            return Err(ERR_TRANSPORT_STATE);
        }
    }
    unsafe {
        // The return is the firmware lock-release result, not queue acceptance.
        // With a live owner, stock callers treat external insertion as infallible.
        let _ = bt_queue_external(owner, sdp_work, bt_queue_free_addr(), token.cast(), 1);
    }
    Ok(true)
}

extern "C" fn sdp_work(_unused: i32, event: i32, argument: *mut core::ffi::c_void) -> i32 {
    let r = runtime();
    if argument.is_null() {
        r.last_error.store(ERR_SDP, Ordering::Release);
        r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
        return 0;
    }
    let token = argument as *const u32;
    let generation = unsafe { token.read() };
    if generation != r.generation {
        unsafe { bt_free(argument) };
        return 0;
    }
    if event != 1 || r.transport_state.load(Ordering::Acquire) != TRANSPORT_INITIALIZING {
        r.last_error.store(ERR_SDP, Ordering::Release);
        r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
        unsafe { bt_free(argument) };
        return 0;
    }
    if !flag(FLAG_ADAPTER_ON) {
        let _ = r.transport_state.compare_exchange(
            TRANSPORT_INITIALIZING,
            TRANSPORT_DORMANT,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
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
        unsafe { bt_free(argument) };
        return 0;
    }
    let avrcp_builder = unsafe {
        sdp_builder_create(
            0,
            SdpAvrcpControllerRecord::SERVICE_UUID,
            SdpAvrcpControllerRecord::PROFILE_VERSION,
            0,
            SdpAvrcpControllerRecord::SERVICE_NAME.as_ptr(),
        )
    };
    if avrcp_builder.is_null() {
        unsafe { sdp_unregister(handle) };
        r.last_error.store(ERR_SDP, Ordering::Release);
        r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
        unsafe { bt_free(argument) };
        return 0;
    }
    for (id, value) in SdpAvrcpControllerRecord::ATTRIBUTES {
        unsafe {
            sdp_set_raw_attribute(
                avrcp_builder,
                id,
                0,
                value.len() as u16,
                value.as_ptr().cast(),
            );
        }
    }
    let avrcp_handle = unsafe { sdp_commit(avrcp_builder) };
    if avrcp_handle == 0 {
        unsafe { sdp_unregister(handle) };
        r.last_error.store(ERR_SDP, Ordering::Release);
        r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
    } else {
        r.sdp_handle.store(handle, Ordering::Release);
        r.avrcp_sdp_handle.store(avrcp_handle, Ordering::Release);
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

extern "C" fn disconnect_work(
    _owner_valid: i32,
    event: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    if argument.is_null() {
        return 0;
    }
    let generation = unsafe { argument.cast::<u32>().read() };
    unsafe { bt_free(argument) };
    let r = runtime();
    if event != DISCONNECT_TIMER_EVENT as i32 || generation != r.generation {
        return 0;
    }
    if let Err(error) = disconnect_owned() {
        r.last_error.store(error, Ordering::Release);
        let _ = try_with_core(|core| {
            if core.controller.model.connection
                == canopus_bluetooth_audio_core::ConnectionState::Disconnecting
            {
                core.controller.model.connection =
                    canopus_bluetooth_audio_core::ConnectionState::Ready;
                core.controller.model.touch();
            }
        });
    }
    0
}

/// Dispatches teardown through the stock external-event ring, which signals the
/// Bluetooth FSM instead of waiting for unrelated radio traffic.
pub fn disconnect() -> Result<(), i32> {
    let r = runtime();
    let state = r.transport_state.load(Ordering::Acquire);
    if state != TRANSPORT_WAIT_BOND
        && ((state != TRANSPORT_CONNECTING && state != TRANSPORT_CONNECTED)
            || r.signaling_cid.load(Ordering::Acquire) <= 0x3f)
    {
        return Err(ERR_STATE);
    }
    let owner = unsafe { bt_l2cap_owner() };
    if owner.is_null() {
        return Err(ERR_STATE);
    }
    let token = unsafe { bt_alloc(core::mem::size_of::<u32>() as u32) } as *mut u32;
    if token.is_null() {
        return Err(ERR_ALLOC);
    }
    unsafe { token.write(r.generation) };
    unsafe {
        let _ = bt_queue_external(
            owner,
            disconnect_work,
            bt_queue_free_addr(),
            token.cast(),
            DISCONNECT_TIMER_EVENT,
        );
    }
    Ok(())
}

fn submit_signaling_disconnect() -> Result<(), i32> {
    let r = runtime();
    if r.transport_state.load(Ordering::Acquire) != TRANSPORT_DISCONNECTING
        || r.signaling_cid.load(Ordering::Acquire) <= 0x3f
    {
        return Err(ERR_STATE);
    }
    let request = unsafe { bt_alloc(4) } as *mut DisconnectRequest;
    if request.is_null() {
        return Err(ERR_ALLOC);
    }
    unsafe {
        (*request).private_cid = r.signaling_cid.load(Ordering::Acquire) as u16;
        (*request).caller_tag = 0;
        bt_l2cap_disconnect(request);
    }
    Ok(())
}

fn submit_avrcp_disconnect() -> Result<(), i32> {
    let r = runtime();
    let cid = r.avrcp_cid.load(Ordering::Acquire);
    if cid <= 0x3f {
        return Err(ERR_AVRCP_STATE);
    }
    let request = unsafe { bt_alloc(4) } as *mut DisconnectRequest;
    if request.is_null() {
        return Err(ERR_ALLOC);
    }
    unsafe {
        (*request).private_cid = cid as u16;
        (*request).caller_tag = 0;
    }
    r.avrcp_state.store(AVRCP_DISCONNECTING, Ordering::Release);
    unsafe { bt_l2cap_disconnect(request) };
    Ok(())
}

fn continue_control_disconnect() -> Result<(), i32> {
    if runtime().avrcp_cid.load(Ordering::Acquire) > 0x3f
        && runtime().avrcp_state.load(Ordering::Acquire) != AVRCP_DISCONNECTING
    {
        submit_avrcp_disconnect()
    } else {
        submit_signaling_disconnect()
    }
}

fn disconnect_owned() -> Result<(), i32> {
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
    if r.transport_state
        .compare_exchange(
            state,
            TRANSPORT_DISCONNECTING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return Err(ERRNO_EBUSY);
    }

    // Disconnect the media channel first. Avoid asking this firmware to process
    // two L2CAP disconnect requests concurrently; the previous overlap could
    // leave one request waiting on the stack's long teardown path. The media
    // completion callback submits signaling teardown immediately afterward.
    let media_state = r.media_state.load(Ordering::Acquire);
    if r.media_cid.load(Ordering::Acquire) > 0x3f && media_state != MEDIA_DISCONNECTING {
        if let Err(error) = media_disconnect() {
            r.transport_state.store(state, Ordering::Release);
            return Err(error);
        }
        return Ok(());
    }
    if let Err(error) = continue_control_disconnect() {
        r.transport_state.store(state, Ordering::Release);
        return Err(error);
    }
    Ok(())
}

unsafe fn u16_at(p: *const u8, off: usize) -> u16 {
    unsafe { core::ptr::read_unaligned(p.add(off) as *const u16) }
}

fn send_signaling(sdu: &[u8]) -> i32 {
    let r = runtime();
    let cid = r.signaling_cid.load(Ordering::Acquire) as u16;
    send_l2cap_sdu(sdu, cid, ERR_STATE, ERR_ALLOC)
}

fn send_avrcp(sdu: &[u8]) -> i32 {
    let r = runtime();
    let cid = r.avrcp_cid.load(Ordering::Acquire) as u16;
    let result = send_l2cap_sdu(sdu, cid, ERR_AVRCP_STATE, ERR_ALLOC);
    if result == 0 {
        r.avrcp_packets_sent.fetch_add(1, Ordering::Relaxed);
    }
    result
}

fn send_l2cap_sdu(sdu: &[u8], cid: u16, state_error: i32, alloc_error: i32) -> i32 {
    if sdu.is_empty() || cid <= 0x3f {
        return state_error;
    }
    let buffer = unsafe { bt_buffer_new(sdu.len() as u16, 12) };
    if buffer.is_null() {
        return alloc_error;
    }
    let queued = unsafe {
        (*buffer).type_ = 1;
        let payload = stock_buffer_payload_mut(buffer);
        core::ptr::copy_nonoverlapping(sdu.as_ptr(), payload, sdu.len());
        bt_l2cap_submit_cid(buffer, cid)
    };
    if queued == 0 {
        unsafe { bt_free(buffer.cast()) };
        return ERR_AUDIO_QUEUE;
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
    let queued = unsafe {
        (*buffer).type_ = 1;
        let payload = stock_buffer_payload_mut(buffer);
        core::ptr::copy_nonoverlapping(sdu.as_ptr(), payload, sdu.len());
        bt_l2cap_submit_cid(buffer, cid)
    };
    if queued == 0 {
        unsafe { bt_free(buffer.cast()) };
        return ERR_AUDIO_QUEUE;
    }
    0
}

pub(super) fn media_flow_available() -> bool {
    runtime().media_tx_outstanding.load(Ordering::Acquire) < MAX_MEDIA_TX_OUTSTANDING
}

fn reserve_media_flow_credit() -> bool {
    let outstanding = &runtime().media_tx_outstanding;
    let mut current = outstanding.load(Ordering::Acquire);
    loop {
        if current >= MAX_MEDIA_TX_OUTSTANDING {
            return false;
        }
        match outstanding.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
}

fn return_media_flow_credit() {
    let outstanding = &runtime().media_tx_outstanding;
    let mut current = outstanding.load(Ordering::Acquire);
    while current != 0 {
        match outstanding.compare_exchange_weak(
            current,
            current - 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

fn release_media_flow_credit() {
    let r = runtime();
    let mut outstanding = r.media_tx_outstanding.load(Ordering::Acquire);
    while outstanding != 0 {
        match r.media_tx_outstanding.compare_exchange_weak(
            outstanding,
            outstanding - 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                r.media_packets_completed.fetch_add(1, Ordering::Relaxed);
                break;
            }
            Err(current) => outstanding = current,
        }
    }
}

pub(super) fn reset_audio_media_flow() {
    let r = runtime();
    r.media_tx_outstanding.store(0, Ordering::Release);
    r.media_packets_queued.store(0, Ordering::Release);
    r.media_packets_completed.store(0, Ordering::Release);
    r.media_startup_silence_queued.store(0, Ordering::Release);
}

pub(super) fn send_audio_media(sdu: &[u8]) -> i32 {
    let r = runtime();
    if !reserve_media_flow_credit() {
        return ERRNO_EBUSY;
    }
    let result = send_media(sdu);
    if result == 0 {
        r.media_packets_queued.fetch_add(1, Ordering::Relaxed);
    } else {
        return_media_flow_credit();
    }
    result
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
    // A tail-queued retry can run again before a lower-priority UI lock holder
    // is scheduled. A short one-shot timer yields the CPU and bounds the retry
    // to one attempt without ever spinning in the Bluetooth owner.
    unsafe {
        bt_timer_add(
            owner,
            CALLBACK_RETRY_DELAY_MS,
            event,
            run as *const () as *mut core::ffi::c_void,
            argument,
            CALLBACK_RETRY_TAG.as_ptr(),
            1,
        ) != 0
    }
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
                let avrcp_result = avrcp_submit_connect();
                if avrcp_result != 0 {
                    // AVRCP is an independent optional profile. Keep A2DP usable
                    // for sinks that do not expose AVCTP PSM 23.
                    r.last_error.store(avrcp_result, Ordering::Release);
                    r.avrcp_error.store(avrcp_result, Ordering::Release);
                    r.avrcp_state.store(AVRCP_FAILED, Ordering::Release);
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
    // A queued retry runs in a high-priority Bluetooth owner context. Blocking
    // here can priority-invert the lower-priority lock holder forever, so both
    // the original callback and its single retry must remain non-blocking.
    let handled = try_with_core(dispatch);
    if handled.is_some() {
        unsafe { bt_free(argument) };
        return 0;
    }
    if !blocking
        && event <= u8::MAX as u32
        && queue_owned_callback(signaling_retry_work, event as u8, argument)
    {
        return 0;
    }
    r.last_error.store(ERRNO_EBUSY, Ordering::Release);
    r.transport_state.store(TRANSPORT_FAILED, Ordering::Release);
    unsafe { bt_free(argument) };
    0
}

// ---------------------------------------------------------------------------
// AVRCP control channel
// ---------------------------------------------------------------------------

fn avrcp_submit_connect() -> i32 {
    let r = runtime();
    if r.transport_state.load(Ordering::Acquire) != TRANSPORT_CONNECTED
        || r.avrcp_state
            .compare_exchange(
                AVRCP_IDLE,
                AVRCP_CONNECTING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
    {
        return ERR_AVRCP_STATE;
    }
    let request = unsafe { bt_alloc(CONNECT_REQUEST_SIZE as u32) } as *mut u8;
    if request.is_null() {
        r.avrcp_state.store(AVRCP_IDLE, Ordering::Release);
        return ERR_ALLOC;
    }
    let Some(address) = target_load() else {
        unsafe { bt_free(request.cast()) };
        r.avrcp_state.store(AVRCP_IDLE, Ordering::Release);
        return ERR_AVRCP_STATE;
    };
    unsafe {
        core::ptr::write_bytes(request, 0, CONNECT_REQUEST_SIZE);
        configure_avctp_connect_request(request);
        core::ptr::write_unaligned(
            request.add(CONNECT_CALLBACK_OFFSET).cast::<u32>(),
            avrcp_l2cap_callback as *const () as usize as u32,
        );
        core::ptr::copy_nonoverlapping(address.as_ptr(), request.add(CONNECT_ADDRESS_OFFSET), 6);
    }
    r.avrcp_generation.fetch_add(1, Ordering::AcqRel);
    r.avrcp_last_event.store(0, Ordering::Release);
    r.avrcp_error.store(0, Ordering::Release);
    r.avrcp_packets_sent.store(0, Ordering::Release);
    r.avrcp_packets_received.store(0, Ordering::Release);
    r.avrcp_rx_header.store(0, Ordering::Release);
    r.avrcp_rx_length.store(0, Ordering::Release);
    let volume = volume_store::selected(address, avrcp::DEFAULT_VOLUME);
    r.avrcp_volume.store(volume as u32, Ordering::Release);
    r.avrcp_cid.store(0, Ordering::Release);
    r.avrcp_mtu.store(0, Ordering::Release);
    if unsafe { bt_l2cap_connect(request.cast()) } == 0 {
        r.avrcp_state.store(AVRCP_IDLE, Ordering::Release);
        return ERR_AVRCP_STATE;
    }
    0
}

extern "C" fn avrcp_retry_work(
    _owner_valid: i32,
    event: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    avrcp_l2cap_callback_impl(event as u32, argument, true)
}

extern "C" fn avrcp_l2cap_callback(event: u32, argument: *mut core::ffi::c_void) -> i32 {
    avrcp_l2cap_callback_impl(event, argument, false)
}

fn avrcp_l2cap_callback_impl(event: u32, argument: *mut core::ffi::c_void, blocking: bool) -> i32 {
    if argument.is_null() {
        return 0;
    }
    let r = runtime();
    let packet = argument.cast::<u8>();
    r.avrcp_last_event.store(event, Ordering::Release);
    let dispatch = |core: &mut Core| {
        let result = (|| match event {
            EVENT_CONNECTION_CONFIRM => {
                let cid = unsafe { u16_at(packet, 0) };
                if r.avrcp_state.load(Ordering::Acquire) != AVRCP_CONNECTING || cid <= 0x3f {
                    return ERR_AVRCP_STATE;
                }
                r.avrcp_cid.store(cid as u32, Ordering::Release);
                0
            }
            EVENT_CONNECTION_COMPLETE => {
                let cid = unsafe { u16_at(packet, EVENT_COMPLETE_CID_OFFSET) };
                if r.avrcp_state.load(Ordering::Acquire) != AVRCP_CONNECTING
                    || cid != r.avrcp_cid.load(Ordering::Acquire) as u16
                {
                    return ERR_AVRCP_STATE;
                }
                r.avrcp_mtu.store(
                    unsafe { u16_at(packet, EVENT_COMPLETE_MTU_OFFSET) } as u32,
                    Ordering::Release,
                );
                r.avrcp_state.store(AVRCP_CONNECTED, Ordering::Release);
                let volume = r
                    .avrcp_volume
                    .load(Ordering::Acquire)
                    .min(avrcp::MAX_VOLUME as u32) as u8;
                match core.avrcp.connected_for_volume_sync(volume) {
                    Ok(()) => match core.avrcp.register_volume_notification(&mut core.avrcp_out) {
                        Ok(len) => send_avrcp(&core.avrcp_out[..len]),
                        Err(_) => ERR_AVRCP_STATE,
                    },
                    Err(_) => ERR_AVRCP_STATE,
                }
            }
            EVENT_DATA => {
                let total = unsafe { u16_at(packet, 0) } as usize;
                let offset = unsafe { u16_at(packet, 2) } as usize;
                let cid = unsafe { u16_at(packet, 4) };
                if r.avrcp_state.load(Ordering::Acquire) != AVRCP_CONNECTED
                    || cid != r.avrcp_cid.load(Ordering::Acquire) as u16
                    || total < offset
                {
                    return ERR_AVRCP_PACKET;
                }
                let input =
                    unsafe { core::slice::from_raw_parts(packet.add(4 + offset), total - offset) };
                let mut header = [0u8; 4];
                let header_len = input.len().min(header.len());
                header[..header_len].copy_from_slice(&input[..header_len]);
                r.avrcp_rx_header
                    .store(u32::from_le_bytes(header), Ordering::Release);
                r.avrcp_rx_length
                    .store(input.len() as u32, Ordering::Release);
                r.avrcp_packets_received.fetch_add(1, Ordering::Relaxed);
                match core.avrcp.receive(input, &mut core.avrcp_out) {
                    Ok(AvrcpEvent::Volume(volume)) => {
                        r.avrcp_volume.store(volume as u32, Ordering::Release);
                        volume_store::mark_target_pending(volume);
                        if core.avrcp.state == avrcp::State::Ready {
                            match core.avrcp.register_volume_notification(&mut core.avrcp_out) {
                                Ok(len) => send_avrcp(&core.avrcp_out[..len]),
                                Err(_) => ERR_AVRCP_STATE,
                            }
                        } else {
                            0
                        }
                    }
                    Ok(AvrcpEvent::Reregister) => {
                        let volume = core.avrcp.volume;
                        r.avrcp_volume.store(volume as u32, Ordering::Release);
                        volume_store::mark_target_pending(volume);
                        match core.avrcp.register_volume_notification(&mut core.avrcp_out) {
                            Ok(len) => send_avrcp(&core.avrcp_out[..len]),
                            Err(_) => ERR_AVRCP_STATE,
                        }
                    }
                    Ok(AvrcpEvent::PeerVolume {
                        volume,
                        response_len,
                    }) => {
                        r.avrcp_volume.store(volume as u32, Ordering::Release);
                        volume_store::mark_target_pending(volume);
                        send_avrcp(&core.avrcp_out[..response_len])
                    }
                    Ok(AvrcpEvent::PeerCommand(len)) => send_avrcp(&core.avrcp_out[..len]),
                    Ok(AvrcpEvent::None) => 0,
                    Err(avrcp::Error::Rejected) => {
                        // Absolute volume and notifications are optional peer
                        // features. Record refusal without tearing down AVCTP or
                        // affecting the independent A2DP media stream.
                        r.avrcp_error.store(ERR_AVRCP_REMOTE, Ordering::Release);
                        0
                    }
                    Err(_) => {
                        // A malformed control SDU is isolated to that SDU. Peers
                        // can legally continue other transactions on this channel.
                        r.avrcp_error.store(ERR_AVRCP_PACKET, Ordering::Release);
                        0
                    }
                }
            }
            EVENT_CHANNEL_STATUS_4 | EVENT_CHANNEL_STATUS_5 | EVENT_FLOW_STATUS => 0,
            EVENT_DISCONNECTION_COMPLETE => {
                let cid = unsafe { u16_at(packet, 0) };
                if cid != r.avrcp_cid.load(Ordering::Acquire) as u16 {
                    return ERR_AVRCP_STATE;
                }
                core.avrcp.disconnected();
                r.avrcp_cid.store(0, Ordering::Release);
                r.avrcp_mtu.store(0, Ordering::Release);
                r.avrcp_state.store(AVRCP_IDLE, Ordering::Release);
                if r.transport_state.load(Ordering::Acquire) == TRANSPORT_DISCONNECTING
                    && r.signaling_cid.load(Ordering::Acquire) > 0x3f
                {
                    match submit_signaling_disconnect() {
                        Ok(()) => 0,
                        Err(error) => error,
                    }
                } else {
                    0
                }
            }
            _ => ERR_AVRCP_STATE,
        })();
        if result != 0 {
            r.last_error.store(result, Ordering::Release);
            r.avrcp_error.store(result, Ordering::Release);
            r.avrcp_state.store(AVRCP_FAILED, Ordering::Release);
        }
        result
    };
    let handled = try_with_core(dispatch);
    if handled.is_some() {
        unsafe { bt_free(argument) };
        return 0;
    }
    if !blocking
        && event <= u8::MAX as u32
        && queue_owned_callback(avrcp_retry_work, event as u8, argument)
    {
        return 0;
    }
    r.last_error.store(ERRNO_EBUSY, Ordering::Release);
    r.avrcp_error.store(ERRNO_EBUSY, Ordering::Release);
    r.avrcp_state.store(AVRCP_FAILED, Ordering::Release);
    unsafe { bt_free(argument) };
    0
}

pub fn absolute_volume_percent() -> u32 {
    let volume = runtime()
        .avrcp_volume
        .load(Ordering::Acquire)
        .min(avrcp::MAX_VOLUME as u32) as u8;
    avrcp::absolute_to_percent(volume)
}

pub fn set_absolute_volume(percent: u32) -> Result<(), i32> {
    let volume = avrcp::percent_to_absolute(percent);
    with_core(|core| {
        let r = runtime();
        r.avrcp_volume.store(volume as u32, Ordering::Release);
        volume_store::mark_target_pending(volume);
        if r.avrcp_state.load(Ordering::Acquire) != AVRCP_CONNECTED {
            return Ok(());
        }
        let len = core
            .avrcp
            .set_absolute_volume(volume, &mut core.avrcp_out)
            .map_err(|_| ERR_AVRCP_STATE)?;
        let result = send_avrcp(&core.avrcp_out[..len]);
        if result == 0 { Ok(()) } else { Err(result) }
    })
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
    media_l2cap_callback_impl(event, argument, 0, false, None)
}

extern "C" fn media_l2cap_callback_odd(event: u32, argument: *mut core::ffi::c_void) -> i32 {
    media_l2cap_callback_impl(event, argument, 1, false, None)
}

extern "C" fn media_retry_work(
    _owner_valid: i32,
    event: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    if argument.is_null() {
        return 0;
    }
    let token = unsafe { argument.cast::<MediaRetryToken>().read() };
    unsafe { bt_free(argument) };
    if event != token.event as i32 {
        unsafe { bt_free(token.argument) };
        return 0;
    }
    media_l2cap_callback_impl(
        token.event,
        token.argument,
        token.generation & 1,
        true,
        Some(token.generation),
    )
}

fn queue_media_retry(event: u32, argument: *mut core::ffi::c_void, generation: u32) -> bool {
    if event > u8::MAX as u32 {
        return false;
    }
    let token =
        unsafe { bt_alloc(core::mem::size_of::<MediaRetryToken>() as u32) } as *mut MediaRetryToken;
    if token.is_null() {
        return false;
    }
    unsafe {
        token.write(MediaRetryToken {
            generation,
            event,
            argument,
        });
    }
    let owner = unsafe { bt_l2cap_owner() };
    let queued = !owner.is_null()
        && unsafe {
            bt_timer_add(
                owner,
                CALLBACK_RETRY_DELAY_MS,
                event as u8,
                media_retry_work as *const () as *mut core::ffi::c_void,
                token.cast(),
                CALLBACK_RETRY_TAG.as_ptr(),
                1,
            ) != 0
        };
    if !queued {
        unsafe { bt_free(token.cast()) };
        return false;
    }
    true
}

fn media_l2cap_callback_impl(
    event: u32,
    argument: *mut core::ffi::c_void,
    generation_parity: u32,
    blocking: bool,
    exact_generation: Option<u32>,
) -> i32 {
    let r = runtime();
    if argument.is_null() {
        return 0;
    }
    let media_generation = r.media_generation.load(Ordering::Acquire);
    if media_generation & 1 != generation_parity
        || exact_generation.is_some_and(|generation| generation != media_generation)
    {
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
                    r.media_tx_outstanding.store(0, Ordering::Release);
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
            EVENT_CHANNEL_STATUS_4 | EVENT_CHANNEL_STATUS_5 | EVENT_DATA => 0,
            EVENT_FLOW_STATUS => {
                release_media_flow_credit();
                audio_stream::media_flow_credit();
                0
            }
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
                r.media_tx_outstanding.store(0, Ordering::Release);
                r.last_error.store(
                    if reason == 0 { 0 } else { ERR_MEDIA_REMOTE },
                    Ordering::Release,
                );
                r.media_state.store(MEDIA_IDLE, Ordering::Release);
                if r.transport_state.load(Ordering::Acquire) == TRANSPORT_DISCONNECTING
                    && r.signaling_cid.load(Ordering::Acquire) > 0x3f
                    && let Err(error) = continue_control_disconnect()
                {
                    return error;
                }
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
    // A queued retry runs in a high-priority Bluetooth owner context. Blocking
    // here can priority-invert the lower-priority lock holder forever, so both
    // the original callback and its single retry must remain non-blocking.
    let handled = try_with_core(dispatch);
    if handled.is_some() {
        unsafe { bt_free(argument) };
        return 0;
    }
    if !blocking && queue_media_retry(event, argument, media_generation) {
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
            if result == 0 {
                core.controller.model.stream = StreamState::Streaming;
                core.controller.model.touch();
                Ok(())
            } else {
                Err(result)
            }
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
    if matches!(
        r.media_state.load(Ordering::Acquire),
        MEDIA_STARTING | MEDIA_STREAMING
    ) {
        r.media_state.store(MEDIA_COMPLETE, Ordering::Release);
    }
    let _ = try_with_core(|core| {
        if core.source.state == SourceState::Open || core.source.state == SourceState::Streaming {
            core.controller.model.stream = StreamState::Open;
            core.controller.model.touch();
        }
    });
}

// ---------------------------------------------------------------------------
// Test tone
// ---------------------------------------------------------------------------

extern "C" fn tone_work(_owner_valid: i32, event: i32, argument: *mut core::ffi::c_void) -> i32 {
    if argument.is_null() {
        return 0;
    }
    let generation = unsafe { argument.cast::<u32>().read() };
    unsafe { bt_free(argument) };
    let r = runtime();
    if event != TONE_TIMER_EVENT as i32 || generation != r.generation {
        return 0;
    }
    match try_with_core(|core| {
        let result = play_tone(core);
        if result.is_err() && core.controller.model.stream == StreamState::Starting {
            core.controller.model.stream = StreamState::Open;
            core.controller.model.touch();
        }
        result
    }) {
        Some(Ok(())) => r.last_error.store(0, Ordering::Release),
        Some(Err(error)) => r.last_error.store(error, Ordering::Release),
        None => r.last_error.store(ERRNO_EBUSY, Ordering::Release),
    }
    0
}

pub fn schedule_play_tone() -> Result<(), i32> {
    let r = runtime();
    let owner = unsafe { bt_l2cap_owner() };
    if owner.is_null() {
        return Err(ERR_STATE);
    }
    let token = unsafe { bt_alloc(core::mem::size_of::<u32>() as u32) } as *mut u32;
    if token.is_null() {
        return Err(ERR_ALLOC);
    }
    unsafe { token.write(r.generation) };
    unsafe {
        let _ = bt_queue_external(
            owner,
            tone_work,
            bt_queue_free_addr(),
            token.cast(),
            TONE_TIMER_EVENT,
        );
    }
    Ok(())
}

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
    let result = (|| {
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
    })();
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
    let timer_generation = r.media_timer_generation.load(Ordering::Acquire);
    unsafe {
        token.write(MediaTimerToken {
            generation: r.generation,
            timer_generation,
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
    if r.media_timer_handle
        .compare_exchange(0, handle, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        let mut duplicate = handle;
        unsafe { bt_timer_cancel(&mut duplicate) };
        return ERR_MEDIA_STATE;
    }
    if r.media_timer_generation.load(Ordering::Acquire) != timer_generation
        && r.media_timer_handle
            .compare_exchange(handle, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        let mut stale = handle;
        unsafe { bt_timer_cancel(&mut stale) };
    }
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
    let result = try_with_core(dispatch);
    if let Some(result) = result {
        unsafe { bt_free(argument) };
        if result != 0 {
            r.media_state.store(MEDIA_FAILED, Ordering::Release);
        }
        return 0;
    }
    if !blocking
        && owner_valid != 0
        && queue_owned_callback(media_timer_retry_work, MEDIA_TIMER_EVENT, argument)
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
