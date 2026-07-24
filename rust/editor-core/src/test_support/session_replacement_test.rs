use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::native_bridge_test_support as bridge;
use crate::session_initialization_test_support::{
    ack_outbound, can_redo, can_undo, client_id, collaboration_drive, collaboration_socket_close,
    collaboration_socket_open, create_local_json, create_local_json_with_options,
    create_room_from_json, destroy_session, encoded_state, get_json, has_document_state,
    lease_outbound, mark_synchronized_for_test, redo, registry_count, request_edit, session_audit,
    set_transport_state_for_test, undo, write_html, write_json, CloseDisposition, DocumentState,
    TransportState,
};
use crate::tiptap_schema;
use crate::yrs_engine::{
    DocumentScope, EditingLimits, InitializationMode, ReplacementHistory, RootReplacementError,
    TransactionOrigin, YrsDocumentEngine, YrsEngineConfig,
};
use yrs::updates::decoder::Decode;
use yrs::{Doc, GetString, ReadTxn, Transact, Update};

const JSON_BEFORE: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"before replacement"}]}]}"#;
const JSON_REPLACEMENT: &str = r#"{"type":"doc","content":[{"type":"h2","content":[{"type":"text","text":"replaced"}]},{"type":"paragraph","content":[{"type":"text","text":"body"}]}]}"#;
const HTML_REPLACEMENT: &str = "<h2>replaced</h2><p>body</p>";
const JSON_MALFORMED: &str = r#"{"type":"doc","content":["#;
const JSON_SCHEMA_INVALID: &str =
    r#"{"type":"doc","content":[{"type":"text","text":"text directly under doc"}]}"#;

const CONNECTED_STATES: [TransportState; 3] = [
    TransportState::Connecting,
    TransportState::Handshaking,
    TransportState::Synchronized,
];

fn test_guard() -> crate::test_support::RegistryConcurrencyGuard {
    crate::test_support::RegistryConcurrencyGuard::acquire()
}

fn replacement_json_value() -> serde_json::Value {
    serde_json::from_str(JSON_REPLACEMENT).unwrap()
}

fn snapshot_source() -> crate::yrs_engine::DocumentSnapshot {
    let mut source = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: "replacement-room".into(),
            lineage_id: "replacement-lineage".into(),
        }),
    })
    .unwrap();
    source
        .import_json(JSON_BEFORE, TransactionOrigin::DocumentImport)
        .unwrap();
    source.export_snapshot().unwrap()
}

fn create_ready_room() -> u64 {
    let snapshot = snapshot_source();
    let config = serde_json::json!({
        "documentId": snapshot.document_id,
        "lineageId": snapshot.lineage_id,
        "snapshot": snapshot,
    });
    create_room_from_json(&config.to_string()).unwrap()
}

fn state_vector_entries(update: &[u8]) -> HashMap<u64, u32> {
    let doc = Doc::new();
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(update).unwrap())
            .unwrap();
    }
    let txn = doc.transact();
    let vector = txn.state_vector();
    vector
        .iter()
        .map(|(client, clock)| (client.get(), *clock))
        .collect()
}

