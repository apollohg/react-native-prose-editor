use std::collections::HashMap;

use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::{NodeRole, Schema};
use crate::selection::Selection;

use super::{SemanticCommandPlan, SemanticOperation};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MarkCommandPlan {
    pub semantic: SemanticCommandPlan,
    pub stored_marks_after: Option<Vec<Mark>>,
}

fn stored_marks_at(
    document: &Document,
    schema: &Schema,
    position: u32,
    stored_marks: Option<&[Mark]>,
) -> Vec<Mark> {
    super::canonical_marks(
        &stored_marks
            .map(<[_]>::to_vec)
            .unwrap_or_else(|| crate::editor_state::marks_at_position(document, position)),
        schema,
    )
}

pub(crate) fn plan_toggle_mark(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    stored_marks: Option<&[Mark]>,
    mark_type: &str,
) -> Option<MarkCommandPlan> {
    let Selection::Text { anchor, head } = selection else {
        return None;
    };
    let from = (*anchor).min(*head);
    let to = (*anchor).max(*head);
    if from == to {
        let mut stored = stored_marks_at(document, schema, from, stored_marks);
        let active = stored.iter().any(|mark| mark.mark_type() == mark_type);
        let operation = if active {
            stored.retain(|mark| mark.mark_type() != mark_type);
            SemanticOperation::RemoveMark {
                from,
                to,
                mark_type: mark_type.to_string(),
            }
        } else {
            let mark = Mark::new(mark_type.to_string(), Default::default());
            stored.push(mark.clone());
            stored = super::canonical_marks(&stored, schema);
            SemanticOperation::AddMark { from, to, mark }
        };
        return Some(MarkCommandPlan {
            semantic: SemanticCommandPlan {
                operations: vec![operation],
                selection_after: Some(selection.clone()),
            },
            stored_marks_after: Some(stored),
        });
    }
    let operation = if crate::editor_state::range_has_mark(document, from, to, mark_type) {
        SemanticOperation::RemoveMark {
            from,
            to,
            mark_type: mark_type.to_string(),
        }
    } else {
        SemanticOperation::AddMark {
            from,
            to,
            mark: Mark::new(mark_type.to_string(), Default::default()),
        }
    };
    Some(MarkCommandPlan {
        semantic: SemanticCommandPlan {
            operations: vec![operation],
            selection_after: Some(selection.clone()),
        },
        stored_marks_after: None,
    })
}

pub(crate) fn plan_set_mark(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    stored_marks: Option<&[Mark]>,
    mark: Mark,
) -> Option<MarkCommandPlan> {
    let Selection::Text { anchor, head } = selection else {
        return None;
    };
    let from = (*anchor).min(*head);
    let to = (*anchor).max(*head);
    let mark_type = mark.mark_type().to_string();
    if from == to {
        if let Some((range_from, range_to)) =
            crate::editor_state::mark_range_at_position(document, from, &mark_type)
        {
            return Some(MarkCommandPlan {
                semantic: SemanticCommandPlan {
                    operations: vec![
                        SemanticOperation::RemoveMark {
                            from: range_from,
                            to: range_to,
                            mark_type,
                        },
                        SemanticOperation::AddMark {
                            from: range_from,
                            to: range_to,
                            mark,
                        },
                    ],
                    selection_after: Some(selection.clone()),
                },
                stored_marks_after: None,
            });
        }
        let mut stored = stored_marks_at(document, schema, from, stored_marks);
        stored.retain(|candidate| candidate.mark_type() != mark_type);
        stored.push(mark.clone());
        stored = super::canonical_marks(&stored, schema);
        return Some(MarkCommandPlan {
            semantic: SemanticCommandPlan {
                operations: vec![SemanticOperation::ReplaceMark { from, to, mark }],
                selection_after: Some(selection.clone()),
            },
            stored_marks_after: Some(stored),
        });
    }
    Some(MarkCommandPlan {
        semantic: SemanticCommandPlan {
            operations: vec![
                SemanticOperation::RemoveMark {
                    from,
                    to,
                    mark_type,
                },
                SemanticOperation::AddMark { from, to, mark },
            ],
            selection_after: Some(selection.clone()),
        },
        stored_marks_after: None,
    })
}

pub(crate) fn plan_unset_mark(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    stored_marks: Option<&[Mark]>,
    mark_type: &str,
) -> Option<MarkCommandPlan> {
    let Selection::Text { anchor, head } = selection else {
        return None;
    };
    let from = (*anchor).min(*head);
    let to = (*anchor).max(*head);
    if from == to {
        if let Some((range_from, range_to)) =
            crate::editor_state::mark_range_at_position(document, from, mark_type)
        {
            return Some(MarkCommandPlan {
                semantic: SemanticCommandPlan {
                    operations: vec![SemanticOperation::RemoveMark {
                        from: range_from,
                        to: range_to,
                        mark_type: mark_type.to_string(),
                    }],
                    selection_after: Some(selection.clone()),
                },
                stored_marks_after: None,
            });
        }
        let mut stored = stored_marks_at(document, schema, from, stored_marks);
        stored.retain(|candidate| candidate.mark_type() != mark_type);
        return Some(MarkCommandPlan {
            semantic: SemanticCommandPlan {
                operations: vec![SemanticOperation::RemoveMark {
                    from,
                    to,
                    mark_type: mark_type.to_string(),
                }],
                selection_after: Some(selection.clone()),
            },
            stored_marks_after: Some(stored),
        });
    }
    Some(MarkCommandPlan {
        semantic: SemanticCommandPlan {
            operations: vec![SemanticOperation::RemoveMark {
                from,
                to,
                mark_type: mark_type.to_string(),
            }],
            selection_after: Some(selection.clone()),
        },
        stored_marks_after: None,
    })
}

