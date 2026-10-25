//! LVX renderer: maps committed semantic snapshots onto the firmware page.
//! Status, action and switch rows are the stock list rows; the headline
//! (artwork, name, status pill, summary), group headers and pill buttons are
//! styled labels and containers built here. Event bindings are
//! generation-checked. Firmware widget pointers live only here and are
//! cleared on page destruction. LVX is never touched from Bluetooth or timer
//! callbacks — only from the page owner thread (create/resume/row events).

use canopus_target_private::*;
use canopus_ui_core::{ColorRole, ComponentVariant, NodeKind, Snapshot, TextStyle};

use super::native_app::{self, PAGE_COUNT, PAGE_OVERVIEW, page_descriptor_ptr};
use super::runtime::runtime;

static EMPTY_TEXT: [u8; 1] = [0];
const REFRESH_PERIOD_MS: u32 = 250;

const MAX_LABELS: usize = 12;
const MAX_BUTTONS: usize = 4;
/// Button event cookies are offset past every row slot.
const BUTTON_COOKIE: usize = 0x80;

/// LVGL v9.0 style property ids, checked against the firmware's own
/// `lv_obj_set_style_*` wrappers.
mod prop {
    pub const RADIUS: u32 = 12;
    pub const PAD_TOP: u32 = 16;
    pub const PAD_BOTTOM: u32 = 17;
    pub const PAD_LEFT: u32 = 18;
    pub const PAD_RIGHT: u32 = 19;
    pub const BG_COLOR: u32 = 28;
    pub const BG_OPA: u32 = 29;
    pub const TEXT_COLOR: u32 = 88;
    pub const TEXT_ALIGN: u32 = 94;
    /// Opacity of the whole object, children included.
    pub const OPA: u32 = 95;
}
const TEXT_ALIGN_LEFT: u32 = 1;
const TEXT_ALIGN_CENTER: u32 = 2;
const LABEL_LONG_WRAP: u32 = 0;
const FLAG_CLICKABLE: u32 = 1 << 1;
const FLAG_SCROLLABLE: u32 = 1 << 4;
const STATE_PRESSED: u32 = 0x0020;
const ALIGN_CENTER: u32 = 9;
/// `LV_SIZE_CONTENT`: the object sizes itself to its content.
const SIZE_CONTENT: i32 = (1 << 29) | 2001;
const OPA_COVER: u32 = 255;
const OPA_DISABLED: u32 = 102;
const OPA_PRESSED: u32 = 180;

const BAND_11: bool = cfg!(any(
    feature = "target-xiaomi-band-11-4-100-139",
    feature = "target-xiaomi-band-11-4-100-155",
    feature = "target-xiaomi-band-11-4-100-179"
));
/// Stock list rows span the content width less this margin.
const ROW_INSET: i32 = 4;
/// Group header text lines up with the stock rows' own text.
const HEADER_PAD: u32 = 14;
/// Centred text keeps clear of the rounded screen edge.
const TEXT_INSET: i32 = 24;
const BUTTON_HEIGHT: i32 = if BAND_11 { 56 } else { 64 };

mod color {
    pub const WHITE: u32 = 0xFF_FFFF;
    pub const SECONDARY: u32 = 0x8E_8E93;
    pub const ACCENT: u32 = 0x2D_7CF6;
    pub const SUCCESS: u32 = 0x30_D158;
    pub const WARNING: u32 = 0xFF_9F0A;
    pub const DANGER: u32 = 0xFF_453A;
    pub const SURFACE: u32 = 0x2A_2A2A;
    pub const ACCENT_TINT: u32 = 0x10_284D;
    pub const SUCCESS_TINT: u32 = 0x0F_3D1D;
    pub const WARNING_TINT: u32 = 0x4A_3008;
    pub const DANGER_TINT: u32 = 0x4D_1613;
}

/// Vertical spacing between stacked elements.
mod gap {
    pub const TOP: i32 = 4;
    pub const AFTER_ICON: i32 = 12;
    pub const AFTER_TITLE: i32 = 10;
    pub const AFTER_PILL: i32 = 10;
    pub const AFTER_TEXT: i32 = 6;
    pub const BEFORE_GROUP: i32 = 22;
    pub const AFTER_HEADER: i32 = 6;
}

#[derive(Copy, Clone)]
#[repr(C)]
struct Binding {
    generation: u32,
    key: u32,
    event_id: u32,
    enabled: bool,
}

const NO_BINDING: Binding = Binding {
    generation: 0,
    key: 0,
    event_id: 0,
    enabled: false,
};

/// How a label is dressed. A label object keeps the font it was created
/// with, so only labels of the same size are reused for one another.
#[derive(Copy, Clone, PartialEq, Eq)]
enum LabelRole {
    Title,
    Description,
    Warning,
    Body,
    Pill(ColorRole),
    Header,
}

impl LabelRole {
    fn large(self) -> bool {
        self == LabelRole::Title
    }

