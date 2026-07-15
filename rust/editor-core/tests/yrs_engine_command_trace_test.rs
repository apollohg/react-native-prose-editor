use std::collections::HashMap;

use editor_core::boundary::ResourceLimits;
use editor_core::editor::{Editor, EditorUpdate};
use editor_core::editor_state::ActiveState;
use editor_core::intercept::InterceptorPipeline;
use editor_core::model::{Document, Node};
use editor_core::render::incremental::render_blocks;
use editor_core::render::{RenderElement, RenderMark};
use editor_core::schema::content_rule::ContentRule;
use editor_core::schema::{AttrSpec, NodeRole, NodeSpec, Schema};
use editor_core::selection::Selection;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    doc_pos_to_relative_point, scalar_offset_to_utf16, Affinity, EditingLimits, EditorOffsetKind,
    HistoryPolicy, InitializationMode, RenderUpdate, ResolvedPoint, ResolvedSelection,
    RevisionedPosition, RevisionedRange, SelectionInput, SelectionIntent, TransactionOrigin,
    TypedCommand, TypedTransaction, TypedTransactionResult, YrsDocumentEngine, YrsEngineConfig,
};
use yrs::types::text::Text;
use yrs::types::xml::{XmlElementPrelim, XmlFragment, XmlTextPrelim};
use yrs::updates::decoder::Decode;
use yrs::{Any, Doc, OffsetKind, Options, ReadTxn, StateVector, Transact, Update, WriteTxn};

const SEEDS: usize = 256;
const ACTIONS_PER_SEED: usize = 100;
const COMMAND_CLASSES: usize = 22;

fn engine(mode: InitializationMode) -> YrsDocumentEngine {
    engine_with_schema(mode, tiptap_schema())
}

fn engine_with_schema(mode: InitializationMode, schema: Schema) -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema,
        fragment_name: "prosemirror".into(),
        initialization_mode: mode,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap()
}

fn trace_schema() -> Schema {
    let base = tiptap_schema();
    let mut nodes = base.all_nodes().cloned().collect::<Vec<_>>();
    nodes.extend([
        NodeSpec {
            name: "codeBlock".into(),
            content: ContentRule::parse("text*").unwrap(),
            group: Some("block".into()),
            attrs: HashMap::new(),
            role: NodeRole::TextBlock,
            html_tag: Some("pre".into()),
            is_void: false,
            allow_undeclared_attrs: false,
        },
        NodeSpec {
            name: "taskList".into(),
            content: ContentRule::parse("taskItem+").unwrap(),
            group: Some("block".into()),
            attrs: HashMap::new(),
            role: NodeRole::List { ordered: false },
            html_tag: Some("ul".into()),
            is_void: false,
            allow_undeclared_attrs: false,
        },
        NodeSpec {
            name: "taskItem".into(),
            content: ContentRule::parse("paragraph block*").unwrap(),
            group: None,
            attrs: HashMap::from([(
                "checked".into(),
                AttrSpec {
                    default: Some(serde_json::Value::Bool(false)),
                    has_default: true,
                },
            )]),
            role: NodeRole::ListItem,
            html_tag: Some("li".into()),
            is_void: false,
            allow_undeclared_attrs: false,
        },
    ]);
    Schema::new(nodes, base.all_marks().cloned().collect())
}

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    }
}

fn mixed_document() -> serde_json::Value {
    serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[
            {"type":"text","text":"Grüße🙂漢","marks":[{"type":"bold"}]},
            {"type":"text","text":" unicode oracle"}
        ]},
        {"type":"paragraph"},
        {"type":"paragraph","content":[{"type":"text","text":"INSERT_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"BACKSPACE_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"REPLACE_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"DELETE_RANGE_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"SPLIT_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"DELETE_SPLIT_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"TOGGLE_MARK_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"SET_MARK_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"UNSET_MARK_TARGET","marks":[{"type":"italic"}]}]},
        {"type":"paragraph","content":[{"type":"text","text":"HEADING_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"CODE_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"QUOTE_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"WRAP_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"NODE_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"JSON_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"HTML_TARGET"}]},
        {"type":"paragraph","content":[{"type":"text","text":"STORED_MARK_TARGET"}]},
        {"type":"bulletList","content":[
            {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"CONVERT_FIRST"}]}]},
            {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"CONVERT_SECOND"}]}]}
        ]},
        {"type":"orderedList","attrs":{"start":3},"content":[
            {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"UNWRAP_TARGET"}]}]}
        ]},
        {"type":"bulletList","content":[
            {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"INDENT_FIRST"}]}]},
            {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"INDENT_SECOND"}]}]}
        ]},
        {"type":"bulletList","content":[
            {"type":"listItem","content":[
                {"type":"paragraph","content":[{"type":"text","text":"OUTDENT_OUTER"}]},
                {"type":"bulletList","content":[
                    {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"OUTDENT_NESTED"}]}]}
                ]}
            ]}
        ]},
        {"type":"taskList","content":[
            {"type":"taskItem","attrs":{"checked":false},"content":[{"type":"paragraph","content":[{"type":"text","text":"TASK_TARGET"}]}]},
            {"type":"taskItem","attrs":{"checked":true},"content":[{"type":"paragraph","content":[{"type":"text","text":"TASK_CHECKED"}]}]}
        ]},
        {"type":"image","attrs":{"src":"https://example.test/trace.png","alt":"trace","title":null,"width":80,"height":60}},
        {"type":"paragraph","content":[{"type":"text","text":"RANDOM_SAFE_TARGET"}]}
    ]})
}

fn rendered_text(document: &Document, schema: &Schema) -> String {
    let mut text = String::new();
    let mut pending_prefix = String::new();
    let mut started_block = false;
    for element in
        editor_core::render::incremental::flatten_render_blocks(&render_blocks(document, schema))
    {
        match element {
            RenderElement::BlockStart {
                node_type,
                list_context,
                ..
            } => {
                if let Some(context) = list_context {
                    pending_prefix = if context.kind.as_deref() == Some("task") {
                        editor_core::render::task_list_marker_string(
                            context.checked.unwrap_or(false),
                        )
                    } else {
                        editor_core::render::list_marker_string(context.ordered, context.index)
                    };
                }
                if schema
                    .node(&node_type)
                    .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
                {
                    if started_block {
                        text.push('\n');
                    }
                    started_block = true;
                    text.push_str(&pending_prefix);
                    pending_prefix.clear();
                }
            }
            RenderElement::TextRun { text: value, .. } => text.push_str(&value),
            RenderElement::VoidInline { .. } => text.push('\n'),
            RenderElement::VoidBlock { .. } => {
                if started_block {
                    text.push('\n');
                }
                started_block = true;
                text.push('\u{fffc}');
            }
            RenderElement::OpaqueInlineAtom {
                node_type, label, ..
            } => text.push_str(&editor_core::render::opaque_atom_visible_string(
                &node_type, &label,
            )),
            RenderElement::OpaqueBlockAtom {
                node_type, label, ..
            } => {
                if started_block {
                    text.push('\n');
                }
                started_block = true;
                text.push_str(&editor_core::render::opaque_atom_visible_string(
                    &node_type, &label,
                ));
            }
            RenderElement::BlockEnd => {}
        }
    }
    text
}

#[derive(Debug, Clone, Copy)]
struct ScalarTarget {
    start: u32,
    len: u32,
}

fn node_start_at_path(document: &Document, path: &[u32]) -> u32 {
    let mut node = document.root();
    let mut node_start = 0u32;
    for (depth, &index) in path.iter().enumerate() {
        let content = node
            .content()
            .unwrap_or_else(|| panic!("node path {path:?} entered a leaf"));
        let mut child_start = if depth == 0 { 0 } else { node_start + 1 };
        for sibling in content.iter().take(index as usize) {
            child_start += sibling.node_size();
        }
        node = content
            .child(index as usize)
            .unwrap_or_else(|| panic!("node path {path:?} is out of bounds"));
        node_start = child_start;
    }
    node_start
}

