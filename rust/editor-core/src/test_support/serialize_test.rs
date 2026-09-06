use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::{AttrSpec, MarkSpec, NodeRole, NodeSpec, Schema};
use crate::serialize::{
    from_html, from_prosemirror_json, to_html, to_prosemirror_json, FromHtmlOptions,
    JsonParseError, UnknownTypeMode,
};
use crate::transform::DocumentValidator;

fn schema() -> Schema {
    crate::tiptap_schema()
}

fn atom_rules_schema() -> Schema {
    Schema::from_json(&super::schema_test::atom_schema_json("counter-card")).unwrap()
}

fn atom_rules_schema_with_array_attr() -> Schema {
    let mut json = super::schema_test::atom_schema_json("counter-card");
    json["nodes"][3]["attrs"] = serde_json::json!({ "sets": { "default": [] } });
    json["nodes"][3]["html"]["attrMap"] = serde_json::json!({ "sets": "data-sets" });
    Schema::from_json(&json).unwrap()
}

include!("serialize_test/schema_projection.rs");

include!("serialize_test/html_output.rs");

include!("serialize_test/html_input.rs");

include!("serialize_test/html_roundtrip.rs");

include!("serialize_test/json_output.rs");

include!("serialize_test/json_input.rs");

include!("serialize_test/json_roundtrip.rs");
