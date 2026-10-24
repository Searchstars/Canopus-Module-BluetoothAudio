//! Semantic pages of the Headphones app: the overview (connected headset and
//! pairing), the headset detail page, and a diagnostics page that keeps every
//! low-level counter out of the user-facing pages.
//!
//! Presentation hints ride on the standard ui-core style record:
//! - a `Title` text is the page's headline, a `Description` text the line under
//!   it, and a `Tonal` text with a colour role is a status pill;
//! - a section title is drawn as a small group header;
//! - a `Filled` button is the page's primary action, a `Destructive` one ends
//!   something.

use crate::{ConnectionState, Model, ScanState, StreamState};
use canopus_ui_core::{
    ColorRole, ComponentVariant, Layout, Snapshot, Style, TextStyle, Tree, UiError,
};
use core::fmt::{self, Write};

pub const EVENT_CONNECTED_DETAIL: u32 = 1;
pub const EVENT_SCAN: u32 = 2;
pub const EVENT_REFRESH: u32 = 3;
pub const EVENT_BACK: u32 = 4;
pub const EVENT_DISCONNECT: u32 = 5;
pub const EVENT_TEST_TONE: u32 = 6;
pub const EVENT_LONG_MP3: u32 = 7;
pub const EVENT_LONG_MP3_DECODE_ONLY: u32 = 8;
pub const EVENT_DIAGNOSTICS: u32 = 9;
pub const EVENT_DEVICE_BASE: u32 = 100;

/// Node keys the device backend dispatches on.
pub mod key {
    pub const PAGE: u32 = 1;
    pub const ICON: u32 = 6;
    pub const HEADLINE: u32 = 2;
    pub const STATUS: u32 = 3;
    pub const SUMMARY: u32 = 4;
    pub const DETAIL: u32 = 5;
    pub const SCAN: u32 = 21;
    pub const TEST_TONE: u32 = 34;
    pub const LONG_MP3: u32 = 35;
    pub const DISCONNECT: u32 = 36;
    pub const DIAGNOSTICS: u32 = 41;
    pub const LONG_MP3_DECODE_ONLY: u32 = 44;
    pub const REFRESH: u32 = 60;
}

/// The headset artwork: the launcher icon, resolved to a path by the device.
pub const ICON_RESOURCE: &str = "appicon_headphones.bin";

// Device error codes the pages explain (see the device crate's `runtime.rs`).
const ERR_STATE: i32 = -1101;
const ERR_REMOTE: i32 = -1105;
const ERR_CODEC_UNSUPPORTED: i32 = -1107;
const ERR_BOND_TIMEOUT: i32 = -1108;
const ERR_HCI_POLICY: i32 = -1109;
const ERR_ADAPTER_UNAVAILABLE: i32 = -1110;
const ERRNO_EBUSY: i32 = -16;

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

/// What the headline says about the headset.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub label: &'static str,
    pub tone: ColorRole,
    pub hint: &'static str,
    /// Error code worth quoting in a report; 0 for none.
    pub error: i32,
}

/// The connection as the user sees it, with what to do next.
pub fn status(model: &Model) -> Status {
    let plain = |label, tone, hint| Status {
        label,
        tone,
        hint,
        error: 0,
    };
    let stream_failed = model.stream == StreamState::Failed;
    match model.connection {
        ConnectionState::Disconnected => plain("未连接", ColorRole::TextSecondary, ""),
        ConnectionState::WaitingForScanStop
        | ConnectionState::CheckingBond
        | ConnectionState::RemovingBond => {
            plain("正在准备", ColorRole::Accent, "请让耳机保持配对模式")
        }
        ConnectionState::Pairing => {
            plain("正在配对", ColorRole::Accent, "如有提示，请在耳机上确认")
        }
        ConnectionState::Connecting => plain("正在连接", ColorRole::Accent, "请让耳机靠近手环"),
        ConnectionState::Configuring if stream_failed => failure(model.last_error),
        ConnectionState::Configuring => plain("正在设置音频", ColorRole::Accent, "马上就好"),
        ConnectionState::Ready if stream_failed => Status {
            label: "播放出错",
            tone: ColorRole::Warning,
            hint: "请重新播放，不行就断开重连",
            error: if model.details.audio_error != 0 {
                model.details.audio_error
            } else {
                model.last_error
            },
        },
        ConnectionState::Ready
            if matches!(model.stream, StreamState::Starting | StreamState::Streaming) =>
        {
            plain("正在播放", ColorRole::Accent, "")
        }
        ConnectionState::Ready => plain("已连接", ColorRole::Success, "可以在音乐应用里播放了"),
        ConnectionState::Disconnecting => plain("正在断开", ColorRole::TextSecondary, ""),
        ConnectionState::Failed => failure(model.last_error),
    }
}

