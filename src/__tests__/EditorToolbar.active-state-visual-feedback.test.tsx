import './helpers/EditorToolbarFixture';
import { EMPTY_ACTIVE_STATE, renderToolbar } from './helpers/EditorToolbarFixture';

describe('EditorToolbar', () => {
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

        it('bullet list button gets selected state when bullet_list node is active', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: { bullet_list: true },
                    commands: {},
                    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
                    insertableNodes: ['horizontal_rule'],
                },
            });

            const bulletButton = getByLabelText('Bullet List');
            expect(bulletButton.props.accessibilityState).toEqual(
                expect.objectContaining({ selected: true })
            );
        });

        it('ordered list button gets selected state when ordered_list node is active', () => {
            const { getByLabelText } = renderToolbar({
                activeState: {
                    marks: {},
                    nodes: { ordered_list: true },
                    commands: {},
                    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
                    insertableNodes: ['horizontal_rule'],
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
            expect(getByLabelText('Underline').props.accessibilityState.selected).toBeFalsy();
        });
    });
});
