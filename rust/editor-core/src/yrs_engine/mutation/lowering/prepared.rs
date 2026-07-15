enum PreparedStructuralInsertion {
    Boundary(usize),
    InsideText {
        child_index: usize,
        local_scalar: u32,
    },
}
fn prepared_structural_insertion(
    semantic_children: &Fragment,
    storage_children: &[PreparedXmlChild],
    position: u32,
) -> Option<PreparedStructuralInsertion> {
    let mut semantic_index = 0usize;
    let mut offset = 0u32;
    for (storage_index, storage) in storage_children.iter().enumerate() {
        if position == offset {
            return Some(PreparedStructuralInsertion::Boundary(storage_index));
        }
        match &storage.node {
            PreparedXmlNode::Text { runs } => {
                let scalar_len = u32::try_from(prepared_runs_text(runs).chars().count()).ok()?;
                let end = offset.checked_add(scalar_len)?;
                if position < end {
                    return Some(PreparedStructuralInsertion::InsideText {
                        child_index: storage_index,
                        local_scalar: position.checked_sub(offset)?,
                    });
                }
                while offset < end {
                    let child = semantic_children.child(semantic_index)?;
                    if !child.is_text() || child.node_size() > end.checked_sub(offset)? {
                        return None;
                    }
                    offset = offset.checked_add(child.node_size())?;
                    semantic_index += 1;
                }
            }
            PreparedXmlNode::Element { .. } => {
                let child = semantic_children.child(semantic_index)?;
                if child.is_text() {
                    return None;
                }
                offset = offset.checked_add(child.node_size())?;
                semantic_index += 1;
            }
        }
    }
    (position == offset).then_some(PreparedStructuralInsertion::Boundary(
        storage_children.len(),
    ))
}

struct ResolvedSpan {
    target: usize,
    from_scalar: u32,
    to_scalar: u32,
    index_utf16: u32,
    len_utf16: u32,
}

struct ResolvedInsertion {
    target_index: usize,
    scalar_index: u32,
}

