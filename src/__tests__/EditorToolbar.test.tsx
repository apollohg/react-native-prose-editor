// ─── EditorToolbar Tests ───────────────────────────────────────
// Tests for the formatting toolbar component.
//
// Tests cover:
// - Renders all 12 buttons (B, I, U, S, bullet list, ordered list, indent, outdent, line break, HR, undo, redo)
// - Active mark state shows visual feedback (bold: true -> button has active style)
// - Disabled state for undo/redo (canUndo: false -> undo button disabled)
// - Button presses fire correct callbacks
// - Uses Record<string, boolean> for ActiveState (not string[])
// ────────────────────────────────────────────────────────────────

import React from 'react';
import { StyleSheet } from 'react-native';
import { render, fireEvent } from '@testing-library/react-native';

import { EditorToolbar, type EditorToolbarProps } from '../EditorToolbar';
import type { ActiveState, HistoryState } from '../NativeEditorBridge';

// ─── Helpers ────────────────────────────────────────────────────

const EMPTY_ACTIVE_STATE: ActiveState = {
    marks: {},
    markAttrs: {},
    nodes: {},
    commands: {},
    allowedMarks: [],
    insertableNodes: [],
};
const ENABLED_BUTTONS_ACTIVE_STATE: ActiveState = {
    marks: {},
    markAttrs: {},
    nodes: {},
    commands: {
        wrapBulletList: true,
        wrapOrderedList: true,
    },
    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
    insertableNodes: ['hardBreak', 'horizontalRule'],
};
const EMPTY_HISTORY_STATE: HistoryState = { canUndo: false, canRedo: false };

function renderToolbar(
    overrides: Partial<Omit<EditorToolbarProps, 'activeState' | 'historyState'>> & {
        activeState?: Partial<ActiveState>;
        historyState?: Partial<HistoryState>;
    } = {}
) {
    const defaultProps: EditorToolbarProps = {
        activeState: {
            ...EMPTY_ACTIVE_STATE,
            ...overrides.activeState,
            marks: {
                ...EMPTY_ACTIVE_STATE.marks,
                ...overrides.activeState?.marks,
            },
            markAttrs: {
                ...EMPTY_ACTIVE_STATE.markAttrs,
                ...overrides.activeState?.markAttrs,
            },
            nodes: {
                ...EMPTY_ACTIVE_STATE.nodes,
                ...overrides.activeState?.nodes,
            },
            commands: {
                ...EMPTY_ACTIVE_STATE.commands,
                ...overrides.activeState?.commands,
            },
            allowedMarks: overrides.activeState?.allowedMarks ?? EMPTY_ACTIVE_STATE.allowedMarks,
            insertableNodes:
                overrides.activeState?.insertableNodes ?? EMPTY_ACTIVE_STATE.insertableNodes,
        },
        historyState: {
            ...EMPTY_HISTORY_STATE,
            ...overrides.historyState,
        },
        onToggleBold: jest.fn(),
        onToggleItalic: jest.fn(),
        onToggleUnderline: jest.fn(),
        onToggleStrike: jest.fn(),
        onToggleBlockquote: jest.fn(),
        onToggleBulletList: jest.fn(),
        onToggleHeading: jest.fn(),
        onToggleOrderedList: jest.fn(),
        onIndentList: jest.fn(),
        onOutdentList: jest.fn(),
        onInsertHorizontalRule: jest.fn(),
        onInsertLineBreak: jest.fn(),
        onUndo: jest.fn(),
        onRedo: jest.fn(),
        ...overrides,
    };
    return { ...render(<EditorToolbar {...defaultProps} />), props: defaultProps };
}

// ─── Tests ──────────────────────────────────────────────────────

