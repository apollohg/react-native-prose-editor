//! Keystroke-level regression coverage for typing, marks, line returns, and
//! backspace.
//!
//! Every test here drives [`TypedCommand`] — the same command surface a real
//! keypress reaches through the native bridge — rather than hand-authored
//! [`TypedOperation`]s. That is deliberate: the operation-level suites already
//! cover the engine's primitives, and the defects users actually report live in
//! the *planner* that turns one keystroke into those primitives.
//!
//! Scalar offsets used below follow the mapping the engine actually implements:
//! text characters occupy one scalar each, a block boundary occupies exactly
//! one more, and a position at the end of a block is only representable with
//! `Affinity::Before` (`Affinity::After` there is rejected as unrepresentable).
//! So for `"ab"` + line return + `"cd"`, offset 2 is the end of the first
//! block, and offset 3 is the start of the second.

use crate::boundary::ResourceLimits;
use crate::tiptap_schema;
use crate::yrs_engine::{
    Affinity, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    RevisionedPosition, SelectionInput, SelectionIntent, TransactionOrigin, TypedCommand,
    TypedTransaction, YrsDocumentEngine, YrsEngineConfig,
};

fn engine() -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap()
}

/// Type `text` at the caret, exactly as the input bridge does per keystroke.
fn type_text(engine: &mut YrsDocumentEngine, request_id: u64, text: &str) {
    engine
        .apply_command(request_id, TypedCommand::InsertText { text: text.into() })
        .unwrap_or_else(|error| panic!("typing {text:?} must apply: {error:?}"))
        .unwrap_or_else(|| panic!("typing {text:?} must not be a no-op"));
}

/// Toggle a mark at the caret (collapsed) or over the selection.
fn toggle_mark(engine: &mut YrsDocumentEngine, request_id: u64, mark_type: &str) {
    engine
        .apply_command(
            request_id,
            TypedCommand::ToggleMark {
                mark_type: mark_type.into(),
            },
        )
        .unwrap_or_else(|error| panic!("toggling {mark_type} must apply: {error:?}"));
}

/// Press Return.
fn press_return(engine: &mut YrsDocumentEngine, request_id: u64) {
    engine
        .apply_command(request_id, TypedCommand::SplitBlock)
        .unwrap_or_else(|error| panic!("Return must apply: {error:?}"))
        .unwrap_or_else(|| panic!("Return must not be a no-op"));
}

/// Press Backspace, surfacing a rejection as a readable failure rather than an
/// `unwrap` panic — the whole point of these tests is *which* backspaces the
/// planner refuses.
fn press_backspace(engine: &mut YrsDocumentEngine, request_id: u64) {
    match engine.apply_command(request_id, TypedCommand::DeleteBackward) {
        Ok(Some(_)) => {}
        Ok(None) => panic!(
            "Backspace was refused as not-applicable; document is {}",
            engine.document_json().unwrap()
        ),
        Err(error) => panic!(
            "Backspace was rejected with {} ({}); document is {}",
            error.code,
            error.message,
            engine.document_json().unwrap()
        ),
    }
}

fn place_caret(engine: &mut YrsDocumentEngine, request_id: u64, offset: u32, affinity: Affinity) {
    let position = RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: position,
                head: position,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap_or_else(|error| panic!("placing the caret at {offset} must apply: {error:?}"));
}

/// The caret position a user reaches by tapping at the very start of the second
/// line: one scalar past the first block's own length (the block boundary).
fn start_of_second_block(first_block_len: u32) -> u32 {
    first_block_len + 1
}

fn document(engine: &YrsDocumentEngine) -> serde_json::Value {
    engine.document_json().expect("document must render")
}

fn paragraph(runs: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "type": "paragraph", "content": runs })
}

fn plain(text: &str) -> serde_json::Value {
    serde_json::json!({ "type": "text", "text": text })
}

