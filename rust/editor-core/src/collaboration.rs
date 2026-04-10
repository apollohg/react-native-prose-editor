use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use serde_json::{json, Map, Value};
use yrs::any::Any;
use yrs::branch::{Branch, BranchPtr};
use yrs::sync::awareness::Awareness;
use yrs::sync::protocol::{DefaultProtocol, Message, Protocol, SyncMessage};
use yrs::types::text::{Text, YChange};
use yrs::types::xml::{
    Xml, XmlElementPrelim, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextPrelim,
    XmlTextRef,
};
use yrs::types::{Attrs, ToJson};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::{Encode, Encoder, EncoderV1};
use yrs::{
    Assoc, Doc, GetString, ReadTxn, StateVector, StickyIndex, Transact, TransactionMut, Update,
    WriteTxn,
};

use crate::schema::presets::{prosemirror_schema, tiptap_schema};
use crate::schema::Schema;

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
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let session = CollaborationSession::new(config_json);
        let mut map = global_registry()
            .lock()
            .expect("collaboration registry lock poisoned");
        map.insert(id, Arc::new(Mutex::new(session)));
        id
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

#[derive(Debug, Clone)]
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
    void_element_tags: HashSet<String>,
    cached_document_json: Value,
    document_revision: u64,
    cached_peers: Vec<CollaborationPeer>,
    cached_peer_states: HashMap<u64, CachedPeerState>,
    peers_revision: u64,
    local_awareness_state: Option<Value>,
}

fn empty_document_json() -> Value {
    json!({
        "type": "doc",
        "content": [],
    })
}

impl CollaborationSession {
    pub fn new(config_json: &str) -> Self {
        let config: Value = serde_json::from_str(config_json).unwrap_or_else(|_| json!({}));
        let client_id = config.get("clientId").and_then(Value::as_u64);
        let doc = if let Some(client_id) = client_id {
            Doc::with_client_id(client_id)
        } else {
            Doc::new()
        };
        let awareness = Awareness::new(doc.clone());
        let fragment_name = config
            .get("fragmentName")
            .and_then(Value::as_str)
            .unwrap_or("prosemirror")
            .to_string();
        let void_element_tags = void_element_tags_from_config(&config);

        let mut session = Self {
            doc,
            awareness,
            fragment_name,
            void_element_tags,
            cached_document_json: empty_document_json(),
            document_revision: 0,
            cached_peers: Vec::new(),
            cached_peer_states: HashMap::new(),
            peers_revision: 0,
            local_awareness_state: None,
        };

        if let Some(initial_json) = config.get("initialDocumentJson") {
            session.replace_document(initial_json.clone());
        } else {
            session.refresh_cached_document_json();
        }

        if let Some(local_awareness) = config.get("localAwareness") {
            session.set_local_awareness(local_awareness.clone());
        }

        session.refresh_cached_peers();

        session
    }

    pub fn encoded_state(&self) -> Vec<u8> {
        self.doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
    }

    pub fn apply_encoded_state(
        &mut self,
        encoded_state: Vec<u8>,
    ) -> Result<CollaborationResult, String> {
        self.merge_encoded_state(encoded_state)
    }

    pub fn replace_encoded_state(
        &mut self,
        encoded_state: Vec<u8>,
    ) -> Result<CollaborationResult, String> {
        self.reset_shared_state();
        self.merge_encoded_state(encoded_state)
    }

    fn merge_encoded_state(
        &mut self,
        encoded_state: Vec<u8>,
    ) -> Result<CollaborationResult, String> {
        let previous_document_revision = self.document_revision;
        let previous_peers_revision = self.peers_revision;

        let update = Update::decode_v1(encoded_state.as_slice())
            .map_err(|error| format!("invalid encoded state: {error}"))?;

        self.doc
            .transact_mut()
            .apply_update(update)
            .map_err(|error| format!("failed to apply encoded state: {error}"))?;

        self.refresh_cached_document_json();
        self.refresh_cached_peers();

        let mut messages = Vec::new();
        if !encoded_state.is_empty() {
            messages.push(encode_message(Message::Sync(SyncMessage::Update(
                encoded_state,
            ))));
        }
        Ok(self.finish_result(
            previous_document_revision,
            previous_peers_revision,
            messages,
        ))
    }