fn failure(error: i32) -> Status {
    let hint = match error {
        ERR_CODEC_UNSUPPORTED => "耳机不支持 SBC 音频格式",
        ERR_BOND_TIMEOUT => "配对超时，请让耳机重新进入配对模式",
        ERR_REMOTE => "耳机拒绝了连接，请重新进入配对模式",
        ERR_ADAPTER_UNAVAILABLE => "请先在手环设置里打开蓝牙",
        ERR_STATE | ERRNO_EBUSY => "蓝牙正忙，请稍后再试",
        ERR_HCI_POLICY => "当前固件暂不支持连接耳机",
        _ => "请让耳机重新进入配对模式后重试",
    };
    Status {
        label: "连接失败",
        tone: ColorRole::Danger,
        hint,
        error,
    }
}

pub fn overview(model: &Model) -> Result<Snapshot, UiError> {
    let mut tree = Tree::begin();
    tree.navigation_page(key::PAGE, "耳机")?;
    let name = match model.connected.as_ref().or(model.selected.as_ref()) {
        Some(peer) => display_name(&peer.name),
        None => "未连接耳机",
    };
    headline(&mut tree, name, model)?;
    if model.connected.is_some() {
        tree.action_row(
            key::DETAIL,
            "耳机详情",
            "音量、音质、断开",
            EVENT_CONNECTED_DETAIL,
            true,
        )?;
    }

    tree.section(20, "连接新耳机")?;
    // Scanning and the rows stay usable only while no connection attempt is
    // under way; searching is the page's main action until a headset is in.
    let idle = matches!(
        model.connection,
        ConnectionState::Disconnected | ConnectionState::Ready | ConnectionState::Failed
    );
    let scanning = matches!(model.scan, ScanState::Scanning | ScanState::Starting);
    tree.button(
        key::SCAN,
        if scanning {
            "停止搜索"
        } else {
            "搜索耳机"
        },
        EVENT_SCAN,
        model.scan != ScanState::Stopping && (idle || scanning),
    )?;
    tree.set_style(
        key::SCAN,
        variant(if scanning || !idle || model.connected.is_some() {
            ComponentVariant::Default
        } else {
            ComponentVariant::Filled
        }),
    )?;
    let entries = model.devices.entries();
    for (index, device) in entries.iter().enumerate() {
        let mut detail = FixedText::<40>::default();
        let _ = detail.write_str(signal(device.rssi));
        // Two headsets of one model only differ by address.
        if entries
            .iter()
            .any(|other| other.address != device.address && other.name == device.name)
        {
            let address = device.address.0;
            let _ = write!(detail, " · 尾号 {:02X}{:02X}", address[4], address[5]);
        }
        tree.action_row(
            device_key(device.address),
            display_name(&device.name),
            detail.as_str(),
            EVENT_DEVICE_BASE + index as u32,
            idle,
        )?;
    }
    let found = !entries.is_empty();
    let note = match model.scan {
        _ if !idle => "",
        ScanState::Starting | ScanState::Scanning if !found => "正在搜索附近的耳机…",
        ScanState::Failed => "搜索没能开始，请稍后再试",
        _ if found => "点按耳机即可连接",
        _ => "请先让耳机进入配对模式",
    };
    if !note.is_empty() {
        tree.text(22, note, TextStyle::Description)?;
    }
    if model.devices.dropped() != 0 {
        let mut text = FixedText::<48>::default();
        let _ = write!(text, "另有 {} 个设备未显示", model.devices.dropped());
        tree.text(30, text.as_str(), TextStyle::Description)?;
    }
    tree.end()?;

    tree.section(40, "")?;
    tree.action_row(
        key::DIAGNOSTICS,
        "诊断信息",
        "排查连接和播放问题",
        EVENT_DIAGNOSTICS,
        true,
    )?;
    tree.end()?;
    tree.end()?;
    tree.commit()
}

