fn empty_yrs_engine() -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".to_string(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .expect("Yrs benchmark engine should initialize")
}

fn yrs_engine_with_document(document: &str) -> YrsDocumentEngine {
    let mut engine = empty_yrs_engine();
    engine
        .import_json(document, TransactionOrigin::DocumentImport)
        .expect("Yrs benchmark fixture document should import");
    engine
}

fn yrs_editing_fixture(document: &str, anchor: u32, head: u32) -> YrsDocumentEngine {
    let mut engine = yrs_engine_with_document(document);
    engine
        .apply_typed_transaction(yrs_selection_transaction(
            1,
            engine.revision(),
            anchor,
            head,
        ))
        .expect("Yrs benchmark selection should apply");
    engine
}

fn yrs_selection_fixture(document: &str) -> (YrsDocumentEngine, u32) {
    let engine = yrs_engine_with_document(document);
    let position = engine
        .position_map()
        .expect("populated Yrs benchmark engine should have a position map")
        .total_scalars()
        / 2;
    (engine, position)
}

fn yrs_selection_transaction(
    request_id: u64,
    revision: u64,
    anchor: u32,
    head: u32,
) -> TypedTransaction {
    let point = |offset| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    TypedTransaction {
        request_id,
        base_document_revision: revision,
        origin: TransactionOrigin::LocalApi,
        operations: vec![],
        selection_intent: SelectionIntent::Set(SelectionInput::Text {
            anchor: point(anchor),
            head: point(head),
        }),
        history_policy: HistoryPolicy::Skip,
    }
}

fn build_pure_insert_document(doc: &Value, text: &str, count: usize) -> Value {
    let mut expected = doc.clone();
    let offset = first_article_paragraph_scalar_offset(&expected, EDITING_CURSOR_SCALAR);
    let target = first_article_paragraph_text_mut(&mut expected);
    let byte_offset = target
        .char_indices()
        .nth(offset)
        .map(|(index, _)| index)
        .unwrap_or(target.len());
    target.insert_str(byte_offset, &text.repeat(count));
    expected
}

fn build_pure_bold_document(doc: &Value, anchor: u32, head: u32) -> Value {
    let mut expected = doc.clone();
    let from = first_article_paragraph_scalar_offset(&expected, anchor);
    let to = first_article_paragraph_scalar_offset(&expected, head);
    assert!(from < to, "expected bold range must be non-empty");

    let inline = first_article_paragraph_inline_mut(&mut expected);
    let original = inline
        .first()
        .and_then(|node| node.get("text"))
        .and_then(Value::as_str)
        .expect("article benchmark first inline node must be text")
        .chars()
        .collect::<Vec<_>>();
    assert!(
        to <= original.len(),
        "expected bold range must stay in the first text run"
    );

    let mut replacement = Vec::with_capacity(3);
    let prefix = char_slice(&original, 0, from);
    if !prefix.is_empty() {
        replacement.push(text_node(prefix));
    }
    replacement.push(marked_text_node(
        char_slice(&original, from, to),
        json!({ "type": "bold" }),
    ));
    let suffix = char_slice(&original, to, original.len());
    if !suffix.is_empty() {
        replacement.push(text_node(suffix));
    }
    inline.splice(0..1, replacement);
    expected
}

fn build_pure_list_document(doc: &Value, position: u32) -> Value {
    let mut expected = doc.clone();
    let offset = first_article_paragraph_scalar_offset(&expected, position);
    assert!(
        offset <= first_article_paragraph_text(&expected).chars().count(),
        "expected list cursor must target the first article paragraph",
    );
    let content = expected
        .get_mut("content")
        .and_then(Value::as_array_mut)
        .expect("article benchmark document must have content");
    let paragraph = content.remove(1);
    content.insert(
        1,
        json!({
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [paragraph]
            }]
        }),
    );
    expected
}

fn first_article_paragraph_scalar_offset(doc: &Value, scalar: u32) -> usize {
    let heading_text = doc
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|heading| heading.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|text| text.get("text"))
        .and_then(Value::as_str)
        .expect("article benchmark heading must start with text");
    let paragraph_start = heading_text.chars().count().saturating_add(1);
    usize::try_from(scalar)
        .expect("editing scalar must fit usize")
        .checked_sub(paragraph_start)
        .expect("editing scalar must target the first article paragraph")
}