fn marked(text: &str, marks: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "type": "text",
        "text": text,
        "marks": marks.iter().map(|m| serde_json::json!({ "type": m })).collect::<Vec<_>>(),
    })
}

fn doc(blocks: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({ "type": "doc", "content": blocks })
}

// ---------------------------------------------------------------------------
// 1. Typing sentences, adding marks
// ---------------------------------------------------------------------------

/// Type a sentence, switch bold on mid-way, then italic, exactly as a user
/// tapping toolbar buttons between words would.
#[test]
fn typing_a_sentence_and_toggling_marks_mid_flow_produces_three_runs() {
    let mut engine = engine();

    type_text(&mut engine, 1, "The quick brown fox");
    toggle_mark(&mut engine, 2, "bold");
    type_text(&mut engine, 3, " jumps");
    toggle_mark(&mut engine, 4, "bold");
    toggle_mark(&mut engine, 5, "italic");
    type_text(&mut engine, 6, " over");
    toggle_mark(&mut engine, 7, "italic");
    type_text(&mut engine, 8, " the lazy dog");

    assert_eq!(
        document(&engine),
        doc(vec![paragraph(serde_json::json!([
            plain("The quick brown fox"),
            marked(" jumps", &["bold"]),
            marked(" over", &["italic"]),
            plain(" the lazy dog"),
        ]))]),
        "each toggle must start a new run and must not retroactively re-mark earlier text"
    );
}

/// A collapsed toggle is stored-mark state only: it must not touch the document
/// until the next character actually arrives.
#[test]
fn a_collapsed_mark_toggle_only_takes_effect_when_the_next_character_is_typed() {
    let mut engine = engine();

    type_text(&mut engine, 1, "plain");
    toggle_mark(&mut engine, 2, "bold");

    assert_eq!(
        document(&engine),
        doc(vec![paragraph(serde_json::json!([plain("plain")]))]),
        "toggling bold with a collapsed caret must not alter the document yet"
    );
    assert_eq!(
        engine.stored_marks().map(|marks| marks
            .iter()
            .map(|mark| mark.mark_type())
            .collect::<Vec<_>>()),
        Some(vec!["bold"]),
        "the pending bold must be held as a stored mark"
    );

    type_text(&mut engine, 3, "BOLD");

    assert_eq!(
        document(&engine),
        doc(vec![paragraph(serde_json::json!([
            plain("plain"),
            marked("BOLD", &["bold"]),
        ]))])
    );
}

/// Two marks stacked on the same run, then both switched off.
#[test]
fn stacking_bold_and_italic_then_clearing_both_returns_to_plain_typing() {
    let mut engine = engine();

    toggle_mark(&mut engine, 1, "bold");
    toggle_mark(&mut engine, 2, "italic");
    type_text(&mut engine, 3, "both");
    toggle_mark(&mut engine, 4, "bold");
    toggle_mark(&mut engine, 5, "italic");
    type_text(&mut engine, 6, "neither");

    assert_eq!(
        document(&engine),
        doc(vec![paragraph(serde_json::json!([
            marked("both", &["bold", "italic"]),
            plain("neither"),
        ]))])
    );
}

// ---------------------------------------------------------------------------
// 2. Line returns, with and against marks
// ---------------------------------------------------------------------------

/// Return at the end of a bold run, then typing on the new line: the new line
/// must inherit the stored bold, and the first line must keep its own runs.
#[test]
fn return_after_a_bold_run_carries_the_stored_mark_onto_the_new_line() {
    let mut engine = engine();

    type_text(&mut engine, 1, "start ");
    toggle_mark(&mut engine, 2, "bold");
    type_text(&mut engine, 3, "bold");
    press_return(&mut engine, 4);
    type_text(&mut engine, 5, "next");

    assert_eq!(
        document(&engine),
        doc(vec![
            paragraph(serde_json::json!([plain("start "), marked("bold", &["bold"])])),
            paragraph(serde_json::json!([marked("next", &["bold"])])),
        ]),
        "bold was still active when Return was pressed, so the new line continues bold"
    );
}

