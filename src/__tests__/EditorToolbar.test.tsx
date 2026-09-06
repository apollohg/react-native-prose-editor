import './helpers/EditorToolbarFixture';
import { renderToolbar } from './helpers/EditorToolbarFixture';

import { ScrollView, StyleSheet, View } from 'react-native';
import { fireEvent } from '@testing-library/react-native';
import { DEFAULT_EDITOR_TOOLBAR_ITEMS } from '../EditorToolbar';

describe('EditorToolbar', () => {
    describe('rendering', () => {
        it('uses ProseMirror node names in the default toolbar items', () => {
            expect(DEFAULT_EDITOR_TOOLBAR_ITEMS).toEqual(
                expect.arrayContaining([
                    expect.objectContaining({ type: 'list', listType: 'bullet_list' }),
                    expect.objectContaining({ type: 'list', listType: 'ordered_list' }),
                    expect.objectContaining({ type: 'node', nodeType: 'hard_break' }),
                    expect.objectContaining({ type: 'node', nodeType: 'horizontal_rule' }),
                ])
            );
        });

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

        it('allows expanded grouped children to override the parent placement', () => {
            const { getByLabelText, UNSAFE_getAllByType } = renderToolbar({
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
                                label: 'Pinned Heading',
                                icon: { type: 'default', id: 'h2' },
                                placement: 'end',
                            },
                        ],
                    },
                ],
                activeState: {
                    commands: { toggleHeading2: true },
                },
            });

            fireEvent.press(getByLabelText('Headings'));

            const scrollButtonLabels = UNSAFE_getAllByType(ScrollView).flatMap((scrollView) =>
                scrollView
                    .findAllByProps({ accessibilityRole: 'button' })
                    .map((button) => button.props.accessibilityLabel)
            );
            expect(scrollButtonLabels).toContain('Headings');
            expect(scrollButtonLabels).not.toContain('Pinned Heading');
            expect(getByLabelText('Pinned Heading')).toBeTruthy();
        });

        it('preserves the outer horizontal inset for pinned placements', () => {
            const { UNSAFE_getAllByType } = renderToolbar({
                toolbarItems: [
                    {
                        type: 'action',
                        key: 'start',
                        label: 'Start',
                        icon: { type: 'glyph', text: 'S' },
                        placement: 'start',
                    },
                    {
                        type: 'action',
                        key: 'end',
                        label: 'End',
                        icon: { type: 'glyph', text: 'E' },
                        placement: 'end',
                    },
                ],
                onToolbarAction: jest.fn(),
            });

            const fixedSections = UNSAFE_getAllByType(View).filter(
                (view) => StyleSheet.flatten(view.props.style)?.flexShrink === 0
            );
            const sectionContaining = (label: string) =>
                fixedSections.find(
                    (section) => section.findAllByProps({ accessibilityLabel: label }).length > 0
                );

            expect(StyleSheet.flatten(sectionContaining('Start')?.props.style)).toEqual(
                expect.objectContaining({ paddingStart: 12 })
            );
            expect(StyleSheet.flatten(sectionContaining('End')?.props.style)).toEqual(
                expect.objectContaining({ paddingEnd: 12 })
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
});
