// This white-box harness compiles a superset of the engine sources; items
// the benchmark does not drive are expected to be unused here.
#![allow(dead_code)]
#![allow(unused_imports)]

use std::env;
use std::hint::black_box;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

// The engine modules are crate-private, so this white-box harness compiles
// them directly via `#[path]` — the same files the shipped crate compiles —
// rather than through the crate API. Expectations below are derived from the
// same render/position/active-state paths the v2 render accessor uses.
#[path = "../src/boundary.rs"]
mod boundary;
#[path = "../src/collaboration_runtime/mod.rs"]
mod collaboration_runtime;
#[path = "command_planner_shim/command_planner.rs"]
mod command_planner;
#[path = "../src/document_api.rs"]
mod document_api;
#[path = "../src/editor_state.rs"]
mod editor_state;
// Path-included engine sources retain their production dependency on the v2
// wire primitives. Re-export only that shared types module at the benchmark
// crate root; the benchmark must not pull in the v2 export entrypoints.
#[path = "../src/ffi_v2/types.rs"]
pub(crate) mod ffi_v2_types;
pub(crate) mod ffi_v2 {
    pub(crate) use super::ffi_v2_types as types;
}
uniffi::setup_scaffolding!();
#[path = "../src/model/mod.rs"]
mod model;
#[path = "../src/native_transaction_bridge.rs"]
mod native_transaction_bridge;
#[path = "../src/position/mod.rs"]
mod position;
#[path = "../src/registry.rs"]
mod registry;
#[path = "../src/render/mod.rs"]
mod render;
#[path = "../src/schema/mod.rs"]
mod schema;
#[path = "../src/selection/mod.rs"]
mod selection;
#[path = "../src/serialize/mod.rs"]
mod serialize;
#[path = "../src/session.rs"]
mod session;
#[path = "../src/transform/mod.rs"]
mod transform;
#[path = "../src/yrs_engine/mod.rs"]
mod yrs_engine;

use crate::boundary::ResourceLimits;
use crate::render::RenderElement;
use crate::transform::DocumentValidator;
use crate::yrs_engine::{
    Affinity, EditorOffsetKind, HistoryPolicy, InitializationMode, RenderUpdate, ResolvedSelection,
    RevisionedPosition, SelectionInput, SelectionIntent, TransactionOrigin, TypedCommand,
    TypedTransaction, TypedTransactionResult, YrsDocumentEngine, YrsEngineConfig,
};
// `cargo bench` compiles this target with the `test` cfg, so the included
// sources' inline test modules compile as well; they reference the crate-root
// schema presets and the session-registry concurrency guard.
pub use schema::presets::{prosemirror_schema, tiptap_schema};

#[cfg(test)]
mod test_support {
    pub(crate) use registry_concurrency::RegistryConcurrencyGuard;

    mod registry_concurrency {
        use std::cell::Cell;
        use std::sync::{Mutex, MutexGuard, OnceLock};

        thread_local! {
            static REGISTRY_GUARD_DEPTH: Cell<usize> = const { Cell::new(0) };
        }

        static REGISTRY_GUARD: OnceLock<Mutex<()>> = OnceLock::new();

        pub(crate) struct RegistryConcurrencyGuard {
            _guard: Option<MutexGuard<'static, ()>>,
        }

        impl RegistryConcurrencyGuard {
            pub(crate) fn acquire() -> Self {
                let already_held = REGISTRY_GUARD_DEPTH.with(|depth| {
                    let held = depth.get() > 0;
                    depth.set(depth.get() + 1);
                    held
                });
                if already_held {
                    return Self { _guard: None };
                }
                let guard = REGISTRY_GUARD
                    .get_or_init(|| Mutex::new(()))
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                Self {
                    _guard: Some(guard),
                }
            }

            pub(crate) fn inherit_for_spawned_thread() -> Self {
                REGISTRY_GUARD_DEPTH.with(|depth| depth.set(depth.get() + 1));
                Self { _guard: None }
            }
        }

        impl Drop for RegistryConcurrencyGuard {
            fn drop(&mut self) {
                REGISTRY_GUARD_DEPTH.with(|depth| depth.set(depth.get() - 1));
            }
        }
    }
}

use serde_json::{json, Value};

#[path = "support/benchmark_filter.rs"]
mod benchmark_filter;

const EDITING_TYPING_BURST: usize = 20;
const EDITING_CURSOR_SCALAR: u32 = 44;

struct EditingCaseExpectation {
    before: Value,
    after: Value,
}

#[derive(Clone, Copy)]
enum BenchMode {
    Quick,
    Standard,
}

impl BenchMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Standard => "standard",
        }
    }

    fn profile(self) -> BenchProfile {
        match self {
            Self::Quick => BenchProfile {
                warmup_iterations: 2,
                iterations: 8,
                article_blocks: 48,
                paragraph_chars: 140,
                mapping_points: 192,
                selection_width: 64,
                typing_burst: 24,
                selection_scrub_points: 48,
                awareness_peer_count: 12,
                opaque_payload_bytes: 32 * 1024,
            },
            Self::Standard => BenchProfile {
                warmup_iterations: 4,
                iterations: 20,
                article_blocks: 160,
                paragraph_chars: 220,
                mapping_points: 768,
                selection_width: 160,
                typing_burst: 64,
                selection_scrub_points: 160,
                awareness_peer_count: 32,
                opaque_payload_bytes: 256 * 1024,
            },
        }
    }
}

