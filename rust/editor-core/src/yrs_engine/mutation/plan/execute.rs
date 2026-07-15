#[allow(dead_code)] // Production execution is consumed by the Task 7 engine boundary.
pub(crate) fn execute_mutation_plan(plan: YrsMutationPlan, txn: &mut TransactionMut<'_>) {
    for action in plan.actions {
        match action {
            YrsMutationAction::DeleteXmlChildren {
                parent,
                child_index,
                child_count,
                ..
            } => parent.remove_range(txn, child_index, child_count),
            YrsMutationAction::InsertXmlChildren {
                parent,
                child_index: _,
                nodes,
                ..
            } => {
                for child in nodes {
                    parent.insert_prepared(txn, child.index, child.node);
                }
            }
            YrsMutationAction::SetXmlAttribute {
                target, key, value, ..
            } => {
                target.insert_attribute(txn, key.as_ref(), value);
            }
            YrsMutationAction::RemoveXmlAttribute { target, key, .. } => {
                target.remove_attribute(txn, &key);
            }
            YrsMutationAction::CreateText {
                parent,
                child_index,
                text,
                attrs,
                follow_up,
                ..
            } => {
                let target = parent.insert(txn, child_index, XmlTextPrelim::new(""));
                target.insert_with_attributes(txn, 0, &text, attrs);
                for follow in follow_up {
                    match follow {
                        CreatedTextAction::Insert {
                            index_utf16,
                            text,
                            attrs,
                            ..
                        } => target.insert_with_attributes(txn, index_utf16, &text, attrs),
                        CreatedTextAction::Delete {
                            index_utf16,
                            len_utf16,
                            ..
                        } => target.remove_range(txn, index_utf16, len_utf16),
                        CreatedTextAction::Format {
                            index_utf16,
                            len_utf16,
                            attrs,
                            ..
                        } => target.format(txn, index_utf16, len_utf16, attrs),
                    }
                }
            }
            YrsMutationAction::InsertText {
                target,
                index_utf16,
                text,
                attrs,
                ..
            } => target.insert_with_attributes(txn, index_utf16, &text, attrs),
            YrsMutationAction::DeleteText {
                target,
                index_utf16,
                len_utf16,
                ..
            } => target.remove_range(txn, index_utf16, len_utf16),
            YrsMutationAction::FormatText {
                target,
                index_utf16,
                len_utf16,
                attrs,
                ..
            } => target.format(txn, index_utf16, len_utf16, attrs),
        }
    }
}
