import { type EditorToolbarProps } from './EditorToolbarTypes';
import { useEditorToolbarState } from './useEditorToolbarState';
import { useEditorToolbarItems } from './useEditorToolbarItems';
import { useEditorToolbarInteractions } from './useEditorToolbarInteractions';
import { useEditorToolbarPresentation } from './useEditorToolbarPresentation';

/**
 * A JavaScript formatting toolbar. `RichTextEditor` renders one for you
 * when `showToolbar` is set — reach for this component directly only to place
 * the toolbar somewhere the editor cannot, in which case wire every handler
 * to the editor's ref and feed it `activeState` and `historyState` from the
 * editor's callbacks.
 *
 * The native keyboard-attached toolbar (`toolbarPlacement="keyboard"`) is a
 * separate implementation that consumes the same {@link EditorToolbarItem}
 * list.
 */
export function EditorToolbar(props: EditorToolbarProps) {
    const state = useEditorToolbarState(props);
    const items = useEditorToolbarItems(state);
    const interactions = useEditorToolbarInteractions({ ...state, ...items });
    return useEditorToolbarPresentation({ ...state, ...items, ...interactions });
}
