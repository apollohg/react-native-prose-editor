#[cfg(test)]
mod prepared_batch_tests {
    use serde_json::json;
    use yrs::types::xml::{XmlElementPrelim, XmlFragment};
    use yrs::{Doc, OffsetKind, Options, Transact, WriteTxn};

    use super::*;
    use crate::schema::presets::tiptap_schema;
    use crate::serialize::{from_prosemirror_json, UnknownTypeMode};
    use crate::transform::{apply_step_canonical_marks, Step};
    use crate::yrs_engine::codec::YrsDocumentCodec;
    use crate::yrs_engine::mutation::execute_mutation_plan;

    fn utf16_doc() -> Doc {
        Doc::with_options(Options {
            offset_kind: OffsetKind::Utf16,
            ..Options::default()
        })
    }

    #[test]
    fn action_slot_stays_pointer_sized_for_tombstone_heavy_plans() {
        assert!(std::mem::size_of::<ActionSlot>() <= 32);
    }

    #[test]
    fn reserved_wire_elements_remain_semantic_void_under_hostile_schema_declarations() {
        let schema = Schema::from_json(&json!({
            "nodes": [
                { "name": "doc", "content": "block+", "role": "doc" },
                {
                    "name": "__opaque",
                    "content": "inline*",
                    "group": "block",
                    "role": "block"
                },
                {
                    "name": "__opaque_json",
                    "content": "inline*",
                    "group": "block",
                    "role": "block"
                },
                {
                    "name": "__skip",
                    "content": "inline*",
                    "group": "block",
                    "role": "block"
                },
                { "name": "text", "content": "", "group": "inline", "role": "text" }
            ],
            "marks": []
        }))
        .unwrap();

        for reserved in ["__opaque", "__opaque_json", "__skip"] {
            let doc = utf16_doc();
            let element = {
                let mut txn = doc.transact_mut();
                let fragment = txn.get_or_insert_xml_fragment("prosemirror");
                let element = fragment.push_back(&mut txn, XmlElementPrelim::empty(reserved));
                element.push_back(&mut txn, yrs::types::xml::XmlTextPrelim::new("hostile"));
                element
            };
            let txn = doc.transact();
            assert!(wire_element_is_semantic_void(&element, &txn, &schema));
        }
    }

    fn seed_two_prepared_root_blocks(
        compiler: &mut MutationCompiler,
        before: &Document,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> OperationResult<()> {
        let root = compiler
            .structural_parents
            .get(&Vec::new())
            .cloned()
            .ok_or_else(|| invalid_action_range(compiler.request_id, 0))?;
        let content = before
            .root()
            .content()
            .ok_or_else(|| invalid_action_range(compiler.request_id, 0))?;
        let json = content
            .iter()
            .take(2)
            .map(|node| crate::serialize::node_to_prosemirror_json(node, schema))
            .collect::<Vec<_>>();
        let mut batch = prepare_xml_nodes(&json, limits, 2)
            .map_err(|error| map_prepared_node_error(compiler.request_id, 0, error))?;
        for (index, child) in batch.nodes.iter_mut().enumerate() {
            child.index =
                u32::try_from(index).map_err(|_| invalid_action_range(compiler.request_id, 0))?;
        }
        let insert_id = compiler.queue_prepared_insert(PendingPreparedInsert {
            parent: root.parent,
            child_index: 0,
            nodes: batch.nodes,
            signature: root.signature,
            operation_index: 0,
            semantic_parent_path: Vec::new(),
            first_semantic_index: 0,
        });

        let mut remapped = HashMap::with_capacity(compiler.structural_parents.len());
        for (mut path, parent) in std::mem::take(&mut compiler.structural_parents) {
            if !path.is_empty() {
                path[0] = path[0]
                    .checked_add(2)
                    .ok_or_else(|| invalid_action_range(compiler.request_id, 0))?;
            } else {
                let mut parent = parent;
                parent.storage_children.splice(
                    0..0,
                    [
                        StorageChildKind::PreparedElement,
                        StorageChildKind::PreparedElement,
                    ],
                );
                remapped.insert(path, parent);
                continue;
            }
            remapped.insert(path, parent);
        }
        compiler.structural_parents = remapped;

        let nodes = compiler.prepared_inserts[insert_id]
            .as_ref()
            .ok_or_else(|| invalid_action_range(compiler.request_id, 0))?
            .nodes
            .clone();
        let mut elements = Vec::new();
        let mut texts = Vec::new();
        collect_prepared_child_handles(insert_id, &nodes, &[], 0, None, &mut elements, &mut texts)?;
        compiler.prepared_elements.extend(elements);
        let old_tail = compiler
            .targets
            .first()
            .ok_or_else(|| invalid_action_range(compiler.request_id, 0))?;
        let old_tail_kind = match &old_tail.kind {
            ResolvedTargetKind::Existing { target, signature } => ResolvedTargetKind::Existing {
                target: target.clone(),
                signature: signature.clone(),
            },
            _ => return Err(invalid_action_range(compiler.request_id, 0)),
        };
        let old_tail_snapshot = (
            old_tail.text.clone(),
            old_tail.scalar_len,
            old_tail.base_runs.clone(),
            old_tail.current_runs.clone(),
        );
        compiler.targets.clear();
        let mut current_end = 0u32;
        for (path, handle, runs) in texts {
            let start = first_text_doc_position(before.root(), &path)
                .ok_or_else(|| invalid_action_range(compiler.request_id, 0))?;
            let text = prepared_runs_text(&runs);
            let scalar_len = u32::try_from(text.chars().count())
                .map_err(|_| invalid_action_range(compiler.request_id, 0))?;
            compiler.targets.push(ResolvedText {
                kind: ResolvedTargetKind::Prepared { handle },
                gap_before: start
                    .checked_sub(current_end)
                    .ok_or_else(|| invalid_action_range(compiler.request_id, 0))?,
                text,
                scalar_len,
                base_runs: Vec::new(),
                current_runs: runs,
                action_slots: Vec::new(),
            });
            current_end = start
                .checked_add(scalar_len)
                .ok_or_else(|| invalid_action_range(compiler.request_id, 0))?;
        }
        let tail_start = first_text_doc_position(before.root(), &[2, 0])
            .ok_or_else(|| invalid_action_range(compiler.request_id, 0))?;
        compiler.targets.push(ResolvedText {
            kind: old_tail_kind,
            gap_before: tail_start
                .checked_sub(current_end)
                .ok_or_else(|| invalid_action_range(compiler.request_id, 0))?,
            text: old_tail_snapshot.0,
            scalar_len: old_tail_snapshot.1,
            base_runs: old_tail_snapshot.2,
            current_runs: old_tail_snapshot.3,
            action_slots: Vec::new(),
        });
        Ok(())
    }

    #[test]
    fn wrapping_either_root_of_one_prepared_batch_preserves_the_other_root() {
        for selected in 0..2usize {
            let schema = tiptap_schema();
            let limits = ResourceLimits::default();
            let source = json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "tail" }]
                }]
            });
            let before_json = json!({
                "type": "doc",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "first" }] },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "second" }] },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
                ]
            });
            let before =
                from_prosemirror_json(&before_json, &schema, UnknownTypeMode::Preserve).unwrap();
            let doc = utf16_doc();
            let codec = YrsDocumentCodec::new(&schema, &limits);
            {
                let mut txn = doc.transact_mut();
                let fragment = txn.get_or_insert_xml_fragment("prosemirror");
                codec
                    .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), &source)
                    .unwrap();
            }
            let mut compiler = {
                let txn = doc.transact();
                let fragment = txn.get_xml_fragment("prosemirror").unwrap();
                MutationCompiler::new(
                    991,
                    &txn,
                    &fragment,
                    &schema,
                    usize::MAX / 4,
                    usize::MAX / 4,
                    0,
                )
                .unwrap()
            };
            seed_two_prepared_root_blocks(&mut compiler, &before, &schema, &limits).unwrap();
            let content = before.root().content().unwrap();
            let from = content
                .iter()
                .take(selected)
                .fold(0u32, |total, node| total + node.node_size());
            let to = from + content.child(selected).unwrap().node_size();
            let (after, _) = apply_step_canonical_marks(
                &before,
                &Step::WrapInList {
                    from,
                    to,
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                    attrs: HashMap::new(),
                    item_attrs: HashMap::new(),
                },
                &schema,
            )
            .unwrap();
            compiler
                .wrap_in_list(1, &before, &after, from, to, &schema, &limits)
                .unwrap();
            let plan = compiler.finish(Some(1)).unwrap();
            assert_eq!(plan.actions.len(), 1);
            {
                let mut txn = doc.transact_mut();
                execute_mutation_plan(plan, &mut txn);
            }
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let actual = codec.read_json(&fragment, &txn).unwrap();
            assert_eq!(
                actual,
                crate::serialize::to_prosemirror_json(&after, &schema)
            );
            let sibling = if selected == 0 { 1 } else { 0 };
            assert_eq!(
                actual["content"][sibling]["content"][0]["text"],
                if selected == 0 { "second" } else { "first" }
            );
        }
    }
}

