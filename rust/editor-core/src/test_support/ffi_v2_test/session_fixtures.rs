fn selection_envelope(request_id: u64, base_revision: u64, anchor: u32, head: u32) -> String {
    selection_envelope_with_affinity(request_id, base_revision, anchor, head, "after")
}

fn selection_envelope_with_affinity(
    request_id: u64,
    base_revision: u64,
    anchor: u32,
    head: u32,
    affinity: &str,
) -> String {
    json!({
        "version": 1,
        "requestId": request_id.to_string(),
        "baseDocumentRevision": base_revision.to_string(),
        "selection": {
            "type": "text",
            "anchor": { "offset": anchor, "kind": "scalar", "affinity": affinity },
            "head": { "offset": head, "kind": "scalar", "affinity": affinity },
        },
    })
    .to_string()
}

fn replace_envelope(request_id: u64, base_revision: u64, json_doc: &str, history: &str) -> String {
    json!({
        "version": 1,
        "requestId": request_id.to_string(),
        "baseDocumentRevision": base_revision.to_string(),
        "setJson": serde_json::from_str::<Value>(json_doc).unwrap(),
        "history": history,
    })
    .to_string()
}

fn history_envelope(request_id: u64) -> String {
    json!({ "version": 1, "requestId": request_id.to_string() }).to_string()
}

// Room/snapshot fixtures and the raw-peer idiom (mirrors the protocol suite)

fn snapshot_source() -> DocumentSnapshot {
    let mut source = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: FRAGMENT_NAME.into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: DOCUMENT_ID.into(),
            lineage_id: LINEAGE_ID.into(),
        }),
    })
    .unwrap();
    source
        .import_json(JSON_SEED, TransactionOrigin::DocumentImport)
        .unwrap();
    source.export_snapshot().unwrap()
}

fn snapshot_metadata_json(snapshot: &DocumentSnapshot) -> Value {
    json!({
        "formatVersion": snapshot.format_version,
        "documentId": snapshot.document_id,
        "lineageId": snapshot.lineage_id,
        "fragmentName": snapshot.fragment_name,
        "schemaFingerprint": snapshot.schema_fingerprint,
    })
}

fn room_config(snapshot: Option<&DocumentSnapshot>) -> Value {
    let mut initialization = json!({
        "type": "room",
        "documentId": DOCUMENT_ID,
        "lineageId": LINEAGE_ID,
    });
    if let Some(snapshot) = snapshot {
        initialization["snapshot"] = snapshot_metadata_json(snapshot);
    }
    json!({ "initialization": initialization })
}

struct RawPeer {
    doc: Doc,
}

impl RawPeer {
    fn from_snapshot(snapshot: &DocumentSnapshot) -> Self {
        let peer = Self { doc: Doc::new() };
        peer.apply(&snapshot.encoded_state);
        peer
    }

    fn apply(&self, update: &[u8]) {
        self.doc
            .transact_mut()
            .apply_update(Update::decode_v1(update).unwrap())
            .unwrap();
    }

