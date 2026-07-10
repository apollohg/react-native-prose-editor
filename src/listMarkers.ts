/**
 * Rendered list-marker strings.
 *
 * CROSS-LAYER CONTRACT: these values must match the Rust core
 * (rust/editor-core/src/render/mod.rs: task_list_marker_string /
 * list_marker_string — pinned by marker_strings_are_the_cross_layer_contract
 * in render_test.rs), Android (LayoutConstants.TASK_LIST_MARKER_* /
 * UNORDERED_LIST_BULLET), and iOS (RenderBridge.listMarkerString). The
 * marker's scalar length feeds position mapping; a mismatch corrupts
 * selection and IME edits.
 */
export const TASK_LIST_MARKER_CHECKED = '☑ ';
export const TASK_LIST_MARKER_UNCHECKED = '☐ ';
export const UNORDERED_LIST_MARKER = '• ';

export function orderedListMarker(index: number): string {
    return `${index}. `;
}