#[cfg(test)]
mod localized_insert_tests {
    use serde_json::{json, Value};
    use yrs::{Doc, OffsetKind, Options, Transact, WriteTxn};

    use super::*;
    use crate::position::PositionMap;
    use crate::schema::presets::tiptap_schema;
    use crate::serialize::{from_prosemirror_json, UnknownTypeMode};
    use crate::transform::{apply_step_canonical_marks, Step};
    use crate::yrs_engine::codec::YrsDocumentCodec;
    use crate::yrs_engine::mutation::{execute_mutation_plan, preflight_mutation_plan};

    fn utf16_doc() -> Doc {
        Doc::with_options(Options {
            offset_kind: OffsetKind::Utf16,
            ..Options::default()
        })
    }

    fn seeded_document(source: &Value, schema: &Schema, limits: &ResourceLimits) -> Doc {
        let doc = utf16_doc();
        let codec = YrsDocumentCodec::new(schema, limits);
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(
                &fragment,
                &mut txn,
                &json!({ "type": schema.doc_node_type() }),
                source,
            )
            .unwrap();
        drop(txn);
        doc
    }

    #[derive(Debug, PartialEq)]
    struct InsertActionView<'a> {
        signature: &'a TargetSignature,
        index_utf16: u32,
        text: &'a str,
        len_utf16: u32,
        attrs: &'a Attrs,
        operation_index: usize,
    }

    #[derive(Debug, PartialEq)]
    struct FormatActionView<'a> {
        signature: &'a TargetSignature,
        index_utf16: u32,
        len_utf16: u32,
        attrs: &'a Attrs,
        operation_index: usize,
    }

    #[derive(Debug, PartialEq)]
    enum TextActionView<'a> {
        Insert(InsertActionView<'a>),
        Format(FormatActionView<'a>),
    }

    fn action_signature(action: &YrsMutationAction) -> InsertActionView<'_> {
        let YrsMutationAction::InsertText {
            index_utf16,
            text,
            len_utf16,
            attrs,
            signature,
            operation_index,
            ..
        } = action
        else {
            panic!("expected InsertText action")
        };
        InsertActionView {
            signature,
            index_utf16: *index_utf16,
            text,
            len_utf16: *len_utf16,
            attrs,
            operation_index: *operation_index,
        }
    }

    fn text_action_view(action: &YrsMutationAction) -> TextActionView<'_> {
        match action {
            YrsMutationAction::InsertText { .. } => TextActionView::Insert(action_signature(action)),
            YrsMutationAction::FormatText {
                index_utf16,
                len_utf16,
                attrs,
                signature,
                operation_index,
                ..
            } => TextActionView::Format(FormatActionView {
                signature,
                index_utf16: *index_utf16,
                len_utf16: *len_utf16,
                attrs,
                operation_index: *operation_index,
            }),
            _ => panic!("expected InsertText or FormatText action"),
        }
    }

    fn compile_pair_at_block_offset(
        source: &Value,
        schema: &Schema,
        block_index: usize,
        block_offset: u32,
        inserted: &str,
    ) -> (Doc, YrsMutationPlan, YrsMutationPlan, MutationCompilerBuild) {
        let limits = ResourceLimits::default();
        let document = from_prosemirror_json(source, schema, UnknownTypeMode::Preserve).unwrap();
        let position_map = PositionMap::build(&document, schema);
        let block = position_map.block(block_index).unwrap();
        let position = block.doc_start + block_offset;
        let doc = seeded_document(source, schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let mut eager =
            MutationCompiler::new(702, &txn, &fragment, schema, 100_000, 100_000, 11).unwrap();
        let (mut localized, mode) = MutationCompiler::new_localized_insert_or_eager(
            702,
            &txn,
            &fragment,
            schema,
            100_000,
            100_000,
            11,
            LocalizedInsertLocator {
                document: &document,
                block_path: block.node_path.as_slice(),
                position,
            },
        )
        .unwrap();
        eager.insert(0, position, inserted, &[]).unwrap();
        localized.insert(0, position, inserted, &[]).unwrap();
        let eager = eager.finish(Some(0)).unwrap();
        let localized = localized.finish(Some(0)).unwrap();
        preflight_mutation_plan(702, &eager, &txn).unwrap();
        preflight_mutation_plan(702, &localized, &txn).unwrap();
        drop(txn);
        (doc, eager, localized, mode)
    }

    fn assert_insert_plans_equal(eager: &YrsMutationPlan, localized: &YrsMutationPlan) {
        assert_eq!(eager.actions.len(), 1);
        assert_eq!(localized.actions.len(), 1);
        assert_plans_equal(eager, localized);
    }

    fn assert_plans_equal(eager: &YrsMutationPlan, localized: &YrsMutationPlan) {
        assert_eq!(
            eager.actions.iter().map(text_action_view).collect::<Vec<_>>(),
            localized
                .actions
                .iter()
                .map(text_action_view)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            eager.compilation_work_for_test(),
            localized.compilation_work_for_test()
        );
        assert_eq!(
            eager.expected_preflight_work_for_test(),
            localized.expected_preflight_work_for_test()
        );
        assert_eq!(eager.scan_work, localized.scan_work);
        assert_eq!(
            eager.position_resolver_work_for_test(),
            localized.position_resolver_work_for_test()
        );
    }

    #[test]
    fn localized_root_window_matches_eager_structural_plan_and_work() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [
                { "type": "h1", "content": [{ "type": "text", "text": "title" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "abc" }] }
            ]
        });
        let expected = json!({
            "type": "doc",
            "content": [
                { "type": "h1", "content": [{ "type": "text", "text": "title" }] },
                {
                    "type": "bulletList",
                    "content": [{
                        "type": "listItem",
                        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "abc" }] }]
                    }]
                }
            ]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let preview = from_prosemirror_json(&expected, &schema, UnknownTypeMode::Preserve).unwrap();
        let replacement_content = Fragment::from(vec![
            preview.root().content().unwrap().child(1).unwrap().clone(),
        ]);
        let replacement = crate::yrs_engine::StructuralReplacement::new(
            Vec::new(),
            1,
            2,
            replacement_content.clone(),
            crate::selection::Selection::cursor(0),
        );
        let from = document.root().content().unwrap().child(0).unwrap().node_size();
        let to = from + document.root().content().unwrap().child(1).unwrap().node_size();
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            720,
            &txn,
            &fragment,
            &schema,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            3,
            7,
        )
        .unwrap();
        fn charge_eager_boundaries(
            node: &Node,
            eager: &mut MutationCompiler,
        ) -> OperationResult<()> {
            eager.charge_boundary_node(0)?;
            if let Some(text) = node.text_str() {
                eager.charge_boundary_text(0, text.len())?;
            }
            if let Some(content) = node.content() {
                for child in content.iter() {
                    charge_eager_boundaries(child, eager)?;
                }
            }
            Ok(())
        }
        fn charge_localized_boundaries(
            node: &Node,
            localized: &mut LocalizedRootWindowCompiler,
        ) -> OperationResult<()> {
            localized.charge_boundary_node(0)?;
            if let Some(text) = node.text_str() {
                localized.charge_boundary_text(0, text.len())?;
            }
            if let Some(content) = node.content() {
                for child in content.iter() {
                    charge_localized_boundaries(child, localized)?;
                }
            }
            Ok(())
        }
        let compile_eager = |action_limit, scan_limit| {
            let mut compiler = MutationCompiler::new(
                720,
                &txn,
                &fragment,
                &schema,
                action_limit,
                scan_limit,
                19,
            )?;
            charge_eager_boundaries(document.root(), &mut compiler)?;
            compiler.replace_structural_range(
                0,
                MutationDocumentContext {
                    before: &document,
                    after: &preview,
                    schema: &schema,
                    limits: &limits,
                },
                ReplacementInput {
                    from,
                    to,
                    boundaries: &[],
                    content: &replacement_content,
                },
            )?;
            compiler.finish(Some(0))
        };
        let compile_localized = |action_limit, scan_limit| {
            let locator = LocalizedRootWindowLocator::mint(
                720,
                &document,
                &preview,
                &replacement,
                &seed,
                &txn,
                &fragment,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                3,
                7,
            )?
            .expect("exact prepared root window must mint");
            let mut compiler = LocalizedRootWindowCompiler::try_new(
                720,
                &txn,
                &fragment,
                &schema,
                action_limit,
                scan_limit,
                19,
                locator,
            )?
            .expect("aligned root must localize");
            charge_localized_boundaries(document.root(), &mut compiler)?;
            compiler.replace_structural_range(
                0,
                MutationDocumentContext {
                    before: &document,
                    after: &preview,
                    schema: &schema,
                    limits: &limits,
                },
                ReplacementInput {
                    from,
                    to,
                    boundaries: &[],
                    content: &replacement_content,
                },
            )
        };

        let baseline = compile_eager(100_000, 100_000).unwrap();
        let exact_compilation = baseline.compilation_work_for_test();
        let exact_preflight = baseline.expected_preflight_work_for_test();
        let exact_actions = exact_compilation
            .checked_add(exact_preflight)
            .expect("root-window action work must fit usize");
        let exact_scan = baseline.scan_work;
        assert!(exact_actions > 0);
        assert!(exact_scan > 0);
        let eager = compile_eager(exact_actions, exact_scan).unwrap();
        let localized = compile_localized(exact_actions, exact_scan).unwrap();
        preflight_mutation_plan(720, &eager, &txn).unwrap();
        preflight_mutation_plan(720, &localized, &txn).unwrap();

        assert_eq!(eager.actions.len(), 2);
        assert_eq!(localized.actions.len(), 2);
        for (eager, localized) in eager.actions.iter().zip(&localized.actions) {
            match (eager, localized) {
                (
                    YrsMutationAction::DeleteXmlChildren {
                        child_index: ei,
                        child_count: ec,
                        signature: es,
                        operation_index: eo,
                        ..
                    },
                    YrsMutationAction::DeleteXmlChildren {
                        child_index: li,
                        child_count: lc,
                        signature: ls,
                        operation_index: lo,
                        ..
                    },
                ) => assert_eq!((ei, ec, es, eo), (li, lc, ls, lo)),
                (
                    YrsMutationAction::InsertXmlChildren {
                        child_index: ei,
                        nodes: en,
                        signature: es,
                        operation_index: eo,
                        ..
                    },
                    YrsMutationAction::InsertXmlChildren {
                        child_index: li,
                        nodes: ln,
                        signature: ls,
                        operation_index: lo,
                        ..
                    },
                ) => {
                    assert_eq!((ei, es, eo), (li, ls, lo));
                    assert_eq!(format!("{en:?}"), format!("{ln:?}"));
                }
                _ => panic!("eager/localized structural action kinds differ"),
            }
        }
        assert_eq!(
            eager.compilation_work_for_test(),
            localized.compilation_work_for_test()
        );
        assert_eq!(
            eager.expected_preflight_work_for_test(),
            localized.expected_preflight_work_for_test()
        );
        assert_eq!(eager.scan_work, localized.scan_work);
        assert_eq!(eager.compilation_work_for_test(), exact_compilation);
        assert_eq!(eager.expected_preflight_work_for_test(), exact_preflight);
        assert_eq!(
            eager.compilation_work_for_test() + eager.expected_preflight_work_for_test(),
            exact_actions
        );
        assert_eq!(eager.scan_work, exact_scan);

        let action_limit = exact_actions - 1;
        let eager = compile_eager(action_limit, exact_scan).unwrap();
        let localized = compile_localized(action_limit, exact_scan).unwrap();
        assert_eq!(
            preflight_mutation_plan(720, &eager, &txn).unwrap_err(),
            preflight_mutation_plan(720, &localized, &txn).unwrap_err()
        );

        assert_eq!(
            compile_eager(exact_actions, exact_scan - 1).unwrap_err(),
            compile_localized(exact_actions, exact_scan - 1).unwrap_err()
        );
    }

    #[test]
    fn localized_root_window_rejects_wrong_replacement_content_with_attribution() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [
                { "type": "h1", "content": [{ "type": "text", "text": "title" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "abc" }] }
            ]
        });
        let expected = json!({
            "type": "doc",
            "content": [
                { "type": "h1", "content": [{ "type": "text", "text": "title" }] },
                {
                    "type": "bulletList",
                    "content": [{
                        "type": "listItem",
                        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "abc" }] }]
                    }]
                }
            ]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let preview = from_prosemirror_json(&expected, &schema, UnknownTypeMode::Preserve).unwrap();
        let expected_content = Fragment::from(vec![
            preview.root().content().unwrap().child(1).unwrap().clone(),
        ]);
        let wrong_content = Fragment::from(vec![
            document.root().content().unwrap().child(0).unwrap().clone(),
        ]);
        let replacement = crate::yrs_engine::StructuralReplacement::new(
            Vec::new(),
            1,
            2,
            expected_content,
            crate::selection::Selection::cursor(0),
        );
        let from = document.root().content().unwrap().child(0).unwrap().node_size();
        let to = from + document.root().content().unwrap().child(1).unwrap().node_size();
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            723,
            &txn,
            &fragment,
            &schema,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            6,
            10,
        )
        .unwrap();
        let locator = LocalizedRootWindowLocator::mint(
            723,
            &document,
            &preview,
            &replacement,
            &seed,
            &txn,
            &fragment,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            6,
            10,
        )
        .unwrap()
        .unwrap();
        let compiler = LocalizedRootWindowCompiler::try_new(
            723,
            &txn,
            &fragment,
            &schema,
            100_000,
            100_000,
            0,
            locator,
        )
        .unwrap()
        .unwrap();

        let error = compiler
            .replace_structural_range(
                4,
                MutationDocumentContext {
                    before: &document,
                    after: &preview,
                    schema: &schema,
                    limits: &limits,
                },
                ReplacementInput {
                    from,
                    to,
                    boundaries: &[],
                    content: &wrong_content,
                },
            )
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 723);
        assert_eq!(error.operation_index, Some(4));
        assert!(error.message.contains("content"));
    }

    #[test]
    fn localized_root_window_streams_normalized_attrs_without_map_materialization() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{ "type": "h1", "content": [{ "type": "text", "text": "title" }] }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let content = Fragment::from(vec![
            document.root().content().unwrap().child(0).unwrap().clone(),
        ]);
        let replacement = crate::yrs_engine::StructuralReplacement::new(
            Vec::new(),
            0,
            1,
            content,
            crate::selection::Selection::cursor(0),
        );
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            724,
            &txn,
            &fragment,
            &schema,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            7,
            11,
        )
        .unwrap();
        let locator = LocalizedRootWindowLocator::mint(
            724,
            &document,
            &document,
            &replacement,
            &seed,
            &txn,
            &fragment,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            7,
            11,
        )
        .unwrap()
        .unwrap();

        reset_localized_root_attr_map_builds_for_test();
        assert!(
            LocalizedRootWindowCompiler::try_new(
                724,
                &txn,
                &fragment,
                &schema,
                100_000,
                100_000,
                0,
                locator,
            )
            .unwrap()
            .is_some()
        );
        assert_eq!(take_localized_root_attr_map_builds_for_test(), 0);
    }

    #[test]
    fn localized_root_window_rejects_exact_stale_seal_matrix() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "a" }] }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let foreign_document =
            from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let replacement_content = Fragment::from(vec![
            document.root().content().unwrap().child(0).unwrap().clone(),
        ]);
        let replacement = crate::yrs_engine::StructuralReplacement::new(
            Vec::new(),
            0,
            1,
            replacement_content,
            crate::selection::Selection::cursor(0),
        );
        let doc = seeded_document(&source, &schema, &limits);
        {
            let mut txn = doc.transact_mut();
            txn.get_or_insert_xml_fragment("alternate");
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let alternate_fragment = txn.get_xml_fragment("alternate").unwrap();
        let foreign_doc = seeded_document(&source, &schema, &limits);
        let foreign_txn = foreign_doc.transact();
        let foreign_fragment = foreign_txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            726,
            &txn,
            &fragment,
            &schema,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            4,
            8,
        )
        .unwrap();
        let mut resource_drift = limits.clone();
        resource_drift.max_input_bytes -= 1;
        let mut editing_drift = editing_limits.clone();
        editing_drift.max_operations_per_transaction -= 1;
        let mint = |semantic,
                    txn,
                    fragment,
                    resource_limits,
                    editing_limits,
                    max_length,
                    fingerprint,
                    epoch,
                    revision| {
            LocalizedRootWindowLocator::mint(
                726,
                semantic,
                semantic,
                &replacement,
                &seed,
                txn,
                fragment,
                resource_limits,
                editing_limits,
                max_length,
                fingerprint,
                epoch,
                revision,
            )
            .unwrap()
            .is_none()
        };

        for (case, rejected) in [
            (
                "semanticRoot",
                mint(
                    &foreign_document,
                    &txn,
                    &fragment,
                    &limits,
                    &editing_limits,
                    None,
                    "schema-a",
                    4,
                    8,
                ),
            ),
            (
                "store",
                mint(
                    &document,
                    &foreign_txn,
                    &foreign_fragment,
                    &limits,
                    &editing_limits,
                    None,
                    "schema-a",
                    4,
                    8,
                ),
            ),
            (
                "fragment",
                mint(
                    &document,
                    &txn,
                    &alternate_fragment,
                    &limits,
                    &editing_limits,
                    None,
                    "schema-a",
                    4,
                    8,
                ),
            ),
            (
                "schemaFingerprint",
                mint(
                    &document,
                    &txn,
                    &fragment,
                    &limits,
                    &editing_limits,
                    None,
                    "schema-b",
                    4,
                    8,
                ),
            ),
            (
                "epoch",
                mint(
                    &document,
                    &txn,
                    &fragment,
                    &limits,
                    &editing_limits,
                    None,
                    "schema-a",
                    5,
                    8,
                ),
            ),
            (
                "revision",
                mint(
                    &document,
                    &txn,
                    &fragment,
                    &limits,
                    &editing_limits,
                    None,
                    "schema-a",
                    4,
                    9,
                ),
            ),
            (
                "resourceLimits",
                mint(
                    &document,
                    &txn,
                    &fragment,
                    &resource_drift,
                    &editing_limits,
                    None,
                    "schema-a",
                    4,
                    8,
                ),
            ),
            (
                "editingLimits",
                mint(
                    &document,
                    &txn,
                    &fragment,
                    &limits,
                    &editing_drift,
                    None,
                    "schema-a",
                    4,
                    8,
                ),
            ),
            (
                "maxLength",
                mint(
                    &document,
                    &txn,
                    &fragment,
                    &limits,
                    &editing_limits,
                    Some(1),
                    "schema-a",
                    4,
                    8,
                ),
            ),
        ] {
            assert!(rejected, "{case} must signal the eager fallback boundary");
        }

        for (case, mismatch_txn, mismatch_fragment) in [
            ("store", &foreign_txn, &foreign_fragment),
            ("fragment", &txn, &alternate_fragment),
        ] {
            let locator = LocalizedRootWindowLocator::mint(
                726,
                &document,
                &document,
                &replacement,
                &seed,
                &txn,
                &fragment,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                4,
                8,
            )
            .unwrap()
            .unwrap();
            assert!(
                LocalizedRootWindowCompiler::try_new(
                    726,
                    mismatch_txn,
                    mismatch_fragment,
                    &schema,
                    100_000,
                    100_000,
                    0,
                    locator,
                )
                .unwrap()
                .is_none(),
                "{case} try_new must signal the eager fallback boundary"
            );
        }
    }

    #[test]
    fn localized_root_window_fails_closed_for_invalid_shapes_and_shallow_drift() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "a" }] }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let replacement_content = Fragment::from(vec![
            document.root().content().unwrap().child(0).unwrap().clone(),
        ]);
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            721,
            &txn,
            &fragment,
            &schema,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            4,
            8,
        )
        .unwrap();
        for (case, parent_path, from_child, to_child) in [
            ("nonroot", vec![0], 0, 1),
            ("empty", vec![], 0, 0),
            ("outOfBounds", vec![], 0, 2),
        ] {
            let replacement = crate::yrs_engine::StructuralReplacement::new(
                parent_path,
                from_child,
                to_child,
                replacement_content.clone(),
                crate::selection::Selection::cursor(0),
            );
            assert!(
                LocalizedRootWindowLocator::mint(
                    721,
                    &document,
                    &document,
                    &replacement,
                    &seed,
                    &txn,
                    &fragment,
                    &limits,
                    &editing_limits,
                    None,
                    "schema-a",
                    4,
                    8,
                )
                .unwrap()
                .is_none(),
                "{case}"
            );
        }

        let assert_alignment_fallback = |case: &str, semantic: Document, wire: Value| {
            let wire_doc = seeded_document(&wire, &schema, &limits);
            let wire_txn = wire_doc.transact();
            let wire_fragment = wire_txn.get_xml_fragment("prosemirror").unwrap();
            let wire_seed = MutationLookupSeed::build(
                722,
                &wire_txn,
                &wire_fragment,
                &schema,
                &semantic,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                5,
                9,
            )
            .unwrap();
            let content = Fragment::from(vec![
                semantic.root().content().unwrap().child(0).unwrap().clone(),
            ]);
            let replacement = crate::yrs_engine::StructuralReplacement::new(
                Vec::new(),
                0,
                1,
                content,
                crate::selection::Selection::cursor(0),
            );
            let locator = LocalizedRootWindowLocator::mint(
                722,
                &semantic,
                &semantic,
                &replacement,
                &wire_seed,
                &wire_txn,
                &wire_fragment,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                5,
                9,
            )
            .unwrap()
            .expect("shape and exact seed context should mint before alignment");
            assert!(
                LocalizedRootWindowCompiler::try_new(
                    722,
                    &wire_txn,
                    &wire_fragment,
                    &schema,
                    100_000,
                    100_000,
                    0,
                    locator,
                )
                .unwrap()
                .is_none(),
                "{case}"
            );
        };
        let heading = from_prosemirror_json(
            &json!({
                "type": "doc",
                "content": [{ "type": "h1", "content": [{ "type": "text", "text": "a" }] }]
            }),
            &schema,
            UnknownTypeMode::Preserve,
        )
        .unwrap();
        assert_alignment_fallback(
            "normalizedType",
            heading.clone(),
            json!({
                "type": "doc",
                "content": [{ "type": "h2", "content": [{ "type": "text", "text": "a" }] }]
            }),
        );
        assert_alignment_fallback(
            "normalizedAttrs",
            heading,
            json!({
                "type": "doc",
                "content": [{
                    "type": "h1",
                    "attrs": { "id": "foreign" },
                    "content": [{ "type": "text", "text": "a" }]
                }]
            }),
        );
        assert_alignment_fallback(
            "cardinality",
            from_prosemirror_json(
                &json!({
                    "type": "doc",
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "a" }] },
                        { "type": "paragraph", "content": [{ "type": "text", "text": "b" }] }
                    ]
                }),
                &schema,
                UnknownTypeMode::Preserve,
            )
            .unwrap(),
            source.clone(),
        );
        let hostile_void = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "horizontalRule".into(),
                HashMap::new(),
                Fragment::empty(),
            )]),
        ));
        assert_alignment_fallback(
            "void",
            hostile_void,
            json!({ "type": "doc", "content": [{ "type": "horizontalRule" }] }),
        );

        let replacement = crate::yrs_engine::StructuralReplacement::new(
            Vec::new(),
            0,
            1,
            replacement_content,
            crate::selection::Selection::cursor(0),
        );
        let mut stale_width_seed = seed.clone();
        let MutationLookupSeedState::Ready(payload) = &mut stale_width_seed.state else {
            panic!("freshly built lookup seed must be ready")
        };
        payload.path_parent_widths = Arc::new(HashMap::from([(
            AsRef::<Branch>::as_ref(&fragment).id(),
            2,
        )]));
        let locator = LocalizedRootWindowLocator::mint(
            721,
            &document,
            &document,
            &replacement,
            &stale_width_seed,
            &txn,
            &fragment,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            4,
            8,
        )
        .unwrap()
        .unwrap();
        assert!(
            LocalizedRootWindowCompiler::try_new(
                721,
                &txn,
                &fragment,
                &schema,
                100_000,
                100_000,
                0,
                locator,
            )
            .unwrap()
            .is_none(),
            "rootWidth"
        );
    }

    #[test]
    fn localized_existing_textblock_insert_matches_eager_action_signature_and_work() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abc" }]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let position_map = PositionMap::build(&document, &schema);
        let block = position_map.block(0).unwrap();
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();

        let mut eager =
            MutationCompiler::new(701, &txn, &fragment, &schema, 100_000, 100_000, 7).unwrap();
        let (mut localized, mode) = MutationCompiler::new_localized_insert_or_eager(
            701,
            &txn,
            &fragment,
            &schema,
            100_000,
            100_000,
            7,
            LocalizedInsertLocator {
                document: &document,
                block_path: block.node_path.as_slice(),
                position: 2,
            },
        )
        .unwrap();
        assert_eq!(mode, MutationCompilerBuild::Localized);

        eager.insert(0, 2, "X", &[]).unwrap();
        localized.insert(0, 2, "X", &[]).unwrap();
        let eager = eager.finish(Some(0)).unwrap();
        let localized = localized.finish(Some(0)).unwrap();

        assert_eq!(eager.actions.len(), localized.actions.len());
        assert_eq!(
            eager
                .actions
                .iter()
                .map(action_signature)
                .collect::<Vec<_>>(),
            localized
                .actions
                .iter()
                .map(action_signature)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            eager.compilation_work_for_test(),
            localized.compilation_work_for_test()
        );
        assert_eq!(eager.scan_work, localized.scan_work);
    }

    #[test]
    fn localized_ascii_and_non_bmp_start_middle_and_end_match_eager_utf16_indices() {
        let schema = tiptap_schema();
        for (text, cases) in [
            ("abc", vec![(0, 0), (1, 1), (3, 3)]),
            ("a😀b", vec![(0, 0), (1, 1), (2, 3), (3, 4)]),
        ] {
            let source = json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": text }]
                }]
            });
            for (block_offset, expected_utf16) in cases {
                let (_doc, eager, localized, mode) =
                    compile_pair_at_block_offset(&source, &schema, 0, block_offset, "Ω");
                assert_eq!(mode, MutationCompilerBuild::Localized);
                assert_insert_plans_equal(&eager, &localized);
                assert_eq!(
                    action_signature(&localized.actions[0]).index_utf16,
                    expected_utf16
                );
            }
        }
    }

    #[test]
    fn localized_fragmented_xml_text_targets_match_eager_signature_and_work() {
        let schema = tiptap_schema();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "ab", "marks": [{ "type": "bold" }] },
                    { "type": "text", "text": "😀c" },
                    { "type": "text", "text": "de", "marks": [{ "type": "italic" }] }
                ]
            }]
        });
        let (_doc, eager, localized, mode) =
            compile_pair_at_block_offset(&source, &schema, 0, 3, "Z");
        assert_eq!(mode, MutationCompilerBuild::Localized);
        assert_plans_equal(&eager, &localized);
    }

    #[test]
    fn localized_fragmented_mark_runs_in_one_xml_text_match_eager() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "ab😀cde" }]
            }]
        });
        let doc = seeded_document(&source, &schema, &limits);
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("paragraph must be an XML element")
            };
            let XmlOut::Text(text) = paragraph.get(&txn, 0).unwrap() else {
                panic!("paragraph must contain XML text")
            };
            text.format(
                &mut txn,
                0,
                2,
                Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
            );
            text.format(
                &mut txn,
                4,
                3,
                Attrs::from([(Arc::<str>::from("italic"), Any::Bool(true))]),
            );
        }
        let codec = YrsDocumentCodec::new(&schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let semantic_json = codec.read_json(&fragment, &txn).unwrap();
        let document =
            from_prosemirror_json(&semantic_json, &schema, UnknownTypeMode::Preserve).unwrap();
        let position_map = PositionMap::build(&document, &schema);
        let block = position_map.block(0).unwrap();
        let position = block.doc_start + 3;
        let mut eager =
            MutationCompiler::new(704, &txn, &fragment, &schema, 100_000, 100_000, 0).unwrap();
        let (mut localized, mode) = MutationCompiler::new_localized_insert_or_eager(
            704,
            &txn,
            &fragment,
            &schema,
            100_000,
            100_000,
            0,
            LocalizedInsertLocator {
                document: &document,
                block_path: block.node_path.as_slice(),
                position,
            },
        )
        .unwrap();
        assert_eq!(mode, MutationCompilerBuild::Localized);
        eager.insert(0, position, "Z", &[]).unwrap();
        localized.insert(0, position, "Z", &[]).unwrap();
        let eager = eager.finish(Some(0)).unwrap();
        let localized = localized.finish(Some(0)).unwrap();
        preflight_mutation_plan(704, &eager, &txn).unwrap();
        preflight_mutation_plan(704, &localized, &txn).unwrap();
        assert_insert_plans_equal(&eager, &localized);
        assert!(action_signature(&localized.actions[0]).signature.runs.len() >= 3);
    }

    #[test]
    fn localized_nested_custom_list_textblock_matches_eager() {
        let schema = Schema::from_json(&json!({
            "nodes": [
                { "name": "doc", "content": "taskList+", "role": "doc" },
                { "name": "taskList", "content": "taskItem+", "role": "list" },
                { "name": "taskItem", "content": "body", "role": "listItem" },
                { "name": "body", "content": "text*", "role": "textBlock" },
                { "name": "text", "content": "", "role": "text" }
            ],
            "marks": []
        }))
        .unwrap();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "taskList",
                "content": [{
                    "type": "taskItem",
                    "content": [{
                        "type": "body",
                        "content": [{ "type": "text", "text": "nested" }]
                    }]
                }]
            }]
        });
        let (_doc, eager, localized, mode) =
            compile_pair_at_block_offset(&source, &schema, 0, 3, "!");
        assert_eq!(mode, MutationCompilerBuild::Localized);
        assert_insert_plans_equal(&eager, &localized);
    }

    #[test]
    fn localized_empty_inline_void_and_cross_block_inputs_choose_eager_before_lowering() {
        let schema = tiptap_schema();
        let cases = [
            (
                json!({
                    "type": "doc",
                    "content": [{ "type": "paragraph" }]
                }),
                0,
                0,
            ),
            (
                json!({
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "a" },
                            { "type": "hardBreak" },
                            { "type": "text", "text": "b" }
                        ]
                    }]
                }),
                0,
                0,
            ),
            (
                json!({
                    "type": "doc",
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "a" }] },
                        { "type": "paragraph", "content": [{ "type": "text", "text": "b" }] }
                    ]
                }),
                0,
                3,
            ),
            (
                json!({
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "ab", "marks": [{ "type": "bold" }] },
                            { "type": "text", "text": "cd" }
                        ]
                    }]
                }),
                0,
                2,
            ),
        ];
        for (source, block_index, extra_position) in cases {
            let limits = ResourceLimits::default();
            let document =
                from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
            let position_map = PositionMap::build(&document, &schema);
            let doc = seeded_document(&source, &schema, &limits);
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let block = position_map.block(block_index).unwrap();
            let position = block.doc_start + extra_position;
            let (_compiler, mode) = MutationCompiler::new_localized_insert_or_eager(
                703,
                &txn,
                &fragment,
                &schema,
                100_000,
                100_000,
                0,
                LocalizedInsertLocator {
                    document: &document,
                    block_path: block.node_path.as_slice(),
                    position,
                },
            )
            .unwrap();
            assert_eq!(mode, MutationCompilerBuild::EagerFallback);
        }
    }

    #[test]
    fn localized_and_eager_hit_the_same_logical_work_limit() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let source = json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "two" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }
            ]
        });
        let (_baseline_doc, baseline, localized_baseline, mode) =
            compile_pair_at_block_offset(&source, &schema, 1, 1, "X");
        assert_eq!(mode, MutationCompilerBuild::Localized);
        assert_insert_plans_equal(&baseline, &localized_baseline);
        let limit = baseline.compilation_work_for_test().saturating_sub(1);

        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let position_map = PositionMap::build(&document, &schema);
        let block = position_map.block(1).unwrap();
        let position = block.doc_start + 1;
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let mut eager =
            MutationCompiler::new(705, &txn, &fragment, &schema, limit, 100_000, 11).unwrap();
        let (mut localized, mode) = MutationCompiler::new_localized_insert_or_eager(
            705,
            &txn,
            &fragment,
            &schema,
            limit,
            100_000,
            11,
            LocalizedInsertLocator {
                document: &document,
                block_path: block.node_path.as_slice(),
                position,
            },
        )
        .unwrap();
        assert_eq!(mode, MutationCompilerBuild::Localized);
        let eager_error = eager.insert(0, position, "X", &[]).unwrap_err();
        let localized_error = localized.insert(0, position, "X", &[]).unwrap_err();
        assert_eq!(eager_error, localized_error);
    }

    #[test]
    fn localized_plan_retains_eager_stale_and_foreign_document_guards() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "guarded" }]
            }]
        });
        let (doc, _eager, localized, mode) =
            compile_pair_at_block_offset(&source, &schema, 0, 2, "!");
        assert_eq!(mode, MutationCompilerBuild::Localized);
        let foreign = seeded_document(&source, &schema, &limits);
        let foreign_txn = foreign.transact();
        let foreign_error = preflight_mutation_plan(702, &localized, &foreign_txn).unwrap_err();
        assert_eq!(foreign_error.code, "ENGINE_INVARIANT_FAILED");

        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("paragraph must be an XML element")
            };
            let XmlOut::Text(text) = paragraph.get(&txn, 0).unwrap() else {
                panic!("paragraph must contain XML text")
            };
            text.insert(&mut txn, 0, "stale");
        }
        let stale_txn = doc.transact();
        let stale_error = preflight_mutation_plan(702, &localized, &stale_txn).unwrap_err();
        assert_eq!(stale_error.code, "ENGINE_INVARIANT_FAILED");
    }

    #[test]
    fn seeded_localized_insert_is_restricted_and_matches_eager_without_eager_rebuild() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "a😀b" }]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let position_map = PositionMap::build(&document, &schema);
        let block = position_map.block(0).unwrap();
        let position = block.doc_start + 2;
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            706,
            &txn,
            &fragment,
            &schema,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            9,
            4,
        )
        .unwrap();
        let localized = LocalizedInsertCompiler::try_new(
            706,
            &txn,
            &fragment,
            &schema,
            100_000,
            100_000,
            11,
            LocalizedInsertLocator {
                document: &document,
                block_path: block.node_path.as_slice(),
                position,
            },
            &seed,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            9,
            4,
        )
        .unwrap()
        .expect("existing text insert must localize");
        let localized = localized.compile(0, position, "Ω", &[]).unwrap();

        let mut eager =
            MutationCompiler::new(706, &txn, &fragment, &schema, 100_000, 100_000, 11).unwrap();
        eager.insert(0, position, "Ω", &[]).unwrap();
        let eager = eager.finish(Some(0)).unwrap();
        assert_insert_plans_equal(&eager, &localized);
    }

    #[test]
    fn seeded_marked_non_bmp_insert_preserves_exact_action_and_input_ceilings() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "a😀b" }]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let block = PositionMap::build(&document, &schema).block(0).unwrap().clone();
        let position = block.doc_start + 2;
        let marks = vec![Mark::new("bold".into(), HashMap::new())];
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            707, &txn, &fragment, &schema, &document, &limits, &editing_limits, None, "schema-a", 3, 2,
        )
        .unwrap();

        let compile_eager = |action_limit, scan_limit| {
            let mut compiler = MutationCompiler::new(
                707, &txn, &fragment, &schema, action_limit, scan_limit, 11,
            )?;
            compiler.insert(0, position, "🦀", &marks)?;
            compiler.finish(Some(0))
        };
        let compile_localized = |action_limit, scan_limit| {
            LocalizedInsertCompiler::try_new(
                707,
                &txn,
                &fragment,
                &schema,
                action_limit,
                scan_limit,
                11,
                LocalizedInsertLocator {
                    document: &document,
                    block_path: block.node_path.as_slice(),
                    position,
                },
                &seed,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                3,
                2,
            )?
            .expect("marked existing-text insert must localize")
            .compile(0, position, "🦀", &marks)
        };

        let baseline = compile_eager(100_000, 100_000).unwrap();
        let exact_actions = baseline.compilation_work_for_test();
        let exact_input = baseline.scan_work;
        let localized = compile_localized(exact_actions, exact_input).unwrap();
        assert_insert_plans_equal(&baseline, &localized);
        assert!(!action_signature(&localized.actions[0]).attrs.is_empty());

        for (action_limit, scan_limit) in [
            (exact_actions.saturating_sub(1), exact_input),
            (exact_actions, exact_input.saturating_sub(1)),
        ] {
            assert_eq!(
                compile_eager(action_limit, scan_limit).unwrap_err(),
                compile_localized(action_limit, scan_limit).unwrap_err()
            );
        }
    }

    #[test]
    fn promoted_marked_fragmented_non_bmp_insert_preserves_second_insert_exact_work_and_errors() {
        use crate::transform::{apply_step_canonical_marks, Step};
        use crate::yrs_engine::mutation::execute_mutation_plan;

        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let bold = Mark::new("bold".into(), HashMap::new());
        let italic = Mark::new("italic".into(), HashMap::new());
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "a", "marks": [{ "type": "bold" }] },
                    { "type": "text", "text": "😀", "marks": [{ "type": "italic" }] },
                    { "type": "text", "text": "b", "marks": [{ "type": "bold" }] }
                ]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let block = PositionMap::build(&document, &schema).block(0).unwrap().clone();
        let first_position = block.doc_start;
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            709, &txn, &fragment, &schema, &document, &limits, &editing_limits, None, "schema-a", 3, 2,
        )
        .unwrap();
        let (first_plan, promotion) = LocalizedInsertCompiler::try_new(
            709,
            &txn,
            &fragment,
            &schema,
            100_000,
            100_000,
            11,
            LocalizedInsertLocator {
                document: &document,
                block_path: block.node_path.as_slice(),
                position: first_position,
            },
            &seed,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            3,
            2,
        )
        .unwrap()
        .expect("fragmented marked text must localize")
        .compile_with_promotion(0, first_position, "🦀", std::slice::from_ref(&bold))
        .unwrap();
        preflight_mutation_plan(709, &first_plan, &txn).unwrap();
        drop(txn);
        {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(first_plan, &mut txn);
        }
        let (after, _) = apply_step_canonical_marks(
            &document,
            &Step::InsertText {
                pos: first_position,
                text: "🦀".into(),
                marks: vec![bold],
            },
            &schema,
        )
        .unwrap();
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let promoted = seed
            .prepare_promotion(
                &txn,
                &fragment,
                &promotion,
                &document,
                &after,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                3,
                2,
                4,
                3,
            )
            .unwrap();
        let second_block = PositionMap::build(&after, &schema).block(0).unwrap().clone();
        let second_position = first_position + 1;

        let compile_eager = |action_limit, scan_limit| {
            let mut compiler = MutationCompiler::new(
                710, &txn, &fragment, &schema, action_limit, scan_limit, 11,
            )?;
            compiler.insert(
                0,
                second_position,
                "界",
                std::slice::from_ref(&italic),
            )?;
            compiler.finish(Some(0))
        };
        let compile_localized = |action_limit, scan_limit| {
            LocalizedInsertCompiler::try_new(
                710,
                &txn,
                &fragment,
                &schema,
                action_limit,
                scan_limit,
                11,
                LocalizedInsertLocator {
                    document: &after,
                    block_path: second_block.node_path.as_slice(),
                    position: second_position,
                },
                &promoted,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                4,
                3,
            )?
            .expect("promoted fragmented marked text must localize again")
            .compile(0, second_position, "界", std::slice::from_ref(&italic))
        };

        let eager = compile_eager(100_000, 100_000).unwrap();
        let exact_actions = eager.compilation_work_for_test();
        let exact_input = eager.scan_work;
        let localized = compile_localized(exact_actions, exact_input).unwrap();
        assert_insert_plans_equal(&eager, &localized);
        for (action_limit, scan_limit) in [
            (exact_actions - 1, exact_input),
            (exact_actions, exact_input - 1),
        ] {
            assert_eq!(
                compile_eager(action_limit, scan_limit).unwrap_err(),
                compile_localized(action_limit, scan_limit).unwrap_err()
            );
        }
    }

    #[test]
    fn promoted_insert_materialization_work_admits_chained_localized_format() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let bold = Mark::new("bold".into(), HashMap::new());
        let italic = Mark::new("italic".into(), HashMap::new());
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abc" }]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let block = PositionMap::build(&document, &schema).block(0).unwrap().clone();
        let insert_position = block.doc_start + 1;
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            711, &txn, &fragment, &schema, &document, &limits, &editing_limits, None, "schema-a", 5, 8,
        )
        .unwrap();
        let (insert_plan, promotion) = LocalizedInsertCompiler::try_new(
            711,
            &txn,
            &fragment,
            &schema,
            100_000,
            100_000,
            13,
            LocalizedInsertLocator {
                document: &document,
                block_path: block.node_path.as_slice(),
                position: insert_position,
            },
            &seed,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            5,
            8,
        )
        .unwrap()
        .expect("existing insert must localize")
        .compile_with_promotion(
            0,
            insert_position,
            "X",
            std::slice::from_ref(&bold),
        )
        .unwrap();
        drop(txn);
        {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(insert_plan, &mut txn);
        }
        let (after, _) = apply_step_canonical_marks(
            &document,
            &Step::InsertText {
                pos: insert_position,
                text: "X".into(),
                marks: vec![bold],
            },
            &schema,
        )
        .unwrap();
        let after_block = PositionMap::build(&after, &schema).block(0).unwrap().clone();
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let promoted = seed
            .prepare_promotion(
                &txn,
                &fragment,
                &promotion,
                &document,
                &after,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                5,
                8,
                6,
                9,
            )
            .unwrap();
        let from = after_block.doc_start;
        let to = after_block.doc_end;
        let boundaries = [from, insert_position, insert_position + 1, to];

        let mut eager =
            MutationCompiler::new(712, &txn, &fragment, &schema, 100_000, 100_000, 13).unwrap();
        eager
            .format(0, from, to, &boundaries, mark_attr(&italic))
            .unwrap();
        let eager = eager.finish(Some(0)).unwrap();
        let locator = LocalizedFormatLocator::mint(
            &after,
            after_block.node_path.as_slice(),
            from,
            to,
            &promoted,
            &txn,
            &fragment,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            6,
            9,
        )
        .expect("the promoted insert seed must mint an exact format locator");
        let localized = LocalizedFormatCompiler::try_new(
            712,
            &txn,
            &fragment,
            &schema,
            100_000,
            100_000,
            13,
            locator,
            "schema-a",
            6,
            9,
        )
        .unwrap()
        .expect("the promoted insert seed must admit a chained localized format")
        .format(0, from, to, &boundaries, mark_attr(&italic))
        .unwrap()
        .0;

        assert_plans_equal(&eager, &localized);
    }

    #[test]
    fn localized_format_promotion_derives_current_work_in_one_target_pass() {
        use yrs::types::xml::XmlTextPrelim;

        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let text = "abcdefghijklmnop";
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": text }]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let block = PositionMap::build(&document, &schema).block(0).unwrap().clone();
        let doc = seeded_document(&source, &schema, &limits);
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("paragraph must be an XML element")
            };
            paragraph.remove_range(&mut txn, 0, 1);
            for scalar in text.chars() {
                paragraph.push_back(&mut txn, XmlTextPrelim::new(scalar.to_string()));
            }
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            713, &txn, &fragment, &schema, &document, &limits, &editing_limits, None, "schema-a", 7, 11,
        )
        .unwrap();
        let from = block.doc_start;
        let to = block.doc_end;
        let locator = LocalizedFormatLocator::mint(
            &document,
            block.node_path.as_slice(),
            from,
            to,
            &seed,
            &txn,
            &fragment,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            7,
            11,
        )
        .expect("exact multi-leaf context must mint a format locator");
        let localized = LocalizedFormatCompiler::try_new(
            713,
            &txn,
            &fragment,
            &schema,
            100_000,
            100_000,
            0,
            locator,
            "schema-a",
            7,
            11,
        )
        .unwrap()
        .expect("multi-leaf textblock must localize");

        reset_localized_format_promotion_target_visits_for_test();
        let (plan, _) = localized
            .format(0, from, to, &[from, to], mark_attr(&Mark::new("bold".into(), HashMap::new())))
            .unwrap();
        let visits = take_localized_format_promotion_target_visits_for_test();

        assert_eq!(plan.actions.len(), text.chars().count());
        assert_eq!(visits, plan.actions.len());
    }

    #[test]
    fn localized_format_add_remove_four_leaf_unicode_fragmented_parity() {
        use yrs::types::xml::XmlTextPrelim;

        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "a😀bc🦀def" }]
            }]
        });
        let doc = seeded_document(&source, &schema, &limits);
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("paragraph must be an XML element")
            };
            paragraph.remove_range(&mut txn, 0, 1);
            let first = paragraph.push_back(&mut txn, XmlTextPrelim::new("a😀"));
            let second = paragraph.push_back(&mut txn, XmlTextPrelim::new("bc"));
            let third = paragraph.push_back(&mut txn, XmlTextPrelim::new("🦀d"));
            let fourth = paragraph.push_back(&mut txn, XmlTextPrelim::new("ef"));
            first.format(
                &mut txn,
                0,
                1,
                Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
            );
            second.format(
                &mut txn,
                0,
                1,
                Attrs::from([(Arc::<str>::from("italic"), Any::Bool(true))]),
            );
            third.format(
                &mut txn,
                0,
                2,
                Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
            );
            fourth.format(
                &mut txn,
                0,
                1,
                Attrs::from([(Arc::<str>::from("italic"), Any::Bool(true))]),
            );
        }
        let codec = YrsDocumentCodec::new(&schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let semantic_json = codec.read_json(&fragment, &txn).unwrap();
        let document =
            from_prosemirror_json(&semantic_json, &schema, UnknownTypeMode::Preserve).unwrap();
        let block = PositionMap::build(&document, &schema).block(0).unwrap().clone();
        let seed = MutationLookupSeed::build(
            716,
            &txn,
            &fragment,
            &schema,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            12,
            18,
        )
        .unwrap();
        let from = block.doc_start;
        let to = block.doc_end;
        let boundaries = (from..=to).collect::<Vec<_>>();
        let bold = Mark::new("bold".into(), HashMap::new());

        for (case, attrs) in [
            ("add", mark_attr(&bold)),
            ("remove", removed_mark_attr("italic")),
        ] {
            let mut eager =
                MutationCompiler::new(716, &txn, &fragment, &schema, 100_000, 100_000, 23)
                    .unwrap();
            eager
                .format(0, from, to, &boundaries, attrs.clone())
                .unwrap();
            let eager = eager.finish(Some(0)).unwrap();

            let locator = LocalizedFormatLocator::mint(
                &document,
                block.node_path.as_slice(),
                from,
                to,
                &seed,
                &txn,
                &fragment,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                12,
                18,
            )
            .expect("exact four-leaf format context must mint a locator");
            let localized = LocalizedFormatCompiler::try_new(
                716,
                &txn,
                &fragment,
                &schema,
                100_000,
                100_000,
                23,
                locator,
                "schema-a",
                12,
                18,
            )
            .unwrap()
            .expect("four-leaf format must localize")
            .format(0, from, to, &boundaries, attrs)
            .unwrap()
            .0;

            preflight_mutation_plan(716, &eager, &txn).unwrap();
            preflight_mutation_plan(716, &localized, &txn).unwrap();
            assert_plans_equal(&eager, &localized);
            let branches = localized
                .actions
                .iter()
                .filter_map(|action| match action {
                    YrsMutationAction::FormatText { signature, .. } => {
                        Some(signature.target.clone())
                    }
                    _ => None,
                })
                .collect::<HashSet<_>>();
            assert_eq!(branches.len(), 4, "{case}");
        }
    }

    #[test]
    fn localized_format_four_leaf_unicode_exact_action_and_scan_limits_match_eager() {
        use yrs::types::xml::XmlTextPrelim;

        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "a😀bc🦀def" }]
            }]
        });
        let doc = seeded_document(&source, &schema, &limits);
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("paragraph must be an XML element")
            };
            paragraph.remove_range(&mut txn, 0, 1);
            let first = paragraph.push_back(&mut txn, XmlTextPrelim::new("a😀"));
            let second = paragraph.push_back(&mut txn, XmlTextPrelim::new("bc"));
            let third = paragraph.push_back(&mut txn, XmlTextPrelim::new("🦀d"));
            let fourth = paragraph.push_back(&mut txn, XmlTextPrelim::new("ef"));
            for text in [&first, &third] {
                text.format(
                    &mut txn,
                    0,
                    1,
                    Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
                );
            }
            for text in [&second, &fourth] {
                text.format(
                    &mut txn,
                    0,
                    1,
                    Attrs::from([(Arc::<str>::from("italic"), Any::Bool(true))]),
                );
            }
        }
        let codec = YrsDocumentCodec::new(&schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let semantic_json = codec.read_json(&fragment, &txn).unwrap();
        let document =
            from_prosemirror_json(&semantic_json, &schema, UnknownTypeMode::Preserve).unwrap();
        let block = PositionMap::build(&document, &schema).block(0).unwrap().clone();
        let seed = MutationLookupSeed::build(
            719,
            &txn,
            &fragment,
            &schema,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            14,
            20,
        )
        .unwrap();
        let from = block.doc_start;
        let to = block.doc_end;
        let boundaries = (from..=to).collect::<Vec<_>>();
        let attrs = mark_attr(&Mark::new("bold".into(), HashMap::new()));
        let compile_eager = |action_limit, scan_limit| {
            let mut compiler = MutationCompiler::new(
                719,
                &txn,
                &fragment,
                &schema,
                action_limit,
                scan_limit,
                23,
            )?;
            compiler.format(0, from, to, &boundaries, attrs.clone())?;
            compiler.finish(Some(0))
        };
        let compile_localized = |action_limit, scan_limit| {
            let locator = LocalizedFormatLocator::mint(
                &document,
                block.node_path.as_slice(),
                from,
                to,
                &seed,
                &txn,
                &fragment,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                14,
                20,
            )
            .expect("exact four-leaf format context must mint a locator");
            LocalizedFormatCompiler::try_new(
                719,
                &txn,
                &fragment,
                &schema,
                action_limit,
                scan_limit,
                23,
                locator,
                "schema-a",
                14,
                20,
            )?
            .expect("four-leaf format must localize")
            .format(0, from, to, &boundaries, attrs.clone())
            .map(|(plan, _)| plan)
        };

        let eager = compile_eager(100_000, 100_000).unwrap();
        let exact_actions = eager.compilation_work_for_test();
        let exact_scan = eager.scan_work;
        let localized = compile_localized(exact_actions, exact_scan).unwrap();
        assert_plans_equal(&eager, &localized);
        for (action_limit, scan_limit) in [
            (exact_actions - 1, exact_scan),
            (exact_actions, exact_scan - 1),
        ] {
            assert_eq!(
                compile_eager(action_limit, scan_limit).unwrap_err(),
                compile_localized(action_limit, scan_limit).unwrap_err()
            );
        }
    }

    #[test]
    fn localized_format_rejects_foreign_semantic_root_with_identical_selected_block() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "source" }] }
            ]
        });
        let foreign_source = json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "foreign" }] }
            ]
        });
        let source_document =
            from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let foreign_document =
            from_prosemirror_json(&foreign_source, &schema, UnknownTypeMode::Preserve).unwrap();
        let block = PositionMap::build(&source_document, &schema).block(0).unwrap().clone();
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            714, &txn, &fragment, &schema, &source_document, &limits, &editing_limits, None, "schema-a", 5, 9,
        )
        .unwrap();

        let localized = LocalizedFormatLocator::mint(
            &foreign_document,
            block.node_path.as_slice(),
            block.doc_start,
            block.doc_end,
            &seed,
            &txn,
            &fragment,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            5,
            9,
        );

        assert!(localized.is_none());
    }

    #[test]
    fn localized_format_seal_rejects_stale_storage_schema_epoch_and_revision() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abc" }]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let block = PositionMap::build(&document, &schema).block(0).unwrap().clone();
        let doc = seeded_document(&source, &schema, &limits);
        {
            let mut txn = doc.transact_mut();
            txn.get_or_insert_xml_fragment("alternate");
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let alternate_fragment = txn.get_xml_fragment("alternate").unwrap();
        let seed = MutationLookupSeed::build(
            717,
            &txn,
            &fragment,
            &schema,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            4,
            9,
        )
        .unwrap();
        let mint = |txn: &yrs::Transaction<'_>,
                    fragment: &XmlFragmentRef,
                    fingerprint: &str,
                    epoch,
                    revision| {
            LocalizedFormatLocator::mint(
                &document,
                block.node_path.as_slice(),
                block.doc_start,
                block.doc_end,
                &seed,
                txn,
                fragment,
                &limits,
                &editing_limits,
                None,
                fingerprint,
                epoch,
                revision,
            )
        };
        let locator = mint(&txn, &fragment, "schema-a", 4, 9)
            .expect("exact format context must mint a locator");
        assert!(LocalizedFormatCompiler::try_new(
            717,
            &txn,
            &fragment,
            &schema,
            100_000,
            100_000,
            0,
            locator,
            "schema-a",
            4,
            9,
        )
        .unwrap()
        .is_some());

        for (case, candidate) in [
            ("differentFragment", mint(&txn, &alternate_fragment, "schema-a", 4, 9)),
            ("schema", mint(&txn, &fragment, "schema-b", 4, 9)),
            ("epoch", mint(&txn, &fragment, "schema-a", 5, 9)),
            ("revision", mint(&txn, &fragment, "schema-a", 4, 10)),
        ] {
            assert!(candidate.is_none(), "{case}");
        }
        for (case, result) in [
            (
                "differentFragment",
                LocalizedFormatCompiler::try_new(
                    717,
                    &txn,
                    &alternate_fragment,
                    &schema,
                    100_000,
                    100_000,
                    0,
                    locator,
                    "schema-a",
                    4,
                    9,
                ),
            ),
            (
                "schema",
                LocalizedFormatCompiler::try_new(
                    717,
                    &txn,
                    &fragment,
                    &schema,
                    100_000,
                    100_000,
                    0,
                    locator,
                    "schema-b",
                    4,
                    9,
                ),
            ),
            (
                "epoch",
                LocalizedFormatCompiler::try_new(
                    717,
                    &txn,
                    &fragment,
                    &schema,
                    100_000,
                    100_000,
                    0,
                    locator,
                    "schema-a",
                    5,
                    9,
                ),
            ),
            (
                "revision",
                LocalizedFormatCompiler::try_new(
                    717,
                    &txn,
                    &fragment,
                    &schema,
                    100_000,
                    100_000,
                    0,
                    locator,
                    "schema-a",
                    4,
                    10,
                ),
            ),
        ] {
            assert!(result.unwrap().is_none(), "{case}");
        }

        let foreign = seeded_document(&source, &schema, &limits);
        let foreign_txn = foreign.transact();
        let foreign_fragment = foreign_txn.get_xml_fragment("prosemirror").unwrap();
        assert!(mint(&foreign_txn, &foreign_fragment, "schema-a", 4, 9).is_none());
        assert!(LocalizedFormatCompiler::try_new(
            717,
            &foreign_txn,
            &foreign_fragment,
            &schema,
            100_000,
            100_000,
            0,
            locator,
            "schema-a",
            4,
            9,
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn mutation_lookup_seed_rejects_semantic_root_and_exact_config_drift() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = crate::yrs_engine::EditingLimits::default();
        let max_length = Some(100);
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "same" }]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let foreign = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            715,
            &txn,
            &fragment,
            &schema,
            &document,
            &limits,
            &editing_limits,
            max_length,
            "schema-a",
            1,
            2,
        )
        .unwrap();

        assert!(seed.matches_context(
            &document,
            &limits,
            &editing_limits,
            max_length
        ));
        assert!(!seed.matches_context(&foreign, &limits, &editing_limits, max_length));

        let mut resource_drift = limits.clone();
        resource_drift.max_input_bytes -= 1;
        assert!(!seed.matches_context(
            &document,
            &resource_drift,
            &editing_limits,
            max_length
        ));

        let mut editing_drift = editing_limits.clone();
        editing_drift.max_operations_per_transaction -= 1;
        assert!(!seed.matches_context(
            &document,
            &limits,
            &editing_drift,
            max_length
        ));
        assert!(!seed.matches_context(
            &document,
            &limits,
            &editing_limits,
            Some(99)
        ));
    }

    #[test]
    fn unavailable_lookup_seed_rejects_promotion_and_rebind_never_revives_it() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abc" }]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            725,
            &txn,
            &fragment,
            &schema,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            8,
            12,
        )
        .unwrap();
        assert!(seed.is_ready_for_test());
        let unavailable = seed
            .prepare_unavailable_transition(
                725,
                &txn,
                &fragment,
                &document,
                &document,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                8,
                12,
                9,
                13,
            )
            .unwrap();
        assert!(unavailable.is_unavailable_for_test());
        assert!(!unavailable.matches(
            &txn,
            &fragment,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            9,
            13,
        ));

        let promotion = MutationLookupPromotion {
            request_id: 725,
            source: MutationLookupPromotionSource::ExistingInsert,
            materialization_work_updates: Vec::new(),
            next_pending_traversal_work: 0,
        };
        let error = unavailable
            .prepare_promotion(
                &txn,
                &fragment,
                &promotion,
                &document,
                &document,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                9,
                13,
                10,
                14,
            )
            .unwrap_err();
        assert_eq!(error.request_id, 725);
        assert!(error.message.contains("unavailable"));

        let rebound = unavailable.rebind_authoritative_store(
            &txn,
            &fragment,
            "schema-a",
            10,
            14,
        );
        assert!(rebound.is_unavailable_for_test());
        assert!(!rebound.matches(
            &txn,
            &fragment,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            10,
            14,
        ));
    }

    #[test]
    fn seeded_localized_insert_treats_stale_schema_epoch_revision_and_store_as_cache_misses() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abc" }]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let block = PositionMap::build(&document, &schema).block(0).unwrap().clone();
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let seed = MutationLookupSeed::build(
            708, &txn, &fragment, &schema, &document, &limits, &editing_limits, None, "schema-a", 3, 2,
        )
        .unwrap();
        let attempt = |txn: &yrs::Transaction<'_>,
                       fragment: &XmlFragmentRef,
                       fingerprint: &str,
                       epoch,
                       revision| {
            LocalizedInsertCompiler::try_new(
                708,
                txn,
                fragment,
                &schema,
                100_000,
                100_000,
                0,
                LocalizedInsertLocator {
                    document: &document,
                    block_path: block.node_path.as_slice(),
                    position: block.doc_start + 1,
                },
                &seed,
                &limits,
                &editing_limits,
                None,
                fingerprint,
                epoch,
                revision,
            )
        };
        for result in [
            attempt(&txn, &fragment, "schema-b", 3, 2),
            attempt(&txn, &fragment, "schema-a", 4, 2),
            attempt(&txn, &fragment, "schema-a", 3, 4),
        ] {
            assert!(result.unwrap().is_none());
        }

        let foreign = seeded_document(&source, &schema, &limits);
        let foreign_txn = foreign.transact();
        let foreign_fragment = foreign_txn.get_xml_fragment("prosemirror").unwrap();
        assert!(
            attempt(&foreign_txn, &foreign_fragment, "schema-a", 3, 2)
                .unwrap()
                .is_none()
        );
    }
}
#[test]
fn import_lookup_child_count_overflow_preserves_frame_specific_diagnostics() {
    let mut structural = ImportLookupMaterializationCollector::new(
        98_001,
        BranchID::Root(Arc::from("root")),
        1,
        None,
    );
    structural.frames.last_mut().unwrap().structural_child_count = usize::MAX;
    structural.begin_fragment();
    assert_eq!(
        structural.finish().err().unwrap().message.as_ref(),
        "structural parent child count overflow"
    );

    let mut textblock = ImportLookupMaterializationCollector::new(
        98_002,
        BranchID::Root(Arc::from("root")),
        1,
        None,
    );
    assert!(textblock.begin_element(
        BranchID::Root(Arc::from("paragraph")),
        ImportElementAttributeWork::new(),
        false,
        true,
    ));
    textblock.frames.last_mut().unwrap().structural_child_count = usize::MAX;
    textblock.begin_fragment();
    assert_eq!(
        textblock.finish().err().unwrap().message.as_ref(),
        "Yrs textblock child count overflow"
    );
}