fn prepared_runs_text(runs: &[PreparedTextRun]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

fn semantic_insertion_index(children: &Fragment, position: u32) -> Option<u32> {
    let mut offset = 0u32;
    for (index, child) in children.iter().enumerate() {
        if position == offset {
            return u32::try_from(index).ok();
        }
        let end = offset.checked_add(child.node_size())?;
        if position < end {
            if !child.is_text() {
                return None;
            }
            return u32::try_from(index.checked_add(1)?).ok();
        }
        offset = end;
    }
    (position == offset)
        .then(|| u32::try_from(children.child_count()).ok())
        .flatten()
}

fn prepared_clone_work(nodes: &[PreparedXmlChild]) -> Option<usize> {
    fn node_work(node: &PreparedXmlNode) -> Option<usize> {
        match node {
            PreparedXmlNode::Text { runs } => runs.iter().try_fold(1usize, |total, run| {
                total
                    .checked_add(run.text.len())?
                    .checked_add(attrs_work(&run.attrs))
            }),
            PreparedXmlNode::Element {
                tag,
                attrs,
                children,
            } => {
                let total = attrs.iter().try_fold(
                    1usize.checked_add(tag.len())?,
                    |total, (key, value)| {
                        total
                            .checked_add(key.len())?
                            .checked_add(any_traversal_work(value)?)
                    },
                )?;
                children.iter().try_fold(total, |total, child| {
                    total.checked_add(node_work(&child.node)?)
                })
            }
        }
    }

    nodes.iter().try_fold(0usize, |total, child| {
        total.checked_add(node_work(&child.node)?)
    })
}

/// Empty textblocks have a semantic insertion gap but no physical `XmlText` child.
/// Materializing an empty prepared text child gives later operations a stable
/// blueprint-backed target without emitting a second live Yrs action.
fn materialize_empty_prepared_textblocks(nodes: &mut [PreparedXmlChild], schema: &Schema) -> usize {
    fn visit(node: &mut PreparedXmlNode, schema: &Schema, work: &mut usize) {
        let PreparedXmlNode::Element { tag, children, .. } = node else {
            return;
        };
        *work = work.saturating_add(1);
        if children.is_empty()
            && schema
                .node(tag)
                .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        {
            children.push(PreparedXmlChild {
                index: 0,
                node: PreparedXmlNode::Text { runs: Vec::new() },
            });
            *work = work.saturating_add(1);
            return;
        }
        for child in children {
            visit(&mut child.node, schema, work);
        }
    }

    let mut work = 0usize;
    for child in nodes {
        visit(&mut child.node, schema, &mut work);
    }
    work
}

fn first_text_doc_position(root: &Node, path: &[u32]) -> Option<u32> {
    fn relative(node: &Node) -> Option<u32> {
        if node.is_text() {
            return Some(0);
        }
        let mut cursor = 1u32;
        for child in node.content()?.iter() {
            if let Some(inner) = relative(child) {
                return cursor.checked_add(inner);
            }
            cursor = cursor.checked_add(child.node_size())?;
        }
        None
    }

    let mut node = root;
    let mut boundary = 0u32;
    for (depth, child_index) in path.iter().copied().enumerate() {
        let content = node.content()?;
        let index = usize::try_from(child_index).ok()?;
        boundary = content
            .iter()
            .take(index)
            .try_fold(boundary, |total, child| {
                total.checked_add(child.node_size())
            })?;
        let Some(child) = content.child(index) else {
            // Prepared empty textblocks carry a virtual zero-width XmlText at
            // semantic child 0. Its document position is the content start.
            return (depth + 1 == path.len() && index == 0 && content.child_count() == 0)
                .then_some(boundary);
        };
        node = child;
        if depth + 1 < path.len() {
            boundary = boundary.checked_add(1)?;
        }
    }
    boundary.checked_add(relative(node)?)
}

fn collect_prepared_handles(
    insert_id: usize,
    nodes: &[PreparedXmlChild],
    semantic_root: &[u32],
    semantic_document: &Document,
    elements: &mut Vec<(Vec<u32>, PreparedHandle)>,
    texts: &mut Vec<(PreparedHandle, Vec<PreparedTextRun>)>,
) -> OperationResult<()> {
    fn visit(
        insert_id: usize,
        node: &PreparedXmlNode,
        ordinal_path: &mut Vec<usize>,
        semantic_path: &mut Vec<u32>,
        semantic_document: &Document,
        elements: &mut Vec<(Vec<u32>, PreparedHandle)>,
        texts: &mut Vec<(PreparedHandle, Vec<PreparedTextRun>)>,
    ) -> OperationResult<()> {
        let handle = PreparedHandle {
            insert_id,
            ordinal_path: ordinal_path.clone().into_boxed_slice(),
        };
        match node {
            PreparedXmlNode::Text { runs } => texts.push((handle, runs.clone())),
            PreparedXmlNode::Element { children, .. } => {
                elements.push((semantic_path.clone(), handle));
                if semantic_document
                    .node_at(semantic_path)
                    .is_some_and(Node::is_void)
                {
                    return Ok(());
                }
                for (ordinal, child) in children.iter().enumerate() {
                    ordinal_path.push(ordinal);
                    semantic_path.push(u32::try_from(ordinal).map_err(|_| {
                        OperationError::engine_invariant_failed(
                            0,
                            None,
                            "prepared semantic child ordinal exceeds u32",
                        )
                    })?);
                    visit(
                        insert_id,
                        &child.node,
                        ordinal_path,
                        semantic_path,
                        semantic_document,
                        elements,
                        texts,
                    )?;
                    semantic_path.pop();
                    ordinal_path.pop();
                }
            }
        }
        Ok(())
    }

    for (ordinal, child) in nodes.iter().enumerate() {
        let mut ordinal_path = vec![ordinal];
        let mut semantic_path = semantic_root.to_vec();
        if nodes.len() > 1 {
            semantic_path.push(u32::try_from(ordinal).map_err(|_| {
                OperationError::engine_invariant_failed(
                    0,
                    None,
                    "prepared root ordinal exceeds u32",
                )
            })?);
        }
        visit(
            insert_id,
            &child.node,
            &mut ordinal_path,
            &mut semantic_path,
            semantic_document,
            elements,
            texts,
        )?;
    }
    Ok(())
}

fn collect_prepared_child_handles(
    insert_id: usize,
    nodes: &[PreparedXmlChild],
    parent_path: &[u32],
    first_semantic_index: u32,
    semantic_document: Option<&Document>,
    elements: &mut Vec<(Vec<u32>, PreparedHandle)>,
    texts: &mut Vec<(Vec<u32>, PreparedHandle, Vec<PreparedTextRun>)>,
) -> OperationResult<()> {
    fn visit(
        insert_id: usize,
        node: &PreparedXmlNode,
        ordinal_path: &mut Vec<usize>,
        semantic_path: &mut Vec<u32>,
        semantic_document: Option<&Document>,
        elements: &mut Vec<(Vec<u32>, PreparedHandle)>,
        texts: &mut Vec<(Vec<u32>, PreparedHandle, Vec<PreparedTextRun>)>,
    ) -> OperationResult<()> {
        let handle = PreparedHandle {
            insert_id,
            ordinal_path: ordinal_path.clone().into_boxed_slice(),
        };
        match node {
            PreparedXmlNode::Text { runs } => {
                texts.push((semantic_path.clone(), handle, runs.clone()));
            }
            PreparedXmlNode::Element { children, .. } => {
                elements.push((semantic_path.clone(), handle));
                if semantic_document
                    .and_then(|document| document.node_at(semantic_path))
                    .is_some_and(Node::is_void)
                {
                    return Ok(());
                }
                for (ordinal, child) in children.iter().enumerate() {
                    ordinal_path.push(ordinal);
                    semantic_path.push(u32::try_from(ordinal).map_err(|_| {
                        OperationError::engine_invariant_failed(
                            0,
                            None,
                            "prepared semantic child ordinal exceeds u32",
                        )
                    })?);
                    visit(
                        insert_id,
                        &child.node,
                        ordinal_path,
                        semantic_path,
                        semantic_document,
                        elements,
                        texts,
                    )?;
                    semantic_path.pop();
                    ordinal_path.pop();
                }
            }
        }
        Ok(())
    }

    for (ordinal, child) in nodes.iter().enumerate() {
        let mut ordinal_path = vec![ordinal];
        let mut semantic_path = parent_path.to_vec();
        semantic_path.push(
            first_semantic_index
                .checked_add(u32::try_from(ordinal).map_err(|_| {
                    OperationError::engine_invariant_failed(
                        0,
                        None,
                        "prepared root ordinal exceeds u32",
                    )
                })?)
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        0,
                        None,
                        "prepared semantic child index overflow",
                    )
                })?,
        );
        visit(
            insert_id,
            &child.node,
            &mut ordinal_path,
            &mut semantic_path,
            semantic_document,
            elements,
            texts,
        )?;
    }
    Ok(())
}