    fn code(self) -> u32 {
        match self {
            LabelRole::Title => 1,
            LabelRole::Description => 2,
            LabelRole::Warning => 3,
            LabelRole::Body => 4,
            LabelRole::Header => 5,
            LabelRole::Pill(role) => 0x100 | role as u32,
        }
    }
}

/// What a stacked object is, for the spacing above the next one.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Placed {
    Icon,
    Title,
    Pill,
    Text,
    Header,
    Row,
    /// A new section starts here.
    Break,
}

#[derive(Copy, Clone)]
struct PageBackend {
    root: *mut core::ffi::c_void,
    content_root: *mut core::ffi::c_void,
    page_title: *mut core::ffi::c_void,
    refresh_timer: *mut core::ffi::c_void,
    icon: *mut core::ffi::c_void,
    rows: [*mut core::ffi::c_void; UI_MAX_ROWS],
    row_kinds: [u8; UI_MAX_ROWS],
    row_keys: [u32; UI_MAX_ROWS],
    row_hashes: [u32; UI_MAX_ROWS],
    bindings: [Binding; UI_MAX_ROWS],
    labels: [*mut core::ffi::c_void; MAX_LABELS],
    label_keys: [u32; MAX_LABELS],
    label_large: [bool; MAX_LABELS],
    label_hashes: [u32; MAX_LABELS],
    buttons: [*mut core::ffi::c_void; MAX_BUTTONS],
    button_labels: [*mut core::ffi::c_void; MAX_BUTTONS],
    button_keys: [u32; MAX_BUTTONS],
    button_hashes: [u32; MAX_BUTTONS],
    button_bindings: [Binding; MAX_BUTTONS],
    rendered_generation: u32,
    layout_hash: u32,
    layout_count: u32,
    page_index: u8,
    layout_valid: bool,
    active: bool,
    interactive: bool,
    refresh_failed: bool,
}

const fn empty_backend() -> PageBackend {
    PageBackend {
        root: core::ptr::null_mut(),
        content_root: core::ptr::null_mut(),
        page_title: core::ptr::null_mut(),
        refresh_timer: core::ptr::null_mut(),
        icon: core::ptr::null_mut(),
        rows: [core::ptr::null_mut(); UI_MAX_ROWS],
        row_kinds: [0; UI_MAX_ROWS],
        row_keys: [0; UI_MAX_ROWS],
        row_hashes: [0; UI_MAX_ROWS],
        bindings: [NO_BINDING; UI_MAX_ROWS],
        labels: [core::ptr::null_mut(); MAX_LABELS],
        label_keys: [0; MAX_LABELS],
        label_large: [false; MAX_LABELS],
        label_hashes: [0; MAX_LABELS],
        buttons: [core::ptr::null_mut(); MAX_BUTTONS],
        button_labels: [core::ptr::null_mut(); MAX_BUTTONS],
        button_keys: [0; MAX_BUTTONS],
        button_hashes: [0; MAX_BUTTONS],
        button_bindings: [NO_BINDING; MAX_BUTTONS],
        rendered_generation: 0,
        layout_hash: 0,
        layout_count: 0,
        page_index: 0,
        layout_valid: false,
        active: false,
        interactive: false,
        refresh_failed: false,
    }
}

static mut PAGES: [PageBackend; PAGE_COUNT] = [empty_backend(); PAGE_COUNT];

fn page_backend(index: usize) -> &'static mut PageBackend {
    // SAFETY: page indices are validated by every caller against PAGE_COUNT;
    // the firmware serializes page lifecycle callbacks on the UI thread.
    // `addr_of_mut!` avoids the `static_mut_refs` deny lint.
    unsafe {
        &mut *core::ptr::addr_of_mut!(PAGES)
            .cast::<PageBackend>()
            .add(index)
    }
}

extern "C" fn refresh_timer(timer: *mut core::ffi::c_void) {
    if timer.is_null() {
        return;
    }
    for page_index in 0..PAGE_COUNT {
        let backend = page_backend(page_index);
        if backend.refresh_timer != timer {
            continue;
        }
        if backend.active && backend.interactive && !backend.refresh_failed {
            let rendered_generation = backend.rendered_generation;
            if super::rebuild_if_changed(page_index, rendered_generation) != 0 {
                // A transient or capacity/LVX failure must not create a 4 Hz
                // rebuild loop. Resume or an explicit Refresh retries it.
                backend.refresh_failed = true;
            }
        }
        return;
    }
}

// ---------------------------------------------------------------------------
// Page lifecycle (delegated from native_app)
// ---------------------------------------------------------------------------

