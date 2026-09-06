#[test]
fn update_node_attrs_toggles_and_removes_task_item_default() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "todo" }] }]
            }]
        }]
    });
    let (actual, expected) = compile_and_execute_attribute_update(
        source,
        HashMap::from([("checked".into(), Value::Bool(true))]),
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["attrs"]["checked"], true);

    let checked_source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "attrs": { "checked": true },
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "done" }] }]
            }]
        }]
    });
    let (removed, removed_expected) =
        compile_and_execute_attribute_update(checked_source, HashMap::new());
    assert_eq!(removed, removed_expected);
    assert!(removed["content"][0]["content"][0]["attrs"]["checked"].is_null());
}

#[test]
fn update_node_attrs_sets_and_removes_image_attributes() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "image", "attrs": { "src": "old", "alt": "old alt" } }]
    });
    let (actual, expected) = compile_and_execute_attribute_update(
        source,
        HashMap::from([("src".into(), Value::String("new".into()))]),
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["attrs"]["src"], "new");
    assert!(actual["content"][0]["attrs"]["alt"].is_null());
}

#[test]
fn update_node_attrs_preserves_nested_custom_any_values() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "customBlock", "attrs": { "old": true } }]
    });
    let attrs = HashMap::from([
        ("flag".into(), Value::Bool(true)),
        ("count".into(), json!(7)),
        ("label".into(), Value::String("custom".into())),
        ("items".into(), json!([1, false, "x"])),
        ("meta".into(), json!({ "nested": { "ok": true } })),
    ]);
    let (actual, expected) = compile_and_execute_attribute_update(source, attrs);
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"][0]["attrs"]["items"],
        json!([1, false, "x"])
    );
    assert_eq!(
        actual["content"][0]["attrs"]["meta"],
        json!({ "nested": { "ok": true } })
    );
}

#[test]
fn update_node_attrs_normalizes_sequential_same_key_changes() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "customBlock", "attrs": { "old": true } }]
    });
    let (_, _, _, compiled) = compile_attribute_operations(
        source.clone(),
        vec![
            HashMap::from([("label".into(), Value::String("first".into()))]),
            HashMap::from([("label".into(), Value::String("final".into()))]),
        ],
    );
    assert_eq!(compiled.mutation_plan.actions.len(), 2);
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::SetXmlAttribute { key, value: Any::String(value), .. },
            YrsMutationAction::RemoveXmlAttribute { key: removed, .. }
        ] if key.as_ref() == "label" && value.as_ref() == "final" && removed.as_ref() == "old"
    ));

    let (_, _, _, removed) = compile_attribute_operations(
        source,
        vec![
            HashMap::from([("label".into(), Value::String("temporary".into()))]),
            HashMap::new(),
        ],
    );
    assert!(matches!(
        removed.mutation_plan.actions.as_slice(),
        [YrsMutationAction::RemoveXmlAttribute { key, .. }] if key.as_ref() == "old"
    ));
}

#[test]
fn update_node_attrs_identical_map_is_a_complete_no_op() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "customBlock", "attrs": { "flag": true } }]
    });
    let (_, _, _, compiled) = compile_attribute_operations(
        source,
        vec![HashMap::from([("flag".into(), Value::Bool(true))])],
    );
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
    assert_eq!(compiled.undo_units_bound, 0);
}

#[test]
fn update_node_attrs_rejects_stale_same_count_attribute_substitution_atomically() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "image", "attrs": { "src": "old", "alt": "old alt" } }]
    });
    let (doc, _, _, compiled) = compile_attribute_operations(
        source,
        vec![HashMap::from([("src".into(), Value::String("new".into()))])],
    );
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(image) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected image")
        };
        image.insert_attribute(&mut txn, "src", Any::String("raced".into()));
    }
    let before = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let error = {
        let txn = doc.transact();
        preflight_mutation_plan(118, &compiled.mutation_plan, &txn).unwrap_err()
    };
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(
        doc.transact()
            .encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn update_node_attrs_keeps_heading_synthetic_level_unchanged() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "attrs": { "id": "old" },
            "content": [{ "type": "text", "text": "Heading" }]
        }]
    });
    let (_, _, _, compiled) = compile_attribute_operations(
        source,
        vec![HashMap::from([("id".into(), Value::String("new".into()))])],
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [YrsMutationAction::SetXmlAttribute { key, .. }] if key.as_ref() == "id"
    ));
}

