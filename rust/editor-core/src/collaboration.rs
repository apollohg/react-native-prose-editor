use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use serde_json::{json, Value};
use yrs::branch::{Branch, BranchPtr};
use yrs::sync::awareness::{Awareness, AwarenessUpdate};
use yrs::sync::protocol::{DefaultProtocol, Message, Protocol, SyncMessage};
use yrs::types::xml::{XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextRef};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::{Encode, Encoder, EncoderV1};
use yrs::{
    Assoc, Doc, GetString, OffsetKind, Options, ReadTxn, StateVector, StickyIndex, Transact,
    Update, WriteTxn,
};

use crate::boundary::{BoundaryError, BoundaryResult, BoundedInput, InputKind, ResourceLimits};
use crate::schema::presets::tiptap_schema;
use crate::schema::Schema;
use crate::serialize::{from_prosemirror_json_with_limits, UnknownTypeMode};
use crate::transform::DocumentValidator;
use crate::yrs_engine::YrsDocumentCodec;

pub type CollaborationSessionId = u64;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static REGISTRY: OnceLock<
    Mutex<HashMap<CollaborationSessionId, Arc<Mutex<CollaborationSession>>>>,
> = OnceLock::new();

fn global_registry(
) -> &'static Mutex<HashMap<CollaborationSessionId, Arc<Mutex<CollaborationSession>>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Default)]
pub struct CollaborationSessionRegistry;

impl CollaborationSessionRegistry {
    pub fn create(config_json: &str) -> CollaborationSessionId {
        Self::create_result(config_json).unwrap_or(0)
    }

