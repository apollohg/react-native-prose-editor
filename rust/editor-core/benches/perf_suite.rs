use std::env;
use std::hint::black_box;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use editor_core::boundary::ResourceLimits;
use editor_core::collaboration::CollaborationSession;
use editor_core::editor::{Editor, EditorSelectionState, EditorUpdate};
use editor_core::intercept::InterceptorPipeline;
use editor_core::render::RenderElement;
use editor_core::schema::presets::tiptap_schema;
use editor_core::transform::DocumentValidator;
use editor_core::yrs_engine::{
    Affinity, EditorOffsetKind, HistoryPolicy, InitializationMode, RenderUpdate, ResolvedSelection,
    RevisionedPosition, SelectionInput, SelectionIntent, TransactionOrigin, TypedCommand,
    TypedTransaction, TypedTransactionResult, YrsDocumentEngine, YrsEngineConfig,
};
use serde_json::{json, Value};

#[path = "support/benchmark_filter.rs"]
mod benchmark_filter;

const EDITING_TYPING_BURST: usize = 20;
const EDITING_CURSOR_SCALAR: u32 = 44;

struct BackendExpectedDocuments {
    legacy: Value,
    yrs: Value,
}

struct EditingCaseExpectation {
    before: BackendExpectedDocuments,
    after: BackendExpectedDocuments,
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
    let edited_article_doc_cell = OnceLock::new();
    let edited_article_doc =
        || edited_article_doc_cell.get_or_init(|| build_edited_article_document(article_doc()));