/// Return in the middle of a marked run splits the run across both blocks
/// without leaking the mark onto neighbouring plain text.
#[test]
fn return_inside_a_marked_run_splits_it_across_both_lines() {
    let mut engine = engine();

    type_text(&mut engine, 1, "aa");
    toggle_mark(&mut engine, 2, "bold");
    type_text(&mut engine, 3, "bbcc");
    toggle_mark(&mut engine, 4, "bold");
    type_text(&mut engine, 5, "dd");

    // Caret between "bb" and "cc": 2 plain + 2 bold characters in.
    place_caret(&mut engine, 6, 4, Affinity::After);
    press_return(&mut engine, 7);

    assert_eq!(
        document(&engine),
        doc(vec![
            paragraph(serde_json::json!([plain("aa"), marked("bb", &["bold"])])),
            paragraph(serde_json::json!([marked("cc", &["bold"]), plain("dd")])),
        ])
    );
}

// ---------------------------------------------------------------------------
// 3. Backspace
// ---------------------------------------------------------------------------

/// The ordinary case: backspace removes characters one at a time from the end
/// of a marked run without disturbing the run in front of it.
#[test]
fn backspace_deletes_characters_out_of_a_marked_run_one_at_a_time() {
    let mut engine = engine();

    type_text(&mut engine, 1, "keep");
    toggle_mark(&mut engine, 2, "bold");
    type_text(&mut engine, 3, "gone");

    for (index, request_id) in (10..14u64).enumerate() {
        press_backspace(&mut engine, request_id);
        let remaining = &"gone"[..3 - index];
        let expected = if remaining.is_empty() {
            paragraph(serde_json::json!([plain("keep")]))
        } else {
            paragraph(serde_json::json!([plain("keep"), marked(remaining, &["bold"])]))
        };
        assert_eq!(
            document(&engine),
            doc(vec![expected]),
            "after {} backspaces the bold run should be {remaining:?}",
            index + 1
        );
    }
}

/// Backspace at the start of a *non-empty* second line must join the two lines.
/// This is the plainest possible form of "backspacing a line return" — no marks
/// anywhere — and it is the shape every marked variant below builds on.
#[test]
fn backspace_at_the_start_of_a_second_line_joins_it_onto_the_first() {
    let mut engine = engine();

    type_text(&mut engine, 1, "first");
    press_return(&mut engine, 2);
    type_text(&mut engine, 3, "second");

    place_caret(&mut engine, 4, start_of_second_block(5), Affinity::After);
    press_backspace(&mut engine, 5);

    assert_eq!(
        document(&engine),
        doc(vec![paragraph(serde_json::json!([plain("firstsecond")]))]),
        "backspace at the head of the second line must merge it into the first"
    );
}

/// The line return sits immediately *after* a mark: the first line ends bold,
/// the second starts plain. Joining must keep the boundary exactly where it
/// was — the bold must not swallow the text that follows it.
#[test]
fn backspacing_a_line_return_that_follows_a_marked_run_keeps_the_mark_boundary() {
    let mut engine = engine();

    toggle_mark(&mut engine, 1, "bold");
    type_text(&mut engine, 2, "bold");
    toggle_mark(&mut engine, 3, "bold");
    press_return(&mut engine, 4);
    type_text(&mut engine, 5, "plain");

    place_caret(&mut engine, 6, start_of_second_block(4), Affinity::After);
    press_backspace(&mut engine, 7);

    assert_eq!(
        document(&engine),
        doc(vec![paragraph(serde_json::json!([
            marked("bold", &["bold"]),
            plain("plain"),
        ]))]),
        "the joined line must keep bold on the first run only"
    );
}