pub fn page_create(page_index: usize, root: *mut core::ffi::c_void) -> i32 {
    if page_index >= PAGE_COUNT || root.is_null() {
        return -1;
    }
    let backend = page_backend(page_index);
    // A prior destroy zeroed the backend; adopt the fresh firmware root.
    backend.root = root;
    backend.page_index = page_index as u8;
    backend.active = true;
    backend.interactive = true;
    let result = super::rebuild(page_index);
    if result != 0 {
        *page_backend(page_index) = empty_backend();
        return result;
    }
    let timer = unsafe {
        lvx_timer_create(
            refresh_timer,
            REFRESH_PERIOD_MS,
            page_index as *mut core::ffi::c_void,
        )
    };
    if timer.is_null() {
        *page_backend(page_index) = empty_backend();
        return -1;
    }
    page_backend(page_index).refresh_timer = timer;
    0
}

pub fn page_resume(page_index: usize) -> i32 {
    if page_index >= PAGE_COUNT {
        return -1;
    }
    let backend = page_backend(page_index);
    if !backend.active {
        return -1;
    }
    backend.interactive = true;
    backend.refresh_failed = false;
    super::rebuild(page_index)
}

pub fn page_pause(page_index: usize) -> i32 {
    if page_index >= PAGE_COUNT {
        return -1;
    }
    page_backend(page_index).interactive = false;
    0
}

pub fn page_destroy(page_index: usize) -> i32 {
    if page_index >= PAGE_COUNT {
        return -1;
    }
    let backend = page_backend(page_index);
    backend.active = false;
    backend.interactive = false;
    if !backend.refresh_timer.is_null() {
        unsafe { lvx_timer_delete(backend.refresh_timer) };
        backend.refresh_timer = core::ptr::null_mut();
    }
    // Drop every firmware widget pointer; the next create starts empty.
    *backend = empty_backend();
    0
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

pub fn navigate(page_index: usize) {
    if page_index >= PAGE_COUNT {
        return;
    }
    if let Some(key) = runtime().app.page_key(page_index) {
        unsafe { activity_navigate(key, 0, 0, 0) };
    }
}

pub fn back(page_index: usize) {
    unsafe { activity_finish(page_descriptor_ptr(page_index)) };
}

// ---------------------------------------------------------------------------
// Event dispatch (firmware LVX events -> module actions)
// ---------------------------------------------------------------------------

fn encoded_cookie(page_index: usize, slot: usize) -> usize {
    (page_index << 8) | slot
}

extern "C" fn row_event(event: *mut core::ffi::c_void) {
    if event.is_null() {
        return;
    }
    // SAFETY: `event` is a firmware LVX event object from the page owner thread.
    let code = unsafe { lvx_event_get_code(event) };
    let encoded = unsafe { lvx_event_get_user_data(event) };
    let page_index = encoded >> 8;
    let slot = encoded & 0xFF;
    if page_index >= PAGE_COUNT {
        return;
    }
    let backend = page_backend(page_index);
    if !backend.active || !backend.interactive {
        return;
    }
    let binding = if slot >= BUTTON_COOKIE {
        let index = slot - BUTTON_COOKIE;
        if index >= MAX_BUTTONS || code != EVENT_CLICKED {
            return;
        }
        backend.button_bindings[index]
    } else {
        if slot >= UI_MAX_ROWS {
            return;
        }
        // Switch rows register on the trailing object for LV_EVENT_ALL and
        // act only on VALUE_CHANGED; rows register for CLICKED.
        if backend.row_kinds[slot] == ROW_SWITCH {
            if code != EVENT_VALUE_CHANGED {
                return;
            }
        } else if code != EVENT_CLICKED {
            return;
        }
        backend.bindings[slot]
    };
    if binding.event_id == 0 || !binding.enabled {
        return;
    }
    super::handle_ui_event(
        page_index,
        binding.generation,
        binding.key,
        binding.event_id,
    );
}

extern "C" fn page_title_back(event: *mut core::ffi::c_void) {
    if event.is_null() {
        return;
    }
    // SAFETY: the title back callback is registered with the page context
    // cookie (page_index << 8).
    let encoded = unsafe { lvx_event_get_user_data(event) };
    let page_index = encoded >> 8;
    if page_index >= PAGE_COUNT || page_index == PAGE_OVERVIEW {
        return;
    }
    let backend = page_backend(page_index);
    if !backend.active || !backend.interactive {
        return;
    }
    backend.interactive = false;
    back(page_index);
}

// ---------------------------------------------------------------------------
// Snapshot render
// ---------------------------------------------------------------------------

fn hash_word(mut hash: u32, value: u32) -> u32 {
    for byte in value.to_le_bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    hash
}

fn hash_text(mut hash: u32, text: &str) -> u32 {
    for byte in text.as_bytes() {
        hash = (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
    }
    hash_word(hash, text.len() as u32)
}

/// Identifies the object structure of a snapshot: which kinds sit in which
/// order under which keys.
fn layout_fingerprint(snapshot: &Snapshot) -> (u32, u32) {
    let mut hash = 0x811C_9DC5u32;
    let mut count = 0u32;
    for node in snapshot.nodes.iter().take(snapshot.node_count as usize) {
        let marker = match node.kind() {
            Some(NodeKind::Text) => 1,
            Some(NodeKind::StatusRow) => 2,
            Some(NodeKind::Button) => 3,
            Some(NodeKind::ActionRow) => 4,
            Some(NodeKind::SwitchRow) => 5,
            Some(NodeKind::Image) => 6,
            Some(NodeKind::Section) => 7,
            _ => continue,
        };
        hash = hash_word(hash, marker);
        hash = hash_word(hash, node.key);
        count += 1;
    }
    (hash, count)
}

fn row_content_hash(primary: &str, secondary: &str, selected: u32, enabled: bool) -> u32 {
    let hash = hash_text(0x811C_9DC5, primary);
    let hash = hash_text(hash, secondary);
    hash_word(hash_word(hash, selected), u32::from(enabled))
}

fn target_row_kind(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::SwitchRow => ROW_SWITCH,
        NodeKind::ActionRow => ROW_ACTION,
        _ => ROW_STATUS,
    }
}

fn is_row(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::StatusRow | NodeKind::ActionRow | NodeKind::SwitchRow
    )
}

