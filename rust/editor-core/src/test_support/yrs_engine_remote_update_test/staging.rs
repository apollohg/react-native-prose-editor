/// Prepare/commit split, sealing, and read-only state-vector/diff
/// encoding. Everything below is staging-only surface; the default-feature
/// test count of this file must stay unchanged.
mod staging {
    use super::*;
    use crate::yrs_engine::{DocumentScope, OperationError};
    use yrs::updates::decoder::Decode;
    use yrs::updates::encoder::Encode;
    use yrs::{Doc, GetString, ReadTxn, StateVector, Transact, Update};

    fn scoped_engine(mode: InitializationMode) -> YrsDocumentEngine {
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: mode,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: Some(DocumentScope {
                document_id: "doc-remote".into(),
                lineage_id: "lineage-remote".into(),
            }),
        })
        .unwrap()
    }

    fn error_json(error: &OperationError) -> serde_json::Value {
        serde_json::to_value(error).unwrap()
    }

    /// Runs one update through both paths on twin engines and asserts full
    /// result and audit parity. Returns both engines for follow-up steps.
    fn assert_step_parity(
        one_shot: &mut YrsDocumentEngine,
        split: &mut YrsDocumentEngine,
        request_id: u64,
        update: &[u8],
    ) {
        let expected = one_shot.apply_remote_update_v1(request_id, update);
        let actual = split
            .prepare_remote_update_v1(request_id, update)
            .and_then(|prepared| split.commit_prepared_remote_update(prepared));
        match (&expected, &actual) {
            (Ok(expected_commit), Ok(actual_commit)) => {
                assert_eq!(expected_commit.changed, actual_commit.changed);
                assert_eq!(expected_commit.revision, actual_commit.revision);
            }
            (Err(expected_error), Err(actual_error)) => {
                assert_eq!(error_json(expected_error), error_json(actual_error));
            }
            (expected, actual) => {
                panic!("path divergence: one-shot {expected:?} vs prepare/commit {actual:?}");
            }
        }
        assert_eq!(audit(one_shot), audit(split));
        assert_eq!(one_shot.is_ready(), split.is_ready());
    }

    #[test]
    fn prepare_commit_matches_one_shot_and_preserves_rejected_dependency_state() {
        // Valid + duplicate no-op.
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(
                700,
                TypedCommand::InsertText {
                    text: "parity".into(),
                },
            )
            .unwrap();
        let valid = source.encoded_state().unwrap();
        let mut one_shot = engine(InitializationMode::AwaitRemote);
        let mut split = engine(InitializationMode::AwaitRemote);
        assert_step_parity(&mut one_shot, &mut split, 701, &valid);
        assert_step_parity(&mut one_shot, &mut split, 702, &valid);

        // Malformed bytes.
        for corrupt in [&[0xff][..], &[1, 1][..], &[0, 1, 0xff][..]] {
            assert_step_parity(&mut one_shot, &mut split, 703, corrupt);
        }

        // Over the encoded-state byte ceiling.
        let tight_resources = ResourceLimits {
            max_encoded_state_bytes: 64,
            ..ResourceLimits::default()
        };
        let mut tight_one_shot = engine_with(
            tiptap_schema(),
            InitializationMode::AwaitRemote,
            tight_resources.clone(),
            EditingLimits::default(),
            None,
        );
        let mut tight_split = engine_with(
            tiptap_schema(),
            InitializationMode::AwaitRemote,
            tight_resources,
            EditingLimits::default(),
            None,
        );
        assert_step_parity(&mut tight_one_shot, &mut tight_split, 704, &[0; 65]);

        // Dependency-pending quarantine, completion, and convergence.
        let (base, delta_a, delta_b, final_state) = dependent_text_updates();
        let mut one_shot = engine(InitializationMode::AwaitRemote);
        let mut split = engine(InitializationMode::AwaitRemote);
        assert_step_parity(&mut one_shot, &mut split, 705, &delta_b);
        assert!(!split.is_ready());
        assert_step_parity(&mut one_shot, &mut split, 706, &delta_a);
        assert_step_parity(&mut one_shot, &mut split, 707, &base);
        assert!(split.is_ready());
        assert_eq!(split.document().unwrap().root().text_content(), "ab");
        let mut expected = engine(InitializationMode::AwaitRemote);
        expected.apply_remote_update_v1(708, &final_state).unwrap();
        assert_eq!(
            split.encoded_state().unwrap(),
            expected.encoded_state().unwrap()
        );

        // A deferred over-ceiling failure preserves the live dependency
        // candidate on both paths. An unrelated update joins that candidate;
        // it cannot bypass the retained dependency state and publish itself.
        let (base, delta_a, delta_b, _) = dependent_text_updates();
        let deferred_engine = |mode| {
            engine_with(
                tiptap_schema(),
                mode,
                ResourceLimits::default(),
                EditingLimits::default(),
                Some(1),
            )
        };
        let mut one_shot = deferred_engine(InitializationMode::AwaitRemote);
        let mut split = deferred_engine(InitializationMode::AwaitRemote);
        assert_step_parity(&mut one_shot, &mut split, 709, &delta_b);
        assert_step_parity(&mut one_shot, &mut split, 710, &delta_a);
        let one_shot_before = audit(&one_shot);
        let split_before = audit(&split);
        let one_shot_dependencies_before = one_shot.pending_remote_dependency_bytes();
        let split_dependencies_before = split.pending_remote_dependency_bytes();
        assert_eq!(one_shot_dependencies_before, split_dependencies_before);
        assert!(one_shot_dependencies_before > 0);
        assert_step_parity(&mut one_shot, &mut split, 711, &base);
        assert_eq!(audit(&one_shot), one_shot_before);
        assert_eq!(audit(&split), split_before);
        assert_eq!(
            one_shot.pending_remote_dependency_bytes(),
            one_shot_dependencies_before,
        );
        assert_eq!(
            split.pending_remote_dependency_bytes(),
            split_dependencies_before,
        );

        let mut valid = engine(InitializationMode::LocalEmpty);
        valid
            .apply_command(712, TypedCommand::InsertText { text: "z".into() })
            .unwrap();
        let unrelated = valid.encoded_state().unwrap();
        let expected = one_shot.apply_remote_update_v1(713, &unrelated).unwrap();
        let prepared = split.prepare_remote_update_v1(713, &unrelated).unwrap();
        assert!(prepared.has_pending_dependencies());
        let expected_retained = prepared.retained_dependency_bytes();
        assert_eq!(
            split.pending_remote_dependency_bytes(),
            split_dependencies_before,
        );
        let actual = split.commit_prepared_remote_update(prepared).unwrap();

        assert!(!expected.changed);
        assert!(!actual.changed);
        assert_eq!(expected.revision, actual.revision);
        assert_eq!(audit(&one_shot), one_shot_before);
        assert_eq!(audit(&split), split_before);
        assert_eq!(audit(&one_shot), audit(&split));
        assert_eq!(
            one_shot.pending_remote_dependency_bytes(),
            expected_retained,
        );
        assert_eq!(split.pending_remote_dependency_bytes(), expected_retained,);
        assert!(!one_shot.is_ready());
        assert!(!split.is_ready());
        assert!(one_shot.document().is_none());
        assert!(split.document().is_none());
    }

    #[test]
    fn prepared_update_rejects_after_a_local_edit_between_prepare_and_commit() {
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(
                720,
                TypedCommand::InsertText {
                    text: "seed".into(),
                },
            )
            .unwrap();
        let seed = source.encoded_state().unwrap();
        let mut target = engine(InitializationMode::AwaitRemote);
        target.apply_remote_update_v1(721, &seed).unwrap();

        source
            .apply_command(722, TypedCommand::InsertText { text: "!".into() })
            .unwrap();
        let follow_up = source.encoded_state().unwrap();

        let prepared = target.prepare_remote_update_v1(723, &follow_up).unwrap();
        target
            .apply_command(
                724,
                TypedCommand::InsertText {
                    text: "local".into(),
                },
            )
            .unwrap()
            .expect("local edit applies");
        let before_commit = audit(&target);

        let error = target.commit_prepared_remote_update(prepared).unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 723);
        assert_eq!(audit(&target), before_commit);

        // A fresh prepare over the new state commits cleanly.
        let prepared = target.prepare_remote_update_v1(725, &follow_up).unwrap();
        assert!(
            target
                .commit_prepared_remote_update(prepared)
                .unwrap()
                .changed
        );
        assert!(target
            .document()
            .unwrap()
            .root()
            .text_content()
            .contains('!'));
    }

    #[test]
    fn prepared_update_rejects_after_a_second_remote_commit() {
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(730, TypedCommand::InsertText { text: "a".into() })
            .unwrap();
        let first = source.encoded_state().unwrap();
        source
            .apply_command(731, TypedCommand::InsertText { text: "b".into() })
            .unwrap();
        let second = source.encoded_state().unwrap();

        let mut target = engine(InitializationMode::AwaitRemote);
        let prepared = target.prepare_remote_update_v1(732, &first).unwrap();
        target.apply_remote_update_v1(733, &second).unwrap();
        let before_commit = audit(&target);

        let error = target.commit_prepared_remote_update(prepared).unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 732);
        assert_eq!(audit(&target), before_commit);
        assert_eq!(target.document().unwrap().root().text_content(), "ab");
    }

    #[test]
    fn prepared_update_rejects_after_a_snapshot_restore() {
        let mut snapshot_source = scoped_engine(InitializationMode::LocalEmpty);
        snapshot_source
            .import_json(
                &serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]})
                    .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let snapshot = snapshot_source.export_snapshot().unwrap();

        let mut update_source = engine(InitializationMode::LocalEmpty);
        update_source
            .apply_command(
                740,
                TypedCommand::InsertText {
                    text: "remote".into(),
                },
            )
            .unwrap();
        let update = update_source.encoded_state().unwrap();

        let mut target = scoped_engine(InitializationMode::AwaitRemote);
        let prepared = target.prepare_remote_update_v1(741, &update).unwrap();
        assert!(target.restore_snapshot(&snapshot).unwrap().changed);
        let before_commit = audit(&target);

        let error = target.commit_prepared_remote_update(prepared).unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 741);
        assert_eq!(audit(&target), before_commit);
        assert_eq!(target.document().unwrap().root().text_content(), "snapshot");
    }

    #[test]
    fn prepared_remote_commits_stay_outside_local_undo_history() {
        // Without any local history, a prepared remote commit creates none.
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(
                749,
                TypedCommand::InsertText {
                    text: "seed".into(),
                },
            )
            .unwrap();
        let mut fresh = engine(InitializationMode::AwaitRemote);
        let prepared = fresh
            .prepare_remote_update_v1(750, &source.encoded_state().unwrap())
            .unwrap();
        assert!(
            fresh
                .commit_prepared_remote_update(prepared)
                .unwrap()
                .changed
        );
        assert!(!fresh.can_undo());
        assert!(!fresh.can_redo());
        assert!(fresh.undo(751).unwrap().is_none());

        // With local history: twin engines run the same sequence through the
        // one-shot and the prepared path; the remote commit must not add an
        // undo group, must not mark local authorship, and undo/redo must stay
        // byte-identical across both paths.
        let build_target = || {
            let mut target = engine(InitializationMode::LocalEmpty);
            target
                .apply_command(
                    752,
                    TypedCommand::InsertText {
                        text: "local".into(),
                    },
                )
                .unwrap()
                .expect("local typing applies");
            assert!(target.can_undo());
            target
        };
        let mut one_shot = build_target();
        let mut split = build_target();

        // A genuine causal follow-up built on a peer that admitted our state.
        let remote_update_for = |target: &YrsDocumentEngine| {
            let mut peer = engine(InitializationMode::AwaitRemote);
            peer.apply_remote_update_v1(753, &target.encoded_state().unwrap())
                .unwrap();
            peer.apply_command(754, TypedCommand::InsertText { text: "R".into() })
                .unwrap()
                .expect("peer typing applies");
            peer.encoded_state().unwrap()
        };

        for (target, prepared_path) in [(&mut one_shot, false), (&mut split, true)] {
            let remote_update = remote_update_for(target);
            let local_clock_before = local_authored_clock(target);
            let commit = if prepared_path {
                let prepared = target
                    .prepare_remote_update_v1(755, &remote_update)
                    .unwrap();
                target.commit_prepared_remote_update(prepared).unwrap()
            } else {
                target.apply_remote_update_v1(755, &remote_update).unwrap()
            };
            assert!(commit.changed);
            assert_eq!(
                target.last_committed_origin(),
                Some(TransactionOrigin::RemoteSync)
            );
            assert!(target
                .document()
                .unwrap()
                .root()
                .text_content()
                .contains('R'));
            // No local-origin authorship: the local client's authored clock
            // is untouched by the remote commit.
            assert_eq!(local_authored_clock(target), local_clock_before);
            // The remote commit added no undo group: exactly the one local
            // group remains poppable.
            assert!(target.can_undo());
            target.undo(756).unwrap().expect("local undo applies");
            assert!(!target.can_undo(), "only the local group was poppable");
            assert!(target.can_redo());
            target.redo(757).unwrap().expect("redo applies");
            assert!(target
                .document()
                .unwrap()
                .root()
                .text_content()
                .contains('R'));
        }
        // Whatever the undo timeline semantics, the prepared path must not
        // drift from the one-shot path. The raw encoded states differ only by
        // the two engines' distinct client identities, so compare everything
        // derived instead.
        let mut one_shot_audit = audit(&one_shot);
        let mut split_audit = audit(&split);
        one_shot_audit.encoded.clear();
        split_audit.encoded.clear();
        assert_eq!(one_shot_audit, split_audit);
    }

    /// The local client's authored clock in the engine's state vector (0 when
    /// the client has authored nothing durable).
    fn local_authored_clock(engine: &YrsDocumentEngine) -> u32 {
        let encoded = engine.encoded_state().unwrap();
        if encoded.is_empty() {
            return 0;
        }
        let sv =
            StateVector::decode_v1(&encode_state_vector_from_update_v1(&encoded).unwrap()).unwrap();
        sv.iter()
            .find(|(client, _)| client.get() == engine.client_id())
            .map(|(_, clock)| *clock)
            .unwrap_or(0)
    }

    #[test]
    fn state_vector_and_diff_encoding_are_read_only_and_standard() {
        let mut engine = engine(InitializationMode::LocalEmpty);
        engine
            .import_json(
                &serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"base"}]}]})
                    .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let base_state = engine.encoded_state().unwrap();
        engine
            .apply_command(
                760,
                TypedCommand::InsertText {
                    text: " grows".into(),
                },
            )
            .unwrap();
        let baseline = audit(&engine);

        // The encoded state vector equals the raw store's state vector.
        let encoded_sv = engine.encode_state_vector_v1(761).unwrap();
        assert_eq!(
            StateVector::decode_v1(&encoded_sv).unwrap(),
            StateVector::decode_v1(
                &encode_state_vector_from_update_v1(&engine.encoded_state().unwrap()).unwrap()
            )
            .unwrap()
        );

        // An independent raw yrs replica holding only the base state
        // reconstructs the exact document from the encoded diff.
        let replica = Doc::new();
        replica
            .transact_mut()
            .apply_update(Update::decode_v1(&base_state).unwrap())
            .unwrap();
        let replica_sv = replica.transact().state_vector().encode_v1();
        let diff = engine.encode_diff_v1(762, &replica_sv).unwrap();
        replica
            .transact_mut()
            .apply_update(Update::decode_v1(&diff).unwrap())
            .unwrap();
        assert_eq!(
            replica.transact().state_vector(),
            StateVector::decode_v1(&encoded_sv).unwrap()
        );
        let replica_text = {
            let txn = replica.transact();
            txn.get_xml_fragment("prosemirror")
                .unwrap()
                .get_string(&txn)
        };
        let engine_text = {
            let engine_state = engine.encoded_state().unwrap();
            let check = Doc::new();
            check
                .transact_mut()
                .apply_update(Update::decode_v1(&engine_state).unwrap())
                .unwrap();
            let txn = check.transact();
            txn.get_xml_fragment("prosemirror")
                .unwrap()
                .get_string(&txn)
        };
        assert_eq!(replica_text, engine_text);

        // An empty state vector yields the full state; an up-to-date state
        // vector yields a dependency-free no-op diff.
        let full = engine
            .encode_diff_v1(763, &StateVector::default().encode_v1())
            .unwrap();
        let fresh = Doc::new();
        fresh
            .transact_mut()
            .apply_update(Update::decode_v1(&full).unwrap())
            .unwrap();
        assert_eq!(
            fresh.transact().state_vector(),
            StateVector::decode_v1(&encoded_sv).unwrap()
        );
        let noop_diff = engine.encode_diff_v1(764, &encoded_sv).unwrap();
        assert!(Update::decode_v1(&noop_diff).is_ok());

        // Every encoding call above was read-only.
        assert_eq!(audit(&engine), baseline);
    }

    #[test]
    fn malformed_or_oversized_state_vector_input_rejects_with_structured_errors() {
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(770, TypedCommand::InsertText { text: "sv".into() })
            .unwrap();
        let baseline = audit(&source);

        for corrupt in [&[0xff, 0xff, 0xff][..], &[0x01][..]] {
            let error = source.encode_diff_v1(771, corrupt).unwrap_err();
            assert_eq!(error.code, "DOCUMENT_INVALID");
            assert_eq!(error.request_id, 771);
            assert_eq!(error.details.as_ref().unwrap()["field"], "stateVector");
            assert_eq!(audit(&source), baseline);
        }

        let tight = engine_with(
            tiptap_schema(),
            InitializationMode::AwaitRemote,
            ResourceLimits {
                max_encoded_state_bytes: 64,
                ..ResourceLimits::default()
            },
            EditingLimits::default(),
            None,
        );
        let error = tight.encode_diff_v1(772, &[0; 65]).unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.request_id, 772);
        assert_eq!(
            error.details.as_ref().unwrap()["field"],
            "maxEncodedStateBytes"
        );
        assert_eq!(error.limit, Some(64));
        assert_eq!(error.actual, Some(65));
    }

    /// Fix round 1: an *unchanged* snapshot restore still clears the
    /// dependency quarantine and rebinds the bounded history without touching
    /// revision/state/epoch or the store handle — it must nonetheless
    /// invalidate an outstanding prepared remote update (brief §2 lists
    /// snapshot restore as a rejecting intervening mutation, and an
    /// unsealed commit could panic mid-install on the rebound replay chain).
    #[test]
    fn prepared_update_rejects_after_an_unchanged_snapshot_restore() {
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(
                780,
                TypedCommand::InsertText {
                    text: "seed".into(),
                },
            )
            .unwrap();
        let mut target = scoped_engine(InitializationMode::AwaitRemote);
        // A prior remote commit so the bounded replay chain already holds an
        // event (the reviewer's panic repro shape).
        target
            .apply_remote_update_v1(781, &source.encoded_state().unwrap())
            .unwrap();
        let snapshot = target.export_snapshot().unwrap();

        source
            .apply_command(782, TypedCommand::InsertText { text: "!".into() })
            .unwrap();
        let follow_up = source.encoded_state().unwrap();
        let prepared = target.prepare_remote_update_v1(783, &follow_up).unwrap();

        // Same-state restore takes the unchanged fast path.
        let restore = target.restore_snapshot(&snapshot).unwrap();
        assert!(!restore.changed);
        let before_commit = audit(&target);

        let error = target.commit_prepared_remote_update(prepared).unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 783);
        assert_eq!(audit(&target), before_commit);

        // A fresh prepare over the post-restore state commits cleanly.
        let prepared = target.prepare_remote_update_v1(784, &follow_up).unwrap();
        assert!(
            target
                .commit_prepared_remote_update(prepared)
                .unwrap()
                .changed
        );
        assert!(target
            .document()
            .unwrap()
            .root()
            .text_content()
            .contains('!'));
    }

    /// Fix round 1: the canonical-equal (no-op) import variant of the same
    /// hole — quarantine clear plus history rebind with no revision change.
    #[test]
    fn prepared_update_rejects_after_a_no_op_import() {
        let mut source = engine(InitializationMode::LocalEmpty);
        source
            .apply_command(
                790,
                TypedCommand::InsertText {
                    text: "seed".into(),
                },
            )
            .unwrap();
        let mut target = engine(InitializationMode::AwaitRemote);
        target
            .apply_remote_update_v1(791, &source.encoded_state().unwrap())
            .unwrap();
        let current_json = target.document_json().unwrap().to_string();

        source
            .apply_command(792, TypedCommand::InsertText { text: "!".into() })
            .unwrap();
        let follow_up = source.encoded_state().unwrap();
        let prepared = target.prepare_remote_update_v1(793, &follow_up).unwrap();

        // Importing the canonical-equal document takes the unchanged path.
        let import = target
            .import_json(&current_json, TransactionOrigin::DocumentImport)
            .unwrap();
        assert!(!import.changed);
        let before_commit = audit(&target);

        let error = target.commit_prepared_remote_update(prepared).unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 793);
        assert_eq!(audit(&target), before_commit);

        let prepared = target.prepare_remote_update_v1(794, &follow_up).unwrap();
        assert!(
            target
                .commit_prepared_remote_update(prepared)
                .unwrap()
                .changed
        );
        assert!(target
            .document()
            .unwrap()
            .root()
            .text_content()
            .contains('!'));
    }

    /// Task 7 no-echo extension: remote updates admitted through both the
    /// one-shot and the sealed prepare/commit paths never produce a
    /// collaboration outbox entry on an attached session, while an immediate
    /// local edit on the same session enqueues exactly one bounded entry.
    #[test]
    fn remote_updates_produce_no_outbox_entries_on_attached_sessions() {
        use crate::native_bridge_test_support::{self as bridge, SessionOptions};

        let mut source = scoped_engine(InitializationMode::LocalEmpty);
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();

        let id = bridge::create_session(SessionOptions {
            attach_runtime: true,
            ..SessionOptions::default()
        })
        .unwrap();

        // One-shot remote apply: no echo.
        let changed =
            bridge::apply_remote_update(id, 901, &source.encoded_state().unwrap()).unwrap();
        assert!(changed);
        assert_eq!(bridge::outbox_pending(id).unwrap(), Some((0, 0)));
        assert_eq!(bridge::last_reserved_upper_bound(id).unwrap(), None);

        // Sealed prepare/commit remote apply: no echo.
        source
            .apply_command(902, TypedCommand::InsertText { text: "!".into() })
            .unwrap();
        let follow_up = source.encoded_state().unwrap();
        let changed = bridge::apply_prepared_remote_update(id, 903, &follow_up).unwrap();
        assert!(changed);
        assert_eq!(bridge::outbox_pending(id).unwrap(), Some((0, 0)));

        // The same session still emits exactly one bounded entry for a local
        // trusted-origin edit.
        let base = bridge::session_audit(id).unwrap().document_revision;
        bridge::submit_input(
            id,
            &serde_json::json!({
                "version": 1,
                "requestId": "904",
                "baseDocumentRevision": base.to_string(),
                "text": "local",
            })
            .to_string(),
        )
        .unwrap();
        let (count, bytes) = bridge::outbox_pending(id).unwrap().unwrap();
        assert_eq!(count, 1);
        assert!(bytes > 0);
        let bound = bridge::last_reserved_upper_bound(id).unwrap().unwrap();
        assert!(bytes <= bound);

        bridge::destroy_session(id);
    }
}
