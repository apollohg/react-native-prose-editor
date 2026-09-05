use super::observability::{
    record_preview_position_map_derivation, record_preview_rendered_text_derivation,
};
#[cfg(test)]
use super::observability::{
    OPERATION_RESULT_RELATIVE_TRAVERSALS, RELATIVE_SELECTION_RESOLUTION_TRAVERSALS,
};
use super::DerivedStateCache;
use crate::model::{Document, Mark};
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::yrs_engine::compiler::selectable_void_at;
use crate::yrs_engine::position::{
    cursor_sticky_index_from_doc_pos, doc_pos_to_relative_point, doc_pos_to_sticky_index,
    relative_point_to_doc_pos, relative_selection_to_selection,
};
use crate::yrs_engine::{
    scalar_offset_to_utf16, Affinity, OperationError, OperationResult, RelativePoint,
    RelativeSelection, ResolvedPoint, ResolvedSelection, TypedOperation,
};
use yrs::types::xml::XmlFragmentRef;
use yrs::{Assoc, ReadTxn};

/// A fully materialized selection state whose three representations were
/// proven against the same prewrite view. Keeping the fields private prevents
/// later compiler stages from mixing representations from different views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FinalizedSelectionState {
    pub(super) relative: RelativeSelection,
    pub(super) resolved: ResolvedSelection,
    pub(super) legacy: Selection,
}

impl FinalizedSelectionState {
    pub(crate) fn new(
        relative: RelativeSelection,
        resolved: ResolvedSelection,
        legacy: Selection,
    ) -> Option<Self> {
        (resolved_to_legacy(&resolved) == legacy).then_some(Self {
            relative,
            resolved,
            legacy,
        })
    }

    pub(crate) fn relative(&self) -> &RelativeSelection {
        &self.relative
    }

    pub(super) fn into_parts(self) -> (RelativeSelection, ResolvedSelection, Selection) {
        (self.relative, self.resolved, self.legacy)
    }

    #[cfg(test)]
    pub(crate) fn tampered_for_test(&self) -> Vec<Self> {
        let mut relative = self.clone();
        relative.relative = RelativeSelection::All;
        let mut resolved = self.clone();
        resolved.resolved = ResolvedSelection::All;
        let mut legacy = self.clone();
        legacy.legacy = Selection::all();
        vec![relative, resolved, legacy]
    }
}

impl DerivedStateCache {
    pub fn update_selection_state(
        &mut self,
        relative_selection: RelativeSelection,
        resolved_selection: ResolvedSelection,
        stored_marks: Option<Vec<Mark>>,
        state_revision: u64,
    ) {
        self.legacy_selection = resolved_to_legacy(&resolved_selection);
        self.relative_selection = relative_selection;
        self.resolved_selection = resolved_selection;
        self.stored_marks = stored_marks;
        self.reseal_state_revision(state_revision);
        self.clear_active_state_certificate();
    }

    pub fn resolve_relative_selection<T: ReadTxn>(
        &self,
        relative_selection: &RelativeSelection,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
    ) -> Option<ResolvedSelection> {
        resolve_selection(
            txn,
            fragment,
            relative_selection,
            schema,
            &self.document,
            &self.position_map,
            &self.rendered_text,
        )
    }

    pub fn legacy_selection(&self) -> Selection {
        self.legacy_selection.clone()
    }
}

pub(crate) fn stored_marks_after_selection_change(
    current: Option<&[Mark]>,
    before: &ResolvedSelection,
    after: &ResolvedSelection,
    _document: &Document,
    schema: &Schema,
) -> Option<Vec<Mark>> {
    let current = current?;
    if before != after || !is_collapsed_text(after) {
        return None;
    }
    Some(canonical_marks(current, schema))
}

pub(crate) fn apply_stored_mark_operation(
    marks: &mut Vec<Mark>,
    operation: &TypedOperation,
    schema: &Schema,
) -> OperationResult<bool> {
    match operation {
        TypedOperation::AddMark { mark, .. } => {
            if let Some(existing) = marks
                .iter()
                .find(|candidate| candidate.mark_type() == mark.mark_type())
            {
                if existing != mark {
                    return Err(OperationError::operation_invalid(
                        0,
                        0,
                        "mark",
                        "AddMark conflicts with an existing same-type mark; use ReplaceMark",
                    ));
                }
                return Ok(false);
            }
            insert_mark_ranked(marks, mark.clone(), schema);
            Ok(true)
        }
        TypedOperation::RemoveMark { mark_type, .. } => {
            let previous_len = marks.len();
            marks.retain(|candidate| candidate.mark_type() != mark_type);
            Ok(marks.len() != previous_len)
        }
        TypedOperation::ReplaceMark { mark, .. } => {
            if marks
                .iter()
                .find(|candidate| candidate.mark_type() == mark.mark_type())
                == Some(mark)
            {
                return Ok(false);
            }
            marks.retain(|candidate| candidate.mark_type() != mark.mark_type());
            insert_mark_ranked(marks, mark.clone(), schema);
            Ok(true)
        }
        _ => Err(OperationError::engine_invariant_failed(
            0,
            None,
            "stored mark transition received a non-mark operation",
        )),
    }
}