/// The line return sits immediately *before* a mark: the first line is plain,
/// the second starts bold. Joining must not extend the bold backwards over the
/// plain text it lands against.
#[test]
fn backspacing_a_line_return_that_precedes_a_marked_run_keeps_the_mark_boundary() {
    let mut engine = engine();

    type_text(&mut engine, 1, "plain");
    press_return(&mut engine, 2);
    toggle_mark(&mut engine, 3, "bold");
    type_text(&mut engine, 4, "bold");

    place_caret(&mut engine, 5, start_of_second_block(5), Affinity::After);
    press_backspace(&mut engine, 6);

    assert_eq!(
        document(&engine),
        doc(vec![paragraph(serde_json::json!([
            plain("plain"),
            marked("bold", &["bold"]),
        ]))]),
        "the joined line must keep the plain run plain"
    );
}

/// Both sides of the line return carry the same mark: joining should produce
/// one continuous marked run, not two adjacent ones.
#[test]
fn backspacing_a_line_return_between_two_bold_runs_produces_one_bold_run() {
    let mut engine = engine();

    toggle_mark(&mut engine, 1, "bold");
    type_text(&mut engine, 2, "one");
    press_return(&mut engine, 3);
    type_text(&mut engine, 4, "two");

    place_caret(&mut engine, 5, start_of_second_block(3), Affinity::After);
    press_backspace(&mut engine, 6);

    assert_eq!(
        document(&engine),
        doc(vec![paragraph(serde_json::json!([marked("onetwo", &["bold"])]))]),
        "identically marked text either side of the removed break must coalesce"
    );
}

/// Backspacing an *empty* trailing line: the line return is removed and the
/// caret returns to the end of the previous line's marked run.
#[test]
fn backspacing_an_empty_line_removes_it_and_leaves_the_marked_line_intact() {
    let mut engine = engine();

    toggle_mark(&mut engine, 1, "bold");
    type_text(&mut engine, 2, "bold");
    press_return(&mut engine, 3);

    press_backspace(&mut engine, 4);

    assert_eq!(
        document(&engine),
        doc(vec![paragraph(serde_json::json!([marked("bold", &["bold"])]))])
    );
}

/// The full round trip a user performs when they change their mind: type two
/// marked lines, then hold backspace until the document is empty again. This
/// walks the caret through the marked run, across the line return, and into the
/// first line's text in one continuous sequence.
#[test]
fn holding_backspace_walks_back_through_marks_and_line_returns_to_an_empty_document() {
    let mut engine = engine();

    type_text(&mut engine, 1, "ab");
    toggle_mark(&mut engine, 2, "bold");
    type_text(&mut engine, 3, "cd");
    press_return(&mut engine, 4);
    type_text(&mut engine, 5, "ef");

    // "ef" (2) + the line return (1) + "cd" (2) + "ab" (2) = 7 keystrokes.
    for (index, request_id) in (10..17u64).enumerate() {
        press_backspace(&mut engine, request_id);
        assert!(
            engine.document_json().is_some(),
            "the document must stay renderable after backspace {}",
            index + 1
        );
    }

    assert_eq!(
        document(&engine),
        serde_json::json!({
            "type": "doc",
            "content": [{ "type": "paragraph" }]
        }),
        "backspacing everything must leave a single empty paragraph"
    );
}

// ---------------------------------------------------------------------------
// 4. Lists
// ---------------------------------------------------------------------------
//
// Scalar offsets in a two-item bullet list `one` / `two`, verified against the
// engine rather than derived on paper: offsets 0-2 all resolve to the head of
// the first item's text, 3-4 step through it, 5 is its end (`Affinity::Before`
// only), 6-8 all resolve to the head of the second item's text. The repeated
// offsets are the list/item/paragraph opening tokens, which share a rendered
// position. `START_OF_SECOND_ITEM` is the caret a user gets by tapping at the
// very start of the second bullet.

const START_OF_SECOND_ITEM: u32 = 6;
/// Head of the quoted line when a blockquote follows a four-character paragraph.
const START_OF_QUOTE_AFTER_PARAGRAPH: u32 = 5;
/// Head of the second line inside a blockquote whose first line is `aa`.
const START_OF_SECOND_QUOTE_LINE: u32 = 3;