fn first_article_paragraph_text(doc: &Value) -> &str {
    doc.get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.get(1))
        .and_then(|paragraph| paragraph.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|text| text.get("text"))
        .and_then(Value::as_str)
        .expect("article benchmark first paragraph must start with text")
}

fn first_article_paragraph_inline_mut(doc: &mut Value) -> &mut Vec<Value> {
    doc.get_mut("content")
        .and_then(Value::as_array_mut)
        .and_then(|content| content.get_mut(1))
        .and_then(|paragraph| paragraph.get_mut("content"))
        .and_then(Value::as_array_mut)
        .expect("article benchmark first paragraph must have inline content")
}

fn first_article_paragraph_text_mut(doc: &mut Value) -> &mut String {
    first_article_paragraph_inline_mut(doc)
        .first_mut()
        .and_then(|text| text.get_mut("text"))
        .and_then(|value| match value {
            Value::String(text) => Some(text),
            _ => None,
        })
        .expect("article benchmark first paragraph must start with text")
}

fn build_yrs_expected_document(doc: &Value) -> Value {
    let encoded = serde_json::to_string(doc).expect("expected editing document should serialize");
    yrs_engine_with_document(&encoded)
        .document_json()
        .expect("expected Yrs editing document should import")
}

fn build_editing_case_expectation(before: &Value, after: Value) -> EditingCaseExpectation {
    EditingCaseExpectation {
        before: build_yrs_expected_document(before),
        after: build_yrs_expected_document(&after),
    }
}

/// v2-native expected fixture: the pre-cutover harness derived expected
/// selections, active state, and render blocks from a throwaway legacy
/// `Editor`. Those derivations are the same retained code paths the v2
/// render accessor uses today (serializer -> `PositionMap` ->
/// `editor_state` -> `render::incremental`), so the expectations are
/// computed from the expected document directly.
struct ExpectedEditingFixture {
    document: crate::model::Document,
    schema: crate::schema::Schema,
    position_map: crate::position::PositionMap,
    selection: crate::selection::Selection,
}

fn expected_editing_fixture(doc_json: &Value, anchor: u32, head: u32) -> ExpectedEditingFixture {
    let schema = tiptap_schema();
    let document = crate::serialize::from_prosemirror_json(
        doc_json,
        &schema,
        crate::serialize::UnknownTypeMode::Preserve,
    )
    .expect("expected editing document should ingest");
    let position_map = crate::position::PositionMap::build(&document, &schema);
    // The legacy harness's `set_selection_scalar`: lenient scalar->doc,
    // collapsed selections become cursors, then cursor normalization.
    let doc_anchor = position_map.scalar_to_doc(anchor, &document);
    let doc_head = position_map.scalar_to_doc(head, &document);
    let selection = if doc_anchor == doc_head {
        crate::selection::Selection::cursor(doc_anchor)
    } else {
        crate::selection::Selection::text(doc_anchor, doc_head)
    }
    .normalized(&document, &position_map);
    ExpectedEditingFixture {
        document,
        schema,
        position_map,
        selection,
    }
}

impl ExpectedEditingFixture {
    fn active_state(&self) -> crate::editor_state::ActiveState {
        let limits = ResourceLimits::default();
        let commands = crate::editor_state::command_applicability(
            &self.document,
            &self.schema,
            &self.selection,
            &limits,
        );
        crate::editor_state::active_state(
            &self.document,
            &self.schema,
            &self.selection,
            None,
            commands,
            &limits,
        )
    }

    fn render_blocks(&self) -> Vec<Vec<RenderElement>> {
        crate::render::incremental::render_blocks(&self.document, &self.schema)
    }
}

fn render_blocks_for(doc_json: &Value) -> Vec<Vec<RenderElement>> {
    let schema = tiptap_schema();
    let document = crate::serialize::from_prosemirror_json(
        doc_json,
        &schema,
        crate::serialize::UnknownTypeMode::Preserve,
    )
    .expect("expected render document should ingest");
    crate::render::incremental::render_blocks(&document, &schema)
}