fn snapshot_uses_row(snapshot: &Snapshot, kind: u8, key: u32) -> bool {
    snapshot
        .nodes
        .iter()
        .take(snapshot.node_count as usize)
        .any(|node| {
            node.key == key
                && node.kind().is_some_and(|node_kind| {
                    is_row(node_kind) && target_row_kind(node_kind) == kind
                })
        })
}

fn find_row(
    backend: &PageBackend,
    snapshot: &Snapshot,
    kind: u8,
    key: u32,
    used_mask: u32,
) -> Option<usize> {
    let mut reusable = None;
    let mut empty = None;
    for i in 0..UI_MAX_ROWS {
        if backend.rows[i].is_null() {
            if empty.is_none() {
                empty = Some(i);
            }
        } else if backend.row_kinds[i] == kind && (used_mask & (1 << i)) == 0 {
            if backend.row_keys[i] == key {
                return Some(i);
            }
            if reusable.is_none() && !snapshot_uses_row(snapshot, kind, backend.row_keys[i]) {
                reusable = Some(i);
            }
        }
    }
    reusable.or(empty)
}

fn snapshot_has_key(snapshot: &Snapshot, key: u32) -> bool {
    snapshot
        .nodes
        .iter()
        .take(snapshot.node_count as usize)
        .any(|node| node.key == key)
}

/// A label slot for `key`: its own, else one of the same font that no node of
/// this snapshot claims, else a new one.
fn find_label(
    backend: &PageBackend,
    snapshot: &Snapshot,
    key: u32,
    large: bool,
    used_mask: u32,
) -> Option<usize> {
    let free = |i: usize| {
        used_mask & (1 << i) == 0 && !backend.labels[i].is_null() && backend.label_large[i] == large
    };
    (0..MAX_LABELS)
        .find(|&i| free(i) && backend.label_keys[i] == key)
        .or_else(|| {
            (0..MAX_LABELS).find(|&i| free(i) && !snapshot_has_key(snapshot, backend.label_keys[i]))
        })
        .or_else(|| (0..MAX_LABELS).find(|&i| backend.labels[i].is_null()))
}

fn find_button(
    backend: &PageBackend,
    snapshot: &Snapshot,
    key: u32,
    used_mask: u32,
) -> Option<usize> {
    let free = |i: usize| used_mask & (1 << i) == 0 && !backend.buttons[i].is_null();
    (0..MAX_BUTTONS)
        .find(|&i| free(i) && backend.button_keys[i] == key)
        .or_else(|| {
            (0..MAX_BUTTONS)
                .find(|&i| free(i) && !snapshot_has_key(snapshot, backend.button_keys[i]))
        })
        .or_else(|| (0..MAX_BUTTONS).find(|&i| backend.buttons[i].is_null()))
}

/// Text colour and tinted background of a status pill.
fn role_color(role: ColorRole) -> (u32, u32) {
    match role {
        ColorRole::Accent => (color::ACCENT, color::ACCENT_TINT),
        ColorRole::Success => (color::SUCCESS, color::SUCCESS_TINT),
        ColorRole::Warning => (color::WARNING, color::WARNING_TINT),
        ColorRole::Danger => (color::DANGER, color::DANGER_TINT),
        _ => (color::SECONDARY, color::SURFACE),
    }
}

fn label_role(snapshot: &Snapshot, index: usize) -> LabelRole {
    let Some(style) = snapshot.style(index) else {
        return LabelRole::Body;
    };
    if style.variant == ComponentVariant::Tonal as u16 {
        let role = [
            ColorRole::Accent,
            ColorRole::Success,
            ColorRole::Warning,
            ColorRole::Danger,
        ]
        .into_iter()
        .find(|role| *role as u16 == style.foreground)
        .unwrap_or(ColorRole::TextSecondary);
        return LabelRole::Pill(role);
    }
    match style.text_style {
        x if x == TextStyle::Title as u16 => LabelRole::Title,
        x if x == TextStyle::Description as u16 => LabelRole::Description,
        x if x == TextStyle::Warning as u16 => LabelRole::Warning,
        _ => LabelRole::Body,
    }
}