fn apply_bullet_list(engine: &mut YrsDocumentEngine, request_id: u64) {
    engine
        .apply_command(
            request_id,
            TypedCommand::ApplyListType {
                list_type: "bulletList".into(),
            },
        )
        .unwrap_or_else(|error| panic!("applying a bullet list must apply: {error:?}"))
        .unwrap_or_else(|| panic!("applying a bullet list must not be a no-op"));
}

fn indent_item(engine: &mut YrsDocumentEngine, request_id: u64) {
    engine
        .apply_command(request_id, TypedCommand::IndentListItem)
        .unwrap_or_else(|error| panic!("indent must apply: {error:?}"))
        .unwrap_or_else(|| panic!("indent must not be a no-op"));
}

fn outdent_item(engine: &mut YrsDocumentEngine, request_id: u64) {
    engine
        .apply_command(request_id, TypedCommand::OutdentListItem)
        .unwrap_or_else(|error| panic!("outdent must apply: {error:?}"))
        .unwrap_or_else(|| panic!("outdent must not be a no-op"));
}

fn toggle_blockquote(engine: &mut YrsDocumentEngine, request_id: u64) {
    engine
        .apply_command(request_id, TypedCommand::ToggleBlockquote)
        .unwrap_or_else(|error| panic!("blockquote toggle must apply: {error:?}"))
        .unwrap_or_else(|| panic!("blockquote toggle must not be a no-op"));
}

fn list_item(blocks: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({ "type": "listItem", "content": blocks })
}

fn bullet_list(items: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({ "type": "bulletList", "content": items })
}

fn blockquote(blocks: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({ "type": "blockquote", "content": blocks })
}

fn empty_paragraph() -> serde_json::Value {
    serde_json::json!({ "type": "paragraph" })
}

/// A two-item bullet list built the way a user builds one.
fn two_item_list(engine: &mut YrsDocumentEngine) {
    type_text(engine, 1, "one");
    apply_bullet_list(engine, 2);
    press_return(engine, 3);
    type_text(engine, 4, "two");
}

#[test]
fn applying_a_bullet_list_wraps_the_current_paragraph_in_an_item() {
    let mut engine = engine();

    type_text(&mut engine, 1, "one");
    apply_bullet_list(&mut engine, 2);

    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![list_item(vec![paragraph(
            serde_json::json!([plain("one")])
        )])])])
    );
}

/// Converting a line into a list item must leave the caret in that line's text,
/// not stranded at a structural boundary — the next character typed has to land
/// where the user left off.
#[test]
fn converting_a_line_into_a_list_item_keeps_the_caret_in_the_text() {
    let mut engine = engine();

    type_text(&mut engine, 1, "one");
    apply_bullet_list(&mut engine, 2);
    type_text(&mut engine, 3, "!");

    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![list_item(vec![paragraph(
            serde_json::json!([plain("one!")])
        )])])]),
        "the caret must still sit at the end of the converted line"
    );
}

#[test]
fn return_in_a_list_item_starts_a_sibling_item() {
    let mut engine = engine();
    two_item_list(&mut engine);

    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![
            list_item(vec![paragraph(serde_json::json!([plain("one")]))]),
            list_item(vec![paragraph(serde_json::json!([plain("two")]))]),
        ])])
    );
}

/// Return on an empty item escapes the list entirely.
#[test]
fn return_on_an_empty_list_item_escapes_the_list() {
    let mut engine = engine();

    type_text(&mut engine, 1, "one");
    apply_bullet_list(&mut engine, 2);
    press_return(&mut engine, 3);
    press_return(&mut engine, 4);

    assert_eq!(
        document(&engine),
        doc(vec![
            bullet_list(vec![list_item(vec![paragraph(serde_json::json!([plain(
                "one"
            )]))])]),
            empty_paragraph(),
        ])
    );
}