pub fn detail(model: &Model) -> Result<Snapshot, UiError> {
    let mut tree = Tree::begin();
    tree.navigation_page(key::PAGE, "耳机详情")?;
    let Some(peer) = model.connected else {
        headline(&mut tree, "未连接耳机", model)?;
        tree.end()?;
        return tree.commit();
    };
    headline(&mut tree, display_name(&peer.name), model)?;

    tree.section(10, "音频")?;
    tree.status_row(11, "音频格式", codec(&model.details).as_str())?;
    tree.status_row(12, "码率", bitrate(&model.details).as_str())?;
    let mut volume = FixedText::<24>::default();
    if model.details.avrcp_state == AVRCP_CONNECTED {
        let percent = u32::from(model.details.avrcp_volume.min(0x7f)) * 100 / 0x7f;
        let _ = write!(volume, "{percent}%");
    } else {
        let _ = volume.write_str("在耳机上调节");
    }
    tree.status_row(13, "音量", volume.as_str())?;
    let address = peer.address.text();
    tree.status_row(14, "设备地址", address.as_str())?;
    tree.end()?;

    #[cfg(not(feature = "production"))]
    {
        let can_play =
            model.connection == ConnectionState::Ready && model.stream == StreamState::Open;
        tree.section(30, "测试")?;
        tree.button(key::TEST_TONE, "播放测试音", EVENT_TEST_TONE, can_play)?;
        tree.button(key::LONG_MP3, "播放长 MP3", EVENT_LONG_MP3, can_play)?;
        tree.button(
            key::LONG_MP3_DECODE_ONLY,
            "仅解码长 MP3",
            EVENT_LONG_MP3_DECODE_ONLY,
            can_play,
        )?;
        tree.end()?;
    }

    tree.section(37, "")?;
    // A link whose audio setup failed still holds channels worth closing.
    let disconnectable = model.connection == ConnectionState::Ready
        || (model.connection == ConnectionState::Configuring
            && model.stream == StreamState::Failed);
    tree.button(
        key::DISCONNECT,
        "断开连接",
        EVENT_DISCONNECT,
        disconnectable,
    )?;
    tree.set_style(key::DISCONNECT, variant(ComponentVariant::Destructive))?;
    tree.action_row(
        key::DIAGNOSTICS,
        "诊断信息",
        "排查连接和播放问题",
        EVENT_DIAGNOSTICS,
        true,
    )?;
    tree.end()?;
    tree.end()?;
    tree.commit()
}