#[test]
fn update_node_attrs_rejects_ambiguous_attrless_target() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }]
    });
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, attribute_schema());
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let error = compile_transaction_with_yrs(
        CompilationContext {
            document: &document,
            selection: None,
            schema: &schema,
            resource_limits: &limits,
            editing_limits: &editing_limits,
            document_revision: 0,
            max_length: None,
        },
        TypedTransaction {
            request_id: 119,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::UpdateNodeAttrs {
                at: point_for_test(0),
                attrs: HashMap::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap_err();
    assert_eq!(error.code, "POSITION_INVALID");
    assert_eq!(error.details.as_ref().unwrap()["field"], "at");
}

fn compile_and_execute_attribute_update(
    source: Value,
    attrs: HashMap<String, Value>,
) -> (Value, Value) {
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, attribute_schema());
    let (before_ids, before_full_len, sticky) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let sticky = first_xml_text(&fragment, &txn).and_then(|text| {
            StickyIndex::at(
                &txn,
                BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&text)),
                0,
                Assoc::After,
            )
        });
        (
            collect_xml_ids(&fragment, &txn),
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
            sticky,
        )
    };
    let mut compiled = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits: &limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 117,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::UpdateNodeAttrs {
                    at: point_for_test(0),
                    attrs,
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let keys = compiled
        .mutation_plan
        .actions
        .iter()
        .filter_map(|action| match action {
            YrsMutationAction::SetXmlAttribute { key, .. }
            | YrsMutationAction::RemoveXmlAttribute { key, .. } => Some(key.as_ref()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(keys.len(), compiled.mutation_plan.actions.len());
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(117, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(117, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(117, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let has_actions = !compiled.mutation_plan.actions.is_empty();
    let update = if has_actions {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    } else {
        Vec::new()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(collect_xml_ids(&fragment, &txn), before_ids);
    if let Some(sticky) = sticky {
        assert!(sticky.get_offset(&txn).is_some());
    }
    let update_len = update.len();
    assert!(update_len <= estimate, "{update_len} > {estimate}");
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
    (
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected,
    )
}

fn compile_attribute_operations(
    source: Value,
    updates: Vec<HashMap<String, Value>>,
) -> (
    Doc,
    crate::schema::Schema,
    ResourceLimits,
    CompiledTransaction,
) {
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, attribute_schema());
    let compiled = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits: &limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 118,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: updates
                    .into_iter()
                    .map(|attrs| TypedOperation::UpdateNodeAttrs {
                        at: point_for_test(0),
                        attrs,
                    })
                    .collect(),
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    (doc, schema, limits, compiled)
}

fn collect_xml_ids<T: ReadTxn>(
    fragment: &yrs::types::xml::XmlFragmentRef,
    txn: &T,
) -> Vec<yrs::branch::BranchID> {
    fn visit<T: ReadTxn>(out: XmlOut, txn: &T, ids: &mut Vec<yrs::branch::BranchID>) {
        ids.push(out.id());
        match out {
            XmlOut::Element(element) => {
                for child in element.children(txn) {
                    visit(child, txn, ids);
                }
            }
            XmlOut::Fragment(fragment) => {
                for child in fragment.children(txn) {
                    visit(child, txn, ids);
                }
            }
            XmlOut::Text(_) => {}
        }
    }
    let mut ids = Vec::new();
    for child in fragment.children(txn) {
        visit(child, txn, &mut ids);
    }
    ids
}

fn first_xml_text<T: ReadTxn>(
    fragment: &yrs::types::xml::XmlFragmentRef,
    txn: &T,
) -> Option<XmlTextRef> {
    fn visit<T: ReadTxn>(out: XmlOut, txn: &T) -> Option<XmlTextRef> {
        match out {
            XmlOut::Text(text) => Some(text),
            XmlOut::Element(element) => element.children(txn).find_map(|child| visit(child, txn)),
            XmlOut::Fragment(fragment) => {
                fragment.children(txn).find_map(|child| visit(child, txn))
            }
        }
    }
    fragment.children(txn).find_map(|child| visit(child, txn))
}