#[allow(clippy::too_many_arguments)]
fn assert_yrs_editing_output(
    engine: &YrsDocumentEngine,
    expectation: &EditingCaseExpectation,
    output: &TypedTransactionResult,
    request_id: u64,
    origin: TransactionOrigin,
    changed: bool,
    document_revision: u64,
    state_revision: u64,
    anchor: u32,
    head: u32,
    can_undo: bool,
    can_redo: bool,
) {
    assert_eq!(engine.revision(), document_revision);
    assert_eq!(engine.state_revision(), state_revision);
    let selection = engine
        .resolved_selection()
        .expect("verified Yrs engine should have a resolved selection");
    let expected_fixture = expected_editing_fixture(&expectation.after, anchor, head);
    assert!(
        expectation.after.to_string().is_ascii(),
        "editing benchmark profile must remain ASCII for scalar/UTF-16 equality",
    );
    let expected_selection = ResolvedSelection::Text {
        anchor: crate::yrs_engine::ResolvedPoint {
            document: expected_fixture
                .position_map
                .scalar_to_doc(anchor, &expected_fixture.document),
            scalar: anchor,
            utf16: anchor,
        },
        head: crate::yrs_engine::ResolvedPoint {
            document: expected_fixture
                .position_map
                .scalar_to_doc(head, &expected_fixture.document),
            scalar: head,
            utf16: head,
        },
    };
    assert_eq!(selection, &expected_selection);
    match selection {
        ResolvedSelection::Text {
            anchor: resolved_anchor,
            head: resolved_head,
        } => {
            assert_eq!(
                resolved_anchor.document,
                expected_fixture
                    .position_map
                    .scalar_to_doc(anchor, &expected_fixture.document)
            );
            assert_eq!(resolved_anchor.scalar, anchor);
            assert_eq!(resolved_anchor.utf16, anchor);
            assert_eq!(
                resolved_head.document,
                expected_fixture
                    .position_map
                    .scalar_to_doc(head, &expected_fixture.document)
            );
            assert_eq!(resolved_head.scalar, head);
            assert_eq!(resolved_head.utf16, head);
        }
        other => panic!("expected verified text selection, got {other:?}"),
    }
    assert_eq!(engine.can_undo(), can_undo);
    assert_eq!(engine.can_redo(), can_redo);
    let actual_document = engine
        .document_json()
        .expect("verified Yrs engine should have canonical JSON");
    let expected_document = &expectation.after;
    assert_eq!(&actual_document, expected_document);

    assert_eq!(output.request_id, request_id);
    assert_eq!(output.origin, origin);
    assert_eq!(output.changed, changed);
    assert_eq!(output.document_revision, document_revision);
    assert_eq!(output.state_revision, state_revision);
    assert_eq!(output.selection, expected_selection);
    assert_eq!(output.active_state, expected_fixture.active_state());
    assert_eq!(output.history_state.can_undo, can_undo);
    assert_eq!(output.history_state.can_redo, can_redo);
    if expectation.before != expectation.after {
        assert_eq!(
            apply_render_update(&expectation.before, &output.render_update),
            expected_fixture.render_blocks(),
        );
    } else {
        assert_eq!(output.render_update, RenderUpdate::None);
    }
}

fn apply_render_patch(
    before_document: &Value,
    patch: &crate::render::incremental::RenderBlocksPatch,
) -> Vec<Vec<RenderElement>> {
    let mut blocks = render_blocks_for(before_document);
    blocks.splice(
        patch.start_index..patch.start_index + patch.delete_count,
        patch.blocks.clone(),
    );
    blocks
}

fn apply_render_update(before_document: &Value, update: &RenderUpdate) -> Vec<Vec<RenderElement>> {
    match update {
        RenderUpdate::None => render_blocks_for(before_document),
        RenderUpdate::Patch(patch) => apply_render_patch(before_document, patch),
        RenderUpdate::Full(blocks) => blocks.clone(),
    }
}

fn evenly_spaced_positions(max_value: u32, points: usize) -> Vec<u32> {
    if points <= 1 || max_value == 0 {
        return vec![0];
    }

    (0..points)
        .map(|index| ((max_value as u64 * index as u64) / (points - 1) as u64) as u32)
        .collect()
}

fn selection_scrub_positions(total_scalar: u32, points: usize) -> Vec<u32> {
    let upper_bound = total_scalar.saturating_sub(1).max(1);
    evenly_spaced_positions(upper_bound, points)
        .into_iter()
        .map(|position| position.max(1))
        .collect()
}

