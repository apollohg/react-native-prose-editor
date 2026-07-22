pub mod html_in;
pub mod html_out;
pub mod json_in;
pub mod json_out;

// Not reachable from production call paths after the Task 16C legacy runtime
// removal; exercised by crate tests.
#[allow(unused_imports)]
pub use html_in::{from_html, from_html_with_limits, FromHtmlOptions, ParseError};
pub use html_out::to_html;
// Not reachable from production call paths after the Task 16C legacy runtime
// removal; exercised by crate tests.
#[allow(unused_imports)]
pub use json_in::{
    from_prosemirror_json, from_prosemirror_json_with_limits, JsonParseError, UnknownTypeMode,
};
pub(crate) use json_in::{
    normalized_wire_json_node_type, parse_wire_heading_level_str, rehydrate_reserved_html_opaque,
};
pub(crate) use json_out::node_to_json as node_to_prosemirror_json;
pub use json_out::to_prosemirror_json;

fn default_node_attrs(
    spec: &crate::schema::NodeSpec,
) -> std::collections::HashMap<String, serde_json::Value> {
    spec.attrs
        .iter()
        .filter_map(|(name, attr)| attr.default.clone().map(|value| (name.clone(), value)))
        .collect()
}