    fn state_vector_bytes(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    fn diff_for(&self, remote_state_vector: &[u8]) -> Vec<u8> {
        self.doc
            .transact()
            .encode_state_as_update_v1(&StateVector::decode_v1(remote_state_vector).unwrap())
    }

    fn fragment_string(&self) -> String {
        let txn = self.doc.transact();
        txn.get_xml_fragment(FRAGMENT_NAME)
            .expect("peer must hold the configured fragment")
            .get_string(&txn)
    }

    fn push_text(&self, text: &str) {
        let mut txn = self.doc.transact_mut();
        let fragment = txn
            .get_xml_fragment(FRAGMENT_NAME)
            .expect("peer must hold the configured fragment");
        let Some(XmlOut::Element(paragraph)) = fragment.get(&txn, 0) else {
            panic!("seed content must start with a paragraph element");
        };
        let Some(XmlOut::Text(content)) = paragraph.get(&txn, 0) else {
            panic!("seed paragraph must start with a text node");
        };
        content.push(&mut txn, text);
    }
}

fn sync_frame(message: SyncMessage) -> Vec<u8> {
    Message::Sync(message).encode_v1()
}

fn step1_frame(state_vector: &[u8]) -> Vec<u8> {
    sync_frame(SyncMessage::SyncStep1(
        StateVector::decode_v1(state_vector).unwrap(),
    ))
}

fn step2_frame(update: Vec<u8>) -> Vec<u8> {
    sync_frame(SyncMessage::SyncStep2(update))
}

/// The state vector inside a framed Sync Step 1 message.
fn step1_state_vector(step1: &[u8]) -> StateVector {
    match Message::decode_v1(step1).expect("step1 frame must decode") {
        Message::Sync(SyncMessage::SyncStep1(state_vector)) => state_vector,
        other => panic!("expected a Sync Step 1 frame, got {other:?}"),
    }
}

fn assert_frozen_directive(directive: &Value) {
    let object = directive
        .as_object()
        .expect("transport directives must be JSON objects");
    let fields = object.keys().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(
        fields,
        vec![
            "expiredPeers",
            "generationToOpen",
            "nextDeadlineMillis",
            "peersChanged",
            "remoteCommitApplied",
            "renewedLocal",
            "transportState",
        ],
        "directive field set is frozen: {directive:?}"
    );
    assert!(directive["transportState"].is_string(), "{directive:?}");
    assert!(
        directive["generationToOpen"].is_null() || directive["generationToOpen"].is_string(),
        "{directive:?}"
    );
    assert!(
        directive["nextDeadlineMillis"].is_null() || directive["nextDeadlineMillis"].is_string(),
        "{directive:?}"
    );
    assert!(
        directive["remoteCommitApplied"].is_boolean(),
        "{directive:?}"
    );
    assert!(directive["peersChanged"].is_boolean(), "{directive:?}");
    assert!(directive["renewedLocal"].is_boolean(), "{directive:?}");
    assert!(
        directive["expiredPeers"]
            .as_array()
            .is_some_and(|peers| peers.iter().all(Value::is_string)),
        "{directive:?}"
    );
}

fn drive_v2(id: &str, now_millis: u64) -> Value {
    let directive = ok_json(&v2_collab::editor_v2_collaboration_drive(
        id.to_string(),
        now_millis.to_string(),
    ));
    assert_frozen_directive(&directive);
    directive
}

fn open_v2(id: &str, generation: &str, now_millis: u64) -> Value {
    let directive = ok_json(&v2_collab::editor_v2_collaboration_socket_open(
        id.to_string(),
        generation.to_string(),
        now_millis.to_string(),
    ));
    assert_frozen_directive(&directive);
    directive
}

fn receive_v2(id: &str, generation: &str, message: Vec<u8>, now_millis: u64) -> Value {
    let directive = ok_json(&v2_collab::editor_v2_collaboration_receive(
        id.to_string(),
        generation.to_string(),
        message,
        now_millis.to_string(),
    ));
    assert_frozen_directive(&directive);
    directive
}

fn close_v2(
    id: &str,
    generation: &str,
    code: Option<u32>,
    reason: Option<String>,
    now_millis: u64,
) -> Value {
    let directive = ok_json(&v2_collab::editor_v2_collaboration_socket_close(
        id.to_string(),
        generation.to_string(),
        code,
        reason,
        now_millis.to_string(),
    ));
    assert_frozen_directive(&directive);
    directive
}

fn lease_v2(id: &str, generation: &str) -> crate::ffi_v2::types::FfiOutboundLease {
    ok_lease(&v2_collab::editor_v2_collaboration_lease_outbound(
        id.to_string(),
        generation.to_string(),
    ))
}

fn ack_v2(id: &str, generation: &str, lease_id: String) {
    ok_json(&v2_collab::editor_v2_collaboration_ack_outbound(
        id.to_string(),
        generation.to_string(),
        lease_id,
    ));
}

fn assert_empty_lease_v2(id: &str, generation: &str) {
    let result =
        v2_collab::editor_v2_collaboration_lease_outbound(id.to_string(), generation.to_string());
    assert!(result.value.is_none(), "{result:?}");
    assert!(result.empty, "{result:?}");
    assert!(result.error.is_none(), "{result:?}");
}

/// Drive a RoomReady editor to Synchronized through the v2 boundary: open,
/// answer the owed Step 1 with a raw peer's Step 2, and return the live
/// generation.
fn synchronize_v2(id: &str, server: &RawPeer) -> String {
    let directive = drive_v2(id, 0);
    let generation = directive["generationToOpen"]
        .as_str()
        .expect("initial drive returns a generation");
    let opened = open_v2(id, generation, 0);
    assert_eq!(opened["transportState"], "Handshaking", "{opened:?}");
    let step1 = lease_v2(id, generation);
    let step2 = server.diff_for(&step1_state_vector(&step1.frame).encode_v1());
    ack_v2(id, generation, step1.lease_id);
    let outcome = receive_v2(id, generation, step2_frame(step2), 0);
    assert_eq!(outcome["transportState"], "Synchronized", "{outcome:?}");
    assert_eq!(state_of(id)["transportState"], "Synchronized");
    generation.to_string()
}
