#[test]
fn history_metadata_restores_group_before_and_latest_after_selection_and_stored_marks() {
    let mut harness = Harness::new();
    harness.import_json(PLAIN_AB);
    harness
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Skip,
            vec![],
            cursor(1),
        )
        .unwrap();
    harness
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Skip,
            vec![TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("bold"),
            }],
            SelectionIntent::Preserve,
        )
        .unwrap();
    let before_selection = harness.engine.resolved_selection().cloned();
    let before_stored_marks = harness.engine.stored_marks().map(<[Mark]>::to_vec);
    assert_eq!(before_stored_marks, Some(vec![mark("bold")]));

    harness
        .apply(
            TransactionOrigin::LocalInput,
            HistoryPolicy::Auto,
            vec![TypedOperation::InsertText {
                at: point(1),
                text: "a".into(),
                marks: vec![mark("bold")],
            }],
            SelectionIntent::UseOperationResult,
        )
        .unwrap();
    harness.advance(100);
    harness
        .apply(
            TransactionOrigin::LocalInput,
            HistoryPolicy::Auto,
            vec![TypedOperation::InsertText {
                at: point(1),
                text: "b".into(),
                marks: vec![mark("bold")],
            }],
            SelectionIntent::UseOperationResult,
        )
        .unwrap();
    let after_selection = harness.engine.resolved_selection().cloned();
    let after_stored_marks = harness.engine.stored_marks().map(<[Mark]>::to_vec);

    harness
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Skip,
            vec![],
            cursor(0),
        )
        .unwrap();
    harness
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Skip,
            vec![TypedOperation::RemoveMark {
                range: range(0, 0),
                mark_type: "bold".into(),
            }],
            SelectionIntent::Preserve,
        )
        .unwrap();
    assert_ne!(
        harness.engine.resolved_selection().cloned(),
        after_selection
    );
    assert_eq!(harness.engine.stored_marks(), Some([].as_slice()));

    undo_commit(&mut harness);
    assert_eq!(
        harness.engine.resolved_selection().cloned(),
        before_selection
    );
    assert_eq!(
        harness.engine.stored_marks().map(<[Mark]>::to_vec),
        before_stored_marks
    );
    redo_commit(&mut harness);
    assert_eq!(
        harness.engine.resolved_selection().cloned(),
        after_selection
    );
    assert_eq!(
        harness.engine.stored_marks().map(<[Mark]>::to_vec),
        after_stored_marks
    );
}

#[test]
fn undo_uses_metadata_from_the_actionable_item_below_an_inert_stack_top() {
    let mut harness = Harness::new();
    harness.import_json(PLAIN_AB);
    harness
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Skip,
            vec![],
            cursor(1),
        )
        .unwrap();
    harness
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Skip,
            vec![TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("bold"),
            }],
            SelectionIntent::Preserve,
        )
        .unwrap();

    harness
        .apply(
            TransactionOrigin::LocalInput,
            HistoryPolicy::Boundary,
            vec![TypedOperation::InsertText {
                at: point(1),
                text: "x".into(),
                marks: vec![mark("bold")],
            }],
            SelectionIntent::UseOperationResult,
        )
        .unwrap();

    harness
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Skip,
            vec![
                TypedOperation::RemoveMark {
                    range: range(2, 2),
                    mark_type: "bold".into(),
                },
                TypedOperation::AddMark {
                    range: range(2, 2),
                    mark: mark("italic"),
                },
            ],
            SelectionIntent::Preserve,
        )
        .unwrap();

    harness
        .apply(
            TransactionOrigin::LocalInput,
            HistoryPolicy::Boundary,
            vec![TypedOperation::InsertText {
                at: point(2),
                text: "y".into(),
                marks: vec![mark("italic")],
            }],
            SelectionIntent::UseOperationResult,
        )
        .unwrap();
    harness.delete(2, 3, HistoryPolicy::Skip).unwrap();

    undo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "ab");
    assert_eq!(
        harness.engine.stored_marks().map(<[Mark]>::to_vec),
        Some(vec![mark("bold")])
    );
}

