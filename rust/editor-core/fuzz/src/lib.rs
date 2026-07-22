//! White-box fuzz harness library (Task 16C).
//!
//! The Yrs engine and its supporting modules are crate-private since the
//! production cutover (the shipped surface is the UniFFI v2 ABI plus the
//! version query), so this fuzz crate compiles the retained engine sources
//! directly via `#[path]` — the exact same files `editor-core` compiles —
//! under the `editor_core` library name the fuzz targets import. No legacy
//! code is included; the legacy runtime was deleted in Task 16C.
#![allow(dead_code)]
#![allow(unused_imports)]

#[path = "../../src/boundary.rs"]
pub mod boundary;
#[path = "command_planner_shim/command_planner.rs"]
pub(crate) mod command_planner;
#[path = "../../src/collaboration_runtime/mod.rs"]
pub mod collaboration_runtime;
#[path = "../../src/document_api.rs"]
pub mod document_api;
#[path = "../../src/editor_state.rs"]
pub mod editor_state;
#[path = "../../src/model/mod.rs"]
pub mod model;
#[path = "../../src/native_transaction_bridge.rs"]
pub mod native_transaction_bridge;
#[path = "../../src/position/mod.rs"]
pub mod position;
#[path = "../../src/registry.rs"]
pub mod registry;
#[path = "../../src/render/mod.rs"]
pub mod render;
#[path = "../../src/schema/mod.rs"]
pub mod schema;
#[path = "../../src/selection/mod.rs"]
pub mod selection;
#[path = "../../src/serialize/mod.rs"]
pub mod serialize;
#[path = "../../src/session.rs"]
pub(crate) mod session;
#[path = "../../src/transform/mod.rs"]
pub mod transform;
#[path = "../../src/yrs_engine/mod.rs"]
pub mod yrs_engine;

pub use schema::presets::{prosemirror_schema, tiptap_schema};