/// Sets every style property a label can carry, always the full set in
/// ascending id order: the first call creates each property and later calls
/// only overwrite values, which keeps `lv_style_set_prop` on its cheap path.
unsafe fn dress_label(label: *mut core::ffi::c_void, role: LabelRole, width: i32) {
    let (text, background, opacity, align) = match role {
        LabelRole::Title | LabelRole::Body => (color::WHITE, 0, 0, TEXT_ALIGN_CENTER),
        LabelRole::Description => (color::SECONDARY, 0, 0, TEXT_ALIGN_CENTER),
        LabelRole::Warning => (color::WARNING, 0, 0, TEXT_ALIGN_CENTER),
        LabelRole::Header => (color::SECONDARY, 0, 0, TEXT_ALIGN_LEFT),
        LabelRole::Pill(role) => {
            let (text, tint) = role_color(role);
            (text, tint, OPA_COVER, TEXT_ALIGN_CENTER)
        }
    };
    let pill = matches!(role, LabelRole::Pill(_));
    let (pad_vertical, pad_horizontal) = match role {
        LabelRole::Pill(_) => (6, 18),
        LabelRole::Header => (0, HEADER_PAD),
        _ => (0, 0),
    };
    unsafe {
        for (id, value) in [
            (prop::RADIUS, if pill { 100 } else { 0 }),
            (prop::PAD_TOP, pad_vertical),
            (prop::PAD_BOTTOM, pad_vertical),
            (prop::PAD_LEFT, pad_horizontal),
            (prop::PAD_RIGHT, pad_horizontal),
            (prop::BG_COLOR, background),
            (prop::BG_OPA, opacity),
            (prop::TEXT_COLOR, text),
            (prop::TEXT_ALIGN, align),
        ] {
            lvx_object_set_local_style_u32(label, id, value, 0);
        }
        if pill {
            lvx_object_set_size(label, SIZE_CONTENT, SIZE_CONTENT);
        } else {
            lvx_label_set_long_mode(label, LABEL_LONG_WRAP);
            lvx_object_set_size(label, width, SIZE_CONTENT);
        }
    }
}

unsafe fn create_label(parent: *mut core::ffi::c_void, large: bool) -> *mut core::ffi::c_void {
    let label = unsafe { lvx_label_create(parent) };
    if label.is_null() {
        return label;
    }
    let font = if large {
        STYLE_MISANS_DEMIBOLD_32
    } else {
        STYLE_MISANS_REGULAR_24
    };
    unsafe {
        lvx_style_apply(label, font as *const core::ffi::c_void, 255, 0);
        lvx_object_clear_flag(label, FLAG_CLICKABLE);
    }
    label
}

/// A rounded, full-width button: a bare container holding one centred label.
unsafe fn create_button(
    parent: *mut core::ffi::c_void,
    cookie: usize,
) -> (*mut core::ffi::c_void, *mut core::ffi::c_void) {
    let button = unsafe { lvx_object_create(parent) };
    if button.is_null() {
        return (button, core::ptr::null_mut());
    }
    unsafe {
        lvx_object_remove_style_all(button);
        lvx_object_clear_flag(button, FLAG_SCROLLABLE);
        lvx_object_add_flag(button, FLAG_CLICKABLE);
        lvx_object_set_size(button, CONTENT_WIDTH - ROW_INSET, BUTTON_HEIGHT);
        for (id, value) in [
            (prop::RADIUS, (BUTTON_HEIGHT / 2) as u32),
            (prop::PAD_TOP, 0),
            (prop::PAD_BOTTOM, 0),
            (prop::PAD_LEFT, 0),
            (prop::PAD_RIGHT, 0),
            (prop::BG_COLOR, color::SURFACE),
            (prop::BG_OPA, OPA_COVER),
            (prop::OPA, OPA_COVER),
        ] {
            lvx_object_set_local_style_u32(button, id, value, 0);
        }
        lvx_object_set_local_style_u32(button, prop::OPA, OPA_PRESSED, STATE_PRESSED);
        lvx_event_add(
            button,
            row_event,
            EVENT_CLICKED,
            cookie as *mut core::ffi::c_void,
        );
    }
    let label = unsafe { create_label(button, false) };
    if !label.is_null() {
        unsafe {
            lvx_object_set_local_style_u32(label, prop::TEXT_COLOR, color::WHITE, 0);
            lvx_object_align(label, ALIGN_CENTER, 0, 0);
        }
    }
    (button, label)
}