    pub fn start(&mut self) -> CollaborationResult {
        let mut result = CollaborationResult::empty();
        result.messages.push(self.encode_sync_step_1());
        if let Some(message) = self.encode_local_awareness_message() {
            result.messages.push(message);
        }
        result.peers_changed = true;
        result.peers = Some(self.cached_peers.clone());
        result
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

    pub fn apply_local_document(&mut self, next_json: Value) -> CollaborationResult {
        let previous_peers_revision = self.peers_revision;
        let previous_document_revision = self.document_revision;
        let previous_state_vector = self.doc.transact().state_vector();

        {
            let current_children = self
                .cached_document_json
                .get("content")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let next_children = next_json
                .get("content")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mut txn = self.doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment(self.fragment_name.as_str());
            apply_children(&fragment, &mut txn, &current_children, &next_children);
        }

        self.refresh_cached_document_json();
        self.refresh_cached_peers();
        let update = self.doc.transact().encode_diff_v1(&previous_state_vector);
        let messages = if update.is_empty() {
            Vec::new()
        } else {
            vec![encode_message(Message::Sync(SyncMessage::Update(update)))]
        };
        self.finish_result(
            previous_document_revision,
            previous_peers_revision,
            messages,
        )
    }

    pub fn handle_message(&mut self, message: Vec<u8>) -> Result<CollaborationResult, String> {
        let previous_document_revision = self.document_revision;
        let previous_peers_revision = self.peers_revision;
        let refresh_scope = refresh_scope_for_message(message.as_slice());

        let protocol = DefaultProtocol;
        let mut responses = Vec::new();
        let replies = protocol
            .handle(&self.awareness, &message)
            .map_err(|error| format!("invalid collaboration message: {error}"))?;
        for reply in replies {
            responses.push(encode_message(reply));
        }

        if refresh_scope.document {
            self.refresh_cached_document_json();
        }
        if refresh_scope.peers || refresh_scope.document {
            self.refresh_cached_peers();
        }

        Ok(self.finish_result(
            previous_document_revision,
            previous_peers_revision,
            responses,
        ))
    }

    pub fn set_local_awareness(&mut self, next_state: Value) -> CollaborationResult {
        let previous_document_revision = self.document_revision;
        let previous_peers_revision = self.peers_revision;
        let next_state = self.normalize_awareness_state(next_state);
        self.local_awareness_state = Some(next_state.clone());
        if self.awareness.set_local_state(&next_state).is_err() {
            return CollaborationResult::empty();
        }
        self.refresh_cached_peers();

        let messages = self
            .encode_local_awareness_message()
            .into_iter()
            .collect::<Vec<_>>();
        self.finish_result(
            previous_document_revision,
            previous_peers_revision,
            messages,
        )
    }

    pub fn clear_local_awareness(&mut self) -> CollaborationResult {
        let previous_document_revision = self.document_revision;
        let previous_peers_revision = self.peers_revision;
        self.local_awareness_state = None;
        self.awareness.remove_state(self.doc.client_id());
        self.refresh_cached_peers();

        self.finish_result(
            previous_document_revision,
            previous_peers_revision,
            Vec::new(),
        )
    }

    fn replace_document(&mut self, next_json: Value) {
        let mut txn = self.doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment(self.fragment_name.as_str());
        let len = fragment.len(&txn);
        if len > 0 {
            fragment.remove_range(&mut txn, 0, len);
        }
        if let Some(content) = next_json.get("content").and_then(Value::as_array) {
            for node in content {
                let next_index = fragment.len(&txn);
                insert_json_node(&fragment, &mut txn, next_index, node);
            }
        }
        drop(txn);
        self.refresh_cached_document_json();
        self.refresh_cached_peers();
    }

    fn reset_shared_state(&mut self) {
        let client_id = self.doc.client_id();
        self.doc = Doc::with_client_id(client_id);
        self.awareness = Awareness::new(self.doc.clone());
        if let Some(local_awareness_state) = self.local_awareness_state.clone() {
            let _ = self.awareness.set_local_state(&local_awareness_state);
        }
        self.refresh_cached_document_json();
        self.refresh_cached_peers();
    }

    fn encode_sync_step_1(&self) -> Vec<u8> {
        let state_vector = self.doc.transact().state_vector();
        encode_message(Message::Sync(SyncMessage::SyncStep1(state_vector)))
    }

    fn encode_local_awareness_message(&self) -> Option<Vec<u8>> {
        self.local_awareness_state.as_ref()?;
        let update = self.awareness.update().ok()?;
        Some(encode_message(Message::Awareness(update)))
    }

    fn refresh_cached_document_json(&mut self) -> bool {
        let txn = self.doc.transact();
        let next_document_json = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .map(|fragment| xml_fragment_to_document_json(&fragment, &txn))
            .unwrap_or_else(empty_document_json);
        if next_document_json == self.cached_document_json {
            return false;
        }
        self.cached_document_json = next_document_json;
        self.document_revision = self.document_revision.wrapping_add(1);
        true
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

        if !object.contains_key("selection") {
            if let Some(selection) = self.selection_from_cursor_value(object.get("cursor")) {
                object.insert("selection".to_string(), selection);
            }
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
                let text_len = text.get_string(txn).chars().count() as u32;
                if target_index <= text_len {
                    return Some(node_start + target_index);
                }
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
                let text_len = text.get_string(txn).chars().count() as u32;
                if target_index <= branch_index + text_len {
                    return Some(doc_pos + (target_index - branch_index));
                }
                branch_index += text_len;
                doc_pos += text_len;
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
                let text_len = text.get_string(txn).chars().count() as u32;
                if doc_pos <= consumed_pm + text_len {
                    return StickyIndex::at(
                        txn,
                        BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(text)),
                        doc_pos - consumed_pm,
                        assoc,
                    );
                }
                branch_index += text_len;
                consumed_pm += text_len;
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

fn default_void_element_tags() -> HashSet<String> {
    let mut tags = void_element_tags_from_schema(&tiptap_schema());
    tags.extend(void_element_tags_from_schema(&prosemirror_schema()));
    tags.extend(
        ["mention", "__opaque", "__opaque_json", "__skip"]
            .into_iter()
            .map(str::to_string),
    );
    tags
}

fn void_element_tags_from_config(config: &Value) -> HashSet<String> {
    if let Some(schema_json) = config.get("schema") {
        if let Ok(schema) = Schema::from_json(schema_json) {
            let mut tags = void_element_tags_from_schema(&schema);
            tags.extend(
                ["__opaque", "__opaque_json", "__skip"]
                    .into_iter()
                    .map(str::to_string),
            );
            return tags;
        }
    }

    default_void_element_tags()
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

fn xml_fragment_to_document_json<T: ReadTxn>(fragment: &XmlFragmentRef, txn: &T) -> Value {
    let content = fragment
        .children(txn)
        .flat_map(|child| xml_out_to_json(child, txn))
        .collect::<Vec<_>>();
    json!({
        "type": "doc",
        "content": content,
    })
}

fn xml_out_to_json<T: ReadTxn>(node: XmlOut, txn: &T) -> Vec<Value> {
    match node {
        XmlOut::Element(element) => vec![xml_element_to_json(&element, txn)],
        XmlOut::Text(text) => xml_text_to_json(&text, txn),
        XmlOut::Fragment(fragment) => fragment
            .children(txn)
            .flat_map(|child| xml_out_to_json(child, txn))
            .collect(),
    }
}

fn xml_element_to_json<T: ReadTxn>(element: &XmlElementRef, txn: &T) -> Value {
    let mut object = Map::new();
    let mut attrs = element
        .attributes(txn)
        .map(|(key, value)| (key.to_string(), any_to_json(&value.to_json(txn))))
        .collect::<Map<String, Value>>();
    let node_type = collaboration_json_node_type_for_element(element.tag(), &mut attrs);
    object.insert("type".to_string(), Value::String(node_type));
    if !attrs.is_empty() {
        object.insert("attrs".to_string(), Value::Object(attrs));
    }

    let children = element
        .children(txn)
        .flat_map(|child| xml_out_to_json(child, txn))
        .collect::<Vec<_>>();
    if !children.is_empty() {
        object.insert("content".to_string(), Value::Array(children));
    }

    Value::Object(object)
}

fn xml_text_to_json<T: ReadTxn>(text: &XmlTextRef, txn: &T) -> Vec<Value> {
    let mut nodes = Vec::new();
    for diff in text.diff(txn, YChange::identity) {
        let yrs::Out::Any(any) = diff.insert else {
            continue;
        };
        let Value::String(text_value) = any_to_json(&any) else {
            continue;
        };
        if text_value.is_empty() {
            continue;
        }
        let mut object = Map::new();
        object.insert("type".to_string(), Value::String("text".to_string()));
        object.insert("text".to_string(), Value::String(text_value));
        let marks = attrs_to_marks(diff.attributes.clone());
        if !marks.is_empty() {
            object.insert("marks".to_string(), Value::Array(marks));
        }
        nodes.push(Value::Object(object));
    }
    nodes
}

fn attrs_to_marks(attrs: Option<Box<Attrs>>) -> Vec<Value> {
    let mut marks = Vec::new();
    let Some(attrs) = attrs else {
        return marks;
    };
    for (name, value) in attrs.iter() {
        let mut object = Map::new();
        object.insert("type".to_string(), Value::String(name.to_string()));
        match any_to_json(value) {
            Value::Bool(true) => {}
            Value::Null => {}
            other => {
                object.insert("attrs".to_string(), other);
            }
        }
        marks.push(Value::Object(object));
    }
    marks
}

fn apply_children<P: XmlFragment>(
    parent: &P,
    txn: &mut TransactionMut<'_>,
    old_children: &[Value],
    new_children: &[Value],
) {
    let mut prefix = 0usize;
    while prefix < old_children.len()
        && prefix < new_children.len()
        && nodes_are_compatible(&old_children[prefix], &new_children[prefix])
    {
        apply_child_at(
            parent,
            txn,
            prefix as u32,
            &old_children[prefix],
            &new_children[prefix],
        );
        prefix += 1;
    }

    let mut old_suffix = old_children.len();
    let mut new_suffix = new_children.len();
    while old_suffix > prefix
        && new_suffix > prefix
        && nodes_are_compatible(&old_children[old_suffix - 1], &new_children[new_suffix - 1])
    {
        old_suffix -= 1;
        new_suffix -= 1;
    }

    let old_mid_len = old_suffix.saturating_sub(prefix) as u32;
    if old_mid_len > 0 {
        parent.remove_range(txn, prefix as u32, old_mid_len);
    }

    for (offset, node) in new_children[prefix..new_suffix].iter().enumerate() {
        insert_json_node(parent, txn, prefix as u32 + offset as u32, node);
    }

    let suffix_len = old_children.len().saturating_sub(old_suffix);
    for offset in 0..suffix_len {
        let new_index = new_suffix + offset;
        let parent_index = (new_suffix + offset) as u32;
        apply_child_at(
            parent,
            txn,
            parent_index,
            &old_children[old_suffix + offset],
            &new_children[new_index],
        );
    }
}

fn apply_child_at<P: XmlFragment>(
    parent: &P,
    txn: &mut TransactionMut<'_>,
    index: u32,
    old_node: &Value,
    new_node: &Value,
) {
    let Some(current) = parent.get(txn, index) else {
        return;
    };

    match current {
        XmlOut::Element(element) => apply_element_node(&element, txn, old_node, new_node),
        XmlOut::Text(text) => apply_text_node(&text, txn, old_node, new_node),
        _ => {}
    }
}

fn apply_element_node(
    element: &XmlElementRef,
    txn: &mut TransactionMut<'_>,
    old_node: &Value,
    new_node: &Value,
) {
    let old_attrs = old_node.get("attrs").and_then(Value::as_object);
    let new_attrs = new_node.get("attrs").and_then(Value::as_object);

    if old_attrs != new_attrs {
        let existing = element
            .attributes(txn)
            .map(|(key, _)| key.to_string())
            .collect::<Vec<_>>();
        for key in existing {
            element.remove_attribute(txn, &key);
        }
        if let Some(attrs) = new_attrs {
            for (key, value) in attrs {
                element.insert_attribute(txn, key.as_str(), json_to_any(value));
            }
        }
    }

    let old_children = old_node
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let new_children = new_node
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    apply_children(element, txn, &old_children, &new_children);
}

fn apply_text_node(
    text: &XmlTextRef,
    txn: &mut TransactionMut<'_>,
    old_node: &Value,
    new_node: &Value,
) {
    let old_marks = normalize_marks(old_node.get("marks").and_then(Value::as_array));
    let new_marks = normalize_marks(new_node.get("marks").and_then(Value::as_array));
    let old_text = old_node.get("text").and_then(Value::as_str).unwrap_or("");
    let new_text = new_node.get("text").and_then(Value::as_str).unwrap_or("");

    if old_marks != new_marks {
        let len = text.len(txn);
        if len > 0 {
            text.remove_range(txn, 0, len);
        }
        if !new_text.is_empty() {
            text.insert_with_attributes(txn, 0, new_text, marks_to_attrs(&new_marks));
        }
        return;
    }

    let (prefix, old_suffix, new_suffix) = shared_text_bounds(old_text, new_text);
    let remove_len = old_text[prefix..old_suffix].len() as u32;
    if remove_len > 0 {
        text.remove_range(txn, prefix as u32, remove_len);
    }

    let insert_text = &new_text[prefix..new_suffix];
    if !insert_text.is_empty() {
        text.insert_with_attributes(txn, prefix as u32, insert_text, marks_to_attrs(&new_marks));
    }
}

fn shared_text_bounds(old_text: &str, new_text: &str) -> (usize, usize, usize) {
    let mut prefix = 0usize;
    let mut old_iter = old_text.char_indices().peekable();
    let mut new_iter = new_text.char_indices().peekable();

    while let (Some((old_index, old_char)), Some((new_index, new_char))) =
        (old_iter.peek().copied(), new_iter.peek().copied())
    {
        if old_char != new_char || old_index != prefix || new_index != prefix {
            break;
        }
        prefix += old_char.len_utf8();
        old_iter.next();
        new_iter.next();
    }

    let mut old_suffix = old_text.len();
    let mut new_suffix = new_text.len();
    let old_tail = old_text[prefix..].chars().rev().collect::<Vec<_>>();
    let new_tail = new_text[prefix..].chars().rev().collect::<Vec<_>>();
    for (old_char, new_char) in old_tail.iter().zip(new_tail.iter()) {
        if old_char != new_char {
            break;
        }
        old_suffix -= old_char.len_utf8();
        new_suffix -= new_char.len_utf8();
    }

    (prefix, old_suffix, new_suffix)
}

fn insert_json_node<P: XmlFragment>(
    parent: &P,
    txn: &mut TransactionMut<'_>,
    index: u32,
    node: &Value,
) {
    let node_type = node.get("type").and_then(Value::as_str).unwrap_or("");
    if node_type == "text" {
        let text_value = node.get("text").and_then(Value::as_str).unwrap_or("");
        let text = parent.insert(txn, index, XmlTextPrelim::new(""));
        if !text_value.is_empty() {
            text.insert_with_attributes(
                txn,
                0,
                text_value,
                marks_to_attrs(&normalize_marks(
                    node.get("marks").and_then(Value::as_array),
                )),
            );
        }
        return;
    }

    let mut attrs = node
        .get("attrs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let element_name = collaboration_element_name_for_json_node(node_type, &mut attrs);
    let element = parent.insert(txn, index, XmlElementPrelim::empty(element_name.as_str()));
    for (key, value) in &attrs {
        element.insert_attribute(txn, key.as_str(), json_to_any(value));
    }
    if let Some(children) = node.get("content").and_then(Value::as_array) {
        for child in children {
            let next_index = element.len(txn);
            insert_json_node(&element, txn, next_index, child);
        }
    }
}

fn collaboration_json_node_type_for_element(tag: &str, attrs: &mut Map<String, Value>) -> String {
    if tag == "heading" {
        if let Some(level) = parse_heading_level_value(attrs.get("level")) {
            attrs.remove("level");
            return format!("h{level}");
        }
    }

    tag.to_string()
}

fn collaboration_element_name_for_json_node(
    node_type: &str,
    attrs: &mut Map<String, Value>,
) -> String {
    if let Some(level) = heading_level_from_internal_node_type(node_type) {
        attrs.insert("level".to_string(), Value::Number(u64::from(level).into()));
        return "heading".to_string();
    }

    node_type.to_string()
}

fn heading_level_from_internal_node_type(node_type: &str) -> Option<u8> {
    let suffix = node_type.strip_prefix('h')?;
    if suffix.len() != 1 {
        return None;
    }
    let level = suffix.parse::<u8>().ok()?;
    if (1..=6).contains(&level) {
        Some(level)
    } else {
        None
    }
}

fn parse_heading_level_value(value: Option<&Value>) -> Option<u8> {
    let value = value?;
    let level = match value {
        Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                u8::try_from(value).ok()?
            } else if let Some(value) = number.as_i64() {
                u8::try_from(value).ok()?
            } else if let Some(value) = number.as_f64() {
                if !value.is_finite() || value.fract() != 0.0 {
                    return None;
                }
                let rounded = value as i64;
                u8::try_from(rounded).ok()?
            } else {
                return None;
            }
        }
        Value::String(value) => value.parse::<u8>().ok()?,
        _ => return None,
    };

    if (1..=6).contains(&level) {
        Some(level)
    } else {
        None
    }
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

fn marks_to_attrs(marks: &[Value]) -> Attrs {
    let mut attrs = Attrs::default();
    for mark in marks {
        let Some(mark_type) = mark.get("type").and_then(Value::as_str) else {
            continue;
        };
        let value = mark
            .get("attrs")
            .map(json_to_any)
            .unwrap_or_else(|| Any::Bool(true));
        attrs.insert(mark_type.into(), value);
    }
    attrs
}

fn nodes_are_compatible(old_node: &Value, new_node: &Value) -> bool {
    let old_type = old_node.get("type").and_then(Value::as_str);
    let new_type = new_node.get("type").and_then(Value::as_str);
    if old_type != new_type {
        return false;
    }

    match old_type {
        Some("text") => {
            normalize_marks(old_node.get("marks").and_then(Value::as_array))
                == normalize_marks(new_node.get("marks").and_then(Value::as_array))
        }
        Some(_) => true,
        None => false,
    }
}

fn json_to_any(value: &Value) -> Any {
    match value {
        Value::Null => Any::Null,
        Value::Bool(value) => Any::Bool(*value),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                Any::BigInt(value)
            } else if let Some(value) = number.as_u64() {
                Any::Number(value as f64)
            } else if let Some(value) = number.as_f64() {
                Any::Number(value)
            } else {
                Any::Null
            }
        }
        Value::String(value) => Any::String(value.clone().into()),
        Value::Array(values) => Any::Array(values.iter().map(json_to_any).collect()),
        Value::Object(values) => Any::Map(Arc::new(
            values
                .iter()
                .map(|(key, value)| (key.clone(), json_to_any(value)))
                .collect(),
        )),
    }
}

fn any_to_json(value: &Any) -> Value {
    match value {
        Any::Null => Value::Null,
        Any::Undefined => Value::Null,
        Any::Bool(value) => Value::Bool(*value),
        Any::Number(value) => serde_json::Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Any::BigInt(value) => Value::Number((*value).into()),
        Any::String(value) => Value::String(value.to_string()),
        Any::Buffer(value) => Value::Array(
            value
                .iter()
                .map(|byte| Value::Number((*byte).into()))
                .collect(),
        ),
        Any::Array(values) => Value::Array(values.iter().map(any_to_json).collect()),
        Any::Map(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.to_string(), any_to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use yrs::IndexedSequence;

    fn apply_messages_to_peer(peer: &Awareness, messages: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        let protocol = DefaultProtocol;
        let mut responses = Vec::new();
        for message in messages {
            let replies = protocol.handle(peer, &message).unwrap();
            responses.extend(replies.into_iter().map(encode_message));
        }
        responses
    }

    fn peer_document_json(peer: &Awareness) -> Value {
        let txn = peer.doc().transact();
        txn.get_xml_fragment("prosemirror")
            .map(|fragment| xml_fragment_to_document_json(&fragment, &txn))
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

        let result = session.apply_local_document(json!({
            "type": "doc",
            "content": [{
                "type": "h2",
                "content": [{
                    "type": "text",
                    "text": "Heading",
                    "marks": [{ "type": "bold" }]
                }]
            }]
        }));
        let _ = apply_messages_to_peer(&peer, result.messages);

        let txn = peer.doc().transact();
        let fragment = txn
            .get_xml_fragment("default")
            .expect("peer fragment should exist");
        let XmlOut::Element(heading) = fragment.get(&txn, 0).expect("heading should exist") else {
            panic!("expected heading element");
        };

        assert_eq!(heading.tag().as_ref(), "heading");
        assert_eq!(
            heading
                .get_attribute(&txn, "level")
                .map(|value| any_to_json(&value.to_json(&txn))),
            Some(json!(2))
        );
    }

    #[test]
    fn collaboration_sessions_round_trip_document_updates() {
        let mut left = CollaborationSession::new(
            r#"{"clientId":1,"initialDocumentJson":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"hello"}]}]}}"#,
        );
        let mut right = CollaborationSession::new(r#"{"clientId":2}"#);

        for message in left.start().messages {
            let reply = right.handle_message(message).unwrap();
            for response in reply.messages {
                let _ = left.handle_message(response).unwrap();
            }
        }
        for message in right.start().messages {
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
        let apply = left.apply_local_document(local.clone());
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
        let peer_replies = apply_messages_to_peer(&peer, session.start().messages);
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

        let session_awareness = session.set_local_awareness(json!({
            "user": {
                "name": "Session"
            },
            "selection": {
                "anchor": 2,
                "head": 4
            }
        }));
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

        let awareness = session.set_local_awareness(json!({
            "user": {
                "name": "Session"
            },
            "selection": {
                "anchor": 2,
                "head": 4
            },
            "focused": true
        }));
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

        let document_update = sender.apply_local_document(document_with_horizontal_rule.clone());
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
}
