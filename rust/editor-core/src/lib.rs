//! editor-core: Yrs-backed rich-text editing engine.
//!
//! The only intentional direct Rust semver surface is
//! [`editor_core_version`]. Everything else reachable from the crate is the
//! UniFFI-generated surface declared under [`ffi_v2`]; all engine internals
//! are crate-private. The legacy pre-Yrs runtime (the legacy
//! editor/collaboration FFI, the standalone/document backends, and the legacy
//! undo history) was deleted from this crate per the 2026-07-20 user
//! directive ("we don't need to keep legacy code"); no legacy code is
//! retained anywhere in the workspace.

pub(crate) mod boundary;
pub(crate) mod collaboration_runtime;
pub(crate) mod command_planner;
pub(crate) mod document_api;
pub(crate) mod editor_state;
pub mod ffi_v2;
pub(crate) mod model;
pub(crate) mod native_transaction_bridge;
pub(crate) mod position;
pub(crate) mod position_epoch;
pub(crate) mod registry;
pub(crate) mod render;
pub(crate) mod schema;
pub(crate) mod selection;
pub(crate) mod serialize;
#[allow(dead_code)]
pub(crate) mod session;
#[cfg(test)]
mod test_support;
pub(crate) mod transform;
pub mod viewer;
pub(crate) mod yrs_engine;

#[cfg(test)]
pub(crate) use document_api::session_initialization_test_support;
#[cfg(test)]
pub(crate) use native_transaction_bridge::native_bridge_test_support;
#[cfg(test)]
pub(crate) use registry::session_lifecycle_test_support;
#[cfg(test)]
pub(crate) use schema::presets::{prosemirror_schema, tiptap_schema};

uniffi::setup_scaffolding!();

/// Return the crate version string.
#[uniffi::export]
pub fn editor_core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_editor_core_version() {
        let version = editor_core_version();
        assert_eq!(
            version,
            env!("CARGO_PKG_VERSION"),
            "editor_core_version() should return the crate version from Cargo.toml"
        );
    }

    #[test]
    fn test_editor_core_version_is_valid_semver() {
        let version = editor_core_version();
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "Version '{}' should have exactly 3 semver components (major.minor.patch)",
            version
        );
        for (i, part) in parts.iter().enumerate() {
            let label = match i {
                0 => "major",
                1 => "minor",
                2 => "patch",
                _ => unreachable!(),
            };
            part.parse::<u32>().unwrap_or_else(|_| {
                panic!(
                    "Version component '{}' ({}) should be a valid u32",
                    part, label
                )
            });
        }
    }
}