fn utf16_byte_index(value: &str, index_utf16: u32) -> Option<usize> {
    let mut utf16 = 0u32;
    for (byte, ch) in value.char_indices() {
        if utf16 == index_utf16 {
            return Some(byte);
        }
        utf16 = utf16.checked_add(u32::try_from(ch.len_utf16()).ok()?)?;
        if utf16 > index_utf16 {
            return None;
        }
    }
    (utf16 == index_utf16).then_some(value.len())
}

fn normalize_prepared_runs(runs: Vec<PreparedTextRun>) -> Option<Vec<PreparedTextRun>> {
    let mut normalized: Vec<PreparedTextRun> = Vec::new();
    for run in runs {
        if run.text.is_empty() {
            continue;
        }
        if let Some(previous) = normalized.last_mut() {
            if previous.attrs == run.attrs {
                previous.text.push_str(&run.text);
                continue;
            }
        }
        normalized.push(PreparedTextRun {
            index_utf16: 0,
            text: run.text,
            attrs: run.attrs,
        });
    }
    let mut index = 0u32;
    for run in &mut normalized {
        run.index_utf16 = index;
        index = index.checked_add(u32::try_from(run.text.encode_utf16().count()).ok()?)?;
    }
    Some(normalized)
}

fn split_runs_utf16(
    runs: &[PreparedTextRun],
    index_utf16: u32,
) -> Option<(Vec<PreparedTextRun>, Vec<PreparedTextRun>)> {
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut cursor = 0u32;
    let mut found = false;
    for run in runs {
        let len = u32::try_from(run.text.encode_utf16().count()).ok()?;
        let end = cursor.checked_add(len)?;
        if found || index_utf16 <= cursor {
            found = true;
            right.push(run.clone());
        } else if index_utf16 >= end {
            left.push(run.clone());
            found = index_utf16 == end;
        } else {
            let local = index_utf16.checked_sub(cursor)?;
            let byte = utf16_byte_index(&run.text, local)?;
            if byte > 0 {
                left.push(PreparedTextRun {
                    index_utf16: 0,
                    text: run.text[..byte].to_owned(),
                    attrs: run.attrs.clone(),
                });
            }
            if byte < run.text.len() {
                right.push(PreparedTextRun {
                    index_utf16: 0,
                    text: run.text[byte..].to_owned(),
                    attrs: run.attrs.clone(),
                });
            }
            found = true;
        }
        cursor = end;
    }
    if !found && index_utf16 != cursor {
        return None;
    }
    Some((
        normalize_prepared_runs(left)?,
        normalize_prepared_runs(right)?,
    ))
}