fn target_by_text(editor: &Editor, needle: &str) -> ScalarTarget {
    for index in 0..editor.position_map().block_count() {
        let block = editor.position_map().block(index).unwrap();
        let Some(node) = editor.document().node_at(&block.node_path) else {
            continue;
        };
        let value = node.text_content();
        let Some(byte_offset) = value.find(needle) else {
            continue;
        };
        let char_offset = value[..byte_offset].chars().count() as u32;
        let needle_len = needle.chars().count() as u32;
        let content_start = node_start_at_path(editor.document(), &block.node_path) + 1;
        return ScalarTarget {
            start: editor.doc_to_scalar(content_start) + char_offset,
            len: needle_len,
        };
    }
    panic!("live PositionMap target {needle:?} not found");
}

fn target_by_yrs_text(engine: &YrsDocumentEngine, needle: &str) -> ScalarTarget {
    let position_map = engine.position_map().unwrap();
    let document = engine.document().unwrap();
    for index in 0..position_map.block_count() {
        let block = position_map.block(index).unwrap();
        let Some(node) = document.node_at(&block.node_path) else {
            continue;
        };
        let value = node.text_content();
        let Some(byte_offset) = value.find(needle) else {
            continue;
        };
        let content_start = node_start_at_path(document, &block.node_path) + 1;
        return ScalarTarget {
            start: position_map.doc_to_scalar(content_start, document)
                + value[..byte_offset].chars().count() as u32,
            len: needle.chars().count() as u32,
        };
    }
    panic!("live Yrs PositionMap target {needle:?} not found");
}

fn image_target(editor: &Editor) -> ScalarTarget {
    for index in 0..editor.position_map().block_count() {
        let block = editor.position_map().block(index).unwrap();
        let Some(node) = editor.document().node_at(&block.node_path) else {
            continue;
        };
        if block.is_void_block && node.node_type() == "image" {
            return ScalarTarget {
                start: editor
                    .doc_to_scalar(node_start_at_path(editor.document(), &block.node_path)),
                len: 1,
            };
        }
    }
    panic!("live PositionMap image target not found");
}