#[test]
fn await_remote_sessions_reject_replacement_not_ready_for_every_transport() {
    let _guard = test_guard();
    let id = create_room_from_json(
        r#"{"documentId":"replacement-room","lineageId":"replacement-lineage"}"#,
    )
    .unwrap();
    bridge::attach_runtime(id).unwrap();
    assert_eq!(
        crate::session_initialization_test_support::document_state(id).unwrap(),
        DocumentState::AwaitRemote
    );

    // Row 1 of the policy matrix: AwaitRemote + ANY transport state is
    // ENGINE_NOT_READY, even when the transport claims to be connected.
    // Every state is reached through the real Task 8 transitions: connect,
    // socket open, current-generation synchronize, incompatible close.
    let mut generation = 0;
    for transport in [
        TransportState::Disconnected,
        TransportState::Connecting,
        TransportState::Handshaking,
        TransportState::Synchronized,
        TransportState::Incompatible,
    ] {
        match transport {
            TransportState::Disconnected => {}
            TransportState::Connecting => {
                generation = collaboration_drive(id, 500, 0)
                    .unwrap()
                    .generation_to_open
                    .expect("the initial Rust drive must issue a generation");
            }
            TransportState::Handshaking => {
                collaboration_socket_open(id, 500, generation, 0).unwrap();
                let step1 = lease_outbound(id, 500, generation)
                    .unwrap()
                    .expect("socket open must queue Sync Step 1");
                ack_outbound(id, 500, generation, step1.lease_id).unwrap();
            }
            TransportState::Synchronized => {
                mark_synchronized_for_test(id, 500, generation).unwrap();
            }
            TransportState::Incompatible => {
                collaboration_socket_close(id, 500, generation, CloseDisposition::Incompatible, 0)
                    .unwrap();
            }
            TransportState::Detached => unreachable!("not part of the AwaitRemote row"),
        }
        assert_eq!(
            crate::session_initialization_test_support::transport_state(id).unwrap(),
            transport
        );
        for (request_id, mode) in [
            (501, ReplacementHistory::UndoableBoundary),
            (502, ReplacementHistory::ResetAndClear),
        ] {
            let error = write_json(id, request_id, JSON_REPLACEMENT, mode).unwrap_err();
            assert_eq!(error.domain, "operation", "{transport:?} {mode:?}");
            assert_eq!(error.code, "ENGINE_NOT_READY", "{transport:?} {mode:?}");
            assert_eq!(error.request_id, Some(request_id), "{transport:?} {mode:?}");
        }
        let error = write_html(
            id,
            503,
            HTML_REPLACEMENT,
            ReplacementHistory::UndoableBoundary,
        )
        .unwrap_err();
        assert_eq!(error.domain, "operation", "{transport:?}");
        assert_eq!(error.code, "ENGINE_NOT_READY", "{transport:?}");
        assert_eq!(error.request_id, Some(503), "{transport:?}");

        // Rejection is atomic: the engine still has no authored state.
        assert!(!has_document_state(id).unwrap(), "{transport:?}");
        assert_eq!(
            crate::session_initialization_test_support::document_state(id).unwrap(),
            DocumentState::AwaitRemote,
            "{transport:?}"
        );
    }

    destroy_session(id);
}

#[test]
fn connected_transports_reject_replacement_with_the_frozen_lifecycle_code() {
    let _guard = test_guard();
    let local = create_local_json(JSON_BEFORE).unwrap();
    let room = create_ready_room();

    for id in [local, room] {
        // Seed real history so the audit proves it survives rejections.
        assert!(request_edit(id, 601, " seeded").unwrap().can_undo);

        for transport in CONNECTED_STATES {
            set_transport_state_for_test(id, transport).unwrap();
            let audit = session_audit(id).unwrap();

            for (request_id, mode) in [
                (611, ReplacementHistory::UndoableBoundary),
                (612, ReplacementHistory::ResetAndClear),
            ] {
                let error = write_json(id, request_id, JSON_REPLACEMENT, mode).unwrap_err();
                assert_eq!(error.domain, "lifecycle", "{transport:?} {mode:?}");
                assert_eq!(
                    error.code, "WHOLE_DOCUMENT_REPLACEMENT_CONNECTED",
                    "{transport:?} {mode:?}"
                );
                assert_eq!(error.request_id, Some(request_id), "{transport:?} {mode:?}");

                let error = write_html(id, request_id + 10, HTML_REPLACEMENT, mode).unwrap_err();
                assert_eq!(error.domain, "lifecycle", "{transport:?} {mode:?}");
                assert_eq!(
                    error.code, "WHOLE_DOCUMENT_REPLACEMENT_CONNECTED",
                    "{transport:?} {mode:?}"
                );
                assert_eq!(
                    error.request_id,
                    Some(request_id + 10),
                    "{transport:?} {mode:?}"
                );
            }

            // The complete audit is untouched by the rejections.
            assert_eq!(session_audit(id).unwrap(), audit, "{transport:?}");
        }

        // History is still live after every rejection: undo restores the
        // pre-edit document and redo restores the edit.
        set_transport_state_for_test(
            id,
            if id == local {
                TransportState::Detached
            } else {
                TransportState::Disconnected
            },
        )
        .unwrap();
        assert!(undo(id, 620).unwrap());
        assert_eq!(
            get_json(id).unwrap(),
            serde_json::from_str::<serde_json::Value>(JSON_BEFORE).unwrap()
        );
        assert!(redo(id, 621).unwrap());
        destroy_session(id);
    }
}

