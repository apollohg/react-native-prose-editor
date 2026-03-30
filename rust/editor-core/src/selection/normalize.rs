use crate::model::Document;
use crate::position::PositionMap;

use super::Selection;

/// Normalize a selection by snapping its positions to the nearest cursorable
/// doc positions via the `PositionMap`.
///
/// - `Text`: both anchor and head are snapped independently.
/// - `Node`: the pos is snapped.
/// - `All`: returned as-is (it resolves lazily from the document).
pub fn normalize_selection(
    selection: Selection,
    doc: &Document,
    pos_map: &PositionMap,
) -> Selection {
    match selection {
        Selection::Text { anchor, head } => {
            let norm_anchor = pos_map.normalize_cursor_pos(anchor, doc);
            let norm_head = pos_map.normalize_cursor_pos(head, doc);
            Selection::Text {
                anchor: norm_anchor,
                head: norm_head,
            }
        }
        Selection::Node { pos } => {
            let norm_pos = pos_map.normalize_cursor_pos(pos, doc);
            Selection::Node { pos: norm_pos }
        }
        Selection::All => Selection::All,
    }
}
