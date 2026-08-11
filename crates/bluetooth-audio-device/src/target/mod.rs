//! Target-selected private backend. The current build profile selects
//! `xiaomi-band-10-pro-3.101.030`; future profiles select sibling private ABI
//! implementations without changing this module logic.
//!
//! These APIs are not part of the public, audited `canopus-target-generated`
//! bindings; every absolute address and ABI record lives in the target-private
//! framework crate. The module itself never hardcodes a firmware address.
//!
//! Activation order (module-owner thread):
//!
//!   1. identity guard against the exact firmware,
//!   2. adapter callback registration,
//!   3. SDP source registration when the Bluetooth owner becomes ready.
//!
//! Loading while Bluetooth is OFF is valid. The adapter ON callback performs
//! idempotent transport scheduling and GAP compatibility installation.
//!
//! Native app and Launcher registration are deliberately not part of module
//! activation. `app_install` mutates miwear's process-local app/page registry;
//! invoking it synchronously from an already-running Manager page re-enters
//! that registry and is not rollback-safe when registration is rejected.
//! Launcher publication must use a separate miwear bootstrap transaction.
//!
//! Only after the Bluetooth backend succeeds is the module marked RESIDENT.
//! Once Bluetooth callbacks are published, unload is rejected and requires a
//! reboot.
//!
//! Lock discipline: Bluetooth/timer callbacks enter the core through the
//! non-blocking `try_with_core`; UI actions enter through the blocking
//! `with_core`. Callbacks never touch LVX.

use core::sync::atomic::Ordering;

use canopus_target_private::*;

use canopus_bluetooth_audio_core::{
    ConnectionState, PAIR_DIAG_BONDED, PAIR_DIAG_DISPLAY, PAIR_DIAG_FILTER_HIT,
    PAIR_DIAG_MHDT_FIXED, PAIR_DIAG_REMOVE_CONFIRMED, PAIR_DIAG_REMOVE_PENDING, PAIR_DIAG_REQUEST,
    StreamState, ui,
};

use runtime::*;

pub mod audio_device;
pub mod audio_stream;
pub mod bluetooth;
pub mod compatibility;
pub mod native_app;
pub mod runtime;
pub mod sbc_encoder;
pub mod transport;
pub mod ui_backend;

pub const PAGE_OVERVIEW: usize = native_app::PAGE_OVERVIEW;
pub const PAGE_DETAIL: usize = native_app::PAGE_DETAIL;

/// Resets the fixed static blocks and core state machines. Called from the
/// module constructor exactly once per load.
pub fn prepare(generation: u32) {
    runtime::prepare(generation);
}

/// Verifies identity, publishes the Bluetooth backend, and marks the module
/// boot-resident. Native app registration is a separate bootstrap operation.
/// Returns 0 on success, a module error code otherwise.
pub fn activate() -> i32 {
    if !runtime::initialized() {
        return -1;
    }
    let guard = canopus_identity_guard();
    if guard != 0 {
        runtime().last_error.store(ERR_STATE, Ordering::Release);
        return guard;
    }
    if let Err(error) = bluetooth::register() {
        runtime().last_error.store(error, Ordering::Release);
        return error;
    }
    // Adapter callbacks outlive activation, so unload is unsafe from this point
    // even while Bluetooth is OFF and transport initialization remains deferred.
    runtime().resident.store(true, Ordering::Release);
    if bluetooth::adapter_is_on() {
        if let Err(error) = transport::schedule_initialize_if_ready() {
            runtime().last_error.store(error, Ordering::Release);
            return error;
        }
        // The ON callback and every connection operation retry hook install. A
        // freshly powered stack may report ON before it has populated the slot,
        // so this first attempt is diagnostic rather than an activation gate.
        if let Err(error) = compatibility::install() {
            runtime().last_error.store(error, Ordering::Release);
        }
    }
    if let Err(error) = audio_device::register() {
        runtime().last_error.store(error, Ordering::Release);
        return error;
    }
    // Build the large decoder workspace during one-time activation rather than
    // after AVDTP START has already been accepted. It remains resident and is
    // reset cheaply for each subsequent stream.
    if let Err(error) = with_core(|_| audio_stream::preallocate_pipeline()) {
        runtime().last_error.store(error, Ordering::Release);
        return error;
    }
    0
}

pub fn resident() -> bool {
    runtime().resident.load(Ordering::Acquire)
}