#[test]
fn indent_nests_an_item_under_the_previous_one_and_outdent_restores_it() {
    let mut engine = engine();
    two_item_list(&mut engine);

    indent_item(&mut engine, 5);
    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![list_item(vec![
            paragraph(serde_json::json!([plain("one")])),
            bullet_list(vec![list_item(vec![paragraph(serde_json::json!([plain(
                "two"
            )]))])]),
        ])])]),
        "the indented item becomes a nested list inside the item above it"
    );

    outdent_item(&mut engine, 6);
    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![
            list_item(vec![paragraph(serde_json::json!([plain("one")]))]),
            list_item(vec![paragraph(serde_json::json!([plain("two")]))]),
        ])]),
        "outdent restores the flat two-item list"
    );
}

#[test]
fn backspace_at_the_start_of_the_first_item_lifts_it_out_of_the_list() {
    let mut engine = engine();

    type_text(&mut engine, 1, "one");
    apply_bullet_list(&mut engine, 2);
    place_caret(&mut engine, 3, 0, Affinity::After);
    press_backspace(&mut engine, 4);

    assert_eq!(
        document(&engine),
        doc(vec![paragraph(serde_json::json!([plain("one")]))]),
        "the list disappears and its only line becomes a plain paragraph"
    );
}

/// Backspace at the head of a later item merges its text into the item above,
/// leaving one item with one paragraph — not one item holding two paragraphs.
#[test]
fn backspace_at_the_start_of_a_later_item_merges_its_text_into_the_previous_item() {
    let mut engine = engine();
    two_item_list(&mut engine);

    place_caret(&mut engine, 5, START_OF_SECOND_ITEM, Affinity::After);
    press_backspace(&mut engine, 6);

    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![list_item(vec![paragraph(
            serde_json::json!([plain("onetwo")])
        )])])]),
        "merging two bullets must produce a single bullet with a single line"
    );
}

/// Backspace at the head of a nested item outdents it, the mirror of Tab.
#[test]
fn backspace_at_the_start_of_a_nested_item_outdents_it() {
    let mut engine = engine();
    two_item_list(&mut engine);
    indent_item(&mut engine, 5);

    place_caret(&mut engine, 6, START_OF_SECOND_ITEM, Affinity::After);
    press_backspace(&mut engine, 7);

    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![
            list_item(vec![paragraph(serde_json::json!([plain("one")]))]),
            list_item(vec![paragraph(serde_json::json!([plain("two")]))]),
        ])]),
        "the nested bullet returns to the parent level with its text intact"
    );
}

#[test]
fn return_on_an_empty_nested_item_outdents_to_the_parent_list() {
    let mut engine = engine();
    two_item_list(&mut engine);
    indent_item(&mut engine, 5);
    press_return(&mut engine, 6);
    press_return(&mut engine, 7);

    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![
            list_item(vec![
                paragraph(serde_json::json!([plain("one")])),
                bullet_list(vec![list_item(vec![paragraph(serde_json::json!([plain(
                    "two"
                )]))])]),
            ]),
            list_item(vec![empty_paragraph()]),
        ])])
    );
}

/// Return in an empty editor leaves two blank lines.
///
/// Neither line holds a character, but the document is no longer the single
/// default block a fresh editor starts with: the user has made a line they can
/// see, put a caret in, and delete again.
#[test]
fn return_in_an_empty_editor_leaves_two_blank_lines() {
    let mut engine = engine();

    press_return(&mut engine, 1);

    assert_eq!(
        document(&engine),
        doc(vec![empty_paragraph(), empty_paragraph()]),
        "Return on an empty line adds a second empty line rather than doing nothing"
    );

    // Where the caret actually is, proven by what the next character does:
    // typing must land on the second line, not the first.
    type_text(&mut engine, 2, "x");
    assert_eq!(
        document(&engine),
        doc(vec![
            empty_paragraph(),
            paragraph(serde_json::json!([plain("x")])),
        ]),
        "the caret must sit on the blank line Return created"
    );

    // And backspace walks it straight back off again.
    press_backspace(&mut engine, 3);
    press_backspace(&mut engine, 4);
    assert_eq!(
        document(&engine),
        doc(vec![empty_paragraph()]),
        "backspace removes the character, then the blank line Return created"
    );
}