pub(crate) fn resolved_from_legacy(
    document: &Document,
    selection: &Selection,
    schema: &Schema,
) -> Option<ResolvedSelection> {
    record_preview_position_map_derivation();
    let position_map = PositionMap::build(document, schema);
    record_preview_rendered_text_derivation();
    let rendered = crate::render::rendered_text(document, schema);
    resolved_from_legacy_with_view(document, selection, schema, &position_map, &rendered)
}

pub(crate) fn resolved_from_legacy_with_view(
    document: &Document,
    selection: &Selection,
    schema: &Schema,
    position_map: &PositionMap,
    rendered: &str,
) -> Option<ResolvedSelection> {
    let point = |document_position| {
        let scalar = position_map.doc_to_scalar(document_position, document);
        Some(ResolvedPoint {
            document: document_position,
            scalar,
            utf16: scalar_offset_to_utf16(rendered, scalar)?,
        })
    };
    match selection {
        Selection::Text { anchor, head } => Some(ResolvedSelection::Text {
            anchor: point(*anchor)?,
            head: point(*head)?,
        }),
        Selection::Node { pos } if selectable_void_at(document.root(), *pos, 0, schema) => {
            Some(ResolvedSelection::Node { at: point(*pos)? })
        }
        Selection::Node { .. } => None,
        Selection::All => Some(ResolvedSelection::All),
    }
}

pub(super) fn is_collapsed_text(selection: &ResolvedSelection) -> bool {
    matches!(selection, ResolvedSelection::Text { anchor, head } if anchor.document == head.document)
}

pub(crate) fn canonical_marks(marks: &[Mark], schema: &Schema) -> Vec<Mark> {
    let mut marks = marks.to_vec();
    marks.sort_by(|left, right| {
        schema
            .mark_rank(left.mark_type())
            .unwrap_or(usize::MAX)
            .cmp(&schema.mark_rank(right.mark_type()).unwrap_or(usize::MAX))
            .then_with(|| left.mark_type().cmp(right.mark_type()))
    });
    marks
}

pub(super) fn insert_mark_ranked(marks: &mut Vec<Mark>, mark: Mark, schema: &Schema) {
    let rank = schema.mark_rank(mark.mark_type()).unwrap_or(usize::MAX);
    let index = marks
        .iter()
        .position(|candidate| {
            schema
                .mark_rank(candidate.mark_type())
                .unwrap_or(usize::MAX)
                > rank
        })
        .unwrap_or(marks.len());
    marks.insert(index, mark);
}

pub(crate) fn marks_at_position(document: &Document, position: u32) -> Vec<Mark> {
    crate::editor_state::marks_at_position(document, position)
}

pub(super) fn preserve_with_mapped_fallback<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    current: &RelativeSelection,
    mapped: &Selection,
    schema: &Schema,
    strict_affinity: bool,
) -> RelativeSelection {
    let point = |current: &RelativePoint, mapped_position| {
        if relative_point_to_doc_pos(txn, fragment, current, schema).is_some() {
            return current.clone();
        }
        if let Some(point) =
            doc_pos_to_relative_point(txn, fragment, mapped_position, current.affinity, schema)
        {
            return point;
        }
        assert!(
            !strict_affinity,
            "prevalidated explicit selection affinity must remain exactly representable"
        );
        let sticky = cursor_sticky_index_from_doc_pos(txn, fragment, mapped_position, true, schema)
            .expect("compiler-normalized mapped fallback has a Yrs association");
        RelativePoint {
            affinity: affinity_from_assoc(sticky.assoc),
            sticky,
        }
    };
    match (current, mapped) {
        (
            RelativeSelection::Text { anchor, head },
            Selection::Text {
                anchor: mapped_anchor,
                head: mapped_head,
            },
        ) => RelativeSelection::Text {
            anchor: point(anchor, *mapped_anchor),
            head: point(head, *mapped_head),
        },
        (RelativeSelection::Node { point: current }, Selection::Node { pos }) => {
            RelativeSelection::Node {
                point: point(current, *pos),
            }
        }
        (RelativeSelection::All, Selection::All) => RelativeSelection::All,
        _ => operation_result_to_relative(txn, fragment, mapped, schema),
    }
}

