use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::boundary::ResourceLimits;
use crate::model::{Fragment, Mark, Node};
use crate::tiptap_schema;
use crate::yrs_engine::{
    Affinity, DocumentScope, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    OperationError, ResolvedSelection, RevisionedPosition, RevisionedRange, SelectionInput,
    SelectionIntent, TransactionCommit, TransactionOrigin, TypedOperation, TypedTransaction,
    YrsDocumentEngine, YrsEngineConfig,
};
use yrs::sync::time::Clock;

const EMPTY: &str = r#"{"type":"doc","content":[{"type":"paragraph"}]}"#;
const PLAIN_AB: &str =
    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#;
const PLAIN_ABC: &str =
    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#;
const BOLD_A: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a","marks":[{"type":"bold"}]}]}]}"#;
const PLAIN_HISTORY_SNAPSHOT_BYTES: usize = 512 + "prosemirror".len() + 2;

#[derive(Debug, Clone, PartialEq)]
struct HistoryAudit {
    encoded_state: Vec<u8>,
    document_json: Option<serde_json::Value>,
    document_revision: u64,
    state_revision: u64,
    selection: Option<ResolvedSelection>,
    stored_marks: Option<Vec<Mark>>,
    can_undo: bool,
    can_redo: bool,
    last_origin: Option<TransactionOrigin>,
}

struct Harness {
    engine: YrsDocumentEngine,
    now: Arc<AtomicU64>,
    next_request_id: u64,
}

impl Harness {
    fn new() -> Self {
        Self::with_limits(EditingLimits::default())
    }

    fn with_limits(editing_limits: EditingLimits) -> Self {
        Self::with_config(
            editing_limits,
            ResourceLimits::default(),
            None,
            InitializationMode::LocalEmpty,
            None,
        )
    }

    fn with_config(
        editing_limits: EditingLimits,
        resource_limits: ResourceLimits,
        max_length: Option<u32>,
        initialization_mode: InitializationMode,
        scope: Option<DocumentScope>,
    ) -> Self {
        let now = Arc::new(AtomicU64::new(10_000));
        let clock_now = Arc::clone(&now);
        let clock: Arc<dyn Clock> = Arc::new(move || clock_now.load(Ordering::SeqCst));
        let engine = YrsDocumentEngine::new_with_history_clock(
            YrsEngineConfig {
                schema: tiptap_schema(),
                fragment_name: "prosemirror".into(),
                initialization_mode,
                resource_limits,
                editing_limits,
                max_length,
                scope,
            },
            clock,
        )
        .unwrap();
        Self {
            engine,
            now,
            next_request_id: 1,
        }
    }

    fn advance(&self, millis: u64) {
        self.now.fetch_add(millis, Ordering::SeqCst);
    }

    fn apply(
        &mut self,
        origin: TransactionOrigin,
        history_policy: HistoryPolicy,
        operations: Vec<TypedOperation>,
        selection_intent: SelectionIntent,
    ) -> Result<TransactionCommit, OperationError> {
        let request_id = self.take_request_id();
        self.engine.apply_typed_transaction(TypedTransaction {
            request_id,
            base_document_revision: self.engine.revision(),
            origin,
            operations,
            selection_intent,
            history_policy,
        })
    }

    fn import_json(&mut self, json: &str) {
        self.engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        assert!(!self.engine.can_undo());
        assert!(!self.engine.can_redo());
    }

    fn insert(
        &mut self,
        text: &str,
        origin: TransactionOrigin,
        history_policy: HistoryPolicy,
    ) -> Result<TransactionCommit, OperationError> {
        let at = self.engine.position_map().unwrap().total_scalars();
        self.apply(
            origin,
            history_policy,
            vec![TypedOperation::InsertText {
                at: point(at),
                text: text.into(),
                marks: vec![],
            }],
            SelectionIntent::UseOperationResult,
        )
    }

    fn delete(
        &mut self,
        from: u32,
        to: u32,
        history_policy: HistoryPolicy,
    ) -> Result<TransactionCommit, OperationError> {
        self.apply(
            TransactionOrigin::LocalInput,
            history_policy,
            vec![TypedOperation::DeleteRange {
                range: range(from, to),
            }],
            SelectionIntent::UseOperationResult,
        )
    }