/// Bounded operational status published through `canopus_mod_query`.
pub fn query_status() -> [u32; 20] {
    let r = runtime();
    [
        r.transport_state.load(Ordering::Acquire),
        r.media_state.load(Ordering::Acquire),
        r.adapter_state.load(Ordering::Acquire) as u32,
        r.discovery_state.load(Ordering::Acquire) as u32,
        r.bond_state.load(Ordering::Acquire) as u32,
        r.callback_count.load(Ordering::Relaxed),
        r.callback_dropped.load(Ordering::Relaxed),
        r.discovery_count.load(Ordering::Relaxed),
        r.signaling_cid.load(Ordering::Acquire),
        r.signaling_mtu.load(Ordering::Acquire),
        r.media_cid.load(Ordering::Acquire),
        r.media_mtu.load(Ordering::Acquire),
        r.media_packets_sent.load(Ordering::Relaxed),
        r.media_frames_sent.load(Ordering::Relaxed),
        r.media_packets_target.load(Ordering::Relaxed),
        r.sdp_registered.load(Ordering::Acquire),
        r.app_state.load(Ordering::Acquire),
        r.app_error.load(Ordering::Acquire) as u32,
        r.app_install_result.load(Ordering::Acquire) as u32,
        r.launcher_add_result.load(Ordering::Acquire) as u32,
    ]
}

/// Copies target-owned diagnostics into the semantic model. This runs under the
/// core lock on the page owner thread; Bluetooth callbacks only publish atomics.
fn sync_target_model(core: &mut Core) {
    let r = runtime();
    let mut changed = false;
    let error = r.last_error.load(Ordering::Acquire);
    if core.controller.model.last_error != error {
        core.controller.model.last_error = error;
        changed = true;
    }
    let transport_state = r.transport_state.load(Ordering::Acquire);
    let media_state = r.media_state.load(Ordering::Acquire);
    if transport_state == TRANSPORT_FAILED
        && core.controller.model.connection != ConnectionState::Failed
    {
        core.controller.model.connection = ConnectionState::Failed;
        changed = true;
    }
    if media_state == MEDIA_FAILED && core.controller.model.stream != StreamState::Failed {
        core.controller.model.stream = StreamState::Failed;
        changed = true;
    }
    let details = &mut core.controller.model.details;
    let signaling_cid = r.signaling_cid.load(Ordering::Acquire) as u16;
    let signaling_mtu = r.signaling_mtu.load(Ordering::Acquire) as u16;
    let media_cid = r.media_cid.load(Ordering::Acquire) as u16;
    let media_mtu = r.media_mtu.load(Ordering::Acquire) as u16;
    let stock_bond_state = r.stock_bond_state.load(Ordering::Acquire) as u8;
    let device_bond_state = r.device_bond_state.load(Ordering::Acquire) as u8;
    let runtime_flags = r.flags.load(Ordering::Acquire);
    let mut pairing_flags = 0u8;
    for (runtime_flag, diagnostic_flag) in [
        (FLAG_CORE_FILTER_HIT, PAIR_DIAG_FILTER_HIT),
        (FLAG_HCI_COMPAT_HIT, PAIR_DIAG_MHDT_FIXED),
        (FLAG_PAIR_REQUEST_SEEN, PAIR_DIAG_REQUEST),
        (FLAG_PAIR_DISPLAY_SEEN, PAIR_DIAG_DISPLAY),
        (FLAG_BONDED, PAIR_DIAG_BONDED),
        (FLAG_REMOVE_PENDING, PAIR_DIAG_REMOVE_PENDING),
        (FLAG_REMOVE_CONFIRMED, PAIR_DIAG_REMOVE_CONFIRMED),
    ] {
        if runtime_flags & runtime_flag != 0 {
            pairing_flags |= diagnostic_flag;
        }
    }
    if details.signaling_cid != signaling_cid
        || details.signaling_mtu != signaling_mtu
        || details.media_cid != media_cid
        || details.media_mtu != media_mtu
        || details.stock_bond_state != stock_bond_state
        || details.device_bond_state != device_bond_state
        || details.pairing_flags != pairing_flags
    {
        details.signaling_cid = signaling_cid;
        details.signaling_mtu = signaling_mtu;
        details.media_cid = media_cid;
        details.media_mtu = media_mtu;
        details.stock_bond_state = stock_bond_state;
        details.device_bond_state = device_bond_state;
        details.pairing_flags = pairing_flags;
        changed = true;
    }
    let audio = audio_device::input().status();
    let (
        audio_elapsed_ms,
        decode_cpu_ms,
        startup_ms,
        startup_queue_ms,
        startup_prepare_ms,
        startup_avdtp_ms,
    ) = audio_stream::diagnostic_timing();
    let audio_stage = audio_stream::diagnostic_stage() as u8;
    let media_packets_queued = r.media_packets_queued.load(Ordering::Relaxed);
    let media_flow_events = r.media_packets_completed.load(Ordering::Relaxed);
    let media_tx_outstanding = r.media_tx_outstanding.load(Ordering::Acquire);
    let startup_silence_packets = r.media_startup_silence_queued.load(Ordering::Relaxed);
    let bitpool = if audio.negotiated_bitpool != 0 {
        audio.negotiated_bitpool as u8
    } else {
        details.bitpool
    };
    let audio_error = if audio.last_error != 0 {
        audio.last_error
    } else if audio_stage == audio_stream::AUDIO_STAGE_FAILED as u8 {
        error
    } else {
        0
    };
    if details.bitpool != bitpool
        || details.audio_state != audio.state as u8
        || details.audio_stage != audio_stage
        || details.decoded_channels != audio.decoded_channels as u8
        || details.decoded_sample_rate != audio.decoded_sample_rate
        || details.input_used != audio.input_used
        || details.pcm_frames != audio.pcm_frames
        || details.audio_rtp_packets != audio.rtp_packets
        || details.media_packets_queued != media_packets_queued
        || details.media_flow_events != media_flow_events
        || details.media_tx_outstanding != media_tx_outstanding
        || details.startup_silence_packets != startup_silence_packets
        || details.underruns != audio.underruns
        || details.audio_elapsed_ms != audio_elapsed_ms
        || details.decode_cpu_ms != decode_cpu_ms
        || details.startup_ms != startup_ms
        || details.startup_queue_ms != startup_queue_ms
        || details.startup_prepare_ms != startup_prepare_ms
        || details.startup_avdtp_ms != startup_avdtp_ms
        || details.audio_error != audio_error
    {
        details.bitpool = bitpool;
        details.audio_state = audio.state as u8;
        details.audio_stage = audio_stage;
        details.decoded_channels = audio.decoded_channels as u8;
        details.decoded_sample_rate = audio.decoded_sample_rate;
        details.input_used = audio.input_used;
        details.pcm_frames = audio.pcm_frames;
        details.audio_rtp_packets = audio.rtp_packets;
        details.media_packets_queued = media_packets_queued;
        details.media_flow_events = media_flow_events;
        details.media_tx_outstanding = media_tx_outstanding;
        details.startup_silence_packets = startup_silence_packets;
        details.underruns = audio.underruns;
        details.audio_elapsed_ms = audio_elapsed_ms;
        details.decode_cpu_ms = decode_cpu_ms;
        details.startup_ms = startup_ms;
        details.startup_queue_ms = startup_queue_ms;
        details.startup_prepare_ms = startup_prepare_ms;
        details.startup_avdtp_ms = startup_avdtp_ms;
        details.audio_error = audio_error;
        changed = true;
    }
    if changed {
        core.controller.model.touch();
    }
}

