use editor_core::boundary::ResourceLimits;
use editor_core::position::PositionMap;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    doc_pos_to_relative_point, relative_point_to_doc_pos, Affinity, DocumentScope,
    DocumentSnapshot, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    RelativePoint, RevisionedPosition, RevisionedRange, SelectionIntent, TransactionOrigin,
    TypedOperation, TypedTransaction, YrsDocumentEngine, YrsEngineConfig,
};
use yrs::updates::decoder::Decode;
use yrs::{ClientID, Doc, OffsetKind, Options, ReadTxn, StateVector, Transact, Update};

const FRAGMENT_NAME: &str = "prosemirror";
const BASE_JSON: &str = r#"{
  "type": "doc",
  "content": [
    {"type": "paragraph", "content": [{"type": "text", "text": "anchor"}]},
    {"type": "paragraph", "content": [{"type": "text", "text": "left"}]},
    {"type": "paragraph", "content": [{"type": "text", "text": "right"}]},
    {"type": "paragraph", "content": [{"type": "text", "text": "tail"}]}
  ]
}"#;

#[derive(Debug, PartialEq)]
struct ConvergedState {
    state_vector: StateVector,
    canonical_json: serde_json::Value,
    canonical_html: String,
    sticky_position: Option<u32>,
}

fn config(mode: InitializationMode) -> YrsEngineConfig {
    YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: FRAGMENT_NAME.into(),
        initialization_mode: mode,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: "convergence-document".into(),
            lineage_id: "convergence-lineage".into(),
        }),
    }
}

fn common_base_snapshot() -> DocumentSnapshot {
    let mut engine = YrsDocumentEngine::new(config(InitializationMode::LocalEmpty)).unwrap();
    engine
        .import_json(BASE_JSON, TransactionOrigin::DocumentImport)
        .unwrap();
    engine.export_snapshot().unwrap()
}

fn replica(snapshot: &DocumentSnapshot) -> YrsDocumentEngine {
    let mut engine = YrsDocumentEngine::new(config(InitializationMode::AwaitRemote)).unwrap();
    engine.restore_snapshot(snapshot).unwrap();
    engine
}

fn block_text_bounds(engine: &YrsDocumentEngine, block_index: usize) -> (u32, u32) {
    let document = engine.document().unwrap();
    let root = document.root();
    let content = root.content().unwrap();
    let block = content.child(block_index).unwrap();
    assert_eq!(block.node_type(), "paragraph");
    let block_start = content
        .iter()
        .take(block_index)
        .map(|node| node.node_size())
        .sum::<u32>()
        + 1;
    let map = PositionMap::build(document, &tiptap_schema());
    (
        map.doc_to_scalar(block_start, document),
        map.doc_to_scalar(block_start + block.content_size(), document),
    )
}

fn scalar_point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

fn apply_local_insert(
    engine: &mut YrsDocumentEngine,
    request_id: u64,
    block_index: usize,
    text: &str,
) {
    let (_, at) = block_text_bounds(engine, block_index);
    let transaction = TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![TypedOperation::InsertText {
            at: scalar_point(at),
            text: text.into(),
            marks: vec![],
        }],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };
    let commit = engine.apply_typed_transaction(transaction).unwrap();
    assert!(commit.changed);
}

fn apply_local_delete_last_scalar(
    engine: &mut YrsDocumentEngine,
    request_id: u64,
    block_index: usize,
) {
    let (start, end) = block_text_bounds(engine, block_index);
    assert!(end > start);
    let transaction = TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: scalar_point(end - 1),
                to: scalar_point(end),
            },
        }],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };
    let commit = engine.apply_typed_transaction(transaction).unwrap();
    assert!(commit.changed);
}

fn utf16_doc(client_id: u64) -> Doc {
    let doc = Doc::with_options(Options {
        client_id: ClientID::new(client_id),
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    });
    doc.get_or_insert_xml_fragment(FRAGMENT_NAME);
    doc
}

fn apply_full_update(doc: &Doc, encoded: &[u8]) {
    doc.transact_mut()
        .apply_update(Update::decode_v1(encoded).unwrap())
        .unwrap();
}

fn raw_doc(encoded: &[u8], client_id: u64) -> Doc {
    let doc = utf16_doc(client_id);
    apply_full_update(&doc, encoded);
    doc
}

fn full_update(doc: &Doc) -> Vec<u8> {
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&StateVector::default())
}

fn state_vector(encoded: &[u8], client_id: u64) -> StateVector {
    let doc = raw_doc(encoded, client_id);
    let txn = doc.transact();
    txn.state_vector()
}

