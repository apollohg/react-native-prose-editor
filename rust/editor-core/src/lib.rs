pub mod backend;
pub mod collaboration;
pub mod editor;
pub mod history;
pub mod intercept;
pub mod model;
pub mod position;
pub mod registry;
pub mod render;
pub mod schema;
pub mod selection;
pub mod serialize;
pub mod transform;

pub use schema::presets::{prosemirror_schema, tiptap_schema};

uniffi::setup_scaffolding!();

// ---------------------------------------------------------------------------
// UniFFI-exported free functions
// ---------------------------------------------------------------------------

/// Return the crate version string.
#[uniffi::export]
pub fn editor_core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Create a new editor from a JSON config object.
///
/// Config fields (all optional):
/// - `"schema"`: custom schema definition (see `Schema::from_json`)
/// - `"maxLength"`: maximum document length in characters
/// - `"readOnly"`: if `true`, rejects non-API mutations
/// - `"inputFilter"`: regex pattern; only matching characters are inserted
/// - `"allowBase64Images"`: if `true`, parses `<img src="data:image/...">` as image nodes
///
/// An empty object creates a default editor.
/// Falls back to the default Tiptap schema when `"schema"` is absent or invalid.
#[uniffi::export]
pub fn editor_create(config_json: String) -> u64 {
    let config: serde_json::Value =
        serde_json::from_str(&config_json).unwrap_or_else(|_| serde_json::json!({}));

    let schema = config
        .get("schema")
        .and_then(|v| schema::Schema::from_json(v).ok())
        .unwrap_or_else(tiptap_schema);

    let mut interceptors = intercept::InterceptorPipeline::new();

    if let Some(max_length) = config.get("maxLength").and_then(|v| v.as_u64()) {
        interceptors.add(Box::new(intercept::MaxLength::new(max_length as u32)));
    }
    if let Some(true) = config.get("readOnly").and_then(|v| v.as_bool()) {
        interceptors.add(Box::new(intercept::ReadOnly::new(true)));
    }
    if let Some(pattern) = config.get("inputFilter").and_then(|v| v.as_str()) {
        if let Ok(filter) = intercept::InputFilter::new(pattern) {
            interceptors.add(Box::new(filter));
        }
    }

    let allow_base64_images = config
        .get("allowBase64Images")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    registry::EditorRegistry::create(schema, interceptors, allow_base64_images)
}

/// Destroy an editor instance, freeing its resources.
#[uniffi::export]
pub fn editor_destroy(id: u64) {
    registry::EditorRegistry::destroy(id);
}

/// Create a Yjs collaboration session backed by yrs.
#[uniffi::export]
pub fn collaboration_session_create(config_json: String) -> u64 {
    collaboration::CollaborationSessionRegistry::create(&config_json)
}

/// Destroy a collaboration session and free its resources.
#[uniffi::export]
pub fn collaboration_session_destroy(id: u64) {
    collaboration::CollaborationSessionRegistry::destroy(id);
}

/// Return the current shared ProseMirror JSON document for a collaboration session.
#[uniffi::export]
pub fn collaboration_session_get_document_json(id: u64) -> String {
    with_collaboration_session(id, |session| {
        serde_json::to_string(&session.document_json()).unwrap_or_else(|_| "{}".to_string())
    })
    .unwrap_or_else(|| "{}".to_string())
}

/// Return the current shared Yjs document state as a JSON byte array.
#[uniffi::export]
pub fn collaboration_session_get_encoded_state(id: u64) -> String {
    with_collaboration_session(id, |session| {
        serde_json::to_string(&session.encoded_state()).unwrap_or_else(|_| "[]".to_string())
    })
    .unwrap_or_else(|| "[]".to_string())
}

/// Return the current awareness peers for a collaboration session.
#[uniffi::export]
pub fn collaboration_session_get_peers_json(id: u64) -> String {
    with_collaboration_session(id, |session| {
        serde_json::to_string(&session.peers()).unwrap_or_else(|_| "[]".to_string())
    })
    .unwrap_or_else(|| "[]".to_string())
}

/// Start the sync handshake for a collaboration session.
#[uniffi::export]
pub fn collaboration_session_start(id: u64) -> String {
    with_collaboration_session(id, |session| {
        serde_json::to_string(&session.start()).unwrap_or_else(|_| "{}".to_string())
    })
    .unwrap_or_else(|| "{\"error\":\"session not found\"}".to_string())
}

