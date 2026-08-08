use crate::{
    ConnectionState, Model, PAIR_DIAG_BONDED, PAIR_DIAG_DISPLAY, PAIR_DIAG_FILTER_HIT,
    PAIR_DIAG_REMOVE_CONFIRMED, PAIR_DIAG_REMOVE_PENDING, PAIR_DIAG_REQUEST, ScanState,
};
use canopus_ui_core::{Snapshot, TextStyle, Tree, UiError};
use core::fmt::{self, Write};

pub const EVENT_CONNECTED_DETAIL: u32 = 1;
pub const EVENT_SCAN: u32 = 2;
pub const EVENT_REFRESH: u32 = 3;
pub const EVENT_BACK: u32 = 4;
pub const EVENT_DISCONNECT: u32 = 5;
pub const EVENT_TEST_TONE: u32 = 6;
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
    tree.button(
        34,
        "Play test tone",
        EVENT_TEST_TONE,
        model.connection == ConnectionState::Ready && model.stream == crate::StreamState::Open,
    )?;
    tree.button(
        35,
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
        crate::StreamState::Streaming => "Playing test tone",
        crate::StreamState::Suspending => "Stopping",
        crate::StreamState::Failed => "Failed",
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
