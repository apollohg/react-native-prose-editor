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
            bullet_list(vec![list_item(vec![paragraph(serde_json::json!([
                plain("one")
            ]))])]),
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
            bullet_list(vec![list_item(vec![paragraph(serde_json::json!([
                plain("two")
            ]))])]),
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

#[test]
fn backspace_walks_a_first_list_item_through_a_preceding_blockquote() {
    let mut engine = engine();

    type_text(&mut engine, 1, "quote");
    toggle_blockquote(&mut engine, 2);
    press_return(&mut engine, 3);
    press_return(&mut engine, 4);
    type_text(&mut engine, 5, "item");
    apply_bullet_list(&mut engine, 6);
    place_caret(&mut engine, 7, start_of_second_block(5), Affinity::After);

    press_backspace(&mut engine, 8);
    assert_eq!(
        document(&engine),
        doc(vec![
            blockquote(vec![paragraph(serde_json::json!([plain("quote")]))]),
            paragraph(serde_json::json!([plain("item")])),
        ]),
        "the first press removes the list marker"
    );

    press_backspace(&mut engine, 9);
    assert_eq!(
        document(&engine),
        doc(vec![blockquote(vec![
            paragraph(serde_json::json!([plain("quote")])),
            paragraph(serde_json::json!([plain("item")])),
        ])]),
        "the second press moves the paragraph into the quote"
    );

    press_backspace(&mut engine, 10);
    assert_eq!(
        document(&engine),
        doc(vec![blockquote(vec![paragraph(serde_json::json!([
            plain("quoteitem")
        ]))])]),
        "the third press joins the quoted paragraphs"
    );
}

