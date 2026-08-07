//! Adapter client: registration, callback filtering, discovery table feed,
//! and the pair-if-needed transaction, driven by the portable controller.

use core::sync::atomic::Ordering;

use canopus_target_private::*;

use canopus_bluetooth_audio_core::{Address, DeviceName, DiscoveredDevice, Platform};

use super::runtime::*;
use super::transport;

pub struct DevicePlatform;

const ERR_CORE_POLICY: i32 = -1107;

type PairRequestCallback = extern "C" fn(*mut core::ffi::c_void, *const u8);

extern "C" fn core_pair_request_filter(cookie: *mut core::ffi::c_void, address: *const u8) {
    let addr = address_from_ptr(address);
    if !address.is_null() && flag(FLAG_BOND_PENDING) && target_matches(addr) {
        flag_set(FLAG_CORE_FILTER_HIT, 0);
        return;
    }
    let original: PairRequestCallback =
        unsafe { core::mem::transmute(CORE_BT_PAIR_REQUEST_CALLBACK) };
    original(cookie, address);
}

fn install_core_pair_filter(address: [u8; 6]) -> Result<(), i32> {
    let r = runtime();
    if unsafe { core_bt_bind_state() } != CORE_BT_BOUND_STATE {
        return Ok(());
    }
    let companion = unsafe { core_bt_companion() };
    let mut companion_match = true;
    for (index, value) in address.iter().enumerate() {
        if unsafe { *companion.add(index) } != *value {
            companion_match = false;
            break;
        }
    }
    if companion_match {
        return Ok(());
    }
    if r.core_filter_table.load(Ordering::Acquire) != 0 {
        return Ok(());
    }
    let adapter = unsafe { core_bt_adapter() };
    let handle_ptr = unsafe { core_bt_callback_handle() };
    let original_handle = unsafe { *handle_ptr };
    let stock = unsafe { core_bt_callback_table() };
    if adapter.is_null()
        || original_handle == 0
        || unsafe { *stock.add(CORE_BT_PAIR_REQUEST_SLOT) } as usize
            != CORE_BT_PAIR_REQUEST_CALLBACK
    {
        return Err(ERR_CORE_POLICY);
    }
    let mirror = unsafe { bt_alloc((CALLBACK_WORDS * 4) as u32) } as *mut u32;
    if mirror.is_null() {
        return Err(ERR_ALLOC);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(stock, mirror, CALLBACK_WORDS);
        *mirror.add(CORE_BT_PAIR_REQUEST_SLOT) =
            core_pair_request_filter as *const () as usize as u32;
    }
    let mirror_handle = unsafe { bt_adapter_register(adapter, mirror) };
    if mirror_handle == 0 {
        unsafe { bt_free(mirror.cast()) };
        return Err(ERR_CORE_POLICY);
    }
    if unsafe { bt_adapter_unregister(adapter, original_handle) } == 0 {
        unsafe {
            bt_adapter_unregister(adapter, mirror_handle);
            bt_free(mirror.cast());
        }
        return Err(ERR_CORE_POLICY);
    }
    unsafe { *handle_ptr = mirror_handle };
    r.core_filter_table
        .store(mirror as usize, Ordering::Release);
    r.core_filter_handle.store(mirror_handle, Ordering::Release);
    flag_set(FLAG_CORE_FILTER_INSTALLED, 0);
    Ok(())
}

impl Platform for DevicePlatform {
    type Error = i32;

    fn start_discovery(&mut self, timeout_seconds: u8) -> Result<(), i32> {
        let adapter = adapter();
        let result = unsafe { bt_discovery_start(adapter, timeout_seconds as i32) };
        if result == 0 {
            Ok(())
        } else {
            runtime().last_error.store(result, Ordering::Release);
            Err(result)
        }
    }

    fn stop_discovery(&mut self) -> Result<(), i32> {
        runtime().scan_stop_pending.store(1, Ordering::Release);
        let adapter = adapter();
        let result = unsafe { bt_discovery_stop(adapter) };
        if result == 0 {
            Ok(())
        } else {
            runtime().last_error.store(result, Ordering::Release);
            Err(result)
        }
    }