/// Every low-level counter, for bug reports. The first line condenses the
/// state into one code that fits a screenshot.
pub fn diagnostics(model: &Model) -> Result<Snapshot, UiError> {
    let d = &model.details;
    let mut tree = Tree::begin();
    tree.navigation_page(key::PAGE, "诊断信息")?;
    tree.text(2, diagnostic_code(model).as_str(), TextStyle::Description)?;

    tree.section(10, "连接")?;
    let mut state = FixedText::<48>::default();
    let _ = write!(state, "{:?} / {:?}", model.connection, model.stream);
    tree.status_row(11, "状态", state.as_str())?;
    let error = number(model.last_error);
    tree.status_row(12, "最近错误", error.as_str())?;
    tree.status_row(13, "AVDTP", avdtp_detail(d).as_str())?;
    let mut mtu = FixedText::<48>::default();
    let _ = write!(
        mtu,
        "sig {} · media {} · ctl {}",
        d.signaling_mtu, d.media_mtu, d.avrcp_mtu
    );
    tree.status_row(14, "L2CAP MTU", mtu.as_str())?;
    let mut avrcp = FixedText::<80>::default();
    let _ = write!(
        avrcp,
        "{} c{} v{} {}/{} ev{} e{} h{:08X}/{}",
        avrcp_state_detail(d.avrcp_state),
        d.avrcp_cid,
        d.avrcp_volume,
        d.avrcp_packets_sent,
        d.avrcp_packets_received,
        d.avrcp_last_event,
        d.avrcp_error,
        d.avrcp_rx_header,
        d.avrcp_rx_length
    );
    tree.status_row(15, "AVRCP", avrcp.as_str())?;
    tree.status_row(16, "配对", pairing_diagnostic(model).as_str())?;
    tree.status_row(17, "音频端点", audio_endpoint_detail(d).as_str())?;
    tree.end()?;

    tree.section(50, "播放")?;
    tree.status_row(51, "Pipeline", audio_stage_detail(d.audio_stage))?;
    let mut input = FixedText::<32>::default();
    let _ = write!(
        input,
        "{} / {} B",
        audio_state_detail(d.audio_state),
        d.input_used
    );
    tree.status_row(52, "Input", input.as_str())?;
    let mut decoded = FixedText::<32>::default();
    if d.decoded_sample_rate == 0 {
        let _ = decoded.write_str("Waiting for frame");
    } else {
        let _ = write!(
            decoded,
            "{} Hz / {} ch",
            d.decoded_sample_rate, d.decoded_channels
        );
    }
    tree.status_row(53, "MP3 decoded", decoded.as_str())?;
    let mut frames = FixedText::<40>::default();
    let _ = write!(frames, "PCM {} · RTP {}", d.pcm_frames, d.audio_rtp_packets);
    tree.status_row(54, "Frames", frames.as_str())?;
    let mut queue = FixedText::<48>::default();
    let _ = write!(
        queue,
        "q{} c{} o{} r{} s{}",
        d.media_packets_queued,
        d.media_flow_events,
        d.media_tx_outstanding,
        d.media_flow_reclaims,
        d.startup_silence_packets
    );
    tree.status_row(55, "TX queue", queue.as_str())?;
    let mut underruns = FixedText::<32>::default();
    if d.pacing_lag_ms == 0 {
        let _ = write!(underruns, "{}", d.underruns);
    } else {
        let _ = write!(underruns, "{} / late {} ms", d.underruns, d.pacing_lag_ms);
    }
    tree.status_row(56, "Underruns", underruns.as_str())?;
    let audio_error = number(d.audio_error);
    tree.status_row(57, "Audio error", audio_error.as_str())?;
    let mut timing = FixedText::<40>::default();
    let _ = write!(
        timing,
        "{} ms / decode {} ms",
        d.audio_elapsed_ms, d.decode_cpu_ms
    );
    tree.status_row(58, "Timing", timing.as_str())?;
    // `pcm_frames` counts frames at the MP3's own rate (see `AudioInput::add_pcm_frames`).
    let rate = match d.decoded_sample_rate {
        0 => 44_100,
        rate => u64::from(rate),
    };
    let realtime_percent = if d.audio_elapsed_ms == 0 {
        0
    } else {
        (u64::from(d.pcm_frames) * 100_000 / (rate * u64::from(d.audio_elapsed_ms))) as u32
    };
    let rtp_per_second = if d.audio_elapsed_ms == 0 {
        0
    } else {
        (u64::from(d.audio_rtp_packets) * 1_000 / u64::from(d.audio_elapsed_ms)) as u32
    };
    let mut speed = FixedText::<40>::default();
    let _ = write!(
        speed,
        "{}% realtime · {} pkt/s",
        realtime_percent, rtp_per_second
    );
    tree.status_row(59, "Rate", speed.as_str())?;
    let mut startup = FixedText::<48>::default();
    let _ = write!(
        startup,
        "{} ms (q{} p{} a{})",
        d.startup_ms, d.startup_queue_ms, d.startup_prepare_ms, d.startup_avdtp_ms,
    );
    tree.status_row(61, "Startup", startup.as_str())?;
    tree.end()?;

    tree.section(62, "")?;
    tree.button(key::REFRESH, "刷新", EVENT_REFRESH, true)?;
    tree.end()?;
    tree.end()?;
    tree.commit()
}