    pub fn create_result(config_json: &str) -> BoundaryResult<CollaborationSessionId> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let session = CollaborationSession::try_new(config_json)?;
        let mut map = global_registry()
            .lock()
            .expect("collaboration registry lock poisoned");
        map.insert(id, Arc::new(Mutex::new(session)));
        Ok(id)
    }

    pub fn get(id: CollaborationSessionId) -> Option<Arc<Mutex<CollaborationSession>>> {
        let map = global_registry()
            .lock()
            .expect("collaboration registry lock poisoned");
        map.get(&id).cloned()
    }

    pub fn destroy(id: CollaborationSessionId) {
        let mut map = global_registry()
            .lock()
            .expect("collaboration registry lock poisoned");
        map.remove(&id);
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationPeer {
    pub client_id: u64,
    pub is_local: bool,
    pub state: Value,
}

#[derive(Debug, Clone, PartialEq)]
struct CachedPeerState {
    raw_json: Option<String>,
    parsed_state: Value,
    normalized_state: Value,
    normalized_document_revision: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationResult {
    pub messages: Vec<Vec<u8>>,
    pub document_changed: bool,
    pub document_json: Option<Value>,
    pub peers_changed: bool,
    pub peers: Option<Vec<CollaborationPeer>>,
}

impl CollaborationResult {
    fn empty() -> Self {
        Self {
            messages: Vec::new(),
            document_changed: false,
            document_json: None,
            peers_changed: false,
            peers: None,
        }
    }
}

pub struct CollaborationSession {
    doc: Doc,
    awareness: Awareness,
    fragment_name: String,
    document_root_type: String,
    void_element_tags: HashSet<String>,
    cached_document_json: Value,
    document_revision: u64,
    cached_peers: Vec<CollaborationPeer>,
    cached_peer_states: HashMap<u64, CachedPeerState>,
    peers_revision: u64,
    local_awareness_state: Option<Value>,
    schema: Schema,
    resource_limits: ResourceLimits,
    max_length: Option<u32>,
}

fn empty_document_json(document_root_type: &str) -> Value {
    json!({
        "type": document_root_type,
        "content": [],
    })
}

fn check_binary_input(actual: usize, limit: usize) -> BoundaryResult<()> {
    if actual > limit {
        return Err(BoundaryError::limit("INPUT_LIMIT_EXCEEDED", limit, actual));
    }
    Ok(())
}

fn apply_awareness_snapshot(
    target: &Awareness,
    update: Result<AwarenessUpdate, yrs::sync::awareness::Error>,
) -> BoundaryResult<()> {
    let update =
        update.map_err(|error| BoundaryError::parse("COLLABORATION_APPLY_FAILED", error))?;
    target
        .apply_update(update)
        .map_err(|error| BoundaryError::parse("COLLABORATION_APPLY_FAILED", error))
}

impl CollaborationSession {
    pub fn new(config_json: &str) -> Self {
        Self::try_new(config_json).expect("invalid collaboration session configuration")
    }

    pub fn try_new(config_json: &str) -> BoundaryResult<Self> {
        check_binary_input(config_json.len(), 64 * 1024 * 1024)?;
        let config: Value = serde_json::from_str(config_json)
            .map_err(|error| BoundaryError::parse("CONFIG_PARSE_FAILED", error))?;
        let limits = ResourceLimits::try_from_config(config.get("resourceLimits"))?;
        BoundedInput::new(config_json, InputKind::Config, &limits)?;
        Self::from_config(&config, limits)
    }

    fn from_config(config: &Value, resource_limits: ResourceLimits) -> BoundaryResult<Self> {
        let client_id = config.get("clientId").and_then(Value::as_u64);
        let mut doc_options = client_id.map(Options::with_client_id).unwrap_or_default();
        doc_options.offset_kind = OffsetKind::Utf16;
        let doc = Doc::with_options(doc_options);
        let awareness = Awareness::new(doc.clone());
        let fragment_name = config
            .get("fragmentName")
            .and_then(Value::as_str)
            .unwrap_or("prosemirror")
            .to_string();
        let schema = match config.get("schema") {
            Some(schema_json) => Schema::from_json_with_limits(schema_json, &resource_limits)?,
            None => tiptap_schema(),
        };
        let void_element_tags = void_element_tags_from_schema_with_opaque(&schema);
        let document_root_type = schema.doc_node_type().to_string();
        let max_length = match config.get("maxLength") {
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| {
                        BoundaryError::new(
                            "CONFIG_INVALID",
                            "maxLength must be an unsigned 32-bit integer",
                        )
                    })?,
            ),
            None => None,
        };

        let mut session = Self {
            doc,
            awareness,
            fragment_name,
            cached_document_json: empty_document_json(&document_root_type),
            document_root_type,
            void_element_tags,
            document_revision: 0,
            cached_peers: Vec::new(),
            cached_peer_states: HashMap::new(),
            peers_revision: 0,
            local_awareness_state: None,
            schema,
            resource_limits,
            max_length,
        };

        if let Some(initial_json) = config.get("initialDocumentJson") {
            session.validate_document_json(initial_json)?;
            session.replace_document(initial_json.clone())?;
        } else {
            session.refresh_cached_document_json()?;
        }

        if let Some(local_awareness) = config.get("localAwareness") {
            session.set_local_awareness(local_awareness.clone())?;
        }

        session.refresh_cached_peers();

        Ok(session)
    }

    pub fn resource_limits(&self) -> &ResourceLimits {
        &self.resource_limits
    }

    pub fn encoded_state(&self) -> Vec<u8> {
        self.doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
    }

    pub fn apply_encoded_state(
        &mut self,
        encoded_state: Vec<u8>,
    ) -> BoundaryResult<CollaborationResult> {
        self.commit_encoded_state(encoded_state, false)
    }

    pub fn replace_encoded_state(
        &mut self,
        encoded_state: Vec<u8>,
    ) -> BoundaryResult<CollaborationResult> {
        self.commit_encoded_state(encoded_state, true)
    }

    fn commit_encoded_state(
        &mut self,
        encoded_state: Vec<u8>,
        replace: bool,
    ) -> BoundaryResult<CollaborationResult> {
        let previous_document_revision = self.document_revision;
        let previous_peers_revision = self.peers_revision;
        let previous_document_json = self.cached_document_json.clone();
        let previous_peers = self.cached_peers.clone();
        let mut candidate = self.decode_candidate_state(&encoded_state, replace)?;

        candidate.document_revision = if candidate.cached_document_json == previous_document_json {
            previous_document_revision
        } else {
            previous_document_revision.wrapping_add(1)
        };
        candidate.peers_revision = if candidate.cached_peers == previous_peers {
            previous_peers_revision
        } else {
            previous_peers_revision.wrapping_add(1)
        };

        let messages = if encoded_state.is_empty() {
            Vec::new()
        } else {
            vec![encode_message_bounded(
                Message::Sync(SyncMessage::Update(encoded_state)),
                &candidate.resource_limits,
            )?]
        };
        *self = candidate;
        Ok(self.finish_result(
            previous_document_revision,
            previous_peers_revision,
            messages,
        ))
    }

    fn decode_candidate_state(&self, encoded_state: &[u8], replace: bool) -> BoundaryResult<Self> {
        check_binary_input(
            encoded_state.len(),
            self.resource_limits.max_encoded_state_bytes,
        )?;

        let update = Update::decode_v1(encoded_state)
            .map_err(|error| BoundaryError::parse("COLLABORATION_DECODE_FAILED", error))?;
        let client_id = self.doc.client_id();
        let mut options = Options::with_client_id(client_id);
        options.offset_kind = OffsetKind::Utf16;
        let doc = Doc::with_options(options);
        if !replace {
            let current_state = self.encoded_state();
            let current_update = Update::decode_v1(&current_state)
                .map_err(|error| BoundaryError::parse("COLLABORATION_DECODE_FAILED", error))?;
            doc.transact_mut()
                .apply_update(current_update)
                .map_err(|error| BoundaryError::parse("COLLABORATION_APPLY_FAILED", error))?;
        }
        doc.transact_mut()
            .apply_update(update)
            .map_err(|error| BoundaryError::parse("COLLABORATION_APPLY_FAILED", error))?;

        let awareness = Awareness::new(doc.clone());
        if !replace {
            apply_awareness_snapshot(&awareness, self.awareness.update())?;
        } else if let Some(local_state) = self.local_awareness_state.as_ref() {
            awareness
                .set_local_state(local_state)
                .map_err(|error| BoundaryError::parse("COLLABORATION_APPLY_FAILED", error))?;
        }

        let mut candidate = Self {
            doc,
            awareness,
            fragment_name: self.fragment_name.clone(),
            document_root_type: self.document_root_type.clone(),
            void_element_tags: self.void_element_tags.clone(),
            cached_document_json: empty_document_json(&self.document_root_type),
            document_revision: 0,
            cached_peers: Vec::new(),
            cached_peer_states: HashMap::new(),
            peers_revision: 0,
            local_awareness_state: self.local_awareness_state.clone(),
            schema: self.schema.clone(),
            resource_limits: self.resource_limits.clone(),
            max_length: self.max_length,
        };
        candidate.refresh_cached_document_json()?;
        candidate.refresh_cached_peers();
        candidate.validate_cached_document()?;
        candidate.validate_encoded_state_size()?;
        Ok(candidate)
    }

    fn validate_cached_document(&self) -> BoundaryResult<()> {
        self.validate_document_json(&self.cached_document_json)
    }

    fn validate_encoded_state_size(&self) -> BoundaryResult<()> {
        check_binary_input(
            self.encoded_state().len(),
            self.resource_limits.max_encoded_state_bytes,
        )
    }

    pub(crate) fn validate_document_json(&self, json: &Value) -> BoundaryResult<()> {
        let document = from_prosemirror_json_with_limits(
            json,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(|error| BoundaryError::parse("DOCUMENT_INVALID", error))?;
        DocumentValidator::validate(&document, &self.schema, &self.resource_limits)?;
        if let Some(limit) = self.max_length {
            let actual = document.root().text_content().chars().count();
            if actual > limit as usize {
                return Err(BoundaryError::limit(
                    "MAX_LENGTH_EXCEEDED",
                    limit as usize,
                    actual,
                ));
            }
        }
        Ok(())
    }

    pub fn start(&mut self) -> BoundaryResult<CollaborationResult> {
        let mut result = CollaborationResult::empty();
        result.messages.push(self.encode_sync_step_1()?);
        if let Some(message) = self.encode_local_awareness_message()? {
            result.messages.push(message);
        }
        result.peers_changed = true;
        result.peers = Some(self.cached_peers.clone());
        Ok(result)
    }

    pub fn document_json(&self) -> Value {
        self.cached_document_json.clone()
    }

    pub fn peers(&self) -> Vec<CollaborationPeer> {
        self.cached_peers.clone()
    }

    fn collect_peers(&mut self) -> Vec<CollaborationPeer> {
        let local_id = self.doc.client_id();
        let mut seen_client_ids = HashSet::new();
        let awareness_states = self
            .awareness
            .iter()
            .map(|(client_id, state)| (client_id, state.data.as_ref().map(ToString::to_string)))
            .collect::<Vec<_>>();
        let peers = awareness_states
            .into_iter()
            .map(|(client_id, raw_json)| {
                seen_client_ids.insert(client_id);
                let parsed_state = self.cached_or_normalize_peer_state(client_id, raw_json);
                CollaborationPeer {
                    client_id,
                    is_local: client_id == local_id,
                    state: parsed_state,
                }
            })
            .collect();
        self.cached_peer_states
            .retain(|client_id, _| seen_client_ids.contains(client_id));
        peers
    }

    pub fn apply_local_document(
        &mut self,
        next_json: Value,
    ) -> BoundaryResult<CollaborationResult> {
        let previous_peers_revision = self.peers_revision;
        let previous_document_revision = self.document_revision;
        let previous_document_json = self.cached_document_json.clone();
        let previous_peers = self.cached_peers.clone();
        let mut candidate = self.clone_candidate()?;
        let previous_state_vector = candidate.doc.transact().state_vector();

        {
            let codec = YrsDocumentCodec::new(&candidate.schema, &candidate.resource_limits);
            let mut txn = candidate.doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment(candidate.fragment_name.as_str());
            codec
                .apply_json(
                    &fragment,
                    &mut txn,
                    &candidate.cached_document_json,
                    &next_json,
                )
                .map_err(BoundaryError::from)?;
        }

        candidate.refresh_cached_document_json()?;
        candidate.refresh_cached_peers();
        candidate.validate_cached_document()?;
        candidate.validate_encoded_state_size()?;
        let update = candidate
            .doc
            .transact()
            .encode_diff_v1(&previous_state_vector);
        let messages = if update.is_empty() {
            Vec::new()
        } else {
            vec![encode_message_bounded(
                Message::Sync(SyncMessage::Update(update)),
                &candidate.resource_limits,
            )?]
        };
        candidate.document_revision = if candidate.cached_document_json == previous_document_json {
            previous_document_revision
        } else {
            previous_document_revision.wrapping_add(1)
        };
        candidate.peers_revision = if candidate.cached_peers == previous_peers {
            previous_peers_revision
        } else {
            previous_peers_revision.wrapping_add(1)
        };
        *self = candidate;
        Ok(self.finish_result(
            previous_document_revision,
            previous_peers_revision,
            messages,
        ))
    }

    pub fn handle_message(&mut self, message: Vec<u8>) -> BoundaryResult<CollaborationResult> {
        check_binary_input(
            message.len(),
            self.resource_limits.max_collaboration_message_bytes,
        )?;
        let previous_document_revision = self.document_revision;
        let previous_peers_revision = self.peers_revision;
        let refresh_scope = refresh_scope_for_message(message.as_slice());
        let previous_document_json = self.cached_document_json.clone();
        let previous_peers = self.cached_peers.clone();
        let mut candidate = self.clone_candidate()?;

        let protocol = DefaultProtocol;
        let mut responses = Vec::new();
        let replies = protocol
            .handle(&candidate.awareness, &message)
            .map_err(|error| BoundaryError::parse("COLLABORATION_DECODE_FAILED", error))?;
        for reply in replies {
            responses.push(encode_message_bounded(reply, &candidate.resource_limits)?);
        }

        if refresh_scope.document {
            candidate.refresh_cached_document_json()?;
            candidate.validate_cached_document()?;
            candidate.validate_encoded_state_size()?;
        }
        if refresh_scope.peers || refresh_scope.document {
            candidate.refresh_cached_peers();
        }

        candidate.document_revision = if candidate.cached_document_json == previous_document_json {
            previous_document_revision
        } else {
            previous_document_revision.wrapping_add(1)
        };
        candidate.peers_revision = if candidate.cached_peers == previous_peers {
            previous_peers_revision
        } else {
            previous_peers_revision.wrapping_add(1)
        };
        *self = candidate;

        Ok(self.finish_result(
            previous_document_revision,
            previous_peers_revision,
            responses,
        ))
    }

    fn clone_candidate(&self) -> BoundaryResult<Self> {
        let client_id = self.doc.client_id();
        let mut options = Options::with_client_id(client_id);
        options.offset_kind = OffsetKind::Utf16;
        let doc = Doc::with_options(options);
        let current_state = self.encoded_state();
        let current_update = Update::decode_v1(&current_state)
            .map_err(|error| BoundaryError::parse("COLLABORATION_DECODE_FAILED", error))?;
        doc.transact_mut()
            .apply_update(current_update)
            .map_err(|error| BoundaryError::parse("COLLABORATION_APPLY_FAILED", error))?;
        let awareness = Awareness::new(doc.clone());
        apply_awareness_snapshot(&awareness, self.awareness.update())?;

        let mut candidate = Self {
            doc,
            awareness,
            fragment_name: self.fragment_name.clone(),
            document_root_type: self.document_root_type.clone(),
            void_element_tags: self.void_element_tags.clone(),
            cached_document_json: empty_document_json(&self.document_root_type),
            document_revision: 0,
            cached_peers: Vec::new(),
            cached_peer_states: HashMap::new(),
            peers_revision: 0,
            local_awareness_state: self.local_awareness_state.clone(),
            schema: self.schema.clone(),
            resource_limits: self.resource_limits.clone(),
            max_length: self.max_length,
        };
        candidate.refresh_cached_document_json()?;
        candidate.refresh_cached_peers();
        Ok(candidate)
    }

    pub fn set_local_awareness(
        &mut self,
        next_state: Value,
    ) -> BoundaryResult<CollaborationResult> {
        let previous_document_revision = self.document_revision;
        let previous_peers_revision = self.peers_revision;
        let mut candidate = self.clone_candidate()?;
        let next_state = self.normalize_awareness_state(next_state);
        candidate.local_awareness_state = Some(next_state.clone());
        candidate
            .awareness
            .set_local_state(&next_state)
            .map_err(|error| BoundaryError::parse("COLLABORATION_APPLY_FAILED", error))?;
        candidate.refresh_cached_peers();

        let messages = candidate
            .encode_local_awareness_message()?
            .into_iter()
            .collect::<Vec<_>>();
        candidate.peers_revision = if candidate.cached_peers == self.cached_peers {
            previous_peers_revision
        } else {
            previous_peers_revision.wrapping_add(1)
        };
        *self = candidate;
        Ok(self.finish_result(
            previous_document_revision,
            previous_peers_revision,
            messages,
        ))
    }

    pub fn clear_local_awareness(&mut self) -> BoundaryResult<CollaborationResult> {
        let previous_document_revision = self.document_revision;
        let previous_peers_revision = self.peers_revision;
        let mut candidate = self.clone_candidate()?;
        let had_local_awareness = candidate.awareness.local_state_raw().is_some();
        candidate.local_awareness_state = None;
        candidate.awareness.remove_state(candidate.doc.client_id());
        candidate.refresh_cached_peers();
        let messages = if had_local_awareness {
            let update = candidate
                .awareness
                .update_with_clients([candidate.doc.client_id()])
                .map_err(|error| BoundaryError::parse("COLLABORATION_APPLY_FAILED", error))?;
            vec![encode_message_bounded(
                Message::Awareness(update),
                &candidate.resource_limits,
            )?]
        } else {
            Vec::new()
        };

        candidate.peers_revision = if candidate.cached_peers == self.cached_peers {
            previous_peers_revision
        } else {
            previous_peers_revision.wrapping_add(1)
        };
        *self = candidate;
        Ok(self.finish_result(
            previous_document_revision,
            previous_peers_revision,
            messages,
        ))
    }

    fn replace_document(&mut self, next_json: Value) -> BoundaryResult<()> {
        let codec = YrsDocumentCodec::new(&self.schema, &self.resource_limits);
        let mut txn = self.doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment(self.fragment_name.as_str());
        let len = fragment.len(&txn);
        if len > 0 {
            fragment.remove_range(&mut txn, 0, len);
        }
        codec
            .apply_json(
                &fragment,
                &mut txn,
                &empty_document_json(&self.document_root_type),
                &next_json,
            )
            .map_err(BoundaryError::from)?;
        drop(txn);
        self.refresh_cached_document_json()?;
        self.refresh_cached_peers();
        self.validate_cached_document()?;
        self.validate_encoded_state_size()?;
        Ok(())
    }

    fn encode_sync_step_1(&self) -> BoundaryResult<Vec<u8>> {
        let state_vector = self.doc.transact().state_vector();
        encode_message_bounded(
            Message::Sync(SyncMessage::SyncStep1(state_vector)),
            &self.resource_limits,
        )
    }

    fn encode_local_awareness_message(&self) -> BoundaryResult<Option<Vec<u8>>> {
        if self.local_awareness_state.is_none() {
            return Ok(None);
        }
        let update = self
            .awareness
            .update()
            .map_err(|error| BoundaryError::parse("COLLABORATION_APPLY_FAILED", error))?;
        Ok(Some(encode_message_bounded(
            Message::Awareness(update),
            &self.resource_limits,
        )?))
    }

    fn refresh_cached_document_json(&mut self) -> BoundaryResult<bool> {
        let txn = self.doc.transact();
        let codec = YrsDocumentCodec::new(&self.schema, &self.resource_limits);
        let next_document_json = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .map(|fragment| codec.read_json(&fragment, &txn))
            .transpose()?
            .unwrap_or_else(|| empty_document_json(&self.document_root_type));
        if next_document_json == self.cached_document_json {
            return Ok(false);
        }
        self.cached_document_json = next_document_json;
        self.document_revision = self.document_revision.wrapping_add(1);
        Ok(true)
    }

    fn refresh_cached_peers(&mut self) -> bool {
        let next_peers = self.collect_peers();
        if next_peers == self.cached_peers {
            return false;
        }
        self.cached_peers = next_peers;
        self.peers_revision = self.peers_revision.wrapping_add(1);
        true
    }

    fn cached_or_normalize_peer_state(
        &mut self,
        client_id: u64,
        raw_json: Option<String>,
    ) -> Value {
        if let Some(cached) = self.cached_peer_states.get(&client_id) {
            if cached.raw_json == raw_json {
                if cached.normalized_document_revision == self.document_revision {
                    return cached.normalized_state.clone();
                }

                let parsed_state = cached.parsed_state.clone();
                let normalized_state = self.normalize_awareness_state(parsed_state.clone());
                self.cached_peer_states.insert(
                    client_id,
                    CachedPeerState {
                        raw_json,
                        parsed_state,
                        normalized_state: normalized_state.clone(),
                        normalized_document_revision: self.document_revision,
                    },
                );
                return normalized_state;
            }
        }

        let parsed_state = raw_json
            .as_ref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or(Value::Null);
        let normalized_state = self.normalize_awareness_state(parsed_state.clone());
        self.cached_peer_states.insert(
            client_id,
            CachedPeerState {
                raw_json,
                parsed_state,
                normalized_state: normalized_state.clone(),
                normalized_document_revision: self.document_revision,
            },
        );
        normalized_state
    }

    fn finish_result(
        &self,
        previous_document_revision: u64,
        previous_peers_revision: u64,
        messages: Vec<Vec<u8>>,
    ) -> CollaborationResult {
        let mut result = CollaborationResult::empty();
        result.messages = messages;
        if self.document_revision != previous_document_revision {
            result.document_changed = true;
            result.document_json = Some(self.cached_document_json.clone());
        }
        if self.peers_revision != previous_peers_revision {
            result.peers_changed = true;
            result.peers = Some(self.cached_peers.clone());
        }
        result
    }

    fn normalize_awareness_state(&self, state: Value) -> Value {
        let mut object = match state {
            Value::Object(object) => object,
            other => return other,
        };

        if let Some(selection) = self.selection_from_cursor_value(object.get("cursor")) {
            object.insert("selection".to_string(), selection);
        }

        if !object.contains_key("cursor") {
            if let Some(cursor) = self.cursor_from_selection_value(object.get("selection")) {
                object.insert("cursor".to_string(), cursor);
            }
        }

        Value::Object(object)
    }

    fn selection_from_cursor_value(&self, cursor: Option<&Value>) -> Option<Value> {
        let cursor = cursor?.as_object()?;
        let anchor = self.sticky_index_value_to_doc_pos(cursor.get("anchor")?)?;
        let head = self.sticky_index_value_to_doc_pos(cursor.get("head")?)?;
        Some(json!({
            "anchor": anchor,
            "head": head,
        }))
    }

    fn cursor_from_selection_value(&self, selection: Option<&Value>) -> Option<Value> {
        let selection = selection?.as_object()?;
        let anchor = selection.get("anchor")?.as_u64()? as u32;
        let head = selection.get("head")?.as_u64()? as u32;
        let txn = self.doc.transact();
        let fragment = txn.get_xml_fragment(self.fragment_name.as_str())?;
        let anchor_cursor = cursor_sticky_index_from_doc_pos(
            &txn,
            &fragment,
            anchor,
            anchor == head,
            &self.void_element_tags,
        )?;
        let head_cursor = cursor_sticky_index_from_doc_pos(
            &txn,
            &fragment,
            head,
            anchor == head,
            &self.void_element_tags,
        )?;
        Some(json!({
            "anchor": anchor_cursor,
            "head": head_cursor,
        }))
    }

    fn sticky_index_value_to_doc_pos(&self, value: &Value) -> Option<u32> {
        let sticky_index: StickyIndex = serde_json::from_value(value.clone()).ok()?;
        let txn = self.doc.transact();
        let fragment = txn.get_xml_fragment(self.fragment_name.as_str())?;
        sticky_index_to_doc_pos(&txn, &fragment, &sticky_index, &self.void_element_tags)
    }
}

fn sticky_index_to_doc_pos<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    sticky_index: &StickyIndex,
    void_element_tags: &HashSet<String>,
) -> Option<u32> {
    let offset = sticky_index.get_offset(txn)?;
    let root_branch = BranchPtr::from(<XmlFragmentRef as AsRef<Branch>>::as_ref(fragment));
    if offset.branch == root_branch {
        return sequence_branch_index_to_doc_pos(
            txn,
            fragment.children(txn),
            offset.index,
            void_element_tags,
        );
    }
    let mut child_start = 0u32;
    for child in fragment.children(txn) {
        if let Some(position) = sticky_index_to_doc_pos_in_node(
            txn,
            &child,
            offset.branch,
            offset.index,
            child_start,
            void_element_tags,
        ) {
            return Some(position);
        }
        child_start += xml_out_pm_size(txn, &child, void_element_tags);
    }
    None
}

