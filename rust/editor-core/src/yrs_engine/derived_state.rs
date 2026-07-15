use yrs::types::xml::XmlFragmentRef;
use yrs::{Assoc, ReadTxn};

use crate::model::{Document, Mark};
use crate::position::update::UpdateMode;
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::transform::StepMap;

use super::compiler::selectable_void_at;
use super::position::{
    cursor_sticky_index_from_doc_pos, doc_pos_to_relative_point, doc_pos_to_sticky_index,
    relative_point_to_doc_pos, relative_selection_to_selection,
};
use super::{
    scalar_offset_to_utf16, Affinity, RelativePoint, RelativeSelection, ResolvedPoint,
    ResolvedSelection,
};

#[derive(Debug, Clone)]
pub(crate) struct DerivedStateCache {
    pub document: Document,
    pub canonical_json: serde_json::Value,
    pub position_map: PositionMap,
    pub relative_selection: RelativeSelection,
    pub resolved_selection: ResolvedSelection,
    pub stored_marks: Option<Vec<Mark>>,
    pub document_revision: u64,
    pub state_revision: u64,
}

impl DerivedStateCache {
    pub fn initialize<T: ReadTxn>(
        document: Document,
        canonical_json: serde_json::Value,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        document_revision: u64,
        state_revision: u64,
    ) -> Option<Self> {
        let position_map = PositionMap::build(&document, schema);
        let selection = (0..position_map.block_count())
            .filter_map(|index| position_map.block(index))
            .find(|block| !block.is_void_block)
            .map(|block| Selection::cursor(block.doc_start))
            .or_else(|| {
                position_map
                    .block(0)
                    .map(|block| Selection::node(block.doc_start))
            })
            .unwrap_or_else(Selection::all);
        let relative_selection = operation_result_to_relative(txn, fragment, &selection, schema);
        let resolved_selection = resolve_selection(
            txn,
            fragment,
            &relative_selection,
            schema,
            &document,
            &position_map,
        )?;
        Some(Self {
            document,
            canonical_json,
            position_map,
            relative_selection,
            resolved_selection,
            stored_marks: None,
            document_revision,
            state_revision,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn after_document_change<T: ReadTxn>(
        &self,
        document: Document,
        canonical_json: serde_json::Value,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        step_map: &StepMap,
        update_mode: UpdateMode,
        affected_top_level_blocks: &[usize],
        explicit_selection: Option<&RelativeSelection>,
        preserved_fallback: Option<&Selection>,
        strict_fallback_affinity: bool,
        document_revision: u64,
        state_revision: u64,
    ) -> Option<Self> {
        let mut position_map = self.position_map.clone();
        let update_mode = if affected_top_level_blocks.is_empty() && self.document != document {
            UpdateMode::Rebuild
        } else {
            update_mode
        };
        position_map.update(step_map, &self.document, &document, update_mode, schema);
        position_map.compact();

        let mut relative_selection = explicit_selection
            .cloned()
            .unwrap_or_else(|| self.relative_selection.clone());
        let mut resolved_selection = resolve_selection(
            txn,
            fragment,
            &relative_selection,
            schema,
            &document,
            &position_map,
        );
        if resolved_selection.is_none() {
            let fallback = preserved_fallback?;
            relative_selection = preserve_with_mapped_fallback(
                txn,
                fragment,
                &relative_selection,
                fallback,
                schema,
                strict_fallback_affinity,
            );
            resolved_selection = resolve_selection(
                txn,
                fragment,
                &relative_selection,
                schema,
                &document,
                &position_map,
            );
        }
        let resolved_selection = resolved_selection?;

        Some(Self {
            document,
            canonical_json,
            position_map,
            relative_selection,
            resolved_selection,
            stored_marks: self.stored_marks.clone(),
            document_revision,
            state_revision,
        })
    }

    pub fn with_relative_selection<T: ReadTxn>(
        &self,
        relative_selection: RelativeSelection,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        state_revision: u64,
    ) -> Option<Self> {
        let resolved_selection = resolve_selection(
            txn,
            fragment,
            &relative_selection,
            schema,
            &self.document,
            &self.position_map,
        )?;
        let mut next = self.clone();
        next.relative_selection = relative_selection;
        next.resolved_selection = resolved_selection;
        next.state_revision = state_revision;
        Some(next)
    }

    pub fn legacy_selection(&self) -> Selection {
        resolved_to_legacy(&self.resolved_selection)
    }
}

fn preserve_with_mapped_fallback<T: ReadTxn>(
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
) -> Option<ResolvedSelection> {
    let selection = relative_selection_to_selection(
        txn,
        fragment,
        relative_selection,
        schema,
        document,
        position_map,
    )?;
    let rendered = crate::render::rendered_text(document, schema);
    let point = |document_position| {
        let scalar = position_map.doc_to_scalar(document_position, document);
        Some(ResolvedPoint {
            document: document_position,
            scalar,
            utf16: scalar_offset_to_utf16(&rendered, scalar)?,
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

pub(crate) fn exact_point_is_representable<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    position: u32,
    point: &RelativePoint,
    schema: &Schema,
) -> bool {
    doc_pos_to_relative_point(txn, fragment, position, point.affinity, schema).is_some()
}

fn resolved_to_legacy(selection: &ResolvedSelection) -> Selection {
    match selection {
        ResolvedSelection::Text { anchor, head } => Selection::text(anchor.document, head.document),
        ResolvedSelection::Node { at } => Selection::node(at.document),
        ResolvedSelection::All => Selection::all(),
    }
}

fn affinity_from_assoc(assoc: Assoc) -> Affinity {
    match assoc {
        Assoc::Before => Affinity::Before,
        Assoc::After => Affinity::After,
    }
}