    fn undo(&mut self) -> Result<Option<TransactionCommit>, OperationError> {
        let request_id = self.take_request_id();
        self.engine.undo(request_id)
    }

    fn redo(&mut self) -> Result<Option<TransactionCommit>, OperationError> {
        let request_id = self.take_request_id();
        self.engine.redo(request_id)
    }

    fn take_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        request_id
    }

    fn audit(&self) -> HistoryAudit {
        HistoryAudit {
            encoded_state: self.engine.encoded_state().unwrap(),
            document_json: self.engine.document_json(),
            document_revision: self.engine.revision(),
            state_revision: self.engine.state_revision(),
            selection: self.engine.resolved_selection().cloned(),
            stored_marks: self.engine.stored_marks().map(<[Mark]>::to_vec),
            can_undo: self.engine.can_undo(),
            can_redo: self.engine.can_redo(),
            last_origin: self.engine.last_committed_origin(),
        }
    }
}

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

fn range(from: u32, to: u32) -> RevisionedRange {
    RevisionedRange {
        from: point(from),
        to: point(to),
    }
}

fn cursor(offset: u32) -> SelectionIntent {
    SelectionIntent::Set(SelectionInput::Text {
        anchor: point(offset),
        head: point(offset),
    })
}

fn mark(mark_type: &str) -> Mark {
    Mark::new(mark_type.into(), HashMap::new())
}

fn text(engine: &YrsDocumentEngine) -> String {
    fn append(node: &Node, output: &mut String) {
        if let Some(value) = node.text_str() {
            output.push_str(value);
        }
        if let Some(content) = node.content() {
            for child in content.iter() {
                append(child, output);
            }
        }
    }

    let mut output = String::new();
    append(engine.document().unwrap().root(), &mut output);
    output
}

fn undo_commit(harness: &mut Harness) -> TransactionCommit {
    harness.undo().unwrap().expect("an undo item should exist")
}

fn redo_commit(harness: &mut Harness) -> TransactionCommit {
    harness.redo().unwrap().expect("a redo item should exist")
}

#[test]
fn local_input_command_and_api_origins_are_recorded_and_undo_redo_report_semantic_origin() {
    for origin in [
        TransactionOrigin::LocalInput,
        TransactionOrigin::LocalCommand,
        TransactionOrigin::LocalApi,
    ] {
        let mut harness = Harness::new();
        harness.insert("a", origin, HistoryPolicy::Auto).unwrap();
        assert!(harness.engine.can_undo(), "{origin:?}");
        assert!(!harness.engine.can_redo(), "{origin:?}");

        let before_undo_revision = harness.engine.revision();
        let before_undo_state_revision = harness.engine.state_revision();
        let undo = undo_commit(&mut harness);
        assert_eq!(text(&harness.engine), "", "{origin:?}");
        assert_eq!(undo.origin, TransactionOrigin::UndoRedo, "{origin:?}");
        assert_eq!(
            undo.document_revision,
            before_undo_revision + 1,
            "{origin:?}"
        );
        assert_eq!(
            undo.state_revision,
            before_undo_state_revision + 1,
            "{origin:?}"
        );
        assert_eq!(
            harness.engine.last_committed_origin(),
            Some(TransactionOrigin::UndoRedo)
        );
        assert!(!harness.engine.can_undo(), "{origin:?}");
        assert!(harness.engine.can_redo(), "{origin:?}");

        let redo = redo_commit(&mut harness);
        assert_eq!(text(&harness.engine), "a", "{origin:?}");
        assert_eq!(redo.origin, TransactionOrigin::UndoRedo, "{origin:?}");
        assert!(harness.engine.can_undo(), "{origin:?}");
        assert!(!harness.engine.can_redo(), "{origin:?}");
    }
}

#[test]
fn empty_undo_and_redo_are_exact_no_ops() {
    let mut harness = Harness::new();
    let before = harness.audit();
    assert_eq!(harness.undo().unwrap(), None);
    assert_eq!(harness.audit(), before);
    assert_eq!(harness.redo().unwrap(), None);
    assert_eq!(harness.audit(), before);
}

