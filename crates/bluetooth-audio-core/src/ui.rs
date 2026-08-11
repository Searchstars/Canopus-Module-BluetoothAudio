use crate::{
    ConnectionState, Model, PAIR_DIAG_BONDED, PAIR_DIAG_DISPLAY, PAIR_DIAG_FILTER_HIT,
    PAIR_DIAG_MHDT_FIXED, PAIR_DIAG_REMOVE_CONFIRMED, PAIR_DIAG_REMOVE_PENDING, PAIR_DIAG_REQUEST,
    ScanState,
};
use canopus_ui_core::{Snapshot, TextStyle, Tree, UiError};
use core::fmt::{self, Write};

pub const EVENT_CONNECTED_DETAIL: u32 = 1;
pub const EVENT_SCAN: u32 = 2;
pub const EVENT_REFRESH: u32 = 3;
pub const EVENT_BACK: u32 = 4;
pub const EVENT_DISCONNECT: u32 = 5;
pub const EVENT_TEST_TONE: u32 = 6;
pub const EVENT_LONG_MP3: u32 = 7;
pub const EVENT_LONG_MP3_DECODE_ONLY: u32 = 8;
pub const EVENT_DEVICE_BASE: u32 = 100;

/// Stable semantic identity for a discovered-device row. Event ids still carry
/// the current bounded table index, while the key keeps the same LVX row object
/// attached to a peer when RSSI updates reorder the table.
pub fn device_key(address: crate::Address) -> u32 {
    let mut hash = 0x811C_9DC5u32;
    for byte in address.0 {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    hash | 0x8000_0000
}

pub fn overview(model: &Model) -> Result<Snapshot, UiError> {
    let mut tree = Tree::begin();
    tree.navigation_page(1, "Headphones")?;
    tree.section(10, "Connected headset")?;
    match model.connected {
        Some(peer) => tree.action_row(
            11,
            display_name(&peer.name),
            connection_detail(model.connection),
            EVENT_CONNECTED_DETAIL,
            true,
        )?,
        None => tree.status_row(11, "Headset", connection_detail(model.connection))?,
    }
    if model.last_error != 0 {
        let error = number(model.last_error);
        tree.text(12, error.as_str(), TextStyle::Warning)?;
    }
    if model.selected.is_some() {
        let diagnostic = pairing_diagnostic(model);
        tree.text(13, diagnostic.as_str(), TextStyle::Description)?;
    }
    tree.end()?;
    tree.section(20, "Nearby headsets")?;
    tree.button(
        21,
        if matches!(model.scan, ScanState::Scanning | ScanState::Starting) {
            "Stop scan"
        } else {
            "Start scan"
        },
        EVENT_SCAN,
        true,
    )?;
    tree.status_row(22, "Scan", scan_detail(model.scan))?;
    for (index, device) in model.devices.entries().iter().enumerate() {
        let key = device_key(device.address);
        let event = EVENT_DEVICE_BASE + index as u32;
        let address = device.address.text();
        tree.action_row(
            key,
            display_name(&device.name),
            address.as_str(),
            event,
            model.connection != ConnectionState::Disconnecting,
        )?;
    }
    if model.devices.dropped() != 0 {
        let mut text = FixedText::<48>::default();
        let _ = write!(
            text,
            "{} more results could not be retained",
            model.devices.dropped()
        );
        tree.text(30, text.as_str(), TextStyle::Description)?;
    }
    tree.button(31, "Refresh results", EVENT_REFRESH, true)?;
    tree.end()?;
    tree.end()?;
    tree.commit()
}

pub fn detail(model: &Model) -> Result<Snapshot, UiError> {
    let mut tree = Tree::begin();
    tree.navigation_page(1, "Headset details")?;
    let Some(peer) = model.connected else {
        tree.text(10, "No headset connected", TextStyle::Description)?;
        tree.end()?;
        return tree.commit();
    };
    tree.section(20, "Headset")?;
    tree.status_row(21, "Name", display_name(&peer.name))?;
    let address = peer.address.text();
    tree.status_row(22, "Address", address.as_str())?;
    let rssi = number(peer.rssi);
    tree.status_row(23, "Signal", rssi.as_str())?;
    let cod = hex(model.connected.unwrap().class_of_device);
    tree.status_row(24, "Class", cod.as_str())?;
    tree.status_row(25, "Connection", connection_detail(model.connection))?;
    tree.status_row(26, "Stream", stream_detail(model.stream))?;
    tree.end()?;
    tree.section(30, "Audio")?;
    let mtu = unsigned(model.details.media_mtu as u32);
    tree.status_row(31, "Media MTU", mtu.as_str())?;
    let bitpool = unsigned(model.details.bitpool as u32);
    tree.status_row(32, "SBC bitpool", bitpool.as_str())?;
    let packets = unsigned(model.details.packets_sent);
    tree.status_row(33, "Packets", packets.as_str())?;
    tree.status_row(
        37,
        "Pipeline",
        audio_stage_detail(model.details.audio_stage),
    )?;
    let mut input = FixedText::<32>::default();
    let _ = write!(
        input,
        "{} / {} B",
        audio_state_detail(model.details.audio_state),
        model.details.input_used
    );
    tree.status_row(38, "Input", input.as_str())?;
    let mut decoded = FixedText::<32>::default();
    if model.details.decoded_sample_rate == 0 {
        let _ = decoded.write_str("Waiting for frame");
    } else {
        let _ = write!(
            decoded,
            "{} Hz / {} ch",
            model.details.decoded_sample_rate, model.details.decoded_channels
        );
    }
    tree.status_row(39, "MP3 decoded", decoded.as_str())?;
    let pcm_frames = unsigned(model.details.pcm_frames);
    tree.status_row(40, "PCM frames", pcm_frames.as_str())?;
    let audio_packets = unsigned(model.details.audio_rtp_packets);
    tree.status_row(41, "MP3 RTP", audio_packets.as_str())?;
    let mut media_flow = FixedText::<48>::default();
    let _ = write!(
        media_flow,
        "q{} f{} out{} pre{}",
        model.details.media_packets_queued,
        model.details.media_flow_events,
        model.details.media_tx_outstanding,
        model.details.startup_silence_packets,
    );
    tree.status_row(49, "Media flow", media_flow.as_str())?;
    let underruns = unsigned(model.details.underruns);
    tree.status_row(42, "Underruns", underruns.as_str())?;
    let audio_error = number(model.details.audio_error);
    tree.status_row(43, "Audio error", audio_error.as_str())?;
    let mut timing = FixedText::<40>::default();
    let _ = write!(
        timing,
        "{} ms / decode {} ms",
        model.details.audio_elapsed_ms, model.details.decode_cpu_ms
    );
    tree.status_row(45, "Timing", timing.as_str())?;
    let realtime_percent = if model.details.audio_elapsed_ms == 0 {
        0
    } else {
        (u64::from(model.details.pcm_frames) * 100_000
            / (44_100 * u64::from(model.details.audio_elapsed_ms))) as u32
    };
    let mut speed = FixedText::<24>::default();
    let _ = write!(speed, "{}% realtime", realtime_percent);
    tree.status_row(46, "Decode rate", speed.as_str())?;
    let mut startup = FixedText::<48>::default();
    let _ = write!(
        startup,
        "{} ms (q{} p{} a{})",
        model.details.startup_ms,
        model.details.startup_queue_ms,
        model.details.startup_prepare_ms,
        model.details.startup_avdtp_ms,
    );
    tree.status_row(47, "Startup", startup.as_str())?;
    let rtp_per_second = if model.details.audio_elapsed_ms == 0 {
        0
    } else {
        (u64::from(model.details.audio_rtp_packets) * 1_000
            / u64::from(model.details.audio_elapsed_ms)) as u32
    };
    let mut rtp_rate = FixedText::<24>::default();
    let _ = write!(rtp_rate, "{} packets/s", rtp_per_second);
    tree.status_row(48, "RTP rate", rtp_rate.as_str())?;
    let can_play =
        model.connection == ConnectionState::Ready && model.stream == crate::StreamState::Open;
    tree.button(34, "Play test tone", EVENT_TEST_TONE, can_play)?;
    tree.button(35, "Play long MP3", EVENT_LONG_MP3, can_play)?;
    tree.button(
        44,
        "Decode long MP3 only",
        EVENT_LONG_MP3_DECODE_ONLY,
        can_play,
    )?;
    tree.button(
        36,
        "Disconnect",
        EVENT_DISCONNECT,
        model.connection == ConnectionState::Ready,
    )?;
    tree.end()?;
    tree.end()?;
    tree.commit()
}

fn pairing_diagnostic(model: &Model) -> FixedText<96> {
    let details = &model.details;
    let mut text = FixedText::<96>::default();
    let _ = write!(
        text,
        "Bond {}/{}",
        details.stock_bond_state, details.device_bond_state
    );
    for (flag, label) in [
        (PAIR_DIAG_REMOVE_PENDING, " remove-wait"),
        (PAIR_DIAG_REMOVE_CONFIRMED, " removed"),
        (PAIR_DIAG_FILTER_HIT, " filter-hit"),
        (PAIR_DIAG_MHDT_FIXED, " mhdt-fixed"),
        (PAIR_DIAG_REQUEST, " request"),
        (PAIR_DIAG_DISPLAY, " confirm"),
        (PAIR_DIAG_BONDED, " bonded"),
    ] {
        if details.pairing_flags & flag != 0 {
            let _ = text.write_str(label);
        }
    }
    text
}

fn display_name(name: &crate::DeviceName) -> &str {
    if name.is_empty() {
        "Unknown headset"
    } else {
        name.as_str()
    }
}
fn scan_detail(state: ScanState) -> &'static str {
    match state {
        ScanState::Idle => "Idle",
        ScanState::Starting => "Starting…",
        ScanState::Scanning => "Scanning…",
        ScanState::Stopping => "Stopping…",
        ScanState::Failed => "Scan failed",
    }
}
fn connection_detail(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Disconnected => "Not connected",
        ConnectionState::WaitingForScanStop => "Preparing…",
        ConnectionState::CheckingBond => "Checking pairing…",
        ConnectionState::RemovingBond => "Clearing old pairing…",
        ConnectionState::Pairing => "Pairing…",
        ConnectionState::Connecting => "Connecting…",
        ConnectionState::Configuring => "Setting up audio…",
        ConnectionState::Ready => "Connected",
        ConnectionState::Disconnecting => "Disconnecting…",
        ConnectionState::Failed => "Connection failed",
    }
}
fn stream_detail(state: crate::StreamState) -> &'static str {
    match state {
        crate::StreamState::Idle => "Idle",
        crate::StreamState::Discovering => "Discovering",
        crate::StreamState::ReadingCapabilities => "Reading capabilities",
        crate::StreamState::Configuring => "Configuring",
        crate::StreamState::Opening => "Opening",
        crate::StreamState::Open => "Ready",
        crate::StreamState::Starting => "Starting",
        crate::StreamState::Streaming => "Playing audio",
        crate::StreamState::Suspending => "Stopping",
        crate::StreamState::Failed => "Failed",
    }
}

