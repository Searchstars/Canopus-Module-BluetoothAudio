//! Fixed static storage and lock discipline for the target backend.
//!
//! The firmware delivers adapter/L2CAP/timer callbacks from the Bluetooth
//! context while UI events arrive from the miwear context. Cross-context state
//! lives in atomics exactly like the legacy bridge; the mutable core state
//! machines (controller, AVDTP source, tone packetizer) are guarded by a
//! spinlock that callbacks acquire non-blockingly and actions acquire
//! blockingly. A callback that loses the race is dropped and counted rather
//! than blocking the Bluetooth stack.

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};

use super::bluetooth::DevicePlatform;
use canopus_bluetooth_audio_core::{Controller, avdtp, avrcp, media};
use canopus_target_private::bt_alloc;

pub const MAGIC: u32 = 0x4241_5541; // "BAUA"

pub const REGISTRATION_NONE: u32 = 0;
pub const REGISTRATION_ACTIVE: u32 = 1;
pub const REGISTRATION_COMPLETE: u32 = 2;
pub const REGISTRATION_FAILED: u32 = 3;

pub const TRANSPORT_DORMANT: u32 = 0;
pub const TRANSPORT_INITIALIZING: u32 = 1;
pub const TRANSPORT_READY: u32 = 2;
pub const TRANSPORT_CONNECTING: u32 = 3;
pub const TRANSPORT_CONNECTED: u32 = 4;
pub const TRANSPORT_DISCONNECTING: u32 = 5;
pub const TRANSPORT_FAILED: u32 = 6;
pub const TRANSPORT_WAIT_BOND: u32 = 7;

pub const MEDIA_IDLE: u32 = 0;
pub const MEDIA_CONNECTING: u32 = 1;
pub const MEDIA_CONNECTED: u32 = 2;
pub const MEDIA_STARTING: u32 = 3;
pub const MEDIA_STREAMING: u32 = 4;
pub const MEDIA_SUSPENDING: u32 = 5;
pub const MEDIA_COMPLETE: u32 = 6;
pub const MEDIA_DISCONNECTING: u32 = 7;
pub const MEDIA_FAILED: u32 = 8;

pub const AVRCP_IDLE: u32 = 0;
pub const AVRCP_CONNECTING: u32 = 1;
pub const AVRCP_CONNECTED: u32 = 2;
pub const AVRCP_DISCONNECTING: u32 = 3;
pub const AVRCP_FAILED: u32 = 4;

pub const MEDIA_FLAG_START_WHEN_CONNECTED: u32 = 1 << 0;
pub const MEDIA_FLAG_FINISH_ON_TIMER: u32 = 1 << 1;
pub const MEDIA_FLAG_EXTERNAL_STREAM: u32 = 1 << 2;

// Peer flags (module-internal encoding of the connect/bond transaction).
pub const FLAG_ADAPTER_REGISTERED: u32 = 1 << 0;
pub const FLAG_ADAPTER_ON: u32 = 1 << 1;
pub const FLAG_DISCOVERY_ACTIVE: u32 = 1 << 2;
pub const FLAG_TARGET_SEEN: u32 = 1 << 4;
pub const FLAG_BOND_PENDING: u32 = 1 << 5;
pub const FLAG_PAIR_REQUEST: u32 = 1 << 6;
pub const FLAG_PAIR_DISPLAY: u32 = 1 << 7;
pub const FLAG_BONDED: u32 = 1 << 8;
pub const FLAG_CONNECT_BOND_TRIED: u32 = 1 << 9;
pub const FLAG_CORE_FILTER_INSTALLED: u32 = 1 << 10;
pub const FLAG_CORE_FILTER_HIT: u32 = 1 << 11;
pub const FLAG_REMOVE_PENDING: u32 = 1 << 12;
pub const FLAG_REMOVE_CONFIRMED: u32 = 1 << 13;
pub const FLAG_PAIR_REQUEST_SEEN: u32 = 1 << 15;
pub const FLAG_PAIR_DISPLAY_SEEN: u32 = 1 << 16;
pub const FLAG_HCI_COMPAT_INSTALLED: u32 = 1 << 17;
pub const FLAG_HCI_COMPAT_HIT: u32 = 1 << 18;