fn sticky_index_to_doc_pos_in_node<T: ReadTxn>(
    txn: &T,
    node: &XmlOut,
    target_branch: BranchPtr,
    target_index: u32,
    node_start: u32,
    void_element_tags: &HashSet<String>,
) -> Option<u32> {
    match node {
        XmlOut::Text(text) => {
            let text_branch = BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(text));
            if text_branch == target_branch {
                let text_value = text.get_string(txn);
                let scalar_offset = utf16_offset_to_scalar(&text_value, target_index)?;
                return Some(node_start + scalar_offset);
            }
            None
        }
        XmlOut::Element(element) => {
            let element_branch = BranchPtr::from(<XmlElementRef as AsRef<Branch>>::as_ref(element));
            if element_branch == target_branch {
                let content_start = node_start + 1;
                return sequence_branch_index_to_doc_pos(
                    txn,
                    element.children(txn),
                    target_index,
                    void_element_tags,
                )
                .map(|value| content_start + value);
            }

            let mut child_start = node_start + 1;
            for child in element.children(txn) {
                if let Some(position) = sticky_index_to_doc_pos_in_node(
                    txn,
                    &child,
                    target_branch,
                    target_index,
                    child_start,
                    void_element_tags,
                ) {
                    return Some(position);
                }
                child_start += xml_out_pm_size(txn, &child, void_element_tags);
            }
            None
        }
        XmlOut::Fragment(fragment) => {
            let fragment_branch =
                BranchPtr::from(<XmlFragmentRef as AsRef<Branch>>::as_ref(fragment));
            if fragment_branch == target_branch {
                return sequence_branch_index_to_doc_pos(
                    txn,
                    fragment.children(txn),
                    target_index,
                    void_element_tags,
                )
                .map(|value| node_start + value);
            }

            let mut child_start = node_start;
            for child in fragment.children(txn) {
                if let Some(position) = sticky_index_to_doc_pos_in_node(
                    txn,
                    &child,
                    target_branch,
                    target_index,
                    child_start,
                    void_element_tags,
                ) {
                    return Some(position);
                }
                child_start += xml_out_pm_size(txn, &child, void_element_tags);
            }
            None
        }
    }
}

fn sequence_branch_index_to_doc_pos<'a, T: ReadTxn>(
    txn: &T,
    children: impl Iterator<Item = XmlOut> + 'a,
    target_index: u32,
    void_element_tags: &HashSet<String>,
) -> Option<u32> {
    let mut branch_index = 0u32;
    let mut doc_pos = 0u32;

    for child in children {
        match &child {
            XmlOut::Text(text) => {
                let text_value = text.get_string(txn);
                let text_scalar_len = scalar_len(&text_value);
                if target_index == branch_index {
                    return Some(doc_pos);
                }
                branch_index += 1;
                doc_pos += text_scalar_len;
                if target_index == branch_index {
                    return Some(doc_pos);
                }
            }
            XmlOut::Element(_) | XmlOut::Fragment(_) => {
                if target_index == branch_index {
                    return Some(doc_pos);
                }
                branch_index += 1;
                doc_pos += xml_out_pm_size(txn, &child, void_element_tags);
                if target_index == branch_index {
                    return Some(doc_pos);
                }
            }
        }
    }

    if target_index == branch_index {
        Some(doc_pos)
    } else {
        None
    }
}

fn doc_pos_to_sticky_index<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    doc_pos: u32,
    assoc: Assoc,
    void_element_tags: &HashSet<String>,
) -> Option<StickyIndex> {
    let content_size = xml_fragment_pm_content_size(txn, fragment, void_element_tags);
    if doc_pos > content_size {
        return None;
    }
    doc_pos_to_sticky_index_in_sequence(
        txn,
        fragment.children(txn),
        doc_pos,
        assoc,
        BranchPtr::from(<XmlFragmentRef as AsRef<Branch>>::as_ref(fragment)),
        void_element_tags,
    )
}

fn cursor_sticky_index_from_doc_pos<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    doc_pos: u32,
    collapsed: bool,
    void_element_tags: &HashSet<String>,
) -> Option<StickyIndex> {
    if !collapsed {
        return doc_pos_to_sticky_index(txn, fragment, doc_pos, Assoc::Before, void_element_tags);
    }

    doc_pos_to_sticky_index(txn, fragment, doc_pos, Assoc::After, void_element_tags).or_else(|| {
        doc_pos_to_sticky_index(txn, fragment, doc_pos, Assoc::Before, void_element_tags)
    })
}

fn doc_pos_to_sticky_index_in_sequence<'a, T: ReadTxn>(
    txn: &T,
    children: impl Iterator<Item = XmlOut> + 'a,
    doc_pos: u32,
    assoc: Assoc,
    branch: BranchPtr,
    void_element_tags: &HashSet<String>,
) -> Option<StickyIndex> {
    let mut branch_index = 0u32;
    let mut consumed_pm = 0u32;

    for child in children {
        match &child {
            XmlOut::Text(text) => {
                let text_value = text.get_string(txn);
                let text_scalar_len = scalar_len(&text_value);
                if doc_pos <= consumed_pm + text_scalar_len {
                    let utf16_offset = scalar_offset_to_utf16(&text_value, doc_pos - consumed_pm)?;
                    return StickyIndex::at(
                        txn,
                        BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(text)),
                        utf16_offset,
                        assoc,
                    );
                }
                branch_index += 1;
                consumed_pm += text_scalar_len;
            }
            XmlOut::Element(element) => {
                let child_size = xml_out_pm_size(txn, &child, void_element_tags);
                if doc_pos == consumed_pm {
                    return StickyIndex::at(txn, branch, branch_index, assoc);
                }
                if doc_pos < consumed_pm + child_size {
                    return doc_pos_to_sticky_index_in_sequence(
                        txn,
                        element.children(txn),
                        doc_pos - consumed_pm - 1,
                        assoc,
                        BranchPtr::from(<XmlElementRef as AsRef<Branch>>::as_ref(element)),
                        void_element_tags,
                    );
                }
                branch_index += 1;
                consumed_pm += child_size;
            }
            XmlOut::Fragment(nested) => {
                let child_size = xml_out_pm_size(txn, &child, void_element_tags);
                if doc_pos == consumed_pm {
                    return StickyIndex::at(txn, branch, branch_index, assoc);
                }
                if doc_pos < consumed_pm + child_size {
                    return doc_pos_to_sticky_index_in_sequence(
                        txn,
                        nested.children(txn),
                        doc_pos - consumed_pm,
                        assoc,
                        BranchPtr::from(<XmlFragmentRef as AsRef<Branch>>::as_ref(nested)),
                        void_element_tags,
                    );
                }
                branch_index += 1;
                consumed_pm += child_size;
            }
        }
    }

    if doc_pos == consumed_pm {
        StickyIndex::at(txn, branch, branch_index, assoc)
    } else {
        None
    }
}