unsafe fn dress_button(
    button: *mut core::ffi::c_void,
    label: *mut core::ffi::c_void,
    variant: u16,
    enabled: bool,
) {
    let (background, text) = match variant {
        x if x == ComponentVariant::Filled as u16 => (color::ACCENT, color::WHITE),
        x if x == ComponentVariant::Destructive as u16 => (color::SURFACE, color::DANGER),
        _ => (color::SURFACE, color::WHITE),
    };
    unsafe {
        lvx_object_set_local_style_u32(button, prop::BG_COLOR, background, 0);
        lvx_object_set_local_style_u32(
            button,
            prop::OPA,
            if enabled { OPA_COVER } else { OPA_DISABLED },
            0,
        );
        if enabled {
            lvx_object_add_flag(button, FLAG_CLICKABLE);
        } else {
            lvx_object_clear_flag(button, FLAG_CLICKABLE);
        }
        lvx_object_set_local_style_u32(label, prop::TEXT_COLOR, text, 0);
    }
}

/// Creates or updates the label in `slot`; returns it and whether its text or
/// dress changed, which can change its height.
fn render_label(
    backend: &mut PageBackend,
    slot: usize,
    key: u32,
    text: &str,
    role: LabelRole,
) -> Option<(*mut core::ffi::c_void, bool)> {
    let mut label = backend.labels[slot];
    if label.is_null() {
        label = unsafe { create_label(backend.content_root, role.large()) };
        if label.is_null() {
            return None;
        }
        backend.labels[slot] = label;
        backend.label_large[slot] = role.large();
        backend.label_hashes[slot] = 0;
    }
    let width = match role {
        LabelRole::Header => CONTENT_WIDTH - ROW_INSET,
        _ => CONTENT_WIDTH - TEXT_INSET,
    };
    let hash = hash_word(hash_text(0x811C_9DC5, text), role.code());
    let changed = backend.label_hashes[slot] != hash;
    if changed {
        unsafe {
            dress_label(label, role, width);
            lvx_label_set_text(label, text.as_ptr());
        }
        backend.label_hashes[slot] = hash;
    }
    unsafe { lvx_set_hidden(label, 0) };
    backend.label_keys[slot] = key;
    Some((label, changed))
}

/// Space above an object, by what it is and what precedes it.
fn spacing(previous: Option<Placed>, next: Placed, new_group: bool) -> i32 {
    let Some(previous) = previous else {
        return gap::TOP;
    };
    if new_group {
        return gap::BEFORE_GROUP;
    }
    match (previous, next) {
        (Placed::Icon, _) => gap::AFTER_ICON,
        (Placed::Title, _) => gap::AFTER_TITLE,
        (Placed::Pill, _) => gap::AFTER_PILL,
        (Placed::Header, _) => gap::AFTER_HEADER,
        (Placed::Text, Placed::Row) => gap::BEFORE_GROUP,
        (Placed::Text, _) | (Placed::Row, Placed::Text) => gap::AFTER_TEXT + 2,
        _ => ROW_GAP,
    }
}

