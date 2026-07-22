//! Task 16B: v2 render/selection/position accessor (production since the
//! Task 16C cutover removed the `ffi-v2-staging` gate).
//!
//! The Task 12 v2 boundary returns structured mutation outcomes (revisions,
//! history) but no render payload, resolved selection, or position mapping,
//! so the Task 15 staging adapters derived those through a STATELESS legacy
//! render probe (create a throwaway legacy editor, feed it the authoritative
//! v2 document JSON, probe, destroy — on every derivation). This module is
//! the probe's replacement: every value the probe provided is derived here
//! directly from the live v2 session, reusing the exact legacy derivation
//! and serialization code paths so the wire output is probe-identical:
//!
//! - `editor_v2_render_update` — the full current-state update the adapters
//!   synthesize per refresh: `renderBlocks` (full blocks; `renderPatch` is
//!   null — the native bridges keep deriving incremental patches), toolbar
//!   `activeState`, the v2 engine's own `historyState`/`documentVersion`,
//!   the document's scalar extent (`scalarLength`, the lenient
//!   `u32::MAX` doc->scalar mapping), and — only when a scalar mirror
//!   selection is supplied — the resolved `selection` (doc anchor/head plus
//!   their scalar round-trip).
//! - `editor_v2_resolve_scalar_selection` — the engine-authoritative
//!   scalar->doc selection resolution the delegate callbacks consume.
//! - `editor_v2_doc_to_scalar` / `editor_v2_scalar_to_doc` — the lenient
//!   position mapping helpers (clamping at the document extent, exactly the
//!   legacy `PositionMap` semantics the probe exposed).
//!
//! Probe-parity semantics (pinned by the fixture-matrix parity tests): the
//! active state and the no-mirror behavior evaluate the same fresh-probe
//! selection (`cursor(1)`, no stored marks), because that is what the
//! probe's `setJson` produced; a mirror maps through the same lenient
//! scalar->doc mapping plus cursor normalization the probe's
//! `setSelectionScalar` applied. History and version are the v2 engine's own
//! facts (the adapters used to override the probe's fabricated values with
//! exactly these).
//!
//! The wire shape is JSON inside the frozen `FfiJsonResult` envelope (never
//! a new exception channel): the native views already consume the legacy
//! update JSON shape, so the accessor emits that shape verbatim and the
//! frozen Task 12 envelope invariants are untouched.
//!
//! The accessor needs the session schema (render blocks and active state
//! are schema-derived) but the engine keeps it private; the create path
//! therefore registers the already-resolved schema per session id here, and
//! destroy unregisters it.

#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed session error envelope"
)]

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::Value;

use crate::boundary::{BoundaryError, ResourceLimits};
use crate::model::Document;
use crate::position::PositionMap;
use crate::schema::{presets::tiptap_schema, Schema};
use crate::selection::Selection;
use crate::session::SessionError;
use crate::yrs_engine::YrsEngineError;

use super::editor::{json_result, with_editor};
use super::types::FfiJsonResult;

/// Schemas resolved at `editor_v2_create` time, keyed by session id. The
/// engine owns its schema privately; this registry is the render accessor's
/// schema source for sessions created through the v2 boundary (the only v2
/// creation path). Entries are removed on `editor_v2_destroy`.
static SESSION_SCHEMAS: LazyLock<Mutex<HashMap<u64, Schema>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Resolve the schema for a v2 create envelope exactly the way the session
/// construction does (default preset when the envelope carries no schema).
/// Resolved BEFORE the session is created so an invalid schema fails with
/// the identical structured error and no partial registration exists.
pub(crate) fn resolve_create_schema(schema: &Option<Value>) -> Result<Schema, SessionError> {
    match schema {
        None => Ok(tiptap_schema()),
        Some(value) => Schema::from_json_with_limits(value, &ResourceLimits::default())
            .map_err(SessionError::from),
    }
}

pub(crate) fn register_session_schema(id: u64, schema: Schema) {
    SESSION_SCHEMAS
        .lock()
        .expect("session schema registry poisoned")
        .insert(id, schema);
}

pub(crate) fn unregister_session_schema(id: u64) {
    SESSION_SCHEMAS
        .lock()
        .expect("session schema registry poisoned")
        .remove(&id);
}

fn engine_not_ready() -> SessionError {
    SessionError::from(YrsEngineError::new(
        "ENGINE_NOT_READY",
        "the document engine is not ready",
    ))
}

fn config_invalid(message: impl Into<String>) -> SessionError {
    SessionError::from(BoundaryError::new("CONFIG_INVALID", message))
}

