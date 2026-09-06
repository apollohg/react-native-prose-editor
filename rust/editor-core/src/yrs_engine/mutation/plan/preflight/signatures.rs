#[derive(Debug)]
struct StructuralPreflightState {
    virtual_len: usize,
    last_index: Option<usize>,
    last_was_delete: bool,
}

#[derive(Clone, Copy)]
struct PreflightActionContext {
    request_id: u64,
    operation_index: usize,
}

#[allow(clippy::too_many_arguments)]
fn preflight_attribute_action<T: ReadTxn>(
    request_id: u64,
    operation_index: usize,
    target: &XmlElementRef,
    key: &Arc<str>,
    set_value: Option<&Any>,
    signature: &ElementSignature,
    txn: &T,
    validated_elements: &mut std::collections::HashSet<BranchID>,
    last_attribute_keys: &mut HashMap<BranchID, Arc<str>>,
    path_children: &mut HashMap<BranchID, Vec<BranchID>>,
    indexed_work: &mut usize,
) -> OperationResult<()> {
    if validated_elements.insert(signature.target.clone()) {
        let attribute_work = validate_element_signature(
            request_id,
            operation_index,
            target,
            signature,
            txn,
            path_children,
        )?;
        *indexed_work = indexed_work
            .checked_add(signature.path.len())
            .and_then(|work| work.checked_add(attribute_work))
            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    }
    if last_attribute_keys
        .get(&signature.target)
        .is_some_and(|previous| previous.as_ref() >= key.as_ref())
    {
        return Err(invalid_action_range(request_id, operation_index));
    }
    let previous = signature
        .attrs
        .binary_search_by(|(candidate, _)| candidate.as_ref().cmp(key.as_ref()))
        .ok()
        .map(|index| &signature.attrs[index].1);
    if set_value.is_some_and(|value| previous == Some(value))
        || (set_value.is_none() && previous.is_none())
    {
        return Err(invalid_action_range(request_id, operation_index));
    }
    last_attribute_keys.insert(signature.target.clone(), key.clone());
    *indexed_work = indexed_work
        .checked_add(1)
        .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    Ok(())
}

fn validate_element_signature<T: ReadTxn>(
    request_id: u64,
    operation_index: usize,
    target: &XmlElementRef,
    signature: &ElementSignature,
    txn: &T,
    path_children: &mut HashMap<BranchID, Vec<BranchID>>,
) -> OperationResult<usize> {
    let path_matches = expected_path_matches(
        signature.target.clone(),
        &signature.path,
        txn,
        path_children,
    );
    if !path_matches {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "XML attribute target path changed before execution",
        ));
    }
    let mut attrs = Vec::new();
    let mut work = 0usize;
    for (key, value) in target.attributes(txn) {
        let yrs::Out::Any(value) = value else {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                "XML attribute resolved to a non-Any shared value",
            ));
        };
        work = work
            .checked_add(key.len())
            .and_then(|work| work.checked_add(any_preflight_work(&value)?))
            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
        attrs.push((Arc::<str>::from(key), value));
    }
    let sort_partitions = binary_partition_work(attrs.len());
    let sort_key_work = attrs.iter().try_fold(0usize, |total, (key, _)| {
        total.checked_add(key.len().checked_mul(sort_partitions)?)
    });
    work = work
        .checked_add(
            attrs
                .len()
                .checked_mul(sort_partitions)
                .ok_or_else(|| invalid_action_range(request_id, operation_index))?,
        )
        .and_then(|work| work.checked_add(sort_key_work?))
        .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    attrs.sort_by(|(left, _), (right, _)| left.cmp(right));
    if AsRef::<Branch>::as_ref(target).id() != signature.target
        || target.tag().as_ref() != signature.tag.as_ref()
        || attrs != signature.attrs
    {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "XML attribute target changed before execution",
        ));
    }
    Ok(work)
}

pub(super) fn any_preflight_work(root: &Any) -> Option<usize> {
    let mut work = 0usize;
    let mut stack = vec![root];
    while let Some(value) = stack.pop() {
        work = work.checked_add(1)?;
        match value {
            Any::String(value) => work = work.checked_add(value.len())?,
            Any::Buffer(value) => work = work.checked_add(value.len())?,
            Any::Array(values) => {
                work = work.checked_add(values.len())?;
                stack.extend(values.iter());
            }
            Any::Map(values) => {
                work = work.checked_add(values.len())?;
                for (key, value) in values.iter() {
                    work = work.checked_add(key.len())?;
                    stack.push(value);
                }
            }
            Any::Null | Any::Undefined | Any::Bool(_) | Any::Number(_) | Any::BigInt(_) => {}
        }
    }
    Some(work)
}

