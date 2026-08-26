//! Adapter client: registration, callback filtering, discovery table feed,
//! and the pair-if-needed transaction, driven by the portable controller.

use core::sync::atomic::Ordering;

use canopus_target_private::*;

use canopus_bluetooth_audio_core::{Address, DeviceName, DiscoveredDevice, Platform};

use super::runtime::*;
use super::transport;
use super::{compatibility, volume_store};

pub struct DevicePlatform;

// Pair-filter failures are module-local diagnostics, not firmware return values.
const ERR_CORE_POLICY: i32 = -1113;
const ERR_CORE_DESCRIPTOR_UNAVAILABLE: i32 = -1114;
const ERR_CORE_DESCRIPTOR_MISMATCH: i32 = -1115;
const ERR_CORE_PAIR_SLOT_MISMATCH: i32 = -1116;
const ERR_CORE_REGISTRATION: i32 = -1117;
const BOND_TIMER_EVENT: u8 = 11;
const BOND_TIMER_REMOVE: u8 = 1;
const BOND_TIMER_PAIR: u8 = 2;
const REMOVE_TIMEOUT_MS: u32 = 8_000;
const PAIR_TIMEOUT_MS: u32 = 60_000;
const BOND_TIMER_TAG: &[u8] = b"A2DPB\0";

#[repr(C)]
struct BondTimerToken {
    generation: u32,
    transaction: u32,
    phase: u8,
    reserved: [u8; 3],
}

/// Suppresses the stock companion client's competing rejection only for the
/// module's active headset transaction. Exact table/slot ABI stays target-private.
extern "C" fn core_pair_request_filter(cookie: *mut core::ffi::c_void, address: *const u8) {
    let addr = address_from_ptr(address);
    if !address.is_null() && flag(FLAG_BOND_PENDING) && target_matches(addr) {
        flag_set(FLAG_CORE_FILTER_HIT, 0);
        return;
    }
    unsafe { bt_forward_pair_request(cookie, address) };
}

