use editor_core::boundary::ResourceLimits;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    DocumentScope, DocumentSnapshot, InitializationMode, YrsDocumentEngine, YrsEngineConfig,
    SNAPSHOT_FORMAT_VERSION,
};
use serde_json::{json, Value};
use yrs::types::text::{Text, YChange};
use yrs::types::xml::{XmlFragment, XmlOut, XmlTextRef};
use yrs::updates::decoder::Decode;
use yrs::{Assoc, ClientID, Doc, OffsetKind, Options, ReadTxn, StickyIndex, Transact, Update};

const FIXTURE_JSON: &str = include_str!("fixtures/yrs-025-update-v1.json");
const FRAGMENT_NAME: &str = "prosemirror";

fn fixture() -> Value {
    serde_json::from_str(FIXTURE_JSON).expect("the checked-in Yrs 0.25 fixture must be valid JSON")
}

fn decode_hex(encoded: &str) -> Vec<u8> {
    assert_eq!(encoded.len() % 2, 0, "fixture hex must contain whole bytes");
    (0..encoded.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&encoded[index..index + 2], 16)
                .expect("fixture must contain lowercase hexadecimal bytes")
        })
        .collect()
}

fn fixture_encoded_state(fixture: &Value) -> Vec<u8> {
    decode_hex(
        fixture["encodedStateHex"]
            .as_str()
            .expect("fixture must contain encodedStateHex"),
    )
}

fn restored_yrs_document(encoded_state: &[u8]) -> Option<Doc> {
    let mut options = Options::with_client_id(ClientID::new(9001));
    options.offset_kind = OffsetKind::Utf16;
    let doc = Doc::with_options(options);
    let update = Update::decode_v1(encoded_state).ok()?;
    doc.transact_mut().apply_update(update).ok()?;
    Some(doc)
}

fn restores_equivalent_state(fixture: &Value, encoded_state: &[u8]) -> bool {
    let scope = DocumentScope {
        document_id: "yrs-025-fixture".into(),
        lineage_id: "yrs-upgrade".into(),
    };
    let mut engine = match YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: FRAGMENT_NAME.into(),
        initialization_mode: InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        scope: Some(scope.clone()),
    }) {
        Ok(engine) => engine,
        Err(_) => return false,
    };
    let snapshot = DocumentSnapshot {
        format_version: SNAPSHOT_FORMAT_VERSION,
        document_id: scope.document_id,
        lineage_id: scope.lineage_id,
        fragment_name: FRAGMENT_NAME.into(),
        schema_fingerprint: engine.schema_fingerprint().into(),
        encoded_state: encoded_state.to_vec(),
    };

    engine.restore_snapshot(&snapshot).is_ok()
        && engine.document_json().as_ref() == Some(&fixture["canonicalJson"])
}

fn utf16_offset_to_scalar(text: &str, utf16_offset: u32) -> Option<u32> {
    let mut utf16 = 0u32;
    let mut scalars = 0u32;
    for character in text.chars() {
        if utf16 == utf16_offset {
            return Some(scalars);
        }
        utf16 = utf16.checked_add(character.len_utf16() as u32)?;
        scalars = scalars.checked_add(1)?;
    }
    (utf16 == utf16_offset).then_some(scalars)
}

fn plain_text<T: ReadTxn>(text: &XmlTextRef, txn: &T) -> String {
    text.diff(txn, YChange::identity)
        .into_iter()
        .filter_map(|diff| match diff.insert {
            yrs::Out::Any(yrs::Any::String(value)) => Some(value),
            _ => None,
        })
        .fold(String::new(), |mut output, value| {
            output.push_str(&value);
            output
        })
}

fn sticky_document_position<T: ReadTxn>(txn: &T, sticky_index: &StickyIndex) -> Option<u32> {
    let offset = sticky_index.get_offset(txn)?;
    let fragment = txn.get_xml_fragment(FRAGMENT_NAME)?;
    let paragraph = match fragment.children(txn).next()? {
        XmlOut::Element(element) if element.tag().as_ref() == "paragraph" => element,
        _ => return None,
    };
    let mut position = 1u32;
    for child in paragraph.children(txn) {
        match child {
            XmlOut::Text(text) => {
                let branch = yrs::branch::BranchPtr::from(<XmlTextRef as AsRef<
                    yrs::branch::Branch,
                >>::as_ref(&text));
                let value = plain_text(&text, txn);
                if branch == offset.branch {
                    return position.checked_add(utf16_offset_to_scalar(&value, offset.index)?);
                }
                position = position.checked_add(value.chars().count() as u32)?;
            }
            XmlOut::Element(element) if element.tag().as_ref() == "hardBreak" => {
                position = position.checked_add(1)?;
            }
            _ => return None,
        }
    }
    None
}

#[test]
fn restores_yrs_025_update_with_identical_json_state_vector_and_sticky_positions() {
    let fixture = fixture();
    let encoded_state = fixture_encoded_state(&fixture);
    assert!(restores_equivalent_state(&fixture, &encoded_state));

    let doc = restored_yrs_document(&encoded_state).expect("Yrs must decode its 0.25 update-v1");
    let txn = doc.transact();
    let mut state_vector = txn
        .state_vector()
        .iter()
        .map(|(&client, &clock)| json!({ "client": client, "clock": clock }))
        .collect::<Vec<_>>();
    state_vector.sort_by_key(|entry| entry["client"].as_u64().unwrap());
    assert_eq!(Value::Array(state_vector), fixture["stateVector"]);

    for expected in fixture["stickyPositions"].as_array().unwrap() {
        let sticky_index: StickyIndex = serde_json::from_value(expected["stickyIndex"].clone())
            .expect("Yrs must deserialize its 0.25 sticky index");
        assert_eq!(
            sticky_index.assoc,
            match expected["affinity"].as_str().unwrap() {
                "before" => Assoc::Before,
                "after" => Assoc::After,
                other => panic!("unexpected fixture affinity {other}"),
            }
        );
        assert_eq!(
            sticky_document_position(&txn, &sticky_index),
            expected["position"]
                .as_u64()
                .map(|position| position as u32),
            "sticky fixture entry {expected}"
        );
    }
}

#[test]
fn corrupted_yrs_025_update_is_not_accepted_as_equivalent() {
    let fixture = fixture();
    let mut encoded_state = fixture_encoded_state(&fixture);
    let midpoint = encoded_state.len() / 2;
    encoded_state[midpoint] ^= 0x80;

    assert!(!restores_equivalent_state(&fixture, &encoded_state));
}
