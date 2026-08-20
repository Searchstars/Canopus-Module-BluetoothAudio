use crate::{ConnectionState, Model, ScanState};
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
    tree.navigation_page(1, "耳机")?;
    tree.section(10, "已连接耳机")?;
    match model.connected {
        Some(peer) => tree.action_row(
            11,
            display_name(&peer.name),
            connection_detail(model.connection),
            EVENT_CONNECTED_DETAIL,
            true,
        )?,
        None => tree.status_row(11, "耳机", connection_detail(model.connection))?,
    }
    if model.last_error != 0 {
        let error = number(model.last_error);
        tree.text(12, error.as_str(), TextStyle::Warning)?;
    }
    tree.status_row(
        14,
        "Audio endpoint",
        audio_endpoint_detail(&model.details).as_str(),
    )?;
    if model.selected.is_some() {
        let diagnostic = pairing_diagnostic(model);
        tree.text(13, diagnostic.as_str(), TextStyle::Description)?;
    }
    tree.end()?;
    tree.section(20, "附近耳机")?;
    tree.button(
        21,
        if matches!(model.scan, ScanState::Scanning | ScanState::Starting) {
            "停止扫描"
        } else {
            "开始扫描"
        },
        EVENT_SCAN,
        true,
    )?;
    tree.status_row(22, "扫描", scan_detail(model.scan))?;
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
        let _ = write!(text, "还有 {} 个结果无法保留", model.devices.dropped());
        tree.text(30, text.as_str(), TextStyle::Description)?;
    }
    tree.button(31, "刷新结果", EVENT_REFRESH, true)?;
    tree.end()?;
    tree.end()?;
    tree.commit()
}

pub fn detail(model: &Model) -> Result<Snapshot, UiError> {
    let mut tree = Tree::begin();
    tree.navigation_page(1, "耳机详情")?;
    let Some(peer) = model.connected else {
        tree.text(10, "未连接耳机", TextStyle::Description)?;
        tree.end()?;
        return tree.commit();
    };
    tree.section(20, "耳机")?;
    tree.status_row(21, "名称", display_name(&peer.name))?;
    let address = peer.address.text();
    tree.status_row(22, "地址", address.as_str())?;
    let rssi = number(peer.rssi);
    tree.status_row(23, "信号", rssi.as_str())?;
    let cod = hex(model.connected.unwrap().class_of_device);
    tree.status_row(24, "类别", cod.as_str())?;
    tree.status_row(25, "连接", connection_detail(model.connection))?;
    tree.status_row(26, "音频流", stream_detail(model.stream))?;
    tree.end()?;
    tree.section(30, "音频")?;
    let mut mtu = FixedText::<32>::default();
    let _ = write!(
        mtu,
        "media {} / ctl {}",
        model.details.media_mtu, model.details.avrcp_mtu
    );
    tree.status_row(31, "Link MTU", mtu.as_str())?;
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
    let mut avrcp = FixedText::<80>::default();
    let _ = write!(
        avrcp,
        "{} c{} v{} {}/{} ev{} e{} h{:08X}/{}",
        avrcp_state_detail(model.details.avrcp_state),
        model.details.avrcp_cid,
        model.details.avrcp_volume,
        model.details.avrcp_packets_sent,
        model.details.avrcp_packets_received,
        model.details.avrcp_last_event,
        model.details.avrcp_error,
        model.details.avrcp_rx_header,
        model.details.avrcp_rx_length
    );
    tree.status_row(49, "AVRCP", avrcp.as_str())?;
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
    #[cfg(not(feature = "production"))]
    {
        let can_play =
            model.connection == ConnectionState::Ready && model.stream == crate::StreamState::Open;
        tree.button(34, "播放测试音调", EVENT_TEST_TONE, can_play)?;
        tree.button(35, "播放长 MP3", EVENT_LONG_MP3, can_play)?;
        tree.button(44, "仅解码长 MP3", EVENT_LONG_MP3_DECODE_ONLY, can_play)?;
    }
    tree.button(
        36,
        "断开连接",
        EVENT_DISCONNECT,
        model.connection == ConnectionState::Ready,
    )?;
    tree.end()?;
    tree.end()?;
    tree.commit()
}

fn audio_endpoint_detail(details: &crate::LinkDetails) -> FixedText<64> {
    let mut text = FixedText::default();
    let _ = write!(
        text,
        "reg {} probe {} abi {} cmd 0x{:03X}",
        details.audio_register_result,
        details.audio_probe_result,
        details.audio_probe_abi,
        details.audio_last_command,
    );
    text
}

fn pairing_diagnostic(model: &Model) -> FixedText<32> {
    let details = &model.details;
    let mut text = FixedText::<32>::default();
    let _ = write!(
        text,
        "Bond {}/{} · {:02X}",
        details.stock_bond_state, details.device_bond_state, details.pairing_flags
    );
    text
}

fn display_name(name: &crate::DeviceName) -> &str {
    if name.is_empty() {
        "未知耳机"
    } else {
        name.as_str()
    }
}
fn scan_detail(state: ScanState) -> &'static str {
    match state {
        ScanState::Idle => "空闲",
        ScanState::Starting => "正在启动…",
        ScanState::Scanning => "正在扫描…",
        ScanState::Stopping => "正在停止…",
        ScanState::Failed => "扫描失败",
    }
}
fn connection_detail(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Disconnected => "未连接",
        ConnectionState::WaitingForScanStop => "准备中…",
        ConnectionState::CheckingBond => "检查配对中…",
        ConnectionState::RemovingBond => "清除配对中…",
        ConnectionState::Pairing => "配对中…",
        ConnectionState::Connecting => "连接中…",
        ConnectionState::Configuring => "音频设置中…",
        ConnectionState::Ready => "已连接",
        ConnectionState::Disconnecting => "正在断开连接…",
        ConnectionState::Failed => "失败",
    }
}
fn stream_detail(state: crate::StreamState) -> &'static str {
    match state {
        crate::StreamState::Idle => "空闲",
        crate::StreamState::Discovering => "正在发现",
        crate::StreamState::ReadingCapabilities => "正在读取能力",
        crate::StreamState::Configuring => "配置中",
        crate::StreamState::Opening => "打开中",
        crate::StreamState::Open => "就绪",
        crate::StreamState::Starting => "启动中",
        crate::StreamState::Streaming => "正在播放音频",
        crate::StreamState::Suspending => "停止中",
        crate::StreamState::Failed => "失败",
    }
}

fn avrcp_state_detail(state: u8) -> &'static str {
    match state {
        0 => "Idle",
        1 => "Connecting PSM 23",
        2 => "Connected",
        3 => "Disconnecting",
        4 => "Failed",
        _ => "Unknown",
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