/// The probe's selection mapping: lenient scalar->doc, collapsed selections
/// become cursors, then cursor normalization (legacy `setSelectionScalar`).
fn map_scalar_selection(
    document: &Document,
    position_map: &PositionMap,
    scalar_anchor: u32,
    scalar_head: u32,
) -> Selection {
    let doc_anchor = position_map.scalar_to_doc(scalar_anchor, document);
    let doc_head = position_map.scalar_to_doc(scalar_head, document);
    if doc_anchor == doc_head {
        Selection::cursor(doc_anchor)
    } else {
        Selection::text(doc_anchor, doc_head)
    }
    .normalized(document, position_map)
}

/// The legacy update-JSON selection shape: doc positions plus the scalar
/// round-trip (`anchorScalar`/`headScalar`).
fn selection_json(document: &Document, position_map: &PositionMap, selection: &Selection) -> Value {
    let scalar = match selection {
        Selection::Text { anchor, head } => Selection::text(
            position_map.doc_to_scalar(*anchor, document),
            position_map.doc_to_scalar(*head, document),
        ),
        Selection::Node { pos } => Selection::node(position_map.doc_to_scalar(*pos, document)),
        Selection::All => Selection::All,
    };
    selection_to_json(selection, Some(&scalar))
}

fn registered_schema(editor_id: &str) -> Result<Schema, SessionError> {
    let id = editor_id
        .parse::<u64>()
        .map_err(|_| config_invalid(format!("malformed editor handle: {editor_id:?}")))?;
    SESSION_SCHEMAS
        .lock()
        .expect("session schema registry poisoned")
        .get(&id)
        .cloned()
        .ok_or_else(|| {
            config_invalid("render accessor has no schema registration for this session")
        })
}

#[uniffi::export]
pub fn editor_v2_render_update(
    editor_id: String,
    mirror_scalar_anchor: Option<u32>,
    mirror_scalar_head: Option<u32>,
) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        let mirror = match (mirror_scalar_anchor, mirror_scalar_head) {
            (None, None) => None,
            (Some(anchor), Some(head)) => Some((anchor, head)),
            _ => {
                return Err(config_invalid(
                    "render update mirror requires both scalar anchor and head",
                ));
            }
        };
        let engine = &session.engine;
        let document = engine.document().ok_or_else(engine_not_ready)?;
        let position_map = engine.position_map().ok_or_else(engine_not_ready)?;
        let schema = registered_schema(&editor_id)?;

        let selection =
            mirror.map(|(anchor, head)| map_scalar_selection(document, position_map, anchor, head));
        // Probe parity: the fresh probe's selection after setJson was a raw
        // cursor(1) with no stored marks; the active state must evaluate the
        // same selection the probe evaluated.
        let active_selection = selection.clone().unwrap_or_else(|| Selection::cursor(1));
        let commands = crate::editor_state::command_applicability(
            document,
            &schema,
            &active_selection,
            engine.resource_limits(),
        );
        let active_state = crate::editor_state::active_state(
            document,
            &schema,
            &active_selection,
            None,
            commands,
            engine.resource_limits(),
        );
        let render_blocks = crate::render::incremental::render_blocks(document, &schema);
        let scalar_length = position_map.doc_to_scalar(u32::MAX, document);

        let mut update = serde_json::Map::new();
        update.insert(
            "renderBlocks".to_string(),
            serialize_render_blocks(&render_blocks),
        );
        // Full blocks every time; the native bridges keep deriving the
        // incremental patches (Task 15 patch-before-view-update semantics).
        update.insert("renderPatch".to_string(), Value::Null);
        if let Some(selection) = &selection {
            update.insert(
                "selection".to_string(),
                selection_json(document, position_map, selection),
            );
        }
        update.insert(
            "activeState".to_string(),
            serialize_active_state(&active_state),
        );
        // History and version are the v2 engine's own facts — the values the
        // adapters used to override onto the probe's payload.
        update.insert(
            "historyState".to_string(),
            serde_json::json!({
                "canUndo": engine.can_undo(),
                "canRedo": engine.can_redo(),
            }),
        );
        update.insert(
            "documentVersion".to_string(),
            Value::from(engine.revision()),
        );
        update.insert("scalarLength".to_string(), Value::from(scalar_length));
        Ok(Value::Object(update).to_string())
    }))
}

#[uniffi::export]
pub fn editor_v2_resolve_scalar_selection(
    editor_id: String,
    scalar_anchor: u32,
    scalar_head: u32,
) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        let engine = &session.engine;
        let document = engine.document().ok_or_else(engine_not_ready)?;
        let position_map = engine.position_map().ok_or_else(engine_not_ready)?;
        let selection = map_scalar_selection(document, position_map, scalar_anchor, scalar_head);
        Ok(selection_json(document, position_map, &selection).to_string())
    }))
}

#[uniffi::export]
pub fn editor_v2_doc_to_scalar(editor_id: String, doc_pos: u32) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        let engine = &session.engine;
        let document = engine.document().ok_or_else(engine_not_ready)?;
        let position_map = engine.position_map().ok_or_else(engine_not_ready)?;
        Ok(
            serde_json::json!({ "scalar": position_map.doc_to_scalar(doc_pos, document) })
                .to_string(),
        )
    }))
}