    let mut results = Vec::new();

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "legacy.edit.insert_char.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    legacy_editing_fixture(
                        article_doc(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_insert_expected(),
                )
            },
            |(editor, _)| {
                black_box(
                    editor
                        .insert_text_scalar(EDITING_CURSOR_SCALAR, "!")
                        .expect("legacy insert-character benchmark should succeed"),
                )
            },
            |(editor, expectation), output| {
                assert_legacy_editing_output(
                    editor,
                    expectation,
                    output,
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
            "legacy.edit.typing_burst.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            EDITING_TYPING_BURST,
            || {
                (
                    legacy_editing_fixture(
                        article_doc(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_burst_expected(),
                )
            },
            |(editor, _)| {
                let mut final_output = None;
                for offset in 0..EDITING_TYPING_BURST as u32 {
                    let output = editor
                        .insert_text_scalar(EDITING_CURSOR_SCALAR + offset, "x")
                        .expect("legacy typing-burst benchmark should succeed");
                    if offset + 1 == EDITING_TYPING_BURST as u32 {
                        final_output = Some(output);
                    } else {
                        black_box(output);
                    }
                }
                black_box(final_output.expect("typing burst must produce a final output"))
            },
            |(editor, expectation), output| {
                assert_legacy_editing_output(
                    editor,
                    expectation,
                    output,
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
            "legacy.state.selection_light.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let editor = editor_with_document(article_doc());
                let position = editor.doc_to_scalar(editor.document().content_size()) / 2;
                (editor, position, editing_original_expected())
            },
            |(editor, position, _)| {
                editor.set_selection_scalar(*position, *position);
                black_box(editor.get_selection_state())
            },
            |(editor, position, expectation), output| {
                assert_legacy_selection_output(
                    editor,
                    expectation,
                    output,
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
            "legacy.command.toggle_mark.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    legacy_editing_fixture(
                        article_doc(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR + 8,
                    ),
                    editing_bold_expected(),
                )
            },
            |(editor, _)| {
                black_box(
                    editor
                        .toggle_mark("bold")
                        .expect("legacy toggle-mark benchmark should succeed"),
                )
            },
            |(editor, expectation), output| {
                assert_legacy_editing_output(
                    editor,
                    expectation,
                    output,
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
            "legacy.command.wrap_list.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    legacy_editing_fixture(
                        article_doc(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_list_expected(),
                )
            },
            |(editor, _)| {
                let from = editor.selection().from(editor.document());
                let to = editor.selection().to(editor.document());
                black_box(
                    editor
                        .wrap_in_list(from, to, "bulletList")
                        .expect("legacy wrap-list benchmark should succeed"),
                )
            },
            |(editor, expectation), output| {
                assert_legacy_editing_output(
                    editor,
                    expectation,
                    output,
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
            "legacy.history.undo.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let mut editor = legacy_editing_fixture(
                    article_doc(),
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR,
                );
                editor
                    .insert_text_scalar(EDITING_CURSOR_SCALAR, "!")
                    .expect("legacy undo fixture edit should succeed");
                (editor, editing_undo_expected())
            },
            |(editor, _)| {
                black_box(
                    editor
                        .undo()
                        .expect("legacy undo benchmark should have history"),
                )
            },
            |(editor, expectation), output| {
                assert_legacy_editing_output(
                    editor,
                    expectation,
                    output,
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
            "legacy.history.redo.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let mut editor = legacy_editing_fixture(
                    article_doc(),
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR,
                );
                editor
                    .insert_text_scalar(EDITING_CURSOR_SCALAR, "!")
                    .expect("legacy redo fixture edit should succeed");
                editor
                    .undo()
                    .expect("legacy redo fixture should create redo history");
                (editor, editing_insert_expected())
            },
            |(editor, _)| {
                black_box(
                    editor
                        .redo()
                        .expect("legacy redo benchmark should have history"),
                )
            },
            |(editor, expectation), output| {
                assert_legacy_editing_output(
                    editor,
                    expectation,
                    output,
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
            "legacy.json_import.article.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || (empty_editor(), article_doc().clone()),
            |(editor, document)| {
                black_box(
                    editor
                        .set_json(document)
                        .expect("legacy JSON import benchmark should succeed"),
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
            "legacy.json_export.article.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || editor_with_document(article_doc()),
            |editor| black_box(editor.get_json()),
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
            |engine| black_box(engine.document_json()),
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
            |engine| black_box(engine.document_json()),
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

    push_case!(
        &mut results,
        &options,
        bench_case(
            "editor.set_json.article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || build_article_document(profile.article_blocks, profile.paragraph_chars),
            |doc| {
                let mut editor = empty_editor();
                black_box(
                    editor
                        .set_json(doc)
                        .expect("set_json benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "editor.get_current_state.article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || editor_with_document(article_doc()),
            |editor| {
                black_box(editor.get_current_state());
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "editor.get_selection_state.article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || editor_with_document(article_doc()),
            |editor| {
                black_box(editor.get_selection_state());
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "editor.get_html.article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || editor_with_document(article_doc()),
            |editor| {
                black_box(editor.get_html());
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "editor.get_json.article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || editor_with_document(article_doc()),
            |editor| {
                black_box(editor.get_json());
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "editor.insert_text_scalar.middle_article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let editor = editor_with_document(article_doc());
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                (editor, total_scalar / 2)
            },
            |(editor, cursor_scalar)| {
                black_box(
                    editor
                        .insert_text_scalar(*cursor_scalar, "!")
                        .expect("insert_text_scalar benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "editor.insert_text_scalar.typing_burst_article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            profile.typing_burst,
            || {
                let editor = editor_with_document(article_doc());
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                (editor, total_scalar / 2)
            },
            |(editor, cursor_scalar)| {
                let mut next_cursor = *cursor_scalar;
                for _ in 0..profile.typing_burst {
                    black_box(
                        editor
                            .insert_text_scalar(next_cursor, "!")
                            .expect("typing burst benchmark should succeed"),
                    );
                    next_cursor = next_cursor.saturating_add(1);
                }
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "editor.toggle_mark_scalar.selection_article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let editor = editor_with_document(article_doc());
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                let anchor = total_scalar / 3;
                let head = (anchor + profile.selection_width).min(total_scalar.max(anchor));
                (editor, anchor, head.max(anchor + 1))
            },
            |(editor, anchor, head)| {
                black_box(
                    editor
                        .toggle_mark_at_selection_scalar(*anchor, *head, "bold")
                        .expect("toggle_mark_at_selection_scalar benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "editor.replace_json.article_small_edit",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    editor_with_document(article_doc()),
                    edited_article_doc().clone(),
                )
            },
            |(editor, next_doc)| {
                black_box(
                    editor
                        .replace_json(next_doc)
                        .expect("replace_json benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "position.doc_to_scalar.article_sweep",
            "position",
            profile.iterations,
            profile.warmup_iterations,
            profile.mapping_points,
            || {
                let editor = editor_with_document(article_doc());
                let positions = evenly_spaced_positions(
                    editor.document().content_size(),
                    profile.mapping_points,
                );
                (editor, positions)
            },
            |(editor, positions)| {
                for position in positions {
                    black_box(editor.doc_to_scalar(*position));
                }
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "position.scalar_to_doc.article_sweep",
            "position",
            profile.iterations,
            profile.warmup_iterations,
            profile.mapping_points,
            || {
                let editor = editor_with_document(article_doc());
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                let positions = evenly_spaced_positions(total_scalar, profile.mapping_points);
                (editor, positions)
            },
            |(editor, positions)| {
                for position in positions {
                    black_box(editor.scalar_to_doc(*position));
                }
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "editor.set_selection_scalar.article_scrub",
            "selection",
            profile.iterations,
            profile.warmup_iterations,
            profile.selection_scrub_points,
            || {
                let editor = editor_with_document(article_doc());
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                let positions =
                    selection_scrub_positions(total_scalar, profile.selection_scrub_points);
                (editor, positions)
            },
            |(editor, positions)| {
                for position in positions {
                    editor.set_selection_scalar(*position, *position);
                    black_box(editor.selection());
                }
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "selection.refresh_toolbar_state_full.article_scrub",
            "selection",
            profile.iterations,
            profile.warmup_iterations,
            profile.selection_scrub_points,
            || {
                let editor = editor_with_document(article_doc());
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                let positions =
                    selection_scrub_positions(total_scalar, profile.selection_scrub_points);
                (editor, positions)
            },
            |(editor, positions)| {
                for position in positions {
                    editor.set_selection_scalar(*position, *position);
                    black_box(editor.get_current_state());
                }
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "selection.refresh_toolbar_state_light.article_scrub",
            "selection",
            profile.iterations,
            profile.warmup_iterations,
            profile.selection_scrub_points,
            || {
                let editor = editor_with_document(article_doc());
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                let positions =
                    selection_scrub_positions(total_scalar, profile.selection_scrub_points);
                (editor, positions)
            },
            |(editor, positions)| {
                for position in positions {
                    editor.set_selection_scalar(*position, *position);
                    black_box(editor.get_selection_state());
                }
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "collaboration.apply_local_document.article_small_edit",
            "collaboration",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    collaboration_session_with_document(article_doc()),
                    edited_article_doc().clone(),
                )
            },
            |(session, next_doc)| {
                black_box(
                    session
                        .apply_local_document(next_doc.clone())
                        .expect("benchmark collaboration document should be valid"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "collaboration.handle_message.document_update",
            "collaboration",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let mut sender = collaboration_session_with_document(article_doc());
                let receiver = collaboration_session_with_document(article_doc());
                let message = sender
                    .apply_local_document(edited_article_doc().clone())
                    .expect("benchmark collaboration document should be valid")
                    .messages
                    .into_iter()
                    .next()
                    .expect("document update benchmark should emit a message");
                (receiver, message)
            },
            |(session, message)| {
                black_box(
                    session
                        .handle_message(message.clone())
                        .expect("document update message benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "collaboration.handle_message.awareness_multi_peer_burst",
            "collaboration",
            profile.iterations,
            profile.warmup_iterations,
            profile.awareness_peer_count,
            || {
                (
                    collaboration_session_with_document(article_doc()),
                    awareness_messages_for_document(
                        article_doc(),
                        profile.awareness_peer_count,
                        profile.selection_width,
                    ),
                )
            },
            |(session, messages)| {
                for message in messages {
                    black_box(
                        session
                            .handle_message(message.clone())
                            .expect("multi-peer awareness benchmark should succeed"),
                    );
                }
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "collaboration.handle_message.awareness",
            "collaboration",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let article = build_article_document(12, 96);
                let mut sender = collaboration_session_with_document(&article);
                let receiver = collaboration_session_with_document(&article);
                let message = sender
                    .set_local_awareness(json!({
                        "user": {
                            "name": "Perf Bench",
                            "color": "#007AFF",
                            "userId": "bench-user"
                        },
                        "selection": {
                            "anchor": 1,
                            "head": 1
                        },
                        "focused": true
                    }))
                    .expect("benchmark awareness should be valid")
                    .messages
                    .into_iter()
                    .next()
                    .expect("awareness benchmark should emit a message");
                (receiver, message)
            },
            |(session, message)| {
                black_box(
                    session
                        .handle_message(message.clone())
                        .expect("awareness message benchmark should succeed"),
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

fn empty_editor() -> Editor {
    Editor::new(tiptap_schema(), InterceptorPipeline::new(), false)
}

fn editor_with_document(doc: &Value) -> Editor {
    let mut editor = empty_editor();
    editor
        .set_json(doc)
        .expect("benchmark fixture document should parse");
    editor
}

fn empty_yrs_engine() -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".to_string(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: editor_core::yrs_engine::EditingLimits::default(),
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

fn legacy_editing_fixture(doc: &Value, anchor: u32, head: u32) -> Editor {
    let mut editor = editor_with_document(doc);
    editor.set_selection_scalar(anchor, head);
    editor
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

fn build_backend_expected_documents(doc: &Value) -> BackendExpectedDocuments {
    let legacy = editor_with_document(doc).get_json();
    let encoded = serde_json::to_string(doc).expect("expected editing document should serialize");
    let yrs = yrs_engine_with_document(&encoded)
        .document_json()
        .expect("expected Yrs editing document should import");
    BackendExpectedDocuments { legacy, yrs }
}

fn build_editing_case_expectation(before: &Value, after: Value) -> EditingCaseExpectation {
    EditingCaseExpectation {
        before: build_backend_expected_documents(before),
        after: build_backend_expected_documents(&after),
    }
}

#[allow(clippy::too_many_arguments)]
fn assert_legacy_editing_output(
    editor: &Editor,
    expectation: &EditingCaseExpectation,
    output: &EditorUpdate,
    document_version: u64,
    anchor: u32,
    head: u32,
    can_undo: bool,
    can_redo: bool,
) {
    let state = editor.get_selection_state();
    assert_eq!(state.document_version, document_version);
    assert_eq!(
        state.selection_scalar,
        editor_core::selection::Selection::text(anchor, head),
    );
    assert_eq!(state.history_state.can_undo, can_undo);
    assert_eq!(state.history_state.can_redo, can_redo);
    assert_eq!(&editor.get_json(), &expectation.after.legacy);

    let expected_editor = legacy_editing_fixture(&expectation.after.legacy, anchor, head);
    let expected = expected_editor.get_current_state();
    assert_eq!(output.document_version, document_version);
    assert_eq!(output.selection, expected.selection);
    assert_eq!(output.selection_scalar, expected.selection_scalar);
    assert_eq!(output.active_state, expected.active_state);
    assert_eq!(output.history_state, state.history_state);
    assert_eq!(output.render_elements, expected.render_elements);
    assert_eq!(output.render_blocks, expected.render_blocks);
    let patch = output
        .render_patch
        .as_ref()
        .expect("changed legacy editing output must include a render patch");
    assert_eq!(
        apply_render_patch(&expectation.before.legacy, patch),
        output.render_blocks,
    );
}

#[allow(clippy::too_many_arguments)]
fn assert_legacy_selection_output(
    editor: &Editor,
    expectation: &EditingCaseExpectation,
    output: &EditorSelectionState,
    document_version: u64,
    anchor: u32,
    head: u32,
    can_undo: bool,
    can_redo: bool,
) {
    let state = editor.get_selection_state();
    assert_eq!(&editor.get_json(), &expectation.after.legacy);
    assert_eq!(state.document_version, document_version);
    assert_eq!(state.selection_scalar, output.selection_scalar);
    assert_eq!(state.selection, output.selection);
    assert_eq!(state.active_state, output.active_state);
    assert_eq!(state.history_state, output.history_state);
    assert_eq!(output.document_version, document_version);
    assert_eq!(
        output.selection_scalar,
        editor_core::selection::Selection::text(anchor, head),
    );
    assert_eq!(output.history_state.can_undo, can_undo);
    assert_eq!(output.history_state.can_redo, can_redo);
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
    let expected_editor = legacy_editing_fixture(&expectation.after.legacy, anchor, head);
    assert!(
        expectation.after.legacy.to_string().is_ascii(),
        "editing benchmark profile must remain ASCII for scalar/UTF-16 equality",
    );
    let expected_selection = ResolvedSelection::Text {
        anchor: editor_core::yrs_engine::ResolvedPoint {
            document: expected_editor.scalar_to_doc(anchor),
            scalar: anchor,
            utf16: anchor,
        },
        head: editor_core::yrs_engine::ResolvedPoint {
            document: expected_editor.scalar_to_doc(head),
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
                expected_editor.scalar_to_doc(anchor)
            );
            assert_eq!(resolved_anchor.scalar, anchor);
            assert_eq!(resolved_anchor.utf16, anchor);
            assert_eq!(resolved_head.document, expected_editor.scalar_to_doc(head));
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
    let expected_document = &expectation.after.yrs;
    assert_eq!(&actual_document, expected_document);

    assert_eq!(output.request_id, request_id);
    assert_eq!(output.origin, origin);
    assert_eq!(output.changed, changed);
    assert_eq!(output.document_revision, document_revision);
    assert_eq!(output.state_revision, state_revision);
    assert_eq!(output.selection, expected_selection);
    let expected_state = expected_editor.get_selection_state();
    assert_eq!(output.active_state, expected_state.active_state);
    assert_eq!(output.history_state.can_undo, can_undo);
    assert_eq!(output.history_state.can_redo, can_redo);
    if expectation.before.legacy != expectation.after.legacy {
        assert_eq!(
            apply_render_update(&expectation.before.legacy, &output.render_update),
            expected_editor.get_current_state().render_blocks,
        );
    } else {
        assert_eq!(output.render_update, RenderUpdate::None);
    }
}

fn apply_render_patch(
    before_document: &Value,
    patch: &editor_core::render::incremental::RenderBlocksPatch,
) -> Vec<Vec<RenderElement>> {
    let mut blocks = editor_with_document(before_document)
        .get_current_state()
        .render_blocks;
    blocks.splice(
        patch.start_index..patch.start_index + patch.delete_count,
        patch.blocks.clone(),
    );
    blocks
}

fn apply_render_update(before_document: &Value, update: &RenderUpdate) -> Vec<Vec<RenderElement>> {
    match update {
        RenderUpdate::None => {
            editor_with_document(before_document)
                .get_current_state()
                .render_blocks
        }
        RenderUpdate::Patch(patch) => apply_render_patch(before_document, patch),
        RenderUpdate::Full(blocks) => blocks.clone(),
    }
}

fn collaboration_session_with_document(doc: &Value) -> CollaborationSession {
    collaboration_session_with_document_and_client(doc, None)
}

fn collaboration_session_with_document_and_client(
    doc: &Value,
    client_id: Option<u64>,
) -> CollaborationSession {
    let config = json!({
        "clientId": client_id,
        "fragmentName": "default",
        "initialDocumentJson": doc
    })
    .to_string();
    CollaborationSession::new(&config)
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

fn awareness_messages_for_document(
    doc: &Value,
    peer_count: usize,
    selection_width: u32,
) -> Vec<Vec<u8>> {
    let editor = editor_with_document(doc);
    let content_size = editor.document().content_size().saturating_sub(1).max(1);
    let positions = evenly_spaced_positions(content_size, peer_count);

    positions
        .into_iter()
        .enumerate()
        .map(|(index, position)| {
            let client_id = index as u64 + 2;
            let mut session =
                collaboration_session_with_document_and_client(doc, Some(client_id));
            let anchor = position.max(1);
            let head = if selection_width > 0 && index % 2 == 1 {
                anchor.saturating_add(selection_width).min(content_size)
            } else {
                anchor
            };
            session
                .set_local_awareness(json!({
                    "user": {
                        "name": format!("Peer {}", client_id),
                        "color": format!("#{:06X}", (0x3366FFu32 + (index as u32 * 0x111111)) & 0xFFFFFF),
                        "userId": format!("bench-peer-{}", client_id)
                    },
                    "selection": {
                        "anchor": anchor,
                        "head": head
                    },
                    "focused": true
                }))
                .expect("benchmark awareness should be valid")
                .messages
                .into_iter()
                .next()
                .expect("awareness benchmark should emit a message")
        })
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