/// Artwork, name, status pill and the line under it.
fn headline(tree: &mut Tree, name: &str, model: &Model) -> Result<(), UiError> {
    let status = status(model);
    tree.image(key::ICON, 1, ICON_RESOURCE, Layout::default())?;
    tree.text(key::HEADLINE, name, TextStyle::Title)?;
    tree.text(key::STATUS, status.label, TextStyle::Body)?;
    tree.set_style(
        key::STATUS,
        Style {
            variant: ComponentVariant::Tonal as u16,
            foreground: status.tone as u16,
            ..Style::default()
        },
    )?;
    let mut summary = FixedText::<128>::default();
    let _ = if status.label == "正在播放" {
        summary.write_str(codec(&model.details).as_str())
    } else {
        summary.write_str(status.hint)
    };
    if status.error != 0 {
        let _ = write!(summary, "\n错误 {}", status.error);
    }
    if !summary.as_str().is_empty() {
        tree.text(key::SUMMARY, summary.as_str(), TextStyle::Description)?;
    }
    Ok(())
}

fn variant(variant: ComponentVariant) -> Style {
    Style {
        variant: variant as u16,
        ..Style::default()
    }
}

const AVRCP_CONNECTED: u8 = 2;

/// "SBC 44.1 kHz · 立体声" once a stream configuration exists.
fn codec(details: &crate::LinkDetails) -> FixedText<48> {
    let mut text = FixedText::default();
    let _ = text.write_str(if details.bitpool == 0 {
        "等待协商"
    } else if details.sbc_frequency_channel & 0x01 != 0 {
        "SBC 44.1 kHz · 联合立体声"
    } else {
        "SBC 44.1 kHz · 立体声"
    });
    text
}

/// Bitrate of the negotiated SBC stream: 128-sample frames of
/// `12 + 2 * bitpool` bytes (one more in joint stereo) at 44.1 kHz.
fn bitrate(details: &crate::LinkDetails) -> FixedText<24> {
    let mut text = FixedText::default();
    if details.bitpool == 0 {
        let _ = text.write_str("—");
        return text;
    }
    let joint = u32::from(details.sbc_frequency_channel & 0x01);
    let frame = 12 + joint + 2 * u32::from(details.bitpool);
    let _ = write!(text, "{} kbps", frame * 8 * 44_100 / 128 / 1_000);
    text
}

fn signal(rssi: i32) -> &'static str {
    match rssi {
        -60.. => "信号强",
        -75..=-61 => "信号中",
        _ => "信号弱",
    }
}

/// `C<connection>S<stream> E<error> A<rejected signal>/<code> B<bitpool>
/// Q<queued>/<completed>/<outstanding>/<written off> U<underruns>`.
fn diagnostic_code(model: &Model) -> FixedText<96> {
    let d = &model.details;
    let mut text = FixedText::default();
    let _ = write!(
        text,
        "C{}S{} E{} A{:02X}/{:02X} B{} Q{}/{}/{}/{} U{}",
        model.connection as u8,
        model.stream as u8,
        model.last_error,
        d.avdtp_reject >> 8,
        d.avdtp_reject & 0xff,
        d.bitpool,
        d.media_packets_queued,
        d.media_flow_events,
        d.media_tx_outstanding,
        d.media_flow_reclaims,
        d.underruns
    );
    text
}

fn avdtp_detail(details: &crate::LinkDetails) -> FixedText<64> {
    let mut text = FixedText::default();
    let caps = &details.sbc_caps;
    if details.bitpool == 0 && caps != &[0u8; 6] {
        let _ = write!(
            text,
            "caps {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
            caps[0], caps[1], caps[2], caps[3], caps[4], caps[5]
        );
    } else {
        let _ = write!(
            text,
            "SEID {} · {:02X} bp{}",
            details.remote_seid, details.sbc_frequency_channel, details.bitpool
        );
    }
    if details.avdtp_reject != 0 {
        let _ = write!(
            text,
            " · rej {:02X}/{:02X}",
            details.avdtp_reject >> 8,
            details.avdtp_reject & 0xff
        );
    }
    text
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