/// Label, session factory, and optional forced transport for an allowed row.
type AllowedReplacementRow = (&'static str, fn() -> u64, Option<TransportState>);

#[test]
fn ready_disconnected_rows_replace_in_the_same_store() {
    let _guard = test_guard();

    // LocalReady + Detached, LocalReady + Incompatible, RoomReady +
    // Disconnected, RoomReady + Incompatible are all allowed rows.
    let cases: [AllowedReplacementRow; 4] = [
        (
            "local detached",
            || create_local_json(JSON_BEFORE).unwrap(),
            None,
        ),
        (
            "local incompatible",
            || create_local_json(JSON_BEFORE).unwrap(),
            Some(TransportState::Incompatible),
        ),
        ("room disconnected", create_ready_room, None),
        (
            "room incompatible",
            create_ready_room,
            Some(TransportState::Incompatible),
        ),
    ];

    for (label, create, forced_transport) in cases {
        let id = create();
        if let Some(transport) = forced_transport {
            set_transport_state_for_test(id, transport).unwrap();
        }

        let before_entries = state_vector_entries(&encoded_state(id).unwrap());
        let writer = client_id(id).unwrap();
        let before_revision = session_audit(id).unwrap().document_revision;

        let outcome = write_json(
            id,
            701,
            JSON_REPLACEMENT,
            ReplacementHistory::UndoableBoundary,
        )
        .unwrap();
        assert!(outcome.changed, "{label}");
        assert_eq!(
            outcome.document_revision,
            before_revision + 1,
            "{label}: same-store replacement continues the durable revision sequence"
        );
        assert_eq!(get_json(id).unwrap(), replacement_json_value(), "{label}");

        // Same-store proof: the same client identity continued writing into
        // the existing store. Its clock strictly advanced and every other
        // state-vector entry is untouched.
        assert_eq!(client_id(id).unwrap(), writer, "{label}");
        let after_entries = state_vector_entries(&encoded_state(id).unwrap());
        assert!(
            after_entries.get(&writer).copied().unwrap_or(0)
                > before_entries.get(&writer).copied().unwrap_or(0),
            "{label}: writer clock must strictly advance ({before_entries:?} -> {after_entries:?})"
        );
        for (client, clock) in &before_entries {
            if *client != writer {
                assert_eq!(
                    after_entries.get(client),
                    Some(clock),
                    "{label}: foreign entry {client} must be untouched"
                );
            }
        }
        // The only permitted state-vector shape change is the writer's own
        // first entry appearing (a snapshot-restored room's fresh local client
        // has clock 0 until it first writes). No other identity may appear.
        let expected_entries =
            before_entries.len() + usize::from(!before_entries.contains_key(&writer));
        assert_eq!(
            after_entries.len(),
            expected_entries,
            "{label}: replacement must not mint a new client identity"
        );

        destroy_session(id);
    }
}

#[test]
fn room_ready_replacement_converges_on_a_peer_via_incremental_update() {
    let _guard = test_guard();
    let id = create_ready_room();

    // The peer replica holds the exact prior room state.
    let prior_state = encoded_state(id).unwrap();
    let peer = Doc::new();
    let peer_fragment = peer.get_or_insert_xml_fragment("prosemirror");
    {
        let mut txn = peer.transact_mut();
        txn.apply_update(Update::decode_v1(&prior_state).unwrap())
            .unwrap();
    }
    let base_state_vector = peer.transact().state_vector();

    let outcome = write_json(
        id,
        711,
        JSON_REPLACEMENT,
        ReplacementHistory::UndoableBoundary,
    )
    .unwrap();
    assert!(outcome.changed);

    // Encode the standard incremental Update-v1 (base state vector -> after)
    // from a replica of the after state, and apply only that increment.
    let after_state = encoded_state(id).unwrap();
    let replica = Doc::new();
    let replica_fragment = replica.get_or_insert_xml_fragment("prosemirror");
    {
        let mut txn = replica.transact_mut();
        txn.apply_update(Update::decode_v1(&after_state).unwrap())
            .unwrap();
    }
    let incremental = replica
        .transact()
        .encode_state_as_update_v1(&base_state_vector);
    assert!(
        !incremental.is_empty(),
        "replacement must produce an incremental update"
    );
    {
        let mut txn = peer.transact_mut();
        txn.apply_update(Update::decode_v1(&incremental).unwrap())
            .unwrap();
    }

    // The peer converged to the exact replacement document with state-vector
    // equality — standard incremental convergence, not a full-state reset.
    let peer_txn = peer.transact();
    let replica_txn = replica.transact();
    assert_eq!(
        peer_txn.state_vector(),
        replica_txn.state_vector(),
        "peer must reach state-vector equality"
    );
    assert_eq!(
        peer_fragment.get_string(&peer_txn),
        replica_fragment.get_string(&replica_txn),
        "peer must hold the exact replacement document"
    );
    assert_ne!(peer_txn.state_vector(), base_state_vector);

    destroy_session(id);
}

#[test]
fn undoable_boundary_replacement_is_exactly_one_undo_group() {
    let _guard = test_guard();
    for (label, html) in [("json", false), ("html", true)] {
        let id = create_local_json(JSON_BEFORE).unwrap();
        assert!(request_edit(id, 801, " typed").unwrap().can_undo);
        let pre_replacement = get_json(id).unwrap();

        let outcome = if html {
            write_html(
                id,
                802,
                HTML_REPLACEMENT,
                ReplacementHistory::UndoableBoundary,
            )
            .unwrap()
        } else {
            write_json(
                id,
                802,
                JSON_REPLACEMENT,
                ReplacementHistory::UndoableBoundary,
            )
            .unwrap()
        };
        assert!(outcome.changed, "{label}");
        assert!(can_undo(id).unwrap(), "{label}");
        let replaced = get_json(id).unwrap();

        // Exactly one undo restores the exact pre-replacement document.
        assert!(undo(id, 803).unwrap(), "{label}");
        assert_eq!(get_json(id).unwrap(), pre_replacement, "{label}");
        assert!(can_redo(id).unwrap(), "{label}");

        // Redo restores the replacement.
        assert!(redo(id, 804).unwrap(), "{label}");
        assert_eq!(get_json(id).unwrap(), replaced, "{label}");
        destroy_session(id);

        // Boundary isolation, proven on a fresh session: typing, then the
        // replacement, then more typing. One undo pops only the trailing
        // typing (trailing boundary), the next pops only the replacement and
        // restores the exact pre-replacement document including the leading
        // typing (leading boundary), and the leading typing group is still
        // undoable on its own.
        let id = create_local_json(JSON_BEFORE).unwrap();
        assert!(request_edit(id, 805, " typed").unwrap().can_undo);
        let pre_replacement = get_json(id).unwrap();
        let outcome = if html {
            write_html(
                id,
                806,
                HTML_REPLACEMENT,
                ReplacementHistory::UndoableBoundary,
            )
            .unwrap()
        } else {
            write_json(
                id,
                806,
                JSON_REPLACEMENT,
                ReplacementHistory::UndoableBoundary,
            )
            .unwrap()
        };
        assert!(outcome.changed, "{label}");
        let replaced = get_json(id).unwrap();
        assert!(request_edit(id, 807, " after").unwrap().can_undo);

        assert!(undo(id, 808).unwrap(), "{label}");
        assert_eq!(
            get_json(id).unwrap(),
            replaced,
            "{label}: trailing typing must not merge into the replacement group"
        );
        assert!(undo(id, 809).unwrap(), "{label}");
        assert_eq!(
            get_json(id).unwrap(),
            pre_replacement,
            "{label}: the replacement group must not swallow the leading typing"
        );
        assert!(
            can_undo(id).unwrap(),
            "{label}: the leading typing group survives as its own undo item"
        );

        destroy_session(id);
    }
}

#[test]
fn reset_and_clear_replacement_clears_history_irrecoverably() {
    let _guard = test_guard();
    for (label, html) in [("json", false), ("html", true)] {
        let id = create_local_json(JSON_BEFORE).unwrap();
        assert!(request_edit(id, 821, " typed").unwrap().can_undo);

        let outcome = if html {
            write_html(id, 822, HTML_REPLACEMENT, ReplacementHistory::ResetAndClear).unwrap()
        } else {
            write_json(id, 822, JSON_REPLACEMENT, ReplacementHistory::ResetAndClear).unwrap()
        };
        assert!(outcome.changed, "{label}");
        assert!(!can_undo(id).unwrap(), "{label}");
        assert!(!can_redo(id).unwrap(), "{label}");

        // Prior history is unrecoverable: undo is a no-op and the document
        // stays the replacement.
        assert!(!undo(id, 823).unwrap(), "{label}");
        if !html {
            assert_eq!(get_json(id).unwrap(), replacement_json_value(), "{label}");
        }

        // New edits after the reset undo back to the replacement, never past.
        assert!(request_edit(id, 824, " after reset").unwrap().can_undo);
        assert!(undo(id, 825).unwrap(), "{label}");
        if !html {
            assert_eq!(get_json(id).unwrap(), replacement_json_value(), "{label}");
        }
        assert!(!can_undo(id).unwrap(), "{label}");

        destroy_session(id);
    }
}

#[test]
fn invalid_replacements_reject_atomically_with_import_parity() {
    let _guard = test_guard();
    let baseline = registry_count();
    let id = create_local_json_with_options(JSON_BEFORE, None, Some(64)).unwrap();
    assert!(request_edit(id, 841, " typed").unwrap().can_undo);
    let audit = session_audit(id).unwrap();

    let over_length = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "x".repeat(65) }],
        }],
    })
    .to_string();

    let cases: [(&str, &str, &str, &str); 3] = [
        (
            "malformed json",
            JSON_MALFORMED,
            "document",
            "DOCUMENT_INVALID",
        ),
        (
            "schema-invalid document",
            JSON_SCHEMA_INVALID,
            "document",
            "DOCUMENT_INVALID",
        ),
        (
            "max length ceiling",
            &over_length,
            "document",
            "DOCUMENT_LIMIT_EXCEEDED",
        ),
    ];

    for (label, input, domain, code) in cases {
        for mode in [
            ReplacementHistory::UndoableBoundary,
            ReplacementHistory::ResetAndClear,
        ] {
            let error = write_json(id, 842, input, mode).unwrap_err();
            assert_eq!(error.domain, domain, "{label} {mode:?}");
            assert_eq!(error.code, code, "{label} {mode:?}");
            assert_eq!(session_audit(id).unwrap(), audit, "{label} {mode:?}");
        }

        // Import parity: creating a session from the same input fails with the
        // same domain/code, and neither path publishes anything.
        let import_error = create_local_json_with_options(input, None, Some(64)).unwrap_err();
        assert_eq!(import_error.domain, domain, "{label}");
        assert_eq!(import_error.code, code, "{label}");
        assert_eq!(registry_count(), baseline + 1, "{label}");
    }

    // History survived every rejection.
    assert!(undo(id, 843).unwrap());
    assert_eq!(
        get_json(id).unwrap(),
        serde_json::from_str::<serde_json::Value>(JSON_BEFORE).unwrap()
    );

    destroy_session(id);
    assert_eq!(registry_count(), baseline);
}

