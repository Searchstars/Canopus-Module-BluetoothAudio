//! Stock LVX renderer: maps committed semantic snapshots onto firmware list
//! rows, labels, page titles and content, and dispatches row/back events with
//! generation-checked bindings. Firmware widget pointers live only here and are
//! cleared on page destruction. LVX is never touched from Bluetooth or timer
//! callbacks — only from the page owner thread (create/resume/row events).

use canopus_target_private::*;
use canopus_ui_core::{NodeKind, Snapshot, TextStyle};

use super::native_app::{APP_ID, PAGE_COUNT, PAGE_OVERVIEW, page_descriptor_ptr};

static EMPTY_TEXT: [u8; 1] = [0];
const REFRESH_PERIOD_MS: u32 = 250;

#[derive(Copy, Clone)]
#[repr(C)]
struct Binding {
    generation: u32,
    key: u32,
    event_id: u32,
    enabled: bool,
}

#[derive(Copy, Clone)]
struct PageBackend {
    root: *mut core::ffi::c_void,
    content_root: *mut core::ffi::c_void,
    page_title: *mut core::ffi::c_void,
    refresh_timer: *mut core::ffi::c_void,
    rows: [*mut core::ffi::c_void; UI_MAX_ROWS],
    labels: [*mut core::ffi::c_void; UI_MAX_LABELS],
    row_kinds: [u8; UI_MAX_ROWS],
    row_keys: [u32; UI_MAX_ROWS],
    row_hashes: [u32; UI_MAX_ROWS],
    label_hashes: [u32; UI_MAX_LABELS],
    bindings: [Binding; UI_MAX_ROWS],
    row_count: u32,
    label_count: u32,
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
        rows: [core::ptr::null_mut(); UI_MAX_ROWS],
        labels: [core::ptr::null_mut(); UI_MAX_LABELS],
        row_kinds: [0; UI_MAX_ROWS],
        row_keys: [0; UI_MAX_ROWS],
        row_hashes: [0; UI_MAX_ROWS],
        label_hashes: [0; UI_MAX_LABELS],
        bindings: [Binding {
            generation: 0,
            key: 0,
            event_id: 0,
            enabled: false,
        }; UI_MAX_ROWS],
        row_count: 0,
        label_count: 0,
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
    let key = ((APP_ID as u32) << 16) | page_index as u32;
    unsafe { activity_navigate(key, 0, 0, 0) };
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
    let row_index = encoded & 0xFF;
    if page_index >= PAGE_COUNT || row_index >= UI_MAX_ROWS {
        return;
    }
    let backend = page_backend(page_index);
    if !backend.active || !backend.interactive {
        return;
    }
    // Switch rows register on the trailing object for LV_EVENT_ALL and act only
    // on VALUE_CHANGED; rows register for CLICKED.
    if backend.row_kinds[row_index] == ROW_SWITCH {
        if code != EVENT_VALUE_CHANGED {
            return;
        }
    } else if code != EVENT_CLICKED {
        return;
    }
    let binding = backend.bindings[row_index];
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

fn layout_fingerprint(snapshot: &Snapshot) -> (u32, u32) {
    let mut hash = 0x811C_9DC5u32;
    let mut count = 0u32;
    for index in 0..snapshot.node_count as usize {
        let node = &snapshot.nodes[index];
        let marker = match node.kind() {
            Some(NodeKind::Text) => 1,
            Some(NodeKind::StatusRow) => 2,
            Some(NodeKind::Button) => 3,
            Some(NodeKind::ActionRow) => 4,
            Some(NodeKind::SwitchRow) => 5,
            _ => continue,
        };
        hash = hash_word(hash, marker);
        hash = hash_word(hash, node.key);
        count += 1;
    }
    (hash, count)
}

fn row_content_hash(primary: &str, secondary: &str, selected: u32) -> u32 {
    let hash = hash_text(0x811C_9DC5, primary);
    let hash = hash_text(hash, secondary);
    hash_word(hash, selected)
}

fn target_row_kind(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::SwitchRow => ROW_SWITCH,
        NodeKind::Button | NodeKind::ActionRow => ROW_ACTION,
        _ => ROW_STATUS,
    }
}

