use std::collections::HashMap;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::boundary::serialize_json_value_stack_safe;
use crate::render::{ListContext, RenderElement, RenderMark};
use crate::schema::schema_fingerprint;

use super::types::{
    FfiViewerCompileRequest, FfiViewerCompileResult, FfiViewerElement, FfiViewerMark,
    ViewerCompiledDocument,
};

const VIEWER_SEMANTIC_KEY_VERSION: u8 = 1;

pub(crate) fn compile(request: FfiViewerCompileRequest) -> FfiViewerCompileResult {
    let result = crate::boundary::with_document_stack(|| {
        let resolved = crate::ffi_v2::editor::resolve_local_document(
            &request.config_json,
            request.source_kind.clone(),
            &request.source,
        )?;
        let elements = crate::render::incremental::flatten_render_blocks(
            &crate::render::incremental::render_blocks(&resolved.document, &resolved.schema),
        )
        .into_iter()
        .filter(|element| request.images_enabled || !is_image_atom(element))
        .map(|element| viewer_element(element, request.mention_prefix.as_deref()))
        .collect::<Vec<_>>();

        let semantic_key = semantic_key(
            &schema_fingerprint(&resolved.schema),
            &elements,
            request.images_enabled,
            request.mention_prefix.as_deref(),
        );
        let retained_bytes = retained_bytes(&semantic_key, &elements);
        let is_empty = semantic_elements_are_empty(&elements, &resolved.schema);

        Ok::<_, crate::session::SessionError>(Arc::new(ViewerCompiledDocument {
            semantic_key,
            elements,
            is_empty,
            retained_bytes,
        }))
    });

    match result {
        Ok(document) => FfiViewerCompileResult::ok(document),
        Err(error) => FfiViewerCompileResult::err(error.into()),
    }
}

fn is_image_atom(element: &RenderElement) -> bool {
    let (node_type, attrs) = match element {
        RenderElement::VoidInline {
            node_type, attrs, ..
        }
        | RenderElement::VoidBlock {
            node_type, attrs, ..
        }
        | RenderElement::OpaqueInlineAtom {
            node_type, attrs, ..
        }
        | RenderElement::OpaqueBlockAtom {
            node_type, attrs, ..
        } => (node_type.as_str(), attrs),
        RenderElement::TextRun { .. }
        | RenderElement::BlockStart { .. }
        | RenderElement::BlockEnd => return false,
    };

    node_type == "image"
        || matches!(node_type, "__opaque_json" | "__opaque")
            && (attrs
                .get("original_type")
                .and_then(serde_json::Value::as_str)
                == Some("image")
                || attrs
                    .get("html_tag")
                    .and_then(serde_json::Value::as_str)
                    == Some("img"))
}

fn viewer_element(element: RenderElement, mention_prefix: Option<&str>) -> FfiViewerElement {
    match element {
        RenderElement::TextRun { text, marks } => FfiViewerElement::TextRun {
            text,
            marks: marks.into_iter().map(viewer_mark).collect(),
        },
        RenderElement::VoidInline {
            node_type,
            doc_pos,
            attrs,
        } => FfiViewerElement::InlineAtom {
            label: prefixed_mention_label(
                &node_type,
                crate::render::inline_atom_label(&node_type, &attrs),
                mention_prefix,
            ),
            node_type,
            doc_pos,
            attrs_json: canonical_attrs_json(&attrs),
        },
        RenderElement::VoidBlock {
            node_type,
            doc_pos,
            attrs,
        } => FfiViewerElement::BlockAtom {
            label: prefixed_mention_label(
                &node_type,
                crate::render::inline_atom_label(&node_type, &attrs),
                mention_prefix,
            ),
            node_type,
            doc_pos,
            attrs_json: canonical_attrs_json(&attrs),
        },
        RenderElement::OpaqueInlineAtom {
            node_type,
            doc_pos,
            label,
            attrs,
            mention_theme: _,
        } => FfiViewerElement::InlineAtom {
            label: prefixed_mention_label(&node_type, label, mention_prefix),
            node_type,
            doc_pos,
            attrs_json: canonical_attrs_json(&attrs),
        },
        RenderElement::OpaqueBlockAtom {
            node_type,
            doc_pos,
            label,
            attrs,
        } => FfiViewerElement::BlockAtom {
            label: prefixed_mention_label(&node_type, label, mention_prefix),
            node_type,
            doc_pos,
            attrs_json: canonical_attrs_json(&attrs),
        },
        RenderElement::BlockStart {
            node_type,
            depth,
            list_context,
        } => FfiViewerElement::BlockStart {
            node_type,
            depth,
            list_context_json: list_context.as_ref().map(canonical_list_context_json),
        },
        RenderElement::BlockEnd => FfiViewerElement::BlockEnd,
    }
}

fn semantic_elements_are_empty(
    elements: &[FfiViewerElement],
    schema: &crate::schema::Schema,
) -> bool {
    match elements {
        [] => true,
        [
            FfiViewerElement::BlockStart { node_type, .. },
            FfiViewerElement::BlockEnd,
        ] => schema
            .preferred_text_block()
            .is_some_and(|preferred| preferred.name == *node_type),
        _ => false,
    }
}

fn viewer_mark(mark: RenderMark) -> FfiViewerMark {
    FfiViewerMark {
        mark_type: mark.mark_type.clone(),
        attrs_json: canonical_attrs_json(&mark.attrs),
    }
}

