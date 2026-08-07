//! Exact-firmware private backend for `xiaomi-band-10-pro-3.101.030`.
//!
//! These APIs are not part of the public, audited `canopus-target-generated`
//! bindings; every absolute address and ABI record lives in the target-private
//! framework crate. The module itself never hardcodes a firmware address.
//!
//! Activation order (module-owner thread):
//!
//!   1. identity guard against the exact firmware,
//!   2. adapter callback registration,
//!   3. SDP source registration (queued on the Bluetooth owner thread).
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

use canopus_bluetooth_audio_core::{ConnectionState, StreamState, ui};

use runtime::*;

pub mod bluetooth;
pub mod native_app;
pub mod runtime;
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
    if let Err(error) = transport::schedule_initialize() {
        runtime().last_error.store(error, Ordering::Release);
        return error;
    }
    runtime().resident.store(true, Ordering::Release);
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

/// Builds the semantic snapshot for `page_index` from the current model and
/// applies it to the stock LVX page. Runs on the page owner thread only.
pub fn rebuild(page_index: usize) -> i32 {
    let snapshot = with_core(|core| {
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

/// Dispatches a generation-checked LVX event to the model and re-renders.
/// Only ever called from the page owner thread (LVX event callbacks).
pub fn handle_ui_event(page_index: usize, generation: u32, key: u32, event_id: u32) {
    let valid = with_core(|core| {
        let model = &core.controller.model;
        if generation != model.generation {
            return false;
        }
        if event_id >= ui::EVENT_DEVICE_BASE {
            let index = (event_id - ui::EVENT_DEVICE_BASE) as usize;
            return key == 1000u32 + index as u32 && index < model.devices.entries().len();
        }
        matches!(
            (key, event_id),
            (11, ui::EVENT_CONNECTED_DETAIL)
                | (21, ui::EVENT_SCAN)
                | (31, ui::EVENT_REFRESH)
                | (34, ui::EVENT_TEST_TONE)
                | (35, ui::EVENT_DISCONNECT)
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
        with_core(|core| {
            if core.controller.model.connection == ConnectionState::Ready
                && core.controller.model.stream == StreamState::Open
                && transport::play_tone(&mut core.source, &mut core.signaling_out).is_ok()
            {
                core.controller.model.stream = StreamState::Starting;
                core.controller.model.touch();
            }
        });
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
