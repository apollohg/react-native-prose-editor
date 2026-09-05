use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::boundary::ResourceLimits;
use crate::model::Mark;
use crate::schema::presets::tiptap_schema;
use crate::selection::Selection;
use crate::serialize::FromHtmlOptions;
use crate::serialize::{from_prosemirror_json, UnknownTypeMode};
use crate::transform::DocumentValidator;
use serde_json::json;
use sha2::Digest;
use yrs::OffsetKind;

use yrs::branch::{Branch, BranchID, BranchPtr};
use yrs::types::xml::{XmlFragment, XmlFragmentPrelim, XmlIn, XmlOut, XmlTextPrelim, XmlTextRef};
use yrs::{updates::decoder::Decode, Update};
use yrs::{Assoc, ClientID, Doc, Options, ReadTxn, StateVector, StickyIndex, Transact, WriteTxn};

use crate::yrs_engine::compiler::SelectionPlan;
use crate::yrs_engine::mutation::YrsMutationAction;
use crate::yrs_engine::{
    Affinity, CommandPlan, EditorOffsetKind, HistoryPolicy, ResolvedSelection, RevisionedPosition,
    RevisionedRange, SelectionInput, SelectionIntent, TransactionOrigin, TypedCommand,
    TypedOperation, TypedTransaction,
};

use super::{
    admit_max_encoded_state_len, check_compiled_commit_preparation_stage_for_test,
    encode_state_bounded, equivalent_private_candidate_doc, fresh_utf16_doc_excluding,
    fresh_utf16_doc_excluding_with, history_metadata_bytes,
    mark_compiled_commit_durable_write_for_test, prepare_import_candidate_cache,
    reset_encoded_state_reuse_counts_for_test, reset_import_receipt_sha256_counts_for_test,
    reset_import_receipt_state_decodings_for_test, reset_import_state_encoding_counts_for_test,
    reset_prepared_candidate_cache_counts_for_test, retained_import_state_charge,
    seal_candidate_state_vector, set_compiled_commit_stage_failpoint_for_test,
    set_outbound_staging_copy_failure_for_test,
    set_quarantined_update_reservation_failure_for_test,
    take_compiled_commit_authority_counts_for_test, take_encoded_state_reuse_counts_for_test,
    take_import_receipt_sha256_counts_for_test, take_import_receipt_state_decodings_for_test,
    take_import_state_encoding_counts_for_test, take_prepared_candidate_cache_counts_for_test,
    utf16_doc, CandidateDocument, CompiledCommitPreparationStage, CompiledTransaction,
    EngineDocumentState, OutboundUpdateSink, ValidatedImportDocument, YrsDocumentCodec,
    YrsDocumentEngine, YrsEngineConfig,
};

#[derive(Debug, PartialEq)]
struct AtomicAudit {
    encoded: Vec<u8>,
    json: Option<serde_json::Value>,
    html: Option<String>,
    revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
    client_id: u64,
    durable_client_ids: HashSet<u64>,
    origin: Option<TransactionOrigin>,
    scope: Option<crate::yrs_engine::DocumentScope>,
    fragment: String,
    fingerprint: String,
    selection: Option<crate::yrs_engine::ResolvedSelection>,
    stored_marks: Option<Vec<crate::model::Mark>>,
    can_undo: bool,
    can_redo: bool,
    retained_history_units: u64,
    replay_audit: (usize, usize, bool),
}

fn atomic_audit(engine: &YrsDocumentEngine) -> AtomicAudit {
    AtomicAudit {
        encoded: engine.encoded_state().unwrap(),
        json: engine.document_json(),
        html: engine.document_html(),
        revision: engine.revision,
        state_revision: engine.state_revision,
        yrs_state_epoch: engine.yrs_state_epoch,
        client_id: engine.client_id(),
        durable_client_ids: engine.durable_client_ids.clone(),
        origin: engine.last_committed_origin,
        scope: engine.scope.clone(),
        fragment: engine.fragment_name.clone(),
        fingerprint: engine.schema_fingerprint.clone(),
        selection: engine.resolved_selection().cloned(),
        stored_marks: engine.stored_marks().map(<[_]>::to_vec),
        can_undo: engine.can_undo(),
        can_redo: engine.can_redo(),
        retained_history_units: engine.history.retained_units(0).unwrap(),
        replay_audit: engine.history.replay_audit_for_test(),
    }
}