fn is_direct_text_block(node: &Node, schema: &Schema) -> bool {
    schema
        .node(node.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        && node.child_count() > 0
        && (0..node.child_count()).all(|index| node.child(index).is_some_and(Node::is_text))
}

fn random_text_targets(editor: &Editor) -> Vec<ScalarTarget> {
    let schema = trace_schema();
    (0..editor.position_map().block_count())
        .filter_map(|index| {
            let block = editor.position_map().block(index)?;
            let node = editor.document().node_at(&block.node_path)?;
            if !is_direct_text_block(node, &schema) || block.scalar_len == 0 {
                return None;
            }
            let content_start = node_start_at_path(editor.document(), &block.node_path) + 1;
            Some(ScalarTarget {
                start: editor.doc_to_scalar(content_start),
                len: block.scalar_len,
            })
        })
        .collect()
}

fn assert_point(
    context: &str,
    side: &str,
    legacy_document: u32,
    yrs: &ResolvedPoint,
    legacy: &Editor,
) {
    let scalar = legacy.doc_to_scalar(legacy_document);
    let visible = rendered_text(legacy.document(), &trace_schema());
    assert_eq!(yrs.document, legacy_document, "{context}: {side} document");
    assert_eq!(yrs.scalar, scalar, "{context}: {side} scalar");
    assert_eq!(
        yrs.utf16,
        scalar_offset_to_utf16(&visible, scalar).unwrap(),
        "{context}: {side} utf16"
    );
}

fn assert_selection(context: &str, legacy: &Editor, yrs: &ResolvedSelection) {
    match (legacy.selection(), yrs) {
        (
            Selection::Text { anchor, head },
            ResolvedSelection::Text {
                anchor: yrs_anchor,
                head: yrs_head,
            },
        ) => {
            assert_point(context, "anchor", *anchor, yrs_anchor, legacy);
            assert_point(context, "head", *head, yrs_head, legacy);
        }
        (Selection::Node { pos }, ResolvedSelection::Node { at }) => {
            assert_point(context, "node", *pos, at, legacy);
        }
        (Selection::All, ResolvedSelection::All) => {}
        pair => panic!("{context}: selection variant/direction mismatch: {pair:?}"),
    }
}

fn assert_exact_text_selection(
    selection: &ResolvedSelection,
    document: u32,
    scalar: u32,
    utf16: u32,
) {
    assert_eq!(
        selection,
        &ResolvedSelection::Text {
            anchor: ResolvedPoint {
                document,
                scalar,
                utf16,
            },
            head: ResolvedPoint {
                document,
                scalar,
                utf16,
            },
        }
    );
}

fn apply_patch(
    context: &str,
    baseline: &mut Vec<Vec<RenderElement>>,
    patch: &editor_core::render::incremental::RenderBlocksPatch,
) {
    assert!(
        patch.start_index <= baseline.len()
            && patch.start_index + patch.delete_count <= baseline.len(),
        "{context}: patch window outside baseline"
    );
    baseline.splice(
        patch.start_index..patch.start_index + patch.delete_count,
        patch.blocks.clone(),
    );
}

struct TraceHarness {
    schema: Schema,
    legacy: Editor,
    yrs: YrsDocumentEngine,
    request: u64,
    legacy_render: Vec<Vec<RenderElement>>,
    yrs_render: Vec<Vec<RenderElement>>,
    yrs_active: ActiveState,
}

impl TraceHarness {
    fn new(seed: usize) -> Self {
        let schema = trace_schema();
        let document = mixed_document();
        let mut legacy = Editor::new(schema.clone(), InterceptorPipeline::new(), false);
        legacy.set_json(&document).unwrap();
        let mut yrs = engine_with_schema(InitializationMode::LocalEmpty, schema.clone());
        yrs.import_json(&document.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        let target = target_by_text(&legacy, "Grüße");
        legacy.set_selection_scalar(target.start, target.start);
        let initial = yrs
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id: 1,
                base_document_revision: yrs.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point(target.start),
                    head: point(target.start),
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        let legacy_render = render_blocks(legacy.document(), &schema);
        let yrs_render = render_blocks(yrs.document().unwrap(), &schema);
        let harness = Self {
            schema,
            legacy,
            yrs,
            request: 2,
            legacy_render,
            yrs_render,
            yrs_active: initial.active_state,
        };
        harness.assert_state(&format!("seed={seed} initial"));
        harness
    }

    fn next_request(&mut self) -> u64 {
        let request = self.request;
        self.request += 1;
        request
    }

    fn select(&mut self, context: &str, anchor: u32, head: u32) {
        let before_json = self.legacy.get_json();
        let before_legacy_render = self.legacy_render.clone();
        let before_yrs_render = self.yrs_render.clone();
        self.legacy.set_selection_scalar(anchor, head);
        let request_id = self.next_request();
        let result = self
            .yrs
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id,
                base_document_revision: self.yrs.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point(anchor),
                    head: point(head),
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap_or_else(|error| panic!("{context}: selection failed: {error:?}"));
        assert!(
            matches!(result.render_update, RenderUpdate::None),
            "{context}"
        );
        assert_eq!(
            before_json,
            self.legacy.get_json(),
            "{context}: selection wrote document"
        );
        assert_eq!(before_legacy_render, self.legacy_render, "{context}");
        assert_eq!(before_yrs_render, self.yrs_render, "{context}");
        self.yrs_active = result.active_state;
    }

    fn boundary(&mut self, context: &str) {
        let request_id = self.next_request();
        let result = self
            .yrs
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id,
                base_document_revision: self.yrs.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap_or_else(|error| panic!("{context}: boundary failed: {error:?}"));
        assert!(
            matches!(result.render_update, RenderUpdate::None),
            "{context}"
        );
        self.yrs_active = result.active_state;
    }

    fn command(&mut self, context: &str, command: TypedCommand) {
        let before_json = self.legacy.get_json();
        let legacy_update = apply_legacy(&mut self.legacy, &command)
            .unwrap_or_else(|error| panic!("{context}: legacy command failed: {error:?}"));
        let request_id = self.next_request();
        let yrs_result = self
            .yrs
            .apply_command(request_id, command)
            .unwrap_or_else(|error| panic!("{context}: Yrs command failed: {error:?}"))
            .unwrap_or_else(|| panic!("{context}: command unexpectedly not applicable"));
        assert!(
            yrs_result.changed,
            "{context}: fixed/contextual command was a no-op"
        );
        let document_changed = before_json != self.legacy.get_json();
        self.consume_updates(
            context,
            Some(&legacy_update),
            Some(&yrs_result),
            document_changed,
        );
        self.yrs_active = yrs_result.active_state;
    }

    fn undo(&mut self, context: &str) {
        let before_json = self.legacy.get_json();
        let legacy = self.legacy.undo();
        let request_id = self.next_request();
        let yrs = self
            .yrs
            .undo_with_result(request_id)
            .unwrap_or_else(|error| panic!("{context}: Yrs undo failed: {error:?}"));
        assert_eq!(legacy.is_some(), yrs.is_some(), "{context}: undo presence");
        let document_changed = before_json != self.legacy.get_json();
        self.consume_updates(context, legacy.as_ref(), yrs.as_ref(), document_changed);
        if let Some(result) = yrs {
            self.yrs_active = result.active_state;
        }
    }

    fn redo(&mut self, context: &str) {
        let before_json = self.legacy.get_json();
        let legacy = self.legacy.redo();
        let request_id = self.next_request();
        let yrs = self
            .yrs
            .redo_with_result(request_id)
            .unwrap_or_else(|error| panic!("{context}: Yrs redo failed: {error:?}"));
        assert_eq!(legacy.is_some(), yrs.is_some(), "{context}: redo presence");
        let document_changed = before_json != self.legacy.get_json();
        self.consume_updates(context, legacy.as_ref(), yrs.as_ref(), document_changed);
        if let Some(result) = yrs {
            self.yrs_active = result.active_state;
        }
    }

    fn consume_updates(
        &mut self,
        context: &str,
        legacy: Option<&EditorUpdate>,
        yrs: Option<&TypedTransactionResult>,
        document_changed: bool,
    ) {
        assert_eq!(
            legacy.is_some(),
            yrs.is_some(),
            "{context}: result presence"
        );
        let Some((legacy, yrs)) = legacy.zip(yrs) else {
            assert!(
                !document_changed,
                "{context}: missing result after document change"
            );
            return;
        };

        match &legacy.render_patch {
            Some(patch) => apply_patch(context, &mut self.legacy_render, patch),
            None => assert!(
                !document_changed,
                "{context}: legacy omitted patch for document mutation"
            ),
        }
        assert_eq!(
            self.legacy_render, legacy.render_blocks,
            "{context}: legacy patch reconstruction"
        );

        match &yrs.render_update {
            RenderUpdate::Patch(patch) => apply_patch(context, &mut self.yrs_render, patch),
            RenderUpdate::None => assert!(
                !document_changed,
                "{context}: Yrs omitted patch for document mutation"
            ),
            RenderUpdate::Full(_) => panic!("{context}: localized action degraded to full render"),
        }
        let actual = render_blocks(self.yrs.document().unwrap(), &self.schema);
        assert_eq!(
            self.yrs_render, actual,
            "{context}: Yrs patch reconstruction"
        );
    }

    fn assert_state(&self, context: &str) {
        assert_eq!(
            self.yrs.document_json().unwrap(),
            self.legacy.get_json(),
            "{context}: canonical JSON"
        );
        assert_eq!(
            self.yrs.document_html().unwrap(),
            self.legacy.get_html(),
            "{context}: canonical HTML"
        );
        assert_selection(
            context,
            &self.legacy,
            self.yrs.resolved_selection().unwrap(),
        );
        assert_eq!(
            self.yrs_active,
            self.legacy.get_selection_state().active_state,
            "{context}: complete ActiveState"
        );
        assert_eq!(
            self.yrs.can_undo(),
            self.legacy.can_undo(),
            "{context}: canUndo"
        );
        assert_eq!(
            self.yrs.can_redo(),
            self.legacy.can_redo(),
            "{context}: canRedo"
        );
        assert_eq!(
            self.legacy_render,
            render_blocks(self.legacy.document(), &self.schema),
            "{context}: legacy render baseline"
        );
        assert_eq!(
            self.yrs_render,
            render_blocks(self.yrs.document().unwrap(), &self.schema),
            "{context}: Yrs render baseline"
        );
    }
}

fn apply_legacy(
    editor: &mut Editor,
    command: &TypedCommand,
) -> Result<EditorUpdate, editor_core::editor::EditorError> {
    let selection = editor.selection().clone();
    match command {
        TypedCommand::InsertText { text } => {
            let at = editor.doc_to_scalar(selection.head(editor.document()));
            editor.insert_text_scalar(at, text)
        }
        TypedCommand::DeleteRange { range } => editor.delete_scalar_range(
            range.from.offset.min(range.to.offset),
            range.from.offset.max(range.to.offset),
        ),
        TypedCommand::DeleteBackward => {
            let Selection::Text { anchor, head } = selection else {
                unreachable!("trace only issues DeleteBackward from a text selection")
            };
            editor.delete_backward_at_selection_scalar(
                editor.doc_to_scalar(anchor),
                editor.doc_to_scalar(head),
            )
        }
        TypedCommand::ReplaceSelectionText { text } => editor.replace_selection_text(text),
        TypedCommand::SplitBlock => {
            let head = selection.head(editor.document());
            editor.split_block(head)
        }
        TypedCommand::DeleteAndSplit => {
            let from = editor.doc_to_scalar(selection.from(editor.document()));
            let to = editor.doc_to_scalar(selection.to(editor.document()));
            editor.delete_and_split_scalar(from, to)
        }
        TypedCommand::InsertContentJson { json } => editor.insert_content_json(json),
        TypedCommand::InsertContentHtml { html } => editor.insert_content_html(html),
        TypedCommand::ToggleMark { mark_type } => editor.toggle_mark(mark_type),
        TypedCommand::SetMark { mark_type, attrs } => editor.set_mark(mark_type, attrs.clone()),
        TypedCommand::UnsetMark { mark_type } => editor.unset_mark(mark_type),
        TypedCommand::ToggleHeading { level } => editor.toggle_heading(*level),
        TypedCommand::ToggleCodeBlock => editor.toggle_code_block(),
        TypedCommand::ToggleBlockquote => editor.toggle_blockquote(),
        TypedCommand::ApplyListType { list_type } => editor.apply_list_type(list_type),
        TypedCommand::WrapInList { list_type, .. } => editor.wrap_in_list(
            selection.from(editor.document()),
            selection.to(editor.document()),
            list_type,
        ),
        TypedCommand::UnwrapFromList => editor.unwrap_from_list(selection.from(editor.document())),
        TypedCommand::IndentListItem => editor.indent_list_item(),
        TypedCommand::OutdentListItem => editor.outdent_list_item(),
        TypedCommand::ToggleTaskItemChecked => editor.toggle_task_item_checked(),
        TypedCommand::InsertNode { node_type } => editor.insert_node_at_selection(node_type),
        TypedCommand::ResizeImage { at, width, height } => {
            editor.resize_image_at_doc_pos(editor.scalar_to_doc(at.offset), *width, *height)
        }
    }
}

#[derive(Debug, Clone)]
enum FixedAction {
    Select {
        label: &'static str,
        needle: &'static str,
        offset: u32,
        len: u32,
        reverse: bool,
    },
    Command {
        label: &'static str,
        needle: &'static str,
        offset: u32,
        len: u32,
        reverse: bool,
        class: usize,
    },
    CommandAtCurrent {
        label: &'static str,
        class: usize,
    },
    ResizeImage,
    Boundary,
    Undo,
    Redo,
}

fn fixed_actions() -> Vec<FixedAction> {
    use FixedAction::{Boundary, Command, CommandAtCurrent, Redo, ResizeImage, Select, Undo};
    vec![
        Select {
            label: "forward selection",
            needle: "Grüße",
            offset: 0,
            len: 4,
            reverse: false,
        },
        Select {
            label: "backward selection",
            needle: "Grüße",
            offset: 0,
            len: 4,
            reverse: true,
        },
        Command {
            label: "insert text",
            needle: "INSERT_TARGET",
            offset: 1,
            len: 0,
            reverse: false,
            class: 0,
        },
        Command {
            label: "delete backward",
            needle: "BACKSPACE_TARGET",
            offset: 2,
            len: 0,
            reverse: false,
            class: 1,
        },
        Command {
            label: "replace selection",
            needle: "REPLACE_TARGET",
            offset: 1,
            len: 3,
            reverse: true,
            class: 2,
        },
        Command {
            label: "delete explicit range",
            needle: "DELETE_RANGE_TARGET",
            offset: 2,
            len: 2,
            reverse: false,
            class: 3,
        },
        Command {
            label: "split block",
            needle: "SPLIT_TARGET",
            offset: 5,
            len: 0,
            reverse: false,
            class: 4,
        },
        Command {
            label: "delete and split",
            needle: "DELETE_SPLIT_TARGET",
            offset: 6,
            len: 3,
            reverse: true,
            class: 5,
        },
        Command {
            label: "toggle mark",
            needle: "TOGGLE_MARK_TARGET",
            offset: 1,
            len: 5,
            reverse: false,
            class: 6,
        },
        Command {
            label: "set mark",
            needle: "SET_MARK_TARGET",
            offset: 1,
            len: 4,
            reverse: true,
            class: 7,
        },
        Command {
            label: "unset mark",
            needle: "UNSET_MARK_TARGET",
            offset: 0,
            len: 17,
            reverse: false,
            class: 8,
        },
        Command {
            label: "toggle heading",
            needle: "HEADING_TARGET",
            offset: 1,
            len: 0,
            reverse: false,
            class: 9,
        },
        Command {
            label: "toggle code block",
            needle: "CODE_TARGET",
            offset: 1,
            len: 0,
            reverse: false,
            class: 10,
        },
        Command {
            label: "toggle blockquote",
            needle: "QUOTE_TARGET",
            offset: 1,
            len: 0,
            reverse: false,
            class: 11,
        },
        Command {
            label: "apply list type",
            needle: "CONVERT_FIRST",
            offset: 1,
            len: 0,
            reverse: false,
            class: 12,
        },
        Command {
            label: "wrap in list",
            needle: "WRAP_TARGET",
            offset: 1,
            len: 3,
            reverse: true,
            class: 13,
        },
        Command {
            label: "unwrap from list",
            needle: "UNWRAP_TARGET",
            offset: 1,
            len: 0,
            reverse: false,
            class: 14,
        },
        Command {
            label: "indent list item",
            needle: "INDENT_SECOND",
            offset: 1,
            len: 0,
            reverse: false,
            class: 15,
        },
        Command {
            label: "outdent list item",
            needle: "OUTDENT_NESTED",
            offset: 1,
            len: 0,
            reverse: false,
            class: 16,
        },
        Command {
            label: "toggle task item",
            needle: "TASK_TARGET",
            offset: 1,
            len: 0,
            reverse: false,
            class: 17,
        },
        Command {
            label: "insert hard break",
            needle: "NODE_TARGET",
            offset: 4,
            len: 0,
            reverse: false,
            class: 18,
        },
        ResizeImage,
        Command {
            label: "insert JSON",
            needle: "JSON_TARGET",
            offset: 0,
            len: 11,
            reverse: false,
            class: 20,
        },
        Command {
            label: "insert HTML",
            needle: "HTML_TARGET",
            offset: 0,
            len: 11,
            reverse: true,
            class: 21,
        },
        Command {
            label: "collapsed stored mark",
            needle: "STORED_MARK_TARGET",
            offset: 3,
            len: 0,
            reverse: false,
            class: 7,
        },
        CommandAtCurrent {
            label: "materialize stored mark",
            class: 0,
        },
        Select {
            label: "clear stored mark by real move",
            needle: "RANDOM_SAFE_TARGET",
            offset: 2,
            len: 0,
            reverse: false,
        },
        Boundary,
        Command {
            label: "independent history probe",
            needle: "RANDOM_SAFE_TARGET",
            offset: 1,
            len: 3,
            reverse: true,
            class: 6,
        },
        Boundary,
        Undo,
        Redo,
    ]
}

fn command_for(class: usize, selected: ScalarTarget) -> TypedCommand {
    match class {
        0 => TypedCommand::InsertText { text: "§".into() },
        1 => TypedCommand::DeleteBackward,
        2 => TypedCommand::ReplaceSelectionText {
            text: "é🙂".into()
        },
        3 => TypedCommand::DeleteRange {
            range: RevisionedRange {
                from: point(selected.start),
                to: point(selected.start + selected.len),
            },
        },
        4 => TypedCommand::SplitBlock,
        5 => TypedCommand::DeleteAndSplit,
        6 => TypedCommand::ToggleMark {
            mark_type: "bold".into(),
        },
        7 => TypedCommand::SetMark {
            mark_type: "italic".into(),
            attrs: HashMap::new(),
        },
        8 => TypedCommand::UnsetMark {
            mark_type: "italic".into(),
        },
        9 => TypedCommand::ToggleHeading { level: 3 },
        10 => TypedCommand::ToggleCodeBlock,
        11 => TypedCommand::ToggleBlockquote,
        12 => TypedCommand::ApplyListType {
            list_type: "orderedList".into(),
        },
        13 => TypedCommand::WrapInList {
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
        },
        14 => TypedCommand::UnwrapFromList,
        15 => TypedCommand::IndentListItem,
        16 => TypedCommand::OutdentListItem,
        17 => TypedCommand::ToggleTaskItemChecked,
        18 => TypedCommand::InsertNode {
            node_type: "hardBreak".into(),
        },
        19 => unreachable!("resize command needs the live image target"),
        20 => TypedCommand::InsertContentJson {
            json: serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"json-insert"}]}]}),
        },
        21 => TypedCommand::InsertContentHtml {
            html: "<p><strong>html-insert</strong></p>".into(),
        },
        _ => unreachable!("unknown command class {class}"),
    }
}