#[test]
fn insertion_grouping_uses_strictly_less_than_500_milliseconds() {
    for (elapsed, expected_after_one_undo, second_undo_exists) in
        [(499, "", false), (500, "a", true), (501, "a", true)]
    {
        let mut harness = Harness::new();
        harness
            .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
            .unwrap();
        harness.advance(elapsed);
        harness
            .insert("b", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
            .unwrap();

        undo_commit(&mut harness);
        assert_eq!(
            text(&harness.engine),
            expected_after_one_undo,
            "{elapsed}ms"
        );
        assert_eq!(harness.engine.can_undo(), second_undo_exists, "{elapsed}ms");

        redo_commit(&mut harness);
        assert_eq!(text(&harness.engine), "ab", "{elapsed}ms");
    }
}

#[test]
fn deletion_grouping_uses_strictly_less_than_500_milliseconds() {
    for (elapsed, expected_after_one_undo, second_undo_exists) in
        [(499, "abc", false), (500, "ab", true), (501, "ab", true)]
    {
        let mut harness = Harness::new();
        harness.import_json(PLAIN_ABC);
        harness.delete(2, 3, HistoryPolicy::Auto).unwrap();
        harness.advance(elapsed);
        harness.delete(1, 2, HistoryPolicy::Auto).unwrap();

        undo_commit(&mut harness);
        assert_eq!(
            text(&harness.engine),
            expected_after_one_undo,
            "{elapsed}ms"
        );
        assert_eq!(harness.engine.can_undo(), second_undo_exists, "{elapsed}ms");
    }
}

#[test]
fn format_structural_and_replacement_operations_force_boundaries() {
    let mut format = Harness::new();
    format
        .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    format.advance(1);
    format
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Auto,
            vec![TypedOperation::AddMark {
                range: range(0, 1),
                mark: mark("bold"),
            }],
            SelectionIntent::Preserve,
        )
        .unwrap();
    undo_commit(&mut format);
    assert_eq!(text(&format.engine), "a");
    assert_eq!(
        format.engine.document_json(),
        serde_json::from_str(PLAIN_AB)
            .ok()
            .map(|mut value: serde_json::Value| {
                value["content"][0]["content"][0]["text"] = serde_json::json!("a");
                value
            })
    );
    undo_commit(&mut format);
    assert_eq!(text(&format.engine), "");

    let mut structural = Harness::new();
    structural.import_json(PLAIN_AB);
    structural
        .insert("x", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    structural.advance(1);
    structural
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Auto,
            vec![TypedOperation::SplitBlock {
                at: point(0),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            }],
            SelectionIntent::UseOperationResult,
        )
        .unwrap();
    assert_eq!(
        structural
            .engine
            .document()
            .unwrap()
            .root()
            .content()
            .unwrap()
            .iter()
            .count(),
        2
    );
    undo_commit(&mut structural);
    assert_eq!(text(&structural.engine), "abx");
    assert_eq!(
        structural
            .engine
            .document()
            .unwrap()
            .root()
            .content()
            .unwrap()
            .iter()
            .count(),
        1
    );
    undo_commit(&mut structural);
    assert_eq!(text(&structural.engine), "ab");

    let mut replacement = Harness::new();
    replacement.import_json(PLAIN_AB);
    replacement
        .insert("x", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    replacement.advance(1);
    replacement
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Auto,
            vec![TypedOperation::ReplaceRange {
                range: range(0, 1),
                content: Fragment::from(vec![Node::text("P".into(), vec![])]),
            }],
            SelectionIntent::UseOperationResult,
        )
        .unwrap();
    assert_eq!(text(&replacement.engine), "Pbx");
    undo_commit(&mut replacement);
    assert_eq!(text(&replacement.engine), "abx");
    undo_commit(&mut replacement);
    assert_eq!(text(&replacement.engine), "ab");
}

#[test]
fn explicit_boundary_separates_otherwise_compatible_edits() {
    let mut harness = Harness::new();
    harness
        .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    harness.advance(1);
    harness
        .insert("b", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    harness.advance(1);
    harness
        .insert("c", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();

    undo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "ab");
    undo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "a");
    undo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "");
}

