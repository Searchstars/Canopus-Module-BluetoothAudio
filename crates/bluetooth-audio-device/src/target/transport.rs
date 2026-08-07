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

use super::bluetooth;
use super::runtime::*;

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
    unsafe {
        bt_queue_external(
            owner,
            sdp_work,
            bt_queue_free_addr(),
            token as *mut core::ffi::c_void,
            1,
        );
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
        core::ptr::write_unaligned(
            request.add(CONNECT_PSM_OFFSET) as *mut u16,
            AVDTP_SIGNALING_PSM,
        );
        // flags at CONNECT_FLAGS_OFFSET stay 0.
        core::ptr::write_unaligned(
            request.add(CONNECT_CALLBACK_OFFSET) as *mut u32,
            l2cap_callback as *const () as usize as u32,
        );
        core::ptr::copy_nonoverlapping(address.as_ptr(), request.add(CONNECT_ADDRESS_OFFSET), 6);
        core::ptr::write_unaligned(
            request.add(CONNECT_CONFIG_OFFSET) as *mut u16,
            AVDTP_L2CAP_CONFIG,
        );
        // options at CONNECT_OPTIONS_OFFSET stay 0.
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
    let result = unsafe { bt_l2cap_connect(request as *mut core::ffi::c_void) };
    if result != 0 {
        r.last_error.store(result, Ordering::Release);
    }
    Ok(())
}

/// Continues the AVDTP connection for `address` from READY (bonded) or from
/// WAIT_BOND (bond just completed).
pub fn connect(address: Address) -> Result<(), i32> {
    let state = runtime().transport_state.load(Ordering::Acquire);
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
        (SourceState::Starting, SourceState::Streaming) => media_begin_tone(core),
        (SourceState::Open, SourceState::Streaming) => {
            // Remote START. Ensure the media channel is ready, then begin.
            let media_state = r.media_state.load(Ordering::Acquire);
            if media_state != MEDIA_CONNECTED && media_state != MEDIA_COMPLETE {
                return ERR_MEDIA_STATE;
            }
            r.media_state.store(MEDIA_STARTING, Ordering::Release);
            media_begin_tone(core)
        }
        (SourceState::Streaming, SourceState::Open) => {
            // SUSPEND accepted; tone complete.
            media_cancel_timer();
            r.media_state.store(MEDIA_COMPLETE, Ordering::Release);
            0
        }
        (SourceState::Idle, _) => {
            if r.media_cid.load(Ordering::Acquire) > 0x3f {
                let _ = media_disconnect();
            }
            0
        }
        _ => 0,
    }
}

extern "C" fn l2cap_callback(event: u32, argument: *mut core::ffi::c_void) -> i32 {
    let r = runtime();
    if argument.is_null() {
        return 0;
    }
    // Route the whole dispatch through the core lock so source + controller
    // stay consistent. A busy lock drops the event rather than blocking.
    let handled = try_with_core(|core| {
        let packet = argument as *const u8;
        match event {
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
                    Err(_) => return ERR_PACKET,
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
                let media_state = r.media_state.load(Ordering::Acquire);
                if r.media_cid.load(Ordering::Acquire) > 0x3f && media_state != MEDIA_DISCONNECTING
                {
                    let _ = media_disconnect();
                }
                core.source = avdtp::Source::new(r.generation);
                r.transport_state.store(TRANSPORT_READY, Ordering::Release);
                if let Some(address) = target_load() {
                    bluetooth::cancel_connect_pairing(address);
                    core.controller.disconnected(Address(address));
                }
                0
            }
            _ => ERR_STATE,
        }
    });
    unsafe { bt_free(argument) };
    handled.unwrap_or(ERR_PACKET)
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
    unsafe {
        core::ptr::write_bytes(request, 0, CONNECT_REQUEST_SIZE);
        core::ptr::write_unaligned(
            request.add(CONNECT_PSM_OFFSET) as *mut u16,
            AVDTP_SIGNALING_PSM,
        );
        core::ptr::write_unaligned(
            request.add(CONNECT_CALLBACK_OFFSET) as *mut u32,
            media_l2cap_callback as *const () as usize as u32,
        );
        core::ptr::copy_nonoverlapping(address.as_ptr(), request.add(CONNECT_ADDRESS_OFFSET), 6);
        core::ptr::write_unaligned(
            request.add(CONNECT_CONFIG_OFFSET) as *mut u16,
            AVDTP_L2CAP_CONFIG,
        );
    }
    r.media_cid.store(0, Ordering::Release);
    r.media_mtu.store(0, Ordering::Release);
    let _ = r.media_generation.fetch_add(1, Ordering::AcqRel);
    unsafe { bt_l2cap_connect(request as *mut core::ffi::c_void) };
    0
}