fn build_article_document(block_count: usize, paragraph_chars: usize) -> Value {
    let mut content = Vec::with_capacity(block_count + (block_count / 12) + 2);
    content.push(json!({
        "type": "h1",
        "content": [text_node(text_fragment(10_000, 42))]
    }));

    for index in 0..block_count {
        if index > 0 && index % 18 == 0 {
            content.push(json!({ "type": "horizontalRule" }));
        }

        if index % 12 == 5 {
            content.push(json!({
                "type": "blockquote",
                "content": [{
                    "type": "paragraph",
                    "content": rich_inline_content(index, paragraph_chars)
                }]
            }));
            continue;
        }

        if index % 9 == 3 {
            content.push(json!({
                "type": "h2",
                "content": [text_node(text_fragment(index + 2_000, paragraph_chars / 3 + 24))]
            }));
            continue;
        }

        content.push(json!({
            "type": "paragraph",
            "content": rich_inline_content(index, paragraph_chars)
        }));
    }

    json!({
        "type": "doc",
        "content": content
    })
}

fn build_opaque_document(payload_bytes: usize) -> Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "benchmarkOpaqueBlock",
            "attrs": {
                "payload": "x".repeat(payload_bytes),
            },
        }],
    })
}

fn build_edited_article_document(doc: &Value) -> Value {
    let mut next = doc.clone();
    let appended = append_to_last_text_node(&mut next, " sync-update");
    assert!(
        appended,
        "edited benchmark document should contain text nodes"
    );
    next
}

fn append_to_last_text_node(node: &mut Value, suffix: &str) -> bool {
    match node {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(Value::String(text)) = object.get_mut("text") {
                    text.push_str(suffix);
                    return true;
                }
            }

            if let Some(children) = object.get_mut("content").and_then(Value::as_array_mut) {
                for child in children.iter_mut().rev() {
                    if append_to_last_text_node(child, suffix) {
                        return true;
                    }
                }
            }
            false
        }
        Value::Array(array) => {
            for child in array.iter_mut().rev() {
                if append_to_last_text_node(child, suffix) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn rich_inline_content(seed: usize, total_chars: usize) -> Vec<Value> {
    let full_text = text_fragment(seed, total_chars.max(32));
    let chars: Vec<char> = full_text.chars().collect();
    let len = chars.len();
    let cut_a = len / 4;
    let cut_b = len / 2;
    let cut_c = (len * 3) / 4;

    let plain_lead = char_slice(&chars, 0, cut_a);
    let bold_text = char_slice(&chars, cut_a, cut_b);
    let italic_text = char_slice(&chars, cut_b, cut_c);
    let tail_text = char_slice(&chars, cut_c, len);

    let mut content = Vec::new();
    if !plain_lead.is_empty() {
        content.push(text_node(plain_lead));
    }
    if !bold_text.is_empty() {
        content.push(marked_text_node(bold_text, json!({ "type": "bold" })));
    }
    if !italic_text.is_empty() {
        content.push(marked_text_node(italic_text, json!({ "type": "italic" })));
    }
    if !tail_text.is_empty() {
        content.push(marked_text_node(
            tail_text,
            json!({
                "type": "link",
                "attrs": {
                    "href": format!("https://example.com/item/{seed}"),
                    "target": "_blank",
                    "rel": "noopener noreferrer nofollow",
                    "class": Value::Null,
                    "title": Value::Null
                }
            }),
        ));
    }

    content
}

fn text_node(text: String) -> Value {
    json!({
        "type": "text",
        "text": text
    })
}

fn marked_text_node(text: String, mark: Value) -> Value {
    json!({
        "type": "text",
        "text": text,
        "marks": [mark]
    })
}

fn text_fragment(seed: usize, min_chars: usize) -> String {
    const WORDS: &[&str] = &[
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
        "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra",
        "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu",
    ];

    let mut text = String::new();
    let mut cursor = 0usize;
    while text.chars().count() < min_chars {
        if !text.is_empty() {
            text.push(' ');
        }
        let word = WORDS[(seed + cursor) % WORDS.len()];
        text.push_str(word);
        cursor += 1;
    }
    text.chars().take(min_chars).collect()
}

fn char_slice(chars: &[char], start: usize, end: usize) -> String {
    let bounded_start = start.min(chars.len());
    let bounded_end = end.min(chars.len()).max(bounded_start);
    chars[bounded_start..bounded_end].iter().collect()
}