fn xml_fragment_pm_content_size<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    void_element_tags: &HashSet<String>,
) -> u32 {
    fragment
        .children(txn)
        .map(|child| xml_out_pm_size(txn, &child, void_element_tags))
        .sum()
}

fn void_element_tags_from_schema(schema: &Schema) -> HashSet<String> {
    schema
        .all_nodes()
        .filter(|spec| spec.is_void)
        .map(|spec| spec.name.clone())
        .collect()
}

fn void_element_tags_from_schema_with_opaque(schema: &Schema) -> HashSet<String> {
    let mut tags = void_element_tags_from_schema(schema);
    tags.extend(
        ["__opaque", "__opaque_json", "__skip"]
            .into_iter()
            .map(str::to_string),
    );
    tags
}

fn is_void_element_tag(tag: &str, void_element_tags: &HashSet<String>) -> bool {
    void_element_tags.contains(tag)
}

fn xml_out_pm_size<T: ReadTxn>(txn: &T, node: &XmlOut, void_element_tags: &HashSet<String>) -> u32 {
    match node {
        XmlOut::Text(text) => text.get_string(txn).chars().count() as u32,
        XmlOut::Element(element) => {
            if is_void_element_tag(element.tag(), void_element_tags) {
                1
            } else {
                2 + element
                    .children(txn)
                    .map(|child| xml_out_pm_size(txn, &child, void_element_tags))
                    .sum::<u32>()
            }
        }
        XmlOut::Fragment(fragment) => fragment
            .children(txn)
            .map(|child| xml_out_pm_size(txn, &child, void_element_tags))
            .sum(),
    }
}

fn scalar_len(value: &str) -> u32 {
    value.chars().count() as u32
}

fn scalar_offset_to_utf16(value: &str, scalar_offset: u32) -> Option<u32> {
    let mut scalar_count = 0u32;
    let mut utf16_count = 0u32;
    if scalar_offset == 0 {
        return Some(0);
    }
    for character in value.chars() {
        scalar_count += 1;
        utf16_count += character.len_utf16() as u32;
        if scalar_count == scalar_offset {
            return Some(utf16_count);
        }
    }
    None
}

fn utf16_offset_to_scalar(value: &str, utf16_offset: u32) -> Option<u32> {
    let mut scalar_count = 0u32;
    let mut utf16_count = 0u32;
    if utf16_offset == 0 {
        return Some(0);
    }
    for character in value.chars() {
        scalar_count += 1;
        utf16_count += character.len_utf16() as u32;
        if utf16_count == utf16_offset {
            return Some(scalar_count);
        }
        if utf16_count > utf16_offset {
            return None;
        }
    }
    None
}

#[derive(Clone, Copy)]
struct RefreshScope {
    document: bool,
    peers: bool,
}

fn refresh_scope_for_message(message: &[u8]) -> RefreshScope {
    let decoded: Result<Message, _> = Decode::decode_v1(message);
    match decoded {
        Ok(Message::Awareness(_)) => RefreshScope {
            document: false,
            peers: true,
        },
        Ok(Message::Sync(SyncMessage::SyncStep1(_))) => RefreshScope {
            document: false,
            peers: false,
        },
        Ok(Message::Sync(SyncMessage::SyncStep2(_)))
        | Ok(Message::Sync(SyncMessage::Update(_))) => RefreshScope {
            document: true,
            peers: true,
        },
        _ => RefreshScope {
            document: true,
            peers: true,
        },
    }
}

fn encode_message(message: Message) -> Vec<u8> {
    let mut encoder = EncoderV1::new();
    message.encode(&mut encoder);
    encoder.to_vec()
}

