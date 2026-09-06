import './helpers/EditorToolbarFixture';
import { renderToolbar } from './helpers/EditorToolbarFixture';

describe('EditorToolbar', () => {
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
                    insertableNodes: ['hard_break'],
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
                    insertableNodes: ['horizontal_rule'],
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

        it('maps snake_case bullet and ordered lists to their matching commands', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: {},
                    commands: { wrapBulletList: false, wrapOrderedList: true },
                    allowedMarks: [],
                    insertableNodes: [],
                },
            });

            expect(getByLabelText('Bullet List').props.accessibilityState.disabled).toBe(true);
            expect(getByLabelText('Ordered List').props.accessibilityState.disabled).toBeFalsy();
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