#[uniffi::export]
pub fn editor_v2_scalar_to_doc(editor_id: String, scalar: u32) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        let engine = &session.engine;
        let document = engine.document().ok_or_else(engine_not_ready)?;
        let position_map = engine.position_map().ok_or_else(engine_not_ready)?;
        Ok(serde_json::json!({ "doc": position_map.scalar_to_doc(scalar, document) }).to_string())
    }))
}

// ---------------------------------------------------------------------------
// Legacy update-JSON serializers (hoisted from the pre-cutover lib.rs during
// the Task 16C cutover; this module is their only retained consumer — the
// v2 render accessor emits the exact legacy update JSON shape by design).
// ---------------------------------------------------------------------------
fn serialize_render_elements(elements: &[crate::render::RenderElement]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = elements
        .iter()
        .map(|el| match el {
            crate::render::RenderElement::TextRun { text, marks } => {
                serde_json::json!({
                    "type": "textRun",
                    "text": text,
                    "marks": marks.iter().map(serialize_render_mark).collect::<Vec<_>>(),
                })
            }
            crate::render::RenderElement::VoidInline {
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
                    obj["attrs"] = serde_json::Value::Object(
                        attrs
                            .iter()
                            .map(|(key, value)| {
                                (
                                    key.clone(),
                                    crate::boundary::clone_json_value_stack_safe(value),
                                )
                            })
                            .collect(),
                    );
                }
                obj
            }
            crate::render::RenderElement::VoidBlock {
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
                    obj["attrs"] = serde_json::Value::Object(
                        attrs
                            .iter()
                            .map(|(key, value)| {
                                (
                                    key.clone(),
                                    crate::boundary::clone_json_value_stack_safe(value),
                                )
                            })
                            .collect(),
                    );
                }
                obj
            }
            crate::render::RenderElement::OpaqueInlineAtom {
                node_type,
                label,
                doc_pos,
                mention_theme,
            } => {
                let mut obj = serde_json::json!({
                    "type": "opaqueInlineAtom",
                    "nodeType": node_type,
                    "label": label,
                    "docPos": doc_pos,
                });
                if let Some(mention_theme) = mention_theme {
                    obj["mentionTheme"] = serde_json::Value::Object(
                        mention_theme
                            .iter()
                            .map(|(key, value)| {
                                (
                                    key.clone(),
                                    crate::boundary::clone_json_value_stack_safe(value),
                                )
                            })
                            .collect(),
                    );
                }
                obj
            }
            crate::render::RenderElement::OpaqueBlockAtom {
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
            crate::render::RenderElement::BlockStart {
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
                    if let Some(kind) = &ctx.kind {
                        obj["listContext"]["kind"] = serde_json::Value::String(kind.clone());
                    }
                    if let Some(checked) = ctx.checked {
                        obj["listContext"]["checked"] = serde_json::Value::Bool(checked);
                    }
                }
                obj
            }
            crate::render::RenderElement::BlockEnd => {
                serde_json::json!({"type": "blockEnd"})
            }
        })
        .collect();
    serde_json::Value::Array(items)
}

fn serialize_render_mark(mark: &crate::render::RenderMark) -> serde_json::Value {
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

fn serialize_render_blocks(blocks: &[Vec<crate::render::RenderElement>]) -> serde_json::Value {
    serde_json::Value::Array(
        blocks
            .iter()
            .map(|block| serialize_render_elements(block))
            .collect(),
    )
}

fn selection_to_json(
    selection: &crate::selection::Selection,
    scalar_selection: Option<&crate::selection::Selection>,
) -> serde_json::Value {
    match (selection, scalar_selection) {
        (
            crate::selection::Selection::Text { anchor, head },
            Some(crate::selection::Selection::Text {
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
        (crate::selection::Selection::Text { anchor, head }, _) => {
            serde_json::json!({"type": "text", "anchor": anchor, "head": head})
        }
        (
            crate::selection::Selection::Node { pos },
            Some(crate::selection::Selection::Node { pos: pos_scalar }),
        ) => serde_json::json!({
            "type": "node",
            "pos": pos,
            "posScalar": pos_scalar,
        }),
        (crate::selection::Selection::Node { pos }, _) => {
            serde_json::json!({"type": "node", "pos": pos})
        }
        (crate::selection::Selection::All, _) => serde_json::json!({"type": "all"}),
    }
}

fn serialize_active_state(active_state: &crate::editor_state::ActiveState) -> serde_json::Value {
    serde_json::json!({
        "marks": &active_state.marks,
        "markAttrs": &active_state.mark_attrs,
        "nodes": &active_state.nodes,
        "commands": &active_state.commands,
        "allowedMarks": &active_state.allowed_marks,
        "insertableNodes": &active_state.insertable_nodes,
    })
}