/// Applies a committed snapshot to the stock LVX page. Returns 0 on success.
pub fn apply_snapshot(page_index: usize, snapshot: &Snapshot) -> i32 {
    if page_index >= PAGE_COUNT || snapshot.node_count == 0 {
        return -1;
    }
    let backend = page_backend(page_index);
    if backend.root.is_null() {
        return -1;
    }
    if backend.content_root.is_null() {
        backend.content_root = unsafe { lvx_content_create(backend.root) };
        if backend.content_root.is_null() {
            return -1;
        }
        unsafe {
            lvx_object_set_size(backend.content_root, CONTENT_WIDTH, CONTENT_HEIGHT);
            lvx_object_align(backend.content_root, ALIGN_TOP_MID, 0, CONTENT_TOP_OFFSET);
        }
    }
    let nodes = &snapshot.nodes[..snapshot.node_count as usize];

    // Capacity: sections and pages are free, everything else is bounded.
    let (mut rows, mut labels, mut buttons, mut images) = (0usize, 0usize, 0usize, 0usize);
    for node in nodes {
        match node.kind() {
            Some(NodeKind::NavigationPage) => {}
            Some(NodeKind::Section) => labels += usize::from(node.primary_len != 0),
            Some(NodeKind::Text) => labels += 1,
            Some(NodeKind::Button) => buttons += 1,
            Some(NodeKind::Image) => images += 1,
            Some(kind) if is_row(kind) => rows += 1,
            _ => return -1,
        }
    }
    if labels > MAX_LABELS || rows > UI_MAX_ROWS || buttons > MAX_BUTTONS || images > 1 {
        return -1;
    }

    let (next_layout_hash, next_layout_count) = layout_fingerprint(snapshot);
    // Any text change can change a height, so the stack is realigned then too;
    // objects that do not move cost LVGL nothing.
    let mut realign = !backend.layout_valid
        || backend.layout_hash != next_layout_hash
        || backend.layout_count != next_layout_count;
    let mut row_mask = 0u32;
    let mut label_mask = 0u32;
    let mut button_mask = 0u32;
    let mut icon_used = false;
    let content = backend.content_root;

    // First pass: create or update every object and record the stacking order.
    let mut order =
        [(core::ptr::null_mut::<core::ffi::c_void>(), Placed::Break); canopus_ui_core::MAX_NODES];
    let mut placed = 0usize;
    let mut push = |object, what| {
        if placed < order.len() {
            order[placed] = (object, what);
            placed += 1;
        }
    };

    for (index, node) in nodes.iter().enumerate() {
        let Some(kind) = node.kind() else {
            return -1;
        };
        let primary = snapshot.primary(node);
        match kind {
            NodeKind::NavigationPage => {
                if backend.page_title.is_null() {
                    // Mode 1 draws the stock back affordance wired to
                    // `page_title_back`; mode 0 passes a NULL back callback, so
                    // no back button is drawn on overview. Keep this nullable
                    // at the ABI boundary: a Rust function pointer cannot
                    // validly contain NULL.
                    let title_mode = u32::from(page_index != PAGE_OVERVIEW);
                    let back_callback = if title_mode != 0 {
                        page_title_back as *const ()
                    } else {
                        core::ptr::null()
                    };
                    let back_context = (page_index << 8) as *mut core::ffi::c_void;
                    backend.page_title = unsafe {
                        lvx_page_title_create(
                            backend.root,
                            primary.as_ptr(),
                            title_mode,
                            back_callback,
                            back_context,
                        )
                    };
                    if backend.page_title.is_null() {
                        return -1;
                    }
                }
                unsafe { lvx_set_hidden(backend.page_title, 0) };
            }
            NodeKind::Section => {
                push(core::ptr::null_mut(), Placed::Break);
                if primary.is_empty() {
                    continue;
                }
                let Some(slot) = find_label(backend, snapshot, node.key, false, label_mask) else {
                    return -1;
                };
                let Some((label, changed)) =
                    render_label(backend, slot, node.key, primary, LabelRole::Header)
                else {
                    return -1;
                };
                realign |= changed;
                label_mask |= 1 << slot;
                push(label, Placed::Header);
            }
            NodeKind::Image => {
                if backend.icon.is_null() {
                    // Without its artwork on disk the headline simply starts
                    // at the name.
                    let Some(path) = native_app::icon_path() else {
                        continue;
                    };
                    let image = unsafe { lvx_image_create(content) };
                    if image.is_null() {
                        return -1;
                    }
                    unsafe {
                        lvx_image_set_src(image, path.as_ptr().cast());
                        lvx_object_clear_flag(image, FLAG_CLICKABLE);
                    }
                    backend.icon = image;
                    realign = true;
                }
                unsafe { lvx_set_hidden(backend.icon, 0) };
                icon_used = true;
                push(backend.icon, Placed::Icon);
            }
            NodeKind::Text => {
                let role = label_role(snapshot, index);
                let Some(slot) = find_label(backend, snapshot, node.key, role.large(), label_mask)
                else {
                    return -1;
                };
                let Some((label, changed)) = render_label(backend, slot, node.key, primary, role)
                else {
                    return -1;
                };
                realign |= changed;
                label_mask |= 1 << slot;
                push(
                    label,
                    match role {
                        LabelRole::Title => Placed::Title,
                        LabelRole::Pill(_) => Placed::Pill,
                        _ => Placed::Text,
                    },
                );
            }
            NodeKind::Button => {
                let Some(slot) = find_button(backend, snapshot, node.key, button_mask) else {
                    return -1;
                };
                if backend.buttons[slot].is_null() {
                    let cookie = encoded_cookie(page_index, BUTTON_COOKIE + slot);
                    let (button, label) = unsafe { create_button(content, cookie) };
                    if button.is_null() || label.is_null() {
                        return -1;
                    }
                    backend.buttons[slot] = button;
                    backend.button_labels[slot] = label;
                    backend.button_hashes[slot] = 0;
                    realign = true;
                }
                let variant = snapshot.style(index).map_or(0, |style| style.variant);
                let hash = hash_word(
                    hash_word(hash_text(0x811C_9DC5, primary), u32::from(variant)),
                    u32::from(node.enabled()),
                );
                if backend.button_hashes[slot] != hash {
                    unsafe {
                        lvx_label_set_text(backend.button_labels[slot], primary.as_ptr());
                        dress_button(
                            backend.buttons[slot],
                            backend.button_labels[slot],
                            variant,
                            node.enabled(),
                        );
                    }
                    backend.button_hashes[slot] = hash;
                }
                unsafe { lvx_set_hidden(backend.buttons[slot], 0) };
                backend.button_keys[slot] = node.key;
                backend.button_bindings[slot] = Binding {
                    generation: snapshot.generation,
                    key: node.key,
                    event_id: node.event_id,
                    enabled: node.enabled(),
                };
                button_mask |= 1 << slot;
                push(backend.buttons[slot], Placed::Row);
            }
            _ if is_row(kind) => {
                // The firmware tests only whether the secondary pointer is
                // NULL. Rust's empty `str` may use the non-dereferenceable
                // dangling sentinel address 1, so pass an actual resident NUL
                // byte for rows without detail text.
                let secondary_text = if node.secondary_len != 0 {
                    snapshot.secondary(node)
                } else {
                    ""
                };
                let secondary = if node.secondary_len != 0 {
                    secondary_text.as_ptr()
                } else {
                    EMPTY_TEXT.as_ptr()
                };
                let row_kind = target_row_kind(kind);
                let trailing = match row_kind {
                    ROW_ACTION => TRAILING_FORWARD,
                    ROW_SWITCH => TRAILING_SWITCH,
                    _ => TRAILING_NONE,
                };
                let Some(slot) = find_row(backend, snapshot, row_kind, node.key, row_mask) else {
                    return -1;
                };
                let mut object = backend.rows[slot];
                let created_now = object.is_null();
                if created_now {
                    let created = unsafe {
                        lvx_list_row_create(content, primary.as_ptr(), secondary, trailing)
                    };
                    if created.is_null() {
                        return -1;
                    }
                    backend.rows[slot] = created;
                    backend.row_kinds[slot] = row_kind;
                    object = created;
                    let event_object = if row_kind == ROW_SWITCH {
                        unsafe { lvx_list_row_trailing(created) }
                    } else {
                        created
                    };
                    if event_object.is_null() {
                        return -1;
                    }
                    let event_code = if row_kind == ROW_SWITCH {
                        EVENT_ALL
                    } else {
                        EVENT_CLICKED
                    };
                    unsafe {
                        lvx_event_add(
                            event_object,
                            row_event,
                            event_code,
                            encoded_cookie(page_index, slot) as *mut core::ffi::c_void,
                        );
                    }
                    realign = true;
                }
                let selected: u8 = if row_kind == ROW_SWITCH {
                    u8::from(node.checked())
                } else {
                    1
                };
                // A tappable row that is not available right now is dimmed.
                let enabled = node.enabled() || row_kind == ROW_STATUS;
                let content_hash =
                    row_content_hash(primary, secondary_text, u32::from(selected), enabled);
                if created_now || backend.row_hashes[slot] != content_hash {
                    unsafe {
                        lvx_list_row_update(
                            object,
                            core::ptr::null(),
                            primary.as_ptr(),
                            secondary,
                            0,
                            selected,
                        );
                        lvx_object_set_local_style_u32(
                            object,
                            prop::OPA,
                            if enabled { OPA_COVER } else { OPA_DISABLED },
                            0,
                        );
                    }
                    backend.row_hashes[slot] = content_hash;
                    realign = true;
                }
                unsafe { lvx_set_hidden(object, 0) };
                backend.row_keys[slot] = node.key;
                backend.bindings[slot] = Binding {
                    generation: snapshot.generation,
                    key: node.key,
                    event_id: node.event_id,
                    enabled: node.enabled(),
                };
                row_mask |= 1 << slot;
                push(object, Placed::Row);
            }
            _ => return -1,
        }
    }

    // Second pass: stack the objects top to bottom.
    if realign {
        let mut previous: Option<(*mut core::ffi::c_void, Placed)> = None;
        let mut new_group = false;
        for &(object, what) in &order[..placed] {
            if what == Placed::Break {
                new_group = previous.is_some();
                continue;
            }
            let gap = spacing(previous.map(|(_, what)| what), what, new_group);
            // `lvx_align_to` against the scrolled content itself would add the
            // current scroll offset and jump the page back to its top, so the
            // first object aligns within its parent and the rest follow it.
            unsafe {
                match previous {
                    None => lvx_object_align(object, ALIGN_TOP_MID, 0, gap),
                    Some((above, _)) => lvx_align_to(object, above, ALIGN_OUT_BOTTOM_MID, 0, gap),
                }
            }
            previous = Some((object, what));
            new_group = false;
        }
    }

    for i in 0..UI_MAX_ROWS {
        if !backend.rows[i].is_null() && (row_mask & (1 << i)) == 0 {
            unsafe { lvx_set_hidden(backend.rows[i], 1) };
            backend.bindings[i] = NO_BINDING;
        }
    }
    for i in 0..MAX_LABELS {
        if !backend.labels[i].is_null() && (label_mask & (1 << i)) == 0 {
            unsafe { lvx_set_hidden(backend.labels[i], 1) };
            backend.label_keys[i] = 0;
        }
    }
    for i in 0..MAX_BUTTONS {
        if !backend.buttons[i].is_null() && (button_mask & (1 << i)) == 0 {
            unsafe { lvx_set_hidden(backend.buttons[i], 1) };
            backend.button_keys[i] = 0;
            backend.button_bindings[i] = NO_BINDING;
        }
    }
    if !backend.icon.is_null() && !icon_used {
        unsafe { lvx_set_hidden(backend.icon, 1) };
    }
    backend.layout_hash = next_layout_hash;
    backend.layout_count = next_layout_count;
    backend.layout_valid = true;
    backend.rendered_generation = snapshot.generation;
    backend.refresh_failed = false;
    0
}