describe('EditorToolbar', () => {
    // ── Rendering ───────────────────────────────────────────────

    describe('rendering', () => {
        it('renders all 13 buttons including blockquote and list depth controls', () => {
            const { getByLabelText } = renderToolbar();

            expect(getByLabelText('Bold')).toBeTruthy();
            expect(getByLabelText('Italic')).toBeTruthy();
            expect(getByLabelText('Underline')).toBeTruthy();
            expect(getByLabelText('Strikethrough')).toBeTruthy();
            expect(getByLabelText('Blockquote')).toBeTruthy();
            expect(getByLabelText('Bullet List')).toBeTruthy();
            expect(getByLabelText('Ordered List')).toBeTruthy();
            expect(getByLabelText('Indent List')).toBeTruthy();
            expect(getByLabelText('Outdent List')).toBeTruthy();
            expect(getByLabelText('Line Break')).toBeTruthy();
            expect(getByLabelText('Horizontal Rule')).toBeTruthy();
            expect(getByLabelText('Undo')).toBeTruthy();
            expect(getByLabelText('Redo')).toBeTruthy();
        });

        it('does not render list/depth/HR buttons when those callbacks are omitted', () => {
            const { queryByLabelText } = renderToolbar({
                onToggleBlockquote: undefined,
                onToggleBulletList: undefined,
                onToggleOrderedList: undefined,
                onIndentList: undefined,
                onOutdentList: undefined,
                onInsertLineBreak: undefined,
                onInsertHorizontalRule: undefined,
            });

            expect(queryByLabelText('Blockquote')).toBeNull();
            expect(queryByLabelText('Bullet List')).toBeNull();
            expect(queryByLabelText('Ordered List')).toBeNull();
            expect(queryByLabelText('Indent List')).toBeNull();
            expect(queryByLabelText('Outdent List')).toBeNull();
            expect(queryByLabelText('Line Break')).toBeNull();
            expect(queryByLabelText('Horizontal Rule')).toBeNull();
        });

        it('renders an image item when configured', () => {
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'image',
                        label: 'Image',
                        icon: { type: 'default', id: 'image' },
                    },
                ],
                activeState: {
                    insertableNodes: ['image'],
                },
                onRequestImage: jest.fn(),
            });

            expect(getByLabelText('Image')).toBeTruthy();
        });

        it('renders a heading item when configured', () => {
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'heading',
                        level: 2,
                        label: 'Heading 2',
                        icon: { type: 'default', id: 'h2' },
                    },
                ],
                activeState: {
                    commands: { toggleHeading2: true },
                },
            });

            expect(getByLabelText('Heading 2')).toBeTruthy();
        });

        it('renders grouped toolbar items as a single button until expanded', () => {
            const { getByLabelText, queryByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'group',
                        key: 'headings',
                        label: 'Headings',
                        icon: { type: 'glyph', text: 'H' },
                        items: [
                            {
                                type: 'heading',
                                level: 1,
                                label: 'Heading 1',
                                icon: { type: 'default', id: 'h1' },
                            },
                            {
                                type: 'heading',
                                level: 2,
                                label: 'Heading 2',
                                icon: { type: 'default', id: 'h2' },
                            },
                        ],
                    },
                ],
                activeState: {
                    commands: { toggleHeading1: true, toggleHeading2: true },
                },
            });

            expect(getByLabelText('Headings')).toBeTruthy();
            expect(queryByLabelText('Heading 1')).toBeNull();

            fireEvent.press(getByLabelText('Headings'));

            expect(getByLabelText('Heading 1')).toBeTruthy();
            expect(getByLabelText('Heading 2')).toBeTruthy();
        });

        it('marks a grouped button active when one of its children is active', () => {
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'group',
                        key: 'headings',
                        label: 'Headings',
                        icon: { type: 'glyph', text: 'H' },
                        items: [
                            {
                                type: 'heading',
                                level: 2,
                                label: 'Heading 2',
                                icon: { type: 'default', id: 'h2' },
                            },
                        ],
                    },
                ],
                activeState: {
                    nodes: { h2: true },
                    commands: { toggleHeading2: true },
                },
            });

            expect(getByLabelText('Headings').props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true, expanded: false })
            );
        });

        it('reports a grouped button as expanded only while its inline children are visible', () => {
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'group',
                        key: 'headings',
                        label: 'Headings',
                        icon: { type: 'glyph', text: 'H' },
                        items: [
                            {
                                type: 'heading',
                                level: 1,
                                label: 'Heading 1',
                                icon: { type: 'default', id: 'h1' },
                            },
                        ],
                    },
                ],
                activeState: {
                    commands: { toggleHeading1: true },
                },
            });

            const groupButton = getByLabelText('Headings');
            expect(groupButton.props.accessibilityState).toEqual(
                expect.objectContaining({ expanded: false })
            );

            fireEvent.press(groupButton);

            expect(getByLabelText('Headings').props.accessibilityState).toEqual(
                expect.objectContaining({ expanded: true })
            );
        });

        it('renders only the configured toolbar items and preserves order', () => {
            const onToggleMark = jest.fn();
            const onInsertNodeType = jest.fn();
            const { getAllByRole, queryByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'mark',
                        mark: 'bold',
                        label: 'Bold',
                        icon: { type: 'default', id: 'bold' },
                    },
                    {
                        type: 'mark',
                        mark: 'highlight',
                        label: 'Highlight',
                        icon: { type: 'glyph', text: 'H' },
                    },
                    { type: 'separator' },
                    {
                        type: 'node',
                        nodeType: 'mention',
                        label: 'Mention',
                        icon: {
                            type: 'platform',
                            ios: { type: 'sfSymbol', name: 'at' },
                            android: { type: 'material', name: 'alternate-email' },
                            fallbackText: '@',
                        },
                    },
                ],
                activeState: {
                    marks: { highlight: true },
                    nodes: {},
                    commands: {},
                    allowedMarks: ['bold', 'highlight'],
                    insertableNodes: ['mention'],
                },
                onToggleMark,
                onInsertNodeType,
            });

            expect(queryByLabelText('Italic')).toBeNull();
            expect(queryByLabelText('Undo')).toBeNull();

            const buttons = getAllByRole('button');
            expect(buttons.map((button) => button.props.accessibilityLabel)).toEqual([
                'Bold',
                'Highlight',
                'Mention',
            ]);
            expect(queryByLabelText('Highlight')?.props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true })
            );
        });

        it('renders custom action items with explicit active and disabled state', () => {
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'action',
                        key: 'insertMention',
                        label: 'Mention',
                        icon: {
                            type: 'platform',
                            ios: { type: 'sfSymbol', name: 'at' },
                            android: { type: 'material', name: 'alternate-email' },
                            fallbackText: '@',
                        },
                        isActive: true,
                        isDisabled: true,
                    },
                ],
                onToolbarAction: jest.fn(),
            });

            expect(getByLabelText('Mention').props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true, disabled: true })
            );
        });

        it('disables an image item when the schema does not allow image insertion', () => {
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'image',
                        label: 'Image',
                        icon: { type: 'default', id: 'image' },
                    },
                ],
                onRequestImage: jest.fn(),
            });

            expect(getByLabelText('Image').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
        });

        it('blockquote button gets selected state from active nodes', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    nodes: { blockquote: true },
                    commands: { toggleBlockquote: true },
                },
            });

            expect(getByLabelText('Blockquote').props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true, disabled: false })
            );
        });

        it('heading button gets selected state from active nodes', () => {
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'heading',
                        level: 3,
                        label: 'Heading 3',
                        icon: { type: 'default', id: 'h3' },
                    },
                ],
                activeState: {
                    nodes: { h3: true },
                    commands: { toggleHeading3: true },
                },
            });

            expect(getByLabelText('Heading 3').props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true, disabled: false })
            );
        });

        it('does not reapply a themed top border when showTopBorder is false', () => {
            const { toJSON } = renderToolbar({
                showTopBorder: false,
                theme: {
                    borderColor: '#123456',
                    borderWidth: 2,
                },
            });
            const tree = toJSON();
            const style = StyleSheet.flatten(tree?.props.style);

            expect(tree).not.toBeNull();
            expect(style.borderTopWidth).toBe(0);
        });

        it('uses theme.showTopBorder when the prop is omitted', () => {
            const { toJSON } = renderToolbar({
                theme: {
                    borderColor: '#123456',
                    borderWidth: 2,
                    showTopBorder: false,
                },
            });
            const tree = toJSON();
            const style = StyleSheet.flatten(tree?.props.style);

            expect(tree).not.toBeNull();
            expect(style.borderTopWidth).toBe(0);
        });
    });

    // ── Active State (Record<string, boolean>) ─────────────────

    describe('active state visual feedback', () => {
        it('bold button gets selected state when bold mark is active', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: { bold: true },
                    nodes: {},
                    commands: {},
                    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
                    insertableNodes: ['horizontalRule'],
                },
            });

            const boldButton = getByLabelText('Bold');
            expect(boldButton.props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true })
            );
        });

        it('italic button gets selected state when italic mark is active', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: { italic: true },
                    nodes: {},
                    commands: {},
                    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
                    insertableNodes: ['horizontalRule'],
                },
            });

            const italicButton = getByLabelText('Italic');
            expect(italicButton.props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true })
            );
        });

        it('underline button gets selected state when underline mark is active', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: { underline: true },
                    nodes: {},
                    commands: {},
                    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
                    insertableNodes: ['horizontalRule'],
                },
            });

            const underlineButton = getByLabelText('Underline');
            expect(underlineButton.props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true })
            );
        });

        it('strikethrough button gets selected state when strike mark is active', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: { strike: true },
                    nodes: {},
                    commands: {},
                    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
                    insertableNodes: ['horizontalRule'],
                },
            });

            const strikeButton = getByLabelText('Strikethrough');
            expect(strikeButton.props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true })
            );
        });

        it('bullet list button gets selected state when bulletList node is active', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: { bulletList: true },
                    commands: {},
                    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
                    insertableNodes: ['horizontalRule'],
                },
            });

            const bulletButton = getByLabelText('Bullet List');
            expect(bulletButton.props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true })
            );
        });

        it('ordered list button gets selected state when orderedList node is active', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: { orderedList: true },
                    commands: {},
                    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
                    insertableNodes: ['horizontalRule'],
                },
            });

            const orderedButton = getByLabelText('Ordered List');
            expect(orderedButton.props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true })
            );
        });

        it('link button gets selected state when link mark is active', () => {
            const onRequestLink = jest.fn();
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    { type: 'link', label: 'Link', icon: { type: 'default', id: 'link' } },
                ],
                activeState: {
                    marks: { link: true },
                    markAttrs: { link: { href: 'https://example.com' } },
                    nodes: {},
                    commands: {},
                    allowedMarks: ['link'],
                    insertableNodes: [],
                },
                onRequestLink,
            });

            const linkButton = getByLabelText('Link');
            expect(linkButton.props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true })
            );
        });

        it('buttons are not selected when their marks/nodes are absent from ActiveState', () => {
            const { getByLabelText } = renderToolbar({
                activeState: EMPTY_ACTIVE_STATE,
            });

            const boldButton = getByLabelText('Bold');
            // RN normalizes undefined -> false for accessibilityState booleans
            expect(boldButton.props.accessibilityState.selected).toBeFalsy();
        });

        it('multiple marks can be active simultaneously', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: { bold: true, italic: true },
                    nodes: {},
                    commands: {},
                    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
                    insertableNodes: ['horizontalRule'],
                },
            });

            expect(getByLabelText('Bold').props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true })
            );
            expect(getByLabelText('Italic').props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true })
            );
            // Underline should not be selected
            expect(getByLabelText('Underline').props.accessibilityState.selected).toBeFalsy();
        });
    });

    // ── Disabled State ─────────────────────────────────────────

    describe('disabled state for undo/redo', () => {
        it('undo button is disabled when canUndo is false', () => {
            const { getByLabelText } = renderToolbar({
                historyState: { canUndo: false, canRedo: false },
            });

            const undoButton = getByLabelText('Undo');
            expect(undoButton.props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
        });

        it('undo button is enabled when canUndo is true', () => {
            const { getByLabelText } = renderToolbar({
                historyState: { canUndo: true, canRedo: false },
            });

            const undoButton = getByLabelText('Undo');
            // RN normalizes undefined -> false for accessibilityState booleans
            expect(undoButton.props.accessibilityState.disabled).toBeFalsy();
        });

        it('redo button is disabled when canRedo is false', () => {
            const { getByLabelText } = renderToolbar({
                historyState: { canUndo: false, canRedo: false },
            });

            const redoButton = getByLabelText('Redo');
            expect(redoButton.props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
        });

        it('redo button is enabled when canRedo is true', () => {
            const { getByLabelText } = renderToolbar({
                historyState: { canUndo: false, canRedo: true },
            });

            const redoButton = getByLabelText('Redo');
            // RN normalizes undefined -> false for accessibilityState booleans
            expect(redoButton.props.accessibilityState.disabled).toBeFalsy();
        });

        it('indent and outdent are disabled when selection is not in a list', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
            });

            expect(getByLabelText('Indent List').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
            expect(getByLabelText('Outdent List').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
        });

        it('indent is disabled on the first list item and outdent respects command availability', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: { bulletList: true, listItem: true },
                    commands: { indentList: false, outdentList: true },
                    allowedMarks: [],
                    insertableNodes: [],
                },
            });

            expect(getByLabelText('Indent List').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
            expect(getByLabelText('Outdent List').props.accessibilityState.disabled).toBeFalsy();
        });

        it('indent and outdent are enabled when both list commands are available', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: { bulletList: true, listItem: true },
                    commands: { indentList: true, outdentList: true },
                    allowedMarks: [],
                    insertableNodes: [],
                },
            });

            expect(getByLabelText('Indent List').props.accessibilityState.disabled).toBeFalsy();
            expect(getByLabelText('Outdent List').props.accessibilityState.disabled).toBeFalsy();
        });
    });

    // ── Button Presses ─────────────────────────────────────────

    describe('button press callbacks', () => {
        it('bold button fires onToggleBold', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: ENABLED_BUTTONS_ACTIVE_STATE,
            });

            fireEvent.press(getByLabelText('Bold'));

            expect(props.onToggleBold).toHaveBeenCalledTimes(1);
        });

        it('italic button fires onToggleItalic', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: ENABLED_BUTTONS_ACTIVE_STATE,
            });

            fireEvent.press(getByLabelText('Italic'));

            expect(props.onToggleItalic).toHaveBeenCalledTimes(1);
        });

        it('underline button fires onToggleUnderline', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: ENABLED_BUTTONS_ACTIVE_STATE,
            });

            fireEvent.press(getByLabelText('Underline'));

            expect(props.onToggleUnderline).toHaveBeenCalledTimes(1);
        });

        it('strikethrough button fires onToggleStrike', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: ENABLED_BUTTONS_ACTIVE_STATE,
            });

            fireEvent.press(getByLabelText('Strikethrough'));

            expect(props.onToggleStrike).toHaveBeenCalledTimes(1);
        });

        it('bullet list button fires onToggleBulletList', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: ENABLED_BUTTONS_ACTIVE_STATE,
            });

            fireEvent.press(getByLabelText('Bullet List'));

            expect(props.onToggleBulletList).toHaveBeenCalledTimes(1);
        });

        it('blockquote button fires onToggleBlockquote', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: {
                    ...ENABLED_BUTTONS_ACTIVE_STATE,
                    commands: {
                        ...ENABLED_BUTTONS_ACTIVE_STATE.commands,
                        toggleBlockquote: true,
                    },
                },
            });

            fireEvent.press(getByLabelText('Blockquote'));

            expect(props.onToggleBlockquote).toHaveBeenCalledTimes(1);
        });

        it('heading button fires onToggleHeading', () => {
            const { getByLabelText, props } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'heading',
                        level: 4,
                        label: 'Heading 4',
                        icon: { type: 'default', id: 'h4' },
                    },
                ],
                activeState: {
                    commands: { toggleHeading4: true },
                },
            });

            fireEvent.press(getByLabelText('Heading 4'));

            expect(props.onToggleHeading).toHaveBeenCalledWith(4);
        });

        it('ordered list button fires onToggleOrderedList', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: ENABLED_BUTTONS_ACTIVE_STATE,
            });

            fireEvent.press(getByLabelText('Ordered List'));

            expect(props.onToggleOrderedList).toHaveBeenCalledTimes(1);
        });

        it('horizontal rule button fires onInsertHorizontalRule', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: ENABLED_BUTTONS_ACTIVE_STATE,
            });

            fireEvent.press(getByLabelText('Horizontal Rule'));

            expect(props.onInsertHorizontalRule).toHaveBeenCalledTimes(1);
        });

        it('line break button fires onInsertLineBreak', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: {},
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: ['hardBreak'],
                },
            });

            fireEvent.press(getByLabelText('Line Break'));

            expect(props.onInsertLineBreak).toHaveBeenCalledTimes(1);
        });

        it('image button fires onRequestImage when image insertion is allowed', () => {
            const onRequestImage = jest.fn();
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'image',
                        label: 'Image',
                        icon: { type: 'default', id: 'image' },
                    },
                ],
                activeState: {
                    insertableNodes: ['image'],
                },
                onRequestImage,
            });

            fireEvent.press(getByLabelText('Image'));

            expect(onRequestImage).toHaveBeenCalledTimes(1);
        });

        it('custom mark and node buttons use the generic handlers', () => {
            const onToggleMark = jest.fn();
            const onInsertNodeType = jest.fn();
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'mark',
                        mark: 'highlight',
                        label: 'Highlight',
                        icon: { type: 'glyph', text: 'H' },
                    },
                    {
                        type: 'node',
                        nodeType: 'mention',
                        label: 'Mention',
                        icon: {
                            type: 'platform',
                            ios: { type: 'sfSymbol', name: 'at' },
                            android: { type: 'material', name: 'alternate-email' },
                            fallbackText: '@',
                        },
                    },
                ],
                activeState: {
                    marks: {},
                    nodes: {},
                    commands: {},
                    allowedMarks: ['highlight'],
                    insertableNodes: ['mention'],
                },
                onToggleMark,
                onInsertNodeType,
            });

            fireEvent.press(getByLabelText('Highlight'));
            fireEvent.press(getByLabelText('Mention'));

            expect(onToggleMark).toHaveBeenCalledWith('highlight');
            expect(onInsertNodeType).toHaveBeenCalledWith('mention');
        });

        it('custom action buttons use onToolbarAction', () => {
            const onToolbarAction = jest.fn();
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'action',
                        key: 'insertMention',
                        label: 'Mention',
                        icon: {
                            type: 'platform',
                            ios: { type: 'sfSymbol', name: 'at' },
                            android: { type: 'material', name: 'alternate-email' },
                            fallbackText: '@',
                        },
                    },
                ],
                onToolbarAction,
            });

            fireEvent.press(getByLabelText('Mention'));

            expect(onToolbarAction).toHaveBeenCalledWith('insertMention');
        });

        it('indent list button fires onIndentList when enabled', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: { bulletList: true, listItem: true },
                    commands: { indentList: true, outdentList: false },
                    allowedMarks: [],
                    insertableNodes: [],
                },
            });

            fireEvent.press(getByLabelText('Indent List'));

            expect(props.onIndentList).toHaveBeenCalledTimes(1);
        });

        it('outdent list button fires onOutdentList when enabled', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: { orderedList: true, listItem: true },
                    commands: { indentList: false, outdentList: true },
                    allowedMarks: [],
                    insertableNodes: [],
                },
            });

            fireEvent.press(getByLabelText('Outdent List'));

            expect(props.onOutdentList).toHaveBeenCalledTimes(1);
        });

        it('undo button fires onUndo when enabled', () => {
            const { getByLabelText, props } = renderToolbar({
                historyState: { canUndo: true, canRedo: false },
            });

            fireEvent.press(getByLabelText('Undo'));

            expect(props.onUndo).toHaveBeenCalledTimes(1);
        });

        it('redo button fires onRedo when enabled', () => {
            const { getByLabelText, props } = renderToolbar({
                historyState: { canUndo: false, canRedo: true },
            });

            fireEvent.press(getByLabelText('Redo'));

            expect(props.onRedo).toHaveBeenCalledTimes(1);
        });
    });

    // ── Schema-Aware Disabled State ─────────────────────────────

    describe('schema-aware disabled state', () => {
        it('mark buttons are disabled when not in allowedMarks', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: {},
                    commands: {},
                    allowedMarks: ['bold', 'italic'],
                    insertableNodes: [],
                },
            });

            expect(getByLabelText('Bold').props.accessibilityState.disabled).toBeFalsy();
            expect(getByLabelText('Italic').props.accessibilityState.disabled).toBeFalsy();
            expect(getByLabelText('Underline').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
            expect(getByLabelText('Strikethrough').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
        });

        it('link button stays enabled inside headings when link is allowed', () => {
            const onRequestLink = jest.fn();
            const { getByLabelText } = renderToolbar({
                toolbarItems: [
                    { type: 'link', label: 'Link', icon: { type: 'default', id: 'link' } },
                ],
                activeState: {
                    marks: {},
                    nodes: { h2: true },
                    commands: {},
                    allowedMarks: ['link'],
                    insertableNodes: [],
                },
                onRequestLink,
            });

            expect(getByLabelText('Link').props.accessibilityState.disabled).toBeFalsy();
        });

        it('horizontal rule is disabled when not in insertableNodes', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: {},
                    commands: {},
                    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
                    insertableNodes: [],
                },
            });

            expect(getByLabelText('Horizontal Rule').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
        });

        it('line break is disabled when not in insertableNodes', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: {},
                    commands: {},
                    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
                    insertableNodes: [],
                },
            });

            expect(getByLabelText('Line Break').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
        });

        it('line break is enabled when in insertableNodes', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: {},
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: ['hardBreak'],
                },
            });

            expect(getByLabelText('Line Break').props.accessibilityState.disabled).toBeFalsy();
        });

        it('horizontal rule is enabled when in insertableNodes', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: {},
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: ['horizontalRule'],
                },
            });

            expect(getByLabelText('Horizontal Rule').props.accessibilityState.disabled).toBeFalsy();
        });

        it('horizontal rule stays disabled inside lists when insertableNodes excludes it', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: { bulletList: true, listItem: true },
                    commands: { wrapBulletList: true, wrapOrderedList: true },
                    allowedMarks: [],
                    insertableNodes: [],
                },
            });

            expect(getByLabelText('Horizontal Rule').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
        });

        it('list toggle buttons are disabled when commands say so', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: {},
                    commands: { wrapBulletList: false, wrapOrderedList: false },
                    allowedMarks: [],
                    insertableNodes: [],
                },
            });

            expect(getByLabelText('Bullet List').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
            expect(getByLabelText('Ordered List').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
        });

        it('list toggle buttons are enabled when commands say so', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: {},
                    commands: { wrapBulletList: true, wrapOrderedList: true },
                    allowedMarks: [],
                    insertableNodes: [],
                },
            });

            expect(getByLabelText('Bullet List').props.accessibilityState.disabled).toBeFalsy();
            expect(getByLabelText('Ordered List').props.accessibilityState.disabled).toBeFalsy();
        });

        it('buttons degrade gracefully with empty allowedMarks and insertableNodes', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: {},
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
            });

            expect(getByLabelText('Bold').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
            expect(getByLabelText('Italic').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
            expect(getByLabelText('Line Break').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
            expect(getByLabelText('Horizontal Rule').props.accessibilityState).toEqual(
                expect.objectContaining({ disabled: true })
            );
        });
    });
});