fn xorshift64(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn diagnostic(
    seed: usize,
    step: usize,
    action: &str,
    rng: u64,
    target: Option<ScalarTarget>,
    before: &serde_json::Value,
) -> String {
    format!(
        "seed={seed} step={step} action={action} rng=0x{rng:016x} target={target:?} pre_json={before}"
    )
}

fn json_text_has_mark(value: &serde_json::Value, text: &str, mark_type: &str) -> bool {
    if value.get("type").and_then(serde_json::Value::as_str) == Some("text")
        && value.get("text").and_then(serde_json::Value::as_str) == Some(text)
        && value
            .get("marks")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|marks| {
                marks.iter().any(|mark| {
                    mark.get("type").and_then(serde_json::Value::as_str) == Some(mark_type)
                })
            })
    {
        return true;
    }
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| json_text_has_mark(value, text, mark_type)),
        serde_json::Value::Object(values) => values
            .values()
            .any(|value| json_text_has_mark(value, text, mark_type)),
        _ => false,
    }
}

fn run_fixed(
    harness: &mut TraceHarness,
    seed: usize,
    step: usize,
    action: &FixedAction,
    coverage: &mut [bool; COMMAND_CLASSES],
) -> String {
    let before = harness.legacy.get_json();
    match action {
        FixedAction::Select {
            label,
            needle,
            offset,
            len,
            reverse,
        } => {
            let target = target_by_text(&harness.legacy, needle);
            assert_eq!(
                (target.start, target.len),
                {
                    let yrs = target_by_yrs_text(&harness.yrs, needle);
                    (yrs.start, yrs.len)
                },
                "seed={seed} step={step}: legacy/Yrs PositionMap target mismatch"
            );
            let from = target.start + offset;
            let to = from + len;
            let context = diagnostic(seed, step, label, 0, Some(target), &before);
            harness.select(
                &context,
                if *reverse { to } else { from },
                if *reverse { from } else { to },
            );
            context
        }
        FixedAction::Command {
            label,
            needle,
            offset,
            len,
            reverse,
            class,
        } => {
            let target = target_by_text(&harness.legacy, needle);
            assert_eq!(
                (target.start, target.len),
                {
                    let yrs = target_by_yrs_text(&harness.yrs, needle);
                    (yrs.start, yrs.len)
                },
                "seed={seed} step={step}: legacy/Yrs PositionMap target mismatch"
            );
            let selected = ScalarTarget {
                start: target.start + offset,
                len: *len,
            };
            let context = diagnostic(seed, step, label, 0, Some(target), &before);
            harness.select(
                &context,
                if *reverse {
                    selected.start + selected.len
                } else {
                    selected.start
                },
                if *reverse {
                    selected.start
                } else {
                    selected.start + selected.len
                },
            );
            harness.command(&context, command_for(*class, selected));
            coverage[*class] = true;
            context
        }
        FixedAction::CommandAtCurrent { label, class } => {
            let context = diagnostic(seed, step, label, 0, None, &before);
            harness.command(
                &context,
                command_for(*class, ScalarTarget { start: 0, len: 0 }),
            );
            coverage[*class] = true;
            let json = harness.legacy.get_json();
            assert!(
                json_text_has_mark(&json, "§", "italic"),
                "{context}: stored italic mark did not materialize on inserted text"
            );
            context
        }
        FixedAction::ResizeImage => {
            let target = image_target(&harness.legacy);
            let context = diagnostic(seed, step, "resize image", 0, Some(target), &before);
            harness.command(
                &context,
                TypedCommand::ResizeImage {
                    at: point(target.start),
                    width: 144,
                    height: 96,
                },
            );
            coverage[19] = true;
            context
        }
        FixedAction::Boundary => {
            let context = diagnostic(seed, step, "state-only boundary", 0, None, &before);
            let prior = harness.yrs_render.clone();
            harness.boundary(&context);
            assert_eq!(
                prior, harness.yrs_render,
                "{context}: boundary changed render"
            );
            context
        }
        FixedAction::Undo => {
            let context = diagnostic(seed, step, "independent undo", 0, None, &before);
            harness.undo(&context);
            context
        }
        FixedAction::Redo => {
            let context = diagnostic(seed, step, "independent redo", 0, None, &before);
            harness.redo(&context);
            context
        }
    }
}

