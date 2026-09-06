#[cfg(test)]
fn legacy_build_lookup_seed_payload<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    fragment: &XmlFragmentRef,
    schema: &Schema,
    target_capacity_hint: Option<usize>,
) -> OperationResult<MutationLookupPayload> {
    fn reserve_entry<K: Eq + std::hash::Hash, V>(
        request_id: u64,
        map: &mut HashMap<K, V>,
    ) -> OperationResult<()> {
        if map.len() < map.capacity() {
            return Ok(());
        }
        #[cfg(test)]
        LOOKUP_SEED_MAP_GROWTH_ATTEMPT_COUNT
            .set(LOOKUP_SEED_MAP_GROWTH_ATTEMPT_COUNT.get().saturating_add(1));
        if lookup_seed_hydration_should_fail("mapGrowth") {
            return Err(lookup_seed_allocation_error(request_id, "mapGrowth"));
        }
        map.try_reserve(1)
            .map_err(|_| lookup_seed_allocation_error(request_id, "mapGrowth"))
    }

    fn measure_element_attributes<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        element: &XmlElementRef,
        work: &mut usize,
    ) -> OperationResult<()> {
        let mut attr_count = 0usize;
        let mut key_bytes = 0usize;
        for (key, value) in element.attributes(txn) {
            let yrs::Out::Any(value) = value else {
                return Err(OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "XML element attribute resolved to a non-Any shared value",
                ));
            };
            attr_count = attr_count.checked_add(1).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "XML attribute traversal work overflow",
                )
            })?;
            key_bytes = key_bytes.checked_add(key.len()).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "XML attribute traversal work overflow",
                )
            })?;
            *work = work
                .checked_add(key.len())
                .and_then(|work| work.checked_add(any_traversal_work(&value)?))
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "XML attribute traversal work overflow",
                    )
                })?;
        }
        let partitions = binary_partition_work(attr_count);
        let sort_work = attr_count
            .checked_mul(partitions)
            .and_then(|work| work.checked_add(key_bytes.checked_mul(partitions)?))
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "XML attribute sort work overflow",
                )
            })?;
        *work = work.checked_add(sort_work).ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "XML attribute sort work overflow",
            )
        })?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn measure_text<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        schema: &Schema,
        children: impl Iterator<Item = XmlOut>,
        ancestor_depth: usize,
        structural_parent: Option<(BranchID, usize)>,
        work: &mut usize,
        target_count: &mut usize,
        target_work: &mut HashMap<BranchID, usize>,
        widths: &mut HashMap<BranchID, usize>,
    ) -> OperationResult<()> {
        let mut structural_child_count = 0usize;
        for child in children {
            structural_child_count = structural_child_count.checked_add(1).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "structural parent child count overflow",
                )
            })?;
            let path_len = ancestor_depth.checked_add(1).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "Yrs mutation path capture work overflow",
                )
            })?;
            *work = work
                .checked_add(1)
                .and_then(|work| work.checked_add(path_len))
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "Yrs mutation target traversal work overflow",
                    )
                })?;
            match child {
                XmlOut::Text(text) => {
                    *work = work.checked_add(path_len).ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "Yrs text signature preflight work overflow",
                        )
                    })?;
                    let capture_work = measure_text_capture_work(request_id, &text, txn)?;
                    *work = work.checked_add(capture_work).ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "Yrs text materialization work overflow",
                        )
                    })?;
                    let target = AsRef::<Branch>::as_ref(&text).id();
                    reserve_entry(request_id, target_work)?;
                    if target_work.insert(target, capture_work).is_some() {
                        return Err(OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "duplicate Yrs text materialization",
                        ));
                    }
                    *target_count = target_count.checked_add(1).ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "Yrs mutation target count overflow",
                        )
                    })?;
                }
                XmlOut::Element(element) => {
                    measure_element_attributes(request_id, txn, &element, work)?;
                    let (is_void, is_textblock) = wire_element_semantics(&element, txn, schema);
                    if is_void {
                        continue;
                    }
                    if is_textblock {
                        let mut child_count = 0usize;
                        let mut previous_was_text = false;
                        for child in element.children(txn) {
                            child_count = child_count.checked_add(1).ok_or_else(|| {
                                OperationError::engine_invariant_failed(
                                    request_id,
                                    None,
                                    "Yrs textblock child count overflow",
                                )
                            })?;
                            let child_is_text = matches!(child, XmlOut::Text(_));
                            if !previous_was_text && !child_is_text {
                                *work = work.checked_add(path_len).ok_or_else(|| {
                                    OperationError::engine_invariant_failed(
                                        request_id,
                                        None,
                                        "Yrs missing-gap signature work overflow",
                                    )
                                })?;
                                *target_count = target_count.checked_add(1).ok_or_else(|| {
                                    OperationError::engine_invariant_failed(
                                        request_id,
                                        None,
                                        "Yrs mutation target count overflow",
                                    )
                                })?;
                            }
                            measure_text(
                                request_id,
                                txn,
                                schema,
                                std::iter::once(child),
                                path_len,
                                None,
                                work,
                                target_count,
                                target_work,
                                widths,
                            )?;
                            previous_was_text = child_is_text;
                        }
                        *work = work
                            .checked_add(child_count)
                            .and_then(|work| work.checked_add(child_count))
                            .and_then(|work| work.checked_add(path_len))
                            .ok_or_else(|| {
                                OperationError::engine_invariant_failed(
                                    request_id,
                                    None,
                                    "Yrs textblock materialization work overflow",
                                )
                            })?;
                        if !previous_was_text {
                            *work = work.checked_add(path_len).ok_or_else(|| {
                                OperationError::engine_invariant_failed(
                                    request_id,
                                    None,
                                    "Yrs missing-gap signature work overflow",
                                )
                            })?;
                            *target_count = target_count.checked_add(1).ok_or_else(|| {
                                OperationError::engine_invariant_failed(
                                    request_id,
                                    None,
                                    "Yrs mutation target count overflow",
                                )
                            })?;
                        }
                        *work = work
                            .checked_add(1)
                            .and_then(|work| work.checked_add(path_len))
                            .and_then(|work| work.checked_add(child_count))
                            .ok_or_else(|| {
                                OperationError::engine_invariant_failed(
                                    request_id,
                                    None,
                                    "structural parent traversal work overflow",
                                )
                            })?;
                        reserve_entry(request_id, widths)?;
                        if widths
                            .insert(AsRef::<Branch>::as_ref(&element).id(), child_count)
                            .is_some()
                        {
                            return Err(OperationError::engine_invariant_failed(
                                request_id,
                                None,
                                "duplicate Yrs structural parent",
                            ));
                        }
                    } else {
                        measure_text(
                            request_id,
                            txn,
                            schema,
                            element.children(txn),
                            path_len,
                            Some((AsRef::<Branch>::as_ref(&element).id(), path_len)),
                            work,
                            target_count,
                            target_work,
                            widths,
                        )?;
                    }
                }
                XmlOut::Fragment(fragment) => measure_text(
                    request_id,
                    txn,
                    schema,
                    fragment.children(txn),
                    path_len,
                    None,
                    work,
                    target_count,
                    target_work,
                    widths,
                )?,
            }
        }
        if let Some((parent_id, branch_depth)) = structural_parent {
            *work = work
                .checked_add(1)
                .and_then(|work| work.checked_add(branch_depth))
                .and_then(|work| work.checked_add(structural_child_count))
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "structural parent traversal work overflow",
                    )
                })?;
            reserve_entry(request_id, widths)?;
            if widths.insert(parent_id, structural_child_count).is_some() {
                return Err(OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "duplicate Yrs structural parent",
                ));
            }
        }
        Ok(())
    }

    let root_width = usize::try_from(fragment.len(txn)).map_err(|_| {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "Yrs fragment child count exceeds usize",
        )
    })?;
    let seed_capacity = root_width.saturating_mul(2).saturating_add(1);
    let mut pending_traversal_work = 0usize;
    let mut target_count = 0usize;
    let target_capacity =
        target_capacity_hint.map_or(seed_capacity, |hint| hint.max(seed_capacity));
    #[cfg(test)]
    let force_map_growth = lookup_seed_hydration_should_fail("mapGrowth");
    #[cfg(not(test))]
    let force_map_growth = false;
    let initial_target_capacity = if force_map_growth { 0 } else { target_capacity };
    let initial_width_capacity = if force_map_growth { 0 } else { seed_capacity };
    let mut target_materialization_work = HashMap::new();
    let mut path_parent_widths = HashMap::new();
    if lookup_seed_hydration_should_fail("initialReservation") {
        return Err(lookup_seed_allocation_error(
            request_id,
            "initialReservation",
        ));
    }
    target_materialization_work
        .try_reserve(initial_target_capacity)
        .map_err(|_| lookup_seed_allocation_error(request_id, "initialReservation"))?;
    path_parent_widths
        .try_reserve(initial_width_capacity)
        .map_err(|_| lookup_seed_allocation_error(request_id, "initialReservation"))?;
    measure_text(
        request_id,
        txn,
        schema,
        fragment.children(txn),
        0,
        Some((AsRef::<Branch>::as_ref(fragment).id(), 0)),
        &mut pending_traversal_work,
        &mut target_count,
        &mut target_materialization_work,
        &mut path_parent_widths,
    )?;
    probe_lookup_seed_publication(
        request_id,
        "mapPublication",
        std::mem::size_of::<HashMap<BranchID, usize>>(),
    )?;
    let path_parent_widths = Arc::new(path_parent_widths);
    probe_lookup_seed_publication(
        request_id,
        "mapPublication",
        std::mem::size_of::<HashMap<BranchID, usize>>(),
    )?;
    let target_materialization_work = Arc::new(target_materialization_work);
    Ok(MutationLookupPayload {
        target_count,
        pending_traversal_work,
        path_parent_widths,
        target_materialization_work,
    })
}

#[cfg(test)]
pub(crate) fn lookup_payload_legacy_parity_for_test<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    schema: &Schema,
) -> bool {
    let Ok(current) = build_lookup_seed_payload(91_001, txn, fragment, schema, None) else {
        return false;
    };
    let Ok(legacy) = legacy_build_lookup_seed_payload(91_002, txn, fragment, schema, None) else {
        return false;
    };
    current.target_count == legacy.target_count
        && current.pending_traversal_work == legacy.pending_traversal_work
        && current.path_parent_widths == legacy.path_parent_widths
        && current.target_materialization_work == legacy.target_materialization_work
}