fn encode_message_bounded(message: Message, limits: &ResourceLimits) -> BoundaryResult<Vec<u8>> {
    let encoded = encode_message(message);
    check_binary_input(encoded.len(), limits.max_collaboration_message_bytes)?;
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use yrs::any::Any;
    use yrs::types::xml::{XmlElementPrelim, XmlTextPrelim};
    use yrs::types::{Attrs, ToJson};
    use yrs::{IndexedSequence, Text, TransactionMut, Xml};

    fn xml_fragment_to_document_json<T: ReadTxn>(
        fragment: &XmlFragmentRef,
        txn: &T,
        document_root_type: &str,
    ) -> Value {
        let schema = tiptap_schema();
        assert_eq!(schema.doc_node_type(), document_root_type);
        YrsDocumentCodec::new(&schema, &ResourceLimits::default())
            .read_json(fragment, txn)
            .expect("test collaboration document should be within default limits")
    }

    fn insert_json_node(
        fragment: &XmlFragmentRef,
        txn: &mut TransactionMut<'_>,
        index: u32,
        node: &Value,
    ) {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let codec = YrsDocumentCodec::new(&schema, &limits);
        let current = codec
            .read_json(fragment, txn)
            .expect("test collaboration document should be within default limits");
        let mut next = current.clone();
        next.get_mut("content")
            .and_then(Value::as_array_mut)
            .expect("codec document should have content")
            .insert(index as usize, node.clone());
        codec
            .apply_json(fragment, txn, &current, &next)
            .expect("test collaboration document should be within default limits");
    }

    fn normalize_marks(marks: Option<&Vec<Value>>) -> Vec<Value> {
        let mut normalized = marks.cloned().unwrap_or_default();
        normalized.sort_by(|left, right| {
            let left_name = left.get("type").and_then(Value::as_str).unwrap_or("");
            let right_name = right.get("type").and_then(Value::as_str).unwrap_or("");
            left_name.cmp(right_name)
        });
        normalized
    }

    fn apply_messages_to_peer(peer: &Awareness, messages: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        let protocol = DefaultProtocol;
        let mut responses = Vec::new();
        for message in messages {
            let replies = protocol.handle(peer, &message).unwrap();
            responses.extend(replies.into_iter().map(encode_message));
        }
        responses
    }

    #[test]
    fn collaboration_session_preserves_empty_document_json_for_custom_schema() {
        let session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "schema": {
                    "nodes": [
                        { "name": "doc", "content": "title? block*", "role": "doc" },
                        { "name": "title", "content": "inline*", "group": "block", "role": "textBlock" },
                        { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock" },
                        { "name": "text", "content": "", "group": "inline", "role": "text" }
                    ],
                    "marks": []
                }
            })
            .to_string(),
        );

        assert_eq!(session.document_json(), empty_document_json("doc"));
    }

    #[test]
    fn collaboration_session_preserves_custom_doc_role_through_encoded_state() {
        let schema = json!({
            "nodes": [
                { "name": "article", "content": "title+", "role": "doc" },
                { "name": "title", "content": "inline*", "group": "block", "role": "textBlock" },
                { "name": "text", "content": "", "group": "inline", "role": "text" }
            ],
            "marks": []
        });
        let initial = json!({
            "type": "article",
            "content": [{ "type": "title", "content": [{ "type": "text", "text": "Hello" }] }]
        });
        let source = CollaborationSession::new(
            &json!({
                "clientId": 41,
                "schema": schema.clone(),
                "initialDocumentJson": initial.clone(),
            })
            .to_string(),
        );
        assert_eq!(source.document_json(), initial);

        let mut restored = CollaborationSession::new(
            &json!({
                "clientId": 42,
                "schema": schema.clone(),
            })
            .to_string(),
        );
        restored
            .apply_encoded_state(source.encoded_state())
            .unwrap();
        assert_eq!(restored.document_json(), initial);

        let parsed_schema = Schema::from_json(&schema).unwrap();
        let mut editor = crate::editor::Editor::new(
            parsed_schema,
            crate::intercept::InterceptorPipeline::new(),
            false,
        );
        assert!(editor.set_json(&restored.document_json()).is_ok());
        assert_eq!(editor.get_json(), initial);
    }

    fn peer_document_json(peer: &Awareness) -> Value {
        let txn = peer.doc().transact();
        txn.get_xml_fragment("prosemirror")
            .map(|fragment| xml_fragment_to_document_json(&fragment, &txn, "doc"))
            .unwrap_or_else(|| {
                json!({
                    "type": "doc",
                    "content": [],
                })
            })
    }

    fn custom_void_schema_json(void_name: &str) -> Value {
        json!({
            "nodes": [
                {
                    "name": "doc",
                    "content": "block+",
                    "role": "doc"
                },
                {
                    "name": "paragraph",
                    "content": "inline*",
                    "group": "block",
                    "role": "textBlock",
                    "htmlTag": "p"
                },
                {
                    "name": void_name,
                    "content": "",
                    "group": "block",
                    "role": "block",
                    "isVoid": true
                },
                {
                    "name": "text",
                    "content": "",
                    "group": "inline",
                    "role": "text"
                }
            ],
            "marks": []
        })
    }

    #[test]
    fn collaboration_session_normalizes_standard_heading_elements_from_peer_documents() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default"
            })
            .to_string(),
        );
        let mut peer = Awareness::new(Doc::with_client_id(2));

        let update = {
            let mut txn = peer.doc_mut().transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("default");
            let heading = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("heading"));
            heading.insert_attribute(&mut txn, "level", Any::BigInt(2));

            let mut marks = Attrs::default();
            marks.insert("bold".into(), Any::Bool(true));
            marks.insert(
                "link".into(),
                Any::Map(Arc::new(
                    [(
                        "href".to_string(),
                        Any::String("https://example.com".to_string().into()),
                    )]
                    .into_iter()
                    .collect(),
                )),
            );

            let text = heading.insert(&mut txn, 0, XmlTextPrelim::new(""));
            text.insert_with_attributes(&mut txn, 0, "Heading", marks);
            txn.encode_update_v1()
        };

        session
            .handle_message(encode_message(Message::Sync(SyncMessage::Update(update))))
            .expect("session should accept peer document");

        let document = session.document_json();
        let heading = document
            .get("content")
            .and_then(Value::as_array)
            .and_then(|content| content.first())
            .expect("expected heading block");
        assert_eq!(heading.get("type").and_then(Value::as_str), Some("h2"));

        let text = heading
            .get("content")
            .and_then(Value::as_array)
            .and_then(|content| content.first())
            .expect("expected heading text");
        assert_eq!(text.get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(text.get("text").and_then(Value::as_str), Some("Heading"));

        let marks = normalize_marks(text.get("marks").and_then(Value::as_array));
        assert_eq!(
            marks,
            vec![
                json!({ "type": "bold" }),
                json!({ "type": "link", "attrs": { "href": "https://example.com" } }),
            ]
        );
    }

    #[test]
    fn collaboration_session_normalizes_float_backed_heading_levels_from_peer_documents() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default"
            })
            .to_string(),
        );
        let mut peer = Awareness::new(Doc::with_client_id(2));

        let update = {
            let mut txn = peer.doc_mut().transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("default");
            let heading = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("heading"));
            heading.insert_attribute(&mut txn, "level", Any::Number(2.0));
            let text = heading.insert(&mut txn, 0, XmlTextPrelim::new(""));
            text.insert(&mut txn, 0, "Heading");
            txn.encode_update_v1()
        };

        session
            .handle_message(encode_message(Message::Sync(SyncMessage::Update(update))))
            .expect("session should accept peer document");

        assert_eq!(
            session.document_json(),
            json!({
                "type": "doc",
                "content": [{
                    "type": "h2",
                    "content": [{
                        "type": "text",
                        "text": "Heading"
                    }]
                }]
            })
        );
    }

    #[test]
    fn collaboration_session_writes_internal_heading_nodes_as_standard_heading_elements() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default"
            })
            .to_string(),
        );
        let peer = Awareness::new(Doc::with_client_id(2));

        let result = session
            .apply_local_document(json!({
                "type": "doc",
                "content": [{
                    "type": "h2",
                    "content": [{
                        "type": "text",
                        "text": "Heading",
                        "marks": [{ "type": "bold" }]
                    }]
                }]
            }))
            .unwrap();
        let _ = apply_messages_to_peer(&peer, result.messages);

        let txn = peer.doc().transact();
        let fragment = txn
            .get_xml_fragment("default")
            .expect("peer fragment should exist");
        let XmlOut::Element(heading) = fragment.get(&txn, 0).expect("heading should exist") else {
            panic!("expected heading element");
        };

        assert_eq!(heading.tag().as_ref(), "heading");
        assert!(matches!(
            heading
                .get_attribute(&txn, "level")
                .map(|value| value.to_json(&txn)),
            Some(Any::BigInt(2))
        ));
    }

    #[test]
    fn collaboration_sessions_round_trip_document_updates() {
        let mut left = CollaborationSession::new(
            r#"{"clientId":1,"initialDocumentJson":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"hello"}]}]}}"#,
        );
        let mut right = CollaborationSession::new(r#"{"clientId":2}"#);

        for message in left.start().unwrap().messages {
            let reply = right.handle_message(message).unwrap();
            for response in reply.messages {
                let _ = left.handle_message(response).unwrap();
            }
        }
        for message in right.start().unwrap().messages {
            let reply = left.handle_message(message).unwrap();
            for response in reply.messages {
                let _ = right.handle_message(response).unwrap();
            }
        }

        assert_eq!(
            right.document_json(),
            json!({
                "type": "doc",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "hello" }
                        ]
                    }
                ]
            })
        );

        let local = json!({
            "type": "doc",
            "content": [
                {
                    "type": "paragraph",
                    "content": [
                        { "type": "text", "text": "hello world" }
                    ]
                }
            ]
        });
        let apply = left.apply_local_document(local.clone()).unwrap();
        for message in apply.messages {
            let _ = right.handle_message(message).unwrap();
        }

        assert_eq!(right.document_json(), local);
    }

    #[test]
    fn collaboration_session_syncs_with_standard_yrs_peer() {
        let expected = json!({
            "type": "doc",
            "content": [
                {
                    "type": "paragraph",
                    "content": [
                        { "type": "text", "text": "hello from session" }
                    ]
                }
            ]
        });
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "initialDocumentJson": expected,
                "localAwareness": {
                    "user": {
                        "name": "Session"
                    }
                }
            })
            .to_string(),
        );
        let peer = Awareness::new(Doc::with_client_id(2));

        let peer_sync_step_1 = encode_message(Message::Sync(SyncMessage::SyncStep1(
            peer.doc().transact().state_vector(),
        )));
        let session_responses = session.handle_message(peer_sync_step_1).unwrap();
        let peer_replies = apply_messages_to_peer(&peer, session.start().unwrap().messages);
        for message in peer_replies {
            let _ = session.handle_message(message).unwrap();
        }
        let _ = apply_messages_to_peer(&peer, session_responses.messages);

        assert_eq!(peer_document_json(&peer), expected);

        let peers: HashMap<_, _> = peer
            .iter()
            .flat_map(|(id, state)| state.data.map(|data| (id, data)))
            .collect();
        assert_eq!(
            peers.get(&1).map(|state| state.as_ref()),
            Some(r#"{"user":{"name":"Session"}}"#)
        );
    }

    #[test]
    fn collaboration_session_applies_standard_yrs_sync_update() {
        let mut session = CollaborationSession::new(r#"{"clientId":1}"#);
        let mut peer = Awareness::new(Doc::with_client_id(2));

        let update = {
            let mut txn = peer.doc_mut().transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            insert_json_node(
                &fragment,
                &mut txn,
                0,
                &json!({
                    "type": "paragraph",
                    "content": [
                        { "type": "text", "text": "hello from raw peer" }
                    ]
                }),
            );
            txn.encode_update_v1()
        };

        let result = session
            .handle_message(encode_message(Message::Sync(SyncMessage::Update(update))))
            .unwrap();

        assert!(result.document_changed);
        assert_eq!(
            session.document_json(),
            json!({
                "type": "doc",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "hello from raw peer" }
                        ]
                    }
                ]
            })
        );
    }

    #[test]
    fn collaboration_session_exchanges_awareness_with_standard_yrs_peer() {
        let mut session = CollaborationSession::new(r#"{"clientId":1}"#);
        let peer = Awareness::new(Doc::with_client_id(2));

        let session_awareness = session
            .set_local_awareness(json!({
                "user": {
                    "name": "Session"
                },
                "selection": {
                    "anchor": 2,
                    "head": 4
                }
            }))
            .unwrap();
        let _ = apply_messages_to_peer(&peer, session_awareness.messages);

        let peer_states: HashMap<_, _> = peer
            .iter()
            .flat_map(|(id, state)| state.data.map(|data| (id, data)))
            .collect();
        assert_eq!(
            peer_states.get(&1).map(|state| state.as_ref()),
            Some(r#"{"selection":{"anchor":2,"head":4},"user":{"name":"Session"}}"#)
        );

        peer.set_local_state(json!({
            "user": {
                "name": "Peer"
            },
            "selection": {
                "anchor": 5,
                "head": 5
            }
        }))
        .unwrap();
        let awareness_message = encode_message(Message::Awareness(peer.update().unwrap()));
        let result = session.handle_message(awareness_message).unwrap();

        assert!(result.peers_changed);
        let peers = result.peers.unwrap_or_default();
        let remote_peer = peers.into_iter().find(|peer| peer.client_id == 2).unwrap();
        assert_eq!(
            remote_peer.state,
            json!({
                "user": {
                    "name": "Peer"
                },
                "selection": {
                    "anchor": 5,
                    "head": 5
                }
            })
        );
    }

    #[test]
    fn collaboration_session_clear_local_awareness_emits_removal_update() {
        let mut session = CollaborationSession::new(r#"{"clientId":1}"#);
        let peer = Awareness::new(Doc::with_client_id(2));

        let awareness = session
            .set_local_awareness(json!({
                "user": {
                    "name": "Session"
                },
                "selection": {
                    "anchor": 2,
                    "head": 4
                }
            }))
            .unwrap();
        let _ = apply_messages_to_peer(&peer, awareness.messages);
        let peer_states: HashMap<_, _> = peer
            .iter()
            .flat_map(|(id, state)| state.data.map(|data| (id, data)))
            .collect();
        assert!(peer_states.contains_key(&1));

        let cleared = session.clear_local_awareness().unwrap();
        assert_eq!(cleared.messages.len(), 1);
        let _ = apply_messages_to_peer(&peer, cleared.messages);

        assert!(peer.state::<Value>(1).is_none());
    }

    #[test]
    fn collaboration_session_augments_local_selection_with_standard_cursor() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default",
                "initialDocumentJson": {
                    "type": "doc",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "hello" }]
                        }
                    ]
                }
            })
            .to_string(),
        );
        let peer = Awareness::new(Doc::with_client_id(2));

        let awareness = session
            .set_local_awareness(json!({
                "user": {
                    "name": "Session"
                },
                "selection": {
                    "anchor": 2,
                    "head": 4
                },
                "focused": true
            }))
            .unwrap();
        let _ = apply_messages_to_peer(&peer, awareness.messages);

        let peer_states: HashMap<_, _> = peer
            .iter()
            .flat_map(|(id, state)| state.data.map(|data| (id, data)))
            .collect();
        let state_json: Value = serde_json::from_str(
            peer_states
                .get(&1)
                .expect("session awareness should sync to peer"),
        )
        .expect("session awareness JSON should decode");

        assert_eq!(
            state_json.get("selection"),
            Some(&json!({ "anchor": 2, "head": 4 }))
        );
        let cursor = state_json
            .get("cursor")
            .and_then(Value::as_object)
            .expect("session awareness should include yjs cursor");
        assert_eq!(
            cursor
                .get("anchor")
                .and_then(Value::as_object)
                .and_then(|value| value.get("assoc"))
                .and_then(Value::as_i64),
            Some(-1)
        );
        assert_eq!(
            cursor
                .get("head")
                .and_then(Value::as_object)
                .and_then(|value| value.get("assoc"))
                .and_then(Value::as_i64),
            Some(-1)
        );
    }

    #[test]
    fn collaboration_session_remaps_local_selection_from_cursor_after_remote_document_update() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default"
            })
            .to_string(),
        );
        let mut peer = Awareness::new(Doc::with_client_id(2));

        let initial_update = {
            let mut txn = peer.doc_mut().transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("default");
            insert_json_node(
                &fragment,
                &mut txn,
                0,
                &json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "hello" }]
                }),
            );
            txn.encode_update_v1()
        };
        session
            .handle_message(encode_message(Message::Sync(SyncMessage::Update(
                initial_update,
            ))))
            .expect("session should accept peer document");

        session
            .set_local_awareness(json!({
                "user": {
                    "name": "Session"
                },
                "selection": {
                    "anchor": 4,
                    "head": 4
                },
                "focused": true
            }))
            .unwrap();

        let remote_insert_update = {
            let mut txn = peer.doc_mut().transact_mut();
            let text = {
                let fragment = txn
                    .get_xml_fragment("default")
                    .expect("peer fragment should exist");
                let XmlOut::Element(paragraph) =
                    fragment.get(&txn, 0).expect("paragraph should exist")
                else {
                    panic!("expected paragraph element");
                };
                let XmlOut::Text(text) = paragraph.get(&txn, 0).expect("text node should exist")
                else {
                    panic!("expected paragraph text");
                };
                text
            };
            text.insert(&mut txn, 0, "X");
            txn.encode_update_v1()
        };

        let result = session
            .handle_message(encode_message(Message::Sync(SyncMessage::Update(
                remote_insert_update,
            ))))
            .expect("session should accept peer edit before local cursor");

        assert!(result.document_changed);
        assert!(result.peers_changed);
        let local_peer = result
            .peers
            .expect("document update should include remapped peers")
            .into_iter()
            .find(|peer| peer.is_local)
            .expect("local peer should be present");
        assert_eq!(
            local_peer.state.get("selection"),
            Some(&json!({
                "anchor": 5,
                "head": 5
            }))
        );
    }

    #[test]
    fn collaboration_session_remaps_local_selection_after_multibyte_replacement_before_cursor() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default",
                "initialDocumentJson": {
                    "type": "doc",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "héllo" }]
                        }
                    ]
                }
            })
            .to_string(),
        );

        session
            .set_local_awareness(json!({
                "user": {
                    "name": "Session"
                },
                "selection": {
                    "anchor": 5,
                    "head": 5
                },
                "focused": true
            }))
            .unwrap();

        let result = session.apply_local_document(json!({
            "type": "doc",
            "content": [
                {
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "hXYllo" }]
                }
            ]
        }));

        let result = result.unwrap();
        assert_eq!(
            result.document_json,
            Some(json!({
                "type": "doc",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "hXYllo" }]
                    }
                ]
            }))
        );
        let local_peer = result
            .peers
            .expect("local replacement should include remapped peers")
            .into_iter()
            .find(|peer| peer.is_local)
            .expect("local peer should be present");
        assert_eq!(
            local_peer.state.get("selection"),
            Some(&json!({
                "anchor": 6,
                "head": 6
            }))
        );
    }

    #[test]
    fn collaboration_session_derives_numeric_selection_from_standard_cursor_awareness() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default"
            })
            .to_string(),
        );
        let mut peer = Awareness::new(Doc::with_client_id(2));

        let update = {
            let mut txn = peer.doc_mut().transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("default");
            insert_json_node(
                &fragment,
                &mut txn,
                0,
                &json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "hello" }]
                }),
            );
            txn.encode_update_v1()
        };
        session
            .handle_message(encode_message(Message::Sync(SyncMessage::Update(update))))
            .expect("session should accept peer document");

        let cursor = {
            let txn = peer.doc().transact();
            let fragment = txn
                .get_xml_fragment("default")
                .expect("peer fragment should exist");
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).expect("paragraph should exist")
            else {
                panic!("expected paragraph element");
            };
            let XmlOut::Text(text) = paragraph.get(&txn, 0).expect("text node should exist") else {
                panic!("expected paragraph text");
            };
            json!({
                "anchor": text.sticky_index(&txn, 1, Assoc::Before).expect("anchor sticky index"),
                "head": text.sticky_index(&txn, 3, Assoc::Before).expect("head sticky index"),
            })
        };

        peer.set_local_state(json!({
            "user": {
                "name": "Peer"
            },
            "cursor": cursor,
            "focused": true
        }))
        .expect("peer awareness should update");

        let result = session
            .handle_message(encode_message(Message::Awareness(
                peer.update().expect("peer awareness update"),
            )))
            .expect("session should accept peer awareness");

        let peers = result.peers.expect("peer update should include peers");
        let remote_peer = peers
            .into_iter()
            .find(|peer| peer.client_id == 2)
            .expect("remote peer should exist");

        assert_eq!(
            remote_peer.state.get("selection"),
            Some(&json!({
                "anchor": 2,
                "head": 4
            }))
        );
    }

    #[test]
    fn collaboration_session_maps_standard_utf16_cursor_after_emoji_to_scalar_position() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default"
            })
            .to_string(),
        );
        let mut peer_options = yrs::Options::with_client_id(2);
        peer_options.offset_kind = yrs::OffsetKind::Utf16;
        let mut peer = Awareness::new(Doc::with_options(peer_options));

        let update = {
            let mut txn = peer.doc_mut().transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("default");
            insert_json_node(
                &fragment,
                &mut txn,
                0,
                &json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "a😀b" }]
                }),
            );
            txn.encode_update_v1()
        };
        session
            .handle_message(encode_message(Message::Sync(SyncMessage::Update(update))))
            .expect("session should accept UTF-16 peer document");

        let cursor = {
            let txn = peer.doc().transact();
            let fragment = txn
                .get_xml_fragment("default")
                .expect("peer fragment should exist");
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).expect("paragraph should exist")
            else {
                panic!("expected paragraph element");
            };
            let XmlOut::Text(text) = paragraph.get(&txn, 0).expect("text node should exist") else {
                panic!("expected paragraph text");
            };
            json!({
                "anchor": text.sticky_index(&txn, 3, Assoc::Before).expect("anchor sticky index"),
                "head": text.sticky_index(&txn, 3, Assoc::Before).expect("head sticky index"),
            })
        };
        peer.set_local_state(json!({
            "user": { "name": "Peer" },
            "cursor": cursor,
            "focused": true
        }))
        .expect("peer awareness should update");

        let result = session
            .handle_message(encode_message(Message::Awareness(
                peer.update().expect("peer awareness update"),
            )))
            .expect("session should accept peer awareness");
        let remote_peer = result
            .peers
            .expect("peer update should include peers")
            .into_iter()
            .find(|peer| peer.client_id == 2)
            .expect("remote peer should exist");

        assert_eq!(
            remote_peer.state.get("selection"),
            Some(&json!({ "anchor": 3, "head": 3 }))
        );
    }

    #[test]
    fn collaboration_session_counts_xml_text_as_one_parent_sequence_item() {
        let session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default",
                "initialDocumentJson": {
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "a😀b" },
                            { "type": "hardBreak" },
                            { "type": "text", "text": "c" }
                        ]
                    }]
                }
            })
            .to_string(),
        );
        let txn = session.doc.transact();
        let fragment = txn
            .get_xml_fragment("default")
            .expect("fragment should exist");
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).expect("paragraph should exist")
        else {
            panic!("expected paragraph element");
        };
        let boundary = paragraph
            .sticky_index(&txn, 1, Assoc::Before)
            .expect("parent boundary should produce sticky index");

        assert_eq!(
            sticky_index_to_doc_pos(&txn, &fragment, &boundary, &session.void_element_tags,),
            Some(4),
            "parent index 1 is after the entire three-scalar XmlText child"
        );
    }

    #[test]
    fn collaboration_session_derives_numeric_selection_from_standard_cursor_awareness_in_later_paragraph(
    ) {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default"
            })
            .to_string(),
        );
        let mut peer = Awareness::new(Doc::with_client_id(2));

        let update = {
            let mut txn = peer.doc_mut().transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("default");
            insert_json_node(
                &fragment,
                &mut txn,
                0,
                &json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "hello" }]
                }),
            );
            insert_json_node(
                &fragment,
                &mut txn,
                1,
                &json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "world" }]
                }),
            );
            txn.encode_update_v1()
        };
        session
            .handle_message(encode_message(Message::Sync(SyncMessage::Update(update))))
            .expect("session should accept peer document");

        let cursor = {
            let txn = peer.doc().transact();
            let fragment = txn
                .get_xml_fragment("default")
                .expect("peer fragment should exist");
            let XmlOut::Element(second_paragraph) = fragment
                .get(&txn, 1)
                .expect("second paragraph should exist")
            else {
                panic!("expected second paragraph element");
            };
            let XmlOut::Text(text) = second_paragraph
                .get(&txn, 0)
                .expect("text node should exist")
            else {
                panic!("expected second paragraph text");
            };
            json!({
                "anchor": text.sticky_index(&txn, 1, Assoc::Before).expect("anchor sticky index"),
                "head": text.sticky_index(&txn, 3, Assoc::Before).expect("head sticky index"),
            })
        };

        peer.set_local_state(json!({
            "user": {
                "name": "Peer"
            },
            "cursor": cursor,
            "focused": true
        }))
        .expect("peer awareness should update");

        let result = session
            .handle_message(encode_message(Message::Awareness(
                peer.update().expect("peer awareness update"),
            )))
            .expect("session should accept peer awareness");

        let peers = result.peers.expect("peer update should include peers");
        let remote_peer = peers
            .into_iter()
            .find(|peer| peer.client_id == 2)
            .expect("remote peer should exist");

        assert_eq!(
            remote_peer.state.get("selection"),
            Some(&json!({
                "anchor": 9,
                "head": 11
            }))
        );
    }

    #[test]
    fn collaboration_session_derives_standard_cursor_awareness_from_numeric_selection_in_later_paragraph(
    ) {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default",
                "initialDocumentJson": {
                    "type": "doc",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "hello" }]
                        },
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "world" }]
                        }
                    ]
                }
            })
            .to_string(),
        );

        let result = session.set_local_awareness(json!({
            "user": { "name": "Local" },
            "selection": {
                "anchor": 9,
                "head": 11
            },
            "focused": true
        }));

        let message = result
            .unwrap()
            .messages
            .into_iter()
            .next()
            .expect("expected awareness update");
        let Message::Awareness(update) =
            yrs::updates::decoder::Decode::decode_v1(message.as_slice()).expect("decode awareness")
        else {
            panic!("expected awareness message");
        };

        let peer = Awareness::new(Doc::with_client_id(2));
        peer.apply_update(update)
            .expect("peer should accept awareness update");

        let peer_states: HashMap<_, _> = peer
            .iter()
            .flat_map(|(id, state)| state.data.map(|data| (id, data)))
            .collect();
        let local_state: Value = serde_json::from_str(
            peer_states
                .get(&1)
                .expect("local awareness should sync to peer"),
        )
        .expect("local awareness JSON should decode");
        let cursor = local_state
            .get("cursor")
            .and_then(Value::as_object)
            .expect("cursor should be published");

        let txn = session.doc.transact();
        let fragment = txn
            .get_xml_fragment("default")
            .expect("fragment should exist");
        let XmlOut::Element(second_paragraph) = fragment
            .get(&txn, 1)
            .expect("second paragraph should exist")
        else {
            panic!("expected second paragraph element");
        };
        let XmlOut::Text(text) = second_paragraph
            .get(&txn, 0)
            .expect("text node should exist")
        else {
            panic!("expected second paragraph text");
        };
        let expected_anchor = serde_json::to_value(
            text.sticky_index(&txn, 1, Assoc::Before)
                .expect("expected anchor sticky index"),
        )
        .expect("anchor should serialize");
        let expected_head = serde_json::to_value(
            text.sticky_index(&txn, 3, Assoc::Before)
                .expect("expected head sticky index"),
        )
        .expect("head should serialize");

        assert_eq!(cursor.get("anchor"), Some(&expected_anchor));
        assert_eq!(cursor.get("head"), Some(&expected_head));
    }

    #[test]
    fn collaboration_session_derives_collapsed_selection_from_standard_cursor_awareness() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default"
            })
            .to_string(),
        );
        let mut peer = Awareness::new(Doc::with_client_id(2));

        let update = {
            let mut txn = peer.doc_mut().transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("default");
            insert_json_node(
                &fragment,
                &mut txn,
                0,
                &json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "hello" }]
                }),
            );
            txn.encode_update_v1()
        };
        session
            .handle_message(encode_message(Message::Sync(SyncMessage::Update(update))))
            .expect("session should accept peer document");

        let cursor = {
            let txn = peer.doc().transact();
            let fragment = txn
                .get_xml_fragment("default")
                .expect("peer fragment should exist");
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).expect("paragraph should exist")
            else {
                panic!("expected paragraph element");
            };
            let XmlOut::Text(text) = paragraph.get(&txn, 0).expect("text node should exist") else {
                panic!("expected paragraph text");
            };
            json!({
                "anchor": text.sticky_index(&txn, 3, Assoc::After).expect("anchor sticky index"),
                "head": text.sticky_index(&txn, 3, Assoc::After).expect("head sticky index"),
            })
        };

        peer.set_local_state(json!({
            "user": {
                "name": "Peer"
            },
            "cursor": cursor,
            "focused": true
        }))
        .expect("peer awareness should update");

        let result = session
            .handle_message(encode_message(Message::Awareness(
                peer.update().expect("peer awareness update"),
            )))
            .expect("session should accept peer awareness");

        let peers = result.peers.expect("peer update should include peers");
        let remote_peer = peers
            .into_iter()
            .find(|peer| peer.client_id == 2)
            .expect("remote peer should exist");

        assert_eq!(
            remote_peer.state.get("selection"),
            Some(&json!({
                "anchor": 4,
                "head": 4
            }))
        );
    }

    #[test]
    fn collaboration_session_uses_after_assoc_for_collapsed_cursor_below_horizontal_rule() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default",
                "initialDocumentJson": {
                    "type": "doc",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "hello" }]
                        },
                        {
                            "type": "horizontalRule"
                        },
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "world" }]
                        }
                    ]
                }
            })
            .to_string(),
        );
        let peer = Awareness::new(Doc::with_client_id(2));

        let awareness = session.set_local_awareness(json!({
            "user": {
                "name": "Session"
            },
            "selection": {
                "anchor": 9,
                "head": 9
            },
            "focused": true
        }));
        let _ = apply_messages_to_peer(&peer, awareness.unwrap().messages);

        let peer_states: HashMap<_, _> = peer
            .iter()
            .flat_map(|(id, state)| state.data.map(|data| (id, data)))
            .collect();
        let state_json: Value = serde_json::from_str(
            peer_states
                .get(&1)
                .expect("session awareness should sync to peer"),
        )
        .expect("session awareness JSON should decode");

        assert_eq!(
            state_json.get("selection"),
            Some(&json!({ "anchor": 9, "head": 9 }))
        );
        let cursor = state_json
            .get("cursor")
            .and_then(Value::as_object)
            .expect("session awareness should include yjs cursor");
        assert_eq!(
            cursor
                .get("anchor")
                .and_then(Value::as_object)
                .and_then(|value| value.get("assoc"))
                .and_then(Value::as_i64),
            Some(0)
        );
        assert_eq!(
            cursor
                .get("head")
                .and_then(Value::as_object)
                .and_then(|value| value.get("assoc"))
                .and_then(Value::as_i64),
            Some(0)
        );
    }

    #[test]
    fn collaboration_session_falls_back_to_before_assoc_for_collapsed_cursor_at_text_end() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default",
                "initialDocumentJson": {
                    "type": "doc",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "hello" }]
                        }
                    ]
                }
            })
            .to_string(),
        );
        let peer = Awareness::new(Doc::with_client_id(2));

        let awareness = session.set_local_awareness(json!({
            "user": {
                "name": "Session"
            },
            "selection": {
                "anchor": 6,
                "head": 6
            },
            "focused": true
        }));
        let _ = apply_messages_to_peer(&peer, awareness.unwrap().messages);

        let peer_states: HashMap<_, _> = peer
            .iter()
            .flat_map(|(id, state)| state.data.map(|data| (id, data)))
            .collect();
        let state_json: Value = serde_json::from_str(
            peer_states
                .get(&1)
                .expect("session awareness should sync to peer"),
        )
        .expect("session awareness JSON should decode");

        assert_eq!(
            state_json.get("selection"),
            Some(&json!({ "anchor": 6, "head": 6 }))
        );

        let cursor = state_json
            .get("cursor")
            .and_then(Value::as_object)
            .expect("session awareness should include yjs cursor");

        let txn = session.doc.transact();
        let fragment = txn
            .get_xml_fragment("default")
            .expect("fragment should exist");
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).expect("paragraph should exist")
        else {
            panic!("expected paragraph element");
        };
        let XmlOut::Text(text) = paragraph.get(&txn, 0).expect("text node should exist") else {
            panic!("expected paragraph text");
        };
        let expected = serde_json::to_value(
            text.sticky_index(&txn, 5, Assoc::Before)
                .expect("expected sticky index"),
        )
        .expect("sticky index should serialize");

        assert_eq!(cursor.get("anchor"), Some(&expected));
        assert_eq!(cursor.get("head"), Some(&expected));
    }

    #[test]
    fn collaboration_session_uses_schema_void_nodes_for_custom_block_positions() {
        let void_name = "calloutDivider";
        let session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default",
                "schema": custom_void_schema_json(void_name),
                "initialDocumentJson": {
                    "type": "doc",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "hello" }]
                        },
                        {
                            "type": void_name
                        },
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "world" }]
                        }
                    ]
                }
            })
            .to_string(),
        );

        assert!(session.void_element_tags.contains(void_name));

        let txn = session.doc.transact();
        let fragment = txn
            .get_xml_fragment("default")
            .expect("fragment should exist");
        let sticky =
            doc_pos_to_sticky_index(&txn, &fragment, 9, Assoc::After, &session.void_element_tags)
                .expect("selection below custom void node should map to a sticky index");
        let XmlOut::Element(second_paragraph) = fragment
            .get(&txn, 2)
            .expect("second paragraph should exist")
        else {
            panic!("expected second paragraph element");
        };
        let XmlOut::Text(text) = second_paragraph
            .get(&txn, 0)
            .expect("text node should exist")
        else {
            panic!("expected second paragraph text");
        };
        let expected = text
            .sticky_index(&txn, 0, Assoc::After)
            .expect("expected sticky index");

        assert_eq!(
            serde_json::to_value(&sticky).expect("sticky index should serialize"),
            serde_json::to_value(&expected).expect("expected sticky index should serialize")
        );
        assert_eq!(
            sticky_index_to_doc_pos(&txn, &fragment, &expected, &session.void_element_tags),
            Some(9)
        );
    }

    #[test]
    fn collaboration_session_derives_collapsed_selection_from_after_assoc_cursor_below_horizontal_rule(
    ) {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default"
            })
            .to_string(),
        );
        let mut peer = Awareness::new(Doc::with_client_id(2));

        let update = {
            let mut txn = peer.doc_mut().transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("default");
            insert_json_node(
                &fragment,
                &mut txn,
                0,
                &json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "hello" }]
                }),
            );
            insert_json_node(
                &fragment,
                &mut txn,
                1,
                &json!({
                    "type": "horizontalRule"
                }),
            );
            insert_json_node(
                &fragment,
                &mut txn,
                2,
                &json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "world" }]
                }),
            );
            txn.encode_update_v1()
        };
        session
            .handle_message(encode_message(Message::Sync(SyncMessage::Update(update))))
            .expect("session should accept peer document");

        let cursor = {
            let txn = peer.doc().transact();
            let fragment = txn
                .get_xml_fragment("default")
                .expect("peer fragment should exist");
            let XmlOut::Element(second_paragraph) = fragment
                .get(&txn, 2)
                .expect("second paragraph should exist")
            else {
                panic!("expected second paragraph element");
            };
            let XmlOut::Text(text) = second_paragraph
                .get(&txn, 0)
                .expect("text node should exist")
            else {
                panic!("expected second paragraph text");
            };
            json!({
                "anchor": text.sticky_index(&txn, 0, Assoc::After).expect("anchor sticky index"),
                "head": text.sticky_index(&txn, 0, Assoc::After).expect("head sticky index"),
            })
        };

        peer.set_local_state(json!({
            "user": {
                "name": "Peer"
            },
            "cursor": cursor,
            "focused": true
        }))
        .expect("peer awareness should update");

        let result = session
            .handle_message(encode_message(Message::Awareness(
                peer.update().expect("peer awareness update"),
            )))
            .expect("session should accept peer awareness");

        let peers = result.peers.expect("peer update should include peers");
        let remote_peer = peers
            .into_iter()
            .find(|peer| peer.client_id == 2)
            .expect("remote peer should exist");

        assert_eq!(
            remote_peer.state.get("selection"),
            Some(&json!({
                "anchor": 9,
                "head": 9
            }))
        );
    }

    #[test]
    fn collaboration_session_preserves_horizontal_rule_insert_order_between_blocks() {
        let base_document = json!({
            "type": "doc",
            "content": [
                {
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "hello" }]
                },
                {
                    "type": "paragraph"
                }
            ]
        });
        let document_with_horizontal_rule = json!({
            "type": "doc",
            "content": [
                {
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "hello" }]
                },
                {
                    "type": "horizontalRule"
                },
                {
                    "type": "paragraph"
                }
            ]
        });
        let mut sender = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "fragmentName": "default",
                "initialDocumentJson": base_document
            })
            .to_string(),
        );
        let mut receiver = CollaborationSession::new(
            &json!({
                "clientId": 2,
                "fragmentName": "default"
            })
            .to_string(),
        );

        receiver
            .apply_encoded_state(sender.encoded_state())
            .expect("receiver should sync base document");

        let document_update = sender
            .apply_local_document(document_with_horizontal_rule.clone())
            .unwrap();
        for message in document_update.messages {
            receiver
                .handle_message(message)
                .expect("receiver should apply horizontal rule update");
        }

        assert_eq!(sender.document_json(), document_with_horizontal_rule);
        assert_eq!(receiver.document_json(), document_with_horizontal_rule);
    }

    #[test]
    fn collaboration_session_round_trips_encoded_state() {
        let source = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "initialDocumentJson": {
                    "type": "doc",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "persist me" }]
                        }
                    ]
                }
            })
            .to_string(),
        );
        let encoded_state = source.encoded_state();

        let mut restored = CollaborationSession::new(r#"{"clientId":2}"#);
        let result = restored.apply_encoded_state(encoded_state).unwrap();

        assert!(result.document_changed);
        assert_eq!(result.messages.len(), 1);
        assert_eq!(restored.document_json(), source.document_json());
    }

    #[test]
    fn collaboration_session_replaces_from_encoded_state() {
        let source = CollaborationSession::new(
            &json!({
                "clientId": 1,
                "initialDocumentJson": {
                    "type": "doc",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "seeded from state" }]
                        }
                    ]
                }
            })
            .to_string(),
        );

        let mut session = CollaborationSession::new(r#"{"clientId":2}"#);
        let result = session
            .replace_encoded_state(source.encoded_state())
            .unwrap();

        assert_eq!(result.messages.len(), 1);
        assert_eq!(session.document_json(), source.document_json());
    }

    #[test]
    fn invalid_replacement_preserves_all_collaboration_state() {
        let mut session = CollaborationSession::new(
            &json!({
                "clientId": 2,
                "initialDocumentJson": {
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "keep me" }]
                    }]
                },
                "localAwareness": { "user": { "name": "Ada" } }
            })
            .to_string(),
        );
        let before_json = session.document_json();
        let before_encoded = session.encoded_state();
        let before_peers = session.peers();
        let before_document_revision = session.document_revision;
        let before_peers_revision = session.peers_revision;
        let before_local_awareness = session.local_awareness_state.clone();

        let error = session.replace_encoded_state(vec![0xff, 0x00]).unwrap_err();

        assert_eq!(error.code(), "COLLABORATION_DECODE_FAILED");
        assert_eq!(session.document_json(), before_json);
        assert_eq!(session.encoded_state(), before_encoded);
        assert_eq!(session.peers(), before_peers);
        assert_eq!(session.document_revision, before_document_revision);
        assert_eq!(session.peers_revision, before_peers_revision);
        assert_eq!(session.local_awareness_state, before_local_awareness);
    }

    #[test]
    fn encoded_state_limit_is_checked_before_yrs_decode() {
        let mut session = CollaborationSession::new(
            &json!({
                "resourceLimits": { "maxEncodedStateBytes": 1 }
            })
            .to_string(),
        );

        let error = session.replace_encoded_state(vec![0xff, 0x00]).unwrap_err();

        assert_eq!(error.code(), "INPUT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(1));
        assert_eq!(error.actual, Some(2));
    }

    #[test]
    fn invalid_remote_document_message_preserves_all_collaboration_state() {
        let mut source = CollaborationSession::new(r#"{"clientId":1}"#);
        let update = source.apply_local_document(json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "too long" }]
            }]
        }));
        let message = update.unwrap().messages.into_iter().next().unwrap();

        let mut session = CollaborationSession::new(
            r#"{"clientId":2,"maxLength":3,"localAwareness":{"user":{"name":"Ada"}}}"#,
        );
        let before_json = session.document_json();
        let before_encoded = session.encoded_state();
        let before_peers = session.peers();
        let before_document_revision = session.document_revision;
        let before_peers_revision = session.peers_revision;

        let error = session.handle_message(message).unwrap_err();

        assert_eq!(error.code(), "MAX_LENGTH_EXCEEDED");
        assert_eq!(session.document_json(), before_json);
        assert_eq!(session.encoded_state(), before_encoded);
        assert_eq!(session.peers(), before_peers);
        assert_eq!(session.document_revision, before_document_revision);
        assert_eq!(session.peers_revision, before_peers_revision);
    }

    #[test]
    fn invalid_local_snapshot_preserves_all_collaboration_state_and_counts_scalars() {
        let mut session = CollaborationSession::try_new(
            r#"{"clientId":2,"maxLength":1,"localAwareness":{"user":{"name":"Ada"}}}"#,
        )
        .unwrap();
        let before_json = session.document_json();
        let before_encoded = session.encoded_state();
        let before_peers = session.peers();

        let one_scalar = session.apply_local_document(json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "😀" }] }]
        }));
        assert!(one_scalar.is_ok());
        let accepted_json = session.document_json();
        let accepted_encoded = session.encoded_state();
        let accepted_peers = session.peers();

        let error = session
            .apply_local_document(json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "😀x" }] }]
            }))
            .unwrap_err();
        assert_eq!(error.code(), "MAX_LENGTH_EXCEEDED");
        assert_ne!(accepted_json, before_json);
        assert_ne!(accepted_encoded, before_encoded);
        assert_eq!(accepted_peers, before_peers);
        assert_eq!(session.document_json(), accepted_json);
        assert_eq!(session.encoded_state(), accepted_encoded);
        assert_eq!(session.peers(), accepted_peers);
    }

    #[test]
    fn deep_and_wide_encoded_states_are_rejected_atomically() {
        let mut deep_child = json!({ "type": "paragraph" });
        for _ in 0..32 {
            deep_child = json!({ "type": "blockquote", "content": [deep_child] });
        }
        let mut deep_source = CollaborationSession::try_new(
            r#"{"clientId":1,"resourceLimits":{"maxDocumentDepth":64}}"#,
        )
        .unwrap();
        deep_source
            .apply_local_document(json!({ "type": "doc", "content": [deep_child] }))
            .unwrap();

        let wide_content = (0..32)
            .map(|_| json!({ "type": "paragraph" }))
            .collect::<Vec<_>>();
        let mut wide_source = CollaborationSession::try_new(r#"{"clientId":3}"#).unwrap();
        wide_source
            .apply_local_document(json!({ "type": "doc", "content": wide_content }))
            .unwrap();

        for (config, state) in [
            (
                r#"{"clientId":2,"resourceLimits":{"maxDocumentDepth":8}}"#,
                deep_source.encoded_state(),
            ),
            (
                r#"{"clientId":4,"resourceLimits":{"maxDocumentNodes":10}}"#,
                wide_source.encoded_state(),
            ),
        ] {
            let mut target = CollaborationSession::try_new(config).unwrap();
            let before_json = target.document_json();
            let before_state = target.encoded_state();
            let error = target.replace_encoded_state(state).unwrap_err();
            assert_eq!(error.code(), "DOCUMENT_LIMIT_EXCEEDED");
            assert_eq!(target.document_json(), before_json);
            assert_eq!(target.encoded_state(), before_state);
        }
    }

    #[test]
    fn local_codec_limit_errors_preserve_structured_boundary_fields() {
        let mut session = CollaborationSession::try_new(
            r#"{"clientId":1,"resourceLimits":{"maxDocumentNodes":2}}"#,
        )
        .unwrap();

        let error = session
            .apply_local_document(json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "third node" }]
                }]
            }))
            .unwrap_err();

        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(2));
        assert_eq!(error.actual, Some(3));
    }

    #[test]
    fn cumulative_encoded_state_growth_is_rejected_atomically() {
        let mut target = CollaborationSession::try_new(r#"{"clientId":11}"#).unwrap();
        target.apply_local_document(json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "local" }] }]
        })).unwrap();
        let mut source = CollaborationSession::try_new(r#"{"clientId":12}"#).unwrap();
        source.apply_local_document(json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "remote" }] }]
        })).unwrap();
        let incoming = source.encoded_state();
        let before_json = target.document_json();
        let before_state = target.encoded_state();
        target.resource_limits.max_encoded_state_bytes = before_state.len().max(incoming.len());

        let error = target.apply_encoded_state(incoming.clone()).unwrap_err();
        assert_eq!(error.code(), "INPUT_LIMIT_EXCEEDED");
        assert_eq!(target.document_json(), before_json);
        assert_eq!(target.encoded_state(), before_state);

        let message = encode_message(Message::Sync(SyncMessage::Update(incoming)));
        let error = target.handle_message(message).unwrap_err();
        assert_eq!(error.code(), "INPUT_LIMIT_EXCEEDED");
        assert_eq!(target.document_json(), before_json);
        assert_eq!(target.encoded_state(), before_state);
    }

    #[test]
    fn merge_and_sync_preserve_awareness_and_awareness_failures_are_structured() {
        let mut target = CollaborationSession::try_new(
            r#"{"clientId":21,"localAwareness":{"user":{"name":"Ada"}}}"#,
        )
        .unwrap();
        let before_peers = target.peers();
        let before_local = target.local_awareness_state.clone();
        let mut source = CollaborationSession::try_new(r#"{"clientId":22}"#).unwrap();
        source.apply_local_document(json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "remote" }] }]
        })).unwrap();

        target.apply_encoded_state(source.encoded_state()).unwrap();
        assert_eq!(target.peers(), before_peers);
        assert_eq!(target.local_awareness_state, before_local);

        let message = encode_message(Message::Sync(SyncMessage::Update(source.encoded_state())));
        target.handle_message(message).unwrap();
        assert_eq!(target.peers(), before_peers);
        assert_eq!(target.local_awareness_state, before_local);

        let empty = Awareness::new(Doc::with_client_id(99));
        let error =
            apply_awareness_snapshot(&empty, Err(yrs::sync::awareness::Error::ClientNotFound(99)))
                .unwrap_err();
        assert_eq!(error.code(), "COLLABORATION_APPLY_FAILED");
    }

    #[derive(Debug, PartialEq)]
    struct CompleteSessionSnapshot {
        document_json: Value,
        encoded_state: Vec<u8>,
        peers: Vec<CollaborationPeer>,
        peer_states: HashMap<u64, CachedPeerState>,
        document_revision: u64,
        peers_revision: u64,
        local_awareness: Option<Value>,
        fragment_name: String,
        document_root_type: String,
    }

    fn complete_snapshot(session: &CollaborationSession) -> CompleteSessionSnapshot {
        CompleteSessionSnapshot {
            document_json: session.document_json(),
            encoded_state: session.encoded_state(),
            peers: session.peers(),
            peer_states: session.cached_peer_states.clone(),
            document_revision: session.document_revision,
            peers_revision: session.peers_revision,
            local_awareness: session.local_awareness_state.clone(),
            fragment_name: session.fragment_name.clone(),
            document_root_type: session.document_root_type.clone(),
        }
    }

    fn session_with_local_and_remote_awareness(client_id: u64) -> CollaborationSession {
        let mut session = CollaborationSession::try_new(
            &json!({
                "clientId": client_id,
                "localAwareness": { "user": { "name": "Local" } }
            })
            .to_string(),
        )
        .unwrap();
        let remote = Awareness::new(Doc::with_client_id(client_id + 100));
        remote
            .set_local_state(json!({ "user": { "name": "Remote" } }))
            .unwrap();
        session
            .handle_message(encode_message(Message::Awareness(remote.update().unwrap())))
            .unwrap();
        assert_eq!(session.peers().len(), 2);
        session
    }

    #[test]
    fn outbound_durable_and_local_update_limits_are_atomic() {
        let mut source = CollaborationSession::try_new(r#"{"clientId":31}"#).unwrap();
        let document = json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "outbound limit" }] }]
        });
        source.apply_local_document(document.clone()).unwrap();
        let state = source.encoded_state();
        let echo_size = encode_message(Message::Sync(SyncMessage::Update(state.clone()))).len();

        for replace in [false, true] {
            let mut session =
                session_with_local_and_remote_awareness(if replace { 32 } else { 33 });
            session.resource_limits.max_collaboration_message_bytes = echo_size - 1;
            assert!(state.len() <= session.resource_limits.max_encoded_state_bytes);
            let before = complete_snapshot(&session);
            let error = if replace {
                session.replace_encoded_state(state.clone()).unwrap_err()
            } else {
                session.apply_encoded_state(state.clone()).unwrap_err()
            };
            assert_eq!(error.code(), "INPUT_LIMIT_EXCEEDED");
            assert_eq!(complete_snapshot(&session), before);
        }

        let mut local = session_with_local_and_remote_awareness(34);
        let before = complete_snapshot(&local);
        local.resource_limits.max_collaboration_message_bytes = 1;
        let error = local.apply_local_document(document).unwrap_err();
        assert_eq!(error.code(), "INPUT_LIMIT_EXCEEDED");
        assert_eq!(complete_snapshot(&local), before);
    }

    #[test]
    fn outbound_sync_start_and_awareness_limits_are_bounded_and_atomic() {
        let mut session = session_with_local_and_remote_awareness(41);
        session.apply_local_document(json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "full state response" }] }]
        })).unwrap();
        let sync_step_1 = encode_message(Message::Sync(SyncMessage::SyncStep1(
            StateVector::default(),
        )));
        let before = complete_snapshot(&session);
        session.resource_limits.max_collaboration_message_bytes = sync_step_1.len();
        let error = session.handle_message(sync_step_1).unwrap_err();
        assert_eq!(error.code(), "INPUT_LIMIT_EXCEEDED");
        assert_eq!(complete_snapshot(&session), before);

        session.resource_limits.max_collaboration_message_bytes = 1;
        assert_eq!(session.start().unwrap_err().code(), "INPUT_LIMIT_EXCEEDED");
        assert_eq!(complete_snapshot(&session), before);

        let error = session
            .set_local_awareness(json!({ "user": { "name": "Changed" } }))
            .unwrap_err();
        assert_eq!(error.code(), "INPUT_LIMIT_EXCEEDED");
        assert_eq!(complete_snapshot(&session), before);

        let error = session.clear_local_awareness().unwrap_err();
        assert_eq!(error.code(), "INPUT_LIMIT_EXCEEDED");
        assert_eq!(complete_snapshot(&session), before);
    }
}