fn run_random(harness: &mut TraceHarness, seed: usize, step: usize, rng: &mut u64) -> String {
    let value = xorshift64(rng);
    let before = harness.legacy.get_json();
    let targets = random_text_targets(&harness.legacy);
    assert!(
        !targets.is_empty(),
        "seed={seed} step={step}: no live text targets"
    );
    let target = targets[(value as usize) % targets.len()];
    let within = ((value >> 16) as u32) % target.len;
    let cursor = target.start + within;
    let choice = ((value >> 48) % 9) as usize;
    let target_text = harness
        .legacy
        .document()
        .resolve(harness.legacy.scalar_to_doc(target.start))
        .ok()
        .map(|resolved| resolved.parent(harness.legacy.document()).text_content())
        .unwrap_or_default();
    let context = diagnostic(
        seed,
        step,
        &format!("random-{choice}:{target_text}"),
        value,
        Some(target),
        &before,
    );
    match choice {
        0 => {
            harness.select(&context, cursor, cursor);
            harness.command(
                &context,
                TypedCommand::InsertText {
                    text: if value & 1 == 0 { "λ" } else { "🌱" }.into(),
                },
            );
        }
        1 => {
            let end = (cursor + 1).min(target.start + target.len);
            harness.select(&context, cursor, end);
            harness.command(
                &context,
                TypedCommand::ReplaceSelectionText { text: "ñ".into() },
            );
        }
        2 => {
            let delete_at = (cursor + 1).min(target.start + target.len);
            harness.select(&context, delete_at, delete_at);
            harness.command(&context, TypedCommand::DeleteBackward);
        }
        3 => {
            let end = (cursor + 1).min(target.start + target.len);
            let (anchor, head) = if value & 1 == 0 {
                (cursor, end)
            } else {
                (end, cursor)
            };
            harness.select(&context, anchor, head);
            harness.command(
                &context,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            );
        }
        4 => {
            let (anchor, head) = if value & 1 == 0 {
                (target.start, target.start + target.len)
            } else {
                (target.start + target.len, target.start)
            };
            harness.select(&context, anchor, head);
        }
        5 | 6 => harness.boundary(&context),
        7 => harness.select(&context, cursor, cursor),
        _ => {
            harness.select(&context, cursor, cursor);
            harness.command(&context, TypedCommand::SplitBlock);
        }
    }
    context
}