pub(super) fn capture_text_signature<T: ReadTxn>(
    request_id: u64,
    operation_index: Option<usize>,
    target: &XmlTextRef,
    txn: &T,
) -> OperationResult<(Vec<TextSignatureRun>, usize)> {
    let mut runs = Vec::<TextSignatureRun>::new();
    let mut work = 0usize;
    for diff in target.diff(txn, yrs::types::text::YChange::identity) {
        let yrs::Out::Any(Any::String(value)) = diff.insert else {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                operation_index,
                "Yrs XML text signature contains a non-string value",
            ));
        };
        let text = value.to_string();
        if text.is_empty() {
            continue;
        }
        let mut attrs = Vec::new();
        if let Some(diff_attrs) = diff.attributes {
            work = work
                .checked_add(diff_attrs.len())
                .ok_or_else(|| text_signature_overflow(request_id, operation_index))?;
            for (key, value) in diff_attrs.iter() {
                work = work
                    .checked_add(key.len())
                    .and_then(|work| work.checked_add(any_preflight_work(value)?))
                    .ok_or_else(|| text_signature_overflow(request_id, operation_index))?;
                attrs.push((key.clone(), value.clone()));
            }
            let sort_partitions = binary_partition_work(attrs.len());
            let sort_key_work = attrs.iter().try_fold(0usize, |total, (key, _)| {
                total.checked_add(key.len().checked_mul(sort_partitions)?)
            });
            let sort_work = attrs
                .len()
                .checked_mul(sort_partitions)
                .and_then(|work| work.checked_add(sort_key_work?))
                .ok_or_else(|| text_signature_overflow(request_id, operation_index))?;
            work = work
                .checked_add(sort_work)
                .ok_or_else(|| text_signature_overflow(request_id, operation_index))?;
            attrs.sort_by(|(left, _), (right, _)| left.cmp(right));
        }
        work = work
            .checked_add(text.len())
            .and_then(|work| work.checked_add(1))
            .ok_or_else(|| text_signature_overflow(request_id, operation_index))?;
        if let Some(previous) = runs.last_mut().filter(|run| run.attrs == attrs) {
            previous.text.push_str(&text);
        } else {
            runs.push(TextSignatureRun { text, attrs });
        }
    }
    Ok((runs, work))
}

fn text_signature_work(runs: &[TextSignatureRun]) -> Option<usize> {
    runs.iter().try_fold(0usize, |work, run| {
        run.attrs.iter().try_fold(
            work.checked_add(1)?.checked_add(run.text.len())?,
            |work, (key, value)| {
                work.checked_add(key.len())?
                    .checked_add(any_preflight_work(value)?)
            },
        )
    })
}

fn text_signature_overflow(request_id: u64, operation_index: Option<usize>) -> OperationError {
    OperationError::engine_invariant_failed(
        request_id,
        operation_index,
        "Yrs XML text signature work overflow",
    )
}

fn validate_structural_parent<'a, T: ReadTxn>(
    context: PreflightActionContext,
    parent: &XmlParentRef,
    signature: &StructuralParentSignature,
    txn: &T,
    parents: &'a mut HashMap<BranchID, StructuralPreflightState>,
    path_children: &mut HashMap<BranchID, Vec<BranchID>>,
    indexed_work: &mut usize,
) -> OperationResult<&'a mut StructuralPreflightState> {
    let PreflightActionContext {
        request_id,
        operation_index,
    } = context;
    if !parents.contains_key(&signature.parent) {
        // Resolve the recorded parent chain from stable BranchIDs before
        // deriving a live branch ID. Detached/GC'd branches are absent from
        // the store and never reach the potentially-panicking id() call.
        if !expected_path_matches(
            signature.parent.clone(),
            &signature.path,
            txn,
            path_children,
        ) || parent.id() != signature.parent
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                "structural mutation parent identity changed before execution",
            ));
        }
        let actual = parent.children(txn);
        *indexed_work = indexed_work
            .checked_add(signature.path.len())
            .and_then(|work| work.checked_add(actual.len()))
            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
        if actual != signature.children {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                "structural mutation parent children changed before execution",
            ));
        }
        parents.insert(
            signature.parent.clone(),
            StructuralPreflightState {
                virtual_len: actual.len(),
                last_index: None,
                last_was_delete: false,
            },
        );
    }
    parents
        .get_mut(&signature.parent)
        .ok_or_else(|| invalid_action_range(request_id, operation_index))
}

#[cfg(test)]
pub(crate) fn preflight_mutation_work_for_test<T: ReadTxn>(
    _request_id: u64,
    plan: &YrsMutationPlan,
    _txn: &T,
) -> OperationResult<usize> {
    Ok(plan.expected_preflight_work)
}