#[test]
fn engine_ceilings_reject_replacement_with_the_exact_import_codes() {
    let _guard = test_guard();

    let build = |resource_limits: ResourceLimits, editing_limits: EditingLimits| {
        let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits,
            editing_limits,
            max_length: None,
            scope: None,
        })
        .unwrap();
        engine
            .import_json(JSON_BEFORE, TransactionOrigin::DocumentImport)
            .unwrap();
        engine
    };
    let engine_audit = |engine: &YrsDocumentEngine| {
        (
            engine.encoded_state().unwrap(),
            engine.document_json(),
            engine.revision(),
            engine.state_revision(),
            engine.can_undo(),
            engine.can_redo(),
        )
    };
    let replacement_code = |error: &RootReplacementError| match error {
        RootReplacementError::Admission(admission) => admission.code.to_string(),
        RootReplacementError::Transaction(transaction) => transaction.code.to_string(),
    };

    let long_body = "y".repeat(4_096);
    let deep = r#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"deep"}]}]}]}]}]}]}"#.to_string();
    let wide = serde_json::json!({
        "type": "doc",
        "content": (0..32)
            .map(|index| serde_json::json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": format!("p{index}") }],
            }))
            .collect::<Vec<_>>(),
    })
    .to_string();
    let big = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": long_body }],
        }],
    })
    .to_string();

    let input_limited = ResourceLimits {
        max_input_bytes: 128,
        ..ResourceLimits::default()
    };
    let node_limited = ResourceLimits {
        max_document_nodes: 8,
        ..ResourceLimits::default()
    };
    let depth_limited = ResourceLimits {
        max_document_depth: 4,
        ..ResourceLimits::default()
    };
    let derived_limited = EditingLimits {
        max_derived_output_bytes: 2_048,
        ..EditingLimits::default()
    };
    let encoded_limited = ResourceLimits {
        max_encoded_state_bytes: 1_024,
        ..ResourceLimits::default()
    };

    let cases: [(&str, ResourceLimits, EditingLimits, &str); 5] = [
        (
            "input bytes",
            input_limited,
            EditingLimits::default(),
            &wide,
        ),
        (
            "document nodes",
            node_limited,
            EditingLimits::default(),
            &wide,
        ),
        (
            "document depth",
            depth_limited,
            EditingLimits::default(),
            &deep,
        ),
        (
            "derived output",
            ResourceLimits::default(),
            derived_limited,
            &big,
        ),
        (
            "encoded state",
            encoded_limited,
            EditingLimits::default(),
            &big,
        ),
    ];

    for (label, resource_limits, editing_limits, input) in cases {
        let mut engine = build(resource_limits.clone(), editing_limits.clone());
        let before = engine_audit(&engine);

        let replacement_error = engine
            .prepare_root_replacement_json(901, input, ReplacementHistory::UndoableBoundary)
            .unwrap_err();
        assert_eq!(engine_audit(&engine), before, "{label}: replacement audit");

        let import_error = engine
            .import_json(input, TransactionOrigin::DocumentImport)
            .unwrap_err();
        assert_eq!(engine_audit(&engine), before, "{label}: import audit");

        assert_eq!(
            replacement_code(&replacement_error),
            import_error.code,
            "{label}: replacement must reject with the exact import code \
             (replacement {replacement_error:?}, import {import_error:?})"
        );
    }
}