fn delta_from_full_state(encoded: &[u8], base_vector: &StateVector, client_id: u64) -> Vec<u8> {
    let doc = raw_doc(encoded, client_id);
    let txn = doc.transact();
    txn.encode_state_as_update_v1(base_vector)
}

fn merge_base_and_deltas(
    base_update: &[u8],
    deltas: &[Vec<u8>],
    order: &[usize],
    client_id: u64,
) -> Doc {
    let merged = utf16_doc(client_id);
    for &index in order {
        if index == 0 {
            apply_full_update(&merged, base_update);
        } else {
            apply_full_update(&merged, &deltas[index - 1]);
        }
    }
    merged
}

fn sticky_anchor(snapshot: &DocumentSnapshot) -> RelativePoint {
    let doc = raw_doc(&snapshot.encoded_state, 80_001);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment(FRAGMENT_NAME).unwrap();
    doc_pos_to_relative_point(&txn, &fragment, 3, Affinity::Before, &tiptap_schema()).unwrap()
}

fn hydrate(
    base_envelope: &DocumentSnapshot,
    merged: &Doc,
    sticky: &RelativePoint,
) -> ConvergedState {
    let encoded_state = full_update(merged);
    let mut snapshot = base_envelope.clone();
    snapshot.encoded_state = encoded_state;
    let engine = replica(&snapshot);

    let txn = merged.transact();
    let fragment = txn.get_xml_fragment(FRAGMENT_NAME).unwrap();
    let hydrated_doc = raw_doc(&engine.encoded_state().unwrap(), 90_001);
    let hydrated_txn = hydrated_doc.transact();
    let hydrated_state_vector = hydrated_txn.state_vector();
    assert_eq!(hydrated_state_vector, txn.state_vector());
    assert!(!hydrated_state_vector.contains_client(&ClientID::new(engine.client_id())));
    assert!(!hydrated_state_vector.contains_client(&ClientID::new(90_001)));
    assert!(!hydrated_state_vector.contains_client(&merged.client_id()));
    let raw_sticky_position = relative_point_to_doc_pos(&txn, &fragment, sticky, &tiptap_schema());
    let hydrated_fragment = hydrated_txn.get_xml_fragment(FRAGMENT_NAME).unwrap();
    let hydrated_sticky_position =
        relative_point_to_doc_pos(&hydrated_txn, &hydrated_fragment, sticky, &tiptap_schema());
    assert_eq!(hydrated_sticky_position, raw_sticky_position);
    ConvergedState {
        state_vector: hydrated_state_vector,
        canonical_json: engine.document_json().unwrap(),
        canonical_html: engine.document_html().unwrap(),
        sticky_position: hydrated_sticky_position,
    }
}

fn assert_same_states(states: &[ConvergedState]) {
    let expected = states.first().unwrap();
    for actual in &states[1..] {
        assert_eq!(actual, expected);
    }
}

fn assert_contains_clients(state: &ConvergedState, client_ids: &[u64]) {
    for &client_id in client_ids {
        assert!(state
            .state_vector
            .contains_client(&ClientID::new(client_id)));
    }
}

fn assert_excludes_clients(state_vector: &StateVector, client_ids: &[u64]) {
    for &client_id in client_ids {
        assert!(!state_vector.contains_client(&ClientID::new(client_id)));
    }
}

