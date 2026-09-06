impl<'a> LocalizedFormatLocator<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mint<T: ReadTxn>(
        document: &'a Document,
        block_path: &'a [u32],
        from: u32,
        to: u32,
        seed: &'a MutationLookupSeed,
        txn: &T,
        fragment: &XmlFragmentRef,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> Option<Self> {
        seed.matches(
            txn,
            fragment,
            document,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
        )
        .then_some(Self {
            document,
            block_path,
            from,
            to,
            seed,
        })
    }
}

impl<'a> LocalizedRootWindowLocator<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mint<T: ReadTxn>(
        request_id: u64,
        document: &'a Document,
        expected_preview: &'a Document,
        replacement: &super::super::StructuralReplacement,
        seed: &'a MutationLookupSeed,
        txn: &T,
        fragment: &XmlFragmentRef,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<Option<Self>> {
        let (from_child, to_child) = replacement.child_window();
        let Some(root) = document.root().content() else {
            return Ok(None);
        };
        let Ok(root_len) = u32::try_from(root.child_count()) else {
            return Ok(None);
        };
        if !replacement.parent_path().is_empty()
            || from_child >= to_child
            || to_child > root_len
            || !seed.matches(
                txn,
                fragment,
                document,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            )
        {
            return Ok(None);
        }
        let mut expected_children = Vec::new();
        expected_children
            .try_reserve_exact(replacement.content().child_count())
            .map_err(|_| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(0),
                    "localized root-window content allocation failed",
                )
            })?;
        expected_children.extend(replacement.content().iter().cloned());
        Ok(Some(Self {
            document,
            expected_preview,
            from_child,
            to_child,
            expected_content: Fragment::from(expected_children),
            seed,
        }))
    }
}

fn localized_root_structural_parent<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    fragment: &XmlFragmentRef,
    document: &Document,
    schema: &Schema,
    resource_limits: &ResourceLimits,
) -> OperationResult<Option<StructuralParentTarget>> {
    let Some(semantic_children) = document.root().content() else {
        return Ok(None);
    };
    let child_count = usize::try_from(fragment.len(txn)).map_err(|_| {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "localized root child count exceeds usize",
        )
    })?;
    if child_count != semantic_children.child_count() {
        return Ok(None);
    }
    let parent_id = AsRef::<Branch>::as_ref(fragment).id();
    let mut storage_children = Vec::new();
    storage_children.try_reserve(child_count).map_err(|_| {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "localized root-window allocation failed",
        )
    })?;
    let mut child_ids = Vec::new();
    child_ids.try_reserve_exact(child_count).map_err(|_| {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "localized root signature allocation failed",
        )
    })?;
    for (index, (wire, semantic)) in fragment
        .children(txn)
        .zip(semantic_children.iter())
        .enumerate()
    {
        child_ids.push(wire.id());
        let XmlOut::Element(element) = wire else {
            return Ok(None);
        };
        let tag = element.tag();
        let wire_spec = super::super::codec::wire_element_node_spec(&element, txn, schema);
        let wire_node_type = wire_spec.map_or(tag.as_ref(), |spec| spec.name.as_str());
        if semantic.is_text()
            || semantic.node_type() != wire_node_type
            || semantic.is_void() != wire_element_is_semantic_void(&element, txn, schema)
        {
            return Ok(None);
        }
        let mut attrs = Vec::new();
        let mut normalized_attr_count = 0usize;
        let projection = wire_spec.and_then(|spec| spec.json_projection.as_ref());
        let synthetic_heading_level = projection.is_none() && wire_node_type != tag.as_ref();
        let mut attribute_budget =
            super::super::codec::WireAttributeJsonBudget::new(resource_limits);
        for (key, value) in element.attributes(txn) {
            let yrs::Out::Any(value) = value else {
                return Ok(None);
            };
            if !projection.is_some_and(|projection| projection.attrs.contains_key(key))
                && !(synthetic_heading_level && key == "level")
            {
                let Some(expected) = semantic.attrs().get(key) else {
                    return Ok(None);
                };
                let Ok(actual) = attribute_budget.convert(&value) else {
                    return Ok(None);
                };
                if &actual != expected {
                    return Ok(None);
                }
                normalized_attr_count = normalized_attr_count.checked_add(1).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "localized root normalized attribute count overflow",
                    )
                })?;
            }
            attrs.try_reserve(1).map_err(|_| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "localized root attribute allocation failed",
                )
            })?;
            attrs.push((Arc::<str>::from(key), value));
        }
        if normalized_attr_count != semantic.attrs().len() {
            return Ok(None);
        }
        attrs.sort_by(|(left, _), (right, _)| left.cmp(right));
        let child_index = u32::try_from(index).map_err(|_| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "localized root child index exceeds u32",
            )
        })?;
        let mut path = Vec::new();
        path.try_reserve_exact(1).map_err(|_| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "localized root path allocation failed",
            )
        })?;
        path.push((parent_id.clone(), child_index));
        storage_children.push(StorageChildKind::Element {
            target: element.clone(),
            signature: Arc::new(ElementSignature {
                target: AsRef::<Branch>::as_ref(&element).id(),
                path,
                tag: element.tag().clone(),
                attrs,
            }),
        });
    }
    Ok(Some(StructuralParentTarget {
        parent: XmlParentRef::Fragment(fragment.clone()),
        signature: Arc::new(StructuralParentSignature {
            parent: parent_id,
            path: Vec::new(),
            children: child_ids,
        }),
        storage_children,
    }))
}