fn insert_prepared_run(
    runs: &mut Vec<PreparedTextRun>,
    index_utf16: u32,
    text: &str,
    attrs: Attrs,
) -> Option<()> {
    let (mut left, right) = split_runs_utf16(runs, index_utf16)?;
    if !text.is_empty() {
        left.push(PreparedTextRun {
            index_utf16: 0,
            text: text.to_owned(),
            attrs,
        });
    }
    left.extend(right);
    *runs = normalize_prepared_runs(left)?;
    Some(())
}

fn delete_prepared_run_range(
    runs: &mut Vec<PreparedTextRun>,
    index_utf16: u32,
    len_utf16: u32,
) -> Option<()> {
    let (mut left, tail) = split_runs_utf16(runs, index_utf16)?;
    let (_, right) = split_runs_utf16(&tail, len_utf16)?;
    left.extend(right);
    *runs = normalize_prepared_runs(left)?;
    Some(())
}

fn format_prepared_run_range(
    runs: &mut Vec<PreparedTextRun>,
    index_utf16: u32,
    len_utf16: u32,
    attrs: &Attrs,
) -> Option<()> {
    let (mut left, tail) = split_runs_utf16(runs, index_utf16)?;
    let (mut middle, right) = split_runs_utf16(&tail, len_utf16)?;
    for run in &mut middle {
        for (key, value) in attrs.iter() {
            if matches!(value, Any::Null | Any::Undefined) {
                run.attrs.remove(key);
            } else {
                run.attrs.insert(key.clone(), value.clone());
            }
        }
    }
    left.extend(middle);
    left.extend(right);
    *runs = normalize_prepared_runs(left)?;
    Some(())
}