/// Apply a local ProseMirror JSON snapshot to the collaboration session.
#[uniffi::export]
pub fn collaboration_session_apply_local_document_json(id: u64, json: String) -> String {
    with_collaboration_session(id, |session| {
        let value: serde_json::Value = match serde_json::from_str(&json) {
            Ok(value) => value,
            Err(error) => return format!("{{\"error\":\"invalid json: {}\"}}", error),
        };
        serde_json::to_string(&session.apply_local_document(value))
            .unwrap_or_else(|_| "{}".to_string())
    })
    .unwrap_or_else(|| "{\"error\":\"session not found\"}".to_string())
}

/// Apply a durable Yjs encoded state/update represented as a JSON byte array.
#[uniffi::export]
pub fn collaboration_session_apply_encoded_state(id: u64, encoded_state_json: String) -> String {
    with_collaboration_session(id, |session| {
        let encoded_state: Vec<u8> = match serde_json::from_str(&encoded_state_json) {
            Ok(bytes) => bytes,
            Err(error) => {
                return format!("{{\"error\":\"invalid encoded state json: {}\"}}", error)
            }
        };
        match session.apply_encoded_state(encoded_state) {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()),
            Err(error) => format!("{{\"error\":\"{}\"}}", error),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"session not found\"}".to_string())
}

/// Replace the collaboration document with a durable Yjs encoded state/update.
#[uniffi::export]
pub fn collaboration_session_replace_encoded_state(id: u64, encoded_state_json: String) -> String {
    with_collaboration_session(id, |session| {
        let encoded_state: Vec<u8> = match serde_json::from_str(&encoded_state_json) {
            Ok(bytes) => bytes,
            Err(error) => {
                return format!("{{\"error\":\"invalid encoded state json: {}\"}}", error)
            }
        };
        match session.replace_encoded_state(encoded_state) {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()),
            Err(error) => format!("{{\"error\":\"{}\"}}", error),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"session not found\"}".to_string())
}

/// Apply an incoming y-sync binary message encoded as a JSON byte array.
#[uniffi::export]
pub fn collaboration_session_handle_message(id: u64, message_json: String) -> String {
    with_collaboration_session(id, |session| {
        let message: Vec<u8> = match serde_json::from_str(&message_json) {
            Ok(bytes) => bytes,
            Err(error) => return format!("{{\"error\":\"invalid message json: {}\"}}", error),
        };
        match session.handle_message(message) {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()),
            Err(error) => format!("{{\"error\":\"{}\"}}", error),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"session not found\"}".to_string())
}

/// Update the local awareness payload for a collaboration session.
#[uniffi::export]
pub fn collaboration_session_set_local_awareness(id: u64, awareness_json: String) -> String {
    with_collaboration_session(id, |session| {
        let value: serde_json::Value = match serde_json::from_str(&awareness_json) {
            Ok(value) => value,
            Err(error) => return format!("{{\"error\":\"invalid awareness json: {}\"}}", error),
        };
        serde_json::to_string(&session.set_local_awareness(value))
            .unwrap_or_else(|_| "{}".to_string())
    })
    .unwrap_or_else(|| "{\"error\":\"session not found\"}".to_string())
}

/// Clear the local awareness payload for a collaboration session.
#[uniffi::export]
pub fn collaboration_session_clear_local_awareness(id: u64) -> String {
    with_collaboration_session(id, |session| {
        serde_json::to_string(&session.clear_local_awareness()).unwrap_or_else(|_| "{}".to_string())
    })
    .unwrap_or_else(|| "{\"error\":\"session not found\"}".to_string())
}

/// Set the editor's content from an HTML string. Returns render elements as JSON.
#[uniffi::export]
pub fn editor_set_html(id: u64, html: String) -> String {
    with_editor(id, |editor| match editor.set_html(&html) {
        Ok(elements) => serde_json::to_string(&serialize_render_elements(&elements))
            .unwrap_or_else(|_| "[]".to_string()),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Get the editor's content as HTML.
#[uniffi::export]
pub fn editor_get_html(id: u64) -> String {
    with_editor(id, |editor| editor.get_html()).unwrap_or_default()
}

/// Set the editor's content from a ProseMirror JSON string.
#[uniffi::export]
pub fn editor_set_json(id: u64, json: String) -> String {
    with_editor(id, |editor| {
        let value: serde_json::Value = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => return format!("{{\"error\":\"invalid json: {}\"}}", e),
        };
        match editor.set_json(&value) {
            Ok(elements) => serde_json::to_string(&serialize_render_elements(&elements))
                .unwrap_or_else(|_| "[]".to_string()),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Get the editor's content as ProseMirror JSON.
#[uniffi::export]
pub fn editor_get_json(id: u64) -> String {
    with_editor(id, |editor| {
        let json = editor.get_json();
        serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_string())
    })
    .unwrap_or_else(|| "{}".to_string())
}

/// Get both HTML and ProseMirror JSON content in one payload.
#[uniffi::export]
pub fn editor_get_content_snapshot(id: u64) -> String {
    with_editor(id, |editor| {
        let result = serde_json::json!({
            "html": editor.get_html(),
            "json": editor.get_json(),
        });
        serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string())
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Insert text at a position. Returns an update JSON string.
#[uniffi::export]
pub fn editor_insert_text(id: u64, pos: u32, text: String) -> String {
    with_editor(id, |editor| match editor.insert_text(pos, &text) {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Replace the current selection with plain text. Returns an update JSON string.
#[uniffi::export]
pub fn editor_replace_selection_text(id: u64, text: String) -> String {
    with_editor(id, |editor| match editor.replace_selection_text(&text) {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Delete a range. Returns an update JSON string.
#[uniffi::export]
pub fn editor_delete_range(id: u64, from: u32, to: u32) -> String {
    with_editor(id, |editor| match editor.delete_range(from, to) {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Toggle a mark on the current selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_toggle_mark(id: u64, mark_name: String) -> String {
    with_editor(id, |editor| match editor.toggle_mark(&mark_name) {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Toggle a mark at an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_toggle_mark_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
    mark_name: String,
) -> String {
    with_editor(id, |editor| {
        match editor.toggle_mark_at_selection_scalar(scalar_anchor, scalar_head, &mark_name) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Set a mark with attrs on the current selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_set_mark(id: u64, mark_name: String, attrs_json: String) -> String {
    with_editor(id, |editor| {
        let attrs = match parse_mark_attrs_json(&attrs_json) {
            Ok(attrs) => attrs,
            Err(error) => return format!("{{\"error\":\"{}\"}}", error),
        };
        match editor.set_mark(&mark_name, attrs) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Remove a mark from the current selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_unset_mark(id: u64, mark_name: String) -> String {
    with_editor(id, |editor| match editor.unset_mark(&mark_name) {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Set a mark with attrs at an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_set_mark_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
    mark_name: String,
    attrs_json: String,
) -> String {
    with_editor(id, |editor| {
        let attrs = match parse_mark_attrs_json(&attrs_json) {
            Ok(attrs) => attrs,
            Err(error) => return format!("{{\"error\":\"{}\"}}", error),
        };
        match editor.set_mark_at_selection_scalar(scalar_anchor, scalar_head, &mark_name, attrs) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Remove a mark at an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_unset_mark_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
    mark_name: String,
) -> String {
    with_editor(id, |editor| {
        match editor.unset_mark_at_selection_scalar(scalar_anchor, scalar_head, &mark_name) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Set the selection. Anchor and head are document positions.
#[uniffi::export]
pub fn editor_set_selection(id: u64, anchor: u32, head: u32) {
    with_editor(id, |editor| {
        let sel = if anchor == head {
            selection::Selection::cursor(anchor)
        } else {
            selection::Selection::text(anchor, head)
        };
        editor.set_selection(sel);
    });
}

/// Undo. Returns an update JSON string, or empty string if nothing to undo.
#[uniffi::export]
pub fn editor_undo(id: u64) -> String {
    with_editor(id, |editor| match editor.undo() {
        Some(update) => serialize_editor_update(&update),
        None => String::new(),
    })
    .unwrap_or_default()
}

/// Redo. Returns an update JSON string, or empty string if nothing to redo.
#[uniffi::export]
pub fn editor_redo(id: u64) -> String {
    with_editor(id, |editor| match editor.redo() {
        Some(update) => serialize_editor_update(&update),
        None => String::new(),
    })
    .unwrap_or_default()
}

/// Check if undo is available.
#[uniffi::export]
pub fn editor_can_undo(id: u64) -> bool {
    with_editor(id, |editor| editor.can_undo()).unwrap_or(false)
}

/// Check if redo is available.
#[uniffi::export]
pub fn editor_can_redo(id: u64) -> bool {
    with_editor(id, |editor| editor.can_redo()).unwrap_or(false)
}

/// Split the block at a position (Enter key). Returns an update JSON string.
#[uniffi::export]
pub fn editor_split_block(id: u64, pos: u32) -> String {
    with_editor(id, |editor| match editor.split_block(pos) {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Insert HTML content at the current selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_insert_content_html(id: u64, html: String) -> String {
    with_editor(id, |editor| match editor.insert_content_html(&html) {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Replace entire document content with HTML via a transaction (preserves history).
#[uniffi::export]
pub fn editor_replace_html(id: u64, html: String) -> String {
    with_editor(id, |editor| match editor.replace_html(&html) {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Insert JSON content at the current selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_insert_content_json(id: u64, json: String) -> String {
    with_editor(id, |editor| {
        let value: serde_json::Value = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => return format!("{{\"error\":\"invalid json: {}\"}}", e),
        };
        match editor.insert_content_json(&value) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Insert JSON content at an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_insert_content_json_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
    json: String,
) -> String {
    with_editor(id, |editor| {
        let value: serde_json::Value = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => return format!("{{\"error\":\"invalid json: {}\"}}", e),
        };
        match editor.insert_content_json_at_selection_scalar(scalar_anchor, scalar_head, &value) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Replace entire document content with JSON via a transaction (preserves history).
#[uniffi::export]
pub fn editor_replace_json(id: u64, json: String) -> String {
    with_editor(id, |editor| {
        let value: serde_json::Value = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => return format!("{{\"error\":\"invalid json: {}\"}}", e),
        };
        match editor.replace_json(&value) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Wrap the current selection in a list. Returns an update JSON string.
#[uniffi::export]
pub fn editor_wrap_in_list(id: u64, list_type: String) -> String {
    with_editor(id, |editor| match editor.apply_list_type(&list_type) {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Toggle a blockquote around the current block selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_toggle_blockquote(id: u64) -> String {
    with_editor(id, |editor| match editor.toggle_blockquote() {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Toggle a blockquote at an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_toggle_blockquote_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
) -> String {
    with_editor(id, |editor| {
        match editor.toggle_blockquote_at_selection_scalar(scalar_anchor, scalar_head) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Toggle a heading level on the current text-block selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_toggle_heading(id: u64, level: u8) -> String {
    with_editor(id, |editor| match editor.toggle_heading(level) {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Toggle a heading level at an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_toggle_heading_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
    level: u8,
) -> String {
    with_editor(id, |editor| {
        match editor.toggle_heading_at_selection_scalar(scalar_anchor, scalar_head, level) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Wrap or convert a list at an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_wrap_in_list_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
    list_type: String,
) -> String {
    with_editor(id, |editor| {
        match editor.apply_list_type_at_selection_scalar(scalar_anchor, scalar_head, &list_type) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Unwrap the current list item back to a paragraph. Returns an update JSON string.
#[uniffi::export]
pub fn editor_unwrap_from_list(id: u64) -> String {
    with_editor(id, |editor| {
        let doc = editor.document();
        let pos = editor.selection().from(doc);
        match editor.unwrap_from_list(pos) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Unwrap the list item at an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_unwrap_from_list_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
) -> String {
    with_editor(id, |editor| {
        match editor.unwrap_from_list_at_selection_scalar(scalar_anchor, scalar_head) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Indent the current list item into a nested list. Returns an update JSON string.
#[uniffi::export]
pub fn editor_indent_list_item(id: u64) -> String {
    with_editor(id, |editor| match editor.indent_list_item() {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Indent the list item at an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_indent_list_item_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
) -> String {
    with_editor(id, |editor| {
        match editor.indent_list_item_at_selection_scalar(scalar_anchor, scalar_head) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Outdent the current list item to the parent list level. Returns an update JSON string.
#[uniffi::export]
pub fn editor_outdent_list_item(id: u64) -> String {
    with_editor(id, |editor| match editor.outdent_list_item() {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Outdent the list item at an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_outdent_list_item_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
) -> String {
    with_editor(id, |editor| {
        match editor.outdent_list_item_at_selection_scalar(scalar_anchor, scalar_head) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Insert a void node at the current selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_insert_node(id: u64, node_type: String) -> String {
    with_editor(id, |editor| {
        match editor.insert_node_at_selection(&node_type) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Insert a node at an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_insert_node_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
    node_type: String,
) -> String {
    with_editor(id, |editor| {
        match editor.insert_node_at_selection_scalar(scalar_anchor, scalar_head, &node_type) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Resize an image node at a document position. Returns an update JSON string.
#[uniffi::export]
pub fn editor_resize_image_at_doc_pos(id: u64, doc_pos: u32, width: u32, height: u32) -> String {
    with_editor(id, |editor| {
        match editor.resize_image_at_doc_pos(doc_pos, width, height) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Get the current editor state (render elements, selection, active state,
/// history state) without performing any edits. Used by native views to pull
/// initial state when binding to an already-loaded editor.
#[uniffi::export]
pub fn editor_get_current_state(id: u64) -> String {
    with_editor(id, |editor| {
        let update = editor.get_current_state();
        serialize_editor_update(&update)
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Get the current selection-related editor state without render elements.
#[uniffi::export]
pub fn editor_get_selection_state(id: u64) -> String {
    with_editor(id, |editor| {
        let state = editor.get_selection_state();
        serialize_editor_selection_state(&state)
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Get the current selection as JSON.
#[uniffi::export]
pub fn editor_get_selection(id: u64) -> String {
    with_editor(id, |editor| {
        selection_to_json(editor.selection(), None).to_string()
    })
    .unwrap_or_else(|| "{\"type\":\"text\",\"anchor\":0,\"head\":0}".to_string())
}

// ---------------------------------------------------------------------------
// Scalar-position APIs (used by native views)
// ---------------------------------------------------------------------------
//
// Native text views work in scalar offsets (Unicode scalar position in the
// rendered text). These APIs convert to document positions internally.

/// Insert text at a scalar offset. Returns an update JSON string.
#[uniffi::export]
pub fn editor_insert_text_scalar(id: u64, scalar_pos: u32, text: String) -> String {
    with_editor(id, |editor| {
        match editor.insert_text_scalar(scalar_pos, &text) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Delete content between two scalar offsets. Returns an update JSON string.
#[uniffi::export]
pub fn editor_delete_scalar_range(id: u64, scalar_from: u32, scalar_to: u32) -> String {
    with_editor(id, |editor| {
        match editor.delete_scalar_range(scalar_from, scalar_to) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Delete backward relative to an explicit scalar selection. Returns an update JSON string.
#[uniffi::export]
pub fn editor_delete_backward_at_selection_scalar(
    id: u64,
    scalar_anchor: u32,
    scalar_head: u32,
) -> String {
    with_editor(id, |editor| {
        match editor.delete_backward_at_selection_scalar(scalar_anchor, scalar_head) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Replace a scalar range with text (atomic delete + insert). Returns an update JSON string.
#[uniffi::export]
pub fn editor_replace_text_scalar(
    id: u64,
    scalar_from: u32,
    scalar_to: u32,
    text: String,
) -> String {
    with_editor(id, |editor| {
        match editor.replace_text_scalar(scalar_from, scalar_to, &text) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Split a block at a scalar offset. Returns an update JSON string.
#[uniffi::export]
pub fn editor_split_block_scalar(id: u64, scalar_pos: u32) -> String {
    with_editor(id, |editor| match editor.split_block_scalar(scalar_pos) {
        Ok(update) => serialize_editor_update(&update),
        Err(e) => format!("{{\"error\":\"{}\"}}", e),
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Delete a scalar range then split the block (Enter with selection). Returns an update.
#[uniffi::export]
pub fn editor_delete_and_split_scalar(id: u64, scalar_from: u32, scalar_to: u32) -> String {
    with_editor(id, |editor| {
        match editor.delete_and_split_scalar(scalar_from, scalar_to) {
            Ok(update) => serialize_editor_update(&update),
            Err(e) => format!("{{\"error\":\"{}\"}}", e),
        }
    })
    .unwrap_or_else(|| "{\"error\":\"editor not found\"}".to_string())
}

/// Set the selection from scalar offsets, converting to document positions internally.
#[uniffi::export]
pub fn editor_set_selection_scalar(id: u64, scalar_anchor: u32, scalar_head: u32) {
    with_editor(id, |editor| {
        editor.set_selection_scalar(scalar_anchor, scalar_head);
    });
}

/// Convert a document position to a rendered-text scalar offset.
#[uniffi::export]
pub fn editor_doc_to_scalar(id: u64, doc_pos: u32) -> u32 {
    with_editor(id, |editor| editor.doc_to_scalar(doc_pos)).unwrap_or(doc_pos)
}

/// Convert a rendered-text scalar offset to a document position.
#[uniffi::export]
pub fn editor_scalar_to_doc(id: u64, scalar: u32) -> u32 {
    with_editor(id, |editor| editor.scalar_to_doc(scalar)).unwrap_or(scalar)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Execute a closure with a mutable reference to the editor identified by `id`.
fn with_editor<F, R>(id: u64, f: F) -> Option<R>
where
    F: FnOnce(&mut editor::Editor) -> R,
{
    let arc = registry::EditorRegistry::get(id)?;
    let mut editor = arc.lock().expect("editor lock poisoned");
    Some(f(&mut editor))
}

fn with_collaboration_session<F, R>(id: u64, f: F) -> Option<R>
where
    F: FnOnce(&mut collaboration::CollaborationSession) -> R,
{
    let arc = collaboration::CollaborationSessionRegistry::get(id)?;
    let mut session = arc.lock().expect("collaboration session lock poisoned");
    Some(f(&mut session))
}

/// Serialize render elements to a JSON-compatible structure.
fn serialize_render_elements(elements: &[render::RenderElement]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = elements
        .iter()
        .map(|el| match el {
            render::RenderElement::TextRun { text, marks } => {
                serde_json::json!({
                    "type": "textRun",
                    "text": text,
                    "marks": marks.iter().map(serialize_render_mark).collect::<Vec<_>>(),
                })
            }
            render::RenderElement::VoidInline {
                node_type,
                doc_pos,
                attrs,
            } => {
                let mut obj = serde_json::json!({
                    "type": "voidInline",
                    "nodeType": node_type,
                    "docPos": doc_pos,
                });
                if !attrs.is_empty() {
                    obj["attrs"] = serde_json::Value::Object(attrs.clone().into_iter().collect());
                }
                obj
            }
            render::RenderElement::VoidBlock {
                node_type,
                doc_pos,
                attrs,
            } => {
                let mut obj = serde_json::json!({
                    "type": "voidBlock",
                    "nodeType": node_type,
                    "docPos": doc_pos,
                });
                if !attrs.is_empty() {
                    obj["attrs"] = serde_json::Value::Object(attrs.clone().into_iter().collect());
                }
                obj
            }
            render::RenderElement::OpaqueInlineAtom {
                node_type,
                label,
                doc_pos,
            } => {
                serde_json::json!({
                    "type": "opaqueInlineAtom",
                    "nodeType": node_type,
                    "label": label,
                    "docPos": doc_pos,
                })
            }
            render::RenderElement::OpaqueBlockAtom {
                node_type,
                label,
                doc_pos,
            } => {
                serde_json::json!({
                    "type": "opaqueBlockAtom",
                    "nodeType": node_type,
                    "label": label,
                    "docPos": doc_pos,
                })
            }
            render::RenderElement::BlockStart {
                node_type,
                depth,
                list_context,
            } => {
                let mut obj = serde_json::json!({
                    "type": "blockStart",
                    "nodeType": node_type,
                    "depth": depth,
                });
                if let Some(ctx) = list_context {
                    obj["listContext"] = serde_json::json!({
                        "ordered": ctx.ordered,
                        "index": ctx.index,
                        "total": ctx.total,
                        "start": ctx.start,
                        "isFirst": ctx.is_first,
                        "isLast": ctx.is_last,
                    });
                }
                obj
            }
            render::RenderElement::BlockEnd => {
                serde_json::json!({"type": "blockEnd"})
            }
        })
        .collect();
    serde_json::Value::Array(items)
}

fn serialize_render_mark(mark: &render::RenderMark) -> serde_json::Value {
    if mark.attrs.is_empty() {
        serde_json::Value::String(mark.mark_type.clone())
    } else {
        let mut obj = serde_json::Map::new();
        obj.insert(
            "type".to_string(),
            serde_json::Value::String(mark.mark_type.clone()),
        );
        for (key, value) in &mark.attrs {
            obj.insert(key.clone(), value.clone());
        }
        serde_json::Value::Object(obj)
    }
}

fn serialize_render_blocks(blocks: &[Vec<render::RenderElement>]) -> serde_json::Value {
    serde_json::Value::Array(
        blocks
            .iter()
            .map(|block| serialize_render_elements(block))
            .collect(),
    )
}

fn serialize_render_patch(patch: &render::incremental::RenderBlocksPatch) -> serde_json::Value {
    serde_json::json!({
        "startIndex": patch.start_index,
        "deleteCount": patch.delete_count,
        "renderBlocks": serialize_render_blocks(&patch.blocks),
    })
}

fn parse_mark_attrs_json(
    attrs_json: &str,
) -> Result<std::collections::HashMap<String, serde_json::Value>, String> {
    if attrs_json.trim().is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let value: serde_json::Value = serde_json::from_str(attrs_json)
        .map_err(|error| format!("invalid mark attrs json: {}", error))?;
    match value {
        serde_json::Value::Object(map) => Ok(map.into_iter().collect()),
        _ => Err("invalid mark attrs json: expected object".to_string()),
    }
}

/// Serialize an EditorUpdate to a JSON string.
fn serialize_editor_update(update: &editor::EditorUpdate) -> String {
    let mut result = serde_json::Map::new();
    let should_serialize_full_render_blocks = update
        .render_patch
        .as_ref()
        .map(|patch| patch.blocks.len() >= update.render_blocks.len())
        .unwrap_or(true);

    if should_serialize_full_render_blocks {
        result.insert(
            "renderBlocks".to_string(),
            serialize_render_blocks(&update.render_blocks),
        );
    }
    result.insert(
        "renderPatch".to_string(),
        update
            .render_patch
            .as_ref()
            .map(serialize_render_patch)
            .unwrap_or(serde_json::Value::Null),
    );
    result.insert(
        "selection".to_string(),
        selection_to_json(&update.selection, Some(&update.selection_scalar)),
    );
    result.insert(
        "activeState".to_string(),
        serialize_active_state(&update.active_state),
    );
    result.insert(
        "historyState".to_string(),
        serialize_history_state(&update.history_state),
    );
    result.insert(
        "documentVersion".to_string(),
        serde_json::Value::from(update.document_version),
    );

    serde_json::to_string(&serde_json::Value::Object(result)).unwrap_or_else(|_| "{}".to_string())
}

fn serialize_editor_selection_state(state: &editor::EditorSelectionState) -> String {
    let result = serde_json::json!({
        "selection": selection_to_json(&state.selection, Some(&state.selection_scalar)),
        "activeState": serialize_active_state(&state.active_state),
        "historyState": serialize_history_state(&state.history_state),
        "documentVersion": state.document_version,
    });

    serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string())
}

fn selection_to_json(
    selection: &selection::Selection,
    scalar_selection: Option<&selection::Selection>,
) -> serde_json::Value {
    match (selection, scalar_selection) {
        (
            selection::Selection::Text { anchor, head },
            Some(selection::Selection::Text {
                anchor: anchor_scalar,
                head: head_scalar,
            }),
        ) => serde_json::json!({
            "type": "text",
            "anchor": anchor,
            "head": head,
            "anchorScalar": anchor_scalar,
            "headScalar": head_scalar,
        }),
        (selection::Selection::Text { anchor, head }, _) => {
            serde_json::json!({"type": "text", "anchor": anchor, "head": head})
        }
        (
            selection::Selection::Node { pos },
            Some(selection::Selection::Node { pos: pos_scalar }),
        ) => serde_json::json!({
            "type": "node",
            "pos": pos,
            "posScalar": pos_scalar,
        }),
        (selection::Selection::Node { pos }, _) => {
            serde_json::json!({"type": "node", "pos": pos})
        }
        (selection::Selection::All, _) => serde_json::json!({"type": "all"}),
    }
}

fn serialize_active_state(active_state: &editor::ActiveState) -> serde_json::Value {
    serde_json::json!({
        "marks": &active_state.marks,
        "markAttrs": &active_state.mark_attrs,
        "nodes": &active_state.nodes,
        "commands": &active_state.commands,
        "allowedMarks": &active_state.allowed_marks,
        "insertableNodes": &active_state.insertable_nodes,
    })
}

fn serialize_history_state(history_state: &editor::HistoryState) -> serde_json::Value {
    serde_json::json!({
        "canUndo": history_state.can_undo,
        "canRedo": history_state.can_redo,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_editor_core_version() {
        let version = editor_core_version();
        assert_eq!(
            version,
            env!("CARGO_PKG_VERSION"),
            "editor_core_version() should return the crate version from Cargo.toml"
        );
    }

    #[test]
    fn test_editor_core_version_is_valid_semver() {
        let version = editor_core_version();
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "Version '{}' should have exactly 3 semver components (major.minor.patch)",
            version
        );
        for (i, part) in parts.iter().enumerate() {
            let label = match i {
                0 => "major",
                1 => "minor",
                2 => "patch",
                _ => unreachable!(),
            };
            part.parse::<u32>().unwrap_or_else(|_| {
                panic!(
                    "Version component '{}' ({}) should be a valid u32",
                    part, label
                )
            });
        }
    }

    #[test]
    fn test_editor_get_selection_state_omits_render_elements() {
        let id = editor_create("{}".to_string());
        let json = editor_get_selection_state(id);
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid selection state");

        assert!(value.get("renderElements").is_none());
        assert!(value.get("selection").is_some());
        assert!(value.get("activeState").is_some());
        assert!(value.get("historyState").is_some());
        assert!(value.get("documentVersion").is_some());

        editor_destroy(id);
    }

    #[test]
    fn test_editor_get_current_state_serializes_render_blocks_without_flattened_elements() {
        let id = editor_create("{}".to_string());
        let json = editor_get_current_state(id);
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid editor state");

        assert!(value.get("renderElements").is_none());
        assert!(value
            .get("renderBlocks")
            .and_then(|raw| raw.as_array())
            .is_some());
        assert!(value.get("selection").is_some());
        assert!(value.get("activeState").is_some());
        assert!(value.get("historyState").is_some());
        assert!(value.get("documentVersion").is_some());

        editor_destroy(id);
    }

    #[test]
    fn test_incremental_editor_update_omits_full_render_blocks_when_patch_is_smaller() {
        let block = vec![
            render::RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            render::RenderElement::TextRun {
                text: "hello".to_string(),
                marks: vec![],
            },
            render::RenderElement::BlockEnd,
        ];
        let update = editor::EditorUpdate {
            render_elements: vec![],
            render_blocks: vec![
                block.clone(),
                block.clone(),
                block.clone(),
                block.clone(),
                block.clone(),
            ],
            render_patch: Some(render::incremental::RenderBlocksPatch {
                start_index: 1,
                delete_count: 2,
                blocks: vec![block.clone(), block.clone(), block],
            }),
            selection: selection::Selection::cursor(0),
            selection_scalar: selection::Selection::cursor(0),
            active_state: editor::ActiveState {
                marks: std::collections::HashMap::new(),
                mark_attrs: std::collections::HashMap::new(),
                nodes: std::collections::HashMap::new(),
                commands: std::collections::HashMap::new(),
                allowed_marks: vec![],
                insertable_nodes: vec![],
            },
            history_state: editor::HistoryState {
                can_undo: true,
                can_redo: false,
            },
            document_version: 7,
        };

        let json = serialize_editor_update(&update);
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid editor update");

        assert!(value.get("renderBlocks").is_none());
        assert!(value
            .get("renderPatch")
            .and_then(|raw| raw.as_object())
            .is_some());
        assert_eq!(
            value
                .get("selection")
                .and_then(|raw| raw.get("anchorScalar"))
                .and_then(|raw| raw.as_u64()),
            Some(0)
        );
        assert_eq!(
            value.get("documentVersion").and_then(|raw| raw.as_u64()),
            Some(7)
        );
    }

    #[test]
    fn test_editor_get_content_snapshot_includes_html_and_json() {
        let id = editor_create("{}".to_string());
        let json = editor_get_content_snapshot(id);
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid snapshot");

        assert!(value.get("html").and_then(|raw| raw.as_str()).is_some());
        assert!(value.get("json").and_then(|raw| raw.as_object()).is_some());

        editor_destroy(id);
    }
}