#[test]
fn custom_root_documents_replace_in_both_modes() {
    let _guard = test_guard();
    let schema_json = serde_json::json!({
        "nodes": [
            { "name": "article", "content": "body+", "role": "doc" },
            {
                "name": "body",
                "content": "inline*",
                "group": "body",
                "role": "textBlock",
                "htmlTag": "section"
            },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    })
    .to_string();
    let before = r#"{"type":"article","content":[{"type":"body","content":[{"type":"text","text":"custom before"}]}]}"#;
    let after = r#"{"type":"article","content":[{"type":"body","content":[{"type":"text","text":"custom after"}]},{"type":"body","content":[{"type":"text","text":"second"}]}]}"#;

    for mode in [
        ReplacementHistory::UndoableBoundary,
        ReplacementHistory::ResetAndClear,
    ] {
        let id = create_local_json_with_options(before, Some(&schema_json), None).unwrap();
        let outcome = write_json(id, 861, after, mode).unwrap();
        assert!(outcome.changed, "{mode:?}");
        assert_eq!(
            get_json(id).unwrap(),
            serde_json::from_str::<serde_json::Value>(after).unwrap(),
            "{mode:?}"
        );
        match mode {
            ReplacementHistory::UndoableBoundary => {
                assert!(can_undo(id).unwrap());
                assert!(undo(id, 862).unwrap());
                assert_eq!(
                    get_json(id).unwrap(),
                    serde_json::from_str::<serde_json::Value>(before).unwrap()
                );
            }
            ReplacementHistory::ResetAndClear => {
                assert!(!can_undo(id).unwrap());
                assert!(!can_redo(id).unwrap());
            }
        }
        destroy_session(id);
    }
}