fn split_prepared_text_runs(
    request_id: u64,
    operation_index: usize,
    runs: &[PreparedTextRun],
    split_scalar: u32,
) -> OperationResult<(u32, u32, Vec<PreparedTextRun>)> {
    let mut scalar_cursor = 0u32;
    let mut utf16_cursor = 0u32;
    let mut delete_index_utf16 = None;
    let mut suffix = Vec::new();
    let mut suffix_utf16 = 0u32;
    for run in runs {
        let run_scalars = u32::try_from(run.text.chars().count())
            .map_err(|_| invalid_action_range(request_id, operation_index))?;
        let run_utf16 = u32::try_from(run.text.encode_utf16().count())
            .map_err(|_| invalid_action_range(request_id, operation_index))?;
        let run_end = scalar_cursor
            .checked_add(run_scalars)
            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
        if delete_index_utf16.is_none() && split_scalar < run_end {
            let local_scalar = split_scalar
                .checked_sub(scalar_cursor)
                .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
            let byte = scalar_byte_index(&run.text, local_scalar)
                .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
            let left_utf16 = u32::try_from(run.text[..byte].encode_utf16().count())
                .map_err(|_| invalid_action_range(request_id, operation_index))?;
            delete_index_utf16 = Some(
                utf16_cursor
                    .checked_add(left_utf16)
                    .ok_or_else(|| invalid_action_range(request_id, operation_index))?,
            );
            let tail = &run.text[byte..];
            if !tail.is_empty() {
                suffix.push(PreparedTextRun {
                    index_utf16: suffix_utf16,
                    text: tail.to_owned(),
                    attrs: run.attrs.clone(),
                });
                suffix_utf16 = suffix_utf16
                    .checked_add(
                        u32::try_from(tail.encode_utf16().count())
                            .map_err(|_| invalid_action_range(request_id, operation_index))?,
                    )
                    .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
            }
        } else if delete_index_utf16.is_some() {
            suffix.push(PreparedTextRun {
                index_utf16: suffix_utf16,
                text: run.text.clone(),
                attrs: run.attrs.clone(),
            });
            suffix_utf16 = suffix_utf16
                .checked_add(run_utf16)
                .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
        }
        scalar_cursor = run_end;
        utf16_cursor = utf16_cursor
            .checked_add(run_utf16)
            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    }
    let delete_index_utf16 =
        delete_index_utf16.ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    let delete_len_utf16 = utf16_cursor
        .checked_sub(delete_index_utf16)
        .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    if suffix.is_empty() || delete_len_utf16 == 0 {
        return Err(invalid_action_range(request_id, operation_index));
    }
    Ok((delete_index_utf16, delete_len_utf16, suffix))
}

fn prepared_runs_utf16_at_scalar(
    request_id: u64,
    operation_index: usize,
    runs: &[PreparedTextRun],
    scalar: u32,
) -> OperationResult<u32> {
    let mut scalar_cursor = 0u32;
    let mut utf16_cursor = 0u32;
    for run in runs {
        let run_scalars = u32::try_from(run.text.chars().count())
            .map_err(|_| invalid_action_range(request_id, operation_index))?;
        let run_end = scalar_cursor
            .checked_add(run_scalars)
            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
        if scalar <= run_end {
            let local = scalar
                .checked_sub(scalar_cursor)
                .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
            let byte = scalar_byte_index(&run.text, local)
                .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
            return utf16_cursor
                .checked_add(
                    u32::try_from(run.text[..byte].encode_utf16().count())
                        .map_err(|_| invalid_action_range(request_id, operation_index))?,
                )
                .ok_or_else(|| invalid_action_range(request_id, operation_index));
        }
        scalar_cursor = run_end;
        utf16_cursor = utf16_cursor
            .checked_add(
                u32::try_from(run.text.encode_utf16().count())
                    .map_err(|_| invalid_action_range(request_id, operation_index))?,
            )
            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    }
    if scalar == scalar_cursor {
        Ok(utf16_cursor)
    } else {
        Err(invalid_action_range(request_id, operation_index))
    }
}

fn prepared_runs_utf16_len(
    request_id: u64,
    operation_index: usize,
    runs: &[PreparedTextRun],
) -> OperationResult<u32> {
    runs.iter().try_fold(0u32, |total, run| {
        total
            .checked_add(
                u32::try_from(run.text.encode_utf16().count())
                    .map_err(|_| invalid_action_range(request_id, operation_index))?,
            )
            .ok_or_else(|| invalid_action_range(request_id, operation_index))
    })
}

fn map_prepared_node_error(
    request_id: u64,
    operation_index: usize,
    error: super::super::YrsEngineError,
) -> OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        OperationError::document_limit_exceeded(
            request_id,
            Some(operation_index),
            "preparedXmlNode",
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        )
    } else {
        OperationError::document_invalid(
            request_id,
            Some(operation_index),
            "preparedXmlNode",
            error.message,
        )
    }
}