/// Pressing the list button in an empty editor, then immediately backspacing.
///
/// This is the shortest path to a lone empty bullet and it never involves any
/// text: the item is empty from the moment it is created, so the backspace has
/// to remove the list structure itself rather than falling out of a text
/// deletion. Reaching the same state by deleting text (see
/// [`backspacing_a_lone_list_item_away_returns_to_an_empty_document`]) exercises
/// a different path into it.
#[test]
fn backspacing_an_empty_list_item_created_in_an_empty_editor_returns_to_an_empty_document() {
    let mut engine = engine();

    apply_bullet_list(&mut engine, 1);
    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![list_item(vec![empty_paragraph()])])]),
        "the list button on an empty editor makes one empty bullet"
    );

    press_backspace(&mut engine, 2);

    assert_eq!(
        document(&engine),
        doc(vec![empty_paragraph()]),
        "backspace must remove the empty bullet and leave an empty editor"
    );
}

/// A document whose entire content is one bullet: backspacing must walk the
/// text out, drop the bullet, and land back on an empty editor.
#[test]
fn backspacing_a_lone_list_item_away_returns_to_an_empty_document() {
    let mut engine = engine();

    type_text(&mut engine, 1, "ab");
    apply_bullet_list(&mut engine, 2);

    // "ab" (2 keystrokes) then one more to drop the now-empty bullet.
    press_backspace(&mut engine, 3);
    press_backspace(&mut engine, 4);
    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![list_item(vec![empty_paragraph()])])]),
        "deleting the text leaves an empty bullet"
    );

    press_backspace(&mut engine, 5);
    assert_eq!(
        document(&engine),
        doc(vec![empty_paragraph()]),
        "one more backspace removes the bullet and leaves an empty document"
    );
}

// ---------------------------------------------------------------------------
// 5. Blockquotes
// ---------------------------------------------------------------------------

#[test]
fn toggling_a_blockquote_wraps_the_current_paragraph() {
    let mut engine = engine();

    type_text(&mut engine, 1, "quoted");
    toggle_blockquote(&mut engine, 2);

    assert_eq!(
        document(&engine),
        doc(vec![blockquote(vec![paragraph(serde_json::json!([plain(
            "quoted"
        )]))])])
    );
}

#[test]
fn return_inside_a_blockquote_adds_another_quoted_line() {
    let mut engine = engine();

    type_text(&mut engine, 1, "aa");
    toggle_blockquote(&mut engine, 2);
    press_return(&mut engine, 3);
    type_text(&mut engine, 4, "bb");

    assert_eq!(
        document(&engine),
        doc(vec![blockquote(vec![
            paragraph(serde_json::json!([plain("aa")])),
            paragraph(serde_json::json!([plain("bb")])),
        ])])
    );
}

#[test]
fn return_on_an_empty_quoted_line_escapes_the_blockquote() {
    let mut engine = engine();

    type_text(&mut engine, 1, "quoted");
    toggle_blockquote(&mut engine, 2);
    press_return(&mut engine, 3);
    press_return(&mut engine, 4);

    assert_eq!(
        document(&engine),
        doc(vec![
            blockquote(vec![paragraph(serde_json::json!([plain("quoted")]))]),
            empty_paragraph(),
        ])
    );
}

#[test]
fn backspace_at_the_start_of_a_later_quoted_line_merges_it_upwards() {
    let mut engine = engine();

    type_text(&mut engine, 1, "aa");
    toggle_blockquote(&mut engine, 2);
    press_return(&mut engine, 3);
    type_text(&mut engine, 4, "bb");

    place_caret(&mut engine, 5, START_OF_SECOND_QUOTE_LINE, Affinity::After);
    press_backspace(&mut engine, 6);

    assert_eq!(
        document(&engine),
        doc(vec![blockquote(vec![paragraph(serde_json::json!([plain(
            "aabb"
        )]))])])
    );
}