#[derive(Clone, Copy)]
struct BenchProfile {
    warmup_iterations: usize,
    iterations: usize,
    article_blocks: usize,
    paragraph_chars: usize,
    mapping_points: usize,
    selection_width: u32,
    typing_burst: usize,
    selection_scrub_points: usize,
    awareness_peer_count: usize,
    opaque_payload_bytes: usize,
}

struct BenchOptions {
    mode: BenchMode,
    json_output: bool,
    filter: Option<String>,
}

#[derive(Debug)]
struct BenchResult {
    name: &'static str,
    group: &'static str,
    iterations: usize,
    ops_per_iteration: usize,
    min_ms: f64,
    p50_ms: f64,
    mean_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    mean_us_per_op: f64,
}

macro_rules! push_case {
    (
        $results:expr,
        $options:expr,
        verified_bench_case($name:expr, $group:expr, $($arguments:tt)*),
    ) => {
        push_case_lazy($results, $options, $name, $group, || {
            verified_bench_case($name, $group, $($arguments)*)
        })
    };
    (
        $results:expr,
        $options:expr,
        bench_case($name:expr, $group:expr, $($arguments:tt)*),
    ) => {
        push_case_lazy($results, $options, $name, $group, || {
            bench_case($name, $group, $($arguments)*)
        })
    };
}