fn audio_state_detail(state: u8) -> &'static str {
    match state {
        0 => "Closed",
        1 => "Idle",
        2 => "Configured",
        3 => "Buffering",
        4 => "Playing",
        5 => "Paused",
        6 => "Draining",
        7 => "Stopped",
        8 => "Error",
        _ => "Unknown",
    }
}

fn audio_stage_detail(stage: u8) -> &'static str {
    match stage {
        0 => "Idle",
        1 => "Queued",
        2 => "Prebuffering",
        3 => "AVDTP START",
        4 => "MP3 decode",
        5 => "PCM ready",
        6 => "SBC encode",
        7 => "RTP sent",
        8 => "Draining",
        9 => "Complete",
        10 => "Failed",
        11 => "Decode only",
        _ => "Unknown",
    }
}

#[derive(Copy, Clone)]
struct FixedText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}
impl<const N: usize> Default for FixedText<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}
impl<const N: usize> FixedText<N> {
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}
impl<const N: usize> Write for FixedText<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if s.len() > N - self.len {
            return Err(fmt::Error);
        }
        self.bytes[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len();
        Ok(())
    }
}
fn number(value: i32) -> FixedText<16> {
    let mut out = FixedText::default();
    let _ = write!(out, "{value}");
    out
}
fn unsigned(value: u32) -> FixedText<16> {
    let mut out = FixedText::default();
    let _ = write!(out, "{value}");
    out
}
fn hex(value: u32) -> FixedText<16> {
    let mut out = FixedText::default();
    let _ = write!(out, "0x{value:06X}");
    out
}