#[test]
fn exact_group_ceiling_is_usable_and_next_group_rolls_the_whole_epoch() {
    let limits = EditingLimits {
        max_undo_groups: 2,
        ..EditingLimits::default()
    };

    let mut exact = Harness::with_limits(limits.clone());
    exact
        .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    exact
        .insert("b", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    undo_commit(&mut exact);
    undo_commit(&mut exact);
    assert_eq!(text(&exact.engine), "");
    assert_eq!(exact.undo().unwrap(), None);

    let mut rollover = Harness::with_limits(limits);
    rollover
        .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    rollover
        .insert("b", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    rollover
        .insert("c", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    undo_commit(&mut rollover);
    assert_eq!(text(&rollover.engine), "ab");
    assert_eq!(rollover.undo().unwrap(), None);
}

#[test]
fn exact_retained_unit_ceiling_is_usable_and_next_fitting_group_rolls_epoch() {
    let limits = EditingLimits {
        // One XmlText item plus the two inserted text clocks.
        max_undo_retained_units: 3,
        ..EditingLimits::default()
    };

    let mut exact = Harness::with_limits(limits.clone());
    exact
        .insert("ab", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    undo_commit(&mut exact);
    assert_eq!(text(&exact.engine), "");

    let mut rollover = Harness::with_limits(limits);
    rollover
        .insert("ab", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    rollover
        .insert("c", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    undo_commit(&mut rollover);
    assert_eq!(text(&rollover.engine), "ab");
    assert_eq!(rollover.undo().unwrap(), None);
}

#[test]
fn individually_oversized_recorded_groups_reject_atomically_but_skip_succeeds() {
    for policy in [HistoryPolicy::Auto, HistoryPolicy::Boundary] {
        let limits = EditingLimits {
            max_undo_retained_units: 3,
            ..EditingLimits::default()
        };
        let mut harness = Harness::with_limits(limits);
        let before = harness.audit();
        let error = harness
            .insert("abc", TransactionOrigin::LocalInput, policy)
            .unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED", "{policy:?}");
        assert_eq!(error.limit, Some(3), "{policy:?}");
        assert_eq!(error.actual, Some(4), "{policy:?}");
        assert_eq!(harness.audit(), before, "{policy:?}");
    }

    let limits = EditingLimits {
        max_undo_retained_units: 3,
        ..EditingLimits::default()
    };
    let mut skipped = Harness::with_limits(limits);
    skipped
        .insert("abc", TransactionOrigin::LocalInput, HistoryPolicy::Skip)
        .unwrap();
    assert_eq!(text(&skipped.engine), "abc");
    assert!(!skipped.engine.can_undo());
}

#[test]
fn bounded_epoch_keeps_undo_redo_scans_within_configured_group_and_work_limits() {
    let limits = EditingLimits {
        max_undo_groups: 1,
        // One XmlText item plus the inserted text clock.
        max_undo_retained_units: 2,
        ..EditingLimits::default()
    };
    let mut harness = Harness::with_limits(limits);
    for value in ["a", "b", "c", "d"] {
        harness
            .insert(
                value,
                TransactionOrigin::LocalInput,
                HistoryPolicy::Boundary,
            )
            .unwrap();
    }
    assert_eq!(text(&harness.engine), "abcd");
    undo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "abc");
    assert_eq!(harness.undo().unwrap(), None);
    redo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "abcd");
    assert_eq!(harness.redo().unwrap(), None);
}

#[test]
fn accepted_undo_and_redo_install_exact_prevalidated_semantic_state_once() {
    let mut harness = Harness::new();
    harness.import_json(PLAIN_AB);
    harness
        .apply(
            TransactionOrigin::LocalCommand,
            HistoryPolicy::Boundary,
            vec![TypedOperation::ReplaceRange {
                range: range(0, 2),
                content: Fragment::from(vec![Node::text("xyz".into(), vec![mark("bold")])]),
            }],
            SelectionIntent::UseOperationResult,
        )
        .unwrap();
    let accepted_after = harness.audit();

    let before_undo_document_revision = harness.engine.revision();
    let before_undo_state_revision = harness.engine.state_revision();
    let undo = undo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "ab");
    assert_eq!(undo.document_revision, before_undo_document_revision + 1);
    assert_eq!(undo.state_revision, before_undo_state_revision + 1);
    assert_eq!(undo.origin, TransactionOrigin::UndoRedo);

    let before_redo_document_revision = harness.engine.revision();
    let before_redo_state_revision = harness.engine.state_revision();
    let redo = redo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "xyz");
    assert_eq!(redo.document_revision, before_redo_document_revision + 1);
    assert_eq!(redo.state_revision, before_redo_state_revision + 1);
    assert_eq!(redo.origin, TransactionOrigin::UndoRedo);
    assert_eq!(harness.engine.document_json(), accepted_after.document_json);
    assert_eq!(
        harness.engine.resolved_selection().cloned(),
        accepted_after.selection
    );
    assert_eq!(
        harness.engine.stored_marks().map(<[Mark]>::to_vec),
        accepted_after.stored_marks
    );
}

#[test]
fn repeated_undo_and_redo_replay_the_same_deterministic_epoch() {
    let mut harness = Harness::new();
    for value in ["a", "b", "c"] {
        harness
            .insert(
                value,
                TransactionOrigin::LocalInput,
                HistoryPolicy::Boundary,
            )
            .unwrap();
    }

    for expected in ["ab", "a", ""] {
        undo_commit(&mut harness);
        assert_eq!(text(&harness.engine), expected);
    }
    assert_eq!(harness.undo().unwrap(), None);

    for expected in ["a", "ab", "abc"] {
        redo_commit(&mut harness);
        assert_eq!(text(&harness.engine), expected);
    }
    assert_eq!(harness.redo().unwrap(), None);
}

#[test]
fn skipped_durable_edits_replay_between_recorded_groups_without_becoming_undoable() {
    let mut harness = Harness::new();
    harness.import_json(PLAIN_AB);
    harness
        .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    harness
        .insert("x", TransactionOrigin::LocalApi, HistoryPolicy::Skip)
        .unwrap();
    harness
        .insert("b", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();

    undo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "abax");
    undo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "abx");
    assert_eq!(harness.undo().unwrap(), None);

    redo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "abax");
    redo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "abaxb");
    assert_eq!(harness.redo().unwrap(), None);
}