// App install result state.
pub const ERR_STATE: i32 = -1101;
pub const ERR_ALLOC: i32 = -1102;
pub const ERR_PACKET: i32 = -1104;
pub const ERR_REMOTE: i32 = -1105;
pub const ERR_SDP: i32 = -1106;
pub const ERR_CODEC_UNSUPPORTED: i32 = -1107;
pub const ERR_BOND_TIMEOUT: i32 = -1108;
pub const ERR_HCI_POLICY: i32 = -1109;
pub const ERR_ADAPTER_UNAVAILABLE: i32 = -1110;
pub const ERR_ADAPTER_REGISTER: i32 = -1111;
pub const ERR_TRANSPORT_STATE: i32 = -1112;
pub const ERR_MEDIA_STATE: i32 = -1201;
pub const ERR_MEDIA_ALLOC: i32 = -1202;
pub const ERR_MEDIA_PACKET: i32 = -1203;
pub const ERR_MEDIA_REMOTE: i32 = -1204;
pub const ERR_MEDIA_TIMER: i32 = -1205;
pub const ERR_AUDIO_DECODE: i32 = -1206;
pub const ERR_AUDIO_CODEC: i32 = -1207;
pub const ERR_AUDIO_QUEUE: i32 = -1208;
pub const ERR_AVRCP_STATE: i32 = -1301;
pub const ERR_AVRCP_PACKET: i32 = -1302;
pub const ERR_AVRCP_REMOTE: i32 = -1303;
pub const ERRNO_EIO: i32 = -5;
pub const ERRNO_EBUSY: i32 = -16;
pub const ERRNO_EINVAL: i32 = -22;
pub const ERRNO_ENOSYS: i32 = -38;

pub const APP_NONE: u32 = 0;
pub const APP_REGISTERED: u32 = 1;
pub const APP_OK: u32 = 2;
pub const APP_FAILED: u32 = 3;

pub struct Core {
    pub controller: Controller<DevicePlatform>,
    pub source: avdtp::Source,
    pub avrcp: avrcp::Controller,
    pub packetizer: Option<media::TonePacketizer>,
    pub signaling_out: [u8; 192],
    pub avrcp_out: [u8; 32],
    pub media_out: [u8; media::MAX_PACKET],
}

impl Core {
    fn new(generation: u32) -> Self {
        Self {
            controller: Controller::new(DevicePlatform),
            source: avdtp::Source::new(generation),
            avrcp: avrcp::Controller::new(),
            packetizer: None,
            signaling_out: [0; 192],
            avrcp_out: [0; 32],
            media_out: [0; media::MAX_PACKET],
        }
    }
}

pub struct Runtime {
    pub magic: u32,
    pub generation: u32,
    pub adapter: AtomicUsize,
    pub registration_state: AtomicU32,
    pub flags: AtomicU32,
    pub scan_stop_pending: AtomicU32,
    pub discovery_state: AtomicI32,
    pub adapter_state: AtomicI32,
    pub bond_transport: AtomicI32,
    pub bond_state: AtomicI32,
    pub stock_bond_state: AtomicU32,
    pub device_bond_state: AtomicU32,
    pub bond_generation: AtomicU32,
    pub bond_timer_handle: AtomicU32,
    pub bond_timer_phase: AtomicU32,
    pub target_low: AtomicU32,
    pub target_high: AtomicU32,
    pub target_sequence: AtomicU32,
    pub scan_epoch: AtomicU32,
    pub callback_count: AtomicU32,
    pub callback_dropped: AtomicU32,
    pub discovery_count: AtomicU32,
    pub last_error: AtomicI32,
    // Transport / media (cross-context).
    pub transport_state: AtomicU32,
    pub signaling_cid: AtomicU32,
    pub signaling_mtu: AtomicU32,
    pub avrcp_state: AtomicU32,
    pub avrcp_cid: AtomicU32,
    pub avrcp_mtu: AtomicU32,
    pub avrcp_generation: AtomicU32,
    pub avrcp_volume: AtomicU32,
    pub avrcp_packets_sent: AtomicU32,
    pub avrcp_packets_received: AtomicU32,
    pub avrcp_last_event: AtomicU32,
    pub avrcp_rx_header: AtomicU32,
    pub avrcp_rx_length: AtomicU32,
    pub avrcp_error: AtomicI32,
    pub media_state: AtomicU32,
    pub media_cid: AtomicU32,
    pub media_mtu: AtomicU32,
    pub media_generation: AtomicU32,
    /// RTP SDUs accepted by the stock L2CAP queue but not yet covered by an
    /// event-8 flow credit. This bounds producer pressure on Bluelet.
    pub media_tx_outstanding: AtomicU32,
    pub media_packets_queued: AtomicU32,
    pub media_packets_completed: AtomicU32,
    pub media_startup_silence_queued: AtomicU32,
    pub media_packets_sent: AtomicU32,
    pub media_frames_sent: AtomicU32,
    pub media_packets_target: AtomicU32,
    pub media_frames_per_packet: AtomicU32,
    pub media_pace_remainder: AtomicU32,
    pub media_rtp_sequence: AtomicU32,
    pub media_rtp_timestamp: AtomicU32,
    pub media_timer_handle: AtomicU32,
    pub media_timer_generation: AtomicU32,
    pub media_flags: AtomicU32,
    pub audio_timer_handle: AtomicU32,
    pub audio_timer_generation: AtomicU32,
    pub sdp_handle: AtomicU32,
    pub avrcp_sdp_handle: AtomicU32,
    pub sdp_registered: AtomicU32,
    // Native app / lifecycle.
    pub app_state: AtomicU32,
    pub app_error: AtomicI32,
    pub app_install_result: AtomicI32,
    pub launcher_add_result: AtomicI32,
    pub core_filter_table: AtomicUsize,
    pub core_filter_handle: AtomicU32,
    pub resident: AtomicBool,
}