pub(crate) fn resolve_selection<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    relative_selection: &RelativeSelection,
    schema: &Schema,
    document: &Document,
    position_map: &PositionMap,
    rendered_text: &str,
) -> Option<ResolvedSelection> {
    #[cfg(test)]
    RELATIVE_SELECTION_RESOLUTION_TRAVERSALS.set(
        RELATIVE_SELECTION_RESOLUTION_TRAVERSALS
            .get()
            .saturating_add(1),
    );
    let selection = relative_selection_to_selection(
        txn,
        fragment,
        relative_selection,
        schema,
        document,
        position_map,
    )?;
    let point = |document_position| {
        let scalar = position_map.doc_to_scalar(document_position, document);
        Some(ResolvedPoint {
            document: document_position,
            scalar,
            utf16: scalar_offset_to_utf16(rendered_text, scalar)?,
        })
    };
    match selection {
        Selection::Text { anchor, head } => Some(ResolvedSelection::Text {
            anchor: point(anchor)?,
            head: point(head)?,
        }),
        Selection::Node { pos } if selectable_void_at(document.root(), pos, 0, schema) => {
            Some(ResolvedSelection::Node { at: point(pos)? })
        }
        Selection::Node { .. } => None,
        Selection::All => Some(ResolvedSelection::All),
    }
}

pub(crate) fn operation_result_to_relative<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    selection: &Selection,
    schema: &Schema,
) -> RelativeSelection {
    #[cfg(test)]
    OPERATION_RESULT_RELATIVE_TRAVERSALS
        .set(OPERATION_RESULT_RELATIVE_TRAVERSALS.get().saturating_add(1));
    let before = |position| {
        doc_pos_to_sticky_index(txn, fragment, position, Assoc::Before, schema).expect(
            "compiler-normalized operation-result position has an exact Before Yrs association",
        )
    };
    match selection {
        Selection::Text { anchor, head } if anchor == head => {
            let sticky = cursor_sticky_index_from_doc_pos(txn, fragment, *anchor, true, schema)
                .expect("compiler-normalized cursor has a Yrs association");
            let point = RelativePoint {
                affinity: affinity_from_assoc(sticky.assoc),
                sticky,
            };
            RelativeSelection::Text {
                anchor: point.clone(),
                head: point,
            }
        }
        Selection::Text { anchor, head } => RelativeSelection::Text {
            anchor: RelativePoint {
                sticky: before(*anchor),
                affinity: Affinity::Before,
            },
            head: RelativePoint {
                sticky: before(*head),
                affinity: Affinity::Before,
            },
        },
        Selection::Node { pos } => RelativeSelection::Node {
            point: RelativePoint {
                sticky: before(*pos),
                affinity: Affinity::Before,
            },
        },
        Selection::All => RelativeSelection::All,
    }
}

pub(crate) fn history_selection_to_relative<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    expected_relative: &RelativeSelection,
    expected_resolved: &ResolvedSelection,
    schema: &Schema,
) -> Option<RelativeSelection> {
    #[cfg(test)]
    OPERATION_RESULT_RELATIVE_TRAVERSALS
        .set(OPERATION_RESULT_RELATIVE_TRAVERSALS.get().saturating_add(1));
    let point = |position, captured: &RelativePoint| {
        doc_pos_to_relative_point(txn, fragment, position, captured.affinity, schema)
    };
    match (expected_relative, expected_resolved) {
        (
            RelativeSelection::Text {
                anchor: captured_anchor,
                head: captured_head,
            },
            ResolvedSelection::Text { anchor, head },
        ) => Some(RelativeSelection::Text {
            anchor: point(anchor.document, captured_anchor)?,
            head: point(head.document, captured_head)?,
        }),
        (RelativeSelection::Node { point: captured }, ResolvedSelection::Node { at }) => {
            Some(RelativeSelection::Node {
                point: point(at.document, captured)?,
            })
        }
        (RelativeSelection::All, ResolvedSelection::All) => Some(RelativeSelection::All),
        _ => None,
    }
}

pub(crate) fn exact_point_is_representable<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    position: u32,
    point: &RelativePoint,
    schema: &Schema,
) -> bool {
    doc_pos_to_relative_point(txn, fragment, position, point.affinity, schema).is_some()
}

pub(crate) fn resolved_to_legacy(selection: &ResolvedSelection) -> Selection {
    match selection {
        ResolvedSelection::Text { anchor, head } => Selection::text(anchor.document, head.document),
        ResolvedSelection::Node { at } => Selection::node(at.document),
        ResolvedSelection::All => Selection::all(),
    }
}

pub(super) fn affinity_from_assoc(assoc: Assoc) -> Affinity {
    match assoc {
        Assoc::Before => Affinity::Before,
        Assoc::After => Affinity::After,
    }
}