#[test]
fn oversized_skip_mutation_commits_and_clears_the_history_epoch() {
    let limits = EditingLimits {
        max_undo_retained_units: 2,
        ..EditingLimits::default()
    };
    let mut harness = Harness::with_limits(limits);
    harness
        .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    assert!(harness.engine.can_undo());

    harness
        .insert("xyz", TransactionOrigin::LocalApi, HistoryPolicy::Skip)
        .unwrap();
    assert_eq!(text(&harness.engine), "axyz");
    assert!(!harness.engine.can_undo());
    assert!(!harness.engine.can_redo());
}

#[test]
fn fitting_skip_work_contributes_to_aggregate_epoch_rollover() {
    let limits = EditingLimits {
        max_undo_retained_units: 4,
        ..EditingLimits::default()
    };
    let mut harness = Harness::with_limits(limits);
    harness
        .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Boundary)
        .unwrap();
    harness
        .insert("x", TransactionOrigin::LocalApi, HistoryPolicy::Skip)
        .unwrap();
    harness
        .insert("y", TransactionOrigin::LocalApi, HistoryPolicy::Skip)
        .unwrap();
    assert!(harness.engine.can_undo());

    harness
        .insert("z", TransactionOrigin::LocalApi, HistoryPolicy::Skip)
        .unwrap();
    assert_eq!(text(&harness.engine), "axyz");
    assert!(!harness.engine.can_undo());
}

#[test]
fn physical_history_metadata_accepts_exact_boundary_and_rejects_one_over_atomically() {
    // The manager and replay journal share one metadata object, which owns one
    // before and one after snapshot.
    let exact = PLAIN_HISTORY_SNAPSHOT_BYTES * 2;
    for (limit, accepted) in [(exact, true), (exact - 1, false)] {
        let limits = EditingLimits {
            max_derived_output_bytes: limit,
            ..EditingLimits::default()
        };
        let mut harness = Harness::with_limits(limits);
        let before = harness.audit();
        let result = harness.insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Boundary);
        if accepted {
            result.unwrap();
            assert!(harness.engine.can_undo());
            undo_commit(&mut harness);
            assert_eq!(text(&harness.engine), "");
            redo_commit(&mut harness);
            assert_eq!(text(&harness.engine), "a");
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
            assert_eq!(error.limit, Some((exact - 1) as u64));
            assert_eq!(error.actual, Some(exact as u64));
            assert_eq!(harness.audit(), before);
        }
    }
}