fn main() {
    let options = parse_options();
    let profile = options.mode.profile();
    let article_doc_cell = OnceLock::new();
    let article_doc = || {
        article_doc_cell
            .get_or_init(|| build_article_document(profile.article_blocks, profile.paragraph_chars))
    };
    let article_doc_2x_cell = OnceLock::new();
    let article_doc_2x = || {
        article_doc_2x_cell.get_or_init(|| {
            build_article_document(profile.article_blocks * 2, profile.paragraph_chars)
        })
    };
    let article_json_cell = OnceLock::new();
    let article_json = || {
        article_json_cell.get_or_init(|| {
            serde_json::to_string(article_doc())
                .expect("article benchmark fixture should serialize")
        })
    };
    let article_json_2x_cell = OnceLock::new();
    let article_json_2x = || {
        article_json_2x_cell.get_or_init(|| {
            serde_json::to_string(article_doc_2x())
                .expect("2x article benchmark fixture should serialize")
        })
    };
    let editing_insert_expected_cell = OnceLock::new();
    let editing_insert_expected = || {
        editing_insert_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc(),
                build_pure_insert_document(article_doc(), "!", 1),
            )
        })
    };
    let editing_original_expected_cell = OnceLock::new();
    let editing_original_expected = || {
        editing_original_expected_cell
            .get_or_init(|| build_editing_case_expectation(article_doc(), article_doc().clone()))
    };
    let editing_burst_expected_cell = OnceLock::new();
    let editing_burst_expected = || {
        editing_burst_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc(),
                build_pure_insert_document(article_doc(), "x", EDITING_TYPING_BURST),
            )
        })
    };
    let editing_bold_expected_cell = OnceLock::new();
    let editing_bold_expected = || {
        editing_bold_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc(),
                build_pure_bold_document(
                    article_doc(),
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR + 8,
                ),
            )
        })
    };
    let editing_list_expected_cell = OnceLock::new();
    let editing_list_expected = || {
        editing_list_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc(),
                build_pure_list_document(article_doc(), EDITING_CURSOR_SCALAR),
            )
        })
    };
    let editing_undo_expected_cell = OnceLock::new();
    let editing_undo_expected = || {
        editing_undo_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                &build_pure_insert_document(article_doc(), "!", 1),
                article_doc().clone(),
            )
        })
    };
    let editing_insert_2x_expected_cell = OnceLock::new();
    let editing_insert_2x_expected = || {
        editing_insert_2x_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc_2x(),
                build_pure_insert_document(article_doc_2x(), "!", 1),
            )
        })
    };
    let editing_original_2x_expected_cell = OnceLock::new();
    let editing_original_2x_expected = || {
        editing_original_2x_expected_cell.get_or_init(|| {
            build_editing_case_expectation(article_doc_2x(), article_doc_2x().clone())
        })
    };
    let editing_list_2x_expected_cell = OnceLock::new();
    let editing_list_2x_expected = || {
        editing_list_2x_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc_2x(),
                build_pure_list_document(article_doc_2x(), EDITING_CURSOR_SCALAR),
            )
        })
    };
    let opaque_json_cell = OnceLock::new();
    let opaque_json = || {
        opaque_json_cell.get_or_init(|| {
            serde_json::to_string(&build_opaque_document(profile.opaque_payload_bytes))
                .expect("opaque benchmark fixture should serialize")
        })
    };
    let opaque_json_2x_cell = OnceLock::new();
    let opaque_json_2x = || {
        opaque_json_2x_cell.get_or_init(|| {
            serde_json::to_string(&build_opaque_document(profile.opaque_payload_bytes * 2))
                .expect("2x opaque benchmark fixture should serialize")
        })
    };
    let candidate_document_cell = OnceLock::new();
    let candidate_document = || {
        candidate_document_cell.get_or_init(|| {
            yrs_engine_with_document(article_json())
                .document()
                .expect("populated Yrs benchmark engine should be ready")
                .clone()
        })
    };

    let mut results = Vec::new();

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.edit.insert_char.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    yrs_editing_fixture(
                        article_json(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_insert_expected(),
                )
            },
            |(engine, _)| {
                black_box(
                    engine
                        .apply_command(2, TypedCommand::InsertText { text: "!".into() })
                        .expect("Yrs insert-character benchmark should succeed")
                        .expect("Yrs insert-character command should apply"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    2,
                    TransactionOrigin::LocalCommand,
                    true,
                    2,
                    3,
                    EDITING_CURSOR_SCALAR + 1,
                    EDITING_CURSOR_SCALAR + 1,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.edit.typing_burst.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            EDITING_TYPING_BURST,
            || {
                (
                    yrs_editing_fixture(
                        article_json(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_burst_expected(),
                )
            },
            |(engine, _)| {
                let mut final_output = None;
                for index in 0..EDITING_TYPING_BURST {
                    let output = engine
                        .apply_command(
                            index as u64 + 2,
                            TypedCommand::InsertText { text: "x".into() },
                        )
                        .expect("Yrs typing-burst benchmark should succeed")
                        .expect("Yrs typing-burst command should apply");
                    if index + 1 == EDITING_TYPING_BURST {
                        final_output = Some(output);
                    } else {
                        black_box(output);
                    }
                }
                black_box(final_output.expect("typing burst must produce a final output"))
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    EDITING_TYPING_BURST as u64 + 1,
                    TransactionOrigin::LocalCommand,
                    true,
                    1 + EDITING_TYPING_BURST as u64,
                    2 + EDITING_TYPING_BURST as u64,
                    EDITING_CURSOR_SCALAR + EDITING_TYPING_BURST as u32,
                    EDITING_CURSOR_SCALAR + EDITING_TYPING_BURST as u32,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.state.selection_light.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let (engine, position) = yrs_selection_fixture(article_json());
                (engine, position, editing_original_expected())
            },
            |(engine, position, _)| {
                black_box(
                    engine
                        .apply_typed_transaction_with_result(yrs_selection_transaction(
                            1,
                            engine.revision(),
                            *position,
                            *position,
                        ))
                        .expect("Yrs light-selection benchmark should succeed"),
                )
            },
            |(engine, position, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    1,
                    TransactionOrigin::LocalApi,
                    true,
                    1,
                    2,
                    *position,
                    *position,
                    false,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.command.toggle_mark.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    yrs_editing_fixture(
                        article_json(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR + 8,
                    ),
                    editing_bold_expected(),
                )
            },
            |(engine, _)| {
                black_box(
                    engine
                        .apply_command(
                            2,
                            TypedCommand::ToggleMark {
                                mark_type: "bold".into(),
                            },
                        )
                        .expect("Yrs toggle-mark benchmark should succeed")
                        .expect("Yrs toggle-mark command should apply"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    2,
                    TransactionOrigin::LocalCommand,
                    true,
                    2,
                    3,
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR + 8,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.command.wrap_list.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    yrs_editing_fixture(
                        article_json(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_list_expected(),
                )
            },
            |(engine, _)| {
                black_box(
                    engine
                        .apply_command(
                            2,
                            TypedCommand::WrapInList {
                                list_type: "bulletList".into(),
                                item_type: "listItem".into(),
                            },
                        )
                        .expect("Yrs wrap-list benchmark should succeed")
                        .expect("Yrs wrap-list command should apply"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    2,
                    TransactionOrigin::LocalCommand,
                    true,
                    2,
                    3,
                    EDITING_CURSOR_SCALAR + 2,
                    EDITING_CURSOR_SCALAR + 2,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.history.undo.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let mut engine = yrs_editing_fixture(
                    article_json(),
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR,
                );
                engine
                    .apply_command(2, TypedCommand::InsertText { text: "!".into() })
                    .expect("Yrs undo fixture edit should succeed")
                    .expect("Yrs undo fixture edit should apply");
                (engine, editing_undo_expected())
            },
            |(engine, _)| {
                black_box(
                    engine
                        .undo_with_result(3)
                        .expect("Yrs undo benchmark should succeed")
                        .expect("Yrs undo benchmark should have history"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    3,
                    TransactionOrigin::UndoRedo,
                    true,
                    3,
                    4,
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR,
                    false,
                    true,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.history.redo.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let mut engine = yrs_editing_fixture(
                    article_json(),
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR,
                );
                engine
                    .apply_command(2, TypedCommand::InsertText { text: "!".into() })
                    .expect("Yrs redo fixture edit should succeed")
                    .expect("Yrs redo fixture edit should apply");
                engine
                    .undo(3)
                    .expect("Yrs redo fixture undo should succeed")
                    .expect("Yrs redo fixture should create redo history");
                (engine, editing_insert_expected())
            },
            |(engine, _)| {
                black_box(
                    engine
                        .redo_with_result(4)
                        .expect("Yrs redo benchmark should succeed")
                        .expect("Yrs redo benchmark should have history"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    4,
                    TransactionOrigin::UndoRedo,
                    true,
                    4,
                    5,
                    EDITING_CURSOR_SCALAR + 1,
                    EDITING_CURSOR_SCALAR + 1,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.edit.insert_char.article.2x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    yrs_editing_fixture(
                        article_json_2x(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_insert_2x_expected(),
                )
            },
            |(engine, _)| {
                black_box(
                    engine
                        .apply_command(2, TypedCommand::InsertText { text: "!".into() })
                        .expect("2x Yrs insert-character benchmark should succeed")
                        .expect("2x Yrs insert-character command should apply"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    2,
                    TransactionOrigin::LocalCommand,
                    true,
                    2,
                    3,
                    EDITING_CURSOR_SCALAR + 1,
                    EDITING_CURSOR_SCALAR + 1,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.state.selection_light.article.2x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let (engine, position) = yrs_selection_fixture(article_json_2x());
                (engine, position, editing_original_2x_expected())
            },
            |(engine, position, _)| {
                black_box(
                    engine
                        .apply_typed_transaction_with_result(yrs_selection_transaction(
                            1,
                            engine.revision(),
                            *position,
                            *position,
                        ))
                        .expect("2x Yrs light-selection benchmark should succeed"),
                )
            },
            |(engine, position, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    1,
                    TransactionOrigin::LocalApi,
                    true,
                    1,
                    2,
                    *position,
                    *position,
                    false,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.command.wrap_list.article.2x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    yrs_editing_fixture(
                        article_json_2x(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_list_2x_expected(),
                )
            },
            |(engine, _)| {
                black_box(
                    engine
                        .apply_command(
                            2,
                            TypedCommand::WrapInList {
                                list_type: "bulletList".into(),
                                item_type: "listItem".into(),
                            },
                        )
                        .expect("2x Yrs wrap-list benchmark should succeed")
                        .expect("2x Yrs wrap-list command should apply"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    2,
                    TransactionOrigin::LocalCommand,
                    true,
                    2,
                    3,
                    EDITING_CURSOR_SCALAR + 2,
                    EDITING_CURSOR_SCALAR + 2,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_import.article.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || (empty_yrs_engine(), article_json().clone()),
            |(engine, document)| {
                black_box(
                    engine
                        .import_json(document, TransactionOrigin::DocumentImport)
                        .expect("Yrs JSON import benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_export.article.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || yrs_engine_with_document(article_json()),
            |engine| black_box(engine.document_json_string()),
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_import.article.2x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || (empty_yrs_engine(), article_json_2x().clone()),
            |(engine, document)| {
                black_box(
                    engine
                        .import_json(document, TransactionOrigin::DocumentImport)
                        .expect("2x Yrs JSON import benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_export.article.2x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || yrs_engine_with_document(article_json_2x()),
            |engine| black_box(engine.document_json_string()),
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.candidate_validation.article.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    candidate_document().clone(),
                    tiptap_schema(),
                    ResourceLimits::default(),
                )
            },
            |(document, schema, limits)| {
                black_box(
                    DocumentValidator::validate(document, schema, limits)
                        .expect("Yrs candidate validation benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.encoded_state.article.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || yrs_engine_with_document(article_json()),
            |engine| {
                black_box(
                    engine
                        .encoded_state()
                        .expect("Yrs encoded-state benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_import.opaque_large.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || (empty_yrs_engine(), opaque_json().clone()),
            |(engine, document)| {
                black_box(
                    engine
                        .import_json(document, TransactionOrigin::DocumentImport)
                        .expect("large opaque Yrs import benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_import.opaque_large.2x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || (empty_yrs_engine(), opaque_json_2x().clone()),
            |(engine, document)| {
                black_box(
                    engine
                        .import_json(document, TransactionOrigin::DocumentImport)
                        .expect("2x large opaque Yrs import benchmark should succeed"),
                );
            },
        ),
    );

    if results.is_empty() {
        eprintln!("no benchmarks matched the provided filter");
        std::process::exit(1);
    }

    if options.json_output {
        print_json_summary(options.mode, profile, &results);
    } else {
        print_table(options.mode, profile, &results);
    }
}

fn parse_options() -> BenchOptions {
    let mut mode = BenchMode::Standard;
    let mut json_output = false;
    let mut filter = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--quick" => mode = BenchMode::Quick,
            "--json" => json_output = true,
            "--bench" => {}
            "--filter" => {
                filter = args.next();
            }
            _ if arg.starts_with("--filter=") => {
                filter = Some(arg["--filter=".len()..].to_string());
            }
            _ => {}
        }
    }

    BenchOptions {
        mode,
        json_output,
        filter,
    }
}

fn push_case_lazy(
    results: &mut Vec<BenchResult>,
    options: &BenchOptions,
    name: &'static str,
    group: &'static str,
    run: impl FnOnce() -> BenchResult,
) {
    if let Some(result) =
        benchmark_filter::run_if_selected(options.filter.as_deref(), name, group, run)
    {
        results.push(result);
    }
}

fn bench_case<S, Setup, Run, Output>(
    name: &'static str,
    group: &'static str,
    iterations: usize,
    warmup_iterations: usize,
    ops_per_iteration: usize,
    mut setup: Setup,
    mut run: Run,
) -> BenchResult
where
    Setup: FnMut() -> S,
    Run: FnMut(&mut S) -> Output,
{
    for _ in 0..warmup_iterations {
        let mut state = setup();
        black_box(run(&mut state));
    }

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut state = setup();
        let started_at = Instant::now();
        black_box(run(&mut state));
        samples.push(started_at.elapsed());
    }

    build_result(name, group, iterations, ops_per_iteration, samples)
}

#[allow(clippy::too_many_arguments)]
fn verified_bench_case<S, Setup, Run, Output, Verify>(
    name: &'static str,
    group: &'static str,
    iterations: usize,
    warmup_iterations: usize,
    ops_per_iteration: usize,
    mut setup: Setup,
    mut run: Run,
    mut verify: Verify,
) -> BenchResult
where
    Setup: FnMut() -> S,
    Run: FnMut(&mut S) -> Output,
    Verify: FnMut(&S, &Output),
{
    for _ in 0..warmup_iterations {
        let mut state = setup();
        let output = black_box(run(&mut state));
        verify(&state, &output);
    }

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut state = setup();
        let started_at = Instant::now();
        let output = black_box(run(&mut state));
        let elapsed = started_at.elapsed();
        verify(&state, &output);
        samples.push(elapsed);
    }

    build_result(name, group, iterations, ops_per_iteration, samples)
}

fn build_result(
    name: &'static str,
    group: &'static str,
    iterations: usize,
    ops_per_iteration: usize,
    mut samples: Vec<Duration>,
) -> BenchResult {
    let total_ms = samples
        .iter()
        .map(|duration| duration_to_ms(*duration))
        .sum::<f64>();
    let mean_ms = total_ms / iterations.max(1) as f64;
    samples.sort_unstable();
    let min_ms = duration_to_ms(*samples.first().unwrap_or(&Duration::ZERO));
    let max_ms = duration_to_ms(*samples.last().unwrap_or(&Duration::ZERO));
    let p50_ms = percentile_ms(&samples, 0.50);
    let p95_ms = percentile_ms(&samples, 0.95);
    let mean_us_per_op = (mean_ms * 1_000.0) / ops_per_iteration.max(1) as f64;

    BenchResult {
        name,
        group,
        iterations,
        ops_per_iteration,
        min_ms,
        p50_ms,
        mean_ms,
        p95_ms,
        max_ms,
        mean_us_per_op,
    }
}

fn percentile_ms(samples: &[Duration], percentile: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let clamped = percentile.clamp(0.0, 1.0);
    let index = ((samples.len() - 1) as f64 * clamped).round() as usize;
    duration_to_ms(samples[index])
}

fn duration_to_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn print_table(mode: BenchMode, profile: BenchProfile, results: &[BenchResult]) {
    println!(
        "editor-core performance suite (mode: {}, iterations: {}, warmup: {})",
        mode.as_str(),
        profile.iterations,
        profile.warmup_iterations
    );
    println!(
        "{:<48} {:>5} {:>8} {:>10} {:>10} {:>10} {:>10} {:>11}",
        "benchmark", "iters", "ops", "mean ms", "p50 ms", "p95 ms", "max ms", "mean us/op"
    );
    println!("{}", "-".repeat(118));

    for result in results {
        println!(
            "{:<48} {:>5} {:>8} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>11.2}",
            result.name,
            result.iterations,
            result.ops_per_iteration,
            result.mean_ms,
            result.p50_ms,
            result.p95_ms,
            result.max_ms,
            result.mean_us_per_op
        );
    }
}

fn print_json_summary(mode: BenchMode, profile: BenchProfile, results: &[BenchResult]) {
    let payload = json!({
        "mode": mode.as_str(),
        "iterations": profile.iterations,
        "warmupIterations": profile.warmup_iterations,
        "documentProfile": {
            "articleBlocks": profile.article_blocks,
            "paragraphChars": profile.paragraph_chars,
            "mappingPoints": profile.mapping_points,
            "selectionWidth": profile.selection_width,
            "typingBurst": profile.typing_burst,
            "editingTypingBurst": EDITING_TYPING_BURST,
            "selectionScrubPoints": profile.selection_scrub_points,
            "awarenessPeerCount": profile.awareness_peer_count,
            "opaquePayloadBytes": profile.opaque_payload_bytes,
        },
        "results": results.iter().map(|result| {
            json!({
                "name": result.name,
                "group": result.group,
                "iterations": result.iterations,
                "opsPerIteration": result.ops_per_iteration,
                "minMs": result.min_ms,
                "p50Ms": result.p50_ms,
                "meanMs": result.mean_ms,
                "p95Ms": result.p95_ms,
                "maxMs": result.max_ms,
                "meanUsPerOp": result.mean_us_per_op,
            })
        }).collect::<Vec<_>>(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).expect("benchmark JSON payload should serialize")
    );
}

fn empty_yrs_engine() -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".to_string(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .expect("Yrs benchmark engine should initialize")
}

fn yrs_engine_with_document(document: &str) -> YrsDocumentEngine {
    let mut engine = empty_yrs_engine();
    engine
        .import_json(document, TransactionOrigin::DocumentImport)
        .expect("Yrs benchmark fixture document should import");
    engine
}

fn yrs_editing_fixture(document: &str, anchor: u32, head: u32) -> YrsDocumentEngine {
    let mut engine = yrs_engine_with_document(document);
    engine
        .apply_typed_transaction(yrs_selection_transaction(
            1,
            engine.revision(),
            anchor,
            head,
        ))
        .expect("Yrs benchmark selection should apply");
    engine
}

fn yrs_selection_fixture(document: &str) -> (YrsDocumentEngine, u32) {
    let engine = yrs_engine_with_document(document);
    let position = engine
        .position_map()
        .expect("populated Yrs benchmark engine should have a position map")
        .total_scalars()
        / 2;
    (engine, position)
}

fn yrs_selection_transaction(
    request_id: u64,
    revision: u64,
    anchor: u32,
    head: u32,
) -> TypedTransaction {
    let point = |offset| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    TypedTransaction {
        request_id,
        base_document_revision: revision,
        origin: TransactionOrigin::LocalApi,
        operations: vec![],
        selection_intent: SelectionIntent::Set(SelectionInput::Text {
            anchor: point(anchor),
            head: point(head),
        }),
        history_policy: HistoryPolicy::Skip,
    }
}

fn build_pure_insert_document(doc: &Value, text: &str, count: usize) -> Value {
    let mut expected = doc.clone();
    let offset = first_article_paragraph_scalar_offset(&expected, EDITING_CURSOR_SCALAR);
    let target = first_article_paragraph_text_mut(&mut expected);
    let byte_offset = target
        .char_indices()
        .nth(offset)
        .map(|(index, _)| index)
        .unwrap_or(target.len());
    target.insert_str(byte_offset, &text.repeat(count));
    expected
}

fn build_pure_bold_document(doc: &Value, anchor: u32, head: u32) -> Value {
    let mut expected = doc.clone();
    let from = first_article_paragraph_scalar_offset(&expected, anchor);
    let to = first_article_paragraph_scalar_offset(&expected, head);
    assert!(from < to, "expected bold range must be non-empty");

    let inline = first_article_paragraph_inline_mut(&mut expected);
    let original = inline
        .first()
        .and_then(|node| node.get("text"))
        .and_then(Value::as_str)
        .expect("article benchmark first inline node must be text")
        .chars()
        .collect::<Vec<_>>();
    assert!(
        to <= original.len(),
        "expected bold range must stay in the first text run"
    );

    let mut replacement = Vec::with_capacity(3);
    let prefix = char_slice(&original, 0, from);
    if !prefix.is_empty() {
        replacement.push(text_node(prefix));
    }
    replacement.push(marked_text_node(
        char_slice(&original, from, to),
        json!({ "type": "bold" }),
    ));
    let suffix = char_slice(&original, to, original.len());
    if !suffix.is_empty() {
        replacement.push(text_node(suffix));
    }
    inline.splice(0..1, replacement);
    expected
}

fn build_pure_list_document(doc: &Value, position: u32) -> Value {
    let mut expected = doc.clone();
    let offset = first_article_paragraph_scalar_offset(&expected, position);
    assert!(
        offset <= first_article_paragraph_text(&expected).chars().count(),
        "expected list cursor must target the first article paragraph",
    );
    let content = expected
        .get_mut("content")
        .and_then(Value::as_array_mut)
        .expect("article benchmark document must have content");
    let paragraph = content.remove(1);
    content.insert(
        1,
        json!({
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [paragraph]
            }]
        }),
    );
    expected
}

fn first_article_paragraph_scalar_offset(doc: &Value, scalar: u32) -> usize {
    let heading_text = doc
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|heading| heading.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|text| text.get("text"))
        .and_then(Value::as_str)
        .expect("article benchmark heading must start with text");
    let paragraph_start = heading_text.chars().count().saturating_add(1);
    usize::try_from(scalar)
        .expect("editing scalar must fit usize")
        .checked_sub(paragraph_start)
        .expect("editing scalar must target the first article paragraph")
}

fn first_article_paragraph_text(doc: &Value) -> &str {
    doc.get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.get(1))
        .and_then(|paragraph| paragraph.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|text| text.get("text"))
        .and_then(Value::as_str)
        .expect("article benchmark first paragraph must start with text")
}

fn first_article_paragraph_inline_mut(doc: &mut Value) -> &mut Vec<Value> {
    doc.get_mut("content")
        .and_then(Value::as_array_mut)
        .and_then(|content| content.get_mut(1))
        .and_then(|paragraph| paragraph.get_mut("content"))
        .and_then(Value::as_array_mut)
        .expect("article benchmark first paragraph must have inline content")
}

fn first_article_paragraph_text_mut(doc: &mut Value) -> &mut String {
    first_article_paragraph_inline_mut(doc)
        .first_mut()
        .and_then(|text| text.get_mut("text"))
        .and_then(|value| match value {
            Value::String(text) => Some(text),
            _ => None,
        })
        .expect("article benchmark first paragraph must start with text")
}

fn build_yrs_expected_document(doc: &Value) -> Value {
    let encoded = serde_json::to_string(doc).expect("expected editing document should serialize");
    yrs_engine_with_document(&encoded)
        .document_json()
        .expect("expected Yrs editing document should import")
}

fn build_editing_case_expectation(before: &Value, after: Value) -> EditingCaseExpectation {
    EditingCaseExpectation {
        before: build_yrs_expected_document(before),
        after: build_yrs_expected_document(&after),
    }
}

/// v2-native expected fixture: the pre-cutover harness derived expected
/// selections, active state, and render blocks from a throwaway legacy
/// `Editor`. Those derivations are the same retained code paths the v2
/// render accessor uses today (serializer -> `PositionMap` ->
/// `editor_state` -> `render::incremental`), so the expectations are
/// computed from the expected document directly.
struct ExpectedEditingFixture {
    document: crate::model::Document,
    schema: crate::schema::Schema,
    position_map: crate::position::PositionMap,
    selection: crate::selection::Selection,
}

fn expected_editing_fixture(doc_json: &Value, anchor: u32, head: u32) -> ExpectedEditingFixture {
    let schema = tiptap_schema();
    let document = crate::serialize::from_prosemirror_json(
        doc_json,
        &schema,
        crate::serialize::UnknownTypeMode::Preserve,
    )
    .expect("expected editing document should ingest");
    let position_map = crate::position::PositionMap::build(&document, &schema);
    // The legacy harness's `set_selection_scalar`: lenient scalar->doc,
    // collapsed selections become cursors, then cursor normalization.
    let doc_anchor = position_map.scalar_to_doc(anchor, &document);
    let doc_head = position_map.scalar_to_doc(head, &document);
    let selection = if doc_anchor == doc_head {
        crate::selection::Selection::cursor(doc_anchor)
    } else {
        crate::selection::Selection::text(doc_anchor, doc_head)
    }
    .normalized(&document, &position_map);
    ExpectedEditingFixture {
        document,
        schema,
        position_map,
        selection,
    }
}

impl ExpectedEditingFixture {
    fn active_state(&self) -> crate::editor_state::ActiveState {
        let limits = ResourceLimits::default();
        let commands = crate::editor_state::command_applicability(
            &self.document,
            &self.schema,
            &self.selection,
            &limits,
        );
        crate::editor_state::active_state(
            &self.document,
            &self.schema,
            &self.selection,
            None,
            commands,
            &limits,
        )
    }

    fn render_blocks(&self) -> Vec<Vec<RenderElement>> {
        crate::render::incremental::render_blocks(&self.document, &self.schema)
    }
}

fn render_blocks_for(doc_json: &Value) -> Vec<Vec<RenderElement>> {
    let schema = tiptap_schema();
    let document = crate::serialize::from_prosemirror_json(
        doc_json,
        &schema,
        crate::serialize::UnknownTypeMode::Preserve,
    )
    .expect("expected render document should ingest");
    crate::render::incremental::render_blocks(&document, &schema)
}

#[allow(clippy::too_many_arguments)]
fn assert_yrs_editing_output(
    engine: &YrsDocumentEngine,
    expectation: &EditingCaseExpectation,
    output: &TypedTransactionResult,
    request_id: u64,
    origin: TransactionOrigin,
    changed: bool,
    document_revision: u64,
    state_revision: u64,
    anchor: u32,
    head: u32,
    can_undo: bool,
    can_redo: bool,
) {
    assert_eq!(engine.revision(), document_revision);
    assert_eq!(engine.state_revision(), state_revision);
    let selection = engine
        .resolved_selection()
        .expect("verified Yrs engine should have a resolved selection");
    let expected_fixture = expected_editing_fixture(&expectation.after, anchor, head);
    assert!(
        expectation.after.to_string().is_ascii(),
        "editing benchmark profile must remain ASCII for scalar/UTF-16 equality",
    );
    let expected_selection = ResolvedSelection::Text {
        anchor: crate::yrs_engine::ResolvedPoint {
            document: expected_fixture
                .position_map
                .scalar_to_doc(anchor, &expected_fixture.document),
            scalar: anchor,
            utf16: anchor,
        },
        head: crate::yrs_engine::ResolvedPoint {
            document: expected_fixture
                .position_map
                .scalar_to_doc(head, &expected_fixture.document),
            scalar: head,
            utf16: head,
        },
    };
    assert_eq!(selection, &expected_selection);
    match selection {
        ResolvedSelection::Text {
            anchor: resolved_anchor,
            head: resolved_head,
        } => {
            assert_eq!(
                resolved_anchor.document,
                expected_fixture
                    .position_map
                    .scalar_to_doc(anchor, &expected_fixture.document)
            );
            assert_eq!(resolved_anchor.scalar, anchor);
            assert_eq!(resolved_anchor.utf16, anchor);
            assert_eq!(
                resolved_head.document,
                expected_fixture
                    .position_map
                    .scalar_to_doc(head, &expected_fixture.document)
            );
            assert_eq!(resolved_head.scalar, head);
            assert_eq!(resolved_head.utf16, head);
        }
        other => panic!("expected verified text selection, got {other:?}"),
    }
    assert_eq!(engine.can_undo(), can_undo);
    assert_eq!(engine.can_redo(), can_redo);
    let actual_document = engine
        .document_json()
        .expect("verified Yrs engine should have canonical JSON");
    let expected_document = &expectation.after;
    assert_eq!(&actual_document, expected_document);

    assert_eq!(output.request_id, request_id);
    assert_eq!(output.origin, origin);
    assert_eq!(output.changed, changed);
    assert_eq!(output.document_revision, document_revision);
    assert_eq!(output.state_revision, state_revision);
    assert_eq!(output.selection, expected_selection);
    assert_eq!(output.active_state, expected_fixture.active_state());
    assert_eq!(output.history_state.can_undo, can_undo);
    assert_eq!(output.history_state.can_redo, can_redo);
    if expectation.before != expectation.after {
        assert_eq!(
            apply_render_update(&expectation.before, &output.render_update),
            expected_fixture.render_blocks(),
        );
    } else {
        assert_eq!(output.render_update, RenderUpdate::None);
    }
}

fn apply_render_patch(
    before_document: &Value,
    patch: &crate::render::incremental::RenderBlocksPatch,
) -> Vec<Vec<RenderElement>> {
    let mut blocks = render_blocks_for(before_document);
    blocks.splice(
        patch.start_index..patch.start_index + patch.delete_count,
        patch.blocks.clone(),
    );
    blocks
}

fn apply_render_update(before_document: &Value, update: &RenderUpdate) -> Vec<Vec<RenderElement>> {
    match update {
        RenderUpdate::None => render_blocks_for(before_document),
        RenderUpdate::Patch(patch) => apply_render_patch(before_document, patch),
        RenderUpdate::Full(blocks) => blocks.clone(),
    }
}

fn evenly_spaced_positions(max_value: u32, points: usize) -> Vec<u32> {
    if points <= 1 || max_value == 0 {
        return vec![0];
    }

    (0..points)
        .map(|index| ((max_value as u64 * index as u64) / (points - 1) as u64) as u32)
        .collect()
}

fn selection_scrub_positions(total_scalar: u32, points: usize) -> Vec<u32> {
    let upper_bound = total_scalar.saturating_sub(1).max(1);
    evenly_spaced_positions(upper_bound, points)
        .into_iter()
        .map(|position| position.max(1))
        .collect()
}

fn build_article_document(block_count: usize, paragraph_chars: usize) -> Value {
    let mut content = Vec::with_capacity(block_count + (block_count / 12) + 2);
    content.push(json!({
        "type": "h1",
        "content": [text_node(text_fragment(10_000, 42))]
    }));

    for index in 0..block_count {
        if index > 0 && index % 18 == 0 {
            content.push(json!({ "type": "horizontalRule" }));
        }

        if index % 12 == 5 {
            content.push(json!({
                "type": "blockquote",
                "content": [{
                    "type": "paragraph",
                    "content": rich_inline_content(index, paragraph_chars)
                }]
            }));
            continue;
        }

        if index % 9 == 3 {
            content.push(json!({
                "type": "h2",
                "content": [text_node(text_fragment(index + 2_000, paragraph_chars / 3 + 24))]
            }));
            continue;
        }

        content.push(json!({
            "type": "paragraph",
            "content": rich_inline_content(index, paragraph_chars)
        }));
    }

    json!({
        "type": "doc",
        "content": content
    })
}

fn build_opaque_document(payload_bytes: usize) -> Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "benchmarkOpaqueBlock",
            "attrs": {
                "payload": "x".repeat(payload_bytes),
            },
        }],
    })
}

fn build_edited_article_document(doc: &Value) -> Value {
    let mut next = doc.clone();
    let appended = append_to_last_text_node(&mut next, " sync-update");
    assert!(
        appended,
        "edited benchmark document should contain text nodes"
    );
    next
}

fn append_to_last_text_node(node: &mut Value, suffix: &str) -> bool {
    match node {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(Value::String(text)) = object.get_mut("text") {
                    text.push_str(suffix);
                    return true;
                }
            }

            if let Some(children) = object.get_mut("content").and_then(Value::as_array_mut) {
                for child in children.iter_mut().rev() {
                    if append_to_last_text_node(child, suffix) {
                        return true;
                    }
                }
            }
            false
        }
        Value::Array(array) => {
            for child in array.iter_mut().rev() {
                if append_to_last_text_node(child, suffix) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn rich_inline_content(seed: usize, total_chars: usize) -> Vec<Value> {
    let full_text = text_fragment(seed, total_chars.max(32));
    let chars: Vec<char> = full_text.chars().collect();
    let len = chars.len();
    let cut_a = len / 4;
    let cut_b = len / 2;
    let cut_c = (len * 3) / 4;

    let plain_lead = char_slice(&chars, 0, cut_a);
    let bold_text = char_slice(&chars, cut_a, cut_b);
    let italic_text = char_slice(&chars, cut_b, cut_c);
    let tail_text = char_slice(&chars, cut_c, len);

    let mut content = Vec::new();
    if !plain_lead.is_empty() {
        content.push(text_node(plain_lead));
    }
    if !bold_text.is_empty() {
        content.push(marked_text_node(bold_text, json!({ "type": "bold" })));
    }
    if !italic_text.is_empty() {
        content.push(marked_text_node(italic_text, json!({ "type": "italic" })));
    }
    if !tail_text.is_empty() {
        content.push(marked_text_node(
            tail_text,
            json!({
                "type": "link",
                "attrs": {
                    "href": format!("https://example.com/item/{seed}"),
                    "target": "_blank",
                    "rel": "noopener noreferrer nofollow",
                    "class": Value::Null,
                    "title": Value::Null
                }
            }),
        ));
    }

    content
}

fn text_node(text: String) -> Value {
    json!({
        "type": "text",
        "text": text
    })
}

fn marked_text_node(text: String, mark: Value) -> Value {
    json!({
        "type": "text",
        "text": text,
        "marks": [mark]
    })
}

fn text_fragment(seed: usize, min_chars: usize) -> String {
    const WORDS: &[&str] = &[
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
        "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra",
        "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu",
    ];

    let mut text = String::new();
    let mut cursor = 0usize;
    while text.chars().count() < min_chars {
        if !text.is_empty() {
            text.push(' ');
        }
        let word = WORDS[(seed + cursor) % WORDS.len()];
        text.push_str(word);
        cursor += 1;
    }
    text.chars().take(min_chars).collect()
}

fn char_slice(chars: &[char], start: usize, end: usize) -> String {
    let bounded_start = start.min(chars.len());
    let bounded_end = end.min(chars.len()).max(bounded_start);
    chars[bounded_start..bounded_end].iter().collect()
}