/// A semantic document-range replacement produced by a shared command planner.
/// It deliberately contains no standalone-backend transform step.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CommandReplacement {
    pub from: u32,
    pub to: u32,
    pub content: Fragment,
    pub selection_after: Selection,
}

pub(crate) fn code_block_node_name(schema: &Schema) -> Option<&str> {
    schema
        .node_by_html_tag("pre")
        .filter(|spec| matches!(spec.role, NodeRole::TextBlock))
        .map(|spec| spec.name.as_str())
}

pub(crate) fn plan_toggle_heading(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    level: u8,
) -> Option<CommandReplacement> {
    let target_type = schema.node_by_html_tag(&format!("h{level}"))?.name.as_str();
    let paragraph_type = crate::editor_state::paragraph_node_name(schema)?;
    let range = crate::editor_state::selected_text_block_range(document, schema, selection)?;
    let replacement_type = if range
        .selected_blocks
        .iter()
        .all(|block| block.node_type() == target_type)
    {
        paragraph_type
    } else {
        target_type
    };
    replacement_for_text_blocks(document, schema, selection, range, replacement_type)
}

pub(crate) fn plan_toggle_code_block(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
) -> Option<CommandReplacement> {
    let code_block_type = code_block_node_name(schema)?;
    let paragraph_type = crate::editor_state::paragraph_node_name(schema)?;
    let range = crate::editor_state::selected_text_block_range(document, schema, selection)?;
    let replacement_type = if range
        .selected_blocks
        .iter()
        .all(|block| block.node_type() == code_block_type)
    {
        paragraph_type
    } else {
        code_block_type
    };
    replacement_for_text_blocks(document, schema, selection, range, replacement_type)
}

pub(crate) fn plan_toggle_blockquote(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
) -> Option<CommandReplacement> {
    let blockquote_type = schema.node_by_html_tag("blockquote")?.name.as_str();
    let pos = selection.from(document);
    if let Some((start, quote)) =
        crate::editor_state::containing_node_at(document, schema, pos, |_, name| {
            name == blockquote_type
        })
    {
        let content = quote.content()?;
        return Some(CommandReplacement {
            from: start,
            to: start.checked_add(quote.node_size())?,
            content: Fragment::from(content.iter().cloned().collect()),
            selection_after: shift_selection(selection, -1)?,
        });
    }
    let range = crate::editor_state::selected_block_range(
        document,
        schema,
        selection.from(document),
        selection.to(document),
    )?;
    let quote_spec = schema.node(blockquote_type)?;
    let selected = range
        .selected_blocks
        .iter()
        .map(Node::node_type)
        .collect::<Vec<_>>();
    if !quote_spec.content.matches(&selected, |child, symbol| {
        schema.node_matches_symbol(child, symbol)
    }) {
        return None;
    }
    Some(CommandReplacement {
        from: range.replace_from,
        to: range.replace_to,
        content: Fragment::from(vec![Node::element(
            blockquote_type.to_string(),
            HashMap::new(),
            Fragment::from(range.selected_blocks),
        )]),
        selection_after: shift_selection(selection, 1)?,
    })
}

fn replacement_for_text_blocks(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    range: crate::editor_state::BlockSelectionRange,
    target_type: &str,
) -> Option<CommandReplacement> {
    if !crate::editor_state::can_replace_selected_text_blocks(document, schema, &range, target_type)
    {
        return None;
    }
    let content = range
        .selected_blocks
        .iter()
        .map(|block| {
            Some(Node::element(
                target_type.to_string(),
                HashMap::new(),
                block.content().cloned().unwrap_or_else(Fragment::empty),
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(CommandReplacement {
        from: range.replace_from,
        to: range.replace_to,
        content: Fragment::from(content),
        selection_after: selection.clone(),
    })
}

fn shift_selection(selection: &Selection, delta: i32) -> Option<Selection> {
    let shift = |position: u32| {
        if delta >= 0 {
            position.checked_add(delta as u32)
        } else {
            position.checked_sub(delta.unsigned_abs())
        }
    };
    match selection {
        Selection::Text { anchor, head } => Some(Selection::text(shift(*anchor)?, shift(*head)?)),
        Selection::Node { pos } => Some(Selection::node(shift(*pos)?)),
        Selection::All => Some(Selection::All),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::serialize::{from_prosemirror_json, to_prosemirror_json, UnknownTypeMode};

    #[test]
    fn set_mark_plan_covers_an_entire_mixed_range() {
        let schema = crate::tiptap_schema();
        let document = from_prosemirror_json(
            &serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[
                {"type":"text","text":"a","marks":[{"type":"link","attrs":{"href":"old"}}]},
                {"type":"text","text":"b"}
            ]}]}),
            &schema,
            UnknownTypeMode::Preserve,
        )
        .unwrap();
        let selection = Selection::text(1, 3);
        let mark = Mark::new(
            "link".into(),
            HashMap::from([("href".into(), serde_json::json!("new"))]),
        );

        let plan = plan_set_mark(&document, &schema, &selection, None, mark).unwrap();

        assert!(plan.stored_marks_after.is_none());
        assert!(matches!(
            plan.semantic.operations.as_slice(),
            [
                SemanticOperation::RemoveMark { from: 1, to: 3, .. },
                SemanticOperation::AddMark { from: 1, to: 3, .. }
            ]
        ));
        let simulated = super::super::simulate_plan(
            &document,
            &schema,
            &selection,
            &plan.semantic,
            &crate::boundary::ResourceLimits::default(),
        )
        .unwrap();
        assert_eq!(
            to_prosemirror_json(&simulated.document, &schema),
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{
                "type":"text","text":"ab","marks":[{"type":"link","attrs":{"href":"new"}}]
            }]}]})
        );
    }
}