impl Runtime {
    const fn const_new(generation: u32) -> Self {
        Self {
            magic: MAGIC,
            generation,
            adapter: AtomicUsize::new(0),
            registration_state: AtomicU32::new(REGISTRATION_NONE),
            flags: AtomicU32::new(0),
            scan_stop_pending: AtomicU32::new(0),
            discovery_state: AtomicI32::new(0),
            adapter_state: AtomicI32::new(-1),
            bond_transport: AtomicI32::new(1),
            bond_state: AtomicI32::new(0),
            stock_bond_state: AtomicU32::new(0),
            device_bond_state: AtomicU32::new(0),
            bond_generation: AtomicU32::new(0),
            bond_timer_handle: AtomicU32::new(0),
            bond_timer_phase: AtomicU32::new(0),
            target_low: AtomicU32::new(0),
            target_high: AtomicU32::new(0),
            target_sequence: AtomicU32::new(0),
            scan_epoch: AtomicU32::new(0),
            callback_count: AtomicU32::new(0),
            callback_dropped: AtomicU32::new(0),
            discovery_count: AtomicU32::new(0),
            last_error: AtomicI32::new(0),
            transport_state: AtomicU32::new(TRANSPORT_DORMANT),
            signaling_cid: AtomicU32::new(0),
            signaling_mtu: AtomicU32::new(0),
            avrcp_state: AtomicU32::new(AVRCP_IDLE),
            avrcp_cid: AtomicU32::new(0),
            avrcp_mtu: AtomicU32::new(0),
            avrcp_generation: AtomicU32::new(0),
            avrcp_volume: AtomicU32::new(avrcp::DEFAULT_VOLUME as u32),
            avrcp_packets_sent: AtomicU32::new(0),
            avrcp_packets_received: AtomicU32::new(0),
            avrcp_last_event: AtomicU32::new(0),
            avrcp_rx_header: AtomicU32::new(0),
            avrcp_rx_length: AtomicU32::new(0),
            avrcp_error: AtomicI32::new(0),
            media_state: AtomicU32::new(MEDIA_IDLE),
            media_cid: AtomicU32::new(0),
            media_mtu: AtomicU32::new(0),
            media_generation: AtomicU32::new(0),
            media_tx_outstanding: AtomicU32::new(0),
            media_packets_queued: AtomicU32::new(0),
            media_packets_completed: AtomicU32::new(0),
            media_startup_silence_queued: AtomicU32::new(0),
            media_packets_sent: AtomicU32::new(0),
            media_frames_sent: AtomicU32::new(0),
            media_packets_target: AtomicU32::new(0),
            media_frames_per_packet: AtomicU32::new(0),
            media_pace_remainder: AtomicU32::new(0),
            media_rtp_sequence: AtomicU32::new(0),
            media_rtp_timestamp: AtomicU32::new(0),
            media_timer_handle: AtomicU32::new(0),
            media_timer_generation: AtomicU32::new(0),
            media_flags: AtomicU32::new(0),
            audio_timer_handle: AtomicU32::new(0),
            audio_timer_generation: AtomicU32::new(0),
            sdp_handle: AtomicU32::new(0),
            avrcp_sdp_handle: AtomicU32::new(0),
            sdp_registered: AtomicU32::new(0),
            app_state: AtomicU32::new(APP_NONE),
            app_error: AtomicI32::new(0),
            app_install_result: AtomicI32::new(0),
            launcher_add_result: AtomicI32::new(0),
            core_filter_table: AtomicUsize::new(0),
            core_filter_handle: AtomicU32::new(0),
            resident: AtomicBool::new(false),
        }
    }
}

static mut RUNTIME: core::mem::MaybeUninit<Runtime> = core::mem::MaybeUninit::uninit();
static mut CALLBACKS: [u32; 17] = [0; 17];
static CORE_PTR: AtomicUsize = AtomicUsize::new(0);
static CORE_LOCK: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);