fn install_core_pair_filter(_address: [u8; 6]) -> Result<(), i32> {
    let r = runtime();
    if r.core_filter_table.load(Ordering::Acquire) != 0 {
        return Ok(());
    }
    match unsafe { bt_install_pair_request_filter(core_pair_request_filter) } {
        Ok(Some(filter)) => {
            r.core_filter_table
                .store(filter.allocation, Ordering::Release);
            r.core_filter_handle
                .store(filter.registration, Ordering::Release);
            flag_set(FLAG_CORE_FILTER_INSTALLED, 0);
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(PairRequestFilterError::Policy) => Err(ERR_CORE_POLICY),
        Err(PairRequestFilterError::DescriptorUnavailable) => Err(ERR_CORE_DESCRIPTOR_UNAVAILABLE),
        Err(PairRequestFilterError::DescriptorMismatch) => Err(ERR_CORE_DESCRIPTOR_MISMATCH),
        Err(PairRequestFilterError::PairSlotMismatch) => Err(ERR_CORE_PAIR_SLOT_MISMATCH),
        Err(PairRequestFilterError::Allocation) => Err(ERR_ALLOC),
        Err(PairRequestFilterError::Registration) => Err(ERR_CORE_REGISTRATION),
    }
}

impl Platform for DevicePlatform {
    type Error = i32;

    fn start_discovery(&mut self, timeout_seconds: u8) -> Result<(), i32> {
        compatibility::install()?;
        transport::schedule_initialize_if_ready()?;
        let r = runtime();
        r.scan_epoch.fetch_add(1, Ordering::AcqRel);
        let adapter = adapter();
        let result = unsafe { bt_discovery_start(adapter, timeout_seconds as i32) };
        if result == 0 {
            flag_set(FLAG_DISCOVERY_ACTIVE, 0);
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
            runtime().scan_stop_pending.store(0, Ordering::Release);
            runtime().last_error.store(result, Ordering::Release);
            Err(result)
        }
    }

    fn prepare_bond(&mut self, address: Address) -> Result<bool, i32> {
        compatibility::install()?;
        let volume = volume_store::select(
            address.0,
            canopus_bluetooth_audio_core::avrcp::DEFAULT_VOLUME,
        );
        runtime()
            .avrcp_volume
            .store(volume as u32, Ordering::Release);
        prepare_fresh_bond(address)
    }

    fn create_bond(&mut self, address: Address) -> Result<(), i32> {
        compatibility::install()?;
        begin_bond(address)
    }

    fn connect_avdtp(&mut self, address: Address) -> Result<(), i32> {
        compatibility::install()?;
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

/// Reads and preserves both stock bond views without promoting a stale device
/// record to authoritative aggregate BONDED state.
fn query_bond(address: Address) -> (u32, u32) {
    let r = runtime();
    let stock = unsafe { bt_get_bond_state(address.0.as_ptr()) };
    let device = unsafe { bt_get_pairing_state(address.0.as_ptr(), CLASSIC_TRANSPORT) };
    r.stock_bond_state.store(stock, Ordering::Release);
    r.device_bond_state.store(device, Ordering::Release);
    r.last_error.store(0, Ordering::Release);
    (stock, device)
}

fn cancel_bond_timer() {
    let r = runtime();
    r.bond_timer_phase.store(0, Ordering::Release);
    let mut handle = r.bond_timer_handle.swap(0, Ordering::AcqRel);
    if handle != 0 {
        unsafe { bt_timer_cancel(&mut handle) };
    }
}

fn arm_bond_timer(phase: u8, delay_ms: u32) -> Result<(), i32> {
    cancel_bond_timer();
    let r = runtime();
    let owner = unsafe { bt_l2cap_owner() };
    if owner.is_null() {
        r.last_error.store(ERR_STATE, Ordering::Release);
        return Err(ERR_STATE);
    }
    let token =
        unsafe { bt_alloc(core::mem::size_of::<BondTimerToken>() as u32) } as *mut BondTimerToken;
    if token.is_null() {
        r.last_error.store(ERR_ALLOC, Ordering::Release);
        return Err(ERR_ALLOC);
    }
    let transaction = r.bond_generation.load(Ordering::Acquire);
    r.bond_timer_phase
        .store(u32::from(phase), Ordering::Release);
    unsafe {
        token.write(BondTimerToken {
            generation: r.generation,
            transaction,
            phase,
            reserved: [0; 3],
        });
    }
    let handle = unsafe {
        bt_timer_add(
            owner,
            delay_ms,
            BOND_TIMER_EVENT,
            bond_timer_callback as *const () as *mut core::ffi::c_void,
            token.cast(),
            BOND_TIMER_TAG.as_ptr(),
            1,
        )
    };
    if handle == 0 {
        unsafe { bt_free(token.cast()) };
        let _ = r.bond_timer_phase.compare_exchange(
            u32::from(phase),
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        r.last_error.store(ERR_ALLOC, Ordering::Release);
        return Err(ERR_ALLOC);
    }
    if r.bond_timer_handle
        .compare_exchange(0, handle, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        let mut duplicate = handle;
        unsafe { bt_timer_cancel(&mut duplicate) };
        return Err(ERR_STATE);
    }
    if (r.bond_generation.load(Ordering::Acquire) != transaction
        || r.bond_timer_phase.load(Ordering::Acquire) != u32::from(phase))
        && r.bond_timer_handle
            .compare_exchange(handle, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        let mut stale = handle;
        unsafe { bt_timer_cancel(&mut stale) };
    }
    Ok(())
}

extern "C" fn bond_timer_callback(
    owner_valid: i32,
    event: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    let r = runtime();
    if argument.is_null() {
        return 0;
    }
    let token = unsafe { &*argument.cast::<BondTimerToken>() };
    let phase = token.phase;
    let valid = owner_valid != 0
        && event == BOND_TIMER_EVENT as i32
        && token.generation == r.generation
        && token.transaction == r.bond_generation.load(Ordering::Acquire)
        && u32::from(phase) == r.bond_timer_phase.load(Ordering::Acquire);
    unsafe { bt_free(argument) };
    if !valid {
        return 0;
    }
    r.bond_timer_phase.store(0, Ordering::Release);
    r.bond_timer_handle.store(0, Ordering::Release);
    let Some(address) = target_load() else {
        return 0;
    };
    r.last_error.store(ERR_BOND_TIMEOUT, Ordering::Release);
    if phase == BOND_TIMER_REMOVE && flag(FLAG_REMOVE_PENDING) {
        flag_set(0, FLAG_REMOVE_PENDING);
        dispatch_core_event(CORE_EVENT_REMOVE_FAILED, address);
    } else if phase == BOND_TIMER_PAIR && flag(FLAG_BOND_PENDING) {
        flag_set(0, FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY);
        transport::bond_failed(address, ERR_BOND_TIMEOUT);
        dispatch_core_event(CORE_EVENT_BOND_FAILED, address);
    }
    0
}

/// Exact-target fresh-pair preflight. Removal is always attempted and pairing
/// waits for the authoritative NONE callback when firmware accepts it; a
/// rejected removal means there was no Classic bond to clear and pairing moves
/// directly to stock bond submission.
fn prepare_fresh_bond(address: Address) -> Result<bool, i32> {
    let r = runtime();
    if r.transport_state.load(Ordering::Acquire) != TRANSPORT_READY
        || r.media_state.load(Ordering::Acquire) != MEDIA_IDLE
        || !flag(FLAG_ADAPTER_REGISTERED)
        || !flag(FLAG_ADAPTER_ON)
        || r.signaling_cid.load(Ordering::Acquire) > 0x3f
        || r.discovery_state.load(Ordering::Acquire) != DISCOVERY_STOPPED
        || r.scan_stop_pending.load(Ordering::Acquire) != 0
        || flag(FLAG_DISCOVERY_ACTIVE | FLAG_BOND_PENDING | FLAG_REMOVE_PENDING)
    {
        r.last_error.store(ERRNO_EBUSY, Ordering::Release);
        return Err(ERRNO_EBUSY);
    }

    cancel_bond_timer();
    r.bond_generation.fetch_add(1, Ordering::AcqRel);
    target_store(address.0);
    flag_set(
        FLAG_TARGET_SEEN,
        FLAG_BONDED
            | FLAG_PAIR_REQUEST
            | FLAG_PAIR_DISPLAY
            | FLAG_PAIR_REQUEST_SEEN
            | FLAG_PAIR_DISPLAY_SEEN
            | FLAG_CONNECT_BOND_TRIED
            | FLAG_CORE_FILTER_HIT
            | FLAG_REMOVE_PENDING
            | FLAG_REMOVE_CONFIRMED,
    );
    // Both bond views are published for the UI, but neither decides anything
    // here. Firmware owns that judgement: native remove accepts a Classic
    // record only in exact state 2 and otherwise returns nonzero having
    // touched no state, so calling it unconditionally is safe and is the
    // authoritative answer to "was there a bond to clear". Deciding from the
    // recovered pairing-state accessor instead would put this path back at the
    // mercy of that one recovered address being right.
    query_bond(address);

    flag_set(FLAG_REMOVE_PENDING, FLAG_REMOVE_CONFIRMED);
    if let Err(error) = arm_bond_timer(BOND_TIMER_REMOVE, REMOVE_TIMEOUT_MS) {
        flag_set(0, FLAG_REMOVE_PENDING);
        return Err(error);
    }
    if unsafe { bt_remove_bond(address.0.as_ptr(), CLASSIC_TRANSPORT) } == 0 {
        // Accepted: the authoritative NONE callback completes the transition.
        return Ok(true);
    }

    // Rejected: there is no removable Classic bond, and no bond-state callback
    // follows a rejected removal, so the watchdog must be released here rather
    // than left to time out. Stock bond submission proceeds; a record firmware
    // will not bond over is reported by create_bond itself.
    cancel_bond_timer();
    flag_set(0, FLAG_REMOVE_PENDING);
    Ok(false)
}

pub fn adapter_is_on() -> bool {
    flag(FLAG_ADAPTER_ON)
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
        return Err(ERR_ADAPTER_UNAVAILABLE);
    }
    r.adapter.store(adapter as usize, Ordering::Release);
    let callbacks = callbacks_ptr();
    unsafe {
        *callbacks.add(CALLBACK_ADAPTER_STATE) = on_adapter_state as *const () as usize as u32;
        *callbacks.add(CALLBACK_DISCOVERY_STATE) = on_discovery_state as *const () as usize as u32;
        *callbacks.add(CALLBACK_DISCOVERY_RESULT) =
            on_discovery_result as *const () as usize as u32;
        *callbacks.add(CALLBACK_PAIR_REQUEST) = on_pair_request as *const () as usize as u32;
        *callbacks.add(CALLBACK_PAIR_DISPLAY) = on_pair_display as *const () as usize as u32;
        *callbacks.add(CALLBACK_BOND_STATE) = on_bond_state as *const () as usize as u32;
    }
    let handle = unsafe { bt_adapter_register(adapter, callbacks.cast_const()) };
    if handle == 0 {
        r.registration_state
            .store(REGISTRATION_FAILED, Ordering::Release);
        return Err(ERR_ADAPTER_REGISTER);
    }
    r.registration_state
        .store(REGISTRATION_COMPLETE, Ordering::Release);
    flag_set(FLAG_ADAPTER_REGISTERED, 0);
    let state = unsafe { bt_adapter_get_state(adapter) };
    r.adapter_state.store(state, Ordering::Release);
    if state == ADAPTER_STATE_ON {
        flag_set(FLAG_ADAPTER_ON, 0);
    } else {
        flag_set(0, FLAG_ADAPTER_ON);
    }
    Ok(())
}

/// Starts the pair-if-needed transaction: records the target, arms the
/// transport bond wait, and submits the stock bond.
pub fn begin_bond(address: Address) -> Result<(), i32> {
    let r = runtime();
    if flag(FLAG_BOND_PENDING) {
        r.last_error.store(ERRNO_EBUSY, Ordering::Release);
        return Err(ERRNO_EBUSY);
    }
    target_store(address.0);
    flag_set(FLAG_TARGET_SEEN | FLAG_BOND_PENDING, FLAG_BONDED);
    r.bond_transport
        .store(CLASSIC_TRANSPORT as i32, Ordering::Release);
    r.bond_state.store(1, Ordering::Release);
    if let Err(error) = transport::arm_bond_wait(address) {
        flag_set(0, FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY);
        r.last_error.store(error, Ordering::Release);
        return Err(error);
    }
    match submit_bond() {
        Ok(()) => Ok(()),
        Err(error) => {
            // Match the native cancel path: every failed submission leaves no
            // published pending transaction or parked WAIT_BOND transport.
            cancel_bond_timer();
            flag_set(0, FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY);
            if error == ERRNO_EBUSY {
                flag_set(0, FLAG_CONNECT_BOND_TRIED);
            }
            transport::bond_failed(address.0, error);
            Err(error)
        }
    }
}

fn normalize_bond_error(result: i32) -> i32 {
    if result == CREATE_BOND_ADAPTER_NOT_READY {
        ERRNO_EBUSY
    } else if result > 0 {
        ERRNO_EIO
    } else {
        result
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
    let previous = r.flags.fetch_or(FLAG_CONNECT_BOND_TRIED, Ordering::AcqRel);
    if previous & FLAG_CONNECT_BOND_TRIED != 0 {
        r.last_error.store(ERRNO_EBUSY, Ordering::Release);
        return Err(ERRNO_EBUSY);
    }
    install_core_pair_filter(address)?;
    let scan_mode = unsafe { bt_adapter_get_scan_mode() };
    if scan_mode < 0 {
        r.last_error.store(scan_mode, Ordering::Release);
        return Err(scan_mode);
    }
    let mode_result = unsafe { bt_adapter_set_scan_mode(scan_mode, 1) };
    if mode_result != 0 {
        let error = normalize_bond_error(mode_result);
        r.last_error.store(mode_result, Ordering::Release);
        return Err(error);
    }
    let pairing = unsafe { bt_get_pairing_state(address.as_ptr(), CLASSIC_TRANSPORT) };
    if pairing != 0 {
        r.last_error.store(pairing as i32, Ordering::Release);
        return Err(ERRNO_EBUSY);
    }
    if let Err(error) = arm_bond_timer(BOND_TIMER_PAIR, PAIR_TIMEOUT_MS) {
        flag_set(0, FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY);
        return Err(error);
    }
    let result = unsafe { bt_create_bond(address.as_ptr(), CLASSIC_TRANSPORT) };
    if result == 0 {
        flag_set(0, 0);
        r.last_error.store(0, Ordering::Release);
        Ok(())
    } else {
        cancel_bond_timer();
        r.bond_state
            .store(BOND_STATE_NONE as i32, Ordering::Release);
        flag_set(0, FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY);
        let error = normalize_bond_error(result);
        r.last_error.store(result, Ordering::Release);
        Err(error)
    }
}

/// Clears a pending pairing transaction for a matching address.
pub fn cancel_connect_pairing(address: [u8; 6]) {
    if !target_matches(address) || flag(FLAG_BONDED) {
        return;
    }
    cancel_bond_timer();
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

const CORE_EVENT_DISCOVERY_STOPPED: u8 = 1;
const CORE_EVENT_REMOVE_OK: u8 = 2;
const CORE_EVENT_BOND_OK: u8 = 3;
const CORE_EVENT_BOND_FAILED: u8 = 4;
const CORE_EVENT_REMOVE_FAILED: u8 = 5;
const CORE_EVENT_QUEUE: u8 = 10;
const DISCOVERY_EVENT_QUEUE: u8 = 12;

#[repr(C)]
struct DeferredDiscoveryEvent {
    generation: u32,
    scan_epoch: u32,
    device: DiscoveredDevice,
}

fn enqueue_discovery_event(event: *mut DeferredDiscoveryEvent) -> bool {
    let owner = unsafe { bt_l2cap_owner() };
    if owner.is_null() {
        return false;
    }
    unsafe {
        let _ = bt_queue_external(
            owner,
            deferred_discovery_work,
            bt_queue_free_addr(),
            event.cast(),
            DISCOVERY_EVENT_QUEUE,
        );
    }
    true
}

fn defer_discovery(device: DiscoveredDevice) {
    let event = unsafe { bt_alloc(core::mem::size_of::<DeferredDiscoveryEvent>() as u32) }
        as *mut DeferredDiscoveryEvent;
    if event.is_null() {
        runtime().last_error.store(ERR_ALLOC, Ordering::Release);
        return;
    }
    unsafe {
        event.write(DeferredDiscoveryEvent {
            generation: runtime().generation,
            scan_epoch: runtime().scan_epoch.load(Ordering::Acquire),
            device,
        });
    }
    if !enqueue_discovery_event(event) {
        unsafe { bt_free(event.cast()) };
        runtime().last_error.store(ERR_ALLOC, Ordering::Release);
    }
}

extern "C" fn deferred_discovery_work(
    _owner_valid: i32,
    event_code: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    if argument.is_null() {
        return 0;
    }
    let event = argument.cast::<DeferredDiscoveryEvent>();
    let valid = event_code == DISCOVERY_EVENT_QUEUE as i32
        && unsafe { (*event).generation } == runtime().generation
        && unsafe { (*event).scan_epoch } == runtime().scan_epoch.load(Ordering::Acquire);
    if !valid {
        unsafe { bt_free(argument) };
        return 0;
    }
    let device = unsafe { (*event).device };
    if try_with_core(|core| core.controller.discovery_result(device)).is_none() {
        runtime().last_error.store(ERRNO_EBUSY, Ordering::Release);
    }
    unsafe { bt_free(argument) };
    0
}

#[repr(C)]
struct DeferredCoreEvent {
    generation: u32,
    address: [u8; 6],
    kind: u8,
    reserved: u8,
}

fn apply_core_event(core: &mut Core, kind: u8, address: [u8; 6]) {
    let address = Address(address);
    let result = match kind {
        CORE_EVENT_DISCOVERY_STOPPED => core.controller.discovery_stopped(),
        CORE_EVENT_REMOVE_OK => core.controller.bond_removed(address, true),
        CORE_EVENT_REMOVE_FAILED => core.controller.bond_removed(address, false),
        CORE_EVENT_BOND_OK => core.controller.bond_complete(address, true),
        CORE_EVENT_BOND_FAILED => core.controller.bond_complete(address, false),
        _ => return,
    };
    if result.is_err() {
        core.controller.model.connection = canopus_bluetooth_audio_core::ConnectionState::Failed;
        core.controller.model.touch();
    }
}

fn queue_core_event(kind: u8, address: [u8; 6]) -> bool {
    let owner = unsafe { bt_l2cap_owner() };
    if owner.is_null() {
        return false;
    }
    let event = unsafe { bt_alloc(core::mem::size_of::<DeferredCoreEvent>() as u32) }
        as *mut DeferredCoreEvent;
    if event.is_null() {
        return false;
    }
    unsafe {
        event.write(DeferredCoreEvent {
            generation: runtime().generation,
            address,
            kind,
            reserved: 0,
        });
    }
    unsafe {
        let _ = bt_queue_external(
            owner,
            deferred_core_work,
            bt_queue_free_addr(),
            event.cast(),
            CORE_EVENT_QUEUE,
        );
    }
    true
}

fn dispatch_core_event(kind: u8, address: [u8; 6]) {
    if try_with_core(|core| apply_core_event(core, kind, address)).is_none()
        && !queue_core_event(kind, address)
    {
        runtime().last_error.store(ERR_ALLOC, Ordering::Release);
    }
}

extern "C" fn deferred_core_work(
    _owner_valid: i32,
    event_code: i32,
    argument: *mut core::ffi::c_void,
) -> i32 {
    if argument.is_null() {
        return 0;
    }
    let event = unsafe { &*argument.cast::<DeferredCoreEvent>() };
    if event_code != CORE_EVENT_QUEUE as i32 || event.generation != runtime().generation {
        unsafe { bt_free(argument) };
        return 0;
    }
    let kind = event.kind;
    let address = event.address;
    if try_with_core(|core| apply_core_event(core, kind, address)).is_none() {
        runtime().last_error.store(ERRNO_EBUSY, Ordering::Release);
    }
    unsafe { bt_free(argument) };
    0
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
        // Bluetooth power-on reconstructs the writable GAP transport vtable.
        // Reassert the compare-before-write hook before owner-thread work and
        // again before each connection flow.
        if let Err(error) = compatibility::install() {
            r.last_error.store(error, Ordering::Release);
        }
        if let Err(error) = transport::schedule_initialize_if_ready() {
            r.last_error.store(error, Ordering::Release);
        }
    } else {
        flag_set(0, FLAG_ADAPTER_ON);
        let removing = flag(FLAG_REMOVE_PENDING);
        let bonding = flag(FLAG_BOND_PENDING)
            || r.transport_state.load(Ordering::Acquire) == TRANSPORT_WAIT_BOND;
        if removing || bonding {
            cancel_bond_timer();
            if let Some(address) = target_load() {
                transport::bond_failed(address, ERR_STATE);
                dispatch_core_event(
                    if removing {
                        CORE_EVENT_REMOVE_FAILED
                    } else {
                        CORE_EVENT_BOND_FAILED
                    },
                    address,
                );
            }
            flag_set(
                0,
                FLAG_REMOVE_PENDING | FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY,
            );
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
        // The controller resolves WaitingForScanStop -> fresh-bond preflight.
        // Never lose this handoff to transient UI/core lock contention.
        dispatch_core_event(CORE_EVENT_DISCOVERY_STOPPED, [0; 6]);
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
    let name = unsafe { discovery_name(result, 32) };
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
    if try_with_core(|core| core.controller.discovery_result(device)).is_none() {
        defer_discovery(device);
    }
}

unsafe extern "C" fn on_pair_request(_cookie: *mut core::ffi::c_void, address: *const u8) {
    let r = runtime();
    r.callback_count.fetch_add(1, Ordering::Relaxed);
    let addr = address_from_ptr(address);
    if !flag(FLAG_BOND_PENDING) || !target_matches(addr) {
        r.callback_dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    flag_set(FLAG_PAIR_REQUEST | FLAG_PAIR_REQUEST_SEEN, 0);
    let result = unsafe { bt_pair_request_reply(adapter(), address, 1) };
    if result != 0 {
        // Match the stock bridge: the authoritative bond callback owns failure.
        // A reply IPC error is diagnostic and must not abort an operation that
        // may already have advanced into controller-managed SSP.
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
    flag_set(FLAG_PAIR_DISPLAY | FLAG_PAIR_DISPLAY_SEEN, 0);
    r.bond_transport.store(transport_arg, Ordering::Release);
    let result = unsafe { bt_pair_display_reply(adapter(), address, CLASSIC_TRANSPORT as i32, 1) };
    if result != 0 {
        // As with Pair Request, preserve the in-flight stock PairDevice
        // transaction and let its bond callback or timeout decide the outcome.
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
    if transport_arg as u32 != CLASSIC_TRANSPORT {
        // A dual-mode headset can report an independent LE bond transition for
        // the same address while Classic pairing is in flight. It is unrelated
        // to the A2DP transaction and must not cancel its timer or WAIT_BOND.
        r.callback_dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }

    if state as u32 == BOND_STATE_NONE && flag(FLAG_REMOVE_PENDING) {
        // Native removal clears the exact transport state but deliberately keeps
        // its reusable device record, so the aggregate Classic presence bit may
        // remain set. Only the exact state is authoritative here.
        let (_, device) = query_bond(Address(addr));
        if device != BOND_STATE_NONE {
            r.callback_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        cancel_bond_timer();
        r.stock_bond_state.store(0, Ordering::Release);
        r.device_bond_state.store(0, Ordering::Release);
        r.last_error.store(0, Ordering::Release);
        flag_set(
            FLAG_REMOVE_CONFIRMED,
            FLAG_REMOVE_PENDING
                | FLAG_BONDED
                | FLAG_BOND_PENDING
                | FLAG_PAIR_REQUEST
                | FLAG_PAIR_DISPLAY,
        );
        dispatch_core_event(CORE_EVENT_REMOVE_OK, addr);
        return;
    }

    if state as u32 == BOND_STATE_BONDED {
        r.stock_bond_state.store(3, Ordering::Release);
        r.device_bond_state
            .store(BOND_STATE_BONDED, Ordering::Release);
        if !flag(FLAG_BOND_PENDING)
            || r.transport_state.load(Ordering::Acquire) != TRANSPORT_WAIT_BOND
        {
            r.callback_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        cancel_bond_timer();
        flag_set(
            FLAG_BONDED,
            FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY,
        );
        dispatch_core_event(CORE_EVENT_BOND_OK, addr);
    } else if state as u32 == BOND_STATE_NONE {
        r.stock_bond_state.store(0, Ordering::Release);
        r.device_bond_state.store(0, Ordering::Release);
        if flag(FLAG_BOND_PENDING) {
            cancel_bond_timer();
            transport::bond_failed(addr, ERR_REMOTE);
            flag_set(
                0,
                FLAG_BONDED | FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY,
            );
            dispatch_core_event(CORE_EVENT_BOND_FAILED, addr);
        }
    } else if state != 1 && flag(FLAG_REMOVE_PENDING) {
        cancel_bond_timer();
        r.last_error.store(ERR_STATE, Ordering::Release);
        flag_set(0, FLAG_REMOVE_PENDING);
        dispatch_core_event(CORE_EVENT_REMOVE_FAILED, addr);
    } else if state != 1 && flag(FLAG_BOND_PENDING) {
        cancel_bond_timer();
        transport::bond_failed(addr, ERR_STATE);
        r.last_error.store(ERR_STATE, Ordering::Release);
        flag_set(0, FLAG_BOND_PENDING | FLAG_PAIR_REQUEST | FLAG_PAIR_DISPLAY);
        dispatch_core_event(CORE_EVENT_BOND_FAILED, addr);
    }
}