fn assert_prepared_candidate_state_vector_exact(engine: &YrsDocumentEngine) {
    let cache = engine
        .prepared_candidate_cache
        .as_ref()
        .expect("successful local mutation must retain its exact private candidate");
    let candidate_txn = cache.doc.transact();
    let live_txn = engine.doc.transact();
    assert_eq!(cache.state_vector, candidate_txn.state_vector());
    assert_eq!(cache.state_vector, live_txn.state_vector());
}

fn transaction_engine() -> YrsDocumentEngine {
    transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits::default())
}

fn transaction_engine_with_editing_limits(
    editing_limits: crate::yrs_engine::EditingLimits,
) -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits,
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "doc".into(),
            lineage_id: "lineage".into(),
        }),
    })
    .unwrap()
}

fn transaction_engine_with_resource_limits_and_mode(
    resource_limits: ResourceLimits,
    initialization_mode: crate::yrs_engine::InitializationMode,
) -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode,
        resource_limits,
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "limit-drift-doc".into(),
            lineage_id: "limit-drift-lineage".into(),
        }),
    })
    .unwrap()
}

fn hard_break_insert_transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
    TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![TypedOperation::InsertNode {
            at: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            node: crate::model::Node::void("hardBreak".into(), HashMap::new()),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Skip,
    }
}