/// Builds the semantic snapshot for `page_index` from the current model and
/// applies it to the stock LVX page. Runs on the page owner thread only.
pub fn rebuild(page_index: usize) -> i32 {
    let snapshot = with_core(|core| {
        sync_target_model(core);
        let model = &core.controller.model;
        let built = if page_index == PAGE_DETAIL {
            ui::detail(model)
        } else {
            ui::overview(model)
        };
        built.map(|mut snap| {
            snap.generation = model.generation;
            snap
        })
    });
    match snapshot {
        Ok(snap) => ui_backend::apply_snapshot(page_index, &snap),
        Err(_) => -1,
    }
}

/// Rebuilds an active page only when callbacks have committed a newer model.
/// The LVGL timer invoking this function belongs to the page owner thread, so
/// applying the snapshot never crosses UI-thread ownership.
pub fn rebuild_if_changed(page_index: usize, rendered_generation: u32) -> i32 {
    let snapshot = match try_with_core(|core| {
        sync_target_model(core);
        let model = &core.controller.model;
        if model.generation == rendered_generation {
            return None;
        }
        let built = if page_index == PAGE_DETAIL {
            ui::detail(model)
        } else {
            ui::overview(model)
        };
        Some(built.map(|mut snap| {
            snap.generation = model.generation;
            snap
        }))
    }) {
        Some(snapshot) => snapshot,
        None => return 0,
    };
    match snapshot {
        None => 0,
        Some(Ok(snap)) => ui_backend::apply_snapshot(page_index, &snap),
        Some(Err(_)) => -1,
    }
}