#[test]
fn frozen_unicode_marks_and_atoms_have_exact_canonical_and_rendered_outputs() {
    let input = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[
            {"type":"text","text":"A🙂","marks":[{"type":"bold"},{"type":"italic"}]},
            {"type":"hardBreak"},
            {"type":"text","text":"漢"}
        ]},
        {"type":"horizontalRule"}
    ]});
    let expected_json = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[
            {"type":"text","text":"A🙂","marks":[{"type":"bold"},{"type":"italic"}]},
            {"type":"hardBreak"},
            {"type":"text","text":"漢"}
        ]},
        {"type":"horizontalRule"}
    ]});
    let expected_html = "<p><strong><em>A🙂</em></strong><br>漢</p><hr>";
    let expected_rendered_text = "A🙂\n漢\n\u{fffc}";
    let expected_blocks = vec![
        vec![
            RenderElement::BlockStart {
                node_type: "paragraph".into(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "A🙂".into(),
                marks: vec![
                    RenderMark {
                        mark_type: "bold".into(),
                        attrs: HashMap::new(),
                    },
                    RenderMark {
                        mark_type: "italic".into(),
                        attrs: HashMap::new(),
                    },
                ],
            },
            RenderElement::VoidInline {
                node_type: "hardBreak".into(),
                doc_pos: 3,
                attrs: HashMap::new(),
            },
            RenderElement::TextRun {
                text: "漢".into(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
        ],
        vec![RenderElement::VoidBlock {
            node_type: "horizontalRule".into(),
            doc_pos: 6,
            attrs: HashMap::new(),
        }],
    ];

    let mut engine = engine(InitializationMode::LocalEmpty);
    engine
        .import_json(&input.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let document = engine.document().unwrap();
    let position_map = engine.position_map().unwrap();

    assert_eq!(engine.document_json().unwrap(), expected_json);
    assert_eq!(engine.document_html().unwrap(), expected_html);
    assert_eq!(render_blocks(document, &tiptap_schema()), expected_blocks);
    assert_eq!(
        rendered_text(document, &tiptap_schema()),
        expected_rendered_text
    );
    assert_eq!(position_map.total_scalars(), 6);
    assert_eq!(position_map.block_count(), 2);
    let paragraph = position_map.block(0).unwrap();
    assert_eq!(
        (
            paragraph.doc_start,
            paragraph.doc_end,
            paragraph.scalar_start,
            paragraph.scalar_len,
            paragraph.scalar_prefix_len,
            paragraph.rendered_break_after,
            paragraph.node_path.as_slice(),
            paragraph.is_void_block,
        ),
        (1, 5, 0, 4, 0, 1, &[0][..], false),
    );
    let atom = position_map.block(1).unwrap();
    assert_eq!(
        (
            atom.doc_start,
            atom.doc_end,
            atom.scalar_start,
            atom.scalar_len,
            atom.scalar_prefix_len,
            atom.rendered_break_after,
            atom.node_path.as_slice(),
            atom.is_void_block,
        ),
        (6, 6, 5, 1, 0, 0, &[1][..], true),
    );
    assert_eq!(scalar_offset_to_utf16(expected_rendered_text, 6), Some(7));
}

#[test]
fn deterministic_mixed_command_traces_match_legacy_after_every_action() {
    let fixed = fixed_actions();
    assert!(fixed.len() < ACTIONS_PER_SEED);
    for seed in 0..SEEDS {
        let mut harness = TraceHarness::new(seed);
        let mut coverage = [false; COMMAND_CLASSES];
        let mut rng = (seed as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        for step in 0..ACTIONS_PER_SEED {
            let context = if let Some(action) = fixed.get(step) {
                run_fixed(&mut harness, seed, step, action, &mut coverage)
            } else {
                run_random(&mut harness, seed, step, &mut rng)
            };
            harness.assert_state(&format!("{context} post-action"));
            if step == 24 {
                assert!(
                    harness.yrs.stored_marks().is_some(),
                    "{context}: collapsed mark did not create exact Yrs stored marks"
                );
                assert_eq!(
                    harness.yrs.stored_marks().unwrap()[0].mark_type(),
                    "italic",
                    "{context}"
                );
            }
            if step == 26 {
                assert_eq!(
                    harness.yrs.stored_marks(),
                    None,
                    "{context}: real selection move did not clear stored marks"
                );
            }
        }
        assert!(
            coverage.into_iter().all(|covered| covered),
            "seed={seed}: fixed prefix command coverage={coverage:?}"
        );
    }
}

#[test]
fn later_marked_text_remains_selectable_after_granular_split() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let document = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[{"type":"text","text":"split"}]},
        {"type":"paragraph","content":[{"type":"text","text":"delete"}]},
        {"type":"paragraph","content":[{"type":"text","text":"marked","marks":[{"type":"italic"}]}]}
    ]});
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let split = target_by_yrs_text(&engine, "split");
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 80_001,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(split.start + 2),
                head: point(split.start + 2),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(80_002, TypedCommand::SplitBlock)
        .unwrap()
        .unwrap();
    let delete = target_by_yrs_text(&engine, "delete");
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 80_003,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(delete.start + 1),
                head: point(delete.start + 3),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(80_004, TypedCommand::DeleteAndSplit)
        .unwrap()
        .unwrap();
    let marked = target_by_yrs_text(&engine, "marked");

    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 80_005,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(marked.start),
                head: point(marked.start + marked.len),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
}

#[test]
fn later_marked_text_remains_selectable_after_granular_range_format() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let document = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[{"type":"text","text":"format"}]},
        {"type":"paragraph","content":[{"type":"text","text":"marked","marks":[{"type":"italic"}]}]}
    ]});
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let format = target_by_yrs_text(&engine, "format");
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 81_001,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(format.start + 1),
                head: point(format.start + 4),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(
            81_002,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    let marked = target_by_yrs_text(&engine, "marked");
    let desired_doc_pos = engine
        .position_map()
        .unwrap()
        .scalar_to_doc(marked.start, engine.document().unwrap());
    let raw = Doc::new();
    let mut raw_txn = raw.transact_mut();
    raw_txn
        .apply_update(Update::decode_v1(&engine.encoded_state().unwrap()).unwrap())
        .unwrap();
    let fragment = raw_txn.get_or_insert_xml_fragment("prosemirror");
    assert!(
        doc_pos_to_relative_point(
            &raw_txn,
            &fragment,
            desired_doc_pos,
            Affinity::Before,
            &tiptap_schema(),
        )
        .is_some(),
        "raw Yrs cannot encode marked start: scalar={}, doc={desired_doc_pos}",
        marked.start,
    );

    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 81_003,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(marked.start),
                head: point(marked.start + marked.len),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
}

#[test]
fn hard_break_split_preserves_the_following_text_target_gap() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let before = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[{"type":"text","text":"left-right"}]},
        {"type":"paragraph","content":[{"type":"text","text":"tail"}]}
    ]});
    engine
        .import_json(&before.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let target = target_by_yrs_text(&engine, "left-right");
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 82_001,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(target.start + 4),
                head: point(target.start + 4),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();

    let result = engine
        .apply_command(
            82_002,
            TypedCommand::InsertNode {
                node_type: "hardBreak".into(),
            },
        )
        .unwrap()
        .unwrap();
    let after = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[
            {"type":"text","text":"left"},
            {"type":"hardBreak"},
            {"type":"text","text":"-right"}
        ]},
        {"type":"paragraph","content":[{"type":"text","text":"tail"}]}
    ]});
    assert_eq!(engine.document_json().unwrap(), after);
    assert_exact_text_selection(&result.selection, 6, 5, 5);

    let undo = engine.undo_with_result(82_003).unwrap().unwrap();
    assert_eq!(engine.document_json().unwrap(), before);
    assert_exact_text_selection(&undo.selection, 5, 4, 4);
    let redo = engine.redo_with_result(82_004).unwrap().unwrap();
    assert_eq!(engine.document_json().unwrap(), after);
    assert_exact_text_selection(&redo.selection, 6, 5, 5);
}

#[test]
fn split_at_the_start_of_a_list_item_textblock_matches_legacy() {
    let document = serde_json::json!({"type":"doc","content":[{
        "type":"orderedList",
        "attrs":{"start":1},
        "content":[
            {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]}]},
            {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"second"}]}]}
        ]
    }]});
    let schema = tiptap_schema();
    let mut legacy = Editor::new(schema, InterceptorPipeline::new(), false);
    legacy.set_json(&document).unwrap();
    let target = target_by_text(&legacy, "second");
    legacy.set_selection_scalar(target.start, target.start);
    legacy
        .split_block(legacy.scalar_to_doc(target.start))
        .unwrap();

    let mut engine = engine(InitializationMode::LocalEmpty);
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 83_001,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(target.start),
                head: point(target.start),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(83_002, TypedCommand::SplitBlock)
        .unwrap()
        .unwrap();

    assert_eq!(engine.document_json().unwrap(), legacy.get_json());

    assert!(legacy.can_undo());
    assert!(engine.can_undo());
    assert!(legacy.undo().is_some(), "advertised legacy undo must apply");
    assert!(engine.undo(83_003).unwrap().is_some());
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());
    assert!(legacy.redo().is_some());
    assert!(engine.redo(83_004).unwrap().is_some());
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());
}