/// Backspace at the head of a later item merges its text into the item above,
/// leaving one item with one paragraph — not one item holding two paragraphs.
#[test]
fn backspace_at_the_start_of_a_later_item_merges_its_text_into_the_previous_item() {
    let mut engine = engine();
    two_item_list(&mut engine);

    place_caret(&mut engine, 5, START_OF_SECOND_ITEM, Affinity::After);
    let result = press_backspace(&mut engine, 6);

    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![list_item(vec![paragraph(
            serde_json::json!([plain("onetwo")])
        )])])]),
        "merging two bullets must produce a single bullet with a single line"
    );
    let ResolvedSelection::Text { anchor, head } = result.selection else {
        panic!("merging two bullets must leave a text selection");
    };
    assert_eq!(anchor, head, "the caret must remain collapsed");
    assert_eq!(
        head.scalar, 5,
        "the caret must land between the marker-prefixed joined item texts"
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
    let nested_empty = press_return(&mut engine, 6);
    let nested_document_json = document(&engine);
    let parent_empty = press_return(&mut engine, 7);

    let ResolvedSelection::Text {
        head: nested_head, ..
    } = nested_empty.selection
    else {
        panic!("the first Return must leave a text selection");
    };
    let ResolvedSelection::Text {
        head: parent_head, ..
    } = parent_empty.selection
    else {
        panic!("the second Return must leave a text selection");
    };
    let schema = tiptap_schema();
    let nested_document =
        from_prosemirror_json(&nested_document_json, &schema, UnknownTypeMode::Error)
            .expect("the nested empty list document must parse");
    assert_eq!(
        crate::command_planner::outdented_list_item_position(
            &nested_document,
            &nested_document,
            nested_head.document,
            &schema,
        ),
        None,
        "a structural no-op must not invent an outdent selection destination"
    );
    let final_document = from_prosemirror_json(&document(&engine), &schema, UnknownTypeMode::Error)
        .expect("the final list document must parse");
    let final_map = PositionMap::build(&final_document, &schema);
    let expected_document = final_map.scalar_to_doc(nested_head.scalar, &final_document);
    assert_eq!(
        parent_head.document, expected_document,
        "outdenting the empty bullet must move the document selection with it"
    );
    assert_eq!(
        parent_head.scalar, nested_head.scalar,
        "outdenting the empty bullet must keep the caret after its render placeholder: nested={nested_head:?}, parent={parent_head:?}"
    );

    assert_eq!(
        document(&engine),
        doc(vec![bullet_list(vec![
            list_item(vec![
                paragraph(serde_json::json!([plain("one")])),
                bullet_list(vec![list_item(vec![paragraph(serde_json::json!([
                    plain("two")
                ]))])]),
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

#[test]
fn toggling_a_blockquote_wraps_the_current_paragraph() {
    let mut engine = engine();

    type_text(&mut engine, 1, "quoted");
    toggle_blockquote(&mut engine, 2);

    assert_eq!(
        document(&engine),
        doc(vec![blockquote(vec![paragraph(serde_json::json!([
            plain("quoted")
        ]))])])
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
        doc(vec![blockquote(vec![paragraph(serde_json::json!([
            plain("aabb")
        ]))])])
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

#[test]
fn backspace_after_nested_list_unwraps_into_a_paragraph() {
    for list_type in ["bulletList", "orderedList"] {
        let text_block = |text: &str| paragraph(serde_json::json!([plain(text)]));
        let list = |items: Vec<serde_json::Value>| serde_json::json!({"type": list_type, "content": items});
        let parent = list_item(vec![text_block("Parent"), list(vec![list_item(vec![text_block("Nested")])])]);
        let mut engine = engine();
        import_json(&mut engine, doc(vec![list(vec![parent.clone(), list_item(vec![text_block("Last")])])]));
        place_caret(&mut engine, 1, 20, Affinity::After);
        press_backspace(&mut engine, 2);
        let mut expected = self::engine();
        import_json(&mut expected, doc(vec![list(vec![parent.clone()]), text_block("Last")]));
        assert_eq!(document(&engine), document(&expected));
        type_text(&mut engine, 3, "!");
        import_json(&mut expected, doc(vec![list(vec![parent]), text_block("!Last")]));
        assert_eq!(document(&engine), document(&expected));
    }
}

#[test]
fn backspace_after_nested_list_preserves_marks_siblings_and_history() {
    for list_type in ["bulletList", "orderedList"] {
        for depth in [1, 3] {
            let text_block = |text: &str| paragraph(serde_json::json!([plain(text)]));
            let nested = |tail: serde_json::Value| {
                let mut list = serde_json::json!({ "type": list_type, "content": [
                    list_item(vec![text_block("Sibling")]), list_item(vec![tail]),
                ] });
                for _ in 1..depth {
                    list = bullet_list(vec![list_item(vec![text_block("Middle"), list])]);
                }
                list
            };
            let list = |items: Vec<serde_json::Value>| serde_json::json!({"type": list_type, "content": items});
            let original = doc(vec![list(vec![
                list_item(vec![text_block("Parent"), nested(text_block("Nested"))]),
                list_item(vec![
                    paragraph(
                        serde_json::json!([marked("Last", &["bold"]), {"type": "hardBreak"}, plain("💡")]),
                    ),
                    text_block("Extra"),
                    bullet_list(vec![list_item(vec![text_block("Child")])]),
                ]),
                list_item(vec![text_block("Following")]),
            ])]);
            let schema = tiptap_schema();
            let parsed = from_prosemirror_json(&original, &schema, UnknownTypeMode::Error).unwrap();
            let rendered = crate::render::rendered_text(&parsed, &schema);
            let offset = rendered[..rendered.find("Last").unwrap()].chars().count() as u32;
            let mut engine = engine();
            import_json(&mut engine, original);
            let before = document(&engine);
            place_caret(&mut engine, 1, offset, Affinity::After);
            press_backspace(&mut engine, 2);
            let expected = doc(vec![
                list(vec![list_item(vec![text_block("Parent"), nested(text_block("Nested"))])]),
                paragraph(serde_json::json!([marked("Last", &["bold"]), {"type": "hardBreak"}, plain("💡")])),
                text_block("Extra"),
                bullet_list(vec![list_item(vec![text_block("Child")])]),
                list(vec![list_item(vec![text_block("Following")])]),
            ]);
            // Import normalizes default attributes, including ordered-list start.
            let mut expected_engine = self::engine();
            import_json(&mut expected_engine, expected);
            assert_eq!(document(&engine), document(&expected_engine));
            engine.undo(3).unwrap().unwrap();
            assert_eq!(document(&engine), before);
            engine.redo(4).unwrap().unwrap();
            assert_eq!(document(&engine), document(&expected_engine));
        }
    }
}
