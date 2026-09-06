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

    #[test]
    fn projected_wire_elements_use_their_native_semantics() {
        let schema = projected_textblock_test_schema();
        let doc = utf16_doc();
        let element = {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let element = fragment.push_back(&mut txn, XmlElementPrelim::empty("callout"));
            element.insert_attribute(&mut txn, "tone", Any::String("info".into()));
            element
        };
        let txn = doc.transact();

        assert_eq!(
            wire_element_semantics(&element, &txn, &schema),
            (false, true)
        );
    }

    #[test]
    fn projected_prepared_textblocks_materialize_an_empty_text_target() {
        let schema = projected_textblock_test_schema();
        let mut nodes = vec![PreparedXmlChild {
            index: 0,
            node: PreparedXmlNode::Element {
                tag: "callout".into(),
                attrs: vec![("tone".into(), Any::String("info".into()))],
                children: Vec::new(),
            },
        }];

        materialize_empty_prepared_textblocks(&mut nodes, &schema);

        let PreparedXmlNode::Element { children, .. } = &nodes[0].node else {
            panic!("projected element expected")
        };
        assert!(matches!(
            children.as_slice(),
            [PreparedXmlChild {
                node: PreparedXmlNode::Text { .. },
                ..
            }]
        ));
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
