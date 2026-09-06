import './helpers/EditorToolbarFixture';
import { renderToolbar } from './helpers/EditorToolbarFixture';

import { fireEvent } from '@testing-library/react-native';

describe('EditorToolbar', () => {
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

        it('indent and outdent trust command availability for task lists', () => {
            const { getByLabelText, props } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: { taskList: true, taskItem: true },
                    commands: { indentList: true, outdentList: true },
                    allowedMarks: [],
                    insertableNodes: [],
                },
            });

            const indentButton = getByLabelText('Indent List');
            const outdentButton = getByLabelText('Outdent List');

            expect(indentButton.props.accessibilityState.disabled).toBeFalsy();
            expect(outdentButton.props.accessibilityState.disabled).toBeFalsy();

            fireEvent.press(indentButton);
            fireEvent.press(outdentButton);

            expect(props.onIndentList).toHaveBeenCalledTimes(1);
            expect(props.onOutdentList).toHaveBeenCalledTimes(1);
        });
    });
});
