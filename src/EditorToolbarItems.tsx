import {
    type EditorToolbarDefaultIconId,
    type EditorToolbarIcon,
    type EditorToolbarItemPlacement,
    type EditorToolbarItem,
} from './EditorToolbarTypes';

export function defaultIcon(id: EditorToolbarDefaultIconId): EditorToolbarIcon {
    return { type: 'default', id };
}

export function resolveToolbarItemPlacement(
    placement: EditorToolbarItemPlacement | undefined
): EditorToolbarItemPlacement {
    return placement ?? 'scroll';
}

/**
 * The toolbar layout used when `toolbarItems` is not supplied. Spread it to
 * extend the built-in set rather than restating it:
 * `[...DEFAULT_EDITOR_TOOLBAR_ITEMS, myItem]`.
 */
export const DEFAULT_EDITOR_TOOLBAR_ITEMS: readonly EditorToolbarItem[] = [
    { type: 'mark', mark: 'bold', label: 'Bold', icon: defaultIcon('bold') },
    { type: 'mark', mark: 'italic', label: 'Italic', icon: defaultIcon('italic') },
    { type: 'mark', mark: 'underline', label: 'Underline', icon: defaultIcon('underline') },
    { type: 'mark', mark: 'strike', label: 'Strikethrough', icon: defaultIcon('strike') },
    { type: 'blockquote', label: 'Blockquote', icon: defaultIcon('blockquote') },
    { type: 'separator' },
    {
        type: 'list',
        listType: 'bullet_list',
        label: 'Bullet List',
        icon: defaultIcon('bulletList'),
    },
    {
        type: 'list',
        listType: 'ordered_list',
        label: 'Ordered List',
        icon: defaultIcon('orderedList'),
    },
    {
        type: 'command',
        command: 'indentList',
        label: 'Indent List',
        icon: defaultIcon('indentList'),
    },
    {
        type: 'command',
        command: 'outdentList',
        label: 'Outdent List',
        icon: defaultIcon('outdentList'),
    },
    { type: 'node', nodeType: 'hard_break', label: 'Line Break', icon: defaultIcon('lineBreak') },
    {
        type: 'node',
        nodeType: 'horizontal_rule',
        label: 'Horizontal Rule',
        icon: defaultIcon('horizontalRule'),
    },
    { type: 'separator' },
    { type: 'command', command: 'undo', label: 'Undo', icon: defaultIcon('undo') },
    { type: 'command', command: 'redo', label: 'Redo', icon: defaultIcon('redo') },
] as const;