#[test]
fn overlapping_marks_follow_declared_schema_order_in_both_engines() {
    let document = serde_json::json!({"type":"doc","content":[{
        "type":"paragraph",
        "content":[{"type":"text","text":"abc","marks":[{"type":"italic"}]}]
    }]});
    let schema = tiptap_schema();
    let mut legacy = Editor::new(schema, InterceptorPipeline::new(), false);
    legacy.set_json(&document).unwrap();
    legacy.set_selection_scalar(1, 2);
    legacy.toggle_mark("bold").unwrap();

    let mut engine = engine(InitializationMode::LocalEmpty);
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 84_001,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(1),
                head: point(2),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(
            84_002,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();

    let expected = serde_json::json!({"type":"doc","content":[{
        "type":"paragraph",
        "content":[
            {"type":"text","text":"a","marks":[{"type":"italic"}]},
            {"type":"text","text":"b","marks":[{"type":"bold"},{"type":"italic"}]},
            {"type":"text","text":"c","marks":[{"type":"italic"}]}
        ]
    }]});
    assert_eq!(engine.document_json().unwrap(), expected);
    assert_eq!(legacy.get_json(), expected);
}

#[test]
fn split_inside_a_fully_marked_textblock_matches_legacy_and_history() {
    let document = serde_json::json!({"type":"doc","content":[{
        "type":"paragraph",
        "content":[{"type":"text","text":"htλml-insert","marks":[{"type":"bold"}]}]
    }]});
    let schema = tiptap_schema();
    let mut legacy = Editor::new(schema, InterceptorPipeline::new(), false);
    legacy.set_json(&document).unwrap();
    let target = target_by_text(&legacy, "htλml-insert");
    legacy.set_selection_scalar(target.start + 4, target.start + 4);
    legacy
        .split_block(legacy.scalar_to_doc(target.start + 4))
        .unwrap();

    let mut engine = engine(InitializationMode::LocalEmpty);
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let target = target_by_yrs_text(&engine, "htλml-insert");
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 84_101,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(target.start + 4),
                head: point(target.start + 4),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(84_102, TypedCommand::SplitBlock)
        .unwrap()
        .unwrap();
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());

    assert!(legacy.undo().is_some());
    assert!(engine.undo(84_103).unwrap().is_some());
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());
    assert!(legacy.redo().is_some());
    assert!(engine.redo(84_104).unwrap().is_some());
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());
}

#[test]
fn split_ignores_unrelated_storage_boundaries_inside_semantic_mark_runs() {
    let raw = Doc::with_options(Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    });
    let update = {
        let mut txn = raw.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let first = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let first_storage = first.push_back(&mut txn, XmlTextPrelim::new("Grüße🙂漢"));
        first_storage.format(
            &mut txn,
            0,
            7,
            yrs::types::Attrs::from([("bold".into(), Any::Bool(true))]),
        );
        first.push_back(&mut txn, XmlTextPrelim::new(" unicode oracle"));
        let second = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        second.push_back(&mut txn, XmlTextPrelim::new("split-me"));
        txn.encode_state_as_update_v1(&StateVector::default())
    };

    let mut engine = engine(InitializationMode::AwaitRemote);
    engine.apply_remote_update_v1(84_201, &update).unwrap();
    let before = engine.document_json().unwrap();
    assert_eq!(
        before,
        serde_json::json!({"type":"doc","content":[
            {"type":"paragraph","content":[
                {"type":"text","text":"Grüße🙂","marks":[{"type":"bold"}]},
                {"type":"text","text":"漢 unicode oracle"}
            ]},
            {"type":"paragraph","content":[{"type":"text","text":"split-me"}]}
        ]})
    );

    let mut legacy = Editor::new(tiptap_schema(), InterceptorPipeline::new(), false);
    legacy.set_json(&before).unwrap();
    let target = target_by_text(&legacy, "split-me");
    legacy.set_selection_scalar(target.start + 4, target.start + 4);
    legacy
        .split_block(legacy.scalar_to_doc(target.start + 4))
        .unwrap();

    let target = target_by_yrs_text(&engine, "split-me");
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 84_202,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(target.start + 4),
                head: point(target.start + 4),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    let result = engine
        .apply_command(84_203, TypedCommand::SplitBlock)
        .unwrap()
        .unwrap();
    let after = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[
            {"type":"text","text":"Grüße🙂","marks":[{"type":"bold"}]},
            {"type":"text","text":"漢 unicode oracle"}
        ]},
        {"type":"paragraph","content":[{"type":"text","text":"spli"}]},
        {"type":"paragraph","content":[{"type":"text","text":"t-me"}]}
    ]});
    assert_eq!(engine.document_json().unwrap(), after);
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());
    assert_exact_text_selection(&result.selection, 31, 28, 29);

    let undo = engine.undo_with_result(84_204).unwrap().unwrap();
    assert_eq!(engine.document_json().unwrap(), before);
    assert_exact_text_selection(&undo.selection, 29, 27, 28);
    let redo = engine.redo_with_result(84_205).unwrap().unwrap();
    assert_eq!(engine.document_json().unwrap(), after);
    assert_exact_text_selection(&redo.selection, 31, 28, 29);
}

#[test]
fn split_inside_the_first_of_multiple_storage_texts_moves_the_full_suffix() {
    let raw = Doc::with_options(Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    });
    let update = {
        let mut txn = raw.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let first = paragraph.push_back(&mut txn, XmlTextPrelim::new("Grüße🙂漢"));
        first.format(
            &mut txn,
            0,
            8,
            yrs::types::Attrs::from([("bold".into(), Any::Bool(true))]),
        );
        paragraph.push_back(&mut txn, XmlTextPrelim::new(" unicode oracle"));
        txn.encode_state_as_update_v1(&StateVector::default())
    };
    let mut engine = engine(InitializationMode::AwaitRemote);
    engine.apply_remote_update_v1(84_301, &update).unwrap();
    let before = engine.document_json().unwrap();

    let mut legacy = Editor::new(tiptap_schema(), InterceptorPipeline::new(), false);
    legacy.set_json(&before).unwrap();
    legacy.set_selection_scalar(1, 1);
    legacy.split_block(legacy.scalar_to_doc(1)).unwrap();

    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 84_302,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(1),
                head: point(1),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(84_303, TypedCommand::SplitBlock)
        .unwrap()
        .unwrap();
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());

    assert!(legacy.undo().is_some());
    assert!(engine.undo(84_304).unwrap().is_some());
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());
    assert!(legacy.redo().is_some());
    assert!(engine.redo(84_305).unwrap().is_some());
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());
}

#[test]
fn formatted_multi_storage_split_converges_via_standard_updates_and_undo_stays_local() {
    let raw = Doc::with_options(Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    });
    let initial_update = {
        let mut txn = raw.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let first = paragraph.push_back(&mut txn, XmlTextPrelim::new("Grüße🙂漢"));
        first.format(
            &mut txn,
            0,
            8,
            yrs::types::Attrs::from([("bold".into(), Any::Bool(true))]),
        );
        paragraph.push_back(&mut txn, XmlTextPrelim::new(" unicode oracle"));
        txn.encode_state_as_update_v1(&StateVector::default())
    };
    let before = serde_json::json!({"type":"doc","content":[{
        "type":"paragraph","content":[
            {"type":"text","text":"Grüße🙂漢","marks":[{"type":"bold"}]},
            {"type":"text","text":" unicode oracle"}
        ]
    }]});
    let after = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[{"type":"text","text":"G","marks":[{"type":"bold"}]}]},
        {"type":"paragraph","content":[
            {"type":"text","text":"rüße🙂漢","marks":[{"type":"bold"}]},
            {"type":"text","text":" unicode oracle"}
        ]}
    ]});

    let mut local = engine(InitializationMode::AwaitRemote);
    let mut replica = engine(InitializationMode::AwaitRemote);
    local
        .apply_remote_update_v1(84_401, &initial_update)
        .unwrap();
    replica
        .apply_remote_update_v1(84_402, &initial_update)
        .unwrap();
    assert_eq!(local.document_json().unwrap(), before);
    assert_eq!(replica.document_json().unwrap(), before);

    local
        .apply_typed_transaction(TypedTransaction {
            request_id: 84_403,
            base_document_revision: local.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(1),
                head: point(1),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    local
        .apply_command(84_404, TypedCommand::SplitBlock)
        .unwrap()
        .unwrap();
    assert_eq!(local.document_json().unwrap(), after);
    assert!(local.can_undo());
    assert!(!replica.can_undo());

    replica
        .apply_remote_update_v1(84_405, &local.encoded_state().unwrap())
        .unwrap();
    assert_eq!(replica.document_json().unwrap(), after);
    assert_eq!(replica.document_html(), local.document_html());
    assert!(
        !replica.can_undo(),
        "remote split must not enter local history"
    );

    local.undo(84_406).unwrap().unwrap();
    assert_eq!(local.document_json().unwrap(), before);
    assert_eq!(replica.document_json().unwrap(), after);
    assert!(local.can_redo());
    assert!(!replica.can_undo());
    replica
        .apply_remote_update_v1(84_407, &local.encoded_state().unwrap())
        .unwrap();
    assert_eq!(replica.document_json().unwrap(), before);
    assert_eq!(replica.document_html(), local.document_html());
    assert!(!replica.can_undo());

    local.redo(84_408).unwrap().unwrap();
    replica
        .apply_remote_update_v1(84_409, &local.encoded_state().unwrap())
        .unwrap();
    assert_eq!(local.document_json().unwrap(), after);
    assert_eq!(replica.document_json().unwrap(), after);
    assert_eq!(replica.document_html(), local.document_html());
    assert!(!replica.can_undo());
}

#[test]
fn split_inside_a_blockquote_matches_legacy_and_history() {
    let document = serde_json::json!({"type":"doc","content":[{
        "type":"blockquote",
        "content":[{"type":"paragraph","content":[{"type":"text","text":"quote"}]}]
    }]});
    let schema = tiptap_schema();
    let mut legacy = Editor::new(schema, InterceptorPipeline::new(), false);
    legacy.set_json(&document).unwrap();
    legacy.set_selection_scalar(2, 2);
    legacy.split_block(legacy.scalar_to_doc(2)).unwrap();

    let mut engine = engine(InitializationMode::LocalEmpty);
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 85_001,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(2),
                head: point(2),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(85_002, TypedCommand::SplitBlock)
        .unwrap()
        .unwrap();
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());

    assert!(legacy.undo().is_some());
    assert!(engine.undo(85_003).unwrap().is_some());
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());
    assert!(legacy.redo().is_some());
    assert!(engine.redo(85_004).unwrap().is_some());
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());
}