/// Resets every fixed block and initializes the core state machines. Called
/// from the module constructor; the firmware invokes it at most once per load.
pub fn prepare(generation: u32) {
    let mut core_pointer = CORE_PTR.load(Ordering::Acquire) as *mut Core;
    if core_pointer.is_null() {
        core_pointer = unsafe { bt_alloc(core::mem::size_of::<Core>() as u32) } as *mut Core;
        if core_pointer.is_null() {
            READY.store(false, Ordering::Release);
            return;
        }
        CORE_PTR.store(core_pointer as usize, Ordering::Release);
    }
    // SAFETY: prepare runs before callback publication. A repeated supervisor
    // prepare reuses the constructor allocation and resets it in place.
    unsafe {
        core::ptr::addr_of_mut!(RUNTIME)
            .cast::<Runtime>()
            .write(Runtime::const_new(generation));
        core::ptr::addr_of_mut!(CALLBACKS).write([0; 17]);
        core_pointer.write(Core::new(generation));
    }
    runtime().adapter_state.store(-1, Ordering::Release);
    runtime().bond_transport.store(1, Ordering::Release);
    CORE_LOCK.store(false, Ordering::Release);
    READY.store(true, Ordering::Release);
}

/// Returns the initialized cross-context state. Every field accessed after
/// publication is immutable or atomic; returning a shared reference avoids
/// manufacturing aliased `&mut Runtime` values across firmware callbacks.
pub fn runtime() -> &'static Runtime {
    // SAFETY: `prepare` initializes RUNTIME before READY publishes it, and no
    // non-atomic field is mutated afterward.
    unsafe { &*core::ptr::addr_of!(RUNTIME).cast::<Runtime>() }
}

/// Returns the separate, write-once adapter callback table. The table is filled
/// before registration and remains resident/read-only from then until reboot.
pub fn callbacks_ptr() -> *mut u32 {
    core::ptr::addr_of_mut!(CALLBACKS).cast::<u32>()
}

/// Runs `f` with exclusive access to the core state machines. Callbacks use
/// [`try_with_core`]; actions use this blocking form.
pub fn with_core<R>(f: impl FnOnce(&mut Core) -> R) -> R {
    while CORE_LOCK
        .compare_exchange_weak(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let pointer = CORE_PTR.load(Ordering::Acquire) as *mut Core;
    // SAFETY: prepare publishes a valid resident allocation before READY, and
    // exclusive ownership of its contents is held by the lock.
    let out = unsafe { f(&mut *pointer) };
    CORE_LOCK.store(false, Ordering::Release);
    out
}

/// Non-blocking core access for callbacks. Returns `None` and counts a drop
/// when another context holds the lock.
pub fn try_with_core<R>(f: impl FnOnce(&mut Core) -> R) -> Option<R> {
    if CORE_LOCK
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        runtime().callback_dropped.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let pointer = CORE_PTR.load(Ordering::Acquire) as *mut Core;
    // SAFETY: prepare publishes a valid resident allocation before READY, and
    // exclusive ownership of its contents is held by the lock.
    let out = unsafe { f(&mut *pointer) };
    CORE_LOCK.store(false, Ordering::Release);
    Some(out)
}

pub fn initialized() -> bool {
    READY.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// Target address seqlock
// ---------------------------------------------------------------------------

pub fn target_store(address: [u8; 6]) {
    let low = u32::from_le_bytes([address[0], address[1], address[2], address[3]]);
    let high = u32::from(address[4]) | (u32::from(address[5]) << 8);
    let r = runtime();
    r.target_sequence.fetch_add(1, Ordering::AcqRel);
    r.target_low.store(low, Ordering::Relaxed);
    r.target_high.store(high, Ordering::Relaxed);
    r.target_sequence.fetch_add(1, Ordering::Release);
}

/// Reads the target address; returns `None` if a concurrent writer was
/// observed (callers retry or drop the callback).
pub fn target_load() -> Option<[u8; 6]> {
    let r = runtime();
    for _ in 0..4 {
        let begin = r.target_sequence.load(Ordering::Acquire);
        if begin & 1 != 0 {
            continue;
        }
        let low = r.target_low.load(Ordering::Relaxed);
        let high = r.target_high.load(Ordering::Relaxed);
        let end = r.target_sequence.load(Ordering::Acquire);
        if begin == end && end & 1 == 0 {
            return Some([
                low as u8,
                (low >> 8) as u8,
                (low >> 16) as u8,
                (low >> 24) as u8,
                high as u8,
                (high >> 8) as u8,
            ]);
        }
    }
    None
}

pub fn target_matches(address: [u8; 6]) -> bool {
    match target_load() {
        Some(target) => target == address,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Flags
// ---------------------------------------------------------------------------

pub fn flag_set(set: u32, clear: u32) {
    let r = runtime();
    if clear != 0 {
        r.flags.fetch_and(!clear, Ordering::AcqRel);
    }
    if set != 0 {
        r.flags.fetch_or(set, Ordering::AcqRel);
    }
}

pub fn flag(bit: u32) -> bool {
    runtime().flags.load(Ordering::Acquire) & bit != 0
}