#[test]
fn compatible_edit_pending_metadata_is_bounded_and_rolls_before_live_mutation() {
    // Existing shared manager/journal metadata (before+after) plus only the
    // replacement after snapshot is the compatible capture's pending peak.
    let exact_pending = PLAIN_HISTORY_SNAPSHOT_BYTES * 3;
    for (limit, expected_after_undo) in [(exact_pending, ""), (exact_pending - 1, "a")] {
        let limits = EditingLimits {
            max_derived_output_bytes: limit,
            ..EditingLimits::default()
        };
        let mut harness = Harness::with_limits(limits);
        harness
            .insert("a", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
            .unwrap();
        harness.advance(1);
        harness
            .insert("b", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
            .unwrap();
        undo_commit(&mut harness);
        assert_eq!(text(&harness.engine), expected_after_undo, "limit={limit}");
    }

    let limits = EditingLimits {
        max_derived_output_bytes: exact_pending,
        ..EditingLimits::default()
    };
    let mut repeated = Harness::with_limits(limits);
    for value in ["a", "b", "c"] {
        repeated
            .insert(value, TransactionOrigin::LocalInput, HistoryPolicy::Auto)
            .unwrap();
        repeated.advance(1);
    }
    undo_commit(&mut repeated);
    assert_eq!(text(&repeated.engine), "ab");
}

#[test]
fn initial_document_contract_used_by_history_fixtures_is_stable() {
    let harness = Harness::new();
    assert_eq!(
        harness.engine.document_json(),
        serde_json::from_str(EMPTY).ok()
    );
}

const REPLACEMENT_DOC: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"replaced"}]}]}"#;

#[test]
fn undoable_boundary_replacement_never_merges_with_adjacent_auto_typing() {
    use crate::yrs_engine::ReplacementHistory;

    let mut harness = Harness::new();
    harness.import_json(PLAIN_AB);
    harness
        .insert("x", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    assert_eq!(text(&harness.engine), "abx");
    let pre_replacement = harness.engine.document_json();

    // The replacement lands inside the Auto capture window and must still be
    // its own boundary group on both sides.
    let request_id = harness.take_request_id();
    let commit = harness
        .engine
        .prepare_root_replacement_json(
            request_id,
            REPLACEMENT_DOC,
            ReplacementHistory::UndoableBoundary,
        )
        .unwrap();
    assert!(commit.changed);
    assert_eq!(commit.request_id, request_id);
    assert_eq!(commit.origin, TransactionOrigin::LocalApi);
    assert_eq!(text(&harness.engine), "replaced");

    // Control probe: two Auto inserts inside the same window DO merge with
    // each other, proving the window was mergeable — yet neither merges into
    // the replacement group.
    harness
        .insert("y", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    harness
        .insert("z", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    assert_eq!(text(&harness.engine), "replacedyz");

    // Exactly three groups: [typing x] [replacement] [typing yz].
    undo_commit(&mut harness);
    assert_eq!(
        text(&harness.engine),
        "replaced",
        "trailing Auto typing must not merge into the replacement group"
    );
    undo_commit(&mut harness);
    assert_eq!(
        harness.engine.document_json(),
        pre_replacement,
        "one undo restores the exact pre-replacement document"
    );
    assert!(
        harness.engine.can_undo(),
        "the leading typing group survives as its own undo item"
    );

    redo_commit(&mut harness);
    assert_eq!(
        text(&harness.engine),
        "replaced",
        "redo restores the replacement"
    );
    redo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "replacedyz");
    assert!(!harness.engine.can_redo());
}

#[test]
fn reset_and_clear_replacement_clears_history_in_the_same_store() {
    use crate::yrs_engine::ReplacementHistory;

    let mut harness = Harness::new();
    harness.import_json(PLAIN_AB);
    harness
        .insert("x", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    assert!(harness.engine.can_undo());
    undo_commit(&mut harness);
    redo_commit(&mut harness);
    assert!(harness.engine.can_undo());

    let client = harness.engine.client_id();
    let revision = harness.engine.revision();
    let request_id = harness.take_request_id();
    let commit = harness
        .engine
        .prepare_root_replacement_json(
            request_id,
            REPLACEMENT_DOC,
            ReplacementHistory::ResetAndClear,
        )
        .unwrap();
    assert!(commit.changed);
    assert_eq!(
        commit.document_revision,
        revision + 1,
        "same-store replacement continues the durable revision sequence"
    );
    assert_eq!(
        harness.engine.client_id(),
        client,
        "same-store replacement keeps the writing client identity"
    );

    assert!(!harness.engine.can_undo());
    assert!(!harness.engine.can_redo());
    assert!(harness.undo().unwrap().is_none());
    assert!(harness.redo().unwrap().is_none());
    assert_eq!(text(&harness.engine), "replaced");

    // Fresh history bottoms out at the replacement, never before it.
    harness
        .insert("q", TransactionOrigin::LocalInput, HistoryPolicy::Auto)
        .unwrap();
    assert!(harness.engine.can_undo());
    undo_commit(&mut harness);
    assert_eq!(text(&harness.engine), "replaced");
    assert!(!harness.engine.can_undo());
}