fn prefixed_mention_label(node_type: &str, label: String, mention_prefix: Option<&str>) -> String {
    let Some(prefix) = mention_prefix.filter(|prefix| !prefix.is_empty()) else {
        return label;
    };
    if node_type == "mention" && !label.starts_with(prefix) {
        format!("{prefix}{label}")
    } else {
        label
    }
}

fn canonical_attrs_json(attrs: &HashMap<String, serde_json::Value>) -> String {
    let value = serde_json::Value::Object(
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
    String::from_utf8(serialize_json_value_stack_safe(&value, 0))
        .expect("canonical JSON serialization is UTF-8")
}

fn canonical_list_context_json(context: &ListContext) -> String {
    let value = serde_json::json!({
        "ordered": context.ordered,
        "index": context.index,
        "total": context.total,
        "start": context.start,
        "isFirst": context.is_first,
        "isLast": context.is_last,
        "kind": context.kind,
        "checked": context.checked,
    });
    String::from_utf8(serialize_json_value_stack_safe(&value, 0))
        .expect("canonical JSON serialization is UTF-8")
}

fn semantic_key(
    schema_fingerprint: &str,
    elements: &[FfiViewerElement],
    images_enabled: bool,
    mention_prefix: Option<&str>,
) -> String {
    let mut digest = Sha256::new();
    digest.update([VIEWER_SEMANTIC_KEY_VERSION]);
    hash_string(&mut digest, schema_fingerprint);
    digest.update([u8::from(images_enabled)]);
    hash_optional_string(&mut digest, mention_prefix);
    hash_u32(
        &mut digest,
        u32::try_from(elements.len()).unwrap_or(u32::MAX),
    );
    for element in elements {
        hash_element(&mut digest, element);
    }
    format!("{:x}", digest.finalize())
}

fn hash_element(digest: &mut Sha256, element: &FfiViewerElement) {
    match element {
        FfiViewerElement::TextRun { text, marks } => {
            digest.update([0]);
            hash_string(digest, text);
            hash_u32(digest, u32::try_from(marks.len()).unwrap_or(u32::MAX));
            for mark in marks {
                hash_string(digest, &mark.mark_type);
                hash_string(digest, &mark.attrs_json);
            }
        }
        FfiViewerElement::InlineAtom {
            node_type,
            doc_pos,
            attrs_json,
            label,
        } => {
            digest.update([1]);
            hash_string(digest, node_type);
            hash_u32(digest, *doc_pos);
            hash_string(digest, attrs_json);
            hash_string(digest, label);
        }
        FfiViewerElement::BlockAtom {
            node_type,
            doc_pos,
            attrs_json,
            label,
        } => {
            digest.update([2]);
            hash_string(digest, node_type);
            hash_u32(digest, *doc_pos);
            hash_string(digest, attrs_json);
            hash_string(digest, label);
        }
        FfiViewerElement::BlockStart {
            node_type,
            depth,
            list_context_json,
        } => {
            digest.update([3]);
            hash_string(digest, node_type);
            digest.update(depth.to_be_bytes());
            hash_optional_string(digest, list_context_json.as_deref());
        }
        FfiViewerElement::BlockEnd => digest.update([4]),
    }
}

fn hash_optional_string(digest: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            digest.update([1]);
            hash_string(digest, value);
        }
        None => digest.update([0]),
    }
}

fn hash_string(digest: &mut Sha256, value: &str) {
    hash_u64(digest, u64::try_from(value.len()).unwrap_or(u64::MAX));
    digest.update(value.as_bytes());
}

fn hash_u32(digest: &mut Sha256, value: u32) {
    digest.update(value.to_be_bytes());
}

fn hash_u64(digest: &mut Sha256, value: u64) {
    digest.update(value.to_be_bytes());
}

fn retained_bytes(semantic_key: &str, elements: &[FfiViewerElement]) -> usize {
    let element_bytes = elements.iter().map(element_retained_bytes).sum::<usize>();
    std::mem::size_of::<ViewerCompiledDocument>()
        .saturating_add(semantic_key.len())
        .saturating_add(
            elements
                .len()
                .saturating_mul(std::mem::size_of::<FfiViewerElement>()),
        )
        .saturating_add(element_bytes)
}

fn element_retained_bytes(element: &FfiViewerElement) -> usize {
    match element {
        FfiViewerElement::TextRun { text, marks } => text
            .capacity()
            .saturating_add(
                marks
                    .capacity()
                    .saturating_mul(std::mem::size_of::<FfiViewerMark>()),
            )
            .saturating_add(
                marks
                    .iter()
                    .map(|mark| {
                        mark.mark_type
                            .capacity()
                            .saturating_add(mark.attrs_json.capacity())
                    })
                    .sum(),
            ),
        FfiViewerElement::InlineAtom {
            node_type,
            attrs_json,
            label,
            ..
        }
        | FfiViewerElement::BlockAtom {
            node_type,
            attrs_json,
            label,
            ..
        } => node_type
            .capacity()
            .saturating_add(attrs_json.capacity())
            .saturating_add(label.capacity()),
        FfiViewerElement::BlockStart {
            node_type,
            list_context_json,
            ..
        } => node_type
            .capacity()
            .saturating_add(list_context_json.as_ref().map_or(0, String::capacity)),
        FfiViewerElement::BlockEnd => 0,
    }
}
