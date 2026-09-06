import './helpers/EditorToolbarFixture';
import { ENABLED_BUTTONS_ACTIVE_STATE, renderToolbar } from './helpers/EditorToolbarFixture';

import { fireEvent } from '@testing-library/react-native';

describe('EditorToolbar', () => {
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
                    insertableNodes: ['hard_break'],
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
});