/// Backspace at the head of a blockquote's only line lifts that line out of the
/// quote, the same escape hatch Return-on-empty provides.
#[test]
fn backspace_at_the_start_of_a_lone_blockquote_lifts_the_line_out() {
    let mut engine = engine();

    type_text(&mut engine, 1, "quoted");
    toggle_blockquote(&mut engine, 2);
    place_caret(&mut engine, 3, 0, Affinity::After);
    press_backspace(&mut engine, 4);

    assert_eq!(
        document(&engine),
        doc(vec![paragraph(serde_json::json!([plain("quoted")]))]),
        "the quote is removed and its text survives as a plain paragraph"
    );
}

/// With a paragraph above it, backspacing at the head of a blockquote lifts the
/// quoted line out to sit beside that paragraph. It does not merge the two
/// lines: the first press escapes the quote, a second press would then join.
#[test]
fn backspace_at_the_start_of_a_blockquote_below_a_paragraph_lifts_the_line_out() {
    let mut engine = engine();

    type_text(&mut engine, 1, "para");
    press_return(&mut engine, 2);
    type_text(&mut engine, 3, "quoted");
    toggle_blockquote(&mut engine, 4);

    place_caret(
        &mut engine,
        5,
        START_OF_QUOTE_AFTER_PARAGRAPH,
        Affinity::After,
    );
    press_backspace(&mut engine, 6);

    assert_eq!(
        document(&engine),
        doc(vec![
            paragraph(serde_json::json!([plain("para")])),
            paragraph(serde_json::json!([plain("quoted")])),
        ]),
        "escaping the quote must preserve both lines separately"
    );
}

// ---------------------------------------------------------------------------
// 6. Marks inside structures
// ---------------------------------------------------------------------------

#[test]
fn bold_carries_across_return_inside_a_list_item() {
    let mut engine = engine();

    toggle_mark(&mut engine, 1, "bold");
    type_text(&mut engine, 2, "bold");
    apply_bullet_list(&mut engine, 3);
    press_return(&mut engine, 4);
    type_text(&mut engine, 5, "next");

    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![
            list_item(vec![paragraph(serde_json::json!([marked(
                "bold",
                &["bold"]
            )]))]),
            list_item(vec![paragraph(serde_json::json!([marked(
                "next",
                &["bold"]
            )]))]),
        ])])
    );
}

#[test]
fn bold_carries_across_return_inside_a_blockquote() {
    let mut engine = engine();

    toggle_mark(&mut engine, 1, "bold");
    type_text(&mut engine, 2, "bold");
    toggle_blockquote(&mut engine, 3);
    press_return(&mut engine, 4);
    type_text(&mut engine, 5, "next");

    assert_eq!(
        document(&engine),
        doc(vec![blockquote(vec![
            paragraph(serde_json::json!([marked("bold", &["bold"])])),
            paragraph(serde_json::json!([marked("next", &["bold"])])),
        ])])
    );
}

/// Wrapping and unwrapping a list is a structural move: the marks on the text
/// must be untouched by both directions.
#[test]
fn wrapping_and_unwrapping_a_list_preserves_marks_on_the_text() {
    let mut engine = engine();

    type_text(&mut engine, 1, "aa");
    toggle_mark(&mut engine, 2, "bold");
    type_text(&mut engine, 3, "bb");

    let runs = serde_json::json!([plain("aa"), marked("bb", &["bold"])]);

    apply_bullet_list(&mut engine, 4);
    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![list_item(vec![paragraph(
            runs.clone()
        )])])])
    );

    engine
        .apply_command(5, TypedCommand::UnwrapFromList)
        .unwrap()
        .expect("unwrap must apply");
    assert_eq!(document(&engine), doc(vec![paragraph(runs)]));
}
