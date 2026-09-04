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
    check_compiled_commit_preparation_stage_for_test, fresh_utf16_doc_excluding_with,
    mark_compiled_commit_durable_write_for_test, reset_encoded_state_reuse_counts_for_test,
    reset_import_receipt_sha256_counts_for_test, reset_import_receipt_state_decodings_for_test,
    reset_import_state_encoding_counts_for_test, reset_prepared_candidate_cache_counts_for_test,
    seal_candidate_state_vector, set_compiled_commit_stage_failpoint_for_test,
    take_compiled_commit_authority_counts_for_test, take_encoded_state_reuse_counts_for_test,
    take_import_receipt_sha256_counts_for_test, take_import_receipt_state_decodings_for_test,
    take_import_state_encoding_counts_for_test, take_prepared_candidate_cache_counts_for_test,
    utf16_doc, CandidateDocument, CompiledCommitPreparationStage, CompiledTransaction,
    EngineDocumentState, OutboundUpdateSink, ValidatedImportDocument, YrsDocumentEngine,
    YrsEngineConfig,
};

#[test]
fn candidate_state_vector_seal_accepts_redundant_inherited_mark_clock_below_bound() {
    let local = ClientID::new(7);
    let remote = ClientID::new(8);
    let base = StateVector::from_iter([(local, 5), (remote, 13)]);
    let actual = StateVector::from_iter([(local, 6), (remote, 13)]);

    assert_eq!(
        seal_candidate_state_vector(1, &base, actual.clone(), local, 3).unwrap(),
        actual
    );
}

#[test]
fn candidate_state_vector_seal_accepts_zero_local_clock_delta() {
    let local = ClientID::new(7);
    let remote = ClientID::new(8);
    let base = StateVector::from_iter([(local, 5), (remote, 13)]);

    assert_eq!(
        seal_candidate_state_vector(1, &base, base.clone(), local, 0).unwrap(),
        base
    );
}

#[test]
fn candidate_state_vector_seal_rejects_authored_clock_bound_excess() {
    let local = ClientID::new(7);
    let base = StateVector::from_iter([(local, 5)]);
    let actual = StateVector::from_iter([(local, 9)]);

    let error = seal_candidate_state_vector(1, &base, actual, local, 3)
        .expect_err("candidate local authorship above the admitted bound must reject");

    assert!(error
        .message
        .contains("exceeded its admitted authored clock bound"));
}

#[test]
fn candidate_state_vector_seal_rejects_local_clock_regression() {
    let local = ClientID::new(7);
    let base = StateVector::from_iter([(local, 5)]);
    let actual = StateVector::from_iter([(local, 4)]);

    let error = seal_candidate_state_vector(1, &base, actual, local, 3)
        .expect_err("candidate local clock regression must reject");

    assert!(error.message.contains("regressed its local authored clock"));
}

#[test]
fn candidate_state_vector_seal_rejects_nonlocal_clock_drift() {
    let local = ClientID::new(7);
    let remote = ClientID::new(8);
    let injected = ClientID::new(9);
    let base = StateVector::from_iter([(local, 5), (remote, 13)]);
    let actual = StateVector::from_iter([(local, 6), (remote, 14), (injected, 1)]);

    let error = seal_candidate_state_vector(1, &base, actual, local, 3)
        .expect_err("candidate nonlocal clock drift must reject");

    assert!(error.message.contains("changed a nonlocal authored clock"));
}

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

#[test]
fn apply_command_runs_one_semantic_compilation() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };
    use crate::yrs_engine::compiler::{
        reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
    };

    let mut engine = transaction_engine();
    reset_semantic_compilation_count_for_test();
    reset_canonical_artifact_counts_for_test();

    let result = engine
        .apply_command(70_001, TypedCommand::InsertText { text: "x".into() })
        .unwrap();

    assert!(result.is_some());
    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));
}

#[test]
fn existing_text_insert_burst_hits_localized_lookup_and_promotes_without_full_rebuild() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    reset_localized_lookup_counts_for_test();

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_101))
        .unwrap();
    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_102))
        .unwrap();

    assert_eq!(take_localized_lookup_counts_for_test(), (0, 2, 2));
}

#[test]
fn prepared_candidate_cache_reuses_one_exact_store_across_successful_insert_burst() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let imported_cache = engine
        .prepared_candidate_cache_store_token_for_test()
        .expect("successful bounded import prepares a candidate cache");
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    reset_prepared_candidate_cache_counts_for_test();

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_103))
        .unwrap();
    assert_prepared_candidate_state_vector_exact(&engine);
    assert_eq!(
        engine.prepared_candidate_cache_store_token_for_test(),
        Some(imported_cache),
        "the exact prepared candidate becomes the next sealed cache"
    );
    let cached_encoded = super::encode_state_bounded(
        &engine.prepared_candidate_cache.as_ref().unwrap().doc,
        &engine.resource_limits,
    )
    .unwrap();
    assert_eq!(cached_encoded, engine.encoded_state().unwrap());
    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_104))
        .unwrap();
    assert_prepared_candidate_state_vector_exact(&engine);

    assert_eq!(
        engine.prepared_candidate_cache_store_token_for_test(),
        Some(imported_cache)
    );
    assert_eq!(take_prepared_candidate_cache_counts_for_test(), (2, 0));
}

#[test]
fn imported_candidate_sealed_state_replaces_only_the_first_commit_full_encode() {
    let mut engine = transaction_engine();
    reset_encoded_state_reuse_counts_for_test();
    reset_import_state_encoding_counts_for_test();

    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 0, 0));
    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_113))
        .unwrap();
    assert_eq!(
        take_encoded_state_reuse_counts_for_test(),
        (0, 0, 1),
        "the import's exact one-shot bytes must replace the first commit-time full encode"
    );

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_114))
        .unwrap();
    assert_eq!(
        take_encoded_state_reuse_counts_for_test(),
        (0, 1, 0),
        "successful mutation caches must not retain the stale import bytes"
    );
}

#[test]
fn validated_html_import_carries_its_first_bounded_encode_into_the_cache() {
    let mut engine = transaction_engine();
    reset_import_state_encoding_counts_for_test();

    engine
        .import_html(
            "<p>abc</p>",
            &FromHtmlOptions::default(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));
    assert_prepared_candidate_state_vector_exact(&engine);
}

#[test]
fn import_cache_eligibility_requires_a_localized_mutation_target() {
    let empty_textblock_engine = transaction_engine();
    let empty_textblock_value = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let empty_textblock_document = from_prosemirror_json(
        &empty_textblock_value,
        &empty_textblock_engine.schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let empty_textblock_source = ValidatedImportDocument::new(
        empty_textblock_document,
        &empty_textblock_engine.schema,
        &empty_textblock_engine.canonical_schema,
        &empty_textblock_engine.resource_limits,
        Some(empty_textblock_value.to_string().len()),
    )
    .unwrap();
    let empty_textblock_candidate = empty_textblock_engine
        .build_candidate_from_document(empty_textblock_source, TransactionOrigin::DocumentImport)
        .unwrap();
    assert!(
        empty_textblock_candidate.import_acceleration_eligible,
        "the collector's trailing empty-textblock gap is a localized target"
    );
    assert!(empty_textblock_candidate
        .import_encoded_state_receipt
        .is_some());

    let mut one_text_target = transaction_engine();
    reset_import_receipt_sha256_counts_for_test();
    one_text_target
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(
        one_text_target.prepared_candidate_cache.is_some(),
        "one localized text target remains eligible"
    );
    assert_eq!(take_import_receipt_sha256_counts_for_test(), (1, 1));

    let mut known_void = transaction_engine();
    reset_import_state_encoding_counts_for_test();
    reset_import_receipt_state_decodings_for_test();
    reset_import_receipt_sha256_counts_for_test();
    known_void
        .import_json(
            r#"{"type":"doc","content":[{"type":"image","attrs":{"src":"asset://one"}}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_eq!(
        take_import_state_encoding_counts_for_test(),
        (1, 0),
        "candidate admission still performs its one mandatory bounded encode"
    );
    assert_eq!(take_import_receipt_state_decodings_for_test(), 0);
    assert_eq!(take_import_receipt_sha256_counts_for_test(), (0, 0));
    assert!(
        known_void.prepared_candidate_cache.is_none(),
        "a textless void-only document has no localized target to accelerate"
    );
    assert_eq!(
        known_void.document_json().unwrap(),
        json!({
            "type": "doc",
            "content": [{
                "type": "image",
                "attrs": { "src": "asset://one" }
            }]
        })
    );

    for (name, value) in [
        (
            "mixedTextOpaque",
            json!({
                "type": "doc",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "addressable" }]
                    },
                    {
                        "type": "customOpaqueBlock",
                        "attrs": { "payload": "retained" }
                    }
                ]
            }),
        ),
        (
            "article",
            json!({
                "type": "doc",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "Title" }]
                    },
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "Body" }]
                    }
                ]
            }),
        ),
    ] {
        let mut engine = transaction_engine();
        engine
            .import_json(&value.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        assert!(
            engine.prepared_candidate_cache.is_some(),
            "{name} must retain import acceleration"
        );
        assert_eq!(engine.document_json().unwrap(), value, "{name}");
    }
}

#[test]
fn deferred_import_still_obeys_exact_candidate_encoded_state_ceiling() {
    fn validated_opaque_source(
        engine: &YrsDocumentEngine,
        value: &serde_json::Value,
    ) -> ValidatedImportDocument {
        let document =
            from_prosemirror_json(value, &engine.schema, UnknownTypeMode::Preserve).unwrap();
        ValidatedImportDocument::new(
            document,
            &engine.schema,
            &engine.canonical_schema,
            &engine.resource_limits,
            Some(value.to_string().len()),
        )
        .unwrap()
    }

    let value = json!({
        "type": "doc",
        "content": [{
            "type": "benchmarkOpaqueBlock",
            "attrs": { "payload": "opaque" }
        }]
    });
    let probe = transaction_engine();
    let candidate = probe
        .build_candidate_from_document(
            validated_opaque_source(&probe, &value),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(!candidate.import_acceleration_eligible);
    assert!(candidate.import_encoded_state_receipt.is_none());
    let encoded_len = super::encode_state_bounded(&candidate.doc, &probe.resource_limits)
        .unwrap()
        .len();
    let exact_doc = super::equivalent_private_candidate_doc(&candidate.doc);
    let one_under_doc = super::equivalent_private_candidate_doc(&candidate.doc);

    let mut exact = transaction_engine();
    exact.resource_limits = ResourceLimits {
        max_encoded_state_bytes: encoded_len,
        ..exact.resource_limits.clone()
    };
    reset_import_state_encoding_counts_for_test();
    let exact_candidate = exact
        .build_candidate_from_document_in_doc(
            validated_opaque_source(&exact, &value),
            TransactionOrigin::DocumentImport,
            exact_doc,
        )
        .expect("the exact authoritative candidate byte ceiling must admit");
    assert!(!exact_candidate.import_acceleration_eligible);
    assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));

    let mut one_under = transaction_engine();
    one_under.resource_limits = ResourceLimits {
        max_encoded_state_bytes: encoded_len - 1,
        ..one_under.resource_limits.clone()
    };
    reset_import_state_encoding_counts_for_test();
    let error = match one_under.build_candidate_from_document_in_doc(
        validated_opaque_source(&one_under, &value),
        TransactionOrigin::DocumentImport,
        one_under_doc,
    ) {
        Ok(_) => panic!("one under the authoritative candidate bytes must reject"),
        Err(error) => error,
    };
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(encoded_len - 1));
    assert_eq!(error.actual, Some(encoded_len));
    assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));
}

#[test]
fn opaque_only_import_defers_replica_then_first_structural_mutation_bootstraps() {
    let opaque = json!({
        "type": "doc",
        "content": [{
            "type": "benchmarkOpaqueBlock",
            "attrs": { "payload": "x".repeat(32 * 1024) }
        }]
    });
    let mut engine = transaction_engine();
    reset_import_state_encoding_counts_for_test();
    reset_import_receipt_state_decodings_for_test();
    reset_import_receipt_sha256_counts_for_test();

    engine
        .import_json(&opaque.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();

    assert_eq!(
        take_import_state_encoding_counts_for_test(),
        (1, 0),
        "candidate admission still performs its one mandatory bounded encode"
    );
    assert_eq!(take_import_receipt_state_decodings_for_test(), 0);
    assert_eq!(take_import_receipt_sha256_counts_for_test(), (0, 0));
    assert!(engine.prepared_candidate_cache.is_none());
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    assert_eq!(engine.document_json().unwrap(), opaque);

    reset_prepared_candidate_cache_counts_for_test();
    reset_encoded_state_reuse_counts_for_test();
    engine
        .apply_typed_transaction(paragraph_insert_transaction(&engine, 70_115))
        .unwrap();

    assert_eq!(take_prepared_candidate_cache_counts_for_test(), (0, 1));
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
    assert!(engine.prepared_candidate_cache.is_some());
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    let json = engine.document_json().unwrap();
    assert_eq!(json["content"][0], opaque["content"][0]);
    assert_eq!(json["content"][1]["type"], "paragraph");
}

#[test]
fn validated_import_commit_does_not_recompute_schema_fingerprint() {
    use crate::schema::{
        reset_schema_fingerprint_count_for_test, take_schema_fingerprint_count_for_test,
    };

    let mut engine = transaction_engine();
    let candidate = validated_json_import_candidate(&engine);
    reset_schema_fingerprint_count_for_test();

    engine
        .commit_candidate(candidate, TransactionOrigin::DocumentImport)
        .unwrap();

    let total_fingerprints = take_schema_fingerprint_count_for_test();
    assert_eq!(
        total_fingerprints, 1,
        "the test-only render-cache slow invariant remains the sole fingerprint call"
    );
    assert_eq!(
        total_fingerprints.saturating_sub(1),
        0,
        "the exact immutable schema and canonical-artifact seals make commit-time hashing redundant"
    );
}

#[test]
fn import_lookup_schema_seal_drift_falls_back_exactly_once() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    for case in [
        "schemaToken",
        "currentSchemaFingerprint",
        "equalDistinctSchemaPointer",
    ] {
        let engine = transaction_engine();
        let mut candidate = validated_json_import_candidate(&engine);
        let (source_document, canonical_artifact) = match &candidate.state {
            EngineDocumentState::Ready {
                document,
                canonical_artifact,
            } => (document.clone(), canonical_artifact.clone()),
            EngineDocumentState::AwaitingRemote => {
                panic!("validated import candidate must be ready")
            }
        };
        let mut receipt = candidate
            .import_encoded_state_receipt
            .take()
            .expect("validated import candidate carries its lookup receipt");
        if case == "schemaToken" {
            receipt
                .lookup_materialization
                .as_mut()
                .unwrap()
                .schema_token ^= 1;
        }
        let equal_schema = engine.schema.clone();
        let schema = if case == "equalDistinctSchemaPointer" {
            &equal_schema
        } else {
            &engine.schema
        };
        let drifted_schema_fingerprint = format!("{}-drifted", engine.schema_fingerprint);
        let schema_fingerprint = if case == "currentSchemaFingerprint" {
            drifted_schema_fingerprint.as_str()
        } else {
            engine.schema_fingerprint.as_str()
        };

        reset_localized_lookup_counts_for_test();
        let fused = receipt.take_matching_lookup_materialization(
            &candidate.doc,
            &engine.fragment_name,
            &source_document,
            &canonical_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            schema,
            schema_fingerprint,
            1,
            1,
        );
        assert!(fused.is_none(), "{case}");

        let txn = candidate.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        crate::yrs_engine::mutation::MutationLookupSeed::build(
            0,
            &txn,
            &fragment,
            schema,
            &source_document,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            schema_fingerprint,
            1,
            1,
        )
        .unwrap();
        assert_eq!(take_localized_lookup_counts_for_test().0, 1, "{case}");
    }
}

fn validated_json_import_candidate(engine: &YrsDocumentEngine) -> CandidateDocument {
    let value = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "abc"}]
        }]
    });
    let document =
        from_prosemirror_json(&value, &engine.schema, UnknownTypeMode::Preserve).unwrap();
    let source = ValidatedImportDocument::new(
        document,
        &engine.schema,
        &engine.canonical_schema,
        &engine.resource_limits,
        Some(serde_json::to_vec(&value).unwrap().len()),
    )
    .unwrap();
    engine
        .build_candidate_from_document(source, TransactionOrigin::DocumentImport)
        .unwrap()
}

fn equal_clock_divergent_valid_update(
    engine: &YrsDocumentEngine,
    candidate: &CandidateDocument,
) -> Vec<u8> {
    let divergent = super::equivalent_private_candidate_doc(&candidate.doc);
    let empty_json = json!({
        "type": engine.schema.doc_node_type(),
        "content": [],
    });
    let divergent_json = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "xyz"}]
        }]
    });
    let codec = super::YrsDocumentCodec::new(&engine.schema, &engine.resource_limits);
    {
        let mut txn =
            divergent.transact_mut_with(TransactionOrigin::DocumentImport.as_yrs_origin());
        let fragment = txn.get_or_insert_xml_fragment(engine.fragment_name.as_str());
        codec
            .apply_json(&fragment, &mut txn, &empty_json, &divergent_json)
            .unwrap();
    }
    let candidate_txn = candidate.doc.transact();
    let divergent_txn = divergent.transact();
    assert_eq!(
        divergent_txn.state_vector(),
        candidate_txn.state_vector(),
        "the tamper must keep identical client clocks"
    );
    let candidate_encoded = candidate_txn.encode_state_as_update_v1(&StateVector::default());
    let divergent_encoded = divergent_txn.encode_state_as_update_v1(&StateVector::default());
    assert_ne!(
        divergent_encoded, candidate_encoded,
        "the tamper must carry different valid content"
    );
    divergent_encoded
}

#[test]
fn tampered_import_encoded_state_receipt_falls_back_to_one_cache_encode() {
    for case in [
        "bytes",
        "sha256",
        "stateVector",
        "fragment",
        "clientId",
        "guid",
        "offsetKind",
        "skipGc",
        "deleteSetEligibility",
        "lookupSourceDocument",
        "lookupCanonicalArtifact",
        "lookupResourceLimits",
        "lookupEditingLimits",
        "lookupMaxLength",
        "lookupSchemaToken",
        "lookupStoreToken",
    ] {
        let lookup_only_tamper = case.starts_with("lookup");
        let mut engine = transaction_engine();
        let mut candidate = validated_json_import_candidate(&engine);
        let installed = engine.derived_state.as_ref().unwrap();
        let foreign_document = installed.document.clone();
        let foreign_artifact = installed.canonical_artifact.clone();
        let receipt = candidate
            .import_encoded_state_receipt
            .as_mut()
            .expect("validated JSON candidates carry one private encoded-state receipt");
        match case {
            "bytes" => receipt.encoded_state = vec![0xff],
            "sha256" => receipt.encoded_state_sha256[0] ^= 1,
            "stateVector" => receipt.state_vector = StateVector::default(),
            "fragment" => receipt.fragment_id = BranchID::Root(Arc::from("foreign")),
            "clientId" => receipt.client_id = ClientID::new(receipt.client_id.get() ^ 1),
            "guid" => receipt.guid = Arc::from("foreign-guid"),
            "offsetKind" => receipt.offset_kind = OffsetKind::Bytes,
            "skipGc" => receipt.skip_gc = !receipt.skip_gc,
            "deleteSetEligibility" => receipt.delete_set_is_empty = !receipt.delete_set_is_empty,
            "lookupSourceDocument" => {
                receipt
                    .lookup_materialization
                    .as_mut()
                    .unwrap()
                    .source_document = foreign_document
            }
            "lookupCanonicalArtifact" => {
                receipt
                    .lookup_materialization
                    .as_mut()
                    .unwrap()
                    .canonical_artifact = foreign_artifact
            }
            "lookupResourceLimits" => {
                receipt
                    .lookup_materialization
                    .as_mut()
                    .unwrap()
                    .resource_limits
                    .max_document_nodes ^= 1
            }
            "lookupEditingLimits" => {
                receipt
                    .lookup_materialization
                    .as_mut()
                    .unwrap()
                    .editing_limits
                    .max_operations_per_transaction ^= 1
            }
            "lookupMaxLength" => {
                receipt.lookup_materialization.as_mut().unwrap().max_length = Some(1)
            }
            "lookupSchemaToken" => {
                receipt
                    .lookup_materialization
                    .as_mut()
                    .unwrap()
                    .schema_token ^= 1
            }
            "lookupStoreToken" => receipt.lookup_materialization.as_mut().unwrap().store_token ^= 1,
            _ => unreachable!(),
        }
        reset_import_state_encoding_counts_for_test();
        crate::yrs_engine::mutation::reset_localized_lookup_counts_for_test();

        engine
            .commit_candidate(candidate, TransactionOrigin::DocumentImport)
            .unwrap();

        assert_eq!(
            take_import_state_encoding_counts_for_test(),
            if lookup_only_tamper { (0, 0) } else { (0, 1) },
            "{case}"
        );
        assert_eq!(
            crate::yrs_engine::mutation::take_localized_lookup_counts_for_test().0,
            1,
            "{case}"
        );
        assert_prepared_candidate_state_vector_exact(&engine);
        assert_eq!(
            engine
                .prepared_candidate_cache
                .as_ref()
                .unwrap()
                .encoded_state_seal
                .as_ref()
                .unwrap()
                .encoded_state,
            super::encode_state_bounded(&engine.doc, &engine.resource_limits).unwrap(),
            "{case}"
        );
    }
}

#[test]
fn equal_clock_divergent_valid_receipt_bytes_fall_back_to_authoritative_state() {
    let mut engine = transaction_engine();
    let mut candidate = validated_json_import_candidate(&engine);
    let divergent_encoded = equal_clock_divergent_valid_update(&engine, &candidate);
    candidate
        .import_encoded_state_receipt
        .as_mut()
        .unwrap()
        .encoded_state = divergent_encoded.clone();
    reset_import_state_encoding_counts_for_test();

    engine
        .commit_candidate(candidate, TransactionOrigin::DocumentImport)
        .unwrap();

    assert_eq!(take_import_state_encoding_counts_for_test(), (0, 1));
    assert_prepared_candidate_state_vector_exact(&engine);
    let sealed = &engine
        .prepared_candidate_cache
        .as_ref()
        .unwrap()
        .encoded_state_seal
        .as_ref()
        .unwrap()
        .encoded_state;
    assert_eq!(
        sealed,
        &super::encode_state_bounded(&engine.doc, &engine.resource_limits).unwrap()
    );
    assert_ne!(sealed, &divergent_encoded);
}

#[test]
fn oversized_receipt_falls_back_before_standard_update_decode() {
    let engine = transaction_engine();
    let mut candidate = validated_json_import_candidate(&engine);
    let mut receipt = candidate.import_encoded_state_receipt.take().unwrap();
    let limit = receipt.encoded_state.len().checked_mul(2).unwrap();
    receipt.encoded_state = vec![0xff; limit + 1];
    receipt.encoded_state_sha256 = sha2::Sha256::digest(&receipt.encoded_state).into();
    reset_import_state_encoding_counts_for_test();
    reset_import_receipt_state_decodings_for_test();

    let cache = super::prepare_import_candidate_cache(
        &candidate.doc,
        &engine.fragment_name,
        &ResourceLimits {
            max_encoded_state_bytes: limit,
            ..engine.resource_limits.clone()
        },
        Some(receipt),
        None,
        1,
        1,
    );

    assert!(cache.is_some());
    assert_eq!(take_import_state_encoding_counts_for_test(), (0, 1));
    assert_eq!(take_import_receipt_state_decodings_for_test(), 0);
}

#[test]
fn import_receipt_obeys_exact_retained_and_two_x_candidate_boundaries() {
    let prepare_at = |boundary: &str| {
        let engine = transaction_engine();
        let mut candidate = validated_json_import_candidate(&engine);
        let receipt = candidate.import_encoded_state_receipt.take().unwrap();
        let len = receipt.encoded_state.len();
        let retained =
            super::retained_import_state_charge(len, receipt.encoded_state.capacity()).unwrap();
        let limit = match boundary {
            "retained" => retained,
            "oneUnderRetained" => retained - 1,
            "twoX" => len.checked_mul(2).unwrap(),
            _ => unreachable!(),
        };
        reset_import_state_encoding_counts_for_test();
        let cache = super::prepare_import_candidate_cache(
            &candidate.doc,
            &engine.fragment_name,
            &ResourceLimits {
                max_encoded_state_bytes: limit,
                ..engine.resource_limits.clone()
            },
            Some(receipt),
            None,
            1,
            1,
        );
        assert_eq!(take_import_state_encoding_counts_for_test(), (0, 0));
        cache
    };
    assert!(prepare_at("retained").unwrap().encoded_state_seal.is_some());
    assert!(prepare_at("oneUnderRetained")
        .unwrap()
        .encoded_state_seal
        .is_none());
    assert!(prepare_at("twoX").unwrap().encoded_state_seal.is_none());
}

#[test]
fn import_encoded_state_seal_obeys_exact_retained_charge_without_dropping_two_x_cache() {
    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let encoded = super::encode_state_bounded(&source.doc, &source.resource_limits).unwrap();
    let encoded_len = encoded.len();
    let encoded_capacity = encoded.capacity();
    let exact_retained_charge =
        super::retained_import_state_charge(encoded_len, encoded_capacity).unwrap();

    let exact_cache = super::prepare_import_candidate_cache(
        &source.doc,
        &source.fragment_name,
        &ResourceLimits {
            max_encoded_state_bytes: exact_retained_charge,
            ..source.resource_limits.clone()
        },
        None,
        None,
        source.revision,
        source.yrs_state_epoch,
    )
    .expect("the exact retained charge retains the private candidate");
    let exact_seal = exact_cache.encoded_state_seal.as_ref().unwrap();
    assert_eq!(exact_seal.encoded_state.len(), encoded_len);
    assert_eq!(exact_seal.encoded_state.capacity(), encoded_capacity);

    let one_under_cache = super::prepare_import_candidate_cache(
        &source.doc,
        &source.fragment_name,
        &ResourceLimits {
            max_encoded_state_bytes: exact_retained_charge - 1,
            ..source.resource_limits.clone()
        },
        None,
        None,
        source.revision,
        source.yrs_state_epoch,
    )
    .expect("a document above one third but within the 2x ceiling retains its candidate");
    assert!(one_under_cache.encoded_state_seal.is_none());

    let exact_two_x_cache = super::prepare_import_candidate_cache(
        &source.doc,
        &source.fragment_name,
        &ResourceLimits {
            max_encoded_state_bytes: encoded_len.checked_mul(2).unwrap(),
            ..source.resource_limits.clone()
        },
        None,
        None,
        source.revision,
        source.yrs_state_epoch,
    )
    .expect("the existing exact 2x candidate admission remains unchanged");
    assert!(exact_two_x_cache.encoded_state_seal.is_none());
}

fn assert_next_insert_uses_full_current_state_encode(
    engine: &mut YrsDocumentEngine,
    request_id: u64,
) {
    reset_encoded_state_reuse_counts_for_test();
    engine
        .apply_typed_transaction(insert_transaction(engine, request_id))
        .unwrap();
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
}

fn imported_engine_with_sealed_state() -> YrsDocumentEngine {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(engine
        .prepared_candidate_cache
        .as_ref()
        .and_then(|cache| cache.encoded_state_seal.as_ref())
        .is_some());
    engine
}

#[test]
fn sealed_state_vector_drift_falls_back() {
    let mut engine = imported_engine_with_sealed_state();
    let compiled = engine
        .compile_typed_transaction(insert_transaction(&engine, 70_115))
        .unwrap();
    let live_doc = engine.doc.clone();
    let live_txn = live_doc.transact();
    let live_fragment = live_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    let exact_state_vector = live_txn.state_vector();
    engine
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .state_vector = StateVector::default();
    let reused = engine
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .take_matching_encoded_state(
            &live_doc,
            &live_fragment,
            &compiled.mutation_plan,
            engine.revision,
            engine.yrs_state_epoch,
            engine.resource_limits.max_encoded_state_bytes,
        );
    assert!(reused.is_none());
    engine
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .state_vector = exact_state_vector;
    drop(live_txn);

    reset_encoded_state_reuse_counts_for_test();
    engine.apply_compiled_transaction(compiled, true).unwrap();
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
}

#[test]
fn import_with_nonempty_delete_set_retains_candidate_without_sealed_bytes() {
    let mut source = imported_engine_with_sealed_state();
    let from = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let to = RevisionedPosition { offset: 2, ..from };
    source
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_116,
            base_document_revision: source.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::DeleteRange {
                range: RevisionedRange { from, to },
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(!source.doc.transact().snapshot().delete_set.is_empty());

    let cache = super::prepare_import_candidate_cache(
        &source.doc,
        &source.fragment_name,
        &source.resource_limits,
        None,
        None,
        source.revision,
        source.yrs_state_epoch,
    )
    .expect("the existing 2x private candidate remains available");
    assert!(cache.encoded_state_seal.is_none());
}

#[test]
fn sealed_state_fragment_options_revision_and_epoch_drift_fall_back() {
    let mut stale_fragment = imported_engine_with_sealed_state();
    stale_fragment
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .encoded_state_seal
        .as_mut()
        .unwrap()
        .fragment_id = BranchID::Root(Arc::from("other"));
    assert_next_insert_uses_full_current_state_encode(&mut stale_fragment, 70_118);

    let mut stale_options = imported_engine_with_sealed_state();
    let seal = stale_options
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .encoded_state_seal
        .as_mut()
        .unwrap();
    seal.offset_kind = match seal.offset_kind {
        OffsetKind::Bytes => OffsetKind::Utf16,
        OffsetKind::Utf16 => OffsetKind::Bytes,
    };
    assert_next_insert_uses_full_current_state_encode(&mut stale_options, 70_119);

    let mut stale_revision = imported_engine_with_sealed_state();
    stale_revision
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .encoded_state_seal
        .as_mut()
        .unwrap()
        .document_revision = stale_revision.revision.saturating_add(1);
    assert_next_insert_uses_full_current_state_encode(&mut stale_revision, 70_120);

    let mut stale_epoch = imported_engine_with_sealed_state();
    stale_epoch
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .encoded_state_seal
        .as_mut()
        .unwrap()
        .yrs_state_epoch = stale_epoch.yrs_state_epoch.saturating_add(1);
    assert_next_insert_uses_full_current_state_encode(&mut stale_epoch, 70_121);
}

#[test]
fn sealed_state_rechecks_current_limit_and_survives_selection_only_state_change() {
    let mut limit_drift = transaction_engine();
    let large_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "a".repeat(2_048)}]
        }]
    })
    .to_string();
    limit_drift
        .import_json(&large_source, TransactionOrigin::DocumentImport)
        .unwrap();
    let retained_len = limit_drift
        .prepared_candidate_cache
        .as_ref()
        .unwrap()
        .encoded_state_seal
        .as_ref()
        .unwrap()
        .encoded_state
        .len();
    limit_drift.resource_limits.max_encoded_state_bytes = retained_len.checked_mul(3).unwrap() - 1;
    assert_next_insert_uses_full_current_state_encode(&mut limit_drift, 70_122);

    let mut selection_only = imported_engine_with_sealed_state();
    let document_revision = selection_only.revision;
    let yrs_state_epoch = selection_only.yrs_state_epoch;
    select_text(&mut selection_only, 70_123, 1, 3);
    assert_eq!(selection_only.revision, document_revision);
    assert_eq!(selection_only.yrs_state_epoch, yrs_state_epoch);
    assert!(selection_only
        .prepared_candidate_cache
        .as_ref()
        .unwrap()
        .encoded_state_seal
        .is_some());
    reset_encoded_state_reuse_counts_for_test();
    selection_only
        .apply_typed_transaction(insert_transaction(&selection_only, 70_124))
        .unwrap();
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 0, 1));
}

#[test]
fn sealed_state_bytes_match_stock_oracle_with_history_undo_redo_parity() {
    let mut optimized = imported_engine_with_sealed_state();
    let mut stock = imported_engine_with_sealed_state();
    stock
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .encoded_state_seal = None;
    let stock_current =
        super::encode_state_bounded(&optimized.doc, &optimized.resource_limits).unwrap();
    assert_eq!(
        optimized
            .prepared_candidate_cache
            .as_ref()
            .unwrap()
            .encoded_state_seal
            .as_ref()
            .unwrap()
            .encoded_state
            .as_slice(),
        stock_current.as_slice()
    );

    optimized
        .apply_typed_transaction(insert_transaction(&optimized, 70_125))
        .unwrap();
    stock
        .apply_typed_transaction(insert_transaction(&stock, 70_125))
        .unwrap();
    assert_eq!(optimized.document_json(), stock.document_json());
    assert_eq!(optimized.can_undo(), stock.can_undo());
    assert_eq!(optimized.can_redo(), stock.can_redo());

    optimized.undo(70_126).unwrap();
    stock.undo(70_126).unwrap();
    assert_eq!(optimized.document_json(), stock.document_json());
    assert_eq!(optimized.can_redo(), stock.can_redo());

    optimized.redo(70_127).unwrap();
    stock.redo(70_127).unwrap();
    assert_eq!(optimized.document_json(), stock.document_json());
    assert_eq!(optimized.can_undo(), stock.can_undo());
}

#[test]
fn prepared_candidate_seals_actual_clock_for_redundant_inherited_mark_insert() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let local_client = engine.doc.client_id();

    let first = engine
        .compile_typed_transaction(marked_insert_transaction(&engine, 70_109, "a"))
        .unwrap();
    assert_eq!(first.authored_clock_units, 3);
    let before_first = engine.doc.transact().state_vector().get(&local_client);
    engine.apply_compiled_transaction(first, true).unwrap();
    let after_first = engine.doc.transact().state_vector().get(&local_client);
    assert_eq!(after_first - before_first, 3);

    let second = engine
        .compile_typed_transaction(marked_insert_transaction(&engine, 70_110, "b"))
        .unwrap();
    assert_eq!(second.authored_clock_units, 3);
    let before_second = engine.doc.transact().state_vector().get(&local_client);
    engine.apply_compiled_transaction(second, true).unwrap();
    let after_second = engine.doc.transact().state_vector().get(&local_client);

    assert_eq!(after_second - before_second, 1);
    assert_prepared_candidate_state_vector_exact(&engine);
}

#[test]
fn prepared_candidate_bounds_inherited_format_suspension_at_text_boundaries() {
    struct Case {
        name: &'static str,
        source: &'static str,
        offset: u32,
        inserted: &'static str,
        marks: Vec<Mark>,
        expected_bound: u64,
    }

    let bold = || Mark::new("bold".into(), HashMap::new());
    let italic = || Mark::new("italic".into(), HashMap::new());
    let cases = [
        Case {
            name: "plain at start",
            source: "ab",
            offset: 0,
            inserted: "x",
            marks: vec![],
            expected_bound: 3,
        },
        Case {
            name: "plain inside",
            source: "ab",
            offset: 1,
            inserted: "x",
            marks: vec![],
            expected_bound: 3,
        },
        Case {
            name: "plain at end",
            source: "ab",
            offset: 2,
            inserted: "x",
            marks: vec![],
            expected_bound: 3,
        },
        Case {
            name: "same mark inside",
            source: "ab",
            offset: 1,
            inserted: "x",
            marks: vec![bold()],
            expected_bound: 3,
        },
        Case {
            name: "different mark inside",
            source: "ab",
            offset: 1,
            inserted: "x",
            marks: vec![italic()],
            expected_bound: 5,
        },
        Case {
            name: "plain unicode inside",
            source: "😀b",
            offset: 1,
            inserted: "🦀",
            marks: vec![],
            expected_bound: 4,
        },
    ];

    for (index, case) in cases.into_iter().enumerate() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                &serde_json::json!({
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{
                            "type": "text",
                            "text": case.source,
                            "marks": [{ "type": "bold" }]
                        }]
                    }]
                })
                .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let request_id = 70_120 + u64::try_from(index).unwrap();
        let compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: case.offset,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: case.inserted.into(),
                    marks: case.marks,
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert_eq!(
            compiled.authored_clock_units, case.expected_bound,
            "{}",
            case.name
        );
        let local_client = engine.doc.client_id();
        let before = engine.doc.transact().state_vector().get(&local_client);
        engine.apply_compiled_transaction(compiled, true).unwrap();
        let after = engine.doc.transact().state_vector().get(&local_client);
        assert!(
            u64::from(after - before) <= case.expected_bound,
            "{}",
            case.name
        );
        assert_prepared_candidate_state_vector_exact(&engine);
    }

    let mut boundary = transaction_engine();
    boundary
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a","marks":[{"type":"bold"}]},{"type":"text","text":"b","marks":[{"type":"italic"}]}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let compiled = boundary
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_126,
            base_document_revision: boundary.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    // The lowering selects one exact storage target at this semantic
    // boundary; only that target's touching bold run contributes.
    assert_eq!(compiled.authored_clock_units, 3);
    boundary.apply_compiled_transaction(compiled, true).unwrap();
    assert_prepared_candidate_state_vector_exact(&boundary);

    let mut delete_then_insert = transaction_engine();
    delete_then_insert
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab","marks":[{"type":"bold"}]}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let compiled = delete_then_insert
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_127,
            base_document_revision: delete_then_insert.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![
                TypedOperation::DeleteRange {
                    range: RevisionedRange {
                        from: RevisionedPosition {
                            offset: 0,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        to: RevisionedPosition {
                            offset: 2,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::Before,
                        },
                    },
                },
                TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 0,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                },
            ],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert_eq!(compiled.authored_clock_units, 3);
    delete_then_insert
        .apply_compiled_transaction(compiled, true)
        .unwrap();
    assert_prepared_candidate_state_vector_exact(&delete_then_insert);
}

#[test]
fn prepared_candidate_cache_failure_is_private_atomic_and_falls_back_once() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before = atomic_audit(&engine);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));
    reset_encoded_state_reuse_counts_for_test();

    let error = engine
        .apply_typed_transaction(insert_transaction(&engine, 70_105))
        .expect_err("candidate encoding failpoint must reject before the live write");

    set_compiled_commit_stage_failpoint_for_test(None);
    assert!(error.message.contains("historyUpdateEncoding"));
    assert_eq!(atomic_audit(&engine), before);
    assert!(engine.prepared_candidate_cache.is_none());
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 0, 1));
    reset_prepared_candidate_cache_counts_for_test();
    reset_encoded_state_reuse_counts_for_test();

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_106))
        .unwrap();

    assert!(engine.prepared_candidate_cache.is_some());
    assert_eq!(take_prepared_candidate_cache_counts_for_test(), (0, 1));
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
}

#[test]
fn prepared_candidate_cache_revalidates_stale_revision_seal_before_reuse() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .document_revision = engine.revision.saturating_add(1);
    reset_prepared_candidate_cache_counts_for_test();
    reset_localized_lookup_counts_for_test();

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_108))
        .unwrap();
    let cache_counts = take_prepared_candidate_cache_counts_for_test();
    let lookup_counts = take_localized_lookup_counts_for_test();
    let cached_encoded = super::encode_state_bounded(
        &engine.prepared_candidate_cache.as_ref().unwrap().doc,
        &engine.resource_limits,
    )
    .unwrap();

    assert_eq!(cache_counts, (0, 1));
    assert_eq!(lookup_counts, (1, 1, 1));
    assert_eq!(
        engine.document_json().unwrap()["content"][0]["content"][0]["text"],
        "axbc"
    );
    assert_eq!(cached_encoded, engine.encoded_state().unwrap());
}

#[test]
fn imported_candidate_cache_supplies_first_staged_lookup_without_live_rebuild() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

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
    reset_localized_lookup_counts_for_test();

    engine
        .apply_command(70_107, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
}

#[test]
fn validated_import_materializes_ready_lookup_without_a_second_tree_scan() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_localized_lookup_counts_for_test();

    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    assert_eq!(
        take_localized_lookup_counts_for_test(),
        (0, 0, 0),
        "validated codec traversal must carry the exact ready lookup payload"
    );
    assert!(engine
        .prepared_candidate_cache
        .as_ref()
        .and_then(|cache| cache.staged_lookup_seed.as_ref())
        .is_some());
}

#[test]
fn validated_import_lookup_materialization_matches_the_ordinary_builder() {
    let inputs = [
        r#"{"type":"doc","content":[{"type":"heading","attrs":{"level":2}}]}"#,
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"plain"},{"type":"text","text":" bold","marks":[{"type":"bold"}]},{"type":"text","text":" 🦀"}]}]}"#,
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"hardBreak"},{"type":"text","text":"middle"},{"type":"hardBreak"},{"type":"hardBreak"}]}]}"#,
        r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"nested"}]}]},{"type":"horizontal_rule"},{"type":"mystery_widget","attrs":{"payload":{"x":[1,true,"v"]}}}]}"#,
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"},{"type":"text","text":"b","marks":[{"type":"italic"}]},{"type":"text","text":"c"}]}]}"#,
    ];

    for input in inputs {
        let mut engine = transaction_engine();
        engine
            .import_json(input, TransactionOrigin::DocumentImport)
            .unwrap();
        let staged = engine
            .prepared_candidate_cache
            .as_ref()
            .and_then(|cache| cache.staged_lookup_seed.as_ref())
            .unwrap_or_else(|| panic!("validated import carries the fused ready seed: {input}"));
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        assert!(
            crate::yrs_engine::mutation::lookup_payload_legacy_parity_for_test(
                &txn,
                &fragment,
                &engine.schema,
            ),
            "{input}"
        );
        let ordinary = crate::yrs_engine::mutation::MutationLookupSeed::build(
            77_001,
            &txn,
            &fragment,
            &engine.schema,
            &state.document,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            engine.yrs_state_epoch,
            engine.revision,
        )
        .unwrap();
        assert!(staged.has_same_ready_payload_for_test(&ordinary), "{input}");
    }
}

#[test]
fn lookup_materialization_matches_legacy_for_nested_fragment_and_empty_text_storage() {
    let engine = transaction_engine();
    let doc = utf16_doc();
    let mut txn = doc.transact_mut();
    let fragment = txn.get_or_insert_xml_fragment("content");
    let nested = XmlFragmentPrelim::new::<_, XmlIn>([
        XmlIn::from(XmlTextPrelim::new("")),
        XmlIn::from(XmlTextPrelim::new("x")),
    ]);
    fragment.insert(&mut txn, 0, XmlIn::from(nested));
    drop(txn);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("content").unwrap();

    assert!(
        crate::yrs_engine::mutation::lookup_payload_legacy_parity_for_test(
            &txn,
            &fragment,
            &engine.schema,
        )
    );
}

#[test]
fn import_lookup_materialization_failpoints_are_opportunistic_and_fallback_once() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, set_lookup_seed_hydration_failpoint_for_test,
        take_localized_lookup_counts_for_test, LookupSeedHydrationFailpoint,
    };

    for failpoint in [
        LookupSeedHydrationFailpoint::InitialReservation,
        LookupSeedHydrationFailpoint::MapGrowth,
        LookupSeedHydrationFailpoint::MapPublication,
        LookupSeedHydrationFailpoint::BindingPublication,
        LookupSeedHydrationFailpoint::SeedPublication,
    ] {
        let mut engine = transaction_engine();
        reset_localized_lookup_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        set_lookup_seed_hydration_failpoint_for_test(None);
        assert_eq!(
            take_localized_lookup_counts_for_test().0,
            1,
            "{failpoint:?}"
        );

        reset_localized_lookup_counts_for_test();
        engine
            .apply_typed_transaction(insert_transaction(&engine, 77_100))
            .unwrap();
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["content"][0]["text"],
            "axbc",
            "{failpoint:?}"
        );
        assert_prepared_candidate_state_vector_exact(&engine);
    }
}

#[test]
fn ordinary_lookup_collection_fails_fast_while_codec_projection_finishes() {
    use crate::yrs_engine::mutation::{
        reset_import_lookup_event_count_for_test, set_lookup_seed_hydration_failpoint_for_test,
        take_import_lookup_event_count_for_test, LookupSeedHydrationFailpoint,
    };

    let value = json!({
        "type": "doc",
        "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "first"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "second"}]}
        ]
    });
    let mut engine = transaction_engine();
    engine
        .import_json(&value.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    reset_import_lookup_event_count_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(LookupSeedHydrationFailpoint::MapGrowth));
    let error = crate::yrs_engine::mutation::MutationLookupSeed::build(
        77_200,
        &txn,
        &fragment,
        &engine.schema,
        &state.document,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        engine.yrs_state_epoch,
        engine.revision,
    )
    .unwrap_err();
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(take_import_lookup_event_count_for_test(), 2);
    drop(txn);

    let document =
        from_prosemirror_json(&value, &engine.schema, UnknownTypeMode::Preserve).unwrap();
    let source = ValidatedImportDocument::new(
        document,
        &engine.schema,
        &engine.canonical_schema,
        &engine.resource_limits,
        Some(value.to_string().len()),
    )
    .unwrap();
    reset_import_lookup_event_count_for_test();
    let candidate = engine
        .build_candidate_from_document(source, TransactionOrigin::DocumentImport)
        .unwrap();
    set_lookup_seed_hydration_failpoint_for_test(None);
    assert_eq!(take_import_lookup_event_count_for_test(), 2);
    assert!(candidate
        .import_encoded_state_receipt
        .as_ref()
        .is_some_and(|receipt| receipt.lookup_materialization.is_none()));
    let EngineDocumentState::Ready { document, .. } = candidate.state else {
        panic!("validated candidate must be ready")
    };
    assert_eq!(document.root().content().unwrap().child_count(), 2);
}

#[test]
fn missing_text_fallback_rebuilds_once_then_next_insert_localizes() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_111, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("empty paragraph insert must apply");
    engine
        .apply_command(70_112, TypedCommand::InsertText { text: "y".into() })
        .unwrap()
        .expect("existing text insert must apply");

    assert_eq!(take_localized_lookup_counts_for_test(), (1, 1, 1));
}

#[test]
fn selection_only_change_retains_document_scoped_lookup_seed() {
    let mut engine = transaction_engine();
    let before = engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .clone();
    let canonical_before = engine
        .derived_state
        .as_ref()
        .unwrap()
        .canonical_artifact
        .clone();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_113,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    let after = &engine.derived_state.as_ref().unwrap().mutation_lookup_seed;
    assert!(Arc::ptr_eq(&before, after));
    assert!(canonical_before.ptr_eq(&engine.derived_state.as_ref().unwrap().canonical_artifact));
}

#[test]
fn localized_root_invalidation_rebuilds_ready_once_then_localizes() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .apply_command(
            70_113_100,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_prepared_candidate_state_vector_exact(&engine);
    let unavailable = engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .clone();
    assert!(unavailable.is_unavailable_for_test());

    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_113_101,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_113_102, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (2, 0, 0));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_113_103, TypedCommand::InsertText { text: "y".into() })
        .unwrap()
        .unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
}

#[test]
fn canonical_artifact_derives_once_per_changed_intermediate_and_never_for_cached_noops() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_canonical_artifact_counts_for_test();
    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_114))
        .unwrap();
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));

    let revision = engine.revision();
    reset_canonical_artifact_counts_for_test();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_115,
            base_document_revision: revision,
            origin: TransactionOrigin::LocalApi,
            operations: vec![
                TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "a".into(),
                    marks: vec![],
                },
                TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "b".into(),
                    marks: vec![],
                },
            ],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert_eq!(take_canonical_artifact_counts_for_test(), (2, 3));

    reset_canonical_artifact_counts_for_test();
    let commit = engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_116,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::DeleteRange {
                range: RevisionedRange {
                    from: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    to: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                },
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(!commit.changed);
    assert_eq!(take_canonical_artifact_counts_for_test(), (0, 0));
}

#[test]
fn public_history_pop_installs_candidate_seed_without_next_edit_rebuild() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_prepared_admission_counts_for_test();
    assert!(engine.undo(70_119).unwrap().is_none());
    assert!(engine.redo(70_120).unwrap().is_none());
    let empty = take_prepared_admission_counts_for_test();
    assert_eq!(empty.staged_seed_preparations, 0);
    assert_eq!(empty.installed_base_seed_publications, 0);
    reset_localized_lookup_counts_for_test();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 0, 0));

    engine
        .apply_command(70_121, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("history insert must apply");
    reset_localized_lookup_counts_for_test();
    reset_prepared_admission_counts_for_test();
    let before_undo = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    assert!(engine.undo(70_122).unwrap().is_some());
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
    let undo_counts = take_prepared_admission_counts_for_test();
    assert_eq!(undo_counts.staged_seed_preparations, 1);
    assert_eq!(undo_counts.installed_base_seed_publications, 0);
    assert!(!Arc::ptr_eq(
        &before_undo,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    reset_localized_lookup_counts_for_test();
    reset_prepared_admission_counts_for_test();
    assert!(engine.redo(70_123).unwrap().is_some());
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
    let redo_counts = take_prepared_admission_counts_for_test();
    assert_eq!(redo_counts.staged_seed_preparations, 1);
    assert_eq!(redo_counts.installed_base_seed_publications, 0);
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_124, TypedCommand::InsertText { text: "y".into() })
        .unwrap()
        .expect("the first edit after history restoration must apply");
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_125, TypedCommand::InsertText { text: "z".into() })
        .unwrap()
        .expect("the second edit after history restoration must apply");
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    reset_localized_lookup_counts_for_test();
    engine.restore_snapshot(&snapshot).unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
}

#[test]
fn accepted_remote_candidate_builds_lookup_seed_in_its_own_store() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let update = source.encoded_state().unwrap();
    let mut target = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "doc".into(),
            lineage_id: "lineage".into(),
        }),
    })
    .unwrap();
    reset_localized_lookup_counts_for_test();

    let commit = target.apply_remote_update_v1(70_131, &update).unwrap();
    assert!(commit.changed);
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
    reset_localized_lookup_counts_for_test();
    target
        .apply_command(70_132, TypedCommand::InsertText { text: "!".into() })
        .unwrap()
        .expect("remote existing text must accept a local insert");
    assert_prepared_candidate_state_vector_exact(&target);
    let live_vector = target.doc.transact().state_vector();
    assert!(live_vector.get(&ClientID::new(source.client_id())) > 0);
    assert!(live_vector.get(&ClientID::new(target.client_id())) > 0);
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
}

#[test]
fn arbitrary_remote_candidate_rebuilds_revision_bound_render_cache_once() {
    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]},{"type":"paragraph","content":[{"type":"text","text":"tail"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut target = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "doc".into(),
            lineage_id: "lineage".into(),
        }),
    })
    .unwrap();
    target
        .apply_remote_update_v1(70_133, &source.encoded_state().unwrap())
        .unwrap();
    source
        .apply_typed_transaction(insert_transaction(&source, 70_134))
        .unwrap();
    let target_vector = target.doc.transact().state_vector();
    let delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);

    crate::render::incremental::reset_cached_render_counts_for_test();
    let commit = target.apply_remote_update_v1(70_135, &delta).unwrap();
    assert!(commit.changed);
    assert_eq!(
        crate::render::incremental::take_cached_render_counts_for_test(),
        (1, 0, 0, 0, 0)
    );
    let next = target.derived_state.as_ref().unwrap();
    assert_eq!(
        next.render_blocks.materialize(),
        crate::render::incremental::render_blocks(&next.document, &target.schema)
    );
    assert_eq!(next.document_revision, target.revision());
    assert_eq!(next.schema_fingerprint, target.schema_fingerprint);
}

#[test]
fn multi_operation_and_explicit_selection_inserts_use_sealed_eager_fallback() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut transaction = insert_transaction(&engine, 70_141);
    transaction.operations.push(TypedOperation::InsertText {
        at: RevisionedPosition {
            offset: 2,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        },
        text: "y".into(),
        marks: vec![],
    });
    reset_localized_lookup_counts_for_test();
    engine.apply_typed_transaction(transaction).unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));

    let mut transaction = insert_transaction(&engine, 70_142);
    let point = RevisionedPosition {
        offset: 2,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    transaction.selection_intent = SelectionIntent::Set(SelectionInput::Text {
        anchor: point,
        head: point,
    });
    reset_localized_lookup_counts_for_test();
    engine.apply_typed_transaction(transaction).unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
}

#[test]
fn localized_insert_preserves_semantic_validation_error_precedence_over_lowering_limits() {
    fn constrained_engine() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine.editing_limits.max_operations_per_transaction = 1;
        engine.resource_limits.max_document_depth = 1;
        engine.resource_limits.max_document_nodes = 1;
        engine
    }

    let localized = constrained_engine();
    let localized_error = localized
        .compile_typed_transaction(insert_transaction(&localized, 70_143))
        .unwrap_err();

    let eager = constrained_engine();
    let mut eager_transaction = insert_transaction(&eager, 70_143);
    eager_transaction.selection_intent = SelectionIntent::Set(SelectionInput::All);
    let eager_error = eager
        .compile_typed_transaction(eager_transaction)
        .unwrap_err();

    assert_eq!(localized_error, eager_error);
    assert_eq!(localized_error.code, "DOCUMENT_LIMIT_EXCEEDED");
}

#[test]
fn engine_compile_reuses_all_cached_base_semantic_inputs() {
    use crate::yrs_engine::compiler::{
        reset_base_compilation_build_counts_for_test, take_base_compilation_build_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut transaction = insert_transaction(&engine, 70_002);
    let TypedOperation::InsertText { at, .. } = &mut transaction.operations[0] else {
        unreachable!()
    };
    at.offset = 2;
    let point = RevisionedPosition {
        offset: 2,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    transaction.selection_intent = SelectionIntent::Set(SelectionInput::Text {
        anchor: point,
        head: point,
    });
    reset_base_compilation_build_counts_for_test();

    engine.compile_typed_transaction(transaction).unwrap();

    assert_eq!(take_base_compilation_build_counts_for_test(), (0, 0, 0));
}

#[test]
fn selection_only_revision_refreshes_the_cached_compilation_view() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = |offset| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_003,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(1),
                head: point(2),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();

    assert_eq!(
        engine.derived_state.as_ref().unwrap().legacy_selection,
        crate::selection::Selection::text(2, 3)
    );
    engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_004,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
}

#[test]
fn changed_rich_command_derives_preview_map_and_render_at_most_once() {
    use crate::yrs_engine::derived_state::{
        reset_preview_derivation_counts_for_test, take_preview_derivation_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_preview_derivation_counts_for_test();

    engine
        .apply_command(70_007, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    let (position_maps, rendered_texts) = take_preview_derivation_counts_for_test();
    assert!(position_maps <= 1, "built {position_maps} preview maps");
    assert!(
        rendered_texts <= 1,
        "built {rendered_texts} preview renders"
    );
}

#[test]
fn existing_text_command_skips_every_proved_document_wide_compiler_pass() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let caret = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_008,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: caret,
                head: caret,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    hydrate_import_for_compile_test(&mut engine);
    reset_full_pass_counts_for_test();

    engine
        .apply_command(70_009, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert_eq!(
        take_full_pass_counts_for_test(),
        FullPassCounts {
            import_model_parses: 0,
            validated_evidence_constructions: 0,
            validation_certificate_constructions: 0,
            planner_simulations: 1,
            document_validations: 1,
            canonical_mark_tree_scans: 0,
            canonical_mark_validation_attempts: 0,
            canonical_mark_validation_completions: 0,
            canonical_mark_nodes_visited: 0,
            canonical_identity_predicate_nodes_visited: 3,
            canonical_projections: 1,
            canonical_serializations: 1,
            canonical_hashes: 1,
            affected_top_level_scans: 0,
            position_map_clones: 1,
            position_map_compactions: 1,
            rendered_text_derivations: 0,
            raw_document_text_scans: 1,
            document_node_count_scans: 0,
            render_limit_tree_scans: 0,
            render_identity_scans: 0,
            render_top_level_start_scans: 0,
            active_applicability_passes: 1,
            ordinary_step_applications: 1,
        }
    );
}

#[test]
fn existing_text_admission_certificate_matches_legacy_compiler_and_commit() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_010,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    hydrate_import_for_compile_test(&mut engine);
    let transaction = TypedTransaction {
        request_id: 70_011,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: point,
            text: "🙂\\\"‍".into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    let compiled = engine
        .compile_typed_transaction(transaction.clone())
        .unwrap();
    let proof = compiled
        .localized_insert_admission
        .as_ref()
        .expect("strict-inside existing text produces E1 admission evidence")
        .clone();
    let read_txn = engine.doc.transact();
    let fragment = read_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    let current = engine.derived_state.as_ref().unwrap();
    let admission_document_position = crate::yrs_engine::position::editor_offset_to_doc_pos(
        point.offset,
        point.kind,
        &current.rendered_text,
        &current.position_map,
        &current.document,
    )
    .unwrap();
    let validated = proof
        .validate_current(
            current,
            &transaction,
            admission_document_position,
            &read_txn,
            &fragment,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            engine.yrs_state_epoch,
        )
        .expect("every private admission claim revalidates");
    let mut same_metrics_different_text = transaction.clone();
    let [TypedOperation::InsertText { text, .. }] =
        same_metrics_different_text.operations.as_mut_slice()
    else {
        unreachable!()
    };
    *text = "🙃\\\"‍".into();
    assert!(proof
        .validate_current(
            current,
            &same_metrics_different_text,
            admission_document_position,
            &read_txn,
            &fragment,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            engine.yrs_state_epoch,
        )
        .is_none());
    for (claim, tampered) in proof.tampered_claims_for_test() {
        assert!(
            tampered
                .validate_current(
                    current,
                    &transaction,
                    admission_document_position,
                    &read_txn,
                    &fragment,
                    &engine.resource_limits,
                    &engine.editing_limits,
                    engine.max_length,
                    engine.yrs_state_epoch,
                )
                .is_none(),
            "tampered private claim must fail closed: {claim}"
        );
    }
    drop(read_txn);
    let full_stats =
        DocumentValidator::validate(&compiled.preview, &engine.schema, &engine.resource_limits)
            .unwrap();
    assert_eq!(
        full_stats,
        engine
            .derived_state
            .as_ref()
            .unwrap()
            .validation_certificate
            .stats()
    );
    let artifact = compiled.canonical_artifact.as_ref().unwrap();
    assert_eq!(
        artifact.text_scalar_len(),
        validated.next_raw_text_scalars()
    );
    assert_eq!(
        artifact.text_utf8_bytes(),
        validated.next_raw_text_utf8_bytes()
    );
    assert_eq!(
        artifact.serialized_len(),
        validated.next_canonical_serialized_len()
    );
    assert_eq!(compiled.undo_units_bound, validated.history_undo_units());
    assert_eq!(
        compiled.replay_work_units_bound,
        validated.history_undo_units()
    );
    assert_eq!(
        compiled
            .preview_derivations
            .as_ref()
            .unwrap()
            .position_map
            .total_scalars(),
        validated.next_rendered_scalars()
    );
    let expected_fingerprint = artifact.sha256();
    let expected_operation_result = validated.operation_result().clone();
    let expected_stored_marks = validated.stored_marks().map(<[_]>::to_vec);
    let expected_rendered_scalars = validated.next_rendered_scalars();

    let result = engine
        .apply_compiled_transaction(compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(result.selection, expected_operation_result);
    assert_eq!(engine.stored_marks(), expected_stored_marks.as_deref());
    assert!(engine.can_undo());
    let next = engine.derived_state.as_ref().unwrap();
    assert_eq!(next.validation_certificate.stats(), full_stats);
    assert_eq!(
        next.validation_certificate.canonical_fingerprint(),
        expected_fingerprint
    );
    assert_eq!(next.position_map.total_scalars(), expected_rendered_scalars);
    assert_eq!(
        u32::try_from(next.rendered_text.chars().count()).unwrap(),
        expected_rendered_scalars
    );
}

#[test]
fn admission_evidence_does_zero_work_before_envelope_admission() {
    use crate::yrs_engine::derived_state::{
        reset_localized_insert_admission_work_for_test,
        take_localized_insert_admission_work_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let position = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let insert = |base_document_revision, origin, text: &str| TypedTransaction {
        request_id: 70_012,
        base_document_revision,
        origin,
        operations: vec![TypedOperation::InsertText {
            at: position,
            text: text.into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    reset_localized_insert_admission_work_for_test();
    assert!(engine
        .compile_typed_transaction(insert(
            engine.revision().saturating_add(1),
            TransactionOrigin::LocalInput,
            "x",
        ))
        .is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);

    reset_localized_insert_admission_work_for_test();
    assert!(engine
        .compile_typed_transaction(insert(
            engine.revision(),
            TransactionOrigin::RemoteSync,
            "x",
        ))
        .is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);

    engine.editing_limits.max_operations_per_transaction = 1;
    let mut excess = insert(engine.revision(), TransactionOrigin::LocalInput, "x");
    excess.operations.push(excess.operations[0].clone());
    reset_localized_insert_admission_work_for_test();
    assert!(engine.compile_typed_transaction(excess).is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);

    engine.resource_limits.max_input_bytes = 1;
    reset_localized_insert_admission_work_for_test();
    assert!(engine
        .compile_typed_transaction(insert(
            engine.revision(),
            TransactionOrigin::LocalInput,
            "oversized",
        ))
        .is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);
}

#[test]
fn localized_insert_admission_does_zero_work_before_cached_view_and_yrs_scan_admission() {
    use crate::yrs_engine::derived_state::{
        reset_localized_insert_admission_work_for_test,
        take_localized_insert_admission_work_for_test,
    };

    let fixture = || {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    };
    let transaction = |engine: &YrsDocumentEngine, request_id| TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            text: "x".into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    let mut cached_view_rejection = fixture();
    let cached_transaction = transaction(&cached_view_rejection, 700_122);
    cached_view_rejection
        .derived_state
        .as_mut()
        .unwrap()
        .rendered_scalars += 1;
    reset_localized_insert_admission_work_for_test();
    assert!(cached_view_rejection
        .compile_typed_transaction(cached_transaction)
        .is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);

    let mut yrs_scan_rejection = fixture();
    let scan_transaction = transaction(&yrs_scan_rejection, 700_123);
    yrs_scan_rejection.resource_limits.max_input_bytes = 8;
    reset_localized_insert_admission_work_for_test();
    let error = yrs_scan_rejection
        .compile_typed_transaction(scan_transaction)
        .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.details.as_ref().unwrap()["field"], "maxInputBytes");
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);
}

#[test]
fn localized_insert_admission_runs_before_mutation_preflight() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::derived_state::{
        reset_localized_insert_admission_work_for_test,
        take_localized_insert_admission_work_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    hydrate_import_for_compile_test(&mut engine);
    let transaction = TypedTransaction {
        request_id: 700_121,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            text: "x".into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    reset_localized_insert_admission_work_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
    let result = engine.compile_typed_transaction(transaction);
    set_atomic_failpoint_for_test(None);

    assert!(result.is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 1);
}

#[test]
fn admission_evidence_rejects_unsupported_selection_and_history_contracts() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let transaction = |selection_intent, history_policy| TypedTransaction {
        request_id: 70_013,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: point,
            text: "x".into(),
            marks: Vec::new(),
        }],
        selection_intent,
        history_policy,
    };

    assert!(engine
        .compile_typed_transaction(transaction(SelectionIntent::Preserve, HistoryPolicy::Auto,))
        .unwrap()
        .localized_insert_admission
        .is_none());
    assert!(engine
        .compile_typed_transaction(transaction(
            SelectionIntent::UseOperationResult,
            HistoryPolicy::Skip,
        ))
        .unwrap()
        .localized_insert_admission
        .is_none());
}

#[test]
fn localized_insert_admission_eligibility_is_exact() {
    let fixture = |marked: bool| {
        let mut engine = transaction_engine();
        let json = if marked {
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"abc"}]}]}"#
        } else {
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#
        };
        engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    };
    let point = |offset| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let transaction = |engine: &YrsDocumentEngine,
                       origin,
                       at,
                       text: &str,
                       marks,
                       selection_intent,
                       history_policy| TypedTransaction {
        request_id: 700_131,
        base_document_revision: engine.revision(),
        origin,
        operations: vec![TypedOperation::InsertText {
            at,
            text: text.into(),
            marks,
        }],
        selection_intent,
        history_policy,
    };

    let engine = fixture(false);
    for origin in [
        TransactionOrigin::LocalInput,
        TransactionOrigin::LocalCommand,
        TransactionOrigin::LocalApi,
    ] {
        assert!(engine
            .compile_typed_transaction(transaction(
                &engine,
                origin,
                point(1),
                "x",
                Vec::new(),
                SelectionIntent::UseOperationResult,
                HistoryPolicy::Auto,
            ))
            .unwrap()
            .localized_insert_admission
            .is_some());
    }

    for boundary in [point(0), point(3)] {
        assert!(engine
            .compile_typed_transaction(transaction(
                &engine,
                TransactionOrigin::LocalInput,
                boundary,
                "x",
                Vec::new(),
                SelectionIntent::UseOperationResult,
                HistoryPolicy::Auto,
            ))
            .unwrap()
            .localized_insert_admission
            .is_none());
    }

    for history_policy in [HistoryPolicy::Boundary, HistoryPolicy::Skip] {
        assert!(engine
            .compile_typed_transaction(transaction(
                &engine,
                TransactionOrigin::LocalInput,
                point(1),
                "x",
                Vec::new(),
                SelectionIntent::UseOperationResult,
                history_policy,
            ))
            .unwrap()
            .localized_insert_admission
            .is_none());
    }
    for origin in [TransactionOrigin::LocalCommand, TransactionOrigin::LocalApi] {
        assert!(engine
            .compile_typed_transaction(transaction(
                &engine,
                origin,
                point(1),
                "x",
                Vec::new(),
                SelectionIntent::UseOperationResult,
                HistoryPolicy::Boundary,
            ))
            .unwrap()
            .localized_insert_admission
            .is_none());
    }
    assert!(engine
        .compile_typed_transaction(transaction(
            &engine,
            TransactionOrigin::LocalInput,
            point(1),
            "x",
            Vec::new(),
            SelectionIntent::Preserve,
            HistoryPolicy::Auto,
        ))
        .unwrap()
        .localized_insert_admission
        .is_none());
    assert!(engine
        .compile_typed_transaction(transaction(
            &engine,
            TransactionOrigin::LocalInput,
            point(1),
            "x",
            Vec::new(),
            SelectionIntent::Set(SelectionInput::Text {
                anchor: point(1),
                head: point(1),
            }),
            HistoryPolicy::Auto,
        ))
        .unwrap()
        .localized_insert_admission
        .is_none());

    let mut multiple = transaction(
        &engine,
        TransactionOrigin::LocalInput,
        point(1),
        "x",
        Vec::new(),
        SelectionIntent::UseOperationResult,
        HistoryPolicy::Auto,
    );
    multiple.operations.push(multiple.operations[0].clone());
    assert!(engine
        .compile_typed_transaction(multiple)
        .unwrap()
        .localized_insert_admission
        .is_none());

    let marked_engine = fixture(true);
    let bold = vec![Mark::new("bold".into(), HashMap::new())];
    assert!(marked_engine
        .compile_typed_transaction(transaction(
            &marked_engine,
            TransactionOrigin::LocalInput,
            point(1),
            "x",
            bold,
            SelectionIntent::UseOperationResult,
            HistoryPolicy::Auto,
        ))
        .unwrap()
        .localized_insert_admission
        .is_some());
    assert!(marked_engine
        .compile_typed_transaction(transaction(
            &marked_engine,
            TransactionOrigin::LocalInput,
            point(1),
            "x",
            Vec::new(),
            SelectionIntent::UseOperationResult,
            HistoryPolicy::Auto,
        ))
        .unwrap()
        .localized_insert_admission
        .is_none());
}

#[test]
fn localized_insert_admission_preserves_generic_results_errors_and_full_pass_counts() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let fixture = || {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    };
    let transaction = |engine: &YrsDocumentEngine, request_id, marks| TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            text: "x".into(),
            marks,
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    let mut admitted = fixture();
    reset_full_pass_counts_for_test();
    let admitted_result = admitted
        .apply_typed_transaction_with_result(transaction(&admitted, 700_132, Vec::new()))
        .unwrap();
    let admitted_counts = take_full_pass_counts_for_test();

    let mut generic = fixture();
    generic.derived_state.as_mut().unwrap().localized_text_index = None;
    reset_full_pass_counts_for_test();
    let generic_result = generic
        .apply_typed_transaction_with_result(transaction(&generic, 700_132, Vec::new()))
        .unwrap();
    let generic_counts = take_full_pass_counts_for_test();

    assert_eq!(admitted_result, generic_result);
    assert_eq!(admitted.document_json(), generic.document_json());
    assert_eq!(admitted_counts.ordinary_step_applications, 0);
    assert_eq!(generic_counts.ordinary_step_applications, 1);
    assert_eq!(admitted.can_undo(), generic.can_undo());
    assert_eq!(admitted.can_redo(), generic.can_redo());

    let admitted_undo = admitted.undo(700_141).unwrap();
    let generic_undo = generic.undo(700_141).unwrap();
    assert_eq!(admitted_undo, generic_undo);
    assert_eq!(admitted.document_json(), generic.document_json());
    assert_eq!(admitted.can_undo(), generic.can_undo());
    assert_eq!(admitted.can_redo(), generic.can_redo());

    let admitted_redo = admitted.redo(700_142).unwrap();
    let generic_redo = generic.redo(700_142).unwrap();
    assert_eq!(admitted_redo, generic_redo);
    assert_eq!(admitted.document_json(), generic.document_json());
    assert_eq!(admitted.can_undo(), generic.can_undo());
    assert_eq!(admitted.can_redo(), generic.can_redo());

    let invalid_mark = vec![Mark::new("unknown".into(), HashMap::new())];
    let mut admitted_error_engine = fixture();
    let mut generic_error_engine = fixture();
    generic_error_engine
        .derived_state
        .as_mut()
        .unwrap()
        .localized_text_index = None;
    let admitted_error = admitted_error_engine
        .apply_typed_transaction_with_result(transaction(
            &admitted_error_engine,
            700_133,
            invalid_mark.clone(),
        ))
        .unwrap_err();
    let generic_error = generic_error_engine
        .apply_typed_transaction_with_result(transaction(
            &generic_error_engine,
            700_133,
            invalid_mark,
        ))
        .unwrap_err();
    assert_eq!(admitted_error, generic_error);
    assert_eq!(
        admitted_error_engine.document_json(),
        generic_error_engine.document_json()
    );
}

#[test]
fn localized_insert_compile_only_skips_every_proved_full_pass() {
    use crate::model::node::{
        reset_deep_node_payload_clones_for_test, take_deep_node_payload_clones_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
    };

    let fixture = || {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    };
    let transaction = |engine: &YrsDocumentEngine, request_id| TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            text: "x".into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    let eligible = fixture();
    reset_full_pass_counts_for_test();
    let compiled = eligible
        .compile_typed_transaction(transaction(&eligible, 700_134))
        .unwrap();
    assert_eq!(compiled.affected_top_level_blocks, vec![0]);
    assert_eq!(
        take_full_pass_counts_for_test(),
        FullPassCounts {
            canonical_projections: 1,
            canonical_serializations: 2,
            canonical_hashes: 1,
            position_map_clones: 1,
            position_map_compactions: 1,
            render_identity_scans: 0,
            ..FullPassCounts::default()
        }
    );

    let mut generic = fixture();
    generic.derived_state.as_mut().unwrap().localized_text_index = None;
    reset_full_pass_counts_for_test();
    generic
        .compile_typed_transaction(transaction(&generic, 700_135))
        .unwrap();
    assert_eq!(
        take_full_pass_counts_for_test(),
        FullPassCounts {
            document_validations: 2,
            canonical_mark_tree_scans: 1,
            canonical_mark_validation_attempts: 1,
            canonical_mark_validation_completions: 1,
            canonical_mark_nodes_visited: 3,
            canonical_identity_predicate_nodes_visited: 3,
            canonical_projections: 1,
            canonical_serializations: 1,
            canonical_hashes: 0,
            affected_top_level_scans: 1,
            position_map_clones: 1,
            position_map_compactions: 1,
            rendered_text_derivations: 1,
            raw_document_text_scans: 2,
            document_node_count_scans: 1,
            render_identity_scans: 0,
            ordinary_step_applications: 1,
            ..FullPassCounts::default()
        }
    );

    let mut wide = transaction_engine();
    let content = (0..160)
        .map(|index| {
            json!({
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": format!("{index:04} {}", "x".repeat(214))
                }]
            })
        })
        .collect::<Vec<_>>();
    wide.import_json(
        &json!({"type": "doc", "content": content}).to_string(),
        TransactionOrigin::DocumentImport,
    )
    .unwrap();
    let rendered = &wide.derived_state.as_ref().unwrap().rendered_text;
    let needle = "0159 ";
    let needle_byte = rendered.find(needle).unwrap();
    let offset = u32::try_from(rendered[..needle_byte].chars().count() + needle.len()).unwrap();
    reset_deep_node_payload_clones_for_test();
    wide.compile_typed_transaction(TypedTransaction {
        request_id: 700_143,
        base_document_revision: wide.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            text: "y".into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    })
    .unwrap();
    assert_eq!(
        take_deep_node_payload_clones_for_test(),
        0,
        "localized reconstruction must copy only immutable node handles"
    );
}

#[test]
fn localized_insert_semantic_preview_matches_forced_generic_matrix() {
    fn assert_compiled_parity(
        localized: &crate::yrs_engine::compiler::CompiledTransaction,
        generic: &crate::yrs_engine::compiler::CompiledTransaction,
    ) {
        assert_eq!(localized.preview, generic.preview);
        let localized_artifact = localized.canonical_artifact.as_ref().unwrap();
        let generic_artifact = generic.canonical_artifact.as_ref().unwrap();
        assert_eq!(localized_artifact.value(), generic_artifact.value());
        assert_eq!(localized_artifact.sha256(), generic_artifact.sha256());
        assert_eq!(
            localized_artifact.serialized_len(),
            generic_artifact.serialized_len()
        );
        assert_eq!(
            localized_artifact.text_scalar_len(),
            generic_artifact.text_scalar_len()
        );
        assert_eq!(
            localized_artifact.text_utf8_bytes(),
            generic_artifact.text_utf8_bytes()
        );
        assert!(localized_artifact.matches_document(&localized.preview));
        assert!(generic_artifact.matches_document(&generic.preview));
        assert_eq!(
            localized.composed_map.ranges(),
            generic.composed_map.ranges()
        );
        assert_eq!(localized.selection_plan, generic.selection_plan);
        assert_eq!(
            localized.relative_selection_plan,
            generic.relative_selection_plan
        );
        assert_eq!(localized.stored_marks_plan, generic.stored_marks_plan);
        assert_eq!(localized.history_class, generic.history_class);
        assert_eq!(localized.undo_units_bound, generic.undo_units_bound);
        assert_eq!(
            localized.replay_work_units_bound,
            generic.replay_work_units_bound
        );
        assert_eq!(localized.encoded_growth_bound, generic.encoded_growth_bound);
        assert_eq!(localized.authored_clock_units, generic.authored_clock_units);
        assert_eq!(
            localized.affected_top_level_blocks,
            generic.affected_top_level_blocks
        );
        assert_eq!(localized.position_update_mode, generic.position_update_mode);
        assert_eq!(
            format!("{:?}", localized.mutation_plan.actions),
            format!("{:?}", generic.mutation_plan.actions)
        );
        assert_eq!(
            localized.mutation_plan.compilation_work_for_test(),
            generic.mutation_plan.compilation_work_for_test()
        );
        assert_eq!(
            localized.mutation_plan.expected_preflight_work_for_test(),
            generic.mutation_plan.expected_preflight_work_for_test()
        );
        assert_eq!(
            localized.mutation_plan.scan_work,
            generic.mutation_plan.scan_work
        );
        let localized_derived = localized.preview_derivations.as_ref().unwrap();
        let generic_derived = generic.preview_derivations.as_ref().unwrap();
        assert_eq!(
            localized_derived.rendered_text,
            generic_derived.rendered_text
        );
        assert_eq!(
            localized_derived.rendered_scalars,
            generic_derived.rendered_scalars
        );
        assert_eq!(
            localized_derived.document_text_bytes,
            generic_derived.document_text_bytes
        );
        assert_eq!(
            localized_derived.document_node_count,
            generic_derived.document_node_count
        );
        assert_eq!(
            format!("{:?}", localized_derived.position_map),
            format!("{:?}", generic_derived.position_map)
        );
    }

    let cases = [
        (
            "ascii",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            "abc",
            1usize,
            "x",
            Vec::new(),
            vec![0],
        ),
        (
            "non-bmp-escaped-control",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            "abc",
            1,
            "🙂\\\"\n\u{1}",
            Vec::new(),
            vec![0],
        ),
        (
            "canonical-mark",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"abc"}]}]}"#,
            "abc",
            1,
            "x",
            vec![Mark::new("bold".into(), HashMap::new())],
            vec![0],
        ),
        (
            "fragmented-mark-leaves",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"ab"},{"type":"text","marks":[{"type":"italic"}],"text":"cd"}]}]}"#,
            "cd",
            1,
            "🙂",
            vec![Mark::new("italic".into(), HashMap::new())],
            vec![0],
        ),
        (
            "deep-nesting",
            r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}]}"#,
            "abc",
            1,
            "x",
            Vec::new(),
            vec![0],
        ),
        (
            "list-prefix",
            r#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}]}"#,
            "abc",
            1,
            "x",
            Vec::new(),
            vec![0],
        ),
        (
            "third-top-level",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]},{"type":"paragraph","content":[{"type":"text","text":"second"}]},{"type":"paragraph","content":[{"type":"text","text":"third"}]}]}"#,
            "third",
            1,
            "x",
            Vec::new(),
            vec![1, 2],
        ),
    ];

    for (case, json, needle, inside, inserted, marks, expected_affected) in cases {
        let mut engine = transaction_engine();
        engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let rendered = &engine.derived_state.as_ref().unwrap().rendered_text;
        let needle_byte = rendered.find(needle).unwrap();
        let offset = u32::try_from(rendered[..needle_byte].chars().count() + inside).unwrap();
        let transaction = TypedTransaction {
            request_id: 700_136,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: inserted.into(),
                marks,
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        };

        let localized = engine
            .compile_typed_transaction(transaction.clone())
            .unwrap();
        assert!(localized.localized_insert_admission.is_some(), "{case}");
        assert_eq!(
            localized.affected_top_level_blocks, expected_affected,
            "{case}"
        );
        let saved_index = engine
            .derived_state
            .as_mut()
            .unwrap()
            .localized_text_index
            .take();
        let generic = engine
            .compile_typed_transaction(transaction.clone())
            .unwrap();
        engine.derived_state.as_mut().unwrap().localized_text_index = saved_index;
        assert_compiled_parity(&localized, &generic);

        let localized_result = engine
            .apply_compiled_transaction(localized, true)
            .unwrap()
            .1
            .unwrap();
        let mut generic_engine = transaction_engine();
        generic_engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        generic_engine
            .derived_state
            .as_mut()
            .unwrap()
            .localized_text_index = None;
        let generic_compiled = generic_engine
            .compile_typed_transaction(transaction)
            .unwrap();
        let generic_result = generic_engine
            .apply_compiled_transaction(generic_compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(localized_result, generic_result, "{case}");
        assert_eq!(
            engine.document_json(),
            generic_engine.document_json(),
            "{case}"
        );
        let localized_state = engine.derived_state.as_ref().unwrap();
        let generic_state = generic_engine.derived_state.as_ref().unwrap();
        assert_eq!(
            localized_state.validation_certificate, generic_state.validation_certificate,
            "{case}"
        );
        assert_eq!(
            localized_state.localized_text_index, generic_state.localized_text_index,
            "{case}"
        );
        assert_eq!(
            localized_state.canonical_artifact.value(),
            generic_state.canonical_artifact.value(),
            "{case}"
        );
        assert_eq!(
            localized_state.canonical_artifact.sha256(),
            generic_state.canonical_artifact.sha256(),
            "{case}"
        );
        assert_eq!(
            localized_state.rendered_text, generic_state.rendered_text,
            "{case}"
        );
        assert_eq!(engine.can_undo(), generic_engine.can_undo(), "{case}");
        assert_eq!(engine.can_redo(), generic_engine.can_redo(), "{case}");
        assert_eq!(
            engine.undo(700_151).unwrap(),
            generic_engine.undo(700_151).unwrap(),
            "{case}"
        );
        assert_eq!(
            engine.document_json(),
            generic_engine.document_json(),
            "{case}"
        );
        assert_eq!(
            engine.redo(700_152).unwrap(),
            generic_engine.redo(700_152).unwrap(),
            "{case}"
        );
        assert_eq!(
            engine.document_json(),
            generic_engine.document_json(),
            "{case}"
        );
    }

    use crate::yrs_engine::compiler::force_localized_semantic_allocation_failure_for_test;
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    hydrate_import_for_compile_test(&mut engine);
    let transaction = TypedTransaction {
        request_id: 700_139,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            text: "x".into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };
    reset_full_pass_counts_for_test();
    force_localized_semantic_allocation_failure_for_test(true);
    let fallback = engine.compile_typed_transaction(transaction.clone());
    force_localized_semantic_allocation_failure_for_test(false);
    let fallback = fallback.unwrap();
    assert!(fallback.localized_insert_admission.is_some());
    assert_eq!(
        take_full_pass_counts_for_test().ordinary_step_applications,
        1
    );
    let saved_index = engine
        .derived_state
        .as_mut()
        .unwrap()
        .localized_text_index
        .take();
    let generic = engine.compile_typed_transaction(transaction).unwrap();
    engine.derived_state.as_mut().unwrap().localized_text_index = saved_index;
    assert_compiled_parity(&fallback, &generic);
}

#[test]
fn localized_insert_exact_limits_and_one_under_errors_match_generic() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };
    use crate::yrs_engine::EditingLimits;

    fn fixture(max_length: Option<u32>, editing_limits: EditingLimits) -> YrsDocumentEngine {
        let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits,
            max_length,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    }

    fn transaction(engine: &YrsDocumentEngine) -> TypedTransaction {
        TypedTransaction {
            request_id: 700_140,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "xy".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    fn assert_error_pair(
        localized: YrsDocumentEngine,
        mut generic: YrsDocumentEngine,
        field: &str,
    ) {
        generic.derived_state.as_mut().unwrap().localized_text_index = None;
        reset_full_pass_counts_for_test();
        let localized_error = localized
            .compile_typed_transaction(transaction(&localized))
            .unwrap_err();
        assert_eq!(
            take_full_pass_counts_for_test().ordinary_step_applications,
            1,
            "{field} must silently fall back to generic compilation"
        );
        let generic_error = generic
            .compile_typed_transaction(transaction(&generic))
            .unwrap_err();
        assert_eq!(localized_error, generic_error);
        assert_eq!(localized_error.details.as_ref().unwrap()["field"], field);
    }

    let probe_engine = fixture(None, EditingLimits::default());
    let probe = probe_engine
        .compile_typed_transaction(transaction(&probe_engine))
        .unwrap();
    let exact_output = probe.canonical_artifact.as_ref().unwrap().serialized_len();
    let exact_undo = probe.undo_units_bound;

    let exact_length = fixture(Some(5), EditingLimits::default());
    assert!(exact_length
        .compile_typed_transaction(transaction(&exact_length))
        .unwrap()
        .localized_insert_admission
        .is_some());
    let rejected_length = fixture(Some(4), EditingLimits::default());
    let generic_length = fixture(Some(4), EditingLimits::default());
    assert_error_pair(rejected_length, generic_length, "maxLength");

    let exact_output_limits = EditingLimits {
        max_derived_output_bytes: exact_output,
        ..EditingLimits::default()
    };
    let exact_output_engine = fixture(None, exact_output_limits);
    assert!(exact_output_engine
        .compile_typed_transaction(transaction(&exact_output_engine))
        .unwrap()
        .localized_insert_admission
        .is_some());
    let rejected_output_limits = EditingLimits {
        max_derived_output_bytes: exact_output - 1,
        ..EditingLimits::default()
    };
    let rejected_output = fixture(None, rejected_output_limits.clone());
    let generic_output = fixture(None, rejected_output_limits);
    assert_error_pair(rejected_output, generic_output, "maxDerivedOutputBytes");

    let exact_undo_limits = EditingLimits {
        max_undo_retained_units: exact_undo,
        ..EditingLimits::default()
    };
    let exact_undo_engine = fixture(None, exact_undo_limits);
    assert!(exact_undo_engine
        .compile_typed_transaction(transaction(&exact_undo_engine))
        .unwrap()
        .localized_insert_admission
        .is_some());
    let rejected_undo_limits = EditingLimits {
        max_undo_retained_units: exact_undo - 1,
        ..EditingLimits::default()
    };
    let rejected_undo = fixture(None, rejected_undo_limits.clone());
    let generic_undo = fixture(None, rejected_undo_limits);
    assert_error_pair(rejected_undo, generic_undo, "maxUndoRetainedUnits");
}

#[test]
fn localized_index_promotion_allocation_failures_drop_only_optional_index() {
    use crate::yrs_engine::derived_state::{
        force_localized_index_allocation_stage_for_test, force_localized_index_budget_for_test,
        reset_localized_index_lifecycle_counts_for_test,
        take_localized_index_lifecycle_counts_for_test, LocalizedIndexAllocationStage,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    }
    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
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
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    let mut baseline = fixture();
    let baseline_result = baseline
        .apply_typed_transaction_with_result(transaction(&baseline, 700_144))
        .unwrap();
    let baseline_json = baseline.document_json();

    for (index, stage) in [
        LocalizedIndexAllocationStage::PromotionClone,
        LocalizedIndexAllocationStage::PromotionGrowth,
        LocalizedIndexAllocationStage::PromotionUpdate,
    ]
    .into_iter()
    .enumerate()
    {
        let mut engine = fixture();
        reset_localized_index_lifecycle_counts_for_test();
        force_localized_index_allocation_stage_for_test(Some(stage));
        let compiled = engine.compile_typed_transaction(transaction(
            &engine,
            700_145 + u64::try_from(index).unwrap(),
        ));
        force_localized_index_allocation_stage_for_test(None);
        let result = engine
            .apply_compiled_transaction(compiled.unwrap(), true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(result.changed, baseline_result.changed, "{stage:?}");
        assert_eq!(result.selection, baseline_result.selection, "{stage:?}");
        assert_eq!(engine.document_json(), baseline_json, "{stage:?}");
        assert!(engine.can_undo(), "{stage:?}");
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .localized_text_index
            .is_none());
        assert_eq!(
            take_localized_index_lifecycle_counts_for_test(),
            (0, 1, 0, 1),
            "{stage:?}"
        );
    }

    let mut engine = fixture();
    reset_localized_index_lifecycle_counts_for_test();
    force_localized_index_budget_for_test(Some(0));
    let compiled = engine.compile_typed_transaction(transaction(&engine, 700_149));
    force_localized_index_budget_for_test(None);
    engine
        .apply_compiled_transaction(compiled.unwrap(), true)
        .unwrap();
    assert_eq!(engine.document_json(), baseline_json);
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .localized_text_index
        .is_none());
    assert_eq!(
        take_localized_index_lifecycle_counts_for_test(),
        (0, 1, 0, 1)
    );
}

#[test]
fn localized_index_promotion_obeys_exact_transient_budget_boundary() {
    use crate::yrs_engine::derived_state::{
        force_localized_index_budget_for_test, reset_localized_index_lifecycle_counts_for_test,
        take_localized_index_lifecycle_counts_for_test,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
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
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    fn history_audit(engine: &YrsDocumentEngine) -> (bool, bool, u64, (usize, usize, bool)) {
        (
            engine.can_undo(),
            engine.can_redo(),
            engine.history.retained_units(0).unwrap(),
            engine.history.replay_audit_for_test(),
        )
    }

    let mut exact = fixture();
    let exact_budget = exact
        .derived_state
        .as_ref()
        .unwrap()
        .localized_text_index
        .as_ref()
        .unwrap()
        .promotion_transient_budget_for_test()
        .unwrap();
    reset_localized_index_lifecycle_counts_for_test();
    force_localized_index_budget_for_test(Some(exact_budget));
    let exact_compiled = exact
        .compile_typed_transaction(transaction(&exact, 700_162))
        .unwrap();
    force_localized_index_budget_for_test(None);
    let exact_result = exact
        .apply_compiled_transaction(exact_compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(
        take_localized_index_lifecycle_counts_for_test(),
        (0, 1, 1, 0)
    );

    let mut generic = fixture();
    generic.derived_state.as_mut().unwrap().localized_text_index = None;
    let generic_transaction = transaction(&generic, 700_162);
    let generic_result = generic
        .apply_typed_transaction_with_result(generic_transaction)
        .unwrap();
    assert_eq!(exact_result, generic_result);
    assert_eq!(exact.document_json(), generic.document_json());
    assert_eq!(history_audit(&exact), history_audit(&generic));
    assert_eq!(
        exact.derived_state.as_ref().unwrap().localized_text_index,
        generic.derived_state.as_ref().unwrap().localized_text_index
    );

    let mut one_under = fixture();
    reset_localized_index_lifecycle_counts_for_test();
    force_localized_index_budget_for_test(Some(exact_budget - 1));
    let one_under_compiled = one_under
        .compile_typed_transaction(transaction(&one_under, 700_162))
        .unwrap();
    force_localized_index_budget_for_test(None);
    let one_under_result = one_under
        .apply_compiled_transaction(one_under_compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(one_under_result, generic_result);
    assert_eq!(one_under.document_json(), generic.document_json());
    assert_eq!(history_audit(&one_under), history_audit(&generic));
    assert!(one_under
        .derived_state
        .as_ref()
        .unwrap()
        .localized_text_index
        .is_none());
    assert_eq!(
        take_localized_index_lifecycle_counts_for_test(),
        (0, 1, 0, 1)
    );
}

#[test]
fn every_localized_derived_evidence_tamper_falls_back_before_write() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::derived_state::{
        reset_localized_index_lifecycle_counts_for_test,
        take_localized_index_lifecycle_counts_for_test, PreparedDerivedEvidence,
    };

    for case in PreparedDerivedEvidence::tamper_cases_for_test() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let mut compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_150,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        compiled
            .prepared_derived_evidence
            .as_mut()
            .unwrap()
            .tamper_for_test(case);
        let before = atomic_audit(&engine);
        reset_localized_index_lifecycle_counts_for_test();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::DurableMetadataAdmission));
        let applied = engine.apply_compiled_transaction(compiled, true);
        set_atomic_failpoint_for_test(None);
        assert!(applied.is_err(), "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
        assert_eq!(
            take_localized_index_lifecycle_counts_for_test(),
            (1, 0, 0, 0),
            "{case} must prepare generic evidence before the failpoint"
        );
    }
}

#[test]
fn every_localized_render_proof_tamper_falls_back_before_write() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
    };
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::derived_state::PreparedDerivedEvidence;
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let cases = PreparedDerivedEvidence::localized_render_tamper_cases_for_test()
        .iter()
        .copied()
        .chain(std::iter::once("affectedRange"));
    for case in cases {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let mut compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_151,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        if case == "affectedRange" {
            compiled.affected_top_level_blocks.clear();
        } else {
            compiled
                .prepared_derived_evidence
                .as_mut()
                .unwrap()
                .tamper_localized_render_for_test(case);
        }
        let before = atomic_audit(&engine);
        reset_full_pass_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::DurableMetadataAdmission));
        let error = engine
            .apply_compiled_transaction(compiled, true)
            .expect_err("durable metadata failpoint must abort the fallback commit");
        set_atomic_failpoint_for_test(None);

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
        let passes = take_full_pass_counts_for_test();
        assert_eq!(passes.render_limit_tree_scans, 1, "{case}");
        assert_eq!(passes.render_identity_scans, 0, "{case}");
        assert_eq!(passes.render_top_level_start_scans, 1, "{case}");
        assert_eq!(
            take_cached_render_counts_for_test(),
            (0, 1, 1, 0, 0),
            "{case}"
        );
        assert_eq!(
            take_localized_render_transition_counts_for_test(),
            (1, 0, 1),
            "{case}"
        );
    }
}

#[test]
fn malformed_multiblock_localized_render_ranges_fall_back_exactly() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
    };
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
    };

    struct RangeAudit {
        error: crate::yrs_engine::OperationError,
        cached_counts: (usize, usize, usize, usize, usize),
        lifecycle_counts: (usize, usize, usize),
        full_pass_counts: FullPassCounts,
    }

    fn run(affected: Option<Vec<usize>>) -> RangeAudit {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"aaa"}]},{"type":"paragraph","content":[{"type":"text","text":"bbb"}]},{"type":"paragraph","content":[{"type":"text","text":"ccc"}]},{"type":"paragraph","content":[{"type":"text","text":"ddd"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let mut compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_154,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 9,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        assert_eq!(compiled.affected_top_level_blocks, [1, 2, 3]);
        match affected {
            Some(affected) => compiled.affected_top_level_blocks = affected,
            None => compiled.localized_semantic_used = false,
        }
        let before = atomic_audit(&engine);
        reset_full_pass_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::DurableMetadataAdmission));
        let applied = engine.apply_compiled_transaction(compiled, true);
        set_atomic_failpoint_for_test(None);
        let error = applied.expect_err("durable metadata failpoint must abort the commit");
        assert_eq!(atomic_audit(&engine), before);
        RangeAudit {
            error,
            cached_counts: take_cached_render_counts_for_test(),
            lifecycle_counts: take_localized_render_transition_counts_for_test(),
            full_pass_counts: take_full_pass_counts_for_test(),
        }
    }

    let generic = run(None);
    assert_eq!(generic.lifecycle_counts, (0, 0, 0));
    for (case, affected) in [
        ("empty", vec![]),
        ("tooNarrow", vec![1, 2]),
        ("wrongStart", vec![0, 1, 2]),
        ("duplicate", vec![1, 2, 2]),
        ("outOfOrder", vec![1, 3, 2]),
        ("outOfRange", vec![1, 2, 4]),
    ] {
        let malformed = run(Some(affected));
        assert_eq!(malformed.error, generic.error, "{case}");
        assert_eq!(malformed.cached_counts, generic.cached_counts, "{case}");
        assert_eq!(
            malformed.full_pass_counts, generic.full_pass_counts,
            "{case}"
        );
        assert_eq!(malformed.lifecycle_counts, (1, 0, 1), "{case}");
    }
}

#[test]
fn every_localized_render_stage_failure_falls_back_with_exact_parity() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test,
        reset_localized_render_failure_checkpoint_counts_for_test,
        reset_localized_render_transition_counts_for_test,
        set_localized_render_failure_stage_for_test, take_cached_render_counts_for_test,
        take_localized_render_failure_checkpoint_counts_for_test,
        take_localized_render_transition_counts_for_test, LocalizedRenderFailureStage,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
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
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    let mut generic = fixture();
    let mut generic_compiled = generic
        .compile_typed_transaction(transaction(&generic, 700_152))
        .unwrap();
    generic_compiled
        .prepared_derived_evidence
        .as_mut()
        .unwrap()
        .tamper_localized_render_for_test("missing");
    let generic_result = generic
        .apply_compiled_transaction(generic_compiled, true)
        .unwrap()
        .1
        .unwrap();

    for stage in [
        LocalizedRenderFailureStage::Allocation,
        LocalizedRenderFailureStage::Resource,
        LocalizedRenderFailureStage::Position,
        LocalizedRenderFailureStage::Invariant,
    ] {
        let mut engine = fixture();
        let compiled = engine
            .compile_typed_transaction(transaction(&engine, 700_152))
            .unwrap();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        reset_localized_render_failure_checkpoint_counts_for_test();
        set_localized_render_failure_stage_for_test(Some(stage));
        let applied = engine.apply_compiled_transaction(compiled, true);
        set_localized_render_failure_stage_for_test(None);
        let result = applied.unwrap().1.unwrap();

        assert_eq!(result, generic_result, "{stage:?}");
        assert_eq!(engine.document_json(), generic.document_json(), "{stage:?}");
        let state = engine.derived_state.as_ref().unwrap();
        let generic_state = generic.derived_state.as_ref().unwrap();
        assert_eq!(
            state.validation_certificate, generic_state.validation_certificate,
            "{stage:?}"
        );
        assert_eq!(
            state.localized_text_index, generic_state.localized_text_index,
            "{stage:?}"
        );
        assert_eq!(
            state.render_blocks.materialize(),
            generic_state.render_blocks.materialize(),
            "{stage:?}"
        );
        assert_eq!(engine.can_undo(), generic.can_undo(), "{stage:?}");
        assert_eq!(engine.can_redo(), generic.can_redo(), "{stage:?}");
        assert_eq!(
            engine.history.retained_units(0).unwrap(),
            generic.history.retained_units(0).unwrap(),
            "{stage:?}"
        );
        assert_eq!(
            engine.history.replay_audit_for_test(),
            generic.history.replay_audit_for_test(),
            "{stage:?}"
        );
        assert_eq!(
            take_cached_render_counts_for_test(),
            (0, 1, 1, 0, 0),
            "{stage:?}"
        );
        assert_eq!(
            take_localized_render_transition_counts_for_test(),
            (1, 0, 1),
            "{stage:?}"
        );
        let expected_checkpoints = match stage {
            LocalizedRenderFailureStage::Allocation => (1, 0, 0, 0),
            LocalizedRenderFailureStage::Resource => (1, 1, 0, 0),
            LocalizedRenderFailureStage::Position => (1, 1, 1, 0),
            LocalizedRenderFailureStage::Invariant => (1, 1, 1, 1),
        };
        assert_eq!(
            take_localized_render_failure_checkpoint_counts_for_test(),
            expected_checkpoints,
            "{stage:?}"
        );
    }
}

#[test]
fn localized_render_failure_exposes_only_the_generic_transition_error() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        set_cached_render_error_for_test, set_localized_render_failure_stage_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
        CachedRenderError, LocalizedRenderFailureStage,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
    };

    struct FailureAudit {
        error: crate::yrs_engine::OperationError,
        cached_counts: (usize, usize, usize, usize, usize),
        lifecycle_counts: (usize, usize, usize),
        full_pass_counts: FullPassCounts,
    }

    fn run(stage: Option<LocalizedRenderFailureStage>) -> FailureAudit {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let mut compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_153,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        compiled.localized_semantic_used = stage.is_some();
        let before = atomic_audit(&engine);
        reset_full_pass_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        set_localized_render_failure_stage_for_test(stage);
        set_cached_render_error_for_test(Some(CachedRenderError::AllocationFailed));
        let applied = engine.apply_compiled_transaction(compiled, true);
        set_localized_render_failure_stage_for_test(None);
        set_cached_render_error_for_test(None);
        let error = applied.expect_err("forced generic transition failure must be returned");
        assert_eq!(atomic_audit(&engine), before);
        FailureAudit {
            error,
            cached_counts: take_cached_render_counts_for_test(),
            lifecycle_counts: take_localized_render_transition_counts_for_test(),
            full_pass_counts: take_full_pass_counts_for_test(),
        }
    }

    let generic = run(None);
    assert_eq!(generic.error.code, "ENGINE_INVARIANT_FAILED");
    assert!(generic.error.message.contains("AllocationFailed"));
    assert_eq!(generic.cached_counts, (0, 1, 0, 0, 0));
    assert_eq!(generic.lifecycle_counts, (0, 0, 0));
    assert_eq!(generic.full_pass_counts, FullPassCounts::default());
    for stage in [
        LocalizedRenderFailureStage::Allocation,
        LocalizedRenderFailureStage::Resource,
        LocalizedRenderFailureStage::Position,
        LocalizedRenderFailureStage::Invariant,
    ] {
        let localized = run(Some(stage));
        assert_eq!(localized.error, generic.error, "{stage:?}");
        assert_eq!(localized.cached_counts, generic.cached_counts, "{stage:?}");
        assert_eq!(localized.lifecycle_counts, (1, 0, 1), "{stage:?}");
        assert_eq!(
            localized.full_pass_counts, generic.full_pass_counts,
            "{stage:?}"
        );
    }
}

#[test]
fn changed_commit_survives_optional_index_allocation_failure_exactly() {
    use crate::yrs_engine::derived_state::force_localized_index_allocation_failure_for_test;

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_revision = engine.revision();
    let before_state_revision = engine.state_revision();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    force_localized_index_allocation_failure_for_test(true);
    let applied = engine.apply_typed_transaction_with_result(TypedTransaction {
        request_id: 700_121,
        base_document_revision: before_revision,
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: point,
            text: "x".into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    });
    force_localized_index_allocation_failure_for_test(false);
    let result = applied.expect("optional index failure cannot abort commit");
    assert!(result.changed);
    assert_eq!(result.document_revision, before_revision + 1);
    assert_eq!(result.state_revision, before_state_revision + 1);
    assert!(result.changed);
    assert!(matches!(
        result.selection,
        crate::yrs_engine::ResolvedSelection::Text { ref anchor, ref head }
            if anchor.document == 3 && head.document == 3
    ));
    assert_eq!(
        engine.document_json().unwrap()["content"][0]["content"][0]["text"],
        "axbc"
    );
    assert!(engine.can_undo());
    let state = engine.derived_state.as_ref().unwrap();
    assert_eq!(state.document_revision, result.document_revision);
    assert_eq!(state.state_revision, result.state_revision);
    assert!(state.localized_text_index.is_none());

    let compiled = engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 700_122,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition { offset: 2, ..point },
                text: "y".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert!(compiled.localized_insert_admission.is_none());
}

#[test]
fn changed_commit_survives_optional_index_budget_failure_exactly() {
    use crate::yrs_engine::derived_state::force_localized_index_budget_for_test;

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_revision = engine.revision();
    force_localized_index_budget_for_test(Some(1));
    let result = engine.apply_typed_transaction_with_result(TypedTransaction {
        request_id: 700_123,
        base_document_revision: before_revision,
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            text: "x".into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    });
    force_localized_index_budget_for_test(None);
    let result = result.expect("optional index budget cannot abort commit");
    assert!(result.changed);
    assert_eq!(result.document_revision, before_revision + 1);
    assert_eq!(
        engine.document_json().unwrap()["content"][0]["content"][0]["text"],
        "axbc"
    );
    assert!(engine.can_undo());
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .localized_text_index
        .is_none());
}

#[test]
fn changed_commit_survives_each_optional_index_allocation_stage() {
    use crate::yrs_engine::derived_state::{
        force_localized_index_allocation_stage_for_test, LocalizedIndexAllocationStage,
    };

    for (stage_index, stage) in [
        LocalizedIndexAllocationStage::InitialLeafCapacity,
        LocalizedIndexAllocationStage::TraversalPath,
        LocalizedIndexAllocationStage::LeafGrowth,
    ]
    .into_iter()
    .enumerate()
    {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"},{"type":"hardBreak"},{"type":"text","text":"cd"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let before_document_revision = engine.revision();
        let before_state_revision = engine.state_revision();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        force_localized_index_allocation_stage_for_test(Some(stage));
        let applied = engine.apply_typed_transaction_with_result(TypedTransaction {
            request_id: 700_130 + u64::try_from(stage_index).unwrap(),
            base_document_revision: before_document_revision,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point,
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        });
        force_localized_index_allocation_stage_for_test(None);

        let result = applied.expect("optional index failure cannot abort a live commit");
        assert!(result.changed, "stage {stage:?}");
        assert_eq!(result.document_revision, before_document_revision + 1);
        assert_eq!(result.state_revision, before_state_revision + 1);
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["content"][0]["text"],
            "axb"
        );
        assert!(engine.can_undo());
        let state = engine.derived_state.as_ref().unwrap();
        assert_eq!(state.document_revision, result.document_revision);
        assert_eq!(state.state_revision, result.state_revision);
        assert!(state.localized_text_index.is_none(), "stage {stage:?}");

        let compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_140 + u64::try_from(stage_index).unwrap(),
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition { offset: 2, ..point },
                    text: "y".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        assert!(compiled.localized_insert_admission.is_none());
    }
}

#[test]
fn selection_only_optional_index_copy_failure_degrades_evidence_to_none() {
    use crate::yrs_engine::derived_state::{
        force_localized_index_allocation_stage_for_test, LocalizedIndexAllocationStage,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_document_revision = engine.revision();
    let before_state_revision = engine.state_revision();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    force_localized_index_allocation_stage_for_test(Some(
        LocalizedIndexAllocationStage::InitialLeafCapacity,
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .clone_with_fallible_localized_index()
        .localized_text_index
        .is_none());
    let applied = engine.apply_typed_transaction_with_result(TypedTransaction {
        request_id: 700_150,
        base_document_revision: before_document_revision,
        origin: TransactionOrigin::LocalApi,
        operations: Vec::new(),
        selection_intent: SelectionIntent::Set(SelectionInput::Text {
            anchor: point,
            head: point,
        }),
        history_policy: HistoryPolicy::Auto,
    });
    force_localized_index_allocation_stage_for_test(None);

    let result = applied.expect("optional evidence copy failure cannot abort selection");
    assert!(result.changed);
    assert_eq!(result.document_revision, before_document_revision);
    assert_eq!(result.state_revision, before_state_revision + 1);
    let state = engine.derived_state.as_ref().unwrap();
    assert!(state.localized_text_index.is_none());
    assert_eq!(state.document_revision, before_document_revision);
    assert_eq!(state.state_revision, before_state_revision + 1);

    let compiled = engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 700_151,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point,
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert!(compiled.localized_insert_admission.is_none());
}

#[test]
fn selection_only_revision_reseal_allows_following_strict_insert_admission() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_014,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    hydrate_import_for_compile_test(&mut engine);
    let state = engine.derived_state.as_ref().unwrap();
    assert_eq!(
        state.validation_certificate.state_revision(),
        engine.state_revision()
    );

    let compiled = engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_015,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point,
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert!(compiled.localized_insert_admission.is_some());

    engine
        .apply_command(
            70_016,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    engine
        .apply_command(
            700_161,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    assert_eq!(
        state.validation_certificate.state_revision(),
        engine.state_revision()
    );
    let stored_marks = engine.stored_marks().unwrap_or_default().to_vec();
    let compiled = engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_017,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point,
                text: "x".into(),
                marks: stored_marks,
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert!(compiled.localized_insert_admission.is_some());
}

#[test]
fn benchmark_shaped_bursts_decompose_direct_result_and_command_full_passes() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
    };
    use crate::yrs_engine::derived_state::{
        reset_active_state_cache_counts_for_test, reset_localized_index_lifecycle_counts_for_test,
        take_active_state_cache_counts_for_test, take_localized_index_lifecycle_counts_for_test,
    };
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        let content = (0..160)
            .map(|index| {
                json!({
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": format!("{index:04} {}", "x".repeat(214))
                    }]
                })
            })
            .collect::<Vec<_>>();
        engine
            .import_json(
                &json!({"type": "doc", "content": content}).to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 44,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_100,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    }

    fn direct(engine: &YrsDocumentEngine, index: usize) -> TypedTransaction {
        TypedTransaction {
            request_id: 70_200 + index as u64,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 44 + index as u32,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    let mut direct_commit = fixture();
    let mut commit_counts = Vec::new();
    for index in 0..20 {
        reset_full_pass_counts_for_test();
        reset_localized_lookup_counts_for_test();
        reset_localized_index_lifecycle_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        let transaction = direct(&direct_commit, index);
        direct_commit.apply_typed_transaction(transaction).unwrap();
        commit_counts.push((
            take_full_pass_counts_for_test(),
            take_localized_lookup_counts_for_test(),
            take_localized_index_lifecycle_counts_for_test(),
            take_cached_render_counts_for_test(),
            take_localized_render_transition_counts_for_test(),
        ));
    }

    let mut direct_result = fixture();
    let mut result_counts = Vec::new();
    for index in 0..20 {
        reset_full_pass_counts_for_test();
        reset_localized_lookup_counts_for_test();
        reset_localized_index_lifecycle_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        let transaction = direct(&direct_result, index);
        direct_result
            .apply_typed_transaction_with_result(transaction)
            .unwrap();
        result_counts.push((
            take_full_pass_counts_for_test(),
            take_localized_lookup_counts_for_test(),
            take_localized_index_lifecycle_counts_for_test(),
            take_cached_render_counts_for_test(),
            take_localized_render_transition_counts_for_test(),
        ));
    }

    let mut command = fixture();
    let mut command_counts = Vec::new();
    reset_active_state_cache_counts_for_test();
    for index in 0..20 {
        reset_full_pass_counts_for_test();
        reset_localized_lookup_counts_for_test();
        reset_localized_index_lifecycle_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        command
            .apply_command(
                70_300 + index as u64,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap()
            .unwrap();
        command_counts.push((
            take_full_pass_counts_for_test(),
            take_localized_lookup_counts_for_test(),
            take_localized_index_lifecycle_counts_for_test(),
            take_cached_render_counts_for_test(),
            take_localized_render_transition_counts_for_test(),
        ));
    }
    assert_eq!(
        take_active_state_cache_counts_for_test(),
        (20, 19, 1, 1, 20, 20, 0, 20, 1),
        "prepared command burst must build ActiveState once, then reuse it"
    );

    let expected_commit = (
        FullPassCounts {
            import_model_parses: 0,
            validated_evidence_constructions: 0,
            validation_certificate_constructions: 0,
            planner_simulations: 0,
            document_validations: 0,
            canonical_mark_tree_scans: 0,
            canonical_mark_validation_attempts: 0,
            canonical_mark_validation_completions: 0,
            canonical_mark_nodes_visited: 0,
            canonical_identity_predicate_nodes_visited: 0,
            canonical_projections: 1,
            canonical_serializations: 2,
            canonical_hashes: 1,
            affected_top_level_scans: 0,
            position_map_clones: 1,
            position_map_compactions: 1,
            rendered_text_derivations: 0,
            raw_document_text_scans: 0,
            document_node_count_scans: 0,
            render_limit_tree_scans: 0,
            render_identity_scans: 0,
            render_top_level_start_scans: 0,
            active_applicability_passes: 0,
            ordinary_step_applications: 0,
        },
        (0, 1, 1),
        (0, 1, 1, 0),
        (0, 1, 1, 0, 0),
        (1, 1, 0),
    );
    let expected_result = (
        FullPassCounts {
            import_model_parses: 0,
            validated_evidence_constructions: 0,
            validation_certificate_constructions: 0,
            planner_simulations: 0,
            document_validations: 0,
            canonical_mark_tree_scans: 0,
            canonical_mark_validation_attempts: 0,
            canonical_mark_validation_completions: 0,
            canonical_mark_nodes_visited: 0,
            canonical_identity_predicate_nodes_visited: 0,
            canonical_projections: 1,
            canonical_serializations: 2,
            canonical_hashes: 1,
            affected_top_level_scans: 0,
            position_map_clones: 1,
            position_map_compactions: 1,
            rendered_text_derivations: 0,
            raw_document_text_scans: 0,
            document_node_count_scans: 0,
            render_limit_tree_scans: 0,
            render_identity_scans: 0,
            render_top_level_start_scans: 0,
            active_applicability_passes: 1,
            ordinary_step_applications: 0,
        },
        (0, 1, 1),
        (0, 1, 1, 0),
        (0, 1, 1, 0, 0),
        (1, 1, 0),
    );
    let expected_command = (
        FullPassCounts {
            import_model_parses: 0,
            validated_evidence_constructions: 0,
            validation_certificate_constructions: 0,
            planner_simulations: 1,
            document_validations: 1,
            canonical_mark_tree_scans: 0,
            canonical_mark_validation_attempts: 0,
            canonical_mark_validation_completions: 0,
            canonical_mark_nodes_visited: 0,
            canonical_identity_predicate_nodes_visited: 321,
            canonical_projections: 1,
            canonical_serializations: 1,
            canonical_hashes: 1,
            affected_top_level_scans: 0,
            position_map_clones: 1,
            position_map_compactions: 1,
            rendered_text_derivations: 0,
            raw_document_text_scans: 1,
            document_node_count_scans: 0,
            render_limit_tree_scans: 0,
            render_identity_scans: 0,
            render_top_level_start_scans: 0,
            active_applicability_passes: 1,
            ordinary_step_applications: 1,
        },
        (0, 1, 1),
        (0, 1, 1, 0),
        (0, 1, 1, 0, 0),
        (1, 1, 0),
    );
    for (index, actual) in commit_counts.iter().enumerate() {
        assert_eq!(*actual, expected_commit, "direct commit edit {index}");
    }
    for (index, actual) in result_counts.iter().enumerate() {
        assert_eq!(*actual, expected_result, "direct result edit {index}");
    }
    for (index, actual) in command_counts.iter().enumerate() {
        let mut expected = expected_command;
        expected.0.active_applicability_passes = usize::from(index == 0);
        assert_eq!(*actual, expected, "command edit {index}");
    }

    let mut promoted = fixture();
    let mut rebuilt = fixture();
    for index in 0..20 {
        rebuilt.derived_state.as_mut().unwrap().localized_text_index = None;
        let promoted_transaction = direct(&promoted, index);
        let rebuilt_transaction = direct(&rebuilt, index);
        let promoted_result = promoted
            .apply_typed_transaction_with_result(promoted_transaction)
            .unwrap();
        let rebuilt_result = rebuilt
            .apply_typed_transaction_with_result(rebuilt_transaction)
            .unwrap();
        assert_eq!(promoted_result, rebuilt_result, "sequential edit {index}");
        assert_eq!(promoted.document_json(), rebuilt.document_json());
        let promoted_state = promoted.derived_state.as_ref().unwrap();
        let rebuilt_state = rebuilt.derived_state.as_ref().unwrap();
        assert_eq!(
            promoted_state.validation_certificate, rebuilt_state.validation_certificate,
            "sequential edit {index}"
        );
        assert_eq!(
            promoted_state.localized_text_index, rebuilt_state.localized_text_index,
            "sequential edit {index}"
        );
    }
    assert_eq!(
        promoted.undo(700_153).unwrap(),
        rebuilt.undo(700_153).unwrap()
    );
    assert_eq!(promoted.document_json(), rebuilt.document_json());
    assert_eq!(
        promoted.redo(700_154).unwrap(),
        rebuilt.redo(700_154).unwrap()
    );
    assert_eq!(promoted.document_json(), rebuilt.document_json());
}

#[test]
fn prepared_active_state_cache_allocation_and_budget_misses_are_optional() {
    use crate::yrs_engine::derived_state::{
        force_active_state_cache_allocation_failure_for_test,
        force_active_state_cache_budget_for_test,
        force_active_state_public_materialization_failure_for_test,
        reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 710_000,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
    }

    for budget_failure in [false, true] {
        let mut engine = fixture();
        reset_active_state_cache_counts_for_test();
        if budget_failure {
            force_active_state_cache_budget_for_test(Some(0));
        } else {
            force_active_state_cache_allocation_failure_for_test(true);
        }
        let result = engine
            .apply_command(710_001, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        force_active_state_cache_budget_for_test(None);
        force_active_state_cache_allocation_failure_for_test(false);

        assert!(result.changed);
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["content"][0]["text"],
            "axbc"
        );
        assert!(engine.can_undo());
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_none());
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 1, 0, 0, 0, 1)
        );
    }

    let mut measured = fixture();
    let measured_result = measured
        .apply_command(710_010, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    let retained = measured
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .unwrap()
        .retained_bytes_for_test();

    let mut exact = fixture();
    reset_active_state_cache_counts_for_test();
    force_active_state_cache_budget_for_test(Some(retained));
    let exact_result = exact
        .apply_command(710_010, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    force_active_state_cache_budget_for_test(None);
    assert_eq!(exact_result, measured_result);
    assert_eq!(
        take_active_state_cache_counts_for_test(),
        (1, 0, 1, 1, 1, 1, 0, 1, 1)
    );

    let mut under = fixture();
    reset_active_state_cache_counts_for_test();
    force_active_state_cache_budget_for_test(Some(retained - 1));
    let under_result = under
        .apply_command(710_010, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    force_active_state_cache_budget_for_test(None);
    assert_eq!(under_result, measured_result);
    assert_eq!(under.document_json(), measured.document_json());
    assert!(under
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .is_none());
    assert_eq!(
        take_active_state_cache_counts_for_test(),
        (1, 0, 1, 1, 1, 0, 0, 0, 1)
    );

    let mut materialization = measured;
    let mut baseline = exact;
    reset_active_state_cache_counts_for_test();
    force_active_state_public_materialization_failure_for_test(true);
    let materialized_result =
        materialization.apply_command(710_011, TypedCommand::InsertText { text: "y".into() });
    force_active_state_public_materialization_failure_for_test(false);
    let materialized_result = materialized_result.unwrap().unwrap();
    assert_eq!(
        take_active_state_cache_counts_for_test(),
        (1, 0, 1, 1, 1, 0, 1, 0, 1)
    );
    let baseline_result = baseline
        .apply_command(710_011, TypedCommand::InsertText { text: "y".into() })
        .unwrap()
        .unwrap();
    assert_eq!(materialized_result, baseline_result);
    assert_eq!(materialization.document_json(), baseline.document_json());
    assert_eq!(materialization.can_undo(), baseline.can_undo());
    assert_eq!(materialization.can_redo(), baseline.can_redo());
    assert!(materialization
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .is_none());
}

#[test]
fn prepared_active_state_transition_tamper_falls_back_with_exact_parity() {
    use crate::yrs_engine::derived_state::{
        reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 711_000,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(711_001, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        engine
    }

    fn compiled_insert(engine: &YrsDocumentEngine, request_id: u64) -> CompiledTransaction {
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id,
                TypedCommand::InsertText { text: "y".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("insert command must prepare a transaction")
        };
        engine
            .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
            .unwrap()
    }

    for (index, claim) in [
        "documentRevision",
        "stateRevision",
        "epoch",
        "schema",
        "resource",
        "editing",
        "maxLength",
        "selection",
        "relativeSelection",
        "legacySelection",
        "storedMarks",
        "structural",
        "resultSelection",
        "preview",
        "render",
        "lookup",
        "validation",
        "cachedPayloadIdentity",
    ]
    .into_iter()
    .enumerate()
    {
        let mut tampered = fixture();
        let mut generic = fixture();
        let request_id = 711_100 + u64::try_from(index).unwrap();
        let mut tampered_compiled = compiled_insert(&tampered, request_id);
        tampered_compiled
            .prepared_active_state_transition
            .as_mut()
            .unwrap()
            .tamper_for_test(claim);
        let mut generic_compiled = compiled_insert(&generic, request_id);
        generic_compiled.prepared_active_state_transition = None;

        reset_active_state_cache_counts_for_test();
        let tampered_result = tampered
            .apply_compiled_transaction(tampered_compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 0, 0, 1, 0, 1),
            "{claim}"
        );
        let generic_result = generic
            .apply_compiled_transaction(generic_compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(tampered_result, generic_result, "{claim}");
        assert_eq!(tampered.document_json(), generic.document_json(), "{claim}");
        assert_eq!(tampered.can_undo(), generic.can_undo(), "{claim}");
        assert_eq!(tampered.can_redo(), generic.can_redo(), "{claim}");
        assert!(tampered
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_none());
    }

    for (index, current_claim) in [
        "missingCurrentCertificate",
        "replacedCurrentCertificate",
        "replacedCurrentPayload",
    ]
    .into_iter()
    .enumerate()
    {
        let mut engine = fixture();
        let compiled = compiled_insert(&engine, 711_500 + u64::try_from(index).unwrap());
        let state = engine.derived_state.as_mut().unwrap();
        match current_claim {
            "missingCurrentCertificate" => state.remove_active_state_certificate_for_test(),
            "replacedCurrentCertificate" => {
                state.replace_active_state_certificate_identity_for_test()
            }
            "replacedCurrentPayload" => state.replace_active_state_payload_identity_for_test(),
            _ => unreachable!(),
        }
        reset_active_state_cache_counts_for_test();
        let result = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert!(result.changed, "{current_claim}");
        let expected_drops = usize::from(current_claim != "missingCurrentCertificate");
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 0, 0, expected_drops, 0, 1),
            "{current_claim}"
        );
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_none());
    }
}

#[test]
fn prepared_active_state_cache_survives_post_result_rejection_by_identity() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 712_000,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(712_001, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    let before = atomic_audit(&engine);
    let cache_before = engine
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .unwrap();

    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            712_002,
            TypedCommand::InsertText { text: "y".into() },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("insert command must prepare a transaction")
    };
    let compiled = engine
        .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
        .unwrap();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::FinalPreflight));
    let rejected = engine.apply_compiled_transaction(compiled, true);
    set_atomic_failpoint_for_test(None);
    assert!(rejected.is_err());
    assert_eq!(atomic_audit(&engine), before);
    let cache_after = engine
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .unwrap();
    assert!(Arc::ptr_eq(&cache_before, &cache_after));
}

#[test]
fn prepared_active_state_certificate_is_cleared_by_changed_state_boundaries() {
    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 713_000,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(713_001, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_some());
        engine
    }

    let assert_cleared = |engine: &YrsDocumentEngine, boundary: &str| {
        assert!(
            engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .is_none(),
            "{boundary}"
        );
    };

    let mut selection = fixture();
    let point = RevisionedPosition {
        offset: 0,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    selection
        .apply_typed_transaction(TypedTransaction {
            request_id: 713_010,
            base_document_revision: selection.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert_cleared(&selection, "selection");

    let mut direct = fixture();
    let caret = direct
        .derived_state
        .as_ref()
        .unwrap()
        .resolved_selection
        .clone();
    let crate::yrs_engine::ResolvedSelection::Text { anchor, .. } = caret else {
        panic!("fixture retains a text caret")
    };
    direct
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 713_011,
            base_document_revision: direct.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: anchor.document,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "y".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert_cleared(&direct, "direct LocalInput");

    let mut undone = fixture();
    undone.undo(713_012).unwrap();
    assert_cleared(&undone, "undo");
    undone.redo(713_013).unwrap();
    assert_cleared(&undone, "redo");

    let mut stored_mark = fixture();
    stored_mark
        .apply_command(
            713_014,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_cleared(&stored_mark, "stored mark");

    let mut deleted = fixture();
    deleted
        .apply_command(713_015, TypedCommand::DeleteBackward)
        .unwrap()
        .unwrap();
    assert_cleared(&deleted, "prepared delete");

    let mut structural = fixture();
    structural
        .apply_command(713_016, TypedCommand::ToggleHeading { level: 2 })
        .unwrap()
        .unwrap();
    assert_cleared(&structural, "prepared structural command");

    let mut no_result = fixture();
    let crate::yrs_engine::ResolvedSelection::Text { anchor, .. } = no_result
        .derived_state
        .as_ref()
        .unwrap()
        .resolved_selection
        .clone()
    else {
        panic!("fixture retains a text caret")
    };
    no_result
        .apply_typed_transaction(TypedTransaction {
            request_id: 713_017,
            base_document_revision: no_result.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: anchor.document,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "z".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert_cleared(&no_result, "no-result changed transaction");

    let mut imported = fixture();
    imported
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"replacement"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_cleared(&imported, "import");

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let mut restored = fixture();
    restored.restore_snapshot(&snapshot).unwrap();
    assert_cleared(&restored, "snapshot restore");

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut remote = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "active-cache".into(),
            lineage_id: "invalidation".into(),
        }),
    })
    .unwrap();
    remote
        .apply_remote_update_v1(713_020, &source.encoded_state().unwrap())
        .unwrap();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    remote
        .apply_typed_transaction(TypedTransaction {
            request_id: 713_021,
            base_document_revision: remote.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    remote
        .apply_command(713_022, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    assert!(remote
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .is_some());
    source
        .apply_typed_transaction(insert_transaction(&source, 713_023))
        .unwrap();
    let remote_vector = remote.doc.transact().state_vector();
    let delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&remote_vector);
    assert!(
        remote
            .apply_remote_update_v1(713_024, &delta)
            .unwrap()
            .changed
    );
    assert_cleared(&remote, "accepted remote update");
}

#[test]
fn prepared_active_state_cache_rejection_and_noop_preserve_arc_identity() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 714_000,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(714_001, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    let cache = engine
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .unwrap();
    let before = atomic_audit(&engine);

    let rejected = engine.apply_typed_transaction(TypedTransaction {
        request_id: 714_002,
        base_document_revision: engine.revision().saturating_add(1),
        origin: TransactionOrigin::LocalApi,
        operations: Vec::new(),
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Auto,
    });
    assert!(rejected.is_err());
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &cache,
        &engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap()
    ));

    let no_op = engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 714_003,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(!no_op.changed);
    assert!(Arc::ptr_eq(
        &cache,
        &engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap()
    ));

    let boundary = engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 714_004,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();
    assert!(!boundary.changed);
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .is_none());
}

#[test]
fn prepared_active_state_warm_hit_matches_forced_generic_at_output_boundaries() {
    use crate::yrs_engine::derived_state::{
        force_active_state_cache_hit_fallback_for_test, reset_active_state_cache_counts_for_test,
        take_active_state_cache_counts_for_test,
    };

    fn fixture(
        json: &str,
        caret: u32,
        first: &str,
        max_derived_output_bytes: usize,
    ) -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine.editing_limits.max_derived_output_bytes = max_derived_output_bytes;
        engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        let point = RevisionedPosition {
            offset: caret,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 715_000,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(715_001, TypedCommand::InsertText { text: first.into() })
            .unwrap()
            .unwrap();
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_some());
        engine
    }

    fn assert_internal_parity(left: &YrsDocumentEngine, right: &YrsDocumentEngine) {
        assert_eq!(left.document_json(), right.document_json());
        assert_eq!(left.can_undo(), right.can_undo());
        assert_eq!(left.can_redo(), right.can_redo());
        let left_state = left.derived_state.as_ref().unwrap();
        let right_state = right.derived_state.as_ref().unwrap();
        assert_eq!(
            left_state.validation_certificate,
            right_state.validation_certificate
        );
        assert_eq!(
            left_state.localized_text_index,
            right_state.localized_text_index
        );
        assert_eq!(
            left_state.render_blocks.materialize(),
            right_state.render_blocks.materialize()
        );
        assert_eq!(
            left_state.active_state_cache_for_test().unwrap().value(),
            right_state.active_state_cache_for_test().unwrap().value()
        );
        for engine in [left, right] {
            let txn = engine.doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let state = engine.derived_state.as_ref().unwrap();
            assert!(state.mutation_lookup_seed.matches(
                &txn,
                &fragment,
                &state.document,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                engine.yrs_state_epoch,
                engine.revision,
            ));
        }
    }

    for (shape, json, caret, first) in [
        (
            "plain",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            1,
            "x",
        ),
        (
            "marked",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc","marks":[{"type":"bold"}]}]}]}"#,
            1,
            "x",
        ),
        (
            "nonBmp",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀b"}]}]}"#,
            1,
            "🦀",
        ),
    ] {
        // Keep the result-output boundary above the independently enforced
        // deep retained-state budget so the warm certificate exists at
        // both the exact and one-under output limits.
        let second = if shape == "nonBmp" {
            "界".repeat(2_048)
        } else {
            "y".repeat(4_096)
        };
        let mut probe = fixture(json, caret, first, usize::MAX / 2);
        let exact = probe
            .apply_command(
                715_002,
                TypedCommand::InsertText {
                    text: second.clone(),
                },
            )
            .unwrap()
            .unwrap()
            .derived_output_bytes();

        let mut hit = fixture(json, caret, first, exact);
        let mut generic = fixture(json, caret, first, exact);
        reset_active_state_cache_counts_for_test();
        let hit_result = hit
            .apply_command(
                715_003,
                TypedCommand::InsertText {
                    text: second.clone(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 1, 0, 0, 1, 1, 0, 1, 0),
            "{shape} hit"
        );
        reset_active_state_cache_counts_for_test();
        force_active_state_cache_hit_fallback_for_test(true);
        let generic_result = generic.apply_command(
            715_003,
            TypedCommand::InsertText {
                text: second.clone(),
            },
        );
        force_active_state_cache_hit_fallback_for_test(false);
        let generic_result = generic_result.unwrap().unwrap();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 1, 1, 0, 1, 1),
            "{shape} generic"
        );
        assert_eq!(hit_result.derived_output_bytes(), exact, "{shape}");
        assert_eq!(hit_result, generic_result, "{shape}");
        assert_internal_parity(&hit, &generic);

        let mut rejected_hit = fixture(json, caret, first, exact - 1);
        let mut rejected_generic = fixture(json, caret, first, exact - 1);
        let hit_cache = rejected_hit
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap();
        let generic_cache = rejected_generic
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap();
        let hit_before = atomic_audit(&rejected_hit);
        let generic_before = atomic_audit(&rejected_generic);
        reset_active_state_cache_counts_for_test();
        let hit_error = rejected_hit
            .apply_command(
                715_004,
                TypedCommand::InsertText {
                    text: second.clone(),
                },
            )
            .unwrap_err();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 1, 0, 0, 1, 0, 0, 1, 0),
            "{shape} rejected hit"
        );
        reset_active_state_cache_counts_for_test();
        force_active_state_cache_hit_fallback_for_test(true);
        let generic_error = rejected_generic.apply_command(
            715_004,
            TypedCommand::InsertText {
                text: second.clone(),
            },
        );
        force_active_state_cache_hit_fallback_for_test(false);
        let generic_error = generic_error.unwrap_err();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 1, 0, 0, 1, 1),
            "{shape} rejected generic"
        );
        assert_eq!(hit_error, generic_error, "{shape}");
        assert_eq!(
            hit_error.details,
            Some(json!({ "field": "maxDerivedOutputBytes" })),
            "{shape}"
        );
        assert_eq!(atomic_audit(&rejected_hit), hit_before, "{shape}");
        assert_eq!(atomic_audit(&rejected_generic), generic_before, "{shape}");
        assert!(Arc::ptr_eq(
            &hit_cache,
            &rejected_hit
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .unwrap()
        ));
        assert!(Arc::ptr_eq(
            &generic_cache,
            &rejected_generic
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .unwrap()
        ));
    }
}

#[test]
fn prepared_active_state_context_matrix_matches_forced_generic() {
    use crate::yrs_engine::derived_state::{
        force_active_state_cache_hit_fallback_for_test, reset_active_state_cache_counts_for_test,
        take_active_state_cache_counts_for_test,
    };

    fn fixture(
        shape: &str,
        json: &str,
        target_text: &str,
        intra_leaf_scalar: u32,
        explicit_stored_bold: bool,
    ) -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        let byte_start = state.rendered_text.find(target_text).unwrap();
        let scalar_start =
            u32::try_from(state.rendered_text[..byte_start].chars().count()).unwrap();
        let rendered_position = scalar_start + intra_leaf_scalar;
        let selection_at = |engine: &YrsDocumentEngine, affinity| {
            let point = RevisionedPosition {
                offset: rendered_position,
                kind: EditorOffsetKind::Scalar,
                affinity,
            };
            TypedTransaction {
                request_id: 716_000,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            }
        };
        if engine
            .apply_typed_transaction(selection_at(&engine, Affinity::After))
            .is_err()
        {
            engine
                .apply_typed_transaction(selection_at(&engine, Affinity::Before))
                .unwrap();
        }
        if explicit_stored_bold {
            for request_id in [716_001, 716_002] {
                engine
                    .apply_command(
                        request_id,
                        TypedCommand::ToggleMark {
                            mark_type: "bold".into(),
                        },
                    )
                    .unwrap()
                    .unwrap();
            }
            assert!(engine
                .stored_marks()
                .is_some_and(|marks| { marks.iter().any(|mark| mark.mark_type() == "bold") }));
        }
        engine
            .apply_command(716_003, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        assert!(
            engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .is_some(),
            "{shape}"
        );
        engine
    }

    let wide = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]},{"type":"paragraph","content":[{"type":"text","text":"middle"}]},{"type":"paragraph","content":[{"type":"text","text":"last"}]}]}"#;
    for (shape, json, target, explicit_stored_bold) in [
        (
            "nested-list-item",
            r#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}]}"#,
            "abc",
            false,
        ),
        (
            "blockquote",
            r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}"#,
            "abc",
            false,
        ),
        ("first-top-level", wide, "first", false),
        ("middle-top-level", wide, "middle", false),
        ("last-top-level", wide, "last", false),
        (
            "explicit-stored-marks",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc","marks":[{"type":"bold"}]}]}]}"#,
            "abc",
            true,
        ),
    ] {
        let mut hit = fixture(shape, json, target, 1, explicit_stored_bold);
        let mut generic = fixture(shape, json, target, 1, explicit_stored_bold);
        reset_active_state_cache_counts_for_test();
        let hit_result = hit
            .apply_command(716_004, TypedCommand::InsertText { text: "y".into() })
            .unwrap()
            .unwrap();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 1, 0, 0, 1, 1, 0, 1, 0),
            "{shape} hit"
        );
        reset_active_state_cache_counts_for_test();
        force_active_state_cache_hit_fallback_for_test(true);
        let generic_result =
            generic.apply_command(716_004, TypedCommand::InsertText { text: "y".into() });
        force_active_state_cache_hit_fallback_for_test(false);
        let generic_result = generic_result.unwrap().unwrap();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 1, 1, 0, 1, 1),
            "{shape} generic"
        );
        assert_eq!(hit_result, generic_result, "{shape}");
        assert_eq!(hit.document_json(), generic.document_json(), "{shape}");
        assert_eq!(hit.can_undo(), generic.can_undo(), "{shape}");
        assert_eq!(hit.can_redo(), generic.can_redo(), "{shape}");
        let hit_state = hit.derived_state.as_ref().unwrap();
        let generic_state = generic.derived_state.as_ref().unwrap();
        assert_eq!(
            hit_state.validation_certificate, generic_state.validation_certificate,
            "{shape}"
        );
        assert_eq!(
            hit_state.localized_text_index, generic_state.localized_text_index,
            "{shape}"
        );
        assert_eq!(
            hit_state.render_blocks.materialize(),
            generic_state.render_blocks.materialize(),
            "{shape}"
        );
        assert_eq!(
            hit_state.active_state_cache_for_test().unwrap().value(),
            generic_state.active_state_cache_for_test().unwrap().value(),
            "{shape}"
        );
    }
}

#[test]
fn prepared_insert_compilation_uses_localized_semantics_after_planner_step() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 700_137,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();

    engine.ensure_mutation_lookup_seed(700_138).unwrap();
    engine
        .derived_state
        .as_mut()
        .unwrap()
        .materialize_mutation_identity();
    reset_canonical_artifact_counts_for_test();
    let preparation = std::cell::RefCell::new(None);
    let plan = engine
        .plan_command_internal(
            700_138,
            TypedCommand::InsertText { text: "x".into() },
            Some(&preparation),
        )
        .unwrap();
    let CommandPlan::Transaction(transaction) = plan else {
        panic!("insert command must produce a transaction");
    };
    let proof = preparation.into_inner().unwrap();
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 0));

    reset_full_pass_counts_for_test();
    let compiled = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap();
    assert!(compiled.localized_insert_admission.is_some());
    assert_eq!(
        take_full_pass_counts_for_test().ordinary_step_applications,
        0
    );
}

#[test]
fn stage4b2_prepared_same_leaf_insert_avoids_postwrite_relative_selection_traversals() {
    use crate::yrs_engine::derived_state::{
        reset_prewrite_selection_proof_counts_for_test,
        reset_relative_selection_traversal_counts_for_test,
        take_prewrite_selection_proof_counts_for_test,
        take_relative_selection_traversal_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 700_153,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();

    reset_relative_selection_traversal_counts_for_test();
    reset_prewrite_selection_proof_counts_for_test();
    let result = engine
        .apply_command(700_154, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert_eq!(
        result.selection,
        engine.resolved_selection().unwrap().clone()
    );
    assert_eq!(take_relative_selection_traversal_counts_for_test(), (0, 0));
    assert_eq!(
        take_prewrite_selection_proof_counts_for_test(),
        (1, 1, 0, 1)
    );
}

#[test]
fn stage4b2_prepared_selection_tamper_fails_closed_to_generic_parity() {
    use crate::yrs_engine::derived_state::{
        reset_prewrite_selection_proof_counts_for_test,
        reset_relative_selection_traversal_counts_for_test,
        take_prewrite_selection_proof_counts_for_test,
        take_relative_selection_traversal_counts_for_test,
    };

    fn fixture(snapshot: &crate::yrs_engine::DocumentSnapshot) -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine.restore_snapshot(snapshot).unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 700_155,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
    }

    fn prepared_insert(engine: &YrsDocumentEngine, request_id: u64) -> CompiledTransaction {
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("insert command must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();
        engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap()
    }

    let mut baseline = transaction_engine();
    baseline
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = baseline.export_snapshot().unwrap();

    let mut tampered = fixture(&snapshot);
    reset_prewrite_selection_proof_counts_for_test();
    let mut compiled = prepared_insert(&tampered, 700_156);
    compiled.prepared_selection_state = Some(
        compiled
            .prepared_selection_state
            .as_ref()
            .unwrap()
            .tampered_for_test()
            .swap_remove(0),
    );
    reset_relative_selection_traversal_counts_for_test();
    let tampered_result = tampered
        .apply_compiled_transaction(compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
    assert_eq!(
        take_prewrite_selection_proof_counts_for_test(),
        (1, 1, 1, 0)
    );

    let mut generic = fixture(&snapshot);
    let mut generic_compiled = prepared_insert(&generic, 700_156);
    generic_compiled.prepared_selection_state = None;
    generic_compiled.prepared_selection_mutation_seal = None;
    reset_relative_selection_traversal_counts_for_test();
    let generic_result = generic
        .apply_compiled_transaction(generic_compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
    assert_eq!(tampered_result, generic_result);
    assert_eq!(tampered.document_json(), generic.document_json());
    assert_eq!(tampered.relative_selection(), generic.relative_selection());
    assert_eq!(tampered.resolved_selection(), generic.resolved_selection());
    assert_eq!(tampered.can_undo(), generic.can_undo());

    let mut optimized = fixture(&snapshot);
    let optimized_result = optimized
        .apply_command(700_156, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    assert_eq!(optimized_result, generic_result);
    assert_eq!(optimized.document_json(), generic.document_json());
    assert_eq!(optimized.relative_selection(), generic.relative_selection());
    assert_eq!(optimized.resolved_selection(), generic.resolved_selection());
    assert_eq!(optimized.can_undo(), generic.can_undo());

    assert_eq!(
        tampered.undo(700_157).unwrap(),
        generic.undo(700_157).unwrap()
    );
    optimized.undo(700_157).unwrap();
    assert_eq!(tampered.document_json(), generic.document_json());
    assert_eq!(optimized.document_json(), generic.document_json());
    assert_eq!(
        tampered.redo(700_158).unwrap(),
        generic.redo(700_158).unwrap()
    );
    optimized.redo(700_158).unwrap();
    assert_eq!(tampered.document_json(), generic.document_json());
    assert_eq!(optimized.document_json(), generic.document_json());

    for tamper_index in 0..3 {
        let mut engine = fixture(&snapshot);
        reset_prewrite_selection_proof_counts_for_test();
        let mut compiled = prepared_insert(&engine, 700_160 + tamper_index as u64);
        compiled.prepared_selection_state = Some(
            compiled
                .prepared_selection_state
                .as_ref()
                .unwrap()
                .tampered_for_test()
                .swap_remove(tamper_index),
        );
        reset_relative_selection_traversal_counts_for_test();
        engine.apply_compiled_transaction(compiled, true).unwrap();
        assert_eq!(
            take_prewrite_selection_proof_counts_for_test(),
            (1, 1, 1, 0)
        );
        assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
    }

    let mut engine = fixture(&snapshot);
    let before = atomic_audit(&engine);
    reset_prewrite_selection_proof_counts_for_test();
    let mut compiled = prepared_insert(&engine, 700_163);
    compiled.prepared_selection_mutation_seal = None;
    reset_relative_selection_traversal_counts_for_test();
    let error = engine
        .apply_compiled_transaction(compiled, true)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        take_prewrite_selection_proof_counts_for_test(),
        (1, 1, 0, 0)
    );
    assert_eq!(take_relative_selection_traversal_counts_for_test(), (0, 0));

    for case in [
        "actionIndex",
        "actionLength",
        "admissionResult",
        "origin",
        "history",
        "selectionPlan",
        "epoch",
        "revision",
    ] {
        let mut engine = fixture(&snapshot);
        let before = atomic_audit(&engine);
        reset_prewrite_selection_proof_counts_for_test();
        let mut compiled = prepared_insert(&engine, 700_164);
        match case {
            "actionIndex" => {
                let [YrsMutationAction::InsertText { index_utf16, .. }] =
                    compiled.mutation_plan.actions.as_mut_slice()
                else {
                    unreachable!()
                };
                *index_utf16 = index_utf16.saturating_add(1);
            }
            "actionLength" => {
                let [YrsMutationAction::InsertText { len_utf16, .. }] =
                    compiled.mutation_plan.actions.as_mut_slice()
                else {
                    unreachable!()
                };
                *len_utf16 = len_utf16.saturating_add(1);
            }
            "admissionResult" => {
                let admission = compiled.localized_insert_admission.as_ref().unwrap();
                compiled.localized_insert_admission = Some(
                    admission
                        .tampered_claims_for_test()
                        .into_iter()
                        .find(|(claim, _)| *claim == "operationResult")
                        .unwrap()
                        .1,
                );
            }
            "origin" => compiled.origin = TransactionOrigin::LocalInput,
            "history" => compiled.history_policy = HistoryPolicy::Auto,
            "selectionPlan" => {
                compiled.selection_plan = SelectionPlan::Explicit(Selection::cursor(1));
            }
            "epoch" => compiled.yrs_state_epoch = compiled.yrs_state_epoch.saturating_add(1),
            "revision" => {
                compiled.base_state_revision = compiled.base_state_revision.saturating_add(1);
            }
            _ => unreachable!(),
        }
        let authority = crate::yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(
            engine.derived_state.as_ref().unwrap(),
        );
        assert!(
            !compiled
                .prepared_selection_mutation_seal
                .as_ref()
                .unwrap()
                .matches(&compiled, &authority),
            "{case}"
        );
        reset_relative_selection_traversal_counts_for_test();
        let error = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
        assert_eq!(
            take_prewrite_selection_proof_counts_for_test(),
            (1, 1, 0, 0),
            "{case}"
        );
        assert_eq!(
            take_relative_selection_traversal_counts_for_test(),
            (0, 0),
            "{case}"
        );
    }

    let mut engine = fixture(&snapshot);
    let before = atomic_audit(&engine);
    reset_prewrite_selection_proof_counts_for_test();
    let mut compiled = prepared_insert(&engine, 700_165);
    let original_target = match compiled.mutation_plan.actions.as_slice() {
        [YrsMutationAction::InsertText { target, .. }] => {
            <XmlTextRef as AsRef<Branch>>::as_ref(target).id()
        }
        _ => unreachable!(),
    };
    let foreign = utf16_doc();
    {
        let update = Update::decode_v1(&snapshot.encoded_state).unwrap();
        foreign.transact_mut().apply_update(update).unwrap();
    }
    let foreign_text = {
        let txn = foreign.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            unreachable!()
        };
        let XmlOut::Text(text) = paragraph.get(&txn, 0).unwrap() else {
            unreachable!()
        };
        text
    };
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&foreign_text).id(),
        original_target
    );
    let [YrsMutationAction::InsertText { target, .. }] =
        compiled.mutation_plan.actions.as_mut_slice()
    else {
        unreachable!()
    };
    *target = foreign_text;
    {
        let authority = crate::yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(
            engine.derived_state.as_ref().unwrap(),
        );
        assert!(!compiled
            .prepared_selection_mutation_seal
            .as_ref()
            .unwrap()
            .matches(&compiled, &authority));
    }
    let error = engine
        .apply_compiled_transaction(compiled, true)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        take_prewrite_selection_proof_counts_for_test(),
        (1, 1, 0, 0)
    );
}

#[test]
fn stage4b2_direct_local_insert_does_not_enter_prewrite_selection_proof_lifecycle() {
    use crate::yrs_engine::derived_state::{
        reset_prewrite_selection_proof_counts_for_test,
        reset_relative_selection_traversal_counts_for_test,
        take_prewrite_selection_proof_counts_for_test,
        take_relative_selection_traversal_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    reset_prewrite_selection_proof_counts_for_test();
    reset_relative_selection_traversal_counts_for_test();
    engine
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 700_159,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert_eq!(
        take_prewrite_selection_proof_counts_for_test(),
        (0, 0, 0, 0)
    );
    assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
}

#[test]
fn stage4b2_prepared_failpoints_never_install_selection_proof() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::derived_state::{
        reset_prewrite_selection_proof_counts_for_test,
        take_prewrite_selection_proof_counts_for_test,
    };

    for failpoint in [
        AtomicFailpoint::CanonicalOutputAdmission,
        AtomicFailpoint::FinalPreflight,
        AtomicFailpoint::EncodedAdmission,
        AtomicFailpoint::RevisionAdmission,
        AtomicFailpoint::DurableMetadataAdmission,
    ] {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 700_166,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                700_167,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            unreachable!()
        };
        reset_prewrite_selection_proof_counts_for_test();
        let compiled = engine
            .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
            .unwrap();
        let before = atomic_audit(&engine);
        set_atomic_failpoint_for_test(Some(failpoint));
        let error = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap_err();
        set_atomic_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        assert_eq!(
            take_prewrite_selection_proof_counts_for_test(),
            (1, 1, 0, 0),
            "{failpoint:?}"
        );
    }
}

#[test]
fn stage4b2_optimized_selection_matches_generic_matrix() {
    fn fixture(snapshot: &crate::yrs_engine::DocumentSnapshot, offset: u32) -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine.restore_snapshot(snapshot).unwrap();
        let point = RevisionedPosition {
            offset,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 700_170,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
    }

    fn prepared_insert(
        engine: &YrsDocumentEngine,
        request_id: u64,
        text: &str,
    ) -> CompiledTransaction {
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id,
                TypedCommand::InsertText { text: text.into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("insert command must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();
        engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap()
    }

    let cases = [
        (
            "non-bmp",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            1,
            "🙂",
        ),
        (
            "marked-fragmented",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"ab"},{"type":"text","marks":[{"type":"italic"}],"text":"cd"}]}]}"#,
            3,
            "x",
        ),
        (
            "nested",
            r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}"#,
            1,
            "x",
        ),
    ];

    for (index, (case, json, offset, inserted)) in cases.into_iter().enumerate() {
        let request_id = 700_171 + index as u64;
        let mut baseline = transaction_engine();
        baseline
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        let snapshot = baseline.export_snapshot().unwrap();
        let mut optimized = fixture(&snapshot, offset);
        let optimized_result = optimized
            .apply_command(
                request_id,
                TypedCommand::InsertText {
                    text: inserted.into(),
                },
            )
            .unwrap()
            .unwrap();

        let mut generic = fixture(&snapshot, offset);
        let mut compiled = prepared_insert(&generic, request_id, inserted);
        assert!(compiled.prepared_selection_state.is_some(), "{case}");
        compiled.prepared_selection_state = None;
        compiled.prepared_selection_mutation_seal = None;
        let generic_result = generic
            .apply_compiled_transaction(compiled, true)
            .unwrap()
            .1
            .unwrap();

        assert_eq!(optimized_result, generic_result, "{case}");
        assert_eq!(optimized.document_json(), generic.document_json(), "{case}");
        assert_eq!(
            optimized.relative_selection(),
            generic.relative_selection(),
            "{case}"
        );
        assert_eq!(
            optimized.resolved_selection(),
            generic.resolved_selection(),
            "{case}"
        );
        assert_eq!(
            optimized.derived_state.as_ref().unwrap().legacy_selection,
            generic.derived_state.as_ref().unwrap().legacy_selection,
            "{case}"
        );
        assert_eq!(optimized.can_undo(), generic.can_undo(), "{case}");
        assert_eq!(
            optimized.undo(700_180).unwrap(),
            generic.undo(700_180).unwrap(),
            "{case}"
        );
        assert_eq!(optimized.document_json(), generic.document_json(), "{case}");
        assert_eq!(
            optimized.redo(700_181).unwrap(),
            generic.redo(700_181).unwrap(),
            "{case}"
        );
        assert_eq!(optimized.document_json(), generic.document_json(), "{case}");
    }
}

#[test]
fn stage4b2_wide_deep_selection_traversal_counts_are_constant() {
    use crate::yrs_engine::derived_state::{
        reset_prewrite_selection_proof_counts_for_test,
        reset_relative_selection_traversal_counts_for_test,
        take_prewrite_selection_proof_counts_for_test,
        take_relative_selection_traversal_counts_for_test,
    };

    let mut observed = Vec::new();
    for factor in [1usize, 2] {
        let mut nested = json!({
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abc" }]
        });
        for _ in 0..(factor * 3) {
            nested = json!({ "type": "blockquote", "content": [nested] });
        }
        let mut content = vec![nested];
        content.extend((1..factor * 32).map(|index| {
            json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": format!("{index:04} abc") }]
            })
        }));
        let mut engine = transaction_engine();
        engine
            .import_json(
                &json!({ "type": "doc", "content": content }).to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 700_190 + factor as u64,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        reset_prewrite_selection_proof_counts_for_test();
        reset_relative_selection_traversal_counts_for_test();
        engine
            .apply_command(
                700_192 + factor as u64,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap()
            .unwrap();
        observed.push((
            take_prewrite_selection_proof_counts_for_test(),
            take_relative_selection_traversal_counts_for_test(),
        ));
    }
    assert_eq!(observed[0], observed[1]);
    assert_eq!(observed[0], ((1, 1, 0, 1), (0, 0)));
}

#[test]
fn prepared_command_preserves_semantic_output_error_before_yrs_scan_admission() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            &json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "x".repeat(4_096) }]
                }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.editing_limits.max_derived_output_bytes = 1;
    engine.resource_limits.max_input_bytes = 128;
    let command = TypedCommand::InsertText { text: "y".into() };

    let probe = engine.plan_command(70_005, command.clone()).unwrap_err();
    let exact = usize::try_from(probe.actual.unwrap()).unwrap();
    engine.editing_limits.max_derived_output_bytes = exact;
    assert!(engine.plan_command(70_005, command.clone()).is_ok());
    let before = atomic_audit(&engine);
    let scan_error = engine.apply_command(70_005, command.clone()).unwrap_err();
    assert_eq!(
        scan_error.details,
        Some(json!({ "field": "maxInputBytes" })),
        "{scan_error:?}",
    );
    assert_eq!(atomic_audit(&engine), before);

    engine.editing_limits.max_derived_output_bytes = exact - 1;
    let planned_error = engine.plan_command(70_005, command.clone()).unwrap_err();
    assert_eq!(planned_error.operation_index, Some(0));
    assert_eq!(planned_error.actual, Some(exact as u64));
    assert_eq!(
        planned_error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );

    let applied_error = engine.apply_command(70_005, command).unwrap_err();

    assert_eq!(applied_error, planned_error);
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_command_preserves_semantic_undo_error_before_yrs_scan_admission() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            &json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "x".repeat(4_096) }]
                }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.editing_limits.max_undo_retained_units = 0;
    engine.resource_limits.max_input_bytes = 128;
    let command = TypedCommand::InsertText { text: "y".into() };

    let probe = engine.plan_command(70_006, command.clone()).unwrap_err();
    let exact = probe.actual.unwrap();
    engine.editing_limits.max_undo_retained_units = exact;
    assert!(engine.plan_command(70_006, command.clone()).is_ok());
    let before = atomic_audit(&engine);
    let scan_error = engine.apply_command(70_006, command.clone()).unwrap_err();
    assert_eq!(
        scan_error.details,
        Some(json!({ "field": "maxInputBytes" })),
        "{scan_error:?}",
    );
    assert_eq!(atomic_audit(&engine), before);

    engine.editing_limits.max_undo_retained_units = exact - 1;
    let planned_error = engine.plan_command(70_006, command.clone()).unwrap_err();
    assert_eq!(planned_error.operation_index, Some(0));
    assert_eq!(planned_error.actual, Some(exact));
    assert_eq!(
        planned_error.details,
        Some(json!({ "field": "maxUndoRetainedUnits" }))
    );

    let applied_error = engine.apply_command(70_006, command).unwrap_err();

    assert_eq!(applied_error, planned_error);
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_insert_applies_collapsed_stored_marks_in_one_compilation() {
    use crate::yrs_engine::compiler::{
        reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .apply_command(
            70_010,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        engine.stored_marks().unwrap(),
        &[Mark::new("bold".into(), HashMap::new())]
    );
    reset_semantic_compilation_count_for_test();

    engine
        .apply_command(70_011, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    assert_eq!(
        engine.document_json().unwrap(),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "x",
                    "marks": [{ "type": "bold" }]
                }]
            }]
        })
    );
    assert_eq!(engine.stored_marks(), None);
}

#[test]
fn delete_empty_block_compiles_once_with_exact_selection() {
    use crate::yrs_engine::compiler::{
        reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]},{"type":"paragraph"}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let scalar = engine
        .position_map()
        .unwrap()
        .doc_to_scalar(4, engine.document().unwrap());
    let point = RevisionedPosition {
        offset: scalar,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_020,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    reset_semantic_compilation_count_for_test();

    let result = engine
        .apply_command(70_021, TypedCommand::DeleteBackward)
        .unwrap()
        .unwrap();

    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    assert_eq!(
        engine.document_json().unwrap(),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "a" }]
            }]
        })
    );
    let crate::yrs_engine::ResolvedSelection::Text { anchor, head } = result.selection else {
        panic!("structural fallback must preserve a text selection");
    };
    assert_eq!((anchor.scalar, head.scalar), (1, 1));
    assert!(result.history_state.can_undo);
}

#[test]
fn ambiguous_wrap_in_list_keeps_the_public_proof_path() {
    use crate::yrs_engine::compiler::{
        reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let engine = transaction_engine();
    reset_semantic_compilation_count_for_test();
    reset_full_pass_counts_for_test();

    let plan = engine
        .plan_command(
            70_030,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap();

    assert!(matches!(plan, CommandPlan::Transaction(_)));
    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    assert_eq!(take_full_pass_counts_for_test().planner_simulations, 1);
    assert_eq!(
        engine.document_json().unwrap()["content"][0]["type"],
        "paragraph"
    );
}

#[test]
fn prepared_toggle_mark_uses_no_eager_whole_tree_collectors() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, reset_range_format_lowering_counts_for_test,
        take_localized_lookup_counts_for_test, take_range_format_lowering_counts_for_test,
    };

    let mut content = Vec::with_capacity(161);
    content.push(json!({
        "type": "h1",
        "content": [{ "type": "text", "text": "h".repeat(42) }]
    }));
    for index in 0..160 {
        let inline = if index == 0 {
            vec![
                json!({ "type": "text", "text": "p".repeat(55) }),
                json!({
                    "type": "text",
                    "text": "b".repeat(55),
                    "marks": [{ "type": "bold" }]
                }),
                json!({
                    "type": "text",
                    "text": "i".repeat(55),
                    "marks": [{ "type": "italic" }]
                }),
                json!({ "type": "text", "text": "t".repeat(55) }),
            ]
        } else {
            vec![json!({
                "type": "text",
                "text": format!("{index:04} {}", "x".repeat(215))
            })]
        };
        content.push(json!({ "type": "paragraph", "content": inline }));
    }
    let mut engine = transaction_engine();
    engine
        .import_json(
            &json!({ "type": "doc", "content": content }).to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_text(&mut engine, 70_030_000, 44, 52);
    hydrate_import_for_compile_test(&mut engine);

    let before_document = engine.document_json().unwrap();
    let before_selection = engine.resolved_selection().unwrap().clone();
    let mut expected_document = before_document.clone();
    let inline = expected_document["content"][1]["content"]
        .as_array_mut()
        .unwrap();
    inline.splice(
        0..1,
        [
            json!({ "type": "text", "text": "p" }),
            json!({
                "type": "text",
                "text": "p".repeat(8),
                "marks": [{ "type": "bold" }]
            }),
            json!({ "type": "text", "text": "p".repeat(46) }),
        ],
    );

    reset_localized_lookup_counts_for_test();
    reset_range_format_lowering_counts_for_test();
    let result = engine
        .apply_command(
            70_030_001,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();

    assert_eq!(engine.document_json().unwrap(), expected_document);
    assert_eq!(result.selection, before_selection);
    assert_eq!(engine.resolved_selection().unwrap(), &before_selection);
    assert!(result.history_state.can_undo);
    assert!(!result.history_state.can_redo);
    assert!(engine.can_undo());
    assert!(!engine.can_redo());
    let range_format_counts = take_range_format_lowering_counts_for_test();
    let localized_lookup_counts = take_localized_lookup_counts_for_test();
    assert_eq!(localized_lookup_counts, (0, 0, 0));
    assert_eq!(range_format_counts, (0, 0, 1, 0));

    engine.undo(70_030_002).unwrap().unwrap();
    assert_eq!(engine.document_json().unwrap(), before_document);
    assert_eq!(engine.resolved_selection().unwrap(), &before_selection);
    assert!(!engine.can_undo());
    assert!(engine.can_redo());
}

#[test]
fn prepared_reverse_toggle_mark_matches_public_eager_transaction_result() {
    let document = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "a😀", "marks": [{ "type": "italic" }] },
                { "type": "text", "text": "bc" },
                { "type": "text", "text": "🦀d", "marks": [{ "type": "bold" }] },
                { "type": "text", "text": "ef" }
            ]
        }]
    });
    let populated = || {
        let mut engine = transaction_engine();
        engine
            .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        select_text(&mut engine, 70_030_100, 7, 1);
        engine
    };
    let command = TypedCommand::ToggleMark {
        mark_type: "bold".into(),
    };

    let mut prepared = populated();
    let prepared_result = prepared
        .apply_command(70_030_101, command.clone())
        .unwrap()
        .unwrap();

    let mut generic = populated();
    let CommandPlan::Transaction(transaction) = generic.plan_command(70_030_101, command).unwrap()
    else {
        panic!("reverse toggle-mark must produce a transaction")
    };
    let generic_result = generic
        .apply_typed_transaction_with_result(transaction)
        .unwrap();

    assert_eq!(prepared_result, generic_result);
    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(prepared.document_html(), generic.document_html());
    assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
    assert_eq!(prepared.stored_marks(), generic.stored_marks());
    assert_eq!(prepared.can_undo(), generic.can_undo());
    assert_eq!(prepared.can_redo(), generic.can_redo());
}

#[test]
fn toggle_mark_structural_ranges_reject_before_lowering_with_public_parity() {
    use crate::yrs_engine::mutation::{
        reset_range_format_lowering_counts_for_test, take_range_format_lowering_counts_for_test,
    };

    let cases = [
        (
            "crossBlock",
            json!({
                "type": "doc",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
                ]
            }),
            0,
            5,
            (0, 0, 0, 0),
        ),
        (
            "inlineVoid",
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [
                        { "type": "text", "text": "a" },
                        { "type": "hardBreak" },
                        { "type": "text", "text": "b" }
                    ]
                }]
            }),
            0,
            3,
            (1, 1, 0, 1),
        ),
    ];

    for (case, document, anchor, head, expected_counts) in cases {
        let populated = || {
            let mut engine = transaction_engine();
            engine
                .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
                .unwrap();
            select_text(&mut engine, 70_030_200, anchor, head);
            engine
        };
        let command = TypedCommand::ToggleMark {
            mark_type: "bold".into(),
        };

        let mut prepared = populated();
        let prepared_before = atomic_audit(&prepared);
        reset_range_format_lowering_counts_for_test();
        let prepared_error = prepared
            .apply_command(70_030_201, command.clone())
            .unwrap_err();
        assert_eq!(
            take_range_format_lowering_counts_for_test(),
            expected_counts,
            "{case}"
        );
        assert_eq!(atomic_audit(&prepared), prepared_before, "{case}");

        let mut generic = populated();
        let generic_before = atomic_audit(&generic);
        reset_range_format_lowering_counts_for_test();
        let plan = generic.plan_command(70_030_201, command);
        let generic_error = if case == "crossBlock" {
            let error = plan.unwrap_err();
            assert_eq!(
                take_range_format_lowering_counts_for_test(),
                (0, 0, 0, 0),
                "{case} public plan"
            );
            error
        } else {
            let CommandPlan::Transaction(transaction) = plan.unwrap() else {
                panic!("{case} must produce a public typed transaction")
            };
            assert_eq!(
                take_range_format_lowering_counts_for_test(),
                (0, 0, 0, 0),
                "{case} public plan"
            );
            reset_range_format_lowering_counts_for_test();
            let error = generic
                .apply_typed_transaction_with_result(transaction)
                .unwrap_err();
            assert_eq!(
                take_range_format_lowering_counts_for_test(),
                (1, 1, 0, 0),
                "{case} public apply"
            );
            error
        };
        assert_eq!(prepared_error, generic_error, "{case}");
        assert_eq!(atomic_audit(&generic), generic_before, "{case}");
    }
}

#[test]
fn prepared_toggle_mark_exact_limits_and_one_under_errors_match_public_eager() {
    use crate::yrs_engine::{OperationResult, TypedTransactionResult};

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀bc🦀def"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut engine, 70_030_300, 0, 8);
        engine
    }

    fn command() -> TypedCommand {
        TypedCommand::ToggleMark {
            mark_type: "bold".into(),
        }
    }

    fn public_eager_apply(
        engine: &mut YrsDocumentEngine,
        request_id: u64,
    ) -> OperationResult<TypedTransactionResult> {
        let CommandPlan::Transaction(transaction) = engine.plan_command(request_id, command())?
        else {
            panic!("range ToggleMark must produce a typed transaction")
        };
        engine.apply_typed_transaction_with_result(transaction)
    }

    fn prepared_apply(
        engine: &mut YrsDocumentEngine,
        request_id: u64,
    ) -> OperationResult<TypedTransactionResult> {
        Ok(engine
            .apply_command(request_id, command())?
            .expect("range ToggleMark must produce a transaction result"))
    }

    fn set_limit(engine: &mut YrsDocumentEngine, field: &str, value: u64) {
        match field {
            "maxUndoRetainedUnits" => {
                engine.editing_limits.max_undo_retained_units = value;
            }
            "maxInputBytes" => {
                engine.resource_limits.max_input_bytes = usize::try_from(value).unwrap();
            }
            "maxDerivedOutputBytes" => {
                engine.editing_limits.max_derived_output_bytes = usize::try_from(value).unwrap();
            }
            "maxEncodedStateBytes" => {
                engine.resource_limits.max_encoded_state_bytes = usize::try_from(value).unwrap();
            }
            _ => unreachable!(),
        }
    }

    fn exact_limit(field: &str) -> u64 {
        let mut limit = 0;
        loop {
            let mut probe = fixture();
            set_limit(&mut probe, field, limit);
            match public_eager_apply(&mut probe, 70_030_301) {
                Ok(_) => return limit,
                Err(error) => {
                    assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
                    let actual = error.actual.expect("limit rejection must report actual");
                    assert!(actual > limit, "{field} probe must make progress");
                    limit = actual;
                }
            }
        }
    }

    let exact_limits = [
        ("maxUndoRetainedUnits", exact_limit("maxUndoRetainedUnits")),
        ("maxInputBytes", exact_limit("maxInputBytes")),
        (
            "maxDerivedOutputBytes",
            exact_limit("maxDerivedOutputBytes"),
        ),
    ];

    for (index, (field, exact)) in exact_limits.into_iter().enumerate() {
        let request_id = 70_030_310 + u64::try_from(index).unwrap();
        let mut prepared = fixture();
        set_limit(&mut prepared, field, exact);
        let prepared_result = prepared
            .apply_command(request_id, command())
            .unwrap()
            .unwrap();
        let mut generic = fixture();
        set_limit(&mut generic, field, exact);
        let generic_result = public_eager_apply(&mut generic, request_id).unwrap();
        assert_eq!(prepared_result, generic_result, "{field} exact");
        assert_eq!(
            prepared.document_json(),
            generic.document_json(),
            "{field} exact"
        );
        assert_eq!(
            prepared.document_html(),
            generic.document_html(),
            "{field} exact"
        );
        assert_eq!(
            prepared.resolved_selection(),
            generic.resolved_selection(),
            "{field} exact"
        );
        assert_eq!(
            prepared.stored_marks(),
            generic.stored_marks(),
            "{field} exact"
        );
        assert_eq!(prepared.can_undo(), generic.can_undo(), "{field} exact");
        assert_eq!(prepared.can_redo(), generic.can_redo(), "{field} exact");

        let limit = exact
            .checked_sub(1)
            .expect("ToggleMark limits must be nonzero");
        let mut rejected_prepared = fixture();
        set_limit(&mut rejected_prepared, field, limit);
        let prepared_before = atomic_audit(&rejected_prepared);
        let prepared_error = rejected_prepared
            .apply_command(request_id, command())
            .unwrap_err();
        assert_eq!(
            atomic_audit(&rejected_prepared),
            prepared_before,
            "{field} prepared"
        );

        let mut rejected_generic = fixture();
        set_limit(&mut rejected_generic, field, limit);
        let generic_before = atomic_audit(&rejected_generic);
        let generic_error = public_eager_apply(&mut rejected_generic, request_id).unwrap_err();
        assert_eq!(
            atomic_audit(&rejected_generic),
            generic_before,
            "{field} generic"
        );

        assert_eq!(prepared_error, generic_error, "{field}");
        assert_eq!(
            prepared_error.details,
            Some(json!({ "field": field })),
            "{field}"
        );
        assert_eq!(prepared_error.limit, Some(limit), "{field}");
        assert_eq!(prepared_error.actual, Some(exact), "{field}");
    }

    fn exercise_max_encoded_state_boundary(
        request_id: u64,
        apply: fn(&mut YrsDocumentEngine, u64) -> OperationResult<TypedTransactionResult>,
    ) -> (YrsDocumentEngine, TypedTransactionResult) {
        let field = "maxEncodedStateBytes";
        let mut engine = fixture();
        let before = atomic_audit(&engine);
        let current_encoded = u64::try_from(engine.encoded_state().unwrap().len()).unwrap();
        set_limit(&mut engine, field, current_encoded);
        let probe_error = apply(&mut engine, request_id).unwrap_err();
        assert_eq!(atomic_audit(&engine), before, "{field} probe");
        assert_eq!(probe_error.details, Some(json!({ "field": field })));
        let exact = probe_error
            .actual
            .expect("encoded-state rejection must report the exact instance size");
        let one_under = exact
            .checked_sub(1)
            .expect("encoded state must consume at least one byte");

        set_limit(&mut engine, field, one_under);
        let one_under_error = apply(&mut engine, request_id).unwrap_err();
        assert_eq!(atomic_audit(&engine), before, "{field} one-under");
        assert_eq!(one_under_error.details, Some(json!({ "field": field })));
        assert_eq!(one_under_error.limit, Some(one_under));
        assert_eq!(one_under_error.actual, Some(exact));

        set_limit(&mut engine, field, exact);
        let result = apply(&mut engine, request_id).unwrap();
        assert!(engine.encoded_state().unwrap().len() <= usize::try_from(exact).unwrap());
        (engine, result)
    }

    let request_id = 70_030_320;
    let (prepared, prepared_result) =
        exercise_max_encoded_state_boundary(request_id, prepared_apply);
    let (generic, generic_result) =
        exercise_max_encoded_state_boundary(request_id, public_eager_apply);
    assert_eq!(
        prepared_result, generic_result,
        "maxEncodedStateBytes exact"
    );
    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(prepared.document_html(), generic.document_html());
    assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
    assert_eq!(prepared.stored_marks(), generic.stored_marks());
    assert_eq!(prepared.can_undo(), generic.can_undo());
    assert_eq!(prepared.can_redo(), generic.can_redo());
}

#[test]
fn prepared_toggle_and_wrap_commands_each_simulate_and_compile_once() {
    use crate::yrs_engine::compiler::{
        reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let mut toggle = transaction_engine();
    toggle
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_text(&mut toggle, 70_031, 0, 2);
    hydrate_import_for_compile_test(&mut toggle);
    reset_semantic_compilation_count_for_test();
    reset_full_pass_counts_for_test();
    toggle
        .apply_command(
            70_032,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    let toggle_passes = take_full_pass_counts_for_test();
    assert_eq!(toggle_passes.planner_simulations, 1);
    assert_eq!(toggle_passes.document_validations, 1);
    assert_eq!(toggle_passes.canonical_mark_tree_scans, 1);
    assert_eq!(toggle_passes.canonical_projections, 1);
    assert_eq!(toggle_passes.canonical_serializations, 2);
    assert_eq!(toggle_passes.canonical_hashes, 1);
    assert_eq!(toggle_passes.position_map_clones, 0);
    assert_eq!(toggle_passes.position_map_compactions, 0);
    assert_eq!(toggle_passes.rendered_text_derivations, 1);

    let mut wrap = transaction_engine();
    hydrate_import_for_compile_test(&mut wrap);
    reset_semantic_compilation_count_for_test();
    reset_full_pass_counts_for_test();
    wrap.apply_command(
        70_033,
        TypedCommand::WrapInList {
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    let wrap_passes = take_full_pass_counts_for_test();
    assert_eq!(wrap_passes.planner_simulations, 1);
    assert_eq!(wrap_passes.document_validations, 1);
    assert_eq!(wrap_passes.canonical_mark_tree_scans, 1);
    assert_eq!(wrap_passes.canonical_projections, 1);
    assert_eq!(wrap_passes.canonical_serializations, 2);
    assert_eq!(wrap_passes.canonical_hashes, 1);
    assert_eq!(wrap_passes.position_map_clones, 0);
    assert_eq!(wrap_passes.position_map_compactions, 0);
    assert_eq!(wrap_passes.rendered_text_derivations, 1);
    assert_eq!(
        wrap.document_json().unwrap()["content"][0]["type"],
        "bulletList"
    );
}

#[test]
fn prepared_wrap_at_a_block_boundary_matches_its_simulated_selection() {
    let document = json!({
        "type": "doc",
        "content": [
            {
                "type": "h1",
                "content": [{ "type": "text", "text": "x".repeat(42) }]
            },
            {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "y".repeat(220) }]
            }
        ]
    });
    let populated = || {
        let mut engine = transaction_engine();
        engine
            .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        select_text(&mut engine, 70_033_001, 44, 44);
        engine
    };

    let mut prepared = populated();
    crate::yrs_engine::compiler::reset_semantic_compilation_count_for_test();
    let prepared_result = prepared
        .apply_command(
            70_033_002,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap();
    assert_eq!(
        crate::yrs_engine::compiler::take_semantic_compilation_count_for_test(),
        1
    );

    let mut generic = populated();
    let CommandPlan::Transaction(transaction) = generic
        .plan_command(
            70_033_002,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap()
    else {
        panic!("public block-boundary wrap must produce a transaction")
    };
    let generic_result = generic
        .apply_typed_transaction_with_result(transaction)
        .unwrap();

    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
    assert_eq!(prepared_result.unwrap().selection, generic_result.selection);
}

#[test]
fn prepared_article_wrap_uses_only_the_localized_root_window() {
    use crate::yrs_engine::mutation::{
        reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
    };

    let mut content = Vec::with_capacity(161);
    content.push(json!({
        "type": "h1",
        "content": [{ "type": "text", "text": "h".repeat(42) }]
    }));
    for index in 0..160 {
        content.push(json!({
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": format!("{index:04} {}", "x".repeat(215))
            }]
        }));
    }
    let document = json!({ "type": "doc", "content": content });
    let mut engine = transaction_engine();
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select_text(&mut engine, 70_033_100, 44, 44);
    hydrate_import_for_compile_test(&mut engine);

    let before_document = engine.document_json().unwrap();
    let before_selection = engine.resolved_selection().unwrap().clone();
    let before_revision = engine.revision();
    let mut expected_document = before_document.clone();
    let root_content = expected_document["content"].as_array_mut().unwrap();
    let paragraph = root_content.remove(1);
    root_content.insert(
        1,
        json!({
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [paragraph]
            }]
        }),
    );

    reset_root_window_lowering_counts_for_test();
    let result = engine
        .apply_command(
            70_033_101,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap()
        .unwrap();
    let observed_counts = take_root_window_lowering_counts_for_test();

    assert_eq!(result.request_id, 70_033_101);
    assert_eq!(result.origin, TransactionOrigin::LocalCommand);
    assert!(result.changed);
    assert_eq!(result.document_revision, before_revision + 1);
    assert_eq!(engine.document_json().unwrap(), expected_document);
    assert!(matches!(
        result.selection,
        ResolvedSelection::Text { ref anchor, ref head }
            if (anchor.scalar, head.scalar) == (46, 46)
    ));
    assert_eq!(engine.resolved_selection().unwrap(), &result.selection);
    assert!(result.history_state.can_undo);
    assert!(!result.history_state.can_redo);
    assert!(engine.can_undo());
    assert!(!engine.can_redo());

    reset_root_window_lowering_counts_for_test();
    engine.undo(70_033_102).unwrap().unwrap();
    assert_eq!(engine.document_json().unwrap(), before_document);
    assert_eq!(engine.resolved_selection().unwrap(), &before_selection);
    assert!(!engine.can_undo());
    assert!(engine.can_redo());

    let redo = engine.redo_with_result(70_033_103).unwrap().unwrap();
    assert_eq!(redo.request_id, 70_033_103);
    assert_eq!(redo.origin, TransactionOrigin::UndoRedo);
    assert!(redo.changed);
    assert_eq!(redo.document_revision, before_revision + 3);
    assert_eq!(engine.document_json().unwrap(), expected_document);
    assert!(matches!(
        redo.selection,
        ResolvedSelection::Text { ref anchor, ref head }
            if (anchor.scalar, head.scalar) == (46, 46)
    ));
    assert_eq!(engine.resolved_selection().unwrap(), &redo.selection);
    assert!(redo.history_state.can_undo);
    assert!(!redo.history_state.can_redo);
    assert!(engine.can_undo());
    assert!(!engine.can_redo());

    assert_eq!(observed_counts, (0, 0, 1, 0, 0, 1));
}

#[test]
fn prepared_wrap_proof_binds_the_exact_transaction_and_candidate_identity() {
    let compile = |engine: &YrsDocumentEngine, request_id| {
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("wrap command must produce a transaction")
        };
        (transaction, preparation.into_inner().unwrap())
    };

    let engine = transaction_engine();
    let before = atomic_audit(&engine);
    let (mut transaction, proof) = compile(&engine, 70_034);
    assert!(matches!(
        transaction.operations.as_slice(),
        [TypedOperation::ReplaceStructure(_)]
    ));
    transaction.selection_intent = SelectionIntent::Preserve;
    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);

    let (transaction, mut proof) = compile(&engine, 70_035);
    proof.document = engine.document().unwrap().clone();
    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);

    let (transaction, mut proof) = compile(&engine, 70_035_000);
    let base_artifact = engine
        .derived_state
        .as_ref()
        .unwrap()
        .canonical_artifact
        .clone();
    proof
        .eager_semantic_admission_mut_for_test()
        .replace_candidate_artifact_for_test(base_artifact);
    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_wrap_proof_rejects_resource_limit_context_drift() {
    let mut engine = transaction_engine();
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            70_035_001,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("prepared wrap must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    engine.resource_limits.max_schema_nodes -= 1;
    let before = atomic_audit(&engine);

    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_insert_without_candidate_certificate_runs_live_preview_validation() {
    use crate::yrs_engine::compiler::force_localized_semantic_allocation_failure_for_test;
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let engine = transaction_engine();
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            70_035_010,
            TypedCommand::InsertText { text: "x".into() },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("prepared insert must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();

    reset_full_pass_counts_for_test();
    force_localized_semantic_allocation_failure_for_test(true);
    let compiled = engine.compile_prepared_typed_transaction(transaction, proof);
    force_localized_semantic_allocation_failure_for_test(false);

    compiled.unwrap();
    let counts = take_full_pass_counts_for_test();
    assert!(counts.document_validations >= 1);
    assert!(counts.canonical_mark_tree_scans >= 1);
}

#[test]
fn prepared_insert_rejects_stale_root_and_foreign_canonical_context_artifacts() {
    let compile = |engine: &YrsDocumentEngine, request_id| {
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("prepared insert must produce a transaction")
        };
        (transaction, preparation.into_inner().unwrap())
    };

    let engine = transaction_engine();
    let separate = transaction_engine();
    let before = atomic_audit(&engine);

    let (transaction, mut proof) = compile(&engine, 70_035_011);
    let stale_root_artifact = engine
        .canonical_schema
        .derive(separate.document().unwrap())
        .unwrap();
    proof
        .eager_semantic_admission_mut_for_test()
        .replace_canonical_artifact_for_test(stale_root_artifact);
    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);

    let (transaction, mut proof) = compile(&engine, 70_035_012);
    let foreign_context_artifact = separate.canonical_schema.derive(&proof.document).unwrap();
    proof
        .eager_semantic_admission_mut_for_test()
        .replace_canonical_artifact_for_test(foreign_context_artifact);
    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_candidate_rejects_foreign_same_total_position_layout() {
    let engine = transaction_engine();
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            70_035_012_001,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("prepared wrap must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();

    let mut foreign = transaction_engine();
    foreign
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let foreign_map =
        crate::position::PositionMap::build(foreign.document().unwrap(), &engine.schema);
    let expected_map = crate::position::PositionMap::build(&proof.document, &engine.schema);
    assert_eq!(foreign_map.total_scalars(), expected_map.total_scalars());
    assert_ne!(
        foreign_map.block(0).unwrap().node_path,
        expected_map.block(0).unwrap().node_path
    );
    let foreign_seed = crate::yrs_engine::compiler::PreparedCandidateSeed::mint(
        transaction.request_id,
        foreign.document().unwrap(),
        &engine.schema,
        &engine.canonical_schema,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
    )
    .unwrap();

    let error = crate::yrs_engine::compiler::PreparedSemanticAdmission::prepare_single_operation(
        transaction.request_id,
        engine.revision,
        engine.state_revision,
        engine.yrs_state_epoch,
        &engine.schema,
        &engine.canonical_schema,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &transaction,
        &proof.document,
        Some(foreign_seed),
        None,
        0,
        crate::yrs_engine::compiler::PreparedCommandContractKind::None,
    )
    .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
}

#[test]
fn prepared_wrap_rejects_max_length_context_drift_atomically() {
    let mut engine = transaction_engine();
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            70_035_013,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("prepared wrap must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    engine.max_length = Some(0);
    let before = atomic_audit(&engine);

    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_wrap_rejects_editing_limit_context_drift_atomically() {
    let mut engine = transaction_engine();
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            70_035_014,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("prepared wrap must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    engine.editing_limits.max_undo_groups -= 1;
    let before = atomic_audit(&engine);

    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_wrap_hard_limit_rejection_is_atomic() {
    use crate::yrs_engine::mutation::{
        reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine.resource_limits.max_input_bytes = 0;
    let before = atomic_audit(&engine);

    reset_root_window_lowering_counts_for_test();
    let error = engine
        .apply_command(
            70_036,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap_err();
    let counts = take_root_window_lowering_counts_for_test();

    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.details, Some(json!({ "field": "maxInputBytes" })));
    assert_eq!((counts.2, counts.3), (0, 0));
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_wrap_accepts_exact_output_limit_and_rejects_one_over_atomically() {
    use crate::yrs_engine::mutation::{
        reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
    };

    let command = TypedCommand::WrapInList {
        list_type: "bulletList".into(),
        item_type: "listItem".into(),
    };
    let mut exact = 1;
    loop {
        let mut probe = transaction_engine();
        probe.editing_limits.max_derived_output_bytes = exact;
        match probe.apply_command(70_036_001, command.clone()) {
            Ok(Some(_)) => break,
            Err(error) if error.details == Some(json!({ "field": "maxDerivedOutputBytes" })) => {
                let required = usize::try_from(error.actual.unwrap()).unwrap();
                assert!(required > exact);
                exact = required;
            }
            outcome => panic!("unexpected output-limit probe result: {outcome:?}"),
        }
    }

    let mut exact_limit = transaction_engine();
    exact_limit.editing_limits.max_derived_output_bytes = exact;
    reset_root_window_lowering_counts_for_test();
    assert!(exact_limit
        .apply_command(70_036_002, command.clone())
        .unwrap()
        .is_some());
    let exact_counts = take_root_window_lowering_counts_for_test();
    assert_eq!((exact_counts.2, exact_counts.3), (1, 0));

    let mut one_over = transaction_engine();
    one_over.editing_limits.max_derived_output_bytes = exact - 1;
    let before = atomic_audit(&one_over);
    reset_root_window_lowering_counts_for_test();
    let error = one_over.apply_command(70_036_003, command).unwrap_err();
    let rejected_counts = take_root_window_lowering_counts_for_test();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.actual, Some(exact as u64));
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );
    assert_eq!((rejected_counts.2, rejected_counts.3), (1, 0));
    assert_eq!(atomic_audit(&one_over), before);
}

#[test]
fn prepared_wrap_undo_and_encoded_limits_match_public_eager_exactly() {
    use crate::yrs_engine::mutation::{
        reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
    };
    use crate::yrs_engine::{EditingLimits, OperationResult, TypedTransactionResult};

    fn command() -> TypedCommand {
        TypedCommand::WrapInList {
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
        }
    }

    fn fixture(field: &str, value: u64) -> YrsDocumentEngine {
        let mut resource_limits = ResourceLimits::default();
        let mut editing_limits = EditingLimits::default();
        match field {
            "maxUndoRetainedUnits" => editing_limits.max_undo_retained_units = value,
            "maxEncodedStateBytes" => {
                resource_limits.max_encoded_state_bytes = usize::try_from(value).unwrap()
            }
            _ => unreachable!(),
        }
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits,
            editing_limits,
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap()
    }

    fn public_eager_apply(
        engine: &mut YrsDocumentEngine,
        request_id: u64,
    ) -> OperationResult<TypedTransactionResult> {
        let CommandPlan::Transaction(transaction) = engine.plan_command(request_id, command())?
        else {
            panic!("WrapInList must produce a public typed transaction")
        };
        engine.apply_typed_transaction_with_result(transaction)
    }

    fn prepared_apply(
        engine: &mut YrsDocumentEngine,
        request_id: u64,
    ) -> OperationResult<TypedTransactionResult> {
        Ok(engine
            .apply_command(request_id, command())?
            .expect("WrapInList must produce a transaction result"))
    }

    fn exact_undo_limit() -> u64 {
        let field = "maxUndoRetainedUnits";
        let mut limit = 1;
        loop {
            let mut probe = fixture(field, limit);
            match public_eager_apply(&mut probe, 70_036_010) {
                Ok(_) => return limit,
                Err(error) => {
                    assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
                    let actual = error.actual.expect("limit rejection must report actual");
                    assert!(actual > limit, "{field} probe must make progress");
                    limit = actual;
                }
            }
        }
    }

    let field = "maxUndoRetainedUnits";
    let exact = exact_undo_limit();
    let request_id = 70_036_020;

    let mut prepared = fixture(field, exact);
    reset_root_window_lowering_counts_for_test();
    let prepared_result = prepared
        .apply_command(request_id, command())
        .unwrap()
        .unwrap();
    assert_eq!(
        take_root_window_lowering_counts_for_test(),
        (0, 0, 1, 0, 0, 1),
        "{field} prepared exact"
    );

    let mut generic = fixture(field, exact);
    reset_root_window_lowering_counts_for_test();
    let generic_result = public_eager_apply(&mut generic, request_id).unwrap();
    assert_eq!(
        take_root_window_lowering_counts_for_test(),
        (1, 1, 0, 0, 1, 0),
        "{field} eager exact"
    );
    assert_eq!(prepared_result, generic_result, "{field} exact");
    assert_eq!(prepared.document_json(), generic.document_json(), "{field}");
    assert_eq!(prepared.document_html(), generic.document_html(), "{field}");
    assert_eq!(
        prepared.resolved_selection(),
        generic.resolved_selection(),
        "{field}"
    );
    assert_eq!(prepared.stored_marks(), generic.stored_marks(), "{field}");
    assert_eq!(prepared.can_undo(), generic.can_undo(), "{field}");
    assert_eq!(prepared.can_redo(), generic.can_redo(), "{field}");

    let limit = exact.checked_sub(1).expect("wrap limits must be nonzero");
    let mut rejected_prepared = fixture(field, limit);
    let prepared_before = atomic_audit(&rejected_prepared);
    reset_root_window_lowering_counts_for_test();
    let prepared_error = rejected_prepared
        .apply_command(request_id, command())
        .unwrap_err();
    let prepared_counts = take_root_window_lowering_counts_for_test();
    assert_eq!(atomic_audit(&rejected_prepared), prepared_before, "{field}");

    let mut rejected_generic = fixture(field, limit);
    let generic_before = atomic_audit(&rejected_generic);
    reset_root_window_lowering_counts_for_test();
    let generic_error = public_eager_apply(&mut rejected_generic, request_id).unwrap_err();
    let generic_counts = take_root_window_lowering_counts_for_test();
    assert_eq!(atomic_audit(&rejected_generic), generic_before, "{field}");
    assert_eq!(prepared_error, generic_error, "{field}");
    assert_eq!(
        prepared_error.details,
        Some(json!({ "field": field })),
        "{field}"
    );
    assert_eq!(prepared_error.limit, Some(limit), "{field}");
    assert_eq!(prepared_error.actual, Some(exact), "{field}");

    let expected_prepared_counts = (0, 0, 1, 0, 0, 0);
    let expected_generic_counts = (1, 1, 0, 0, 0, 0);
    assert_eq!(
        prepared_counts, expected_prepared_counts,
        "{field} prepared reject"
    );
    assert_eq!(
        generic_counts, expected_generic_counts,
        "{field} eager reject"
    );

    fn exercise_max_encoded_state_boundary(
        request_id: u64,
        apply: fn(&mut YrsDocumentEngine, u64) -> OperationResult<TypedTransactionResult>,
        probe_counts: (usize, usize, usize, usize, usize, usize),
        rejected_counts: (usize, usize, usize, usize, usize, usize),
        success_counts: (usize, usize, usize, usize, usize, usize),
    ) -> (YrsDocumentEngine, TypedTransactionResult) {
        let field = "maxEncodedStateBytes";
        let default_limit =
            u64::try_from(ResourceLimits::default().max_encoded_state_bytes).unwrap();
        let mut engine = fixture(field, default_limit);
        let before = atomic_audit(&engine);
        let current_encoded = u64::try_from(engine.encoded_state().unwrap().len()).unwrap();

        engine.resource_limits.max_encoded_state_bytes = usize::try_from(current_encoded).unwrap();
        reset_root_window_lowering_counts_for_test();
        let probe_error = apply(&mut engine, request_id).unwrap_err();
        assert_eq!(
            take_root_window_lowering_counts_for_test(),
            probe_counts,
            "{field} probe"
        );
        assert_eq!(atomic_audit(&engine), before, "{field} probe");
        assert_eq!(probe_error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(probe_error.details, Some(json!({ "field": field })));
        assert_eq!(probe_error.limit, Some(current_encoded));
        let exact = probe_error
            .actual
            .expect("encoded-state rejection must report the exact instance size");
        assert!(exact > current_encoded);
        let one_under = exact
            .checked_sub(1)
            .expect("encoded state must consume at least one byte");

        engine.resource_limits.max_encoded_state_bytes = usize::try_from(one_under).unwrap();
        reset_root_window_lowering_counts_for_test();
        let one_under_error = apply(&mut engine, request_id).unwrap_err();
        assert_eq!(
            take_root_window_lowering_counts_for_test(),
            rejected_counts,
            "{field} one-under"
        );
        assert_eq!(atomic_audit(&engine), before, "{field} one-under");
        assert_eq!(one_under_error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(one_under_error.details, Some(json!({ "field": field })));
        assert_eq!(one_under_error.limit, Some(one_under));
        assert_eq!(one_under_error.actual, Some(exact));

        engine.resource_limits.max_encoded_state_bytes = usize::try_from(exact).unwrap();
        reset_root_window_lowering_counts_for_test();
        let result = apply(&mut engine, request_id).unwrap();
        assert_eq!(
            take_root_window_lowering_counts_for_test(),
            success_counts,
            "{field} exact"
        );
        assert!(engine.encoded_state().unwrap().len() <= usize::try_from(exact).unwrap());
        (engine, result)
    }

    let request_id = 70_036_021;
    // The mutation entry point refreshes the ResourceLimits-bound lookup
    // seed before compilation, so the prepared root window remains valid.
    let (prepared, prepared_result) = exercise_max_encoded_state_boundary(
        request_id,
        prepared_apply,
        (0, 0, 1, 0, 1, 0),
        (0, 0, 1, 0, 1, 0),
        (0, 0, 1, 0, 1, 1),
    );
    let (generic, generic_result) = exercise_max_encoded_state_boundary(
        request_id,
        public_eager_apply,
        (1, 1, 0, 0, 1, 0),
        (1, 1, 0, 0, 1, 0),
        (1, 1, 0, 0, 2, 0),
    );
    assert_eq!(
        prepared_result, generic_result,
        "maxEncodedStateBytes exact"
    );
    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(prepared.document_html(), generic.document_html());
    assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
    assert_eq!(prepared.stored_marks(), generic.stored_marks());
    assert_eq!(prepared.can_undo(), generic.can_undo());
    assert_eq!(prepared.can_redo(), generic.can_redo());
}

#[test]
fn prepared_wrap_is_atomic_at_every_recoverable_failpoint() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::mutation::{
        reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
    };

    let failpoints = [
        AtomicFailpoint::EnvelopeAdmission,
        AtomicFailpoint::SemanticCompilation,
        AtomicFailpoint::MutationPreflight,
        AtomicFailpoint::FinalPreflight,
        AtomicFailpoint::EncodedAdmission,
        AtomicFailpoint::CanonicalOutputAdmission,
        AtomicFailpoint::RevisionAdmission,
        AtomicFailpoint::DurableMetadataAdmission,
    ];
    for (index, failpoint) in failpoints.into_iter().enumerate() {
        let mut engine = transaction_engine();
        let before = atomic_audit(&engine);
        let seed_before = engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .clone();
        assert!(seed_before.is_ready_for_test());
        reset_root_window_lowering_counts_for_test();
        set_atomic_failpoint_for_test(Some(failpoint));

        let error = engine
            .apply_command(
                70_036_100 + index as u64,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
            )
            .unwrap_err();

        set_atomic_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(
            error.details,
            Some(json!({ "failpoint": failpoint.field_name() })),
            "{failpoint:?}"
        );
        assert_eq!(
            take_root_window_lowering_counts_for_test().5,
            0,
            "{failpoint:?}"
        );
        assert!(Arc::ptr_eq(
            &seed_before,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
    }
}

#[test]
fn prepared_toggle_mark_is_atomic_at_every_recoverable_failpoint() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, reset_range_format_lowering_counts_for_test,
        take_localized_lookup_counts_for_test, take_range_format_lowering_counts_for_test,
    };

    let failpoints = [
        AtomicFailpoint::EnvelopeAdmission,
        AtomicFailpoint::SemanticCompilation,
        AtomicFailpoint::MutationPreflight,
        AtomicFailpoint::FinalPreflight,
        AtomicFailpoint::EncodedAdmission,
        AtomicFailpoint::CanonicalOutputAdmission,
        AtomicFailpoint::RevisionAdmission,
        AtomicFailpoint::DurableMetadataAdmission,
    ];
    for (index, failpoint) in failpoints.into_iter().enumerate() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut engine, 70_036_200 + index as u64, 0, 3);
        hydrate_import_for_compile_test(&mut engine);
        let before = atomic_audit(&engine);
        let seed_before = engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .clone();
        reset_localized_lookup_counts_for_test();
        reset_range_format_lowering_counts_for_test();
        set_atomic_failpoint_for_test(Some(failpoint));

        let error = engine
            .apply_command(
                70_036_300 + index as u64,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap_err();

        set_atomic_failpoint_for_test(None);
        let lookup_counts = take_localized_lookup_counts_for_test();
        let range_counts = take_range_format_lowering_counts_for_test();
        let expected_range_counts = if matches!(
            failpoint,
            AtomicFailpoint::EnvelopeAdmission | AtomicFailpoint::SemanticCompilation
        ) {
            (0, 0, 0, 0)
        } else {
            (0, 0, 1, 0)
        };
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(
            error.details,
            Some(json!({ "failpoint": failpoint.field_name() })),
            "{failpoint:?}"
        );
        assert_eq!(range_counts, expected_range_counts, "{failpoint:?}");
        assert_eq!(lookup_counts, (0, 0, 0), "{failpoint:?}");
        assert!(Arc::ptr_eq(
            &seed_before,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
    }
}

#[test]
fn prepared_wrap_matches_the_public_planned_transaction_path() {
    let mut prepared = transaction_engine();
    let mut generic = transaction_engine();
    let command = TypedCommand::WrapInList {
        list_type: "bulletList".into(),
        item_type: "listItem".into(),
    };

    let prepared_result = prepared
        .apply_command(70_037, command.clone())
        .unwrap()
        .unwrap();
    let CommandPlan::Transaction(transaction) = generic.plan_command(70_037, command).unwrap()
    else {
        panic!("public wrap planning must produce a transaction")
    };
    let generic_result = generic
        .apply_typed_transaction_with_result(transaction)
        .unwrap();

    assert_eq!(prepared_result, generic_result);
    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(prepared.document_html(), generic.document_html());
    assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
    assert_eq!(prepared.stored_marks(), generic.stored_marks());
    assert_eq!(prepared.can_undo(), generic.can_undo());
    assert_eq!(prepared.can_redo(), generic.can_redo());

    assert_eq!(
        prepared.undo_with_result(70_038).unwrap(),
        generic.undo_with_result(70_038).unwrap()
    );
    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(
        prepared.redo_with_result(70_039).unwrap(),
        generic.redo_with_result(70_039).unwrap()
    );
    assert_eq!(prepared.document_json(), generic.document_json());
}

#[test]
fn derived_state_node_count_refreshes_and_empty_results_use_equivalent_commands() {
    let mut engine = transaction_engine();
    let initial = engine.derived_state.as_ref().unwrap();
    assert_eq!(
        initial.document_node_count,
        crate::editor_state::document_node_count(initial.document.root())
    );

    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let refreshed = engine.derived_state.as_ref().unwrap();
    assert_eq!(refreshed.document_revision, engine.revision());
    assert_eq!(
        refreshed.document_node_count,
        crate::editor_state::document_node_count(refreshed.document.root())
    );

    let transaction = TypedTransaction {
        request_id: 991,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: Vec::new(),
        selection_intent: SelectionIntent::Set(SelectionInput::Text {
            anchor: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::Before,
            },
            head: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::Before,
            },
        }),
        history_policy: HistoryPolicy::Skip,
    };
    let result = engine
        .apply_typed_transaction_with_result(transaction)
        .unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    let selection = state.legacy_selection();
    assert_eq!(
        result.active_state.commands,
        crate::editor_state::command_applicability(
            &state.document,
            &engine.schema,
            &selection,
            &engine.resource_limits,
        )
    );
}

#[test]
fn utf16_doc_preserves_fresh_client_ids_and_uses_utf16_offsets() {
    let first = utf16_doc();
    let second = utf16_doc();

    assert_eq!(first.offset_kind(), OffsetKind::Utf16);
    assert_eq!(second.offset_kind(), OffsetKind::Utf16);
    assert_ne!(first.client_id(), second.client_id());
}

#[test]
fn validated_import_source_reuses_one_schema_ranked_canonical_result() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, reset_canonical_schema_context_count_for_test,
        take_canonical_artifact_counts_for_test, take_canonical_schema_context_count_for_test,
    };

    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let input = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "ordered",
                "marks": [{ "type": "bold" }, { "type": "italic" }]
            }]
        }]
    });
    let parsed = from_prosemirror_json(&input, &schema, UnknownTypeMode::Preserve).unwrap();
    let canonical_schema = crate::yrs_engine::canonical::CanonicalSchemaContext::new(&schema);
    let engine = transaction_engine();
    reset_canonical_artifact_counts_for_test();
    reset_canonical_schema_context_count_for_test();
    crate::yrs_engine::observability::reset_full_pass_counts_for_test();

    let input_len = serde_json::to_vec(&input).unwrap().len();
    let validated =
        ValidatedImportDocument::new(parsed, &schema, &canonical_schema, &limits, Some(input_len))
            .unwrap();
    let artifact = validated.canonical_artifact.clone();

    assert_eq!(
        validated.canonical_artifact.value(),
        &json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "ordered",
                    "marks": [{ "type": "bold" }, { "type": "italic" }]
                }]
            }]
        })
    );
    assert_eq!(
        validated.canonical_artifact.value(),
        &crate::serialize::to_prosemirror_json(&validated.document, &schema)
    );
    let candidate = engine
        .build_candidate_from_document(validated, TransactionOrigin::DocumentImport)
        .unwrap();
    let super::EngineDocumentState::Ready {
        canonical_artifact, ..
    } = candidate.state
    else {
        panic!("validated import candidate must be ready")
    };
    assert!(artifact.ptr_eq(&canonical_artifact));
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 0));
    assert_eq!(take_canonical_schema_context_count_for_test(), 0);
    let counts = crate::yrs_engine::observability::take_full_pass_counts_for_test();
    assert_eq!(counts.canonical_mark_nodes_visited, 3);
    assert_eq!(counts.canonical_identity_predicate_nodes_visited, 0);
}

#[test]
fn admitted_import_runs_one_validation_certificate_and_render_path() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_full_pass_counts_for_test();
    crate::render::incremental::reset_cached_render_counts_for_test();

    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    let passes = take_full_pass_counts_for_test();
    let render = crate::render::incremental::take_cached_render_counts_for_test();
    assert_eq!(passes.import_model_parses, 1);
    assert_eq!(passes.validated_evidence_constructions, 1);
    assert_eq!(passes.validation_certificate_constructions, 1);
    assert_eq!(passes.document_validations, 1);
    assert_eq!(passes.canonical_mark_validation_attempts, 1);
    assert_eq!(passes.canonical_mark_validation_completions, 1);
    assert_eq!(passes.canonical_projections, 1);
    assert_eq!(passes.canonical_serializations, 0);
    assert_eq!(passes.canonical_hashes, 0);
    assert_eq!(
        passes.render_limit_tree_scans, 0,
        "sealed validation evidence should replace the redundant render node/depth scan"
    );
    assert_eq!(
        render.0, 1,
        "the admitted import should build one render cache"
    );

    let artifact = &engine.derived_state.as_ref().unwrap().canonical_artifact;
    let _ = artifact.sha256();
    assert_eq!(take_full_pass_counts_for_test().canonical_hashes, 1);
    let _ = artifact.sha256();
    assert_eq!(take_full_pass_counts_for_test().canonical_hashes, 0);
}

#[test]
fn admitted_import_hydrates_before_seed_consumers_but_not_selection_only_state() {
    let mut typed_input = import_document_with_unavailable_lookup_seed();
    typed_input
        .apply_typed_transaction(insert_transaction(&typed_input, 65_100))
        .unwrap();
    assert!(typed_input
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    let mut command = import_document_with_unavailable_lookup_seed();
    command
        .apply_command(65_101, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("default-selection command should apply without preparatory selection");
    assert!(command
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    let mut selection = import_document_with_unavailable_lookup_seed();
    selection
        .apply_typed_transaction(TypedTransaction {
            request_id: 65_102,
            base_document_revision: selection.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(selection
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());

    let mut rich_local_api = import_document_with_unavailable_lookup_seed();
    rich_local_api
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 65_103,
            base_document_revision: rich_local_api.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(rich_local_api
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());

    let mut history = import_document_with_unavailable_lookup_seed();
    assert!(history.undo(65_104).unwrap().is_none());
    assert!(history
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    history
        .apply_command(65_105, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    force_lookup_seed_unavailable(&mut history);
    let unavailable_before_undo =
        Arc::clone(&history.derived_state.as_ref().unwrap().mutation_lookup_seed);
    assert!(history.undo(65_106).unwrap().is_some());
    assert!(!Arc::ptr_eq(
        &unavailable_before_undo,
        &history.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    let unavailable_before_redo =
        Arc::clone(&history.derived_state.as_ref().unwrap().mutation_lookup_seed);
    assert!(history.redo(65_107).unwrap().is_some());
    assert!(!Arc::ptr_eq(
        &unavailable_before_redo,
        &history.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
}

#[test]
fn deferred_insert_shape_and_output_bound_eligibility_is_exact() {
    let exact = deferred_insert_fixture(DeferredInsertCase::StrictInteriorEqualMarks);
    assert_eq!(
        exact.execution_admission_kind(),
        ExecutionAdmissionKind::Deferred
    );

    for case in [
        DeferredInsertCase::Empty,
        DeferredInsertCase::LeafBoundary,
        DeferredInsertCase::MarkMismatch,
        DeferredInsertCase::StructuralGrowth,
        DeferredInsertCase::UnavailableUpperBound,
        DeferredInsertCase::OverflowingUpperBound,
        DeferredInsertCase::OneOverOutputLimit,
    ] {
        assert_eq!(
            deferred_insert_fixture(case).execution_admission_kind(),
            ExecutionAdmissionKind::Eager,
            "{case:?}",
        );
    }
}

#[test]
fn eager_semantic_errors_precede_staged_hydration_failure() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    for case in eager_pre_admission_error_cases() {
        let mut engine = case.engine;
        let before = atomic_audit(&engine);
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::InitialReservation,
        ));
        let error = engine
            .apply_command(case.request_id, case.command)
            .unwrap_err();
        set_lookup_seed_hydration_failpoint_for_test(None);
        assert_eq!(error, case.expected_error, "{}", case.name);
        assert_eq!(atomic_audit(&engine), before, "{}", case.name);
    }
}

#[test]
fn first_imported_deferred_insert_uses_two_serializations_two_hashes_once() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_199, 2, 2);
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    reset_localized_lookup_counts_for_test();

    engine
        .apply_command(65_200, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("strict-interior imported insert should apply");

    let passes = take_full_pass_counts_for_test();
    assert_eq!(passes.planner_simulations, 1);
    assert_eq!(passes.document_validations, 1);
    assert_eq!(passes.canonical_serializations, 2);
    assert_eq!(passes.canonical_hashes, 2);
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(admission.staged_seed_preparations, 1);
    assert_eq!(admission.installed_base_seed_publications, 0);
}

#[test]
fn public_insert_uses_eager_admission_after_admissible_resource_limit_change() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_201, 2, 2);
    engine.resource_limits.max_input_bytes -= 1;
    let changed_limits = engine.resource_limits.clone();
    let mut preconfigured = transaction_engine();
    preconfigured.resource_limits = changed_limits;
    preconfigured
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_text(&mut preconfigured, 65_201, 2, 2);
    let command = TypedCommand::InsertText { text: "x".into() };
    let preparation = std::cell::RefCell::new(None);
    assert!(matches!(
        engine
            .plan_command_internal(65_202, command.clone(), Some(&preparation))
            .unwrap(),
        CommandPlan::Transaction(_)
    ));
    assert!(matches!(
        preparation.into_inner().unwrap().execution_admission,
        crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(_)
    ));
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();

    let result = engine.apply_command(65_202, command).unwrap().unwrap();
    let passes = take_full_pass_counts_for_test();
    let counts = take_prepared_admission_counts_for_test();
    let preconfigured_result = preconfigured
        .apply_command(65_202, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert!(result.changed);
    assert_eq!(passes.planner_simulations, 1);
    assert_eq!(passes.document_validations, 4);
    assert_eq!(result, preconfigured_result);
    assert_eq!(engine.document_json(), preconfigured.document_json());
    assert_eq!(engine.document_html(), preconfigured.document_html());
    assert_eq!(
        engine.resolved_selection(),
        preconfigured.resolved_selection()
    );
    assert!(!Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn private_prepared_command_orchestrator_finalizes_deferred_admission_once() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };
    use crate::yrs_engine::TransactionCommit;

    let mut engine = import_document_with_unavailable_lookup_seed();
    let mut public = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_260, 2, 2);
    select_text(&mut public, 65_260, 2, 2);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let preparation = std::cell::RefCell::new(None);
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    reset_localized_lookup_counts_for_test();

    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            65_261,
            TypedCommand::InsertText { text: "x".into() },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("strict-interior imported insert must produce a transaction")
    };
    let proof = preparation
        .into_inner()
        .expect("strict-interior imported insert must retain its exact proof");
    assert!(matches!(
        &proof.execution_admission,
        crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(_)
    ));
    let (commit, result) = engine
        .apply_prepared_command_transaction(
            transaction,
            proof,
            true,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap();
    let result = result.expect("changed command must return a result");
    let authority_counts = take_compiled_commit_authority_counts_for_test();
    let passes = take_full_pass_counts_for_test();
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(passes.planner_simulations, 1);
    assert_eq!(passes.document_validations, 1);
    assert_eq!(passes.canonical_serializations, 2);
    assert_eq!(passes.canonical_hashes, 2);
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
    assert_eq!(admission.staged_seed_preparations, 1);
    assert_eq!(admission.staged_identity_materializations, 1);
    assert_eq!(admission.installed_base_seed_publications, 0);
    assert_eq!(admission.deferred_capsules_created, 1);
    assert_eq!(admission.deferred_capsules_finalized, 1);
    assert_eq!(authority_counts, (1, 1));
    assert!(!Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    assert_eq!(
        commit,
        TransactionCommit {
            request_id: result.request_id,
            changed: result.changed,
            document_revision: result.document_revision,
            state_revision: result.state_revision,
            origin: result.origin,
        }
    );

    let public_result = public
        .apply_command(65_261, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    assert_eq!(result, public_result);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());
    let private_undo = engine.undo(65_262).unwrap().unwrap();
    let public_undo = public.undo(65_262).unwrap().unwrap();
    assert_eq!(private_undo, public_undo);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());
}

#[test]
fn first_imported_prepared_insert_traverses_each_history_document_once() {
    use crate::model::{
        reset_history_snapshot_retained_bytes_traversals_for_test,
        take_history_snapshot_retained_bytes_traversals_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_263, 2, 2);
    reset_history_snapshot_retained_bytes_traversals_for_test();

    engine
        .apply_command(65_264, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("strict-interior imported insert must apply");

    assert_eq!(
        take_history_snapshot_retained_bytes_traversals_for_test(),
        2,
        "history admission must traverse the before and after source documents once each"
    );
}

#[test]
fn first_imported_prepared_insert_uses_localized_history_render_evidence() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_265, 2, 2);
    reset_full_pass_counts_for_test();
    reset_cached_render_counts_for_test();
    reset_localized_render_transition_counts_for_test();

    engine
        .apply_command(65_266, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("strict-interior imported insert must apply");

    let passes = take_full_pass_counts_for_test();
    let localized = take_localized_render_transition_counts_for_test();
    assert_eq!((passes.render_limit_tree_scans, localized), (0, (1, 1, 0)));
    assert_eq!(
        (
            passes.position_map_clones,
            passes.position_map_compactions,
            passes.rendered_text_derivations,
        ),
        (1, 1, 0),
        "sealed strict-interior evidence must incrementally derive the candidate map and text",
    );
    assert_eq!(take_cached_render_counts_for_test(), (0, 1, 1, 0, 0));
}

#[test]
fn tampered_localized_history_render_evidence_falls_back_with_exact_results() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };
    use crate::yrs_engine::prepared_admission::{
        DeferredCommandAdmission, ExecutionSemanticAdmission,
    };

    for case in DeferredCommandAdmission::history_render_tamper_cases_for_test() {
        let mut actual = import_document_with_unavailable_lookup_seed();
        let mut expected = import_document_with_unavailable_lookup_seed();
        select_text(&mut actual, 65_267, 2, 2);
        select_text(&mut expected, 65_267, 2, 2);
        let command = TypedCommand::InsertText { text: "x".into() };
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = actual
            .plan_command_internal(65_268, command.clone(), Some(&preparation))
            .unwrap()
        else {
            panic!("strict-interior imported insert must produce a transaction")
        };
        let mut proof = preparation.into_inner().unwrap();
        let ExecutionSemanticAdmission::Deferred(deferred) = &mut proof.execution_admission else {
            panic!("strict-interior imported insert must retain deferred evidence")
        };
        deferred.tamper_history_render_for_test(case);
        reset_full_pass_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();

        let actual_result = actual
            .apply_prepared_command_transaction(
                transaction,
                proof,
                true,
                &mut OutboundUpdateSink::detached(),
            )
            .unwrap()
            .1
            .unwrap();
        let passes = take_full_pass_counts_for_test();
        let cached = take_cached_render_counts_for_test();
        let localized = take_localized_render_transition_counts_for_test();
        let expected_result = expected.apply_command(65_268, command).unwrap().unwrap();

        assert_eq!(actual_result, expected_result, "{case}");
        assert_eq!(actual.document_json(), expected.document_json(), "{case}");
        assert_eq!(
            actual.resolved_selection(),
            expected.resolved_selection(),
            "{case}"
        );
        assert_eq!(actual.can_undo(), expected.can_undo(), "{case}");
        assert_eq!(passes.render_limit_tree_scans, 1, "{case}");
        assert_eq!(cached, (0, 1, 1, 0, 0), "{case}");
        assert_eq!(localized, (1, 0, 1), "{case}");
    }
}

#[test]
fn localized_history_render_errors_fall_back_with_exact_results() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        set_localized_render_failure_stage_for_test, take_cached_render_counts_for_test,
        take_localized_render_transition_counts_for_test, LocalizedRenderFailureStage,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    for stage in [
        LocalizedRenderFailureStage::Allocation,
        LocalizedRenderFailureStage::Resource,
        LocalizedRenderFailureStage::Position,
        LocalizedRenderFailureStage::Invariant,
    ] {
        let mut actual = import_document_with_unavailable_lookup_seed();
        let mut expected = import_document_with_unavailable_lookup_seed();
        let two_blocks = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#;
        actual
            .import_json(two_blocks, TransactionOrigin::DocumentImport)
            .unwrap();
        expected
            .import_json(two_blocks, TransactionOrigin::DocumentImport)
            .unwrap();
        select_text(&mut actual, 65_269, 2, 2);
        select_text(&mut expected, 65_269, 2, 2);
        reset_full_pass_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        set_localized_render_failure_stage_for_test(Some(stage));

        let actual_result = actual
            .apply_command(65_270, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        set_localized_render_failure_stage_for_test(None);
        let passes = take_full_pass_counts_for_test();
        let cached = take_cached_render_counts_for_test();
        let localized = take_localized_render_transition_counts_for_test();
        let expected_result = expected
            .apply_command(65_270, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();

        assert_eq!(actual_result, expected_result, "{stage:?}");
        assert_eq!(
            actual.document_json(),
            expected.document_json(),
            "{stage:?}"
        );
        assert_eq!(
            actual.resolved_selection(),
            expected.resolved_selection(),
            "{stage:?}"
        );
        assert_eq!(actual.can_undo(), expected.can_undo(), "{stage:?}");
        assert_eq!(passes.render_limit_tree_scans, 1, "{stage:?}");
        assert_eq!(cached, (0, 1, 1, 0, 0), "{stage:?}");
        assert_eq!(localized, (1, 0, 1), "{stage:?}");
    }
}

#[test]
fn private_prepared_eager_noninsert_uses_staged_context_without_identity() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };
    use crate::yrs_engine::TransactionCommit;

    let mut engine = import_document_with_unavailable_lookup_seed();
    let mut public = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_263, 0, 2);
    select_text(&mut public, 65_263, 0, 2);
    let preparation = std::cell::RefCell::new(None);
    reset_prepared_admission_counts_for_test();
    let command = TypedCommand::ToggleMark {
        mark_type: "bold".into(),
    };
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(65_264, command.clone(), Some(&preparation))
        .unwrap()
    else {
        panic!("range mark command must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    assert!(matches!(
        &proof.execution_admission,
        crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(_)
    ));

    let (commit, result) = engine
        .apply_prepared_command_transaction(
            transaction,
            proof,
            true,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap();
    let result = result.unwrap();
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(admission.staged_seed_preparations, 1);
    assert_eq!(admission.staged_identity_materializations, 0);
    assert_eq!(admission.installed_base_seed_publications, 0);
    assert_eq!(
        commit,
        TransactionCommit {
            request_id: result.request_id,
            changed: result.changed,
            document_revision: result.document_revision,
            state_revision: result.state_revision,
            origin: result.origin,
        }
    );

    let public_result = public.apply_command(65_264, command).unwrap().unwrap();
    assert_eq!(result, public_result);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());
}

#[test]
fn private_prepared_history_error_precedes_staged_hydration_failure() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let limits = crate::yrs_engine::EditingLimits {
        max_derived_output_bytes: 100,
        ..crate::yrs_engine::EditingLimits::default()
    };
    let mut engine = transaction_engine_with_editing_limits(limits);
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_text(&mut engine, 65_265, 2, 2);
    engine.derived_state.as_mut().unwrap().canonical_artifact = engine
        .derived_state
        .as_ref()
        .unwrap()
        .canonical_artifact
        .with_admission_upper_bound_for_test(usize::MAX);
    let expected_actual = super::history_metadata_bytes(engine.stored_marks(), "prosemirror") * 2;
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            65_266,
            TypedCommand::InsertText { text: "x".into() },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("insert command must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let before = atomic_audit(&engine);
    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::InitialReservation,
    ));
    let error = engine
        .apply_prepared_command_transaction(
            transaction,
            proof,
            true,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap_err();
    set_lookup_seed_hydration_failpoint_for_test(None);

    assert_eq!(
        error,
        crate::yrs_engine::OperationError::document_limit_exceeded(
            65_266,
            None,
            "maxDerivedOutputBytes",
            100,
            expected_actual as u64,
        )
    );
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(admission.staged_seed_preparations, 0);
    assert_eq!(admission.installed_base_seed_publications, 0);
}

#[test]
fn private_prepared_deferred_compiler_failure_is_prewrite_and_atomic() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_267, 2, 2);
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            65_268,
            TypedCommand::InsertText { text: "x".into() },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("strict-interior imported insert must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    assert!(matches!(
        &proof.execution_admission,
        crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(_)
    ));
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let before = atomic_audit(&engine);
    reset_prepared_admission_counts_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::SemanticCompilation));
    let error = engine
        .apply_prepared_command_transaction(
            transaction,
            proof,
            true,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap_err();
    set_atomic_failpoint_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(admission.staged_seed_preparations, 1);
    assert_eq!(admission.staged_identity_materializations, 1);
    assert_eq!(admission.installed_base_seed_publications, 0);
    assert_eq!(admission.deferred_capsules_finalized, 1);
}

#[test]
fn eager_non_insert_first_mutations_do_not_materialize_base_identity() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut toggle = import_document_with_unavailable_lookup_seed();
    select_text(&mut toggle, 65_201, 0, 2);
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    toggle
        .apply_command(
            65_202,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    let toggle_passes = take_full_pass_counts_for_test();
    let toggle_admission = take_prepared_admission_counts_for_test();
    assert_eq!(toggle_passes.canonical_serializations, 3);
    assert_eq!(toggle_passes.canonical_hashes, 2);
    assert_eq!(toggle_admission.staged_identity_materializations, 0);

    let mut wrap = import_document_with_unavailable_lookup_seed();
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    wrap.apply_command(
        65_203,
        TypedCommand::WrapInList {
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
        },
    )
    .unwrap()
    .unwrap();
    let wrap_passes = take_full_pass_counts_for_test();
    let wrap_admission = take_prepared_admission_counts_for_test();
    assert_eq!(wrap_passes.canonical_serializations, 3);
    assert_eq!(wrap_passes.canonical_hashes, 2);
    assert_eq!(wrap_admission.staged_identity_materializations, 0);

    let mut undo = import_document_with_unavailable_lookup_seed();
    undo.apply_command(65_204, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    force_lookup_seed_unavailable(&mut undo);
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    undo.undo(65_205).unwrap().unwrap();
    let undo_passes = take_full_pass_counts_for_test();
    let undo_admission = take_prepared_admission_counts_for_test();
    assert_eq!(undo_passes.canonical_serializations, 0);
    assert_eq!(undo_passes.canonical_hashes, 0);
    assert_eq!(undo_admission.staged_identity_materializations, 0);

    let mut redo = import_document_with_unavailable_lookup_seed();
    redo.apply_command(65_206, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    redo.undo(65_207).unwrap().unwrap();
    force_lookup_seed_unavailable(&mut redo);
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    redo.redo(65_208).unwrap().unwrap();
    let redo_passes = take_full_pass_counts_for_test();
    let redo_admission = take_prepared_admission_counts_for_test();
    assert_eq!(redo_passes.canonical_serializations, 0);
    assert_eq!(redo_passes.canonical_hashes, 0);
    assert_eq!(redo_admission.staged_identity_materializations, 0);
}

#[test]
fn staged_authority_supplies_every_unavailable_seed_consumer_without_installed_reads() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };

    reset_prepared_admission_counts_for_test();
    let (engine, deferred, mut context, transaction, expected_document) =
        deferred_finalization_fixture();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    engine.prepare_mutation_identity(&mut context).unwrap();
    reset_full_pass_counts_for_test();

    let prepared = engine
        .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
        .unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    let cached = state.compilation_view();
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let authority = context
        .authority(
            crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                request_id: transaction.request_id,
                installed: state,
                txn: &txn,
                fragment: &fragment,
                fragment_name: &engine.fragment_name,
                schema_fingerprint: &engine.schema_fingerprint,
                resource_limits: &engine.resource_limits,
                editing_limits: &engine.editing_limits,
                max_length: engine.max_length,
                document_revision: engine.revision,
                state_revision: engine.state_revision,
                yrs_state_epoch: engine.yrs_state_epoch,
            },
        )
        .unwrap();
    assert!(installed.is_unavailable_for_test());
    assert!(authority.lookup_seed().is_ready_for_test());
    assert!(!Arc::ptr_eq(&installed, authority.lookup_seed()));

    let format_from = crate::yrs_engine::position::editor_offset_to_doc_pos(
        0,
        EditorOffsetKind::Scalar,
        &state.rendered_text,
        &state.position_map,
        &state.document,
    )
    .unwrap();
    let format_to = crate::yrs_engine::position::editor_offset_to_doc_pos(
        2,
        EditorOffsetKind::Scalar,
        &state.rendered_text,
        &state.position_map,
        &state.document,
    )
    .unwrap();
    let format_block = state
        .position_map
        .find_block_for_doc_pos(format_from)
        .and_then(|index| state.position_map.block(index))
        .unwrap();
    let format_locator = crate::yrs_engine::mutation::LocalizedFormatLocator::mint(
        &state.document,
        &format_block.node_path,
        format_from,
        format_to,
        authority.lookup_seed().as_ref(),
        &txn,
        &fragment,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        engine.yrs_state_epoch,
        engine.revision,
    )
    .expect("staged authority mints a current localized format locator");
    assert!(
        crate::yrs_engine::mutation::LocalizedFormatCompiler::try_new(
            transaction.request_id,
            &txn,
            &fragment,
            &engine.schema,
            usize::MAX,
            engine.resource_limits.max_input_bytes,
            0,
            format_locator,
            &engine.schema_fingerprint,
            engine.yrs_state_epoch,
            engine.revision,
        )
        .unwrap()
        .is_some()
    );

    let first_child = state
        .document
        .root()
        .content()
        .and_then(|content| content.child(0))
        .unwrap()
        .clone();
    let root_replacement = crate::yrs_engine::StructuralReplacement::new(
        Vec::new(),
        0,
        1,
        crate::model::Fragment::from(vec![first_child]),
        Selection::cursor(0),
    );
    let root_locator = crate::yrs_engine::mutation::LocalizedRootWindowLocator::mint(
        transaction.request_id,
        &state.document,
        &state.document,
        &root_replacement,
        authority.lookup_seed().as_ref(),
        &txn,
        &fragment,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        engine.yrs_state_epoch,
        engine.revision,
    )
    .unwrap()
    .expect("staged authority mints a current localized root-window locator");
    assert!(
        crate::yrs_engine::mutation::LocalizedRootWindowCompiler::try_new(
            transaction.request_id,
            &txn,
            &fragment,
            &engine.schema,
            usize::MAX,
            engine.resource_limits.max_input_bytes,
            0,
            root_locator,
        )
        .unwrap()
        .is_some()
    );

    let mut compiled =
        crate::yrs_engine::compiler::compile_prepared_transaction_with_yrs_and_stored_marks(
            crate::yrs_engine::compiler::CompilationContext {
                document: cached.document,
                selection: Some(cached.selection),
                schema: &engine.schema,
                resource_limits: &engine.resource_limits,
                editing_limits: &engine.editing_limits,
                document_revision: engine.revision,
                max_length: engine.max_length,
            },
            transaction.clone(),
            &txn,
            &fragment,
            crate::yrs_engine::compiler::StoredMarksCompilationContext {
                stored_marks: state.stored_marks.as_deref(),
                resolved_selection: &state.resolved_selection,
                relative_selection: &state.relative_selection,
            },
            crate::yrs_engine::compiler::PreparedSemanticContext {
                admission: &prepared,
                expected_preview: &expected_document,
                yrs_state_epoch: engine.yrs_state_epoch,
                state_revision: engine.state_revision,
                schema_fingerprint: &engine.schema_fingerprint,
            },
            crate::yrs_engine::compiler::EngineCompilationView {
                cached,
                authority: &authority,
                state_revision: engine.state_revision,
                schema_fingerprint: &engine.schema_fingerprint,
                yrs_state_epoch: engine.yrs_state_epoch,
            },
        )
        .unwrap();
    assert!(compiled.localized_semantic_used);
    assert!(compiled.localized_insert_admission.is_some());
    assert!(compiled.prepared_derived_evidence.is_some());
    assert!(compiled.mutation_lookup_transition.is_some());

    let admission = compiled.localized_insert_admission.as_ref().unwrap();
    let crate::yrs_engine::compiler::StoredMarksPlan::Set(stored_marks) =
        &compiled.stored_marks_plan
    else {
        panic!("localized compiler seals stored marks")
    };
    let active_transition = state
        .prepare_active_state_transition(
            transaction.request_id,
            &authority,
            admission,
            &compiled.preview,
            admission.operation_result_selection(),
            stored_marks.as_deref(),
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            engine.yrs_state_epoch,
        )
        .unwrap();
    let structural = admission.active_state_structural_seal();
    assert!(state
        .validate_active_state_transition(
            &authority,
            &active_transition,
            &structural,
            &compiled.preview,
            admission.operation_result_selection(),
            stored_marks.as_deref(),
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            engine.yrs_state_epoch,
        )
        .is_some());

    let selection_seal =
        crate::yrs_engine::compiler::PreparedSelectionMutationSeal::capture(&compiled)
            .expect("localized insert captures its prepared selection seal");
    assert!(selection_seal.matches(&compiled, &authority));

    let evidence = compiled.prepared_derived_evidence.take().unwrap();
    let derivations = compiled.preview_derivations.as_ref().unwrap();
    let render_transition = evidence
        .prepare_localized_render_transition(
            state,
            &compiled.preview,
            derivations,
            &compiled.affected_top_level_blocks,
            &engine.schema,
            &engine.schema_fingerprint,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
        )
        .expect("localized render proof remains current")
        .unwrap();
    let next_document_revision = engine.revision.checked_add(1).unwrap();
    let next_state_revision = engine.state_revision.checked_add(1).unwrap();
    let next_yrs_state_epoch = engine.yrs_state_epoch.checked_add(1).unwrap();
    assert!(evidence
        .finalize(
            &authority,
            &compiled.preview,
            compiled.canonical_artifact.as_ref().unwrap(),
            derivations,
            &render_transition.cache,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_document_revision,
            next_state_revision,
            next_yrs_state_epoch,
        )
        .is_some());

    let next_seed = engine
        .prepare_mutation_lookup_transition_with_authority(
            transaction.request_id,
            &authority,
            compiled.mutation_lookup_transition.as_ref().unwrap(),
            &txn,
            &fragment,
            &compiled.preview,
            compiled.canonical_artifact.as_ref().unwrap(),
            next_yrs_state_epoch,
            next_document_revision,
        )
        .unwrap();
    assert!(next_seed.is_ready_for_test());
    assert!(!Arc::ptr_eq(&installed, &next_seed));
    let installed_adapter =
        crate::yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(state);
    assert!(
        crate::yrs_engine::prepared_admission::DerivedStateAuthority::lookup_seed(
            &installed_adapter,
            transaction.request_id,
        )
        .is_err()
    );
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    let passes = take_full_pass_counts_for_test();
    assert_eq!(passes.document_validations, 0);
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn staged_authority_rejects_installed_substitution_and_live_seal_drift_before_transition() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for case in [
        "request",
        "store",
        "fragment",
        "schema",
        "resource_limits",
        "editing_limits",
        "max_length",
        "document_revision",
        "state_revision",
        "epoch",
        "identity",
    ] {
        let mut engine = import_document_with_unavailable_lookup_seed();
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        reset_prepared_admission_counts_for_test();
        let mut context = engine.prepare_mutation_lookup_seed(65_250).unwrap();
        engine.prepare_mutation_identity(&mut context).unwrap();

        if case == "identity" {
            let state = engine.derived_state.as_mut().unwrap();
            state.canonical_artifact = state
                .canonical_artifact
                .schema_context()
                .derive(&state.document)
                .unwrap();
        }
        let before = atomic_audit(&engine);
        let state = engine.derived_state.as_ref().unwrap();
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let foreign = transaction_engine();
        let foreign_txn = foreign.doc.transact();
        let foreign_fragment = foreign_txn
            .get_xml_fragment(foreign.fragment_name.as_str())
            .unwrap();
        let mut drifted_resources = engine.resource_limits.clone();
        drifted_resources.max_input_bytes = drifted_resources
            .max_input_bytes
            .checked_sub(1)
            .expect("fixture resource limit is positive");
        let mut drifted_editing = engine.editing_limits.clone();
        drifted_editing.max_operations_per_transaction = drifted_editing
            .max_operations_per_transaction
            .checked_sub(1)
            .expect("fixture editing limit is positive");
        let drifted_max_length = match engine.max_length {
            Some(_) => None,
            None => Some(1),
        };
        let drifted_document_revision = engine
            .revision
            .checked_add(1)
            .expect("fixture document revision can advance");
        let drifted_state_revision = engine
            .state_revision
            .checked_add(1)
            .expect("fixture state revision can advance");
        let drifted_schema = format!("{}!", engine.schema_fingerprint);

        let error = match case {
            "request" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_251,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "store" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &foreign_txn,
                        fragment: &foreign_fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "fragment" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: "foreign-fragment",
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "schema" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &drifted_schema,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "resource_limits" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &drifted_resources,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "editing_limits" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &drifted_editing,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "max_length" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: drifted_max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "document_revision" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: drifted_document_revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "state_revision" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: drifted_state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "epoch" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch.saturating_add(1),
                    },
                )
                .err(),
            "identity" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            _ => unreachable!(),
        }
        .expect("drifted live context must not mint an authority");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        drop(foreign_txn);
        drop(txn);
        assert_eq!(atomic_audit(&engine), before, "{case}");
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 1, "{case}");
        assert_eq!(counts.installed_base_seed_publications, 0, "{case}");
    }
}

#[test]
fn generic_typed_compilation_uses_staged_authority_without_publishing_base_seed() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    let mut public = import_document_with_unavailable_lookup_seed();
    let mut public_rich = import_document_with_unavailable_lookup_seed();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    reset_prepared_admission_counts_for_test();
    let transaction = insert_transaction(&engine, 65_225);
    let (commit, result) = engine
        .apply_typed_transaction_with_staged_context(
            transaction,
            false,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap();
    assert!(result.is_none());
    let counts = take_prepared_admission_counts_for_test();
    let authority_counts = take_compiled_commit_authority_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert_eq!(authority_counts, (1, 1));
    assert!(!Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    reset_prepared_admission_counts_for_test();
    let public_commit = public
        .apply_typed_transaction(insert_transaction(&public, 65_225))
        .unwrap();
    let public_counts = take_prepared_admission_counts_for_test();
    assert_eq!(public_counts.staged_seed_preparations, 1);
    assert_eq!(public_counts.installed_base_seed_publications, 0);
    assert!(public
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    assert_eq!(commit, public_commit);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());

    reset_prepared_admission_counts_for_test();
    let rich_result = public_rich
        .apply_typed_transaction_with_result(insert_transaction(&public_rich, 65_225))
        .unwrap();
    assert!(rich_result.changed);
    let rich_counts = take_prepared_admission_counts_for_test();
    assert_eq!(rich_counts.staged_seed_preparations, 1);
    assert_eq!(rich_counts.installed_base_seed_publications, 0);
    assert!(public_rich
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
}

#[test]
fn staged_generic_compiler_semantic_failure_is_prewrite_and_atomic() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let before = atomic_audit(&engine);
    reset_prepared_admission_counts_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::SemanticCompilation));
    let transaction = insert_transaction(&engine, 65_226);
    let error = engine
        .apply_typed_transaction_with_staged_context(
            transaction,
            false,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap_err();
    set_atomic_failpoint_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn staged_generic_lookup_transition_failure_is_prewrite_and_atomic() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let before = atomic_audit(&engine);
    reset_prepared_admission_counts_for_test();
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::LookupTransition,
    ));
    let transaction = insert_transaction(&engine, 65_227);
    let error = engine
        .apply_typed_transaction_with_staged_context(
            transaction,
            false,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap_err();
    set_compiled_commit_stage_failpoint_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn history_candidate_swap_prepares_ready_candidate_seed_without_compiled_transaction() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };
    use crate::yrs_engine::TransactionCommit;

    let mut engine = import_document_with_unavailable_lookup_seed();
    let mut public = import_document_with_unavailable_lookup_seed();
    engine
        .apply_command(65_226, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    public
        .apply_command(65_226, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    force_lookup_seed_unavailable(&mut engine);
    force_lookup_seed_unavailable(&mut public);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    reset_prepared_admission_counts_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::SemanticCompilation));
    let result = engine.apply_history_pop(65_227, true, true, &mut OutboundUpdateSink::detached());
    let compiler_failpoint = crate::yrs_engine::compiler::check_atomic_failpoint(
        65_227,
        AtomicFailpoint::SemanticCompilation,
    );
    set_atomic_failpoint_for_test(None);
    let (commit, result) = result.unwrap().unwrap();
    let result = result.unwrap();
    let compiler_error = compiler_failpoint.unwrap_err();
    assert_eq!(compiler_error.code, "ENGINE_INVARIANT_FAILED");
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert!(!Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    let state = engine.derived_state.as_ref().unwrap();
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    assert!(state
        .mutation_lookup_seed
        .matches_canonical_artifact(&state.canonical_artifact));
    assert!(state.mutation_lookup_seed.matches(
        &txn,
        &fragment,
        &state.document,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        engine.yrs_state_epoch,
        engine.revision,
    ));
    drop(txn);
    assert_eq!(
        commit,
        TransactionCommit {
            request_id: result.request_id,
            changed: result.changed,
            document_revision: result.document_revision,
            state_revision: result.state_revision,
            origin: result.origin,
        }
    );

    let public_result = public.undo_with_result(65_227).unwrap().unwrap();
    assert_eq!(result, public_result);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());
    assert_eq!(
        engine.history.replay_audit_for_test(),
        public.history.replay_audit_for_test()
    );
    assert_eq!(
        engine.history.retained_units(65_227).unwrap(),
        public.history.retained_units(65_227).unwrap()
    );
}

#[test]
fn history_candidate_publication_failures_are_pre_swap_and_atomic() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (request_id, failpoint, stage) in [
        (
            65_228,
            LookupSeedHydrationFailpoint::CandidateBindingPublication,
            "candidateBindingPublication",
        ),
        (
            65_229,
            LookupSeedHydrationFailpoint::CandidateSeedPublication,
            "candidateSeedPublication",
        ),
    ] {
        let mut engine = import_document_with_unavailable_lookup_seed();
        engine
            .apply_command(
                request_id - 1,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap()
            .unwrap();
        force_lookup_seed_unavailable(&mut engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let before = atomic_audit(&engine);
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let error = engine
            .apply_history_pop(request_id, true, true, &mut OutboundUpdateSink::detached())
            .unwrap_err();
        set_lookup_seed_hydration_failpoint_for_test(None);

        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED", "{stage}");
        assert_eq!(
            error.message.as_ref(),
            format!("mutation lookup seed allocation failed during {stage}"),
            "{stage}"
        );
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" })),
            "{stage}"
        );
        assert_eq!(atomic_audit(&engine), before, "{stage}");
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0, "{stage}");
        assert_eq!(counts.installed_base_seed_publications, 0, "{stage}");
    }
}

fn task5_changed_remote_fixture() -> (YrsDocumentEngine, Vec<u8>) {
    let target = import_document_with_unavailable_lookup_seed();
    let base = target.encoded_state().unwrap();
    let mut source = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    source.apply_remote_update_v1(65_228, &base).unwrap();
    source
        .apply_command(65_229, TypedCommand::InsertText { text: "r".into() })
        .unwrap()
        .unwrap();
    let target_vector = target.doc.transact().state_vector();
    let delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);
    (target, delta)
}

fn task5_candidate_publication_fixture() -> (
    YrsDocumentEngine,
    Doc,
    crate::model::Document,
    crate::yrs_engine::canonical::CanonicalArtifact,
    u64,
    u64,
) {
    let (engine, delta) = task5_changed_remote_fixture();
    let current_encoded = engine.encoded_state().unwrap();
    let candidate_doc =
        super::fresh_utf16_doc_excluding(&engine.durable_client_ids, engine.client_id());
    {
        let mut txn = candidate_doc.transact_mut();
        txn.apply_update(Update::decode_v1(&current_encoded).unwrap())
            .unwrap();
        txn.apply_update(Update::decode_v1(&delta).unwrap())
            .unwrap();
    }
    let (candidate_document, candidate_artifact) = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let json =
            crate::yrs_engine::YrsDocumentCodec::new(&engine.schema, &engine.resource_limits)
                .read_json(&fragment, &txn)
                .unwrap();
        let document =
            from_prosemirror_json(&json, &engine.schema, UnknownTypeMode::Preserve).unwrap();
        let artifact = engine.canonical_schema.derive(&document).unwrap();
        (document, artifact)
    };
    let next_revision = engine.revision.checked_add(1).unwrap();
    let next_epoch = engine.yrs_state_epoch.checked_add(1).unwrap();
    (
        engine,
        candidate_doc,
        candidate_document,
        candidate_artifact,
        next_revision,
        next_epoch,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_history_candidate_capability_for_test<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    fragment: &yrs::types::xml::XmlFragmentRef,
    schema: &crate::schema::Schema,
    source_document: &crate::model::Document,
    canonical_artifact: &crate::yrs_engine::canonical::CanonicalArtifact,
    resource_limits: &ResourceLimits,
    editing_limits: &crate::yrs_engine::EditingLimits,
    max_length: Option<u32>,
    schema_fingerprint: &str,
    yrs_state_epoch: u64,
    document_revision: u64,
) -> crate::yrs_engine::derived_state::HistoryMutationLookupCapability {
    let (json, admission) =
        crate::yrs_engine::derived_state::prepare_history_candidate_read_for_test(
            request_id,
            txn,
            fragment,
            schema,
            source_document,
            canonical_artifact,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
        )
        .unwrap()
        .into_parts();
    assert_eq!(&json, canonical_artifact.value());
    admission
        .expect("exact candidate read must create one consuming admission")
        .mint_capability_for_test(request_id, txn, fragment)
        .unwrap()
}

#[allow(clippy::too_many_arguments)]
fn publish_history_candidate_seed_for_test<T: ReadTxn>(
    capability: crate::yrs_engine::derived_state::HistoryMutationLookupCapability,
    request_id: u64,
    txn: &T,
    fragment: &yrs::types::xml::XmlFragmentRef,
    schema: &crate::schema::Schema,
    source_document: &crate::model::Document,
    canonical_artifact: &crate::yrs_engine::canonical::CanonicalArtifact,
    resource_limits: &ResourceLimits,
    editing_limits: &crate::yrs_engine::EditingLimits,
    max_length: Option<u32>,
    schema_fingerprint: &str,
    yrs_state_epoch: u64,
    document_revision: u64,
) -> crate::yrs_engine::OperationResult<Arc<crate::yrs_engine::mutation::MutationLookupSeed>> {
    capability.prepare_candidate_publication(
        request_id,
        txn,
        fragment,
        schema,
        source_document,
        canonical_artifact,
        resource_limits,
        editing_limits,
        max_length,
        schema_fingerprint,
        yrs_state_epoch,
        document_revision,
    )
}

#[test]
fn candidate_seed_publication_is_ready_and_bound_only_to_its_candidate_store() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (engine, delta) = task5_changed_remote_fixture();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let current_encoded = engine.encoded_state().unwrap();
    let candidate_doc =
        super::fresh_utf16_doc_excluding(&engine.durable_client_ids, engine.client_id());
    {
        let mut txn = candidate_doc.transact_mut();
        txn.apply_update(Update::decode_v1(&current_encoded).unwrap())
            .unwrap();
        txn.apply_update(Update::decode_v1(&delta).unwrap())
            .unwrap();
    }
    let (candidate_document, candidate_artifact, next_revision, next_epoch) = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let json =
            crate::yrs_engine::YrsDocumentCodec::new(&engine.schema, &engine.resource_limits)
                .read_json(&fragment, &txn)
                .unwrap();
        let document =
            from_prosemirror_json(&json, &engine.schema, UnknownTypeMode::Preserve).unwrap();
        DocumentValidator::validate(&document, &engine.schema, &engine.resource_limits).unwrap();
        let artifact = engine.canonical_schema.derive(&document).unwrap();
        (
            document,
            artifact,
            engine.revision.checked_add(1).unwrap(),
            engine.yrs_state_epoch.checked_add(1).unwrap(),
        )
    };

    reset_prepared_admission_counts_for_test();
    let candidate_seed = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        prepare_history_candidate_capability_for_test(
            65_233,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .prepare_candidate_publication(
            65_233,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .unwrap()
    };
    let counts = take_prepared_admission_counts_for_test();

    let candidate_txn = candidate_doc.transact();
    let candidate_fragment = candidate_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    assert!(candidate_seed.is_ready_for_test());
    assert!(candidate_seed.matches_canonical_artifact(&candidate_artifact));
    assert!(candidate_seed.matches(
        &candidate_txn,
        &candidate_fragment,
        &candidate_document,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        next_epoch,
        next_revision,
    ));
    let live_txn = engine.doc.transact();
    let live_fragment = live_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    assert!(!candidate_seed.matches(
        &live_txn,
        &live_fragment,
        &candidate_document,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        next_epoch,
        next_revision,
    ));
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(installed.is_unavailable_for_test());
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn consumed_history_capability_cannot_be_replayed_through_a_general_seed_clone() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (engine, candidate_doc, candidate_document, candidate_artifact, next_revision, next_epoch) =
        task5_candidate_publication_fixture();
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let txn = candidate_doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let capability = prepare_history_candidate_capability_for_test(
        65_244,
        &txn,
        &fragment,
        &engine.schema,
        &candidate_document,
        &candidate_artifact,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        next_epoch,
        next_revision,
    );
    let general_seed = capability
        .into_unavailable_seed_for_test(65_244)
        .expect("consuming conversion must publish one unavailable general seed");

    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::BindingPublication,
    ));
    let result = general_seed.as_ref().clone().prepare_candidate_publication(
        65_245,
        &txn,
        &fragment,
        &engine.schema,
        &candidate_document,
        &candidate_artifact,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        next_epoch,
        next_revision,
    );
    set_lookup_seed_hydration_failpoint_for_test(None);

    let error = result.expect_err("a general seed clone must not retain the one-shot seal");
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 0);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
}

#[test]
fn history_capability_rejects_request_relabeling_before_publication_probes() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (publish_candidate, failpoint) in [
        (true, LookupSeedHydrationFailpoint::BindingPublication),
        (false, LookupSeedHydrationFailpoint::SeedPublication),
    ] {
        let (
            engine,
            candidate_doc,
            candidate_document,
            candidate_artifact,
            next_revision,
            next_epoch,
        ) = task5_candidate_publication_fixture();
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let capability = prepare_history_candidate_capability_for_test(
            65_246,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        );

        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let result = if publish_candidate {
            capability.prepare_candidate_publication(
                65_247,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            )
        } else {
            capability.into_unavailable_seed_for_test(65_247)
        };
        set_lookup_seed_hydration_failpoint_for_test(None);

        let error = result.expect_err("a one-shot history request must not be relabeled");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 65_247);
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }
}

#[test]
fn history_candidate_seed_publication_rejects_contradictory_claims_before_failpoints() {
    use crate::schema::presets::prosemirror_schema;
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    #[derive(Clone, Copy, Debug)]
    enum Case {
        Document,
        CanonicalArtifact,
        CanonicalIdentity,
        Schema,
        SchemaFingerprint,
        ResourceLimits,
        EditingLimits,
        MaxLength,
        Store,
        Revision,
        Epoch,
        Fragment,
    }

    for case in [
        Case::Document,
        Case::CanonicalArtifact,
        Case::CanonicalIdentity,
        Case::Schema,
        Case::SchemaFingerprint,
        Case::ResourceLimits,
        Case::EditingLimits,
        Case::MaxLength,
        Case::Store,
        Case::Revision,
        Case::Epoch,
        Case::Fragment,
    ] {
        for failpoint in [
            LookupSeedHydrationFailpoint::BindingPublication,
            LookupSeedHydrationFailpoint::SeedPublication,
        ] {
            let (
                engine,
                candidate_doc,
                candidate_document,
                candidate_artifact,
                next_revision,
                next_epoch,
            ) = task5_candidate_publication_fixture();
            let before = atomic_audit(&engine);
            let installed =
                Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
            let txn = candidate_doc.transact();
            let candidate_fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let unavailable = prepare_history_candidate_capability_for_test(
                65_236,
                &txn,
                &candidate_fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            );
            drop(txn);
            let wrong_fragment = candidate_doc.get_or_insert_xml_fragment("foreign");
            let txn = candidate_doc.transact();
            let candidate_fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let wrong_document = engine.derived_state.as_ref().unwrap().document.clone();
            let wrong_artifact = engine
                .derived_state
                .as_ref()
                .unwrap()
                .canonical_artifact
                .clone();
            let fresh_same_content_artifact =
                engine.canonical_schema.derive(&candidate_document).unwrap();
            let wrong_schema = prosemirror_schema();
            let mut wrong_resource_limits = engine.resource_limits.clone();
            wrong_resource_limits.max_input_bytes =
                wrong_resource_limits.max_input_bytes.saturating_add(1);
            let mut wrong_editing_limits = engine.editing_limits.clone();
            wrong_editing_limits.max_operations_per_transaction = wrong_editing_limits
                .max_operations_per_transaction
                .saturating_add(1);
            let wrong_max_length = match engine.max_length {
                Some(_) => None,
                None => Some(u32::MAX),
            };
            let foreign_doc =
                super::fresh_utf16_doc_excluding(&engine.durable_client_ids, engine.client_id());
            let foreign_store_fragment =
                foreign_doc.get_or_insert_xml_fragment(engine.fragment_name.as_str());
            let foreign_txn = foreign_doc.transact();
            let source_document = if matches!(case, Case::Document) {
                &wrong_document
            } else {
                &candidate_document
            };
            let canonical_artifact = match case {
                Case::CanonicalArtifact => &wrong_artifact,
                Case::CanonicalIdentity => &fresh_same_content_artifact,
                _ => &candidate_artifact,
            };
            let schema = if matches!(case, Case::Schema) {
                &wrong_schema
            } else {
                &engine.schema
            };
            let resource_limits = if matches!(case, Case::ResourceLimits) {
                &wrong_resource_limits
            } else {
                &engine.resource_limits
            };
            let editing_limits = if matches!(case, Case::EditingLimits) {
                &wrong_editing_limits
            } else {
                &engine.editing_limits
            };
            let max_length = if matches!(case, Case::MaxLength) {
                wrong_max_length
            } else {
                engine.max_length
            };
            let schema_fingerprint = if matches!(case, Case::SchemaFingerprint) {
                "contradictory-schema-fingerprint"
            } else {
                engine.schema_fingerprint.as_str()
            };
            let revision = if matches!(case, Case::Revision) {
                next_revision.saturating_add(1)
            } else {
                next_revision
            };
            let epoch = if matches!(case, Case::Epoch) {
                next_epoch.saturating_add(1)
            } else {
                next_epoch
            };
            let fragment = if matches!(case, Case::Fragment) {
                &wrong_fragment
            } else if matches!(case, Case::Store) {
                &foreign_store_fragment
            } else {
                &candidate_fragment
            };
            reset_prepared_admission_counts_for_test();
            set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
            let publish_txn = if matches!(case, Case::Store) {
                &foreign_txn
            } else {
                &txn
            };
            let error = publish_history_candidate_seed_for_test(
                unavailable,
                65_236,
                publish_txn,
                fragment,
                schema,
                source_document,
                canonical_artifact,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                epoch,
                revision,
            )
            .expect_err("contradictory history candidate claims must reject before probes");
            set_lookup_seed_hydration_failpoint_for_test(None);
            assert_eq!(
                error.code, "ENGINE_INVARIANT_FAILED",
                "{case:?}/{failpoint:?}"
            );
            let counts = take_prepared_admission_counts_for_test();
            assert_eq!(counts.staged_seed_preparations, 0, "{case:?}/{failpoint:?}");
            assert_eq!(
                counts.installed_base_seed_publications, 0,
                "{case:?}/{failpoint:?}"
            );
            assert_eq!(atomic_audit(&engine), before, "{case:?}/{failpoint:?}");
            assert!(Arc::ptr_eq(
                &installed,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ));
        }
    }
}

#[test]
fn history_candidate_seed_publication_rejects_same_store_deletion_after_mint() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };
    use yrs::types::Text;

    for failpoint in [
        LookupSeedHydrationFailpoint::BindingPublication,
        LookupSeedHydrationFailpoint::SeedPublication,
    ] {
        let (
            engine,
            candidate_doc,
            candidate_document,
            candidate_artifact,
            next_revision,
            next_epoch,
        ) = task5_candidate_publication_fixture();
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let (unavailable, text) = {
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let unavailable = prepare_history_candidate_capability_for_test(
                65_237,
                &txn,
                &fragment,
                &engine.schema,
                &candidate_document,
                &candidate_artifact,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                next_epoch,
                next_revision,
            );
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("candidate paragraph missing")
            };
            let XmlOut::Text(text) = paragraph.get(&txn, 0).unwrap() else {
                panic!("candidate text missing")
            };
            (unavailable, text)
        };
        {
            let mut txn = candidate_doc.transact_mut();
            text.remove_range(&mut txn, 0, 1);
        }
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let error = publish_history_candidate_seed_for_test(
            unavailable,
            65_237,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .expect_err("same-store deletion after mint must reject before publication probes");
        set_lookup_seed_hydration_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0, "{failpoint:?}");
        assert_eq!(counts.installed_base_seed_publications, 0, "{failpoint:?}");
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }
}

#[test]
fn history_candidate_read_rejects_a_self_consistent_document_from_another_store_before_probes() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (
        engine,
        candidate_doc,
        _candidate_document,
        _candidate_artifact,
        next_revision,
        next_epoch,
    ) = task5_candidate_publication_fixture();
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let foreign_state = engine.derived_state.as_ref().unwrap();
    let txn = candidate_doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();

    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::BindingPublication,
    ));
    let result = crate::yrs_engine::derived_state::prepare_history_candidate_read_for_test(
        65_238,
        &txn,
        &fragment,
        &engine.schema,
        &foreign_state.document,
        &foreign_state.canonical_artifact,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        next_epoch,
        next_revision,
    );
    set_lookup_seed_hydration_failpoint_for_test(None);

    let (_json, admission) = result
        .expect("exact codec read remains available for generic history fallback")
        .into_parts();
    assert!(
        admission.is_none(),
        "a self-consistent document/artifact from another store must not mint history proof"
    );
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 0);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
}

#[test]
fn authoritative_store_rebind_rejects_a_foreign_candidate_store() {
    let (engine, delta) = task5_changed_remote_fixture();
    let current_encoded = engine.encoded_state().unwrap();
    let build_candidate = || {
        let doc = super::fresh_utf16_doc_excluding(&engine.durable_client_ids, engine.client_id());
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(Update::decode_v1(&current_encoded).unwrap())
                .unwrap();
            txn.apply_update(Update::decode_v1(&delta).unwrap())
                .unwrap();
        }
        doc
    };
    let candidate_doc = build_candidate();
    let foreign_candidate_doc = build_candidate();
    let (candidate_document, candidate_artifact) = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let json =
            crate::yrs_engine::YrsDocumentCodec::new(&engine.schema, &engine.resource_limits)
                .read_json(&fragment, &txn)
                .unwrap();
        let document =
            from_prosemirror_json(&json, &engine.schema, UnknownTypeMode::Preserve).unwrap();
        let artifact = engine.canonical_schema.derive(&document).unwrap();
        (document, artifact)
    };
    let next_revision = engine.revision.checked_add(1).unwrap();
    let next_epoch = engine.yrs_state_epoch.checked_add(1).unwrap();
    let candidate_seed = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        prepare_history_candidate_capability_for_test(
            65_234,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .prepare_candidate_publication(
            65_234,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .unwrap()
    };
    let foreign_txn = foreign_candidate_doc.transact();
    let foreign_fragment = foreign_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    let live_txn = engine.doc.transact();
    let live_fragment = live_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();

    let error = candidate_seed
        .prepare_authoritative_store_rebind(
            65_235,
            &foreign_txn,
            &foreign_fragment,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
            &live_txn,
            &live_fragment,
        )
        .expect_err("a foreign candidate store must not be relabeled as live authority");
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
}

#[test]
fn authoritative_store_rebind_rejects_a_foreign_live_fragment_before_probes() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (engine, candidate_doc, candidate_document, candidate_artifact, next_revision, next_epoch) =
        task5_candidate_publication_fixture();
    let candidate_seed = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        prepare_history_candidate_capability_for_test(
            65_239,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .prepare_candidate_publication(
            65_239,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .unwrap()
    };
    let foreign_live_fragment = engine.doc.get_or_insert_xml_fragment("foreign-live");
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let candidate_txn = candidate_doc.transact();
    let candidate_fragment = candidate_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    let live_txn = engine.doc.transact();

    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::BindingPublication,
    ));
    let result = candidate_seed.prepare_authoritative_store_rebind(
        65_240,
        &candidate_txn,
        &candidate_fragment,
        &candidate_document,
        &candidate_artifact,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        next_epoch,
        next_revision,
        &live_txn,
        &foreign_live_fragment,
    );
    set_lookup_seed_hydration_failpoint_for_test(None);

    let error = result.expect_err("a foreign live fragment must reject before publication");
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 0);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
}

#[test]
fn matching_history_seed_publications_reach_all_four_exact_failpoint_stages() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (failpoint, expected_stage) in [
        (
            LookupSeedHydrationFailpoint::BindingPublication,
            "candidateBindingPublication",
        ),
        (
            LookupSeedHydrationFailpoint::SeedPublication,
            "candidateSeedPublication",
        ),
    ] {
        let (
            engine,
            candidate_doc,
            candidate_document,
            candidate_artifact,
            next_revision,
            next_epoch,
        ) = task5_candidate_publication_fixture();
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let unavailable = prepare_history_candidate_capability_for_test(
            65_241,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        );
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let result = unavailable.prepare_candidate_publication(
            65_241,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        );
        set_lookup_seed_hydration_failpoint_for_test(None);
        let error = result.expect_err("matching candidate must reach armed publication stage");
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(error.request_id, 65_241);
        assert_eq!(
            error.message.as_ref(),
            format!("mutation lookup seed allocation failed during {expected_stage}")
        );
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" }))
        );
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }

    let (engine, candidate_doc, candidate_document, candidate_artifact, next_revision, next_epoch) =
        task5_candidate_publication_fixture();
    let candidate_seed = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        prepare_history_candidate_capability_for_test(
            65_242,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .prepare_candidate_publication(
            65_242,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .unwrap()
    };
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let candidate_txn = candidate_doc.transact();
    let candidate_fragment = candidate_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    let live_txn = engine.doc.transact();
    let live_fragment = live_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    for (failpoint, expected_stage) in [
        (
            LookupSeedHydrationFailpoint::BindingPublication,
            "authoritativeStoreBindingPublication",
        ),
        (
            LookupSeedHydrationFailpoint::SeedPublication,
            "authoritativeStoreSeedPublication",
        ),
    ] {
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let result = candidate_seed.prepare_authoritative_store_rebind(
            65_243,
            &candidate_txn,
            &candidate_fragment,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
            &live_txn,
            &live_fragment,
        );
        set_lookup_seed_hydration_failpoint_for_test(None);
        let error = result.expect_err("matching rebind must reach armed publication stage");
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(error.request_id, 65_243);
        assert_eq!(
            error.message.as_ref(),
            format!("mutation lookup seed allocation failed during {expected_stage}")
        );
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" }))
        );
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }
}

#[test]
fn changed_remote_candidate_installs_only_its_candidate_owned_seed() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (mut engine, delta) = task5_changed_remote_fixture();
    let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let unchanged = engine.encoded_state().unwrap();
    reset_prepared_admission_counts_for_test();
    assert!(
        !engine
            .apply_remote_update_v1(65_230, &unchanged)
            .unwrap()
            .changed
    );
    let unchanged_counts = take_prepared_admission_counts_for_test();
    assert_eq!(unchanged_counts.staged_seed_preparations, 0);
    assert_eq!(unchanged_counts.installed_base_seed_publications, 0);
    assert!(Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));

    reset_prepared_admission_counts_for_test();
    assert!(
        engine
            .apply_remote_update_v1(65_231, &delta)
            .unwrap()
            .changed
    );
    let changed_counts = take_prepared_admission_counts_for_test();
    assert_eq!(changed_counts.staged_seed_preparations, 1);
    assert_eq!(changed_counts.installed_base_seed_publications, 0);
    assert!(!Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
}

#[test]
fn remote_live_store_rebind_allocation_failure_is_prewrite_and_atomic() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (mut engine, delta) = task5_changed_remote_fixture();
    let before = atomic_audit(&engine);
    let quarantine_before = engine.quarantined_remote_update.clone();
    let ledger_before = engine.history.replay_ledger_allocation_audit_for_test();
    let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::SeedPublication,
    ));
    let result = engine.apply_remote_update_v1(65_232, &delta);
    set_lookup_seed_hydration_failpoint_for_test(None);
    let error = result.expect_err("live-store rebind allocation failure must reject");
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(
        error.message.as_ref(),
        "mutation lookup seed allocation failed during authoritativeStoreSeedPublication"
    );
    assert_eq!(
        error.details,
        Some(json!({ "field": "mutationLookupSeed" }))
    );
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 0);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(engine.quarantined_remote_update, quarantine_before);
    assert_eq!(
        engine.history.replay_ledger_allocation_audit_for_test(),
        ledger_before
    );
    assert!(Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
}

#[test]
fn deferred_finalization_reuses_saved_evidence_without_revalidation() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };

    reset_prepared_admission_counts_for_test();
    let (engine, deferred, mut context, transaction, expected_document) =
        deferred_finalization_fixture();
    engine.prepare_mutation_identity(&mut context).unwrap();
    reset_full_pass_counts_for_test();
    let prepared = engine
        .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
        .unwrap();
    assert!(prepared.admits_expected_document(&expected_document));
    let passes = take_full_pass_counts_for_test();
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(passes.planner_simulations, 0);
    assert_eq!(passes.document_validations, 0);
    assert_eq!(passes.render_limit_tree_scans, 0);
    assert_eq!(passes.render_identity_scans, 0);
    assert_eq!(admission.deferred_capsules_created, 1);
    assert_eq!(admission.deferred_capsules_finalized, 1);
}

#[test]
fn deferred_capsule_tamper_cases_reject_before_write() {
    for case in
        crate::yrs_engine::prepared_admission::DeferredCommandAdmission::tamper_cases_for_test()
    {
        let (engine, deferred, mut context, transaction, expected_document) =
            deferred_tamper_fixture(case);
        engine.prepare_mutation_identity(&mut context).unwrap();
        let before = atomic_audit(&engine);
        let error = engine
            .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
            .expect_err(&format!("tampered deferred capsule must reject: {case}"));
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
    }
}

#[test]
fn deferred_same_summary_evidence_replacements_reject_without_identity_scans() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    for case in ["position", "render"] {
        let (engine, mut deferred, mut context, transaction, expected_document) =
            deferred_finalization_fixture();
        deferred.tamper_same_summary_evidence_for_test(case);
        engine.prepare_mutation_identity(&mut context).unwrap();
        let before = atomic_audit(&engine);
        reset_full_pass_counts_for_test();

        let error = engine
            .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
        let passes = take_full_pass_counts_for_test();
        assert_eq!(passes.position_map_clones, 0, "{case}");
        assert_eq!(passes.render_limit_tree_scans, 0, "{case}");
        assert_eq!(passes.render_identity_scans, 0, "{case}");
    }
}

#[test]
fn deferred_shape_rejects_matching_transaction_position_tamper() {
    let (engine, mut deferred, mut context, mut transaction, expected_document) =
        deferred_finalization_fixture();
    deferred.tamper_matching_transaction_position_for_test(&mut transaction);
    engine.prepare_mutation_identity(&mut context).unwrap();
    let before = atomic_audit(&engine);

    let error = engine
        .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn deferred_finalization_preserves_warmed_candidate_scalar_caches() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let (engine, deferred, mut context, transaction, expected_document) =
        deferred_finalization_fixture();
    let (expected_len, expected_sha256) = deferred.warm_candidate_caches_for_test();
    engine.prepare_mutation_identity(&mut context).unwrap();
    reset_full_pass_counts_for_test();

    let prepared = engine
        .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
        .unwrap();

    assert_eq!(prepared.canonical_artifact().serialized_len(), expected_len);
    assert_eq!(prepared.canonical_artifact().sha256(), expected_sha256);
    let passes = take_full_pass_counts_for_test();
    assert_eq!(passes.canonical_serializations, 0);
    assert_eq!(passes.canonical_hashes, 0);
}

#[test]
fn deferred_finalization_rejects_mismatched_prefilled_candidate_caches() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    for case in ["length", "sha256"] {
        let (engine, mut deferred, mut context, transaction, expected_document) =
            deferred_finalization_fixture();
        let _ = deferred.warm_candidate_caches_for_test();
        deferred.tamper_candidate_cache_for_test(case);
        engine.prepare_mutation_identity(&mut context).unwrap();
        let before = atomic_audit(&engine);
        reset_full_pass_counts_for_test();

        let error = engine
            .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
        let passes = take_full_pass_counts_for_test();
        assert_eq!(passes.canonical_serializations, 0, "{case}");
        assert_eq!(passes.canonical_hashes, 0, "{case}");
    }
}

#[test]
fn imported_commands_plan_not_applicable_and_stored_marks_before_hydration() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut not_applicable = import_document_with_unavailable_lookup_seed();
    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::InitialReservation,
    ));
    let result = not_applicable
        .apply_command(65_130, TypedCommand::ToggleTaskItemChecked)
        .unwrap();
    set_lookup_seed_hydration_failpoint_for_test(None);
    assert!(result.is_none());
    let not_applicable_counts = take_prepared_admission_counts_for_test();
    assert_eq!(not_applicable_counts.staged_seed_preparations, 0);
    assert_eq!(not_applicable_counts.installed_base_seed_publications, 0);
    assert!(not_applicable
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());

    let mut stored_mark = import_document_with_unavailable_lookup_seed();
    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::InitialReservation,
    ));
    let result = stored_mark
        .apply_command(
            65_131,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap();
    set_lookup_seed_hydration_failpoint_for_test(None);
    assert!(result.is_some());
    let stored_mark_counts = take_prepared_admission_counts_for_test();
    assert_eq!(stored_mark_counts.staged_seed_preparations, 0);
    assert_eq!(stored_mark_counts.installed_base_seed_publications, 0);
    assert_eq!(
        stored_mark
            .stored_marks()
            .unwrap()
            .iter()
            .map(Mark::mark_type)
            .collect::<Vec<_>>(),
        vec!["bold"]
    );
    assert!(stored_mark
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
}

#[test]
fn immediate_import_local_input_local_api_and_structural_routes_hydrate_real_consumers() {
    let mut local_input = import_document_with_unavailable_lookup_seed();
    let mut transaction = insert_transaction(&local_input, 65_140);
    transaction.origin = TransactionOrigin::LocalInput;
    local_input.apply_typed_transaction(transaction).unwrap();
    assert!(local_input
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    let mut local_api = import_document_with_unavailable_lookup_seed();
    local_api
        .apply_typed_transaction(insert_transaction(&local_api, 65_141))
        .unwrap();
    assert!(local_api
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    let mut structural = import_document_with_unavailable_lookup_seed();
    crate::yrs_engine::mutation::reset_localized_lookup_counts_for_test();
    structural
        .apply_command(
            65_142,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap()
        .expect("paragraph should wrap in a bullet list");
    assert_eq!(
        crate::yrs_engine::mutation::take_localized_lookup_counts_for_test(),
        (0, 0, 0),
        "the structural command must consume the staged seed without a live rebuild"
    );
    assert_eq!(
        structural.document_json().unwrap()["content"][0]["type"],
        "bulletList"
    );
}

#[test]
fn immediate_import_noop_remote_candidate_does_not_hydrate_live_seed() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    let update = engine.encoded_state().unwrap();
    let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::InitialReservation,
    ));

    let commit = engine.apply_remote_update_v1(65_143, &update).unwrap();

    set_lookup_seed_hydration_failpoint_for_test(None);
    assert!(!commit.changed);
    assert!(Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());

    let mut source = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    source.apply_remote_update_v1(65_144, &update).unwrap();
    source
        .apply_command(65_145, TypedCommand::InsertText { text: "r".into() })
        .unwrap()
        .unwrap();
    let target_vector = engine.doc.transact().state_vector();
    let delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);

    let commit = engine.apply_remote_update_v1(65_146, &delta).unwrap();

    assert!(commit.changed);
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
}

#[test]
fn prepare_mutation_context_does_not_publish_the_installed_seed() {
    let engine = import_document_with_unavailable_lookup_seed();
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    crate::yrs_engine::observability::reset_prepared_admission_counts_for_test();
    let context = engine.prepare_mutation_lookup_seed(65_210).unwrap();
    assert!(context.lookup_seed().is_ready_for_test());
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    assert_eq!(atomic_audit(&engine), before);
    let counts = crate::yrs_engine::observability::take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn prepared_mutation_identity_is_lazy_and_does_not_mutate_installed_caches() {
    let engine = import_document_with_unavailable_lookup_seed();
    let mut context = engine.prepare_mutation_lookup_seed(65_211).unwrap();
    crate::yrs_engine::observability::reset_prepared_admission_counts_for_test();
    assert!(context.materialized_identity().is_none());
    engine.prepare_mutation_identity(&mut context).unwrap();
    assert!(context.materialized_identity().is_some());
    assert_eq!(
        crate::yrs_engine::observability::take_prepared_admission_counts_for_test()
            .staged_identity_materializations,
        1,
    );
    assert!(!engine
        .derived_state
        .as_ref()
        .unwrap()
        .validation_certificate
        .canonical_fingerprint_materialized_for_test());
    assert!(!engine
        .derived_state
        .as_ref()
        .unwrap()
        .localized_text_index
        .as_ref()
        .unwrap()
        .canonical_fingerprint_materialized_for_test());
}

#[test]
fn prepared_mutation_authority_rejects_request_mismatch_atomically() {
    let engine = import_document_with_unavailable_lookup_seed();
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    crate::yrs_engine::observability::reset_prepared_admission_counts_for_test();
    let context = engine.prepare_mutation_lookup_seed(65_212).unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();

    let error = match context.authority(
        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
            request_id: 65_213,
            installed: state,
            txn: &txn,
            fragment: &fragment,
            fragment_name: &engine.fragment_name,
            schema_fingerprint: &engine.schema_fingerprint,
            resource_limits: &engine.resource_limits,
            editing_limits: &engine.editing_limits,
            max_length: engine.max_length,
            document_revision: engine.revision,
            state_revision: engine.state_revision,
            yrs_state_epoch: engine.yrs_state_epoch,
        },
    ) {
        Ok(_) => panic!("a prepared context must not authorize another request"),
        Err(error) => error,
    };
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.request_id, 65_212);

    {
        let authority = context
            .authority(
                crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                    request_id: 65_212,
                    installed: state,
                    txn: &txn,
                    fragment: &fragment,
                    fragment_name: &engine.fragment_name,
                    schema_fingerprint: &engine.schema_fingerprint,
                    resource_limits: &engine.resource_limits,
                    editing_limits: &engine.editing_limits,
                    max_length: engine.max_length,
                    document_revision: engine.revision,
                    state_revision: engine.state_revision,
                    yrs_state_epoch: engine.yrs_state_epoch,
                },
            )
            .unwrap();
        assert!(authority.lookup_seed().is_ready_for_test());
    }
    drop(txn);

    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    assert_eq!(atomic_audit(&engine), before);
    let counts = crate::yrs_engine::observability::take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn lookup_seed_rejects_same_value_stale_canonical_artifact_identity() {
    let mut engine = import_document_with_unavailable_lookup_seed();
    engine.ensure_mutation_lookup_seed(65_108).unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    let stale_seed = Arc::clone(&state.mutation_lookup_seed);
    assert!(stale_seed.matches_canonical_artifact(&state.canonical_artifact));

    let replacement = state
        .canonical_artifact
        .schema_context()
        .derive(&state.document)
        .unwrap();
    assert!(!replacement.ptr_eq(&state.canonical_artifact));
    engine.derived_state.as_mut().unwrap().canonical_artifact = replacement;
    assert!(!stale_seed
        .matches_canonical_artifact(&engine.derived_state.as_ref().unwrap().canonical_artifact));

    crate::yrs_engine::mutation::reset_localized_lookup_counts_for_test();
    engine.ensure_mutation_lookup_seed(65_109).unwrap();
    assert_eq!(
        crate::yrs_engine::mutation::take_localized_lookup_counts_for_test().0,
        1
    );
    let state = engine.derived_state.as_ref().unwrap();
    assert!(state
        .mutation_lookup_seed
        .matches_canonical_artifact(&state.canonical_artifact));
}

#[test]
fn unavailable_lookup_hydration_failure_is_atomic() {
    let mut engine = import_document_with_unavailable_lookup_seed();
    engine.fragment_name = "missing-after-import".into();
    let before = atomic_audit(&engine);
    let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);

    let error = engine
        .apply_command(65_108, TypedCommand::InsertText { text: "x".into() })
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert!(Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn unavailable_lookup_allocation_failpoints_are_resource_errors_and_atomic() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    for (index, failpoint) in [
        LookupSeedHydrationFailpoint::InitialReservation,
        LookupSeedHydrationFailpoint::MapGrowth,
        LookupSeedHydrationFailpoint::MapPublication,
        LookupSeedHydrationFailpoint::BindingPublication,
        LookupSeedHydrationFailpoint::SeedPublication,
    ]
    .into_iter()
    .enumerate()
    {
        let mut engine = import_document_with_unavailable_lookup_seed();
        assert!(engine.prepared_candidate_cache.take().is_some());
        let before = atomic_audit(&engine);
        let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));

        let error = engine
            .apply_command(
                65_120 + index as u64,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap_err();

        set_lookup_seed_hydration_failpoint_for_test(None);
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED", "{failpoint:?}");
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" })),
            "{failpoint:?}"
        );
        assert!(
            Arc::ptr_eq(
                &unavailable,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ),
            "{failpoint:?}"
        );
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
    }
}

#[test]
fn lookup_seed_hydration_does_not_reserve_growth_with_spare_capacity() {
    use crate::yrs_engine::mutation::{
        reset_lookup_seed_map_growth_attempts_for_test,
        take_lookup_seed_map_growth_attempts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    assert!(engine.prepared_candidate_cache.take().is_some());
    reset_lookup_seed_map_growth_attempts_for_test();
    engine
        .apply_command(65_126, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert_eq!(take_lookup_seed_map_growth_attempts_for_test(), 0);
}

#[test]
fn engine_commands_reuse_the_proven_schema_context_without_recomputing_it() {
    use crate::yrs_engine::canonical::{
        reset_canonical_schema_context_count_for_test, take_canonical_schema_context_count_for_test,
    };

    let mut engine = transaction_engine();
    reset_canonical_schema_context_count_for_test();
    engine
        .apply_command(65_001, TypedCommand::InsertText { text: "x".into() })
        .unwrap();

    assert_eq!(take_canonical_schema_context_count_for_test(), 0);
}

#[test]
fn collision_excluding_candidate_selection_retries_live_and_durable_ids() {
    let durable = HashSet::from([7_u64]);
    let mut ids = [5_u64, 7_u64, 11_u64].into_iter();
    let selected = fresh_utf16_doc_excluding_with(&durable, 5, || {
        Doc::with_options(Options {
            client_id: ClientID::new(ids.next().unwrap()),
            offset_kind: OffsetKind::Utf16,
            ..Options::default()
        })
    });

    assert_eq!(selected.client_id().get(), 11);
}

#[test]
fn restored_and_local_candidates_cache_all_relevant_durable_clients() {
    let config = || crate::yrs_engine::YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "doc".into(),
            lineage_id: "lineage".into(),
        }),
    };
    let source = crate::yrs_engine::YrsDocumentEngine::new(config()).unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let expected = Update::decode_v1(&snapshot.encoded_state)
        .unwrap()
        .state_vector()
        .iter()
        .map(|(client, _)| client.get())
        .collect::<HashSet<_>>();
    let mut target = crate::yrs_engine::YrsDocumentEngine::new(config()).unwrap();

    target.restore_snapshot(&snapshot).unwrap();
    assert_eq!(target.durable_client_ids, expected);

    target
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"local"}]}]}"#,
            crate::yrs_engine::TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_eq!(
        target.durable_client_ids,
        HashSet::from([target.client_id()])
    );
}

#[test]
fn revision_overflow_rejects_before_candidate_swap() {
    let mut engine =
        crate::yrs_engine::YrsDocumentEngine::new(crate::yrs_engine::YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap();
    engine.revision = u64::MAX;
    engine.derived_state.as_mut().unwrap().document_revision = u64::MAX;
    let before_client = engine.client_id();
    let before_json = engine.document_json();
    let before_state = engine.encoded_state().unwrap();

    let error = engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"changed"}]}]}"#,
            crate::yrs_engine::TransactionOrigin::DocumentImport,
        )
        .unwrap_err();

    assert_eq!(error.code, "REVISION_OVERFLOW");
    assert_eq!(engine.revision(), u64::MAX);
    assert_eq!(engine.client_id(), before_client);
    assert_eq!(engine.document_json(), before_json);
    assert_eq!(engine.encoded_state().unwrap(), before_state);
}

#[test]
fn candidate_state_revision_and_epoch_overflow_reject_before_swap() {
    for field in ["stateRevision", "yrsStateEpoch"] {
        let mut engine = transaction_engine();
        if field == "stateRevision" {
            engine.state_revision = u64::MAX;
            engine.derived_state.as_mut().unwrap().state_revision = u64::MAX;
        } else {
            engine.yrs_state_epoch = u64::MAX;
        }
        let before = atomic_audit(&engine);

        let error = engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"changed"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap_err();

        assert_eq!(error.code, "REVISION_OVERFLOW", "{field}");
        assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
        assert_eq!(atomic_audit(&engine), before, "{field}");
    }
}

#[test]
fn identical_selection_is_no_op_even_when_state_revision_is_max() {
    let mut engine = transaction_engine();
    engine.state_revision = u64::MAX;
    if let Some(state) = &mut engine.derived_state {
        state.state_revision = u64::MAX;
    }
    let before = atomic_audit(&engine);
    let transaction = TypedTransaction {
        request_id: 90_001,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![],
        selection_intent: SelectionIntent::Set(crate::yrs_engine::SelectionInput::Text {
            anchor: RevisionedPosition {
                offset: 0,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::Before,
            },
            head: RevisionedPosition {
                offset: 0,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::Before,
            },
        }),
        history_policy: HistoryPolicy::Skip,
    };
    let commit = engine.apply_typed_transaction(transaction).unwrap();
    assert!(!commit.changed);
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn snapshot_export_envelope_budget_has_exact_and_over_boundaries_without_mutation() {
    let mut engine =
        crate::yrs_engine::YrsDocumentEngine::new(crate::yrs_engine::YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap();
    let state = engine.encoded_state().unwrap();
    let metadata_bytes =
        "doc".len() + "lineage".len() + "prosemirror".len() + engine.schema_fingerprint().len();
    engine.resource_limits.max_input_bytes = metadata_bytes;
    engine.resource_limits.max_encoded_state_bytes = state.len();
    assert!(engine.export_snapshot().is_ok());

    let before_revision = engine.revision();
    let before_client = engine.client_id();
    let before_json = engine.document_json();
    engine.resource_limits.max_input_bytes = metadata_bytes - 1;
    let error = engine.export_snapshot().unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        error.details,
        Some(serde_json::json!({"phase": "snapshotExport"}))
    );
    assert_eq!(engine.revision(), before_revision);
    assert_eq!(engine.client_id(), before_client);
    assert_eq!(engine.document_json(), before_json);
    assert_eq!(engine.encoded_state().unwrap(), state);
}

#[test]
fn typed_transaction_rejects_every_revision_or_epoch_overflow_before_mutation() {
    for field in ["documentRevision", "stateRevision", "yrsStateEpoch"] {
        let mut engine = transaction_engine();
        match field {
            "documentRevision" => {
                engine.revision = u64::MAX;
                engine.derived_state.as_mut().unwrap().document_revision = u64::MAX;
            }
            "stateRevision" => {
                engine.state_revision = u64::MAX;
                engine.derived_state.as_mut().unwrap().state_revision = u64::MAX;
            }
            "yrsStateEpoch" => engine.yrs_state_epoch = u64::MAX,
            _ => unreachable!(),
        }
        let transaction = insert_transaction(&engine, 71);
        let before = atomic_audit(&engine);

        let error = engine.apply_typed_transaction(transaction).unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{field}");
        assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
        assert_eq!(atomic_audit(&engine), before, "{field}");
    }
}

#[test]
fn compiled_transaction_epoch_is_checked_before_yrs_metadata_revalidation() {
    for changed in [true, false] {
        let mut engine = transaction_engine();
        let transaction = if changed {
            insert_transaction(&engine, 72)
        } else {
            TypedTransaction {
                request_id: 72,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            }
        };
        let compiled = engine.compile_typed_transaction(transaction).unwrap();
        engine.yrs_state_epoch += 1;
        let before = atomic_audit(&engine);

        let error = engine
            .apply_compiled_transaction(compiled, false)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "changed={changed}");
        assert!(error.message.contains("stale"), "changed={changed}");
        assert_eq!(atomic_audit(&engine), before, "changed={changed}");
    }
}

#[test]
fn compiled_transaction_state_revision_is_checked_before_result_or_no_op_work() {
    for changed in [true, false] {
        let mut engine = transaction_engine();
        let transaction = if changed {
            insert_transaction(&engine, 72_001)
        } else {
            TypedTransaction {
                request_id: 72_001,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            }
        };
        let compiled = engine.compile_typed_transaction(transaction).unwrap();
        let seed = engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .clone();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 72_002,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert!(Arc::ptr_eq(
            &seed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        let before = atomic_audit(&engine);

        let error = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "changed={changed}");
        assert!(error.message.contains("stale"), "changed={changed}");
        assert_eq!(atomic_audit(&engine), before, "changed={changed}");
    }
}

#[test]
fn projected_encoded_ceiling_accepts_exact_and_rejects_one_under_without_new_clock() {
    let mut exact = transaction_engine();
    let exact_transaction = insert_transaction(&exact, 73);
    let exact_compiled = exact
        .compile_typed_transaction(exact_transaction.clone())
        .unwrap();
    let exact_limit = exact
        .encoded_state()
        .unwrap()
        .len()
        .checked_add(exact_compiled.encoded_growth_bound)
        .unwrap();
    exact.resource_limits.max_encoded_state_bytes = exact_limit;

    let commit = exact.apply_typed_transaction(exact_transaction).unwrap();

    assert!(commit.changed);
    assert!(exact.encoded_state().unwrap().len() <= exact_limit);

    let mut one_under = transaction_engine();
    let rejected_transaction = insert_transaction(&one_under, 74);
    let rejected_compiled = one_under
        .compile_typed_transaction(rejected_transaction.clone())
        .unwrap();
    let rejected_limit = one_under
        .encoded_state()
        .unwrap()
        .len()
        .checked_add(rejected_compiled.encoded_growth_bound)
        .unwrap()
        - 1;
    one_under.resource_limits.max_encoded_state_bytes = rejected_limit;
    let before = atomic_audit(&one_under);

    let error = one_under
        .apply_typed_transaction(rejected_transaction)
        .unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxEncodedStateBytes" }))
    );
    assert_eq!(error.limit, Some(rejected_limit as u64));
    assert_eq!(error.actual, Some((rejected_limit + 1) as u64));
    assert_eq!(atomic_audit(&one_under), before);
}

#[test]
fn canonical_cache_output_accepts_exact_rejects_one_under_and_reuses_empty_noop_cache() {
    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "x" }]
        }]
    });
    let exact_bytes = serde_json::to_vec(&expected).unwrap().len();

    let mut exact = transaction_engine();
    exact.editing_limits.max_derived_output_bytes = exact_bytes;
    let transaction = insert_transaction(&exact, 77);
    exact.apply_typed_transaction(transaction).unwrap();
    assert_eq!(exact.document_json(), Some(expected));

    let mut one_under = transaction_engine();
    one_under.editing_limits.max_derived_output_bytes = exact_bytes - 1;
    let transaction = insert_transaction(&one_under, 78);
    let before = atomic_audit(&one_under);
    let error = one_under.apply_typed_transaction(transaction).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some((exact_bytes - 1) as u64));
    assert_eq!(error.actual, Some(exact_bytes as u64));
    assert_eq!(atomic_audit(&one_under), before);

    let mut empty_noop = transaction_engine();
    empty_noop.editing_limits.max_derived_output_bytes = 1;
    let transaction = TypedTransaction {
        request_id: 79,
        base_document_revision: empty_noop.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };
    let before = atomic_audit(&empty_noop);
    let commit = empty_noop.apply_typed_transaction(transaction).unwrap();
    assert!(!commit.changed);
    assert_eq!(atomic_audit(&empty_noop), before);
}

#[test]
fn local_empty_initialization_enforces_the_exact_canonical_output_ceiling() {
    let schema = tiptap_schema();
    let document = schema.default_document().unwrap();
    let value = crate::serialize::to_prosemirror_json(&document, &schema);
    let exact = serde_json::to_vec(&value).unwrap().len();
    let config = |limit| YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        },
        max_length: None,
        scope: None,
    };

    assert_eq!(
        YrsDocumentEngine::new(config(exact))
            .unwrap()
            .document_json(),
        Some(value)
    );
    let error = YrsDocumentEngine::new(config(exact - 1)).err().unwrap();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(exact - 1));
    assert_eq!(error.actual, Some(exact));
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );
}

#[test]
fn json_and_html_import_enforce_output_before_any_live_state_change() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };

    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "x"}]
        }]
    });
    let exact = serde_json::to_vec(&expected).unwrap().len();
    for (is_html, input) in [
        (false, serde_json::to_string(&expected).unwrap()),
        (true, "<p>x</p>".to_string()),
    ] {
        let mut accepted = transaction_engine();
        accepted.editing_limits.max_derived_output_bytes = exact;
        reset_canonical_artifact_counts_for_test();
        let commit = if is_html {
            accepted.import_html(
                &input,
                &FromHtmlOptions::default(),
                TransactionOrigin::DocumentImport,
            )
        } else {
            accepted.import_json(&input, TransactionOrigin::DocumentImport)
        }
        .unwrap();
        assert!(commit.changed);
        assert_eq!(accepted.document_json(), Some(expected.clone()));
        assert_eq!(
            take_canonical_artifact_counts_for_test(),
            (1, usize::from(is_html))
        );

        let mut rejected = transaction_engine();
        rejected.editing_limits.max_derived_output_bytes = exact - 1;
        rejected.revision = u64::MAX;
        rejected.state_revision = u64::MAX;
        rejected.yrs_state_epoch = u64::MAX;
        rejected.derived_state.as_mut().unwrap().document_revision = u64::MAX;
        rejected.derived_state.as_mut().unwrap().state_revision = u64::MAX;
        let before = atomic_audit(&rejected);
        let artifact_before = rejected
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .clone();
        reset_canonical_artifact_counts_for_test();
        let error = if is_html {
            rejected.import_html(
                &input,
                &FromHtmlOptions::default(),
                TransactionOrigin::DocumentImport,
            )
        } else {
            rejected.import_json(&input, TransactionOrigin::DocumentImport)
        }
        .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED", "is_html={is_html}");
        assert_eq!(error.limit, Some(exact - 1));
        assert_eq!(error.actual, Some(exact));
        assert_eq!(
            error.details,
            Some(json!({ "field": "maxDerivedOutputBytes" }))
        );
        assert_eq!(atomic_audit(&rejected), before);
        assert!(
            artifact_before.ptr_eq(&rejected.derived_state.as_ref().unwrap().canonical_artifact)
        );
        assert_eq!(take_canonical_artifact_counts_for_test(), (1, 1));
    }
}

#[test]
fn changed_snapshot_restore_enforces_output_before_revisions_history_or_swap() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let exact = serde_json::to_vec(&source.document_json().unwrap())
        .unwrap()
        .len();

    let mut accepted = transaction_engine();
    accepted.editing_limits.max_derived_output_bytes = exact;
    reset_canonical_artifact_counts_for_test();
    assert!(accepted.restore_snapshot(&snapshot).unwrap().changed);
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));
    accepted.editing_limits.max_derived_output_bytes = 1;
    reset_canonical_artifact_counts_for_test();
    assert!(!accepted.restore_snapshot(&snapshot).unwrap().changed);
    assert_eq!(take_canonical_artifact_counts_for_test(), (0, 0));

    let mut rejected = transaction_engine();
    rejected.editing_limits.max_derived_output_bytes = exact - 1;
    rejected.revision = u64::MAX;
    rejected.state_revision = u64::MAX;
    rejected.yrs_state_epoch = u64::MAX;
    rejected.derived_state.as_mut().unwrap().document_revision = u64::MAX;
    rejected.derived_state.as_mut().unwrap().state_revision = u64::MAX;
    let before = atomic_audit(&rejected);
    let artifact_before = rejected
        .derived_state
        .as_ref()
        .unwrap()
        .canonical_artifact
        .clone();
    reset_canonical_artifact_counts_for_test();
    let error = rejected.restore_snapshot(&snapshot).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(exact - 1));
    assert_eq!(error.actual, Some(exact));
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );
    assert_eq!(atomic_audit(&rejected), before);
    assert!(artifact_before.ptr_eq(&rejected.derived_state.as_ref().unwrap().canonical_artifact));
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 1));
}

#[test]
fn typed_commit_installs_local_client_origin_and_candidate_revisions() {
    let mut source = transaction_engine();
    let imported = source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(imported.changed);
    assert_eq!(
        (
            source.revision,
            source.state_revision,
            source.yrs_state_epoch
        ),
        (1, 1, 1)
    );
    let snapshot = source.export_snapshot().unwrap();
    let mut target = transaction_engine();
    target.restore_snapshot(&snapshot).unwrap();
    let local_client = target.client_id();
    assert!(!target.durable_client_ids.contains(&local_client));
    assert_eq!(
        (
            target.revision,
            target.state_revision,
            target.yrs_state_epoch
        ),
        (1, 1, 1)
    );

    let transaction = insert_transaction(&target, 75);
    let commit = target.apply_typed_transaction(transaction).unwrap();

    assert!(commit.changed);
    assert!(target.durable_client_ids.contains(&local_client));
    assert_eq!(
        target.last_committed_origin,
        Some(TransactionOrigin::LocalApi)
    );
    assert_eq!(
        (
            target.revision,
            target.state_revision,
            target.yrs_state_epoch
        ),
        (2, 2, 2)
    );

    let unchanged = target.document_json().unwrap();
    let commit = target
        .import_json(
            &serde_json::to_string(&unchanged).unwrap(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(!commit.changed);
    assert_eq!(
        (
            target.revision,
            target.state_revision,
            target.yrs_state_epoch
        ),
        (2, 2, 2)
    );
}

#[test]
fn restored_deletion_only_commit_does_not_claim_an_unauthored_local_client() {
    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let mut target = transaction_engine();
    target.restore_snapshot(&snapshot).unwrap();
    let local_client = target.client_id();
    assert!(!target.durable_client_ids.contains(&local_client));
    let from = RevisionedPosition {
        offset: 0,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let to = RevisionedPosition { offset: 1, ..from };
    let transaction = TypedTransaction {
        request_id: 80,
        base_document_revision: target.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![TypedOperation::DeleteRange {
            range: RevisionedRange { from, to },
        }],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };

    let compiled = target
        .compile_typed_transaction(transaction.clone())
        .unwrap();
    assert_eq!(compiled.authored_clock_units, 0);
    target.apply_typed_transaction(transaction).unwrap();

    assert_prepared_candidate_state_vector_exact(&target);
    assert!(!target.durable_client_ids.contains(&local_client));
    let durable_clients = Update::decode_v1(&target.encoded_state().unwrap())
        .unwrap()
        .state_vector();
    assert!(durable_clients.get(&ClientID::new(local_client)) == 0);
}

#[test]
fn restored_format_only_commit_records_its_authored_local_clock() {
    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let mut target = transaction_engine();
    target.restore_snapshot(&snapshot).unwrap();
    let local_client = target.client_id();
    let from = RevisionedPosition {
        offset: 0,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let to = RevisionedPosition { offset: 1, ..from };
    let transaction = TypedTransaction {
        request_id: 81,
        base_document_revision: target.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![TypedOperation::AddMark {
            range: RevisionedRange { from, to },
            mark: Mark::new("bold".into(), HashMap::new()),
        }],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };

    let compiled = target
        .compile_typed_transaction(transaction.clone())
        .unwrap();
    assert!(compiled.authored_clock_units > 0);
    target.apply_typed_transaction(transaction).unwrap();

    assert_prepared_candidate_state_vector_exact(&target);
    assert!(target.durable_client_ids.contains(&local_client));
    let durable_clients = Update::decode_v1(&target.encoded_state().unwrap())
        .unwrap()
        .state_vector();
    assert!(durable_clients.get(&ClientID::new(local_client)) > 0);
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

fn unaffected_text_sticky(
    engine: &YrsDocumentEngine,
    text_child: u32,
    utf16_index: u32,
) -> (crate::yrs_engine::RelativePoint, BranchPtr, u32) {
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("expected paragraph")
    };
    let XmlOut::Text(text) = paragraph.get(&txn, text_child).unwrap() else {
        panic!("expected text child")
    };
    let branch = BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&text));
    let sticky = StickyIndex::at(&txn, branch, utf16_index, Assoc::After).unwrap();
    let point = crate::yrs_engine::RelativePoint {
        sticky,
        affinity: Affinity::After,
    };
    let Some(offset) = point.sticky.get_offset(&txn) else {
        panic!("sticky must resolve")
    };
    let doc_pos = crate::yrs_engine::position::relative_point_to_doc_pos(
        &txn,
        &fragment,
        &point,
        &engine.schema,
    )
    .unwrap();
    let scalar = engine
        .position_map()
        .unwrap()
        .doc_to_scalar(doc_pos, engine.document().unwrap());
    (point, offset.branch, scalar)
}

fn assert_unaffected_sticky(
    engine: &YrsDocumentEngine,
    point: &crate::yrs_engine::RelativePoint,
    branch: BranchPtr,
    expected_scalar: u32,
) {
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let offset = point.sticky.get_offset(&txn).unwrap();
    assert_eq!(
        offset.branch, branch,
        "unaffected Yrs branch identity changed"
    );
    let doc_pos = crate::yrs_engine::position::relative_point_to_doc_pos(
        &txn,
        &fragment,
        point,
        &engine.schema,
    )
    .unwrap();
    assert_eq!(
        engine
            .position_map()
            .unwrap()
            .doc_to_scalar(doc_pos, engine.document().unwrap()),
        expected_scalar,
        "unaffected sticky point moved to the wrong rendered position"
    );
}

#[test]
fn granular_command_lowering_preserves_classification_locality_and_unaffected_sticky_identity() {
    let mut format = transaction_engine();
    format
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a","marks":[{"type":"link","attrs":{"href":"old"}}]},{"type":"text","text":"bc tail"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let (format_sticky, format_branch, format_scalar) = unaffected_text_sticky(&format, 1, 5);
    select_text(&mut format, 100, 2, 0);
    let CommandPlan::Transaction(format_transaction) = format
        .plan_command(
            101,
            TypedCommand::SetMark {
                mark_type: "link".into(),
                attrs: HashMap::from([("href".into(), json!("new"))]),
            },
        )
        .unwrap()
    else {
        panic!("range format must plan")
    };
    assert!(matches!(
        format_transaction.operations.as_slice(),
        [
            TypedOperation::RemoveMark { .. },
            TypedOperation::AddMark { .. }
        ]
    ));
    let compiled = format
        .compile_typed_transaction(format_transaction.clone())
        .unwrap();
    assert_eq!(
        compiled.history_class,
        crate::yrs_engine::compiler::HistoryClass::Format
    );
    assert_eq!(
        compiled.position_update_mode,
        crate::position::update::UpdateMode::MarksOnly
    );
    assert_eq!(compiled.affected_top_level_blocks, vec![0]);
    let format_result = format
        .apply_typed_transaction_with_result(format_transaction)
        .unwrap();
    let crate::yrs_engine::RenderUpdate::Patch(format_patch) = format_result.render_update else {
        panic!("range format must produce a local render patch")
    };
    assert_eq!(
        (
            format_patch.start_index,
            format_patch.delete_count,
            format_patch.blocks.len(),
        ),
        (0, 1, 1)
    );
    assert_unaffected_sticky(&format, &format_sticky, format_branch, format_scalar);
    assert!(format.can_undo());

    let mut replace = transaction_engine();
    replace
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"left target right"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let (replace_sticky, replace_branch, replace_scalar) = unaffected_text_sticky(&replace, 0, 13);
    select_text(&mut replace, 102, 11, 5);
    let CommandPlan::Transaction(replace_transaction) = replace
        .plan_command(
            103,
            TypedCommand::ReplaceSelectionText { text: "new".into() },
        )
        .unwrap()
    else {
        panic!("range replacement must plan")
    };
    assert!(matches!(
        replace_transaction.operations.as_slice(),
        [
            TypedOperation::DeleteRange { .. },
            TypedOperation::InsertText { .. }
        ]
    ));
    let compiled = replace
        .compile_typed_transaction(replace_transaction.clone())
        .unwrap();
    assert_eq!(
        compiled.history_class,
        crate::yrs_engine::compiler::HistoryClass::Structural
    );
    assert_eq!(
        compiled.position_update_mode,
        crate::position::update::UpdateMode::InlineTextOnly
    );
    assert_eq!(compiled.affected_top_level_blocks, vec![0]);
    let replace_result = replace
        .apply_typed_transaction_with_result(replace_transaction)
        .unwrap();
    let crate::yrs_engine::RenderUpdate::Patch(replace_patch) = replace_result.render_update else {
        panic!("range replacement must produce a local render patch")
    };
    assert_eq!(
        (
            replace_patch.start_index,
            replace_patch.delete_count,
            replace_patch.blocks.len(),
        ),
        (0, 1, 1)
    );
    assert_unaffected_sticky(
        &replace,
        &replace_sticky,
        replace_branch,
        replace_scalar - 3,
    );
    assert!(replace.can_undo());
}

#[test]
fn typed_edits_advance_cached_render_blocks_while_selection_only_retains_arc() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let initial = Arc::clone(&engine.derived_state.as_ref().unwrap().render_blocks);
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 104,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(Arc::ptr_eq(
        &initial,
        &engine.derived_state.as_ref().unwrap().render_blocks
    ));

    let old_blocks = initial.materialize();
    crate::render::incremental::reset_cached_render_counts_for_test();
    let result = engine
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 105,
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
        })
        .unwrap();
    assert_eq!(
        crate::render::incremental::take_cached_render_counts_for_test(),
        (0, 1, 1, 0, 0)
    );
    let next = engine.derived_state.as_ref().unwrap();
    assert!(!Arc::ptr_eq(&initial, &next.render_blocks));

    let reconstructed = match result.render_update {
        crate::yrs_engine::RenderUpdate::None => old_blocks,
        crate::yrs_engine::RenderUpdate::Full(blocks) => blocks,
        crate::yrs_engine::RenderUpdate::Patch(patch) => {
            let mut blocks = old_blocks;
            blocks.splice(
                patch.start_index..patch.start_index + patch.delete_count,
                patch.blocks,
            );
            blocks
        }
    };
    assert_eq!(reconstructed, next.render_blocks.materialize());
    assert_eq!(
        next.render_blocks.materialize(),
        crate::render::incremental::render_blocks(&next.document, &engine.schema)
    );
}

#[test]
fn history_results_compare_sealed_render_caches_without_full_old_new_render() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_edit_cache = Arc::clone(
        &engine
            .derived_state
            .as_ref()
            .expect("import initializes derived state")
            .render_blocks,
    );
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 106,
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
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    let after_edit_cache = Arc::clone(
        &engine
            .derived_state
            .as_ref()
            .expect("edit initializes derived state")
            .render_blocks,
    );

    let before_undo = engine
        .derived_state
        .as_ref()
        .unwrap()
        .render_blocks
        .materialize();
    crate::render::incremental::reset_cached_render_counts_for_test();
    let undo = engine.undo_with_result(107).unwrap().unwrap();
    assert_eq!(
        crate::render::incremental::take_cached_render_counts_for_test(),
        (0, 0, 0, 0, 0)
    );
    let after_undo = engine.derived_state.as_ref().unwrap();
    assert!(Arc::ptr_eq(&before_edit_cache, &after_undo.render_blocks));
    let reconstructed = apply_render_update_for_test(before_undo, undo.render_update);
    assert_eq!(reconstructed, after_undo.render_blocks.materialize());

    let before_redo = after_undo.render_blocks.materialize();
    crate::render::incremental::reset_cached_render_counts_for_test();
    let redo = engine.redo_with_result(108).unwrap().unwrap();
    assert_eq!(
        crate::render::incremental::take_cached_render_counts_for_test(),
        (0, 0, 0, 0, 0)
    );
    let after_redo = engine.derived_state.as_ref().unwrap();
    assert!(Arc::ptr_eq(&after_edit_cache, &after_redo.render_blocks));
    assert_eq!(
        apply_render_update_for_test(before_redo, redo.render_update),
        after_redo.render_blocks.materialize()
    );
}

#[test]
fn history_snapshot_seed_publication_errors_propagate_real_request_atomically() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (request_id, failpoint, expected_stage) in [
        (
            108_056,
            LookupSeedHydrationFailpoint::BindingPublication,
            "historyStoreSnapshotPublication",
        ),
        (
            108_057,
            LookupSeedHydrationFailpoint::SeedPublication,
            "historyUnavailableSeedPublication",
        ),
    ] {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: request_id - 1,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
        assert!(engine.can_undo());
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);

        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let result = engine.undo_with_result(request_id);
        set_lookup_seed_hydration_failpoint_for_test(None);

        let error = result.expect_err("history snapshot publication failure must propagate");
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(error.request_id, request_id);
        assert_eq!(
            error.message.as_ref(),
            format!("mutation lookup seed allocation failed during {expected_stage}")
        );
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" }))
        );
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }
}

#[test]
fn history_snapshot_equality_uses_document_snapshot_arc_identity() {
    let engine = transaction_engine();
    let state = engine.derived_state.as_ref().unwrap();
    let retained = crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
        crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
            document: &state.document,
            canonical_artifact: &state.canonical_artifact,
            position_map: &state.position_map,
            rendered_text: &state.rendered_text,
            render_blocks: &state.render_blocks,
            schema_fingerprint: &engine.schema_fingerprint,
            fragment_name: &engine.fragment_name,
            scope: engine.scope.as_ref(),
        },
    )
    .unwrap();
    let document_snapshot = state.capture_history_document_snapshot(
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.fragment_name,
        engine.scope.as_ref(),
        retained,
    );
    let snapshot = crate::yrs_engine::history::HistorySnapshot {
        relative_selection: state.relative_selection.clone(),
        resolved_selection: state.resolved_selection.clone(),
        stored_marks: state.stored_marks.clone(),
        text_length: state.canonical_artifact.text_scalar_len(),
        canonical_fingerprint: state.canonical_artifact.sha256(),
        derived_output_bytes: state.canonical_artifact.serialized_len(),
        metadata_bytes: retained.get(),
        document_snapshot: Some(document_snapshot),
    };
    let shared = snapshot.clone();
    assert_eq!(snapshot, shared);

    let mut equivalent_but_distinct = snapshot.clone();
    let document_snapshot = snapshot
        .document_snapshot
        .as_ref()
        .expect("default article history retains its document snapshot");
    equivalent_but_distinct.document_snapshot = Some(Arc::new((**document_snapshot).clone()));
    assert_ne!(snapshot, equivalent_but_distinct);
}

#[test]
fn history_restoration_resolves_only_the_popped_selection_without_a_default_roundtrip() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_json = engine.document_json().unwrap();
    let before_selection = engine.resolved_selection().cloned().unwrap();
    let before_marks = engine.stored_marks().map(<[_]>::to_vec);
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 108_001,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 2,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();
    let after_json = engine.document_json().unwrap();
    let after_selection = engine.resolved_selection().cloned().unwrap();
    let after_marks = engine.stored_marks().map(<[_]>::to_vec);

    for (request_id, undoing) in [(108_002, true), (108_003, false)] {
        crate::yrs_engine::derived_state::reset_relative_selection_traversal_counts_for_test();
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();

        if undoing {
            engine.undo_with_result(request_id).unwrap().unwrap();
        } else {
            engine.redo_with_result(request_id).unwrap().unwrap();
        }

        let (expected_json, expected_selection, expected_marks) = if undoing {
            (&before_json, &before_selection, &before_marks)
        } else {
            (&after_json, &after_selection, &after_marks)
        };
        assert_eq!(engine.document_json().as_ref(), Some(expected_json));
        assert_eq!(engine.resolved_selection(), Some(expected_selection));
        assert_eq!(
            engine.stored_marks().map(<[_]>::to_vec).as_ref(),
            expected_marks.as_ref()
        );

        assert_eq!(
            crate::yrs_engine::derived_state::take_relative_selection_traversal_counts_for_test(),
            (1, 1),
            "history restoration should materialize only the exact popped selection"
        );
        let full_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        // The document-scoped history snapshot is admitted by exact
        // candidate JSON equality, so no canonical projection,
        // serialization, or hash pass is repeated during the pop.
        assert_eq!(full_passes.canonical_projections, 0);
        assert_eq!(full_passes.canonical_serializations, 0);
        assert_eq!(full_passes.canonical_hashes, 0);
    }
}

#[test]
fn tight_history_metadata_budget_falls_back_to_full_candidate_derivation() {
    let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
        max_derived_output_bytes: 2 * (512 + "prosemirror".len() + 2),
        ..crate::yrs_engine::EditingLimits::default()
    });
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 108_004,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();

    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    engine.undo_with_result(108_005).unwrap().unwrap();

    assert_eq!(
        engine.document_json().unwrap(),
        serde_json::from_str::<serde_json::Value>(
            r#"{"type":"doc","content":[{"type":"paragraph"}]}"#,
        )
        .unwrap()
    );
    let full_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
    assert!(full_passes.canonical_projections > 0);
    assert!(full_passes.canonical_serializations > 0);
    assert!(full_passes.canonical_hashes > 0);
}

#[test]
fn deep_wide_history_snapshot_budget_accounts_for_spilled_position_paths() {
    fn deep_wide_document() -> serde_json::Value {
        let mut content = (0..24)
            .map(|index| {
                json!({
                    "type": "paragraph",
                    "content": [{"type": "text", "text": format!("row {index}")}]
                })
            })
            .collect::<Vec<_>>();
        for _ in 0..10 {
            content = vec![json!({"type": "blockquote", "content": content})];
        }
        json!({"type": "doc", "content": content})
    }

    fn insert(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
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
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        }
    }

    let document = deep_wide_document();
    let mut probe = transaction_engine();
    probe
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let compiled = probe
        .compile_typed_transaction(insert(&probe, 108_006))
        .unwrap();
    let after = compiled.preview_derivations.as_ref().unwrap();
    let before = probe.derived_state.as_ref().unwrap();
    let before_retained =
        crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &before.document,
                canonical_artifact: &before.canonical_artifact,
                position_map: &before.position_map,
                rendered_text: &before.rendered_text,
                render_blocks: &before.render_blocks,
                schema_fingerprint: &probe.schema_fingerprint,
                fragment_name: &probe.fragment_name,
                scope: probe.scope.as_ref(),
            },
        )
        .unwrap();
    let after_retained =
        crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &compiled.preview,
                canonical_artifact: compiled.canonical_artifact.as_ref().unwrap(),
                position_map: &after.position_map,
                rendered_text: &after.rendered_text,
                render_blocks: &crate::render::incremental::CachedRenderBlocks::build(
                    &compiled.preview,
                    &probe.schema,
                    &probe.resource_limits,
                )
                .unwrap(),
                schema_fingerprint: &probe.schema_fingerprint,
                fragment_name: &probe.fragment_name,
                scope: probe.scope.as_ref(),
            },
        )
        .unwrap();
    let exact_budget =
        super::history_metadata_bytes(before.stored_marks.as_deref(), &probe.fragment_name)
            .checked_add(super::history_metadata_bytes(None, &probe.fragment_name))
            .and_then(|bytes| bytes.checked_add(before_retained.get()))
            .and_then(|bytes| bytes.checked_add(after_retained.get()))
            .unwrap();

    let run = |limit, request_id| {
        let mut engine = transaction_engine();
        engine
            .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        engine.editing_limits.max_derived_output_bytes = limit;
        engine
            .apply_typed_transaction(insert(&engine, request_id))
            .unwrap();
        assert!(
            engine.can_undo(),
            "base history capture must remain admitted"
        );
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        engine.undo_with_result(request_id + 1).unwrap().unwrap();
        crate::yrs_engine::observability::take_full_pass_counts_for_test()
    };

    let exact_passes = run(exact_budget, 108_007);
    assert_eq!(
        exact_passes.canonical_projections, 0,
        "the exact retained bound should admit the optional snapshots"
    );

    let full_passes = run(exact_budget - 1, 108_009);
    assert!(
        full_passes.canonical_projections > 0,
        "one under the retained bound must omit only the optional snapshots"
    );
}

#[test]
fn history_snapshot_charge_tracks_spare_node_string_capacity() {
    const SPARE_CAPACITY: usize = 1024 * 1024;

    fn fixture(limit: usize) -> YrsDocumentEngine {
        let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        });
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        let mut node_type = String::with_capacity(SPARE_CAPACITY);
        node_type.push_str("hardBreak");
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                node: crate::model::Node::void(node_type, HashMap::new()),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        }
    }

    fn snapshot_charge(engine: &YrsDocumentEngine) -> usize {
        let state = engine.derived_state.as_ref().unwrap();
        crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &state.document,
                canonical_artifact: &state.canonical_artifact,
                position_map: &state.position_map,
                rendered_text: &state.rendered_text,
                render_blocks: &state.render_blocks,
                schema_fingerprint: &engine.schema_fingerprint,
                fragment_name: &engine.fragment_name,
                scope: engine.scope.as_ref(),
            },
        )
        .unwrap()
        .get()
    }

    let before_probe =
        fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
    let before_charge = snapshot_charge(&before_probe);
    let before_metadata =
        super::history_metadata_bytes(before_probe.stored_marks(), &before_probe.fragment_name);
    let mut after_probe =
        fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
    after_probe
        .apply_typed_transaction(transaction(&after_probe, 108_020))
        .unwrap();
    let after_charge = snapshot_charge(&after_probe);
    assert!(after_charge >= SPARE_CAPACITY);
    let exact = before_metadata
        .checked_add(super::history_metadata_bytes(
            after_probe.stored_marks(),
            &after_probe.fragment_name,
        ))
        .and_then(|bytes| bytes.checked_add(before_charge))
        .and_then(|bytes| bytes.checked_add(after_charge))
        .unwrap();

    for (limit, expect_fast, request_id) in [(exact, true, 108_021), (exact - 1, false, 108_023)] {
        let mut engine = fixture(limit);
        engine
            .apply_typed_transaction(transaction(&engine, request_id))
            .unwrap();
        assert!(engine.can_undo());
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        engine.undo_with_result(request_id + 1).unwrap().unwrap();
        let passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        assert_eq!(passes.canonical_projections == 0, expect_fast);
    }
}

#[test]
fn stored_mark_metadata_accounts_spare_hash_capacity_at_exact_boundary() {
    const SPARE_ENTRIES: usize = 32 * 1024;

    fn fixture(limit: usize) -> YrsDocumentEngine {
        let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        });
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut engine, 108_030, 1, 1);
        let mut attrs = HashMap::with_capacity(SPARE_ENTRIES);
        attrs.insert("href".into(), json!("x"));
        engine
            .apply_command(
                108_031,
                TypedCommand::SetMark {
                    mark_type: "link".into(),
                    attrs,
                },
            )
            .unwrap()
            .unwrap();
        assert!(engine.stored_marks().is_some());
        engine
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
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
                text: "x".into(),
                marks: engine.stored_marks().unwrap().to_vec(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        }
    }

    let mut probe = fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
    let before_metadata = super::history_metadata_bytes(probe.stored_marks(), &probe.fragment_name);
    probe
        .apply_typed_transaction(transaction(&probe, 108_032))
        .unwrap();
    let exact = before_metadata
        .checked_add(super::history_metadata_bytes(
            probe.stored_marks(),
            &probe.fragment_name,
        ))
        .unwrap();

    let mut accepted = fixture(exact);
    accepted
        .apply_typed_transaction(transaction(&accepted, 108_033))
        .unwrap();
    assert!(accepted.can_undo());

    let mut rejected = fixture(exact - 1);
    let before = atomic_audit(&rejected);
    let error = rejected
        .apply_typed_transaction(transaction(&rejected, 108_034))
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(atomic_audit(&rejected), before);
}

#[test]
fn compatible_auto_capture_admits_exact_after_only_metadata_increment() {
    fn fixture(limit: usize) -> YrsDocumentEngine {
        let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        });
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn insert(engine: &YrsDocumentEngine, request_id: u64, offset: u32) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    let default_limit = crate::yrs_engine::EditingLimits::default().max_derived_output_bytes;
    let mut probe = fixture(default_limit);
    probe
        .apply_typed_transaction(insert(&probe, 108_040, 1))
        .unwrap();
    let retained_before_second = probe.history.replay_metadata_bytes_for_test();
    let second_before_metadata = {
        let state = probe.derived_state.as_ref().unwrap();
        let retained = crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &state.document,
                canonical_artifact: &state.canonical_artifact,
                position_map: &state.position_map,
                rendered_text: &state.rendered_text,
                render_blocks: &state.render_blocks,
                schema_fingerprint: &probe.schema_fingerprint,
                fragment_name: &probe.fragment_name,
                scope: probe.scope.as_ref(),
            },
        )
        .unwrap()
        .get();
        super::history_metadata_bytes(probe.stored_marks(), &probe.fragment_name)
            .checked_add(retained)
            .unwrap()
    };
    probe
        .apply_typed_transaction(insert(&probe, 108_041, 2))
        .unwrap();
    let second_after_metadata = probe
        .history
        .replay_metadata_bytes_for_test()
        .checked_sub(retained_before_second)
        .unwrap();
    let exact = retained_before_second
        .checked_add(second_after_metadata)
        .unwrap();
    assert!(
        exact
            < retained_before_second
                .checked_add(second_before_metadata)
                .and_then(|bytes| bytes.checked_add(second_after_metadata))
                .unwrap()
    );

    let mut engine = fixture(exact);
    engine
        .apply_typed_transaction(insert(&engine, 108_042, 1))
        .unwrap();
    engine
        .apply_typed_transaction(insert(&engine, 108_043, 2))
        .unwrap();
    assert_eq!(engine.document().unwrap().root().text_content(), "axxb");
    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    engine.undo_with_result(108_044).unwrap().unwrap();
    assert_eq!(engine.document().unwrap().root().text_content(), "ab");
    assert!(!engine.can_undo(), "compatible edits must remain one group");
    assert_eq!(
        crate::yrs_engine::observability::take_full_pass_counts_for_test().canonical_projections,
        0,
        "the exact boundary keeps optional document snapshots enabled"
    );
}

#[test]
fn history_snapshot_and_forced_fallback_match_affinity_and_stored_marks() {
    use crate::yrs_engine::derived_state::force_history_document_snapshot_fallback_for_test;

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                &json!({
                    "type": "doc",
                    "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "a😀b"}]},
                        {"type": "horizontalRule"},
                        {"type": "paragraph", "content": [{"type": "text", "text": "tail"}]}
                    ]
                })
                .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let boundary = |affinity| RevisionedPosition {
            offset: 2,
            kind: EditorOffsetKind::Scalar,
            affinity,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_052,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: boundary(Affinity::Before),
                    head: boundary(Affinity::After),
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(
                108_051,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert!(engine.stored_marks().is_some());
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_053,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: engine.stored_marks().unwrap().to_vec(),
                }],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
        engine
    }

    fn local_state(
        engine: &YrsDocumentEngine,
    ) -> (
        serde_json::Value,
        Option<ResolvedSelection>,
        Option<Vec<crate::model::Mark>>,
        bool,
        bool,
    ) {
        (
            engine.document_json().unwrap(),
            engine.resolved_selection().cloned(),
            engine.stored_marks().map(<[_]>::to_vec),
            engine.can_undo(),
            engine.can_redo(),
        )
    }

    fn text_affinities(engine: &YrsDocumentEngine) -> (Affinity, Affinity) {
        let Some(crate::yrs_engine::RelativeSelection::Text { anchor, head }) =
            engine.relative_selection()
        else {
            panic!("history restores the captured text selection");
        };
        (anchor.affinity, head.affinity)
    }

    let mut fast = fixture();
    let mut fallback = fixture();

    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    fast.undo_with_result(108_054).unwrap().unwrap();
    let fast_undo_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
    let fallback_undo_passes = {
        let _fallback = force_history_document_snapshot_fallback_for_test();
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        fallback.undo_with_result(108_054).unwrap().unwrap();
        crate::yrs_engine::observability::take_full_pass_counts_for_test()
    };
    assert_eq!(local_state(&fast), local_state(&fallback));
    assert_eq!(text_affinities(&fast), text_affinities(&fallback));
    assert_eq!(text_affinities(&fast), (Affinity::Before, Affinity::After));
    assert_eq!(fast_undo_passes.canonical_projections, 0);
    assert!(fallback_undo_passes.canonical_projections > 0);

    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    fast.redo_with_result(108_055).unwrap().unwrap();
    let fast_redo_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
    let fallback_redo_passes = {
        let _fallback = force_history_document_snapshot_fallback_for_test();
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        fallback.redo_with_result(108_055).unwrap().unwrap();
        crate::yrs_engine::observability::take_full_pass_counts_for_test()
    };
    assert_eq!(local_state(&fast), local_state(&fallback));
    assert_eq!(text_affinities(&fast), text_affinities(&fallback));
    assert_eq!(text_affinities(&fast), (Affinity::Before, Affinity::After));
    assert_eq!(fast_redo_passes.canonical_projections, 0);
    assert!(fallback_redo_passes.canonical_projections > 0);
}

#[test]
fn history_snapshot_context_drift_falls_back_without_changing_undo_result() {
    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_060,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
        engine
    }

    for context in ["resource", "editing", "maxLength", "scope"] {
        let mut engine = fixture();
        match context {
            "resource" => {
                engine.resource_limits.max_document_depth =
                    engine.resource_limits.max_document_depth.saturating_add(1)
            }
            "editing" => {
                engine.editing_limits.max_operations_per_transaction = engine
                    .editing_limits
                    .max_operations_per_transaction
                    .saturating_add(1)
            }
            "maxLength" => engine.max_length = Some(100),
            "scope" => engine
                .scope
                .as_mut()
                .expect("fixture is document scoped")
                .lineage_id
                .push_str("-changed"),
            _ => unreachable!(),
        }

        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        engine.undo_with_result(108_061).unwrap().unwrap();
        let passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        assert_eq!(engine.document().unwrap().root().text_content(), "ab");
        assert!(
            passes.canonical_projections > 0,
            "{context} drift must reject snapshot reuse and run the fallback"
        );
    }
}

#[test]
fn invalid_history_stored_marks_precede_snapshot_publication_and_preserve_atomicity() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 108_070,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 2,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();
    engine
        .history
        .replace_next_undo_stored_marks_for_test(vec![Mark::new("unknown".into(), HashMap::new())]);
    let before = atomic_audit(&engine);

    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::BindingPublication,
    ));
    let result = engine.undo_with_result(108_071);
    set_lookup_seed_hydration_failpoint_for_test(None);

    let error = result.expect_err("invalid history metadata must precede snapshot publication");
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.request_id, 108_071);
    assert_eq!(
        error.message.as_ref(),
        "history metadata contains invalid stored marks: unknown mark 'unknown'"
    );
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn every_history_snapshot_semantic_fallback_precedes_seed_publication() {
    use crate::yrs_engine::derived_state::{
        force_history_document_snapshot_fallback_for_test,
        force_history_snapshot_semantic_fallback_for_test, HistorySnapshotSemanticFallbackForTest,
    };
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_072,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
        engine
    }

    for stage in [
        HistorySnapshotSemanticFallbackForTest::RenderIdentity,
        HistorySnapshotSemanticFallbackForTest::RelativeSelection,
        HistorySnapshotSemanticFallbackForTest::ResolvedSelection,
        HistorySnapshotSemanticFallbackForTest::ResolvedMismatch,
    ] {
        for failpoint in [
            LookupSeedHydrationFailpoint::BindingPublication,
            LookupSeedHydrationFailpoint::SeedPublication,
        ] {
            let mut expected = fixture();
            let expected_result = {
                let _fallback = force_history_document_snapshot_fallback_for_test();
                expected.undo_with_result(108_073).unwrap().unwrap()
            };
            let mut actual = fixture();
            let actual_result = {
                let _fallback = force_history_snapshot_semantic_fallback_for_test(stage);
                set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
                let result = actual.undo_with_result(108_073);
                set_lookup_seed_hydration_failpoint_for_test(None);
                result.unwrap().unwrap()
            };

            assert_eq!(actual_result, expected_result, "{stage:?}/{failpoint:?}");
            assert_eq!(
                actual.document_json(),
                expected.document_json(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.resolved_selection(),
                expected.resolved_selection(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.stored_marks(),
                expected.stored_marks(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.can_undo(),
                expected.can_undo(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.can_redo(),
                expected.can_redo(),
                "{stage:?}/{failpoint:?}"
            );
        }
    }
}

#[test]
fn history_restore_request_relabeling_precedes_forced_semantic_fallback_and_probes() {
    use crate::yrs_engine::derived_state::{
        force_history_snapshot_semantic_fallback_for_test,
        history_document_snapshot_retained_bytes, DerivedStateCache,
        HistoryDocumentSnapshotRetainedInput, HistorySnapshotSemanticFallbackForTest,
    };
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    let engine = transaction_engine();
    let state = engine.derived_state.as_ref().unwrap();
    let retained = history_document_snapshot_retained_bytes(HistoryDocumentSnapshotRetainedInput {
        document: &state.document,
        canonical_artifact: &state.canonical_artifact,
        position_map: &state.position_map,
        rendered_text: &state.rendered_text,
        render_blocks: &state.render_blocks,
        schema_fingerprint: &state.schema_fingerprint,
        fragment_name: &engine.fragment_name,
        scope: engine.scope.as_ref(),
    })
    .unwrap();
    let snapshot = state.capture_history_document_snapshot(
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.fragment_name,
        engine.scope.as_ref(),
        retained,
    );
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();

    for failpoint in [
        LookupSeedHydrationFailpoint::BindingPublication,
        LookupSeedHydrationFailpoint::SeedPublication,
    ] {
        let (_, admission) = snapshot
            .prepare_candidate_read(
                108_074,
                &txn,
                &fragment,
                &engine.schema,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                &engine.fragment_name,
                engine.scope.as_ref(),
                engine.yrs_state_epoch,
                engine.revision,
            )
            .unwrap()
            .into_parts();
        let _fallback = force_history_snapshot_semantic_fallback_for_test(
            HistorySnapshotSemanticFallbackForTest::RenderIdentity,
        );
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let result = DerivedStateCache::restore_history_document_snapshot(
            108_075,
            &snapshot,
            admission.expect("matching read admits the retained snapshot"),
            &txn,
            &fragment,
            &engine.schema,
            &state.relative_selection,
            &state.resolved_selection,
            state.stored_marks.clone(),
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            engine.revision,
            engine.state_revision,
            engine.yrs_state_epoch,
        );
        set_lookup_seed_hydration_failpoint_for_test(None);

        let error = result.expect_err("request relabeling must precede semantic fallback");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(error.request_id, 108_075, "{failpoint:?}");
    }
}

#[test]
fn history_specific_initialization_keeps_candidate_limit_rejection_atomic() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.max_length = Some(2);
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 108_004,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::DeleteRange {
                range: RevisionedRange {
                    from: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    to: RevisionedPosition {
                        offset: 3,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                },
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();
    let before = atomic_audit(&engine);

    let error = engine.undo_with_result(108_005).unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(2));
    assert_eq!(error.actual, Some(3));
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn second_history_pop_max_length_drift_rejects_before_live_pop() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.max_length = Some(1);
    for (request_id, from, to) in [(108_006, 1, 2), (108_007, 0, 1)] {
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::DeleteRange {
                    range: RevisionedRange {
                        from: RevisionedPosition {
                            offset: from,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        to: RevisionedPosition {
                            offset: to,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                    },
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
    }

    engine
        .undo(108_008)
        .unwrap()
        .expect("first pop must restore the one-character document");
    assert_eq!(engine.document().unwrap().root().text_content(), "a");
    let before = atomic_audit(&engine);

    let error = engine.undo(108_009).unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(1));
    assert_eq!(error.actual, Some(2));
    assert_eq!(error.details, Some(json!({ "field": "maxLength" })));
    assert_eq!(atomic_audit(&engine), before);
    let repeated = engine.undo(108_010).unwrap_err();
    assert_eq!(repeated.code, error.code);
    assert_eq!(repeated.limit, error.limit);
    assert_eq!(repeated.actual, error.actual);
    assert_eq!(repeated.details, error.details);
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn cached_render_preparation_failure_is_atomic_before_durable_write() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before = atomic_audit(&engine);
    crate::render::incremental::set_cached_render_error_for_test(Some(
        crate::render::incremental::CachedRenderError::AllocationFailed,
    ));
    let error = engine
        .apply_typed_transaction(insert_transaction(&engine, 109))
        .unwrap_err();
    crate::render::incremental::set_cached_render_error_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

fn apply_render_update_for_test(
    mut old_blocks: Vec<Vec<crate::render::RenderElement>>,
    update: crate::yrs_engine::RenderUpdate,
) -> Vec<Vec<crate::render::RenderElement>> {
    match update {
        crate::yrs_engine::RenderUpdate::None => old_blocks,
        crate::yrs_engine::RenderUpdate::Full(blocks) => blocks,
        crate::yrs_engine::RenderUpdate::Patch(patch) => {
            old_blocks.splice(
                patch.start_index..patch.start_index + patch.delete_count,
                patch.blocks,
            );
            old_blocks
        }
    }
}

#[test]
fn direct_command_admission_error_is_not_replanned_as_structure() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"target"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_text(&mut engine, 104, 6, 0);
    engine.resource_limits.max_input_bytes = 0;
    let before = atomic_audit(&engine);

    let error = engine
        .plan_command(105, TypedCommand::ReplaceSelectionText { text: "x".into() })
        .unwrap_err();

    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.details, Some(json!({ "field": "maxInputBytes" })));
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn every_recoverable_atomic_stage_failpoint_is_pre_open_and_read_only() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};

    let failpoints = [
        AtomicFailpoint::EnvelopeAdmission,
        AtomicFailpoint::SemanticCompilation,
        AtomicFailpoint::MutationPreflight,
        AtomicFailpoint::FinalPreflight,
        AtomicFailpoint::EncodedAdmission,
        AtomicFailpoint::CanonicalOutputAdmission,
        AtomicFailpoint::RevisionAdmission,
        AtomicFailpoint::DurableMetadataAdmission,
    ];
    for failpoint in failpoints {
        let mut engine = transaction_engine();
        let transaction = insert_transaction(&engine, 76);
        let before = atomic_audit(&engine);
        let canonical_before = engine
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .clone();
        set_atomic_failpoint_for_test(Some(failpoint));

        let error = engine.apply_typed_transaction(transaction).unwrap_err();

        set_atomic_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(
            error.details,
            Some(json!({ "failpoint": failpoint.field_name() })),
            "{failpoint:?}"
        );
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        assert!(canonical_before.ptr_eq(&engine.derived_state.as_ref().unwrap().canonical_artifact));
    }
}

#[test]
fn compiled_history_failure_does_not_publish_candidate_active_state_lifecycle() {
    use crate::yrs_engine::derived_state::{
        reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
    };

    for pending_install in [true, false] {
        let request_id = if pending_install { 760_010 } else { 760_020 };
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(
                request_id + 1,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap()
            .unwrap();
        let live_certificate = engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .expect("fixture must retain a live active-state certificate");
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id + 2,
                TypedCommand::InsertText { text: "y".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("insert command must prepare a transaction")
        };
        let mut compiled = engine
            .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
            .unwrap();
        assert!(compiled.prepared_active_state_transition.is_some());
        if !pending_install {
            compiled.prepared_active_state_transition = None;
        }
        let before = atomic_audit(&engine);
        reset_active_state_cache_counts_for_test();
        set_compiled_commit_stage_failpoint_for_test(Some(
            CompiledCommitPreparationStage::HistorySnapshotConstruction,
        ));

        let error = engine
            .apply_compiled_transaction(compiled, true)
            .expect_err("late snapshot construction must reject the prepared candidate");

        set_compiled_commit_stage_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{pending_install}");
        assert!(
            error.message.contains("historySnapshotConstruction"),
            "{pending_install}"
        );
        let counts = take_active_state_cache_counts_for_test();
        assert_eq!(counts.5, 0, "pending install={pending_install}");
        assert_eq!(counts.6, 0, "pending install={pending_install}");
        assert_eq!(atomic_audit(&engine), before, "{pending_install}");
        assert!(Arc::ptr_eq(
            &live_certificate,
            &engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .unwrap(),
        ));
    }
}

#[test]
fn compiled_recorded_history_admission_preserves_live_replay_allocation_on_later_failure() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.history.compact_replay_event_capacity_for_test();
    let before = atomic_audit(&engine);
    let ledger_before = engine.history.replay_ledger_allocation_audit_for_test();
    let mut transaction = insert_transaction(&engine, 760_030);
    transaction.history_policy = HistoryPolicy::Auto;
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));

    let error = engine
        .apply_typed_transaction(transaction)
        .expect_err("candidate update encoding must fail after recorded admission");

    set_compiled_commit_stage_failpoint_for_test(None);
    assert!(error.message.contains("historyUpdateEncoding"));
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        engine.history.replay_ledger_allocation_audit_for_test(),
        ledger_before
    );
}

#[test]
fn compiled_excluded_history_admission_preserves_live_replay_allocation_on_later_failure() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.history.compact_replay_event_capacity_for_test();
    let before = atomic_audit(&engine);
    let ledger_before = engine.history.replay_ledger_allocation_audit_for_test();
    let transaction = insert_transaction(&engine, 760_040);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));

    let error = engine
        .apply_typed_transaction(transaction)
        .expect_err("candidate update encoding must fail after excluded admission");

    set_compiled_commit_stage_failpoint_for_test(None);
    assert!(error.message.contains("historyUpdateEncoding"));
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        engine.history.replay_ledger_allocation_audit_for_test(),
        ledger_before
    );
}

#[test]
fn compiled_history_admission_error_precedes_candidate_preparation_failure() {
    use crate::yrs_engine::history::set_replay_update_allocation_failure_for_test;

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut transaction = insert_transaction(&engine, 760_020);
    transaction.history_policy = HistoryPolicy::Auto;
    let compiled = engine.compile_typed_transaction(transaction).unwrap();
    let before = atomic_audit(&engine);
    set_replay_update_allocation_failure_for_test(true);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));

    let error = engine
        .apply_compiled_transaction(compiled, true)
        .expect_err("history admission must win error precedence");

    set_replay_update_allocation_failure_for_test(false);
    set_compiled_commit_stage_failpoint_for_test(None);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(error.details, Some(json!({ "field": "historyReplay" })));
    assert_eq!(atomic_audit(&engine), before);

    let mut lookup_first = transaction_engine();
    lookup_first
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    lookup_first.ensure_mutation_lookup_seed(760_021).unwrap();
    let mut transaction = insert_transaction(&lookup_first, 760_022);
    transaction.history_policy = HistoryPolicy::Auto;
    let compiled = lookup_first.compile_typed_transaction(transaction).unwrap();
    let before = atomic_audit(&lookup_first);
    set_replay_update_allocation_failure_for_test(true);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::LookupTransition,
    ));

    let error = lookup_first
        .apply_compiled_transaction(compiled, true)
        .expect_err("baseline lookup failure must retain precedence over history admission");

    set_replay_update_allocation_failure_for_test(false);
    set_compiled_commit_stage_failpoint_for_test(None);
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert!(error.message.contains("lookupTransition"));
    assert_eq!(atomic_audit(&lookup_first), before);
}

#[test]
fn compiled_first_structural_mutation_supports_an_empty_configured_root() {
    let schema = crate::schema::Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema,
        fragment_name: "empty-root".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "empty-root-doc".into(),
            lineage_id: "empty-root-lineage".into(),
        }),
    })
    .unwrap();
    let initial_json = engine.document_json().unwrap();
    let initial_encoded = engine.encoded_state().unwrap();
    let initial_revision = engine.revision();
    let initial_state_revision = engine.state_revision();
    let initial_selection = engine.resolved_selection().cloned();
    let initial_history = engine.history.replay_audit_for_test();
    let result = engine
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 760_030,
            base_document_revision: initial_revision,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset: 0,
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
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();

    let changed_json = engine.document_json().unwrap();
    assert_ne!(engine.encoded_state().unwrap(), initial_encoded);
    assert_eq!(changed_json["type"], "doc");
    assert_eq!(changed_json["content"][0]["type"], "paragraph");
    assert_eq!(engine.revision(), initial_revision + 1);
    assert_eq!(engine.state_revision(), initial_state_revision + 1);
    assert_eq!(result.document_revision, engine.revision());
    assert_eq!(result.state_revision, engine.state_revision());
    assert_eq!(engine.resolved_selection(), Some(&result.selection));
    assert_eq!(result.history_state.can_undo, engine.can_undo());
    assert_eq!(result.history_state.can_redo, engine.can_redo());
    assert!(engine.can_undo());
    assert_ne!(engine.history.replay_audit_for_test(), initial_history);

    let undo = engine
        .undo(760_031)
        .unwrap()
        .expect("insert must be undoable");
    assert!(undo.changed);
    assert_eq!(engine.document_json().unwrap(), initial_json);
    assert_eq!(engine.resolved_selection(), initial_selection.as_ref());
    assert!(!engine.can_undo());
    assert!(engine.can_redo());
}

#[test]
fn compiled_excluded_rebase_rolls_baseline_and_appends_the_event() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_encoded = engine.encoded_state().unwrap();
    engine.history.force_rebase_before_next_event_for_test();
    let transaction = insert_transaction(&engine, 760_040);

    engine.apply_typed_transaction(transaction).unwrap();

    let (rebase, baseline, event_count, last_is_excluded) =
        engine.history.compiled_excluded_rebase_audit_for_test();
    assert!(!rebase);
    assert_eq!(baseline, before_encoded);
    assert_eq!(event_count, 1);
    assert!(last_is_excluded);
}

#[test]
fn compiled_commit_guard_rejects_every_preparation_stage_after_durable_open() {
    let stages = [
        CompiledCommitPreparationStage::AllocationProbe,
        CompiledCommitPreparationStage::OperationPreparation,
        CompiledCommitPreparationStage::DocumentValidation,
        CompiledCommitPreparationStage::LookupTransition,
        CompiledCommitPreparationStage::HistoryReservation,
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
        CompiledCommitPreparationStage::SelectionFinalization,
        CompiledCommitPreparationStage::DerivedStateBuild,
        CompiledCommitPreparationStage::HistorySnapshotConstruction,
    ];
    for stage in stages {
        set_compiled_commit_stage_failpoint_for_test(None);
        mark_compiled_commit_durable_write_for_test();
        let error = check_compiled_commit_preparation_stage_for_test(760_050, stage)
            .expect_err("every guarded preparation stage must reject after durable open");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{stage:?}");
        assert!(error.message.contains("postwrite"), "{stage:?}");
    }
    set_compiled_commit_stage_failpoint_for_test(None);
}

#[test]
fn compiled_commit_prepares_all_recoverable_work_before_durable_write() {
    let stages = [
        CompiledCommitPreparationStage::AllocationProbe,
        CompiledCommitPreparationStage::OperationPreparation,
        CompiledCommitPreparationStage::DocumentValidation,
        CompiledCommitPreparationStage::LookupTransition,
        CompiledCommitPreparationStage::HistoryReservation,
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
        CompiledCommitPreparationStage::SelectionFinalization,
        CompiledCommitPreparationStage::DerivedStateBuild,
        CompiledCommitPreparationStage::HistorySnapshotConstruction,
    ];
    for (index, stage) in stages.into_iter().enumerate() {
        let mut engine = transaction_engine();
        let request_id = 760_100 + index as u64;
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine.ensure_mutation_lookup_seed(request_id).unwrap();
        let mut transaction = insert_transaction(&engine, request_id);
        transaction.history_policy = HistoryPolicy::Auto;
        let before = atomic_audit(&engine);
        let seed_before = engine
            .derived_state
            .as_ref()
            .expect("ready fixture has derived state")
            .mutation_lookup_seed
            .clone();
        set_compiled_commit_stage_failpoint_for_test(Some(stage));

        let error = engine
            .apply_typed_transaction(transaction)
            .expect_err("every recoverable compiled-commit stage must be injectable");

        set_compiled_commit_stage_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{stage:?}");
        assert_eq!(atomic_audit(&engine), before, "{stage:?}");
        assert!(Arc::ptr_eq(
            &seed_before,
            &engine
                .derived_state
                .as_ref()
                .expect("failed commit retains derived state")
                .mutation_lookup_seed,
        ));
    }
}

#[test]
fn localized_seed_promotion_is_not_installed_before_any_recoverable_failpoint() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let failpoints = [
        AtomicFailpoint::EnvelopeAdmission,
        AtomicFailpoint::SemanticCompilation,
        AtomicFailpoint::MutationPreflight,
        AtomicFailpoint::FinalPreflight,
        AtomicFailpoint::EncodedAdmission,
        AtomicFailpoint::CanonicalOutputAdmission,
        AtomicFailpoint::RevisionAdmission,
        AtomicFailpoint::DurableMetadataAdmission,
    ];
    for failpoint in failpoints {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀b"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let transaction = insert_transaction(&engine, 76_001);
        let before = atomic_audit(&engine);
        reset_localized_lookup_counts_for_test();
        set_atomic_failpoint_for_test(Some(failpoint));

        let error = engine.apply_typed_transaction(transaction).unwrap_err();

        set_atomic_failpoint_for_test(None);
        let (_, _, promotions) = take_localized_lookup_counts_for_test();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(promotions, 0, "{failpoint:?}");
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
    }
}

#[test]
fn empty_skip_selection_bypasses_mutation_preflight_but_not_admission_or_boundaries() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let selection_transaction =
        |engine: &YrsDocumentEngine, request_id, history_policy| TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy,
        };

    let mut skip = transaction_engine();
    reset_prepared_admission_counts_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
    let result = skip
        .apply_typed_transaction_with_result(selection_transaction(&skip, 760, HistoryPolicy::Skip))
        .expect("empty Skip selection must not enter mutation preflight");
    set_atomic_failpoint_for_test(None);
    assert!(result.changed);
    assert_eq!(skip.revision(), 0);
    assert_eq!(skip.state_revision(), 1);
    let skip_counts = take_prepared_admission_counts_for_test();
    assert_eq!(skip_counts.staged_seed_preparations, 0);
    assert_eq!(skip_counts.installed_base_seed_publications, 0);

    let mut boundary = transaction_engine();
    let before_boundary = atomic_audit(&boundary);
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
    let boundary_error = boundary
        .apply_typed_transaction(selection_transaction(
            &boundary,
            761,
            HistoryPolicy::Boundary,
        ))
        .unwrap_err();
    set_atomic_failpoint_for_test(None);
    assert_eq!(
        boundary_error.details,
        Some(json!({ "failpoint": "mutationPreflight" }))
    );
    assert_eq!(atomic_audit(&boundary), before_boundary);

    let mut rejected = transaction_engine();
    let before_rejected = atomic_audit(&rejected);
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::EnvelopeAdmission));
    let admission_error = rejected
        .apply_typed_transaction(selection_transaction(&rejected, 762, HistoryPolicy::Skip))
        .unwrap_err();
    set_atomic_failpoint_for_test(None);
    assert_eq!(
        admission_error.details,
        Some(json!({ "failpoint": "envelopeAdmission" }))
    );
    assert_eq!(atomic_audit(&rejected), before_rejected);
}

#[test]
fn empty_generic_state_only_transactions_do_not_prepare_lookup_seed() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (offset, history_policy) in [
        HistoryPolicy::Skip,
        HistoryPolicy::Auto,
        HistoryPolicy::Boundary,
    ]
    .into_iter()
    .enumerate()
    {
        let request_id = 760_100 + u64::try_from(offset).unwrap();
        let mut engine = import_document_with_unavailable_lookup_seed();
        engine
            .apply_command(
                request_id,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .expect("collapsed toggle must set stored marks");
        assert_eq!(
            engine
                .stored_marks()
                .unwrap()
                .iter()
                .map(Mark::mark_type)
                .collect::<Vec<_>>(),
            vec!["bold"]
        );
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let before_document_revision = engine.revision();
        let before_state_revision = engine.state_revision();
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::InitialReservation,
        ));

        let result = engine
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id: request_id + 10,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy,
            })
            .expect("state-only generic transaction must not consume hydration failure");

        set_lookup_seed_hydration_failpoint_for_test(None);
        let counts = take_prepared_admission_counts_for_test();
        assert!(result.changed, "{history_policy:?}");
        assert_eq!(
            result.selection,
            ResolvedSelection::All,
            "{history_policy:?}"
        );
        assert_eq!(
            engine.revision(),
            before_document_revision,
            "{history_policy:?}"
        );
        assert_eq!(
            engine.state_revision(),
            before_state_revision + 1,
            "{history_policy:?}"
        );
        assert!(engine.stored_marks().is_none(), "{history_policy:?}");
        assert_eq!(counts.staged_seed_preparations, 0, "{history_policy:?}");
        assert_eq!(
            counts.installed_base_seed_publications, 0,
            "{history_policy:?}"
        );
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());
    }
}

#[test]
fn empty_generic_boundary_preserves_recorded_grouping_semantics() {
    let apply_insert = |engine: &mut YrsDocumentEngine, request_id, text: &str| {
        let at = engine.position_map().unwrap().total_scalars();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: at,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: text.into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
    };

    for (offset, state_only_policy) in [HistoryPolicy::Auto, HistoryPolicy::Boundary]
        .into_iter()
        .enumerate()
    {
        let request_id = 760_120 + u64::try_from(offset).unwrap() * 10;
        let mut engine = import_document_with_unavailable_lookup_seed();
        apply_insert(&mut engine, request_id, "x");
        force_lookup_seed_unavailable(&mut engine);
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: request_id + 1,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: state_only_policy,
            })
            .unwrap();
        apply_insert(&mut engine, request_id + 2, "y");
        assert_eq!(engine.document().unwrap().root().text_content(), "abcxy");

        engine
            .undo(request_id + 3)
            .unwrap()
            .expect("recorded insert must be undoable");
        let expected_after_first_pop = if state_only_policy == HistoryPolicy::Boundary {
            "abcx"
        } else {
            "abc"
        };
        assert_eq!(
            engine.document().unwrap().root().text_content(),
            expected_after_first_pop,
            "{state_only_policy:?}"
        );
        if state_only_policy == HistoryPolicy::Boundary {
            engine
                .undo(request_id + 4)
                .unwrap()
                .expect("Boundary must retain the earlier group");
            assert_eq!(engine.document().unwrap().root().text_content(), "abc");
        }
    }
}

#[test]
fn changed_state_boundary_revision_overflow_precedes_replay_allocation() {
    let mut engine = import_document_with_unavailable_lookup_seed();
    engine.history.compact_replay_event_capacity_for_test();
    engine.state_revision = u64::MAX;
    engine
        .derived_state
        .as_mut()
        .unwrap()
        .reseal_state_revision(u64::MAX);
    let before = atomic_audit(&engine);
    let replay_before = engine.history.replay_ledger_allocation_audit_for_test();

    let error = engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 760_110,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{error:?}");
    assert_eq!(
        error.message.as_ref(),
        "stateRevision cannot be incremented"
    );
    assert_eq!(error.details, Some(json!({ "field": "stateRevision" })));
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        engine.history.replay_ledger_allocation_audit_for_test(),
        replay_before
    );
}

#[test]
fn generic_structural_limit_drift_matches_preconfigured_current_and_reuses_evidence() {
    let source =
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}"#;
    let schema = tiptap_schema();
    let base_document = from_prosemirror_json(
        &serde_json::from_str(source).unwrap(),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let old_node_limit = crate::editor_state::document_node_count(base_document.root());
    let current_node_limit = old_node_limit + 1;
    let old_limits = ResourceLimits {
        max_document_nodes: old_node_limit,
        ..ResourceLimits::default()
    };
    let current_limits = ResourceLimits {
        max_document_nodes: current_node_limit,
        ..old_limits.clone()
    };

    let mut drifted = transaction_engine_with_resource_limits_and_mode(
        old_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    let mut preconfigured = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    let mut one_under = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    for engine in [&mut drifted, &mut preconfigured, &mut one_under] {
        engine
            .import_json(source, TransactionOrigin::DocumentImport)
            .unwrap();
    }
    assert_eq!(
        drifted.derived_state.as_ref().unwrap().document_node_count,
        old_node_limit
    );
    assert!(derived_evidence_matches_runtime_limits(&drifted));
    drifted.resource_limits = current_limits.clone();
    assert!(!derived_evidence_matches_runtime_limits(&drifted));

    let drifted_commit = drifted
        .apply_typed_transaction(hard_break_insert_transaction(&drifted, 760_200))
        .expect("loosened runtime limit must admit the generic structural candidate");
    let preconfigured_commit = preconfigured
        .apply_typed_transaction(hard_break_insert_transaction(&preconfigured, 760_200))
        .unwrap();
    assert_eq!(drifted_commit, preconfigured_commit);
    assert_eq!(drifted_commit.document_revision, 2);
    assert_eq!(drifted_commit.state_revision, 2);
    assert_eq!(
        drifted.derived_state.as_ref().unwrap().document_node_count,
        current_node_limit
    );
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);

    let drifted_followup = drifted
        .apply_typed_transaction(insert_transaction(&drifted, 760_201))
        .expect("current-limit evidence must be reusable by the following mutation");
    let preconfigured_followup = preconfigured
        .apply_typed_transaction(insert_transaction(&preconfigured, 760_201))
        .unwrap();
    assert_eq!(drifted_followup, preconfigured_followup);
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);

    one_under.resource_limits = old_limits;
    let before = atomic_audit(&one_under);
    let error = one_under
        .apply_typed_transaction(hard_break_insert_transaction(&one_under, 760_202))
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(old_node_limit).unwrap()));
    assert_eq!(
        error.actual,
        Some(u64::try_from(current_node_limit).unwrap())
    );
    assert_eq!(atomic_audit(&one_under), before);
}

#[test]
fn remote_limit_drift_matches_preconfigured_current_and_reuses_evidence() {
    let source_json =
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}"#;
    let schema = tiptap_schema();
    let base_document = from_prosemirror_json(
        &serde_json::from_str(source_json).unwrap(),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let old_node_limit = crate::editor_state::document_node_count(base_document.root());
    let current_node_limit = old_node_limit + 1;
    let old_limits = ResourceLimits {
        max_document_nodes: old_node_limit,
        ..ResourceLimits::default()
    };
    let current_limits = ResourceLimits {
        max_document_nodes: current_node_limit,
        ..old_limits.clone()
    };
    let mut source = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    source
        .import_json(source_json, TransactionOrigin::DocumentImport)
        .unwrap();
    let base_update = source.encoded_state().unwrap();
    let mut drifted = transaction_engine_with_resource_limits_and_mode(
        old_limits,
        crate::yrs_engine::InitializationMode::AwaitRemote,
    );
    let mut preconfigured = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::AwaitRemote,
    );
    let drifted_base = drifted
        .apply_remote_update_v1(760_210, &base_update)
        .unwrap();
    let preconfigured_base = preconfigured
        .apply_remote_update_v1(760_210, &base_update)
        .unwrap();
    assert_eq!(drifted_base, preconfigured_base);
    assert!(derived_evidence_matches_runtime_limits(&drifted));
    drifted.resource_limits = current_limits;
    assert!(!derived_evidence_matches_runtime_limits(&drifted));

    let target_vector = drifted.doc.transact().state_vector();
    source
        .apply_typed_transaction(paragraph_insert_transaction(&source, 760_211))
        .unwrap();
    let structural_delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);
    let drifted_commit = drifted
        .apply_remote_update_v1(760_212, &structural_delta)
        .expect("loosened runtime limit must admit the changed remote candidate");
    let preconfigured_commit = preconfigured
        .apply_remote_update_v1(760_212, &structural_delta)
        .unwrap();
    assert_eq!(drifted_commit, preconfigured_commit);
    assert_eq!(
        drifted.derived_state.as_ref().unwrap().document_node_count,
        current_node_limit
    );
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);

    let target_vector = drifted.doc.transact().state_vector();
    source
        .apply_typed_transaction(insert_transaction(&source, 760_213))
        .unwrap();
    let followup_delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);
    let drifted_followup = drifted
        .apply_remote_update_v1(760_214, &followup_delta)
        .expect("remote current-limit evidence must be reusable");
    let preconfigured_followup = preconfigured
        .apply_remote_update_v1(760_214, &followup_delta)
        .unwrap();
    assert_eq!(drifted_followup, preconfigured_followup);
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);
}

#[test]
fn empty_skip_collapsed_text_prepares_one_forward_point_without_reverse_traversal() {
    use crate::yrs_engine::derived_state::{
        reset_relative_selection_traversal_counts_for_test,
        take_relative_selection_traversal_counts_for_test,
    };
    use crate::yrs_engine::position::{
        reset_relative_position_traversal_counts_for_test,
        take_relative_position_traversal_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"prefix"}]},{"type":"paragraph","content":[{"type":"text","text":"a😀middle"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = RevisionedPosition {
        offset: 9,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    reset_relative_position_traversal_counts_for_test();
    reset_relative_selection_traversal_counts_for_test();

    let result = engine
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 759,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();

    assert!(result.changed);
    assert_eq!(result.document_revision, 1);
    assert_eq!(result.state_revision, 2);
    assert!(matches!(
        result.selection,
        ResolvedSelection::Text { anchor, head }
            if anchor == head && anchor.scalar == point.offset
    ));
    assert_eq!(
        take_relative_position_traversal_counts_for_test(),
        (0, 1, 0),
        "collapsed exact inputs must share one admitted forward materialization"
    );
    assert_eq!(
        take_relative_selection_traversal_counts_for_test(),
        (0, 0),
        "prepared resolved points must not round-trip through Yrs"
    );
}

#[test]
fn empty_skip_prepared_collapsed_text_preserves_overflow_and_output_atomicity() {
    fn populated_engine() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abcdef"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        let point = RevisionedPosition {
            offset: 3,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::Before,
        };
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        }
    }

    let mut overflow = populated_engine();
    overflow.state_revision = u64::MAX;
    overflow.derived_state.as_mut().unwrap().state_revision = u64::MAX;
    let overflow_before = atomic_audit(&overflow);
    let overflow_transaction = transaction(&overflow, 759_001);

    let overflow_error = overflow
        .apply_typed_transaction_with_result(overflow_transaction)
        .unwrap_err();

    assert_eq!(overflow_error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(
        overflow_error.details,
        Some(json!({ "field": "stateRevision" }))
    );
    assert_eq!(atomic_audit(&overflow), overflow_before);

    let mut output_limited = populated_engine();
    output_limited.editing_limits.max_derived_output_bytes = 1;
    let output_before = atomic_audit(&output_limited);
    let output_transaction = transaction(&output_limited, 759_002);

    let output_error = output_limited
        .apply_typed_transaction_with_result(output_transaction)
        .unwrap_err();

    assert_eq!(output_error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        output_error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );
    assert_eq!(atomic_audit(&output_limited), output_before);
}

#[test]
fn empty_skip_fast_path_matches_full_compiler_at_yrs_scan_work_boundary() {
    fn populated_engine() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"scan boundary"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn scan_work(engine: &YrsDocumentEngine) -> usize {
        let document_text_bytes = engine.document().unwrap().root().text_content().len();
        let txn = engine.doc.transact();
        let crdt_clock_work = txn
            .state_vector()
            .iter()
            .map(|(_, clock)| usize::try_from(*clock).unwrap() + 1)
            .sum::<usize>();
        document_text_bytes * 2 + crdt_clock_work * 2
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Skip,
        }
    }

    let required = scan_work(&populated_engine());

    let mut exact_fast = populated_engine();
    exact_fast.resource_limits.max_input_bytes = required;
    let exact_fast_result = exact_fast
        .apply_typed_transaction_with_result(transaction(&exact_fast, 763))
        .unwrap();
    let mut exact_slow = populated_engine();
    exact_slow.resource_limits.max_input_bytes = required;
    let exact_slow_transaction = transaction(&exact_slow, 763);
    let exact_slow_compiled = exact_slow
        .compile_typed_transaction(exact_slow_transaction)
        .unwrap();
    let exact_slow_result = exact_slow
        .apply_compiled_transaction(exact_slow_compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(exact_fast_result, exact_slow_result);
    assert_eq!(exact_fast.document_json(), exact_slow.document_json());
    assert_eq!(exact_fast.document_html(), exact_slow.document_html());
    assert_eq!(exact_fast.revision(), exact_slow.revision());
    assert_eq!(exact_fast.state_revision(), exact_slow.state_revision());
    assert_eq!(
        exact_fast.resolved_selection(),
        exact_slow.resolved_selection()
    );
    assert_eq!(exact_fast.stored_marks(), exact_slow.stored_marks());
    assert_eq!(exact_fast.can_undo(), exact_slow.can_undo());
    assert_eq!(exact_fast.can_redo(), exact_slow.can_redo());

    let mut one_under_slow = populated_engine();
    one_under_slow.resource_limits.max_input_bytes = required - 1;
    let before_slow = atomic_audit(&one_under_slow);
    let slow_error = one_under_slow
        .compile_typed_transaction(transaction(&one_under_slow, 764))
        .unwrap_err();
    assert_eq!(atomic_audit(&one_under_slow), before_slow);

    let mut one_under_fast = populated_engine();
    one_under_fast.resource_limits.max_input_bytes = required - 1;
    let before_fast = atomic_audit(&one_under_fast);
    let fast_error = one_under_fast
        .apply_typed_transaction_with_result(transaction(&one_under_fast, 764))
        .unwrap_err();
    assert_eq!(fast_error, slow_error);
    assert_eq!(atomic_audit(&one_under_fast), before_fast);

    let mut changed_document = populated_engine();
    changed_document
        .apply_command(765, TypedCommand::InsertText { text: "é".into() })
        .unwrap()
        .unwrap();
    let cached_text_bytes = changed_document
        .derived_state
        .as_ref()
        .unwrap()
        .document_text_bytes;
    assert_eq!(
        cached_text_bytes,
        changed_document
            .document()
            .unwrap()
            .root()
            .text_content()
            .len()
    );
    let changed_required = scan_work(&changed_document);
    changed_document.resource_limits.max_input_bytes = changed_required - 1;
    let before_changed = atomic_audit(&changed_document);
    let changed_error = changed_document
        .apply_typed_transaction_with_result(transaction(&changed_document, 766))
        .unwrap_err();
    assert_eq!(changed_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(
        changed_error.limit,
        Some(u64::try_from(changed_required - 1).unwrap())
    );
    assert_eq!(
        changed_error.actual,
        Some(u64::try_from(changed_required).unwrap())
    );
    assert_eq!(atomic_audit(&changed_document), before_changed);

    let invalid_selection = |engine: &YrsDocumentEngine| TypedTransaction {
        request_id: 767,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![],
        selection_intent: SelectionIntent::Set(SelectionInput::Text {
            anchor: RevisionedPosition {
                offset: u32::MAX,
                kind: EditorOffsetKind::Utf16,
                affinity: Affinity::Before,
            },
            head: RevisionedPosition {
                offset: u32::MAX,
                kind: EditorOffsetKind::Utf16,
                affinity: Affinity::After,
            },
        }),
        history_policy: HistoryPolicy::Skip,
    };
    let invalid_slow = populated_engine();
    let before_invalid_slow = atomic_audit(&invalid_slow);
    let invalid_slow_error = invalid_slow
        .compile_typed_transaction(invalid_selection(&invalid_slow))
        .unwrap_err();
    assert_eq!(atomic_audit(&invalid_slow), before_invalid_slow);
    let mut invalid_fast = populated_engine();
    let before_invalid_fast = atomic_audit(&invalid_fast);
    let invalid_fast_error = invalid_fast
        .apply_typed_transaction_with_result(invalid_selection(&invalid_fast))
        .unwrap_err();
    assert_eq!(invalid_fast_error, invalid_slow_error);
    assert_eq!(atomic_audit(&invalid_fast), before_invalid_fast);
}

#[test]
fn empty_skip_fast_path_matches_full_compiler_for_selection_forms_and_local_state() {
    fn populated_engine() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                &json!({
                    "type": "doc",
                    "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "a😀b"}]},
                        {"type": "horizontalRule"},
                        {"type": "paragraph", "content": [{"type": "text", "text": "tail"}]}
                    ]
                })
                .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn transaction(
        engine: &YrsDocumentEngine,
        request_id: u64,
        selection_intent: SelectionIntent,
    ) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent,
            history_policy: HistoryPolicy::Skip,
        }
    }

    fn slow_result(
        engine: &mut YrsDocumentEngine,
        transaction: TypedTransaction,
    ) -> crate::yrs_engine::TypedTransactionResult {
        let compiled = engine.compile_typed_transaction(transaction).unwrap();
        engine
            .apply_compiled_transaction(compiled, true)
            .unwrap()
            .1
            .unwrap()
    }

    let scalar = |offset, affinity| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity,
    };
    let utf16 = |offset, affinity| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Utf16,
        affinity,
    };
    let intents = [
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(2, Affinity::Before),
            head: scalar(2, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(2, Affinity::Before),
            head: scalar(2, Affinity::After),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: utf16(3, Affinity::Before),
            head: utf16(3, Affinity::After),
        }),
        SelectionIntent::Set(SelectionInput::Node {
            at: scalar(4, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::All),
        SelectionIntent::Preserve,
        SelectionIntent::UseOperationResult,
    ];

    for (index, intent) in intents.into_iter().enumerate() {
        let mut fast = populated_engine();
        let mut slow = populated_engine();
        let fast_before = atomic_audit(&fast);
        let slow_before = atomic_audit(&slow);
        let fast_transaction = transaction(&fast, 770 + index as u64, intent.clone());
        let slow_transaction = transaction(&slow, 770 + index as u64, intent.clone());

        let fast_result = fast
            .apply_typed_transaction_with_result(fast_transaction)
            .unwrap();
        let slow_result = slow_result(&mut slow, slow_transaction);

        assert_eq!(fast_result, slow_result, "intent={intent:?}");
        assert_eq!(
            fast.document_json(),
            slow.document_json(),
            "intent={intent:?}"
        );
        assert_eq!(
            fast.document_html(),
            slow.document_html(),
            "intent={intent:?}"
        );
        assert_eq!(fast.revision(), slow.revision(), "intent={intent:?}");
        assert_eq!(
            fast.state_revision(),
            slow.state_revision(),
            "intent={intent:?}"
        );
        assert_eq!(
            fast.resolved_selection(),
            slow.resolved_selection(),
            "intent={intent:?}"
        );
        assert_eq!(
            fast.stored_marks(),
            slow.stored_marks(),
            "intent={intent:?}"
        );
        assert_eq!(fast.can_undo(), slow.can_undo(), "intent={intent:?}");
        assert_eq!(fast.can_redo(), slow.can_redo(), "intent={intent:?}");
        assert_eq!(fast.encoded_state().unwrap(), fast_before.encoded);
        assert_eq!(slow.encoded_state().unwrap(), slow_before.encoded);
        assert_eq!(fast.yrs_state_epoch, fast_before.yrs_state_epoch);
        assert_eq!(slow.yrs_state_epoch, slow_before.yrs_state_epoch);
        assert_eq!(
            fast.history.replay_audit_for_test(),
            fast_before.replay_audit
        );
        assert_eq!(
            slow.history.replay_audit_for_test(),
            slow_before.replay_audit
        );
    }

    let stored_mark_intents = [
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(1, Affinity::Before),
            head: scalar(1, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(1, Affinity::Before),
            head: scalar(1, Affinity::After),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(2, Affinity::Before),
            head: scalar(2, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::Node {
            at: scalar(4, Affinity::Before),
        }),
    ];
    for (index, intent) in stored_mark_intents.into_iter().enumerate() {
        let mut fast = populated_engine();
        let mut slow = populated_engine();
        select_text(&mut fast, 780, 1, 1);
        select_text(&mut slow, 780, 1, 1);
        for engine in [&mut fast, &mut slow] {
            engine
                .apply_command(
                    781,
                    TypedCommand::ToggleMark {
                        mark_type: "bold".into(),
                    },
                )
                .unwrap()
                .unwrap();
            assert!(engine.stored_marks().is_some());
        }
        let fast_transaction = transaction(&fast, 782 + index as u64, intent.clone());
        let slow_transaction = transaction(&slow, 782 + index as u64, intent.clone());

        let fast_result = fast
            .apply_typed_transaction_with_result(fast_transaction)
            .unwrap();
        let slow_result = slow_result(&mut slow, slow_transaction);

        assert_eq!(fast_result, slow_result, "stored intent={intent:?}");
        assert_eq!(
            fast.resolved_selection(),
            slow.resolved_selection(),
            "stored intent={intent:?}"
        );
        assert_eq!(
            fast.stored_marks(),
            slow.stored_marks(),
            "stored intent={intent:?}"
        );
        if index <= 1 {
            assert!(fast.stored_marks().is_some());
        } else {
            assert!(fast.stored_marks().is_none());
        }
    }
}

#[test]
fn remote_history_admission_failure_retains_dependency_quarantine_for_retry() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use yrs::{diff_updates_v1, encode_state_vector_from_update_v1};

    let mut source = transaction_engine();
    let base = source.encoded_state().unwrap();
    source
        .apply_command(200, TypedCommand::InsertText { text: "a".into() })
        .unwrap();
    let after_a = source.encoded_state().unwrap();
    source
        .apply_command(201, TypedCommand::InsertText { text: "b".into() })
        .unwrap();
    let after_b = source.encoded_state().unwrap();
    let base_sv = encode_state_vector_from_update_v1(&base).unwrap();
    let after_a_sv = encode_state_vector_from_update_v1(&after_a).unwrap();
    let delta_a = diff_updates_v1(&after_a, &base_sv).unwrap();
    let delta_b = diff_updates_v1(&after_b, &after_a_sv).unwrap();

    let mut target = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "doc".into(),
            lineage_id: "lineage".into(),
        }),
    })
    .unwrap();
    assert!(
        !target
            .apply_remote_update_v1(202, &delta_b)
            .unwrap()
            .changed
    );
    assert!(
        !target
            .apply_remote_update_v1(203, &delta_a)
            .unwrap()
            .changed
    );
    let before = atomic_audit(&target);

    set_atomic_failpoint_for_test(Some(AtomicFailpoint::RemoteHistoryAdmission));
    let error = target.apply_remote_update_v1(204, &base).unwrap_err();
    set_atomic_failpoint_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(
        error.details,
        Some(json!({ "failpoint": "remoteHistoryAdmission" }))
    );
    assert_eq!(atomic_audit(&target), before);
    let retry = target.apply_remote_update_v1(205, &base).unwrap();
    assert!(retry.changed);
    assert_eq!(target.document().unwrap().root().text_content(), "ab");
    assert_eq!(target.encoded_state().unwrap(), after_b);
}

/// Task 9 classification seam: the read-only preflight accepts exactly
/// what the prepare pipeline's ingress admission accepts, rejects
/// malformed encodings with the same structured errors, and never
/// touches engine state.
#[test]
fn preflight_remote_update_v1_classifies_encoding_without_engine_effects() {
    let mut source = transaction_engine();
    source
        .apply_command(210, TypedCommand::InsertText { text: "pf".into() })
        .unwrap();
    let valid = source.encoded_state().unwrap();
    let engine = transaction_engine();
    let before = atomic_audit(&engine);

    engine.preflight_remote_update_v1(211, &valid).unwrap();
    engine.preflight_remote_update_v1(212, &[0, 0]).unwrap();

    let error = engine
        .preflight_remote_update_v1(213, &[0xff, 0xff, 0xff])
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(error.request_id, 213);

    let mut truncated = valid.clone();
    truncated.truncate(valid.len() / 2);
    assert!(engine.preflight_remote_update_v1(214, &truncated).is_err());

    assert_eq!(atomic_audit(&engine), before);
}

/// Task 9 accounting seam: the engine reports its retained
/// dependency-quarantine bytes (the exact pending payload length) and
/// returns to zero once the dependency completes.
#[test]
fn pending_remote_dependency_bytes_tracks_the_quarantine_lifecycle() {
    use yrs::{diff_updates_v1, encode_state_vector_from_update_v1};

    let mut source = transaction_engine();
    let base = source.encoded_state().unwrap();
    source
        .apply_command(220, TypedCommand::InsertText { text: "q".into() })
        .unwrap();
    let after = source.encoded_state().unwrap();
    let base_sv = encode_state_vector_from_update_v1(&base).unwrap();
    let delta = diff_updates_v1(&after, &base_sv).unwrap();

    let mut target = transaction_engine();
    assert_eq!(target.pending_remote_dependency_bytes(), 0);

    // transaction_engine() starts from a different lineage than
    // `source`, so the delta's dependencies are missing and quarantine.
    assert!(!target.apply_remote_update_v1(221, &delta).unwrap().changed);
    assert_eq!(target.pending_remote_dependency_bytes(), delta.len());

    assert!(target.apply_remote_update_v1(222, &base).unwrap().changed);
    assert_eq!(target.pending_remote_dependency_bytes(), 0);
    assert_eq!(target.document().unwrap().root().text_content(), "q");
}

#[test]
fn state_only_boundary_reservation_failure_is_fully_atomic() {
    use crate::yrs_engine::history::set_boundary_reservation_failure_for_test;

    let mut engine = transaction_engine();
    let before = atomic_audit(&engine);
    set_boundary_reservation_failure_for_test(true);

    let error = engine
        .apply_command(
            90,
            crate::yrs_engine::TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap_err();

    set_boundary_reservation_failure_for_test(false);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(atomic_audit(&engine), before);
}

/// Task 16B: the quarantined remote-update reservation is a demonstrated
/// fallible allocation seam and keeps OPERATION_RESOURCE_EXHAUSTED.
#[test]
fn quarantined_remote_update_reservation_failure_keeps_resource_exhausted() {
    use yrs::{diff_updates_v1, encode_state_vector_from_update_v1};

    let mut source = transaction_engine();
    let base = source.encoded_state().unwrap();
    source
        .apply_command(220, TypedCommand::InsertText { text: "q".into() })
        .unwrap();
    let after = source.encoded_state().unwrap();
    let base_sv = encode_state_vector_from_update_v1(&base).unwrap();
    let delta = diff_updates_v1(&after, &base_sv).unwrap();

    let mut target = transaction_engine();
    let before = atomic_audit(&target);
    super::set_quarantined_update_reservation_failure_for_test(true);
    let error = target.apply_remote_update_v1(221, &delta).unwrap_err();
    super::set_quarantined_update_reservation_failure_for_test(false);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(error.details, Some(json!({ "field": "remoteUpdate" })));
    assert_eq!(atomic_audit(&target), before);
    // Recovery: the identical update quarantines once allocation recovers.
    assert!(!target.apply_remote_update_v1(221, &delta).unwrap().changed);
}

/// Task 16B: the outbound staging-copy allocation seam keeps
/// OPERATION_RESOURCE_EXHAUSTED.
#[test]
fn outbound_staging_copy_allocation_failure_keeps_resource_exhausted() {
    let limits = crate::session::CollaborationLimits::default();
    let mut outbox = crate::collaboration_runtime::CollaborationOutbox::from_limits(&limits);
    let mut sink = OutboundUpdateSink::attached(&mut outbox);
    super::set_outbound_staging_copy_failure_for_test(true);
    let error = sink.reserve_and_stage(41, 4, &[1, 2, 3]).unwrap_err();
    super::set_outbound_staging_copy_failure_for_test(false);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(
        error.details,
        Some(json!({ "field": "pendingOutboxUpdateBytes" }))
    );
    sink.reserve_and_stage(41, 4, &[1, 2, 3]).unwrap();
}

/// Task 6 fix round 1: exact/one-over coverage of the shared
/// `maxEncodedStateBytes` gate used by the remote pipeline and the sealed
/// state-vector/diff encoders. The state-vector *output* branch is
/// unreachable through any consistent engine (the full encoded state is
/// strictly larger than its state vector and is bounded by the same
/// ceiling on every admission path), so the gate is proven here at the
/// boundary instead.
#[test]
fn max_encoded_state_gate_admits_exact_and_rejects_one_over() {
    assert!(super::admit_max_encoded_state_len(90_001, 64, 64).is_ok());
    assert!(super::admit_max_encoded_state_len(90_002, 0, 0).is_ok());

    let error = super::admit_max_encoded_state_len(90_003, 65, 64).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.request_id, 90_003);
    assert_eq!(error.limit, Some(64));
    assert_eq!(error.actual, Some(65));
    assert_eq!(
        error.details.as_ref().unwrap()["field"],
        "maxEncodedStateBytes"
    );
}

/// Task 6 same-doc binding proof: the codec's sole `Awareness` wraps the
/// live authoritative `Doc` handle (documents edits are visible through
/// it, the client identity matches), and the binding follows every store
/// swap (undo/redo candidate installation and import).
#[test]
fn awareness_codec_owns_an_awareness_bound_to_the_live_doc() {
    use yrs::GetString;

    fn bound_fragment_text(engine: &YrsDocumentEngine) -> String {
        let codec = engine.awareness.as_ref().expect("codec stays bound");
        let doc = codec.doc_for_test();
        assert!(
            Doc::ptr_eq(doc, &engine.doc),
            "awareness must wrap the live authoritative doc handle"
        );
        assert_eq!(doc.client_id().get(), engine.client_id());
        let txn = doc.transact();
        txn.get_xml_fragment("prosemirror")
            .expect("live doc retains the document fragment")
            .get_string(&txn)
    }

    let mut engine =
        transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits::default());
    engine.awareness();

    engine
        .apply_command(
            1,
            TypedCommand::InsertText {
                text: "bound".into(),
            },
        )
        .unwrap()
        .expect("insert applies");
    assert!(bound_fragment_text(&engine).contains("bound"));

    engine.undo(2).unwrap().expect("undo applies");
    assert!(!bound_fragment_text(&engine).contains("bound"));

    engine
        .import_json(
            &json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"imported"}]}]})
                .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(bound_fragment_text(&engine).contains("imported"));
}