fn paragraph_insert_transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
    TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![TypedOperation::InsertNode {
            at: RevisionedPosition {
                offset: engine.position_map().unwrap().total_scalars(),
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            node: crate::model::Node::element(
                "paragraph".into(),
                HashMap::new(),
                crate::model::Fragment::empty(),
            ),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Skip,
    }
}

fn derived_evidence_matches_runtime_limits(engine: &YrsDocumentEngine) -> bool {
    let state = engine.derived_state.as_ref().unwrap();
    state.matches_materialized_mutation_identity(
        &state.canonical_artifact,
        state.canonical_artifact.sha256(),
        state.canonical_artifact.serialized_len(),
        &engine.resource_limits,
        &engine.schema_fingerprint,
        engine.revision,
        engine.state_revision,
        engine.yrs_state_epoch,
    )
}

fn assert_limit_drift_semantic_parity(
    drifted: &YrsDocumentEngine,
    preconfigured: &YrsDocumentEngine,
) {
    assert_eq!(drifted.document_json(), preconfigured.document_json());
    assert_eq!(drifted.document_html(), preconfigured.document_html());
    assert_eq!(
        drifted.resolved_selection(),
        preconfigured.resolved_selection()
    );
    assert_eq!(drifted.revision(), preconfigured.revision());
    assert_eq!(drifted.state_revision(), preconfigured.state_revision());
    assert_eq!(drifted.yrs_state_epoch, preconfigured.yrs_state_epoch);
    assert_eq!(drifted.can_undo(), preconfigured.can_undo());
    assert_eq!(drifted.can_redo(), preconfigured.can_redo());
    let drifted_state = drifted.derived_state.as_ref().unwrap();
    let preconfigured_state = preconfigured.derived_state.as_ref().unwrap();
    assert_eq!(
        drifted_state.canonical_artifact.sha256(),
        preconfigured_state.canonical_artifact.sha256()
    );
    assert!(derived_evidence_matches_runtime_limits(drifted));
    assert!(derived_evidence_matches_runtime_limits(preconfigured));
}

#[derive(Debug, Clone, Copy)]
enum DeferredInsertCase {
    StrictInteriorEqualMarks,
    Empty,
    LeafBoundary,
    MarkMismatch,
    StructuralGrowth,
    UnavailableUpperBound,
    OverflowingUpperBound,
    OneOverOutputLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionAdmissionKind {
    Eager,
    Deferred,
}

struct DeferredInsertFixture {
    engine: YrsDocumentEngine,
    command: TypedCommand,
}

impl DeferredInsertFixture {
    fn execution_admission_kind(&self) -> ExecutionAdmissionKind {
        let preparation = std::cell::RefCell::new(None);
        let _ = self
            .engine
            .plan_command_internal(65_201, self.command.clone(), Some(&preparation));
        let Some(proof) = preparation.into_inner() else {
            return ExecutionAdmissionKind::Eager;
        };
        match proof.execution_admission {
            crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(_) => {
                ExecutionAdmissionKind::Eager
            }
            crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(_) => {
                ExecutionAdmissionKind::Deferred
            }
        }
    }
}

fn deferred_insert_fixture(case: DeferredInsertCase) -> DeferredInsertFixture {
    let mut engine = match case {
        DeferredInsertCase::StructuralGrowth => transaction_engine(),
        _ => import_document_with_unavailable_lookup_seed(),
    };
    let command = match case {
        DeferredInsertCase::Empty => TypedCommand::InsertText {
            text: String::new(),
        },
        DeferredInsertCase::OverflowingUpperBound => TypedCommand::InsertText { text: "xx".into() },
        _ => TypedCommand::InsertText { text: "x".into() },
    };
    if !matches!(case, DeferredInsertCase::StructuralGrowth) {
        let position = if matches!(case, DeferredInsertCase::LeafBoundary) {
            0
        } else {
            2
        };
        select_text(&mut engine, 65_202, position, position);
    }
    if matches!(case, DeferredInsertCase::MarkMismatch) {
        engine
            .apply_command(
                65_203,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .expect("collapsed mark toggle must update stored marks");
    }
    if matches!(
        case,
        DeferredInsertCase::UnavailableUpperBound | DeferredInsertCase::OverflowingUpperBound
    ) {
        let upper_bound = if matches!(case, DeferredInsertCase::UnavailableUpperBound) {
            usize::MAX
        } else {
            usize::MAX - 1
        };
        let state = engine.derived_state.as_mut().unwrap();
        state.canonical_artifact = state
            .canonical_artifact
            .with_admission_upper_bound_for_test(upper_bound);
        engine.editing_limits.max_derived_output_bytes = usize::MAX;
    }
    if matches!(case, DeferredInsertCase::StrictInteriorEqualMarks) {
        let base = engine
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .admitted_serialized_upper_bound();
        engine.editing_limits.max_derived_output_bytes = base + 1;
    } else if matches!(case, DeferredInsertCase::OneOverOutputLimit) {
        let base = engine
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .admitted_serialized_upper_bound();
        engine.editing_limits.max_derived_output_bytes = base;
    }
    DeferredInsertFixture { engine, command }
}

fn deferred_finalization_fixture() -> (
    YrsDocumentEngine,
    crate::yrs_engine::prepared_admission::DeferredCommandAdmission,
    crate::yrs_engine::prepared_admission::PreparedMutationContext,
    TypedTransaction,
    crate::model::Document,
) {
    let mut engine = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_240, 2, 2);
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            65_241,
            TypedCommand::InsertText { text: "x".into() },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("strict-interior imported insert must produce a transaction")
    };
    let proof = preparation
        .into_inner()
        .expect("strict-interior unavailable-seed insert retains preparation");
    let deferred = match proof.execution_admission {
        crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(deferred) => {
            deferred
        }
        crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(_) => {
            panic!("strict-interior unavailable-seed insert must defer admission")
        }
    };
    let context = engine.prepare_mutation_lookup_seed(65_241).unwrap();
    (engine, deferred, context, transaction, proof.document)
}

fn deferred_tamper_fixture(
    case: &str,
) -> (
    YrsDocumentEngine,
    crate::yrs_engine::prepared_admission::DeferredCommandAdmission,
    crate::yrs_engine::prepared_admission::PreparedMutationContext,
    TypedTransaction,
    crate::model::Document,
) {
    let (engine, mut deferred, context, transaction, expected_document) =
        deferred_finalization_fixture();
    deferred.tamper_for_test(case);
    (engine, deferred, context, transaction, expected_document)
}

struct EagerPreAdmissionErrorCase {
    name: &'static str,
    engine: YrsDocumentEngine,
    request_id: u64,
    command: TypedCommand,
    expected_error: crate::yrs_engine::OperationError,
}

fn eager_pre_admission_error_cases() -> Vec<EagerPreAdmissionErrorCase> {
    let mut output = import_document_with_unavailable_lookup_seed();
    select_text(&mut output, 65_220, 2, 2);
    output.editing_limits.max_derived_output_bytes = 88;

    let mut undo = import_document_with_unavailable_lookup_seed();
    select_text(&mut undo, 65_221, 2, 2);
    undo.editing_limits.max_undo_retained_units = 0;

    let retained_limits = crate::yrs_engine::EditingLimits {
        max_derived_output_bytes: 100,
        ..crate::yrs_engine::EditingLimits::default()
    };
    let mut retained_history = transaction_engine_with_editing_limits(retained_limits);
    retained_history
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(retained_history
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    select_text(&mut retained_history, 65_222, 2, 2);
    let state = retained_history.derived_state.as_mut().unwrap();
    state.canonical_artifact = state
        .canonical_artifact
        .with_admission_upper_bound_for_test(usize::MAX);
    let retained_preparation = std::cell::RefCell::new(None);
    assert!(retained_history
        .plan_command_internal(
            65_232,
            TypedCommand::InsertText { text: "x".into() },
            Some(&retained_preparation),
        )
        .is_ok());
    let retained_proof = retained_preparation.into_inner().unwrap();
    assert_ne!(
        retained_proof
            .execution_admission
            .transaction()
            .history_policy,
        HistoryPolicy::Skip,
    );
    assert_ne!(
        retained_proof.document,
        *retained_history.document().unwrap()
    );
    let retained_history_actual =
        super::history_metadata_bytes(retained_history.stored_marks(), "prosemirror") * 2;

    let command_contract = import_document_with_unavailable_lookup_seed();

    let mut selection = import_document_with_unavailable_lookup_seed();
    let invalid = crate::yrs_engine::ResolvedPoint {
        document: 999,
        scalar: 999,
        utf16: 999,
    };
    selection.derived_state.as_mut().unwrap().resolved_selection =
        crate::yrs_engine::ResolvedSelection::Text {
            anchor: invalid,
            head: invalid,
        };

    vec![
        EagerPreAdmissionErrorCase {
            name: "exact output",
            engine: output,
            request_id: 65_230,
            command: TypedCommand::InsertText { text: "x".into() },
            expected_error: crate::yrs_engine::OperationError::document_limit_exceeded(
                65_230,
                Some(0),
                "maxDerivedOutputBytes",
                88,
                89,
            ),
        },
        EagerPreAdmissionErrorCase {
            name: "undo",
            engine: undo,
            request_id: 65_231,
            command: TypedCommand::InsertText { text: "x".into() },
            expected_error: crate::yrs_engine::OperationError::operation_limit_exceeded(
                65_231,
                Some(0),
                "maxUndoRetainedUnits",
                0,
                1,
            ),
        },
        EagerPreAdmissionErrorCase {
            name: "retained history",
            engine: retained_history,
            request_id: 65_232,
            command: TypedCommand::InsertText { text: "x".into() },
            expected_error: crate::yrs_engine::OperationError::document_limit_exceeded(
                65_232,
                None,
                "maxDerivedOutputBytes",
                100,
                retained_history_actual as u64,
            ),
        },
        EagerPreAdmissionErrorCase {
            name: "command contract",
            engine: command_contract,
            request_id: 65_233,
            command: TypedCommand::ToggleMark {
                mark_type: "missing".into(),
            },
            expected_error: crate::yrs_engine::OperationError::operation_invalid(
                65_233,
                0,
                "mark",
                "unknown mark 'missing'",
            ),
        },
        EagerPreAdmissionErrorCase {
            name: "selection",
            engine: selection,
            request_id: 65_234,
            command: TypedCommand::InsertText { text: "x".into() },
            expected_error: crate::yrs_engine::OperationError::operation_invalid(
                65_234,
                0,
                "command",
                "command simulation failed",
            ),
        },
    ]
}

fn insert_transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
    TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            text: "x".into(),
            marks: vec![],
        }],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    }
}