    fn is_bonded(&mut self, address: Address) -> Result<bool, i32> {
        query_bond(address);
        let r = runtime();
        Ok(r.stock_bond_state.load(Ordering::Acquire) == 3)
    }

    fn create_bond(&mut self, address: Address) -> Result<(), i32> {
        begin_bond(address)
    }

    fn connect_avdtp(&mut self, address: Address) -> Result<(), i32> {
        transport::connect(address)
    }

    fn disconnect_avdtp(&mut self, _address: Address) -> Result<(), i32> {
        transport::disconnect()
    }

    fn play_test_tone(&mut self) -> Result<(), i32> {
        // The tone needs the AVDTP source, which lives in the core lock, not
        // on the platform. The UI action handler drives it directly via
        // `transport::play_tone` under `with_core`; this seam is unreachable
        // on device and only exercised by host fakes.
        Err(ERRNO_ENOSYS)
    }
}

fn adapter() -> *mut core::ffi::c_void {
    runtime().adapter.load(Ordering::Acquire) as *mut core::ffi::c_void
}

/// Reads the stock bond state for `address`.
fn query_bond(address: Address) {
    let r = runtime();
    let stock = unsafe { bt_get_bond_state(address.0.as_ptr()) };
    let device = unsafe { bt_get_pairing_state(address.0.as_ptr(), CLASSIC_TRANSPORT) };
    let bonded = stock == 3 || device == BOND_STATE_BONDED;
    r.stock_bond_state
        .store(if bonded { 3 } else { stock }, Ordering::Release);
    r.last_error.store(0, Ordering::Release);
}

/// Registers the adapter client and publishes the persistent callback table.
pub fn register() -> Result<(), i32> {
    let r = runtime();
    if r.registration_state.load(Ordering::Acquire) == REGISTRATION_COMPLETE {
        return Ok(());
    }
    let adapter = unsafe { bt_adapter_get_instance() };
    if adapter.is_null() {
        r.registration_state
            .store(REGISTRATION_FAILED, Ordering::Release);
        return Err(ERR_STATE);
    }
    r.adapter.store(adapter as usize, Ordering::Release);
    r.callbacks[CALLBACK_ADAPTER_STATE] = on_adapter_state as *const () as usize as u32;
    r.callbacks[CALLBACK_DISCOVERY_STATE] = on_discovery_state as *const () as usize as u32;
    r.callbacks[CALLBACK_DISCOVERY_RESULT] = on_discovery_result as *const () as usize as u32;
    r.callbacks[CALLBACK_PAIR_REQUEST] = on_pair_request as *const () as usize as u32;
    r.callbacks[CALLBACK_PAIR_DISPLAY] = on_pair_display as *const () as usize as u32;
    r.callbacks[CALLBACK_BOND_STATE] = on_bond_state as *const () as usize as u32;
    let handle = unsafe { bt_adapter_register(adapter, r.callbacks.as_ptr()) };
    if handle == 0 {
        r.registration_state
            .store(REGISTRATION_FAILED, Ordering::Release);
        return Err(ERR_STATE);
    }
    r.registration_state
        .store(REGISTRATION_COMPLETE, Ordering::Release);
    flag_set(FLAG_ADAPTER_REGISTERED, 0);
    let state = unsafe { bt_adapter_get_state(adapter) };
    r.adapter_state.store(state, Ordering::Release);
    if state == ADAPTER_STATE_ON {
        flag_set(FLAG_ADAPTER_ON, 0);
    }
    Ok(())
}