/// Dispatches a generation-checked LVX event to the model and re-renders.
/// Only ever called from the page owner thread (LVX event callbacks).
pub fn handle_ui_event(page_index: usize, generation: u32, key: u32, event_id: u32) {
    let valid = with_core(|core| {
        let model = &core.controller.model;
        // Refresh must remain usable when asynchronous callbacks have advanced
        // the model beyond the generation rendered into this row. Other actions
        // keep strict generation checks so stale device-index bindings cannot
        // target a different discovery result.
        if (key, event_id) == (31, ui::EVENT_REFRESH) {
            return true;
        }
        if generation != model.generation {
            return false;
        }
        if event_id >= ui::EVENT_DEVICE_BASE {
            let index = (event_id - ui::EVENT_DEVICE_BASE) as usize;
            return model
                .devices
                .entries()
                .get(index)
                .is_some_and(|device| key == ui::device_key(device.address));
        }
        matches!(
            (key, event_id),
            (11, ui::EVENT_CONNECTED_DETAIL)
                | (21, ui::EVENT_SCAN)
                | (31, ui::EVENT_REFRESH)
                | (34, ui::EVENT_TEST_TONE)
                | (35, ui::EVENT_LONG_MP3)
                | (44, ui::EVENT_LONG_MP3_DECODE_ONLY)
                | (36, ui::EVENT_DISCONNECT)
                | (1, ui::EVENT_BACK)
        )
    });
    if valid {
        handle_action(page_index, event_id);
    }
}

fn handle_action(page_index: usize, event_id: u32) {
    use canopus_bluetooth_audio_core::ScanState;

    if event_id == ui::EVENT_CONNECTED_DETAIL {
        ui_backend::navigate(PAGE_DETAIL);
        return;
    }
    if event_id == ui::EVENT_BACK {
        ui_backend::back(page_index);
        return;
    }
    if event_id == ui::EVENT_SCAN {
        with_core(|core| {
            if matches!(
                core.controller.model.scan,
                ScanState::Scanning | ScanState::Starting
            ) {
                let _ = core.controller.stop_scan();
            } else {
                let _ = core.controller.start_scan();
            }
        });
        rebuild(page_index);
        return;
    }
    if event_id == ui::EVENT_REFRESH {
        rebuild(page_index);
        return;
    }
    if event_id == ui::EVENT_DISCONNECT {
        with_core(|core| {
            let _ = core.controller.disconnect();
        });
        rebuild(page_index);
        return;
    }
    if event_id == ui::EVENT_TEST_TONE {
        let ready = with_core(|core| {
            if core.controller.model.connection == ConnectionState::Ready
                && core.controller.model.stream == StreamState::Open
            {
                core.controller.model.stream = StreamState::Starting;
                core.controller.model.touch();
                true
            } else {
                false
            }
        });
        if ready {
            match transport::schedule_play_tone() {
                Ok(()) => runtime().last_error.store(0, Ordering::Release),
                Err(error) => {
                    runtime().last_error.store(error, Ordering::Release);
                    with_core(|core| {
                        if core.controller.model.stream == StreamState::Starting {
                            core.controller.model.stream = StreamState::Open;
                            core.controller.model.touch();
                        }
                    });
                }
            }
        }
        rebuild(page_index);
        return;
    }
    if event_id == ui::EVENT_LONG_MP3 {
        // Publish Starting before enqueueing work, but never enqueue while the
        // UI context owns CORE_LOCK. The Bluetooth owner may run the work
        // immediately; queueing under the lock creates a priority inversion.
        let ready = with_core(|core| {
            if core.controller.model.connection == ConnectionState::Ready
                && core.controller.model.stream == StreamState::Open
            {
                core.controller.model.stream = StreamState::Starting;
                core.controller.model.touch();
                true
            } else {
                false
            }
        });
        if ready {
            match audio_stream::start_long_test() {
                Ok(()) => runtime().last_error.store(0, Ordering::Release),
                Err(error) => {
                    runtime().last_error.store(error, Ordering::Release);
                    with_core(|core| {
                        if core.controller.model.stream == StreamState::Starting {
                            core.controller.model.stream = StreamState::Open;
                            core.controller.model.touch();
                        }
                    });
                }
            }
        }
        rebuild(page_index);
        return;
    }
    if event_id == ui::EVENT_LONG_MP3_DECODE_ONLY {
        let ready = with_core(|core| {
            core.controller.model.connection == ConnectionState::Ready
                && core.controller.model.stream == StreamState::Open
        });
        if ready {
            match audio_stream::start_long_test_decode_only() {
                Ok(()) => runtime().last_error.store(0, Ordering::Release),
                Err(error) => runtime().last_error.store(error, Ordering::Release),
            }
        }
        rebuild(page_index);
        return;
    }
    if event_id >= ui::EVENT_DEVICE_BASE {
        let index = (event_id - ui::EVENT_DEVICE_BASE) as usize;
        with_core(|core| {
            let _ = core.controller.select(index);
        });
        rebuild(page_index);
    }
}