fn snapshot_uses_row(snapshot: &Snapshot, kind: u8, key: u32) -> bool {
    for index in 0..snapshot.node_count as usize {
        let node = &snapshot.nodes[index];
        let is_row = node.kind().is_some_and(|node_kind| {
            matches!(
                node_kind,
                NodeKind::StatusRow | NodeKind::Button | NodeKind::ActionRow | NodeKind::SwitchRow
            ) && target_row_kind(node_kind) == kind
        });
        if node.key == key && is_row {
            return true;
        }
    }
    false
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
        if backend.content_root.is_null() {
            return -1;
        }
    }

    // Capacity check mirrors the C backend: sections/pages are free, labels
    // and rows are bounded.
    let mut visible_rows = 0u32;
    let mut visible_labels = 0u32;
    for index in 0..snapshot.node_count as usize {
        let node = &snapshot.nodes[index];
        match node.kind() {
            Some(NodeKind::Section) | Some(NodeKind::NavigationPage) => {}
            Some(NodeKind::Text) => visible_labels += 1,
            Some(NodeKind::StatusRow)
            | Some(NodeKind::Button)
            | Some(NodeKind::ActionRow)
            | Some(NodeKind::SwitchRow) => visible_rows += 1,
            _ => return -1,
        }
    }
    if visible_labels > UI_MAX_LABELS as u32 || visible_rows > UI_MAX_ROWS as u32 {
        return -1;
    }

    let (next_layout_hash, next_layout_count) = layout_fingerprint(snapshot);
    let layout_changed = !backend.layout_valid
        || backend.layout_hash != next_layout_hash
        || backend.layout_count != next_layout_count;
    let mut used_mask = 0u32;
    let mut label_used = 0u32;
    let mut previous: *mut core::ffi::c_void = core::ptr::null_mut();

    for index in 0..snapshot.node_count as usize {
        let node = &snapshot.nodes[index];
        let kind = match node.kind() {
            Some(kind) => kind,
            None => return -1,
        };
        let primary = snapshot.primary(node);

        if kind == NodeKind::Section {
            continue;
        }
        if kind == NodeKind::NavigationPage {
            let title_mode = if page_index == PAGE_OVERVIEW {
                0u32
            } else {
                1u32
            };
            if backend.page_title.is_null() {
                // Mode 1 draws the stock back affordance wired to
                // `page_title_back`; mode 0 passes a NULL back callback exactly
                // like the C backend, so no back button is drawn on overview.
                // Keep this nullable at the ABI boundary: a Rust function
                // pointer cannot validly contain NULL, even if it is never
                // called, and constructing one lets the compiler eliminate the
                // overview title creation path as unreachable.
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
            previous = backend.page_title;
            continue;
        }
        if kind == NodeKind::Text {
            let mut object = backend.labels[label_used as usize];
            let created_now = object.is_null();
            if created_now {
                let created = unsafe { lvx_label_create(backend.content_root) };
                if created.is_null() {
                    return -1;
                }
                backend.labels[label_used as usize] = created;
                backend.label_count += 1;
                object = created;
            }
            let label_hash = hash_word(
                hash_text(0x811C_9DC5, primary),
                snapshot.styles[index].text_style as u32,
            );
            if created_now || backend.label_hashes[label_used as usize] != label_hash {
                unsafe { lvx_label_set_text(object, primary.as_ptr()) };
                if snapshot.styles[index].text_style == TextStyle::Title as u16 {
                    unsafe {
                        lvx_style_apply(
                            object,
                            STYLE_MISANS_DEMIBOLD_32 as *const core::ffi::c_void,
                            255,
                            0,
                        );
                    }
                }
                backend.label_hashes[label_used as usize] = label_hash;
            }
            unsafe { lvx_set_hidden(object, 0) };
            if layout_changed {
                if previous.is_null() {
                    unsafe { lvx_align_to(object, backend.content_root, ALIGN_TOP_MID, 0, 0) };
                } else {
                    unsafe { lvx_align_to(object, previous, ALIGN_OUT_BOTTOM_MID, 0, 4) };
                }
            }
            previous = object;
            label_used += 1;
            continue;
        }
        if !matches!(
            kind,
            NodeKind::StatusRow | NodeKind::Button | NodeKind::ActionRow | NodeKind::SwitchRow
        ) {
            return -1;
        }

        // The firmware tests only whether the secondary pointer is NULL. Rust's
        // empty `str` may use the non-dereferenceable dangling sentinel address
        // 1, so pass an actual resident NUL byte for rows without detail text.
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
        let slot = match find_row(backend, snapshot, row_kind, node.key, used_mask) {
            Some(slot) => slot,
            None => return -1,
        };
        let mut object = backend.rows[slot];
        let created_now = object.is_null();
        if created_now {
            let created = unsafe {
                lvx_list_row_create(backend.content_root, primary.as_ptr(), secondary, trailing)
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
            backend.row_count += 1;
        }
        let selected: u8 = if row_kind == ROW_SWITCH {
            if node.checked() { 1 } else { 0 }
        } else {
            1
        };
        let content_hash = row_content_hash(primary, secondary_text, u32::from(selected));
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
            }
            backend.row_hashes[slot] = content_hash;
        }
        unsafe { lvx_set_hidden(object, 0) };
        if layout_changed {
            if previous.is_null() {
                unsafe { lvx_align_to(object, backend.content_root, ALIGN_TOP_MID, 0, 0) };
            } else {
                unsafe { lvx_align_to(object, previous, ALIGN_OUT_BOTTOM_MID, 0, ROW_GAP) };
            }
        }
        previous = object;
        backend.row_keys[slot] = node.key;
        backend.bindings[slot] = Binding {
            generation: snapshot.generation,
            key: node.key,
            event_id: node.event_id,
            enabled: node.enabled(),
        };
        used_mask |= 1 << slot;
    }

    for i in 0..UI_MAX_ROWS {
        if !backend.rows[i].is_null() && (used_mask & (1 << i)) == 0 {
            unsafe { lvx_set_hidden(backend.rows[i], 1) };
            backend.bindings[i] = Binding {
                generation: 0,
                key: 0,
                event_id: 0,
                enabled: false,
            };
        }
    }
    for i in label_used as usize..UI_MAX_LABELS {
        if !backend.labels[i].is_null() {
            unsafe { lvx_set_hidden(backend.labels[i], 1) };
        }
    }
    backend.layout_hash = next_layout_hash;
    backend.layout_count = next_layout_count;
    backend.layout_valid = true;
    backend.rendered_generation = snapshot.generation;
    backend.refresh_failed = false;
    0
}