#[test]
fn skip_no_op_and_selection_only_transactions_create_no_history() {
    let mut skipped = Harness::new();
    skipped
        .insert("a", TransactionOrigin::LocalApi, HistoryPolicy::Skip)
        .unwrap();
    assert!(!skipped.engine.can_undo());

    let mut no_op = Harness::new();
    no_op.import_json(BOLD_A);
    let commit = no_op
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Auto,
            vec![TypedOperation::AddMark {
                range: range(0, 1),
                mark: mark("bold"),
            }],
            SelectionIntent::Preserve,
        )
        .unwrap();
    assert!(!commit.changed);
    assert!(!no_op.engine.can_undo());

    let mut selection_only = Harness::new();
    selection_only.import_json(PLAIN_AB);
    let before_document_revision = selection_only.engine.revision();
    let commit = selection_only
        .apply(
            TransactionOrigin::LocalInput,
            HistoryPolicy::Auto,
            vec![],
            cursor(1),
        )
        .unwrap();
    assert!(commit.changed);
    assert_eq!(selection_only.engine.revision(), before_document_revision);
    assert!(!selection_only.engine.can_undo());
}

#[test]
fn no_op_between_compatible_edits_does_not_reset_capture_time() {
    let mut harness = Harness::new();
    harness
        .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    harness.advance(250);
    let no_op = harness
        .apply(
            TransactionOrigin::LocalInput,
            HistoryPolicy::Auto,
            vec![],
            SelectionIntent::Preserve,
        )
        .unwrap();
    assert!(!no_op.changed);
    harness.advance(249);
    harness
        .insert("b", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    undo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "");
}

#[test]
fn excluded_typed_origins_reject_atomically_and_import_restore_create_no_history() {
    for origin in [
        TransactionOrigin::UndoRedo,
        TransactionOrigin::RemoteSync,
        TransactionOrigin::SnapshotRestore,
        TransactionOrigin::DocumentImport,
    ] {
        let mut harness = Harness::new();
        let before = harness.audit();
        let error = harness
            .apply(
                origin,
                HistoryPolicy::Auto,
                vec![TypedOperation::InsertText {
                    at: point(1),
                    text: "x".into(),
                    marks: vec![],
                }],
                SelectionIntent::UseOperationResult,
            )
            .unwrap_err();
        assert_eq!(error.code, "TRANSACTION_INVALID", "{origin:?}");
        assert_eq!(harness.audit(), before, "{origin:?}");
    }

    let mut imported = Harness::new();
    imported
        .insert("x", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    assert!(imported.engine.can_undo());
    imported.import_json(PLAIN_AB);
    assert!(!imported.engine.can_undo());
    assert!(!imported.engine.can_redo());

    let scope = DocumentScope {
        document_id: "history-doc".into(),
        lineage_id: "history-lineage".into(),
    };
    let mut source = Harness::with_config(
        EditingLimits::default(),
        ResourceLimits::default(),
        None,
        InitializationMode::LocalEmpty,
        Some(scope.clone()),
    );
    source.import_json(PLAIN_ABC);
    let snapshot = source.engine.export_snapshot().unwrap();

    let mut restored = Harness::with_config(
        EditingLimits::default(),
        ResourceLimits::default(),
        None,
        InitializationMode::LocalEmpty,
        Some(scope),
    );
    restored
        .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    restored
        .insert("b", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    undo_commit(&mut restored);
    assert!(restored.engine.can_undo());
    assert!(restored.engine.can_redo());
    let commit = restored.engine.restore_snapshot(&snapshot).unwrap();
    assert!(commit.changed);
    assert_eq!(text(&restored.engine), "abc");
    assert!(!restored.engine.can_undo());
    assert!(!restored.engine.can_redo());
}

#[test]
fn new_recorded_edit_after_undo_clears_redo() {
    let mut harness = Harness::new();
    harness
        .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    harness
        .insert("b", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    undo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "a");
    assert!(harness.engine.can_redo());

    harness
        .insert("c", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    assert_eq!(text(&harness.engine), "ac");
    assert!(!harness.engine.can_redo());
    assert_eq!(harness.redo().unwrap(), None);
}

include!("yrs_engine_history_test/metadata_and_replay.rs");