#[test]
fn two_replicas_converge_with_delete_only_delta_pending_before_base_and_a_second_round() {
    let base = common_base_snapshot();
    let sticky = sticky_anchor(&base);
    let base_vector = state_vector(&base.encoded_state, 80_002);
    let mut left = replica(&base);
    let mut right = replica(&base);
    assert_ne!(left.client_id(), right.client_id());
    let first_client_ids = [left.client_id(), right.client_id()];
    assert_excludes_clients(&base_vector, &first_client_ids);

    apply_local_delete_last_scalar(&mut left, 1, 1);
    apply_local_insert(&mut right, 2, 2, "-B1");
    let first_full_states = [
        left.encoded_state().unwrap(),
        right.encoded_state().unwrap(),
    ];
    let first_deltas: [Vec<u8>; 2] = std::array::from_fn(|index| {
        delta_from_full_state(
            &first_full_states[index],
            &base_vector,
            81_000 + index as u64,
        )
    });
    let delete_only = Update::decode_v1(&first_deltas[0]).unwrap();
    assert!(delete_only.state_vector().is_empty());
    assert!(!delete_only.delete_set().is_empty());
    let insert_only = Update::decode_v1(&first_deltas[1]).unwrap();
    assert!(insert_only
        .state_vector()
        .contains_client(&ClientID::new(first_client_ids[1])));

    let first_ab = merge_base_and_deltas(&base.encoded_state, &first_deltas, &[0, 1, 2], 81_101);
    let first_ba_pending =
        merge_base_and_deltas(&base.encoded_state, &first_deltas, &[2, 1, 0], 81_102);
    let first_states = [
        hydrate(&base, &first_ab, &sticky),
        hydrate(&base, &first_ba_pending, &sticky),
    ];
    assert_same_states(&first_states);
    assert!(!first_states[0]
        .state_vector
        .contains_client(&ClientID::new(first_client_ids[0])));
    assert_contains_clients(&first_states[0], &first_client_ids[1..]);
    assert_eq!(first_states[0].sticky_position, Some(3));
    assert_eq!(
        first_states[0].canonical_json["content"][1]["content"][0]["text"],
        "lef"
    );

    let mut round_two_snapshot = base.clone();
    round_two_snapshot.encoded_state = full_update(&first_ab);
    let round_two_base_vector = state_vector(&round_two_snapshot.encoded_state, 82_000);
    let mut left = replica(&round_two_snapshot);
    let mut right = replica(&round_two_snapshot);
    assert_ne!(left.client_id(), right.client_id());
    let second_client_ids = [left.client_id(), right.client_id()];
    assert_excludes_clients(&round_two_base_vector, &second_client_ids);
    apply_local_insert(&mut left, 3, 1, "-A2");
    apply_local_insert(&mut right, 4, 2, "-B2");

    let second_full_states = [
        left.encoded_state().unwrap(),
        right.encoded_state().unwrap(),
    ];
    let second_deltas: [Vec<u8>; 2] = std::array::from_fn(|index| {
        delta_from_full_state(
            &second_full_states[index],
            &round_two_base_vector,
            82_100 + index as u64,
        )
    });
    let second_ab = merge_base_and_deltas(
        &round_two_snapshot.encoded_state,
        &second_deltas,
        &[0, 1, 2],
        82_201,
    );
    let second_ba_pending = merge_base_and_deltas(
        &round_two_snapshot.encoded_state,
        &second_deltas,
        &[2, 1, 0],
        82_202,
    );
    let second_states = [
        hydrate(&round_two_snapshot, &second_ab, &sticky),
        hydrate(&round_two_snapshot, &second_ba_pending, &sticky),
    ];
    assert_same_states(&second_states);
    assert_contains_clients(&second_states[0], &second_client_ids);
    assert_eq!(second_states[0].sticky_position, Some(3));
    assert_eq!(
        second_states[0].canonical_json["content"][1]["content"][0]["text"],
        "lef-A2"
    );
    assert_eq!(
        second_states[0].canonical_json["content"][2]["content"][0]["text"],
        "right-B1-B2"
    );
}

#[test]
fn three_replicas_converge_for_six_delta_orders_with_base_first_or_last() {
    let base = common_base_snapshot();
    let sticky = sticky_anchor(&base);
    let base_vector = state_vector(&base.encoded_state, 83_000);
    let mut replicas = [replica(&base), replica(&base), replica(&base)];
    let client_ids = replicas.each_ref().map(|engine| engine.client_id());
    assert_ne!(client_ids[0], client_ids[1]);
    assert_ne!(client_ids[0], client_ids[2]);
    assert_ne!(client_ids[1], client_ids[2]);
    assert_excludes_clients(&base_vector, &client_ids);

    for (index, engine) in replicas.iter_mut().enumerate() {
        apply_local_insert(
            engine,
            10 + index as u64,
            index + 1,
            ["-A", "-B", "-C"][index],
        );
    }
    let full_states = replicas.map(|engine| engine.encoded_state().unwrap());
    let deltas: [Vec<u8>; 3] = std::array::from_fn(|index| {
        delta_from_full_state(&full_states[index], &base_vector, 83_100 + index as u64)
    });
    let permutations = [
        [0, 1, 2, 3],
        [1, 3, 2, 0],
        [0, 2, 1, 3],
        [2, 3, 1, 0],
        [0, 3, 1, 2],
        [3, 2, 1, 0],
    ];
    let states = permutations
        .iter()
        .enumerate()
        .map(|(index, order)| {
            let merged =
                merge_base_and_deltas(&base.encoded_state, &deltas, order, 83_200 + index as u64);
            hydrate(&base, &merged, &sticky)
        })
        .collect::<Vec<_>>();

    assert_same_states(&states);
    assert_contains_clients(&states[0], &client_ids);
    assert_eq!(states[0].sticky_position, Some(3));
    assert_eq!(
        states[0].canonical_json["content"][1]["content"][0]["text"],
        "left-A"
    );
    assert_eq!(
        states[0].canonical_json["content"][2]["content"][0]["text"],
        "right-B"
    );
    assert_eq!(
        states[0].canonical_json["content"][3]["content"][0]["text"],
        "tail-C"
    );
}