fn marked_insert_transaction(
    engine: &YrsDocumentEngine,
    request_id: u64,
    text: &str,
) -> TypedTransaction {
    TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            text: text.into(),
            marks: vec![Mark::new("bold".into(), HashMap::new())],
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    }
}

fn import_document_with_unavailable_lookup_seed() -> YrsDocumentEngine {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    engine
}

fn hydrate_import_for_compile_test(engine: &mut YrsDocumentEngine) {
    engine.ensure_mutation_lookup_seed(0).unwrap();
    engine
        .derived_state
        .as_mut()
        .unwrap()
        .materialize_mutation_identity();
}

fn force_lookup_seed_unavailable(engine: &mut YrsDocumentEngine) {
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    let unavailable =
        crate::yrs_engine::mutation::MutationLookupSeed::unavailable_for_validated_import(
            &txn,
            &fragment,
            &state.document,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            engine.yrs_state_epoch,
            engine.revision,
        )
        .with_canonical_artifact(&state.canonical_artifact);
    drop(txn);
    engine.derived_state.as_mut().unwrap().mutation_lookup_seed = Arc::new(unavailable);
}

fn select_text(engine: &mut YrsDocumentEngine, request_id: u64, anchor: u32, head: u32) {
    let point = |offset| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
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
}

#[path = "engine_tests/candidate_cache.rs"]
mod candidate_cache;
#[path = "engine_tests/candidate_publication.rs"]
mod candidate_publication;
#[path = "engine_tests/history_and_commit.rs"]
mod history_and_commit;
#[path = "engine_tests/import_admission.rs"]
mod import_admission;
#[path = "engine_tests/localized_compilation.rs"]
mod localized_compilation;
#[path = "engine_tests/prepared_commands.rs"]
mod prepared_commands;
#[path = "engine_tests/remote_updates.rs"]
mod remote_updates;