/// Starts the pair-if-needed transaction: records the target, arms the
/// transport bond wait, and submits the stock bond.
pub fn begin_bond(address: Address) -> Result<(), i32> {
    let r = runtime();
    target_store(address.0);
    install_core_pair_filter(address.0)?;
    flag_set(FLAG_TARGET_SEEN | FLAG_BOND_PENDING, FLAG_BONDED);
    r.bond_transport
        .store(CLASSIC_TRANSPORT as i32, Ordering::Release);
    r.bond_state.store(1, Ordering::Release);
    transport::arm_bond_wait(address)?;
    match submit_bond() {
        Ok(()) => Ok(()),
        Err(error) => {
            // Do not leave the transport parked in WAIT_BOND with no bond in
            // flight; return to READY so a later retry can connect cleanly.
            transport::bond_failed(address.0, error);
            Err(error)
        }
    }
}

fn submit_bond() -> Result<(), i32> {
    let r = runtime();
    if !flag(FLAG_ADAPTER_ON) || !flag(FLAG_BOND_PENDING) {
        r.last_error.store(ERR_STATE, Ordering::Release);
        return Err(ERR_STATE);
    }
    let Some(address) = target_load() else {
        r.last_error.store(ERR_STATE, Ordering::Release);
        return Err(ERR_STATE);
    };
    let pairing = unsafe { bt_get_pairing_state(address.as_ptr(), CLASSIC_TRANSPORT) };
    if pairing != 0 {
        r.last_error.store(pairing as i32, Ordering::Release);
        return Err(ERRNO_EBUSY);
    }
    let result = unsafe { bt_create_bond(address.as_ptr(), CLASSIC_TRANSPORT) };
    if result == 0 {
        flag_set(0, 0);
        r.last_error.store(0, Ordering::Release);
        Ok(())
    } else {
        r.bond_state
            .store(BOND_STATE_NONE as i32, Ordering::Release);
        if result != CREATE_BOND_ADAPTER_NOT_READY {
            flag_set(0, FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY);
        }
        r.last_error.store(result, Ordering::Release);
        Err(result)
    }
}

/// Clears a pending pairing transaction for a matching address.
pub fn cancel_connect_pairing(address: [u8; 6]) {
    if !target_matches(address) || flag(FLAG_BONDED) {
        return;
    }
    runtime()
        .bond_state
        .store(BOND_STATE_NONE as i32, Ordering::Release);
    flag_set(0, FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY);
}