#[test]
fn split_at_a_storage_text_boundary_inside_a_blockquote_matches_legacy() {
    let raw = Doc::with_options(Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    });
    let update = {
        let mut txn = raw.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let quote = fragment.push_back(&mut txn, XmlElementPrelim::empty("blockquote"));
        let paragraph = quote.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("QUO🌱T"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("E_TARGET"));
        txn.encode_state_as_update_v1(&StateVector::default())
    };
    let mut engine = engine(InitializationMode::AwaitRemote);
    engine.apply_remote_update_v1(85_101, &update).unwrap();
    let before = engine.document_json().unwrap();
    assert_eq!(
        before,
        serde_json::json!({"type":"doc","content":[{
            "type":"blockquote",
            "content":[{"type":"paragraph","content":[{"type":"text","text":"QUO🌱TE_TARGET"}]}]
        }]})
    );

    let mut legacy = Editor::new(tiptap_schema(), InterceptorPipeline::new(), false);
    legacy.set_json(&before).unwrap();
    let target = target_by_text(&legacy, "QUO🌱TE_TARGET");
    legacy.set_selection_scalar(target.start + 5, target.start + 5);
    legacy
        .split_block(legacy.scalar_to_doc(target.start + 5))
        .unwrap();

    let target = target_by_yrs_text(&engine, "QUO🌱TE_TARGET");
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 85_102,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(target.start + 5),
                head: point(target.start + 5),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    let result = engine
        .apply_command(85_103, TypedCommand::SplitBlock)
        .unwrap()
        .unwrap();
    let after = serde_json::json!({"type":"doc","content":[{
        "type":"blockquote",
        "content":[
            {"type":"paragraph","content":[{"type":"text","text":"QUO🌱T"}]},
            {"type":"paragraph","content":[{"type":"text","text":"E_TARGET"}]}
        ]
    }]});
    assert_eq!(engine.document_json().unwrap(), after);
    assert_eq!(engine.document_json().unwrap(), legacy.get_json());
    assert_exact_text_selection(&result.selection, 9, 6, 7);

    let undo = engine.undo_with_result(85_104).unwrap().unwrap();
    assert_eq!(engine.document_json().unwrap(), before);
    assert_exact_text_selection(&undo.selection, 7, 5, 6);
    let redo = engine.redo_with_result(85_105).unwrap().unwrap();
    assert_eq!(engine.document_json().unwrap(), after);
    assert_exact_text_selection(&redo.selection, 9, 6, 7);
}

#[test]
fn remote_standard_updates_shift_and_normalize_relative_selection_and_keep_undo_local() {
    let mut server = engine(InitializationMode::LocalEmpty);
    server
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"base"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let server_state = server.encoded_state().unwrap();
    let mut local = engine(InitializationMode::AwaitRemote);
    local.apply_remote_update_v1(1, &server_state).unwrap();
    let mut remote = engine(InitializationMode::AwaitRemote);
    remote.apply_remote_update_v1(2, &server_state).unwrap();

    let set_local = |engine: &mut YrsDocumentEngine, request_id, anchor, head| {
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point(anchor),
                    head: point(head),
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
    };
    set_local(&mut local, 3, 4, 4);
    local
        .apply_command(
            4,
            TypedCommand::InsertText {
                text: "local".into(),
            },
        )
        .unwrap();
    remote
        .apply_remote_update_v1(5, &local.encoded_state().unwrap())
        .unwrap();

    set_local(&mut local, 6, 9, 5);
    let before_duplicate_revision = local.revision();
    let before_duplicate_state = local.state_revision();
    let before_duplicate_history = (local.can_undo(), local.can_redo());
    let duplicate = local
        .apply_remote_update_v1(7, &remote.encoded_state().unwrap())
        .unwrap();
    assert!(!duplicate.changed);
    assert_eq!(local.revision(), before_duplicate_revision);
    assert_eq!(local.state_revision(), before_duplicate_state);
    assert_eq!(
        (local.can_undo(), local.can_redo()),
        before_duplicate_history
    );

    set_local(&mut remote, 8, 0, 0);
    remote
        .apply_command(
            9,
            TypedCommand::InsertText {
                text: "remote ".into(),
            },
        )
        .unwrap();
    local
        .apply_remote_update_v1(10, &remote.encoded_state().unwrap())
        .unwrap();
    assert_eq!(local.document_json(), remote.document_json());
    assert_eq!(local.document_html(), remote.document_html());
    let ResolvedSelection::Text { anchor, head } = local.resolved_selection().unwrap() else {
        panic!("remote update lost text selection")
    };
    assert_eq!(
        (anchor.scalar, head.scalar),
        (16, 12),
        "backward relative selection did not shift"
    );
    assert_eq!((anchor.document, head.document), (17, 13));
    assert_eq!((anchor.utf16, head.utf16), (16, 12));
    assert_eq!(
        local.last_committed_origin(),
        Some(TransactionOrigin::RemoteSync)
    );

    set_local(&mut local, 11, 0, 16);
    remote
        .apply_command(
            12,
            TypedCommand::DeleteRange {
                range: RevisionedRange {
                    from: point(0),
                    to: point(1),
                },
            },
        )
        .unwrap();
    local
        .apply_remote_update_v1(13, &remote.encoded_state().unwrap())
        .unwrap();
    let ResolvedSelection::Text { anchor, head } = local.resolved_selection().unwrap() else {
        panic!("remote delete failed to normalize selection")
    };
    assert_eq!((anchor.scalar, head.scalar), (0, 15));
    assert_eq!((anchor.document, head.document), (1, 16));
    assert_eq!((anchor.utf16, head.utf16), (0, 15));

    local.undo(14).unwrap().unwrap();
    assert_eq!(
        local.document().unwrap().root().text_content(),
        "emote base"
    );
    local.redo(15).unwrap().unwrap();
    assert_eq!(
        local.document().unwrap().root().text_content(),
        "emote baselocal"
    );

    remote
        .apply_remote_update_v1(16, &local.encoded_state().unwrap())
        .unwrap();
    local
        .apply_remote_update_v1(17, &remote.encoded_state().unwrap())
        .unwrap();
    assert_eq!(
        local.encoded_state().unwrap(),
        remote.encoded_state().unwrap()
    );
    assert_eq!(local.document_json(), remote.document_json());
}