#[allow(dead_code)] // calls this after installing atomic production application.
fn validate_signature<T: ReadTxn>(
    request_id: u64,
    operation_index: usize,
    target: &XmlTextRef,
    expected: &TargetSignature,
    txn: &T,
    path_children: &mut std::collections::HashMap<BranchID, Vec<BranchID>>,
) -> OperationResult<usize> {
    let branch = <XmlTextRef as AsRef<Branch>>::as_ref(target);
    let path_matches =
        expected_path_matches(expected.target.clone(), &expected.path, txn, path_children);
    if !path_matches {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved Yrs XML text target path changed before mutation",
        ));
    }
    let actual_len = Some(Text::len(target, txn));
    let (actual_runs, actual_work) =
        capture_text_signature(request_id, Some(operation_index), target, txn)?;
    let compare_work = text_signature_work(&expected.runs)
        .and_then(|work| work.checked_add(actual_work))
        .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    if branch.is_deleted()
        || branch.id() != expected.target
        || actual_len != Some(expected.initial_len_utf16)
        || actual_runs != expected.runs
    {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            format!(
                "resolved Yrs XML text target signature changed before mutation (deleted={}, id_match={}, path_match={}, content_match={}, expected_utf16={}, actual_len={actual_len:?})",
                branch.is_deleted(),
                branch.id() == expected.target,
                path_matches,
                actual_runs == expected.runs,
                expected.initial_len_utf16,
            ),
        ));
    }
    Ok(compare_work)
}

fn validate_parent_identity<T: ReadTxn>(
    request_id: u64,
    operation_index: usize,
    parent: &XmlElementRef,
    expected: &ParentSignature,
    txn: &T,
    path_children: &mut std::collections::HashMap<BranchID, Vec<BranchID>>,
) -> OperationResult<()> {
    let branch = <XmlElementRef as AsRef<Branch>>::as_ref(parent);
    let path_matches =
        expected_path_matches(expected.parent.clone(), &expected.path, txn, path_children);
    if !path_matches {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved empty Yrs textblock path changed before mutation",
        ));
    }
    if branch.is_deleted() || branch.id() != expected.parent || parent.tag() != &expected.tag {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved empty Yrs textblock signature changed before mutation",
        ));
    }
    path_children
        .entry(branch.id())
        .or_insert_with(|| parent.children(txn).map(|child| child.id()).collect());
    Ok(())
}

fn validate_parent_gap(
    request_id: u64,
    operation_index: usize,
    expected: &ParentSignature,
    children: &[BranchID],
) -> OperationResult<()> {
    let actual_child_count = u32::try_from(children.len()).ok();
    let left_neighbor = expected
        .initial_child_index
        .checked_sub(1)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| children.get(index))
        .cloned();
    let right_neighbor = usize::try_from(expected.initial_child_index)
        .ok()
        .and_then(|index| children.get(index))
        .cloned();
    if actual_child_count != Some(expected.child_count)
        || expected.initial_child_index > expected.child_count
        || left_neighbor != expected.left_neighbor
        || right_neighbor != expected.right_neighbor
    {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved empty Yrs textblock gap signature changed before mutation",
        ));
    }
    Ok(())
}

pub(super) fn invalid_action_range(request_id: u64, operation_index: usize) -> OperationError {
    OperationError::engine_invariant_failed(
        request_id,
        Some(operation_index),
        "resolved Yrs mutation action is outside its preflighted UTF-16 target range",
    )
}

fn expected_path_matches<T: ReadTxn>(
    mut child_id: BranchID,
    expected: &[(BranchID, u32)],
    txn: &T,
    path_children: &mut std::collections::HashMap<BranchID, Vec<BranchID>>,
) -> bool {
    for (expected_parent, expected_index) in expected {
        let node = match expected_parent {
            // A decoded update may retain an undefined root type internally;
            // `get_xml_fragment` performs the same safe XML reinterpretation
            // used by the engine/compiler boundary.
            BranchID::Root(name) => txn.get_xml_fragment(name.clone()).map(XmlOut::Fragment),
            BranchID::Nested(_) => expected_parent
                .get_branch(txn)
                .and_then(|parent| XmlOut::try_from(parent).ok()),
        };
        let Some(node) = node else { return false };
        let children = path_children
            .entry(node.id())
            .or_insert_with(|| match &node {
                XmlOut::Element(element) => element.children(txn).map(|child| child.id()).collect(),
                XmlOut::Fragment(fragment) => {
                    fragment.children(txn).map(|child| child.id()).collect()
                }
                XmlOut::Text(_) => Vec::new(),
            });
        let expected_index = match usize::try_from(*expected_index) {
            Ok(index) => index,
            Err(_) => return false,
        };
        if children.get(expected_index) != Some(&child_id) {
            return false;
        }
        child_id = node.id();
    }
    matches!(child_id, BranchID::Root(_))
}