fn address_from_ptr(ptr: *const u8) -> [u8; 6] {
    let mut out = [0u8; 6];
    if !ptr.is_null() {
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = unsafe { *ptr.add(i) };
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Adapter callbacks (firmware -> module). No LVX is ever touched here.
// ---------------------------------------------------------------------------

unsafe extern "C" fn on_adapter_state(_cookie: *mut core::ffi::c_void, state: i32) {
    let r = runtime();
    r.callback_count.fetch_add(1, Ordering::Relaxed);
    r.adapter_state.store(state, Ordering::Release);
    if state == ADAPTER_STATE_ON {
        flag_set(FLAG_ADAPTER_ON, 0);
    } else {
        flag_set(0, FLAG_ADAPTER_ON);
        if r.transport_state.load(Ordering::Acquire) == TRANSPORT_WAIT_BOND {
            if let Some(address) = target_load() {
                transport::bond_failed(address, ERR_STATE);
            }
            flag_set(0, FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY);
        }
    }
}

unsafe extern "C" fn on_discovery_state(_cookie: *mut core::ffi::c_void, state: i32) {
    let r = runtime();
    r.callback_count.fetch_add(1, Ordering::Relaxed);
    r.discovery_state.store(state, Ordering::Release);
    if state == DISCOVERY_STOPPED {
        r.scan_stop_pending.swap(0, Ordering::AcqRel);
        flag_set(0, FLAG_DISCOVERY_ACTIVE);
        // The controller resolves WaitingForScanStop -> bond/connect here.
        try_with_core(|core| {
            let _ = core.controller.discovery_stopped();
        });
    } else {
        flag_set(FLAG_DISCOVERY_ACTIVE, 0);
    }
}

unsafe extern "C" fn on_discovery_result(
    _cookie: *mut core::ffi::c_void,
    result: *const DiscoveryResult,
) {
    let r = runtime();
    r.callback_count.fetch_add(1, Ordering::Relaxed);
    r.discovery_count.fetch_add(1, Ordering::Relaxed);
    if result.is_null() || !flag(FLAG_DISCOVERY_ACTIVE) {
        return;
    }
    // Bounded name read straight from the firmware payload.
    let name = unsafe { discovery_name(result, 128) };
    let device = DiscoveredDevice {
        address: Address(unsafe { (*result).address }),
        name: DeviceName::from_bytes(name),
        rssi: unsafe { (*result).rssi } as i32,
        class_of_device: unsafe { (*result).class_of_device },
        last_seen_epoch: 0,
    };
    if target_matches(device.address.0) {
        flag_set(FLAG_TARGET_SEEN, 0);
    }
    try_with_core(|core| {
        core.controller.discovery_result(device);
    });
}

unsafe extern "C" fn on_pair_request(_cookie: *mut core::ffi::c_void, address: *const u8) {
    let r = runtime();
    r.callback_count.fetch_add(1, Ordering::Relaxed);
    let addr = address_from_ptr(address);
    if !flag(FLAG_BOND_PENDING) || !target_matches(addr) {
        r.callback_dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    flag_set(FLAG_PAIR_REQUEST, 0);
    let result = unsafe { bt_pair_request_reply(adapter(), address, 1) };
    if result != 0 {
        r.last_error.store(result, Ordering::Release);
    }
}

unsafe extern "C" fn on_pair_display(
    _cookie: *mut core::ffi::c_void,
    address: *const u8,
    transport_arg: i32,
    _kind: i32,
    _passkey: u32,
) {
    let r = runtime();
    r.callback_count.fetch_add(1, Ordering::Relaxed);
    let addr = address_from_ptr(address);
    if transport_arg as u32 != CLASSIC_TRANSPORT
        || !flag(FLAG_BOND_PENDING)
        || !target_matches(addr)
    {
        r.callback_dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    flag_set(FLAG_PAIR_DISPLAY, 0);
    r.bond_transport.store(transport_arg, Ordering::Release);
    let result = unsafe { bt_pair_display_reply(adapter(), address, CLASSIC_TRANSPORT as i32, 1) };
    if result != 0 {
        r.last_error.store(result, Ordering::Release);
    }
}

unsafe extern "C" fn on_bond_state(
    _cookie: *mut core::ffi::c_void,
    address: *const u8,
    transport_arg: i32,
    state: i32,
) {
    let r = runtime();
    r.callback_count.fetch_add(1, Ordering::Relaxed);
    let addr = address_from_ptr(address);
    if !target_matches(addr) {
        r.callback_dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    r.bond_transport.store(transport_arg, Ordering::Release);
    r.bond_state.store(state, Ordering::Release);
    let address = Address(addr);
    if transport_arg as u32 != CLASSIC_TRANSPORT {
        transport::bond_failed(addr, ERR_STATE);
        flag_set(0, FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY);
        try_with_core(|core| {
            let _ = core.controller.bond_complete(address, false);
        });
        return;
    }
    if state as u32 == BOND_STATE_BONDED {
        flag_set(
            FLAG_BONDED,
            FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY,
        );
        r.stock_bond_state.store(3, Ordering::Release);
        try_with_core(|core| {
            let _ = core.controller.bond_complete(address, true);
        });
    } else if state as u32 == BOND_STATE_NONE {
        transport::bond_failed(addr, ERR_REMOTE);
        flag_set(
            0,
            FLAG_BONDED | FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY,
        );
        try_with_core(|core| {
            let _ = core.controller.bond_complete(address, false);
        });
    } else if state != 1 {
        transport::bond_failed(addr, ERR_STATE);
        r.last_error.store(ERR_STATE, Ordering::Release);
        flag_set(0, FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY);
        try_with_core(|core| {
            let _ = core.controller.bond_complete(address, false);
        });
    }
}