extern "C" fn media_l2cap_callback(event: u32, argument: *mut core::ffi::c_void) -> i32 {
    let r = runtime();
    if argument.is_null() {
        return 0;
    }
    let handled = try_with_core(|core| {
        let packet = argument as *const u8;
        let media_state = r.media_state.load(Ordering::Acquire);
        match event {
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
                    if core.source.state == SourceState::Open {
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
        }
    });
    unsafe { bt_free(argument) };
    handled.unwrap_or(ERR_MEDIA_STATE)
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
    let mut handle = r.media_timer_handle.load(Ordering::Acquire);
    if handle != 0 {
        unsafe { bt_timer_cancel(&mut handle) };
        r.media_timer_handle.store(0, Ordering::Release);
    }
    r.media_flags.store(0, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Test tone
// ---------------------------------------------------------------------------

/// Builds and sends the AVDTP START for the test tone. Called from the UI
/// action while the core lock is already held.
pub fn play_tone(source: &mut avdtp::Source, out: &mut [u8]) -> Result<(), i32> {
    let r = runtime();
    let media_state = r.media_state.load(Ordering::Acquire);
    if media_state != MEDIA_CONNECTED && media_state != MEDIA_COMPLETE {
        return Err(ERR_MEDIA_STATE);
    }
    if source.state != SourceState::Open
        || r.media_cid.load(Ordering::Acquire) <= 0x3f
        || r.media_timer_handle.load(Ordering::Acquire) != 0
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
    // Validate the negotiated SBC configuration matches the tone frame.
    let sbc = &core.source.selected_sbc;
    if sbc.frequency_channel != 0x22
        || sbc.blocks_subbands_allocation != 0x15
        || sbc.minimum_bitpool > 53
        || sbc.maximum_bitpool < 53
    {
        r.media_state.store(MEDIA_FAILED, Ordering::Release);
        return ERR_MEDIA_STATE;
    }
    let mtu = r.media_mtu.load(Ordering::Acquire) as u16;
    let packetizer = match media::TonePacketizer::new(mtu) {
        Ok(packetizer) => packetizer,
        Err(_) => {
            r.media_state.store(MEDIA_FAILED, Ordering::Release);
            return ERR_MEDIA_PACKET;
        }
    };
    core.packetizer = Some(packetizer);
    r.media_state.store(MEDIA_STREAMING, Ordering::Release);
    let result = {
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
            media_finish_tone(core)
        } else {
            media_schedule_packet(core)
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
        core.controller.model.details.bitpool = 53;
        core.controller.model.details.sbc_frequency_channel = 0x22;
        core.controller.model.details.sbc_blocks_subbands_allocation = 0x15;
    }
}

fn media_schedule_packet(core: &mut Core) -> i32 {
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
    let token = unsafe { bt_alloc(4) } as *mut MediaTimerToken;
    if token.is_null() {
        return ERR_MEDIA_ALLOC;
    }
    let Some(packetizer) = core.packetizer.as_mut() else {
        unsafe { bt_free(token as *mut core::ffi::c_void) };
        return ERR_MEDIA_STATE;
    };
    unsafe { (*token).generation = r.generation };
    let delay_ms = packetizer.next_delay_ms();
    let handle = unsafe {
        bt_timer_add(
            owner,
            delay_ms,
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

extern "C" fn media_timer_callback(
    owner_valid: i32,
    event: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    let r = runtime();
    if argument.is_null() {
        r.media_state.store(MEDIA_FAILED, Ordering::Release);
        return 0;
    }
    let token = argument as *const MediaTimerToken;
    if unsafe { (*token).generation } != r.generation {
        unsafe { bt_free(argument) };
        return 0;
    }
    r.media_timer_handle.store(0, Ordering::Release);
    let result = try_with_core(|core| {
        if owner_valid == 0
            || event != MEDIA_TIMER_EVENT as i32
            || r.media_state.load(Ordering::Acquire) != MEDIA_STREAMING
            || core.source.state != SourceState::Streaming
        {
            return ERR_MEDIA_STATE;
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
            media_finish_tone(core)
        } else {
            media_schedule_packet(core)
        }
    });
    unsafe { bt_free(argument) };
    if result.unwrap_or(ERR_MEDIA_STATE) != 0 {
        r.media_state.store(MEDIA_FAILED, Ordering::Release);
    }
    0
}

/// Sends AVDTP SUSPEND after the last tone packet.
fn media_finish_tone(core: &mut Core) -> i32 {
    let out = core.signaling_out.as_mut_slice();
    let len = match core.source.suspend(out) {
        Ok(len) if len > 0 => len,
        _ => return ERR_MEDIA_STATE,
    };
    let r = runtime();
    r.media_state.store(MEDIA_SUSPENDING, Ordering::Release);
    let send = send_signaling(&out[..len]);
    if send != 0 {
        r.last_error.store(send, Ordering::Release);
        return ERR_MEDIA_PACKET;
    }
    0
}
