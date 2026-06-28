// ─── NativeRichTextEditor Tests ────────────────────────────────
// Tests for the React component wrapper around the native view.
// Both the native module and native view manager are mocked.
//
// Tests cover:
// - Rendering and bridge creation
// - Props passthrough to native view
// - Ref methods (all go through runAndApply -> applyEditorUpdate)
// - Controlled mode (value prop diffing, suppressContentCallbacks)
// - Callbacks (onActiveStateChange, onContentChangeJSON)
// - Cleanup on unmount
// - getBridge() does NOT exist on ref
// ────────────────────────────────────────────────────────────────

// ─── Mock Constants ─────────────────────────────────────────────

const MOCK_EMPTY_UPDATE_JSON = JSON.stringify({
    renderElements: [],
    selection: { type: 'text', anchor: 0, head: 0 },
    activeState: {
        marks: {},
        markAttrs: {},
        nodes: {},
        commands: {},
        allowedMarks: [],
        insertableNodes: [],
    },
    historyState: { canUndo: false, canRedo: false },
    documentVersion: 1,
});

const MOCK_BOLD_UPDATE_JSON = JSON.stringify({
    renderElements: [],
    selection: { type: 'text', anchor: 0, head: 5 },
    activeState: {
        marks: { bold: true },
        markAttrs: {},
        nodes: { paragraph: true },
        commands: {},
        allowedMarks: [],
        insertableNodes: [],
    },
    historyState: { canUndo: true, canRedo: false },
    documentVersion: 2,
});

const MOCK_COLLAPSED_BOLD_UPDATE_JSON = JSON.stringify({
    renderElements: [],
    selection: { type: 'text', anchor: 3, head: 3 },
    activeState: {
        marks: { bold: true },
        markAttrs: {},
        nodes: { paragraph: true },
        commands: {},
        allowedMarks: [],
        insertableNodes: [],
    },
    historyState: { canUndo: true, canRedo: false },
    documentVersion: 1,
});

const MOCK_LIST_UPDATE_JSON = JSON.stringify({
    renderElements: [],
    selection: { type: 'text', anchor: 0, head: 0 },
    activeState: {
        marks: {},
        markAttrs: {},
        nodes: { bulletList: true },
        commands: { indentList: true, outdentList: false },
        allowedMarks: [],
        insertableNodes: [],
    },
    historyState: { canUndo: true, canRedo: false },
    documentVersion: 2,
});

const MOCK_ORDERED_LIST_UPDATE_JSON = JSON.stringify({
    renderElements: [],
    selection: { type: 'text', anchor: 0, head: 0 },
    activeState: {
        marks: {},
        markAttrs: {},
        nodes: { orderedList: true },
        commands: { indentList: true, outdentList: false },
        allowedMarks: [],
        insertableNodes: [],
    },
    historyState: { canUndo: true, canRedo: false },
    documentVersion: 3,
});

const MOCK_NODE_UPDATE_JSON = JSON.stringify({
    renderElements: [{ type: 'voidBlock', nodeType: 'horizontalRule', docPos: 0 }],
    selection: { type: 'text', anchor: 1, head: 1 },
    activeState: {
        marks: {},
        markAttrs: {},
        nodes: {},
        commands: {},
        allowedMarks: [],
        insertableNodes: [],
    },
    historyState: { canUndo: true, canRedo: false },
    documentVersion: 2,
});

const MOCK_INSERT_UPDATE_JSON = JSON.stringify({
    renderElements: [],
    selection: { type: 'text', anchor: 5, head: 5 },
    activeState: {
        marks: {},
        markAttrs: {},
        nodes: { paragraph: true },
        commands: {},
        allowedMarks: [],
        insertableNodes: [],
    },
    historyState: { canUndo: true, canRedo: false },
});

const MOCK_AUTO_LINK_SOURCE_UPDATE_JSON = JSON.stringify({
    renderElements: [],
    selection: { type: 'text', anchor: 27, head: 27 },
    activeState: {
        marks: {},
        markAttrs: {},
        nodes: { paragraph: true },
        commands: {},
        allowedMarks: ['link'],
        insertableNodes: [],
    },
    historyState: { canUndo: true, canRedo: false },
    documentVersion: 5,
});

const MOCK_AUTO_LINKED_UPDATE_JSON = JSON.stringify({
    renderElements: [],
    selection: { type: 'text', anchor: 27, head: 27 },
    activeState: {
        marks: {},
        markAttrs: {},
        nodes: { paragraph: true },
        commands: {},
        allowedMarks: ['link'],
        insertableNodes: [],
    },
    historyState: { canUndo: true, canRedo: false },
    documentVersion: 6,
});

const MOCK_UNDO_UPDATE_JSON = JSON.stringify({
    renderElements: [],
    selection: { type: 'text', anchor: 0, head: 0 },
    activeState: {
        marks: {},
        markAttrs: {},
        nodes: {},
        commands: {},
        allowedMarks: [],
        insertableNodes: [],
    },
    historyState: { canUndo: false, canRedo: true },
    documentVersion: 1,
});

const MOCK_REDO_UPDATE_JSON = JSON.stringify({
    renderElements: [],
    selection: { type: 'text', anchor: 3, head: 3 },
    activeState: {
        marks: {},
        markAttrs: {},
        nodes: {},
        commands: {},
        allowedMarks: [],
        insertableNodes: [],
    },
    historyState: { canUndo: true, canRedo: false },
    documentVersion: 2,
});

const MOCK_DOCUMENT_JSON_STR = JSON.stringify({
    type: 'doc',
    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello' }] }],
});

const NORMALIZED_EMPTY_DOC = {
    type: 'doc',
    content: [{ type: 'paragraph' }],
};

const TITLE_FIRST_SCHEMA = {
    nodes: [
        { name: 'doc', content: 'title block*', role: 'doc' },
        {
            name: 'title',
            content: 'inline*',
            group: 'block',
            role: 'textBlock',
            htmlTag: 'h1',
        },
        {
            name: 'paragraph',
            content: 'inline*',
            group: 'block',
            role: 'textBlock',
            htmlTag: 'p',
        },
        { name: 'text', content: '', group: 'inline', role: 'text' },
    ],
    marks: [],
};

const TITLE_EMPTY_DOC = {
    type: 'doc',
    content: [{ type: 'title' }],
};

// ─── Mock Setup (must be before imports) ────────────────────────

let mockEditorIdCounter = 0;
const mockApplyEditorUpdate = jest.fn();
const mockApplyEditorResetUpdate = jest.fn();
const mockNativeFocus = jest.fn();
const mockNativeBlur = jest.fn();
const mockNativeGetCaretRect = jest.fn();

const mockNativeModule = {
    editorCreate: jest.fn((_configJson: string) => ++mockEditorIdCounter),
    editorDestroy: jest.fn(),
    editorPrepareForCommand: jest.fn(() => '{"ready":true}'),
    editorSetHtml: jest.fn(() => '[]'),
    editorGetHtml: jest.fn(() => '<p>test content</p>'),
    editorSetJson: jest.fn(() => '[]'),
    editorGetJson: jest.fn(() => MOCK_DOCUMENT_JSON_STR),
    editorGetContentSnapshot: jest.fn(() =>
        JSON.stringify({
            html: '<p>test content</p>',
            json: JSON.parse(MOCK_DOCUMENT_JSON_STR),
        })
    ),
    editorInsertText: jest.fn(() => MOCK_INSERT_UPDATE_JSON),
    editorReplaceSelectionText: jest.fn(() => MOCK_INSERT_UPDATE_JSON),
    editorDeleteRange: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorToggleMark: jest.fn(() => MOCK_BOLD_UPDATE_JSON),
    editorSetMark: jest.fn(() => MOCK_BOLD_UPDATE_JSON),
    editorUnsetMark: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorToggleHeading: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorSetSelection: jest.fn(),
    editorGetSelection: jest.fn(() => JSON.stringify({ type: 'text', anchor: 5, head: 5 })),
    editorGetSelectionState: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorGetCurrentState: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorSplitBlock: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorInsertContentHtml: jest.fn(() => MOCK_INSERT_UPDATE_JSON),
    editorInsertContentJson: jest.fn(() => MOCK_INSERT_UPDATE_JSON),
    editorInsertContentJsonAtSelectionScalar: jest.fn(() => MOCK_INSERT_UPDATE_JSON),
    editorReplaceHtml: jest.fn(() => MOCK_INSERT_UPDATE_JSON),
    editorReplaceJson: jest.fn(() => MOCK_INSERT_UPDATE_JSON),
    // Scalar-position APIs
    editorInsertTextScalar: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorDeleteScalarRange: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorReplaceTextScalar: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorSplitBlockScalar: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorDeleteAndSplitScalar: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorSetSelectionScalar: jest.fn(),
    editorToggleMarkAtSelectionScalar: jest.fn(() => MOCK_BOLD_UPDATE_JSON),
    editorSetMarkAtSelectionScalar: jest.fn(() => MOCK_BOLD_UPDATE_JSON),
    editorUnsetMarkAtSelectionScalar: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorToggleBlockquoteAtSelectionScalar: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorToggleHeadingAtSelectionScalar: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorWrapInListAtSelectionScalar: jest.fn(() => MOCK_LIST_UPDATE_JSON),
    editorUnwrapFromListAtSelectionScalar: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorIndentListItemAtSelectionScalar: jest.fn(() => MOCK_LIST_UPDATE_JSON),
    editorOutdentListItemAtSelectionScalar: jest.fn(() => MOCK_LIST_UPDATE_JSON),
    editorInsertNodeAtSelectionScalar: jest.fn(() => MOCK_NODE_UPDATE_JSON),
    editorDocToScalar: jest.fn((_: number, pos: number) => pos),
    editorScalarToDoc: jest.fn((_: number, scalar: number) => scalar),
    // List / node APIs
    editorWrapInList: jest.fn(() => MOCK_LIST_UPDATE_JSON),
    editorUnwrapFromList: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorIndentListItem: jest.fn(() => MOCK_LIST_UPDATE_JSON),
    editorOutdentListItem: jest.fn(() => MOCK_LIST_UPDATE_JSON),
    editorInsertNode: jest.fn(() => MOCK_NODE_UPDATE_JSON),
    editorUndo: jest.fn(() => MOCK_UNDO_UPDATE_JSON),
    editorRedo: jest.fn(() => MOCK_REDO_UPDATE_JSON),
    editorCanUndo: jest.fn(() => true),
    editorCanRedo: jest.fn(() => false),
};

jest.mock('expo-modules-core', () => {
    const React = require('react');
    const { View } = require('react-native');

    const MockNativeView = React.forwardRef(
        (props: Record<string, unknown>, ref: React.Ref<unknown>) => {
            React.useImperativeHandle(ref, () => ({
                focus: mockNativeFocus,
                blur: mockNativeBlur,
                getCaretRect: mockNativeGetCaretRect,
                applyEditorUpdate: mockApplyEditorUpdate,
                applyEditorResetUpdate: mockApplyEditorResetUpdate,
            }));
            return React.createElement(View, { testID: 'native-editor-view', ...props });
        }
    );
    MockNativeView.displayName = 'MockNativeView';

    return {
        requireNativeModule: () => mockNativeModule,
        requireNativeViewManager: () => MockNativeView,
    };
});

// ─── Imports (after mock setup) ─────────────────────────────────

import React, { createRef } from 'react';
import { render, act, fireEvent } from '@testing-library/react-native';
import { PixelRatio, Platform, StyleSheet, View } from 'react-native';

import { NativeRichTextEditor, type NativeRichTextEditorRef } from '../NativeRichTextEditor';
import {
    EditorToolbar,
    _beginEditorToolbarInteractionForTests,
    _resetEditorToolbarFrameRegistryForTests,
    _setEditorToolbarFrameForTests,
} from '../EditorToolbar';
import { _resetNativeModuleCache } from '../NativeEditorBridge';

// ─── Tests ──────────────────────────────────────────────────────

describe('NativeRichTextEditor', () => {
    beforeEach(() => {
        _resetNativeModuleCache();
        _resetEditorToolbarFrameRegistryForTests();
        mockEditorIdCounter = 0;
        mockApplyEditorUpdate.mockClear();
        mockApplyEditorResetUpdate.mockClear();
        mockNativeFocus.mockClear();
        mockNativeBlur.mockClear();
        mockNativeGetCaretRect.mockReset();

        // Reset all mocks to defaults (mockClear only clears call history,
        // mockReset also clears return values and implementations)
        for (const key of Object.keys(mockNativeModule)) {
            (mockNativeModule as Record<string, jest.Mock>)[key].mockReset();
        }

        // Re-establish default return values
        mockNativeModule.editorCreate.mockImplementation(
            (_configJson: string) => ++mockEditorIdCounter
        );
        mockNativeModule.editorPrepareForCommand.mockReturnValue('{"ready":true}');
        mockNativeModule.editorSetHtml.mockReturnValue('[]');
        mockNativeModule.editorGetHtml.mockReturnValue('<p>test content</p>');
        mockNativeModule.editorSetJson.mockReturnValue('[]');
        mockNativeModule.editorGetJson.mockReturnValue(MOCK_DOCUMENT_JSON_STR);
        mockNativeModule.editorGetContentSnapshot.mockReturnValue(
            JSON.stringify({
                html: '<p>test content</p>',
                json: JSON.parse(MOCK_DOCUMENT_JSON_STR),
            })
        );
        mockNativeModule.editorInsertText.mockReturnValue(MOCK_INSERT_UPDATE_JSON);
        mockNativeModule.editorReplaceSelectionText.mockReturnValue(MOCK_INSERT_UPDATE_JSON);
        mockNativeModule.editorDeleteRange.mockReturnValue(MOCK_EMPTY_UPDATE_JSON);
        mockNativeModule.editorToggleMark.mockReturnValue(MOCK_BOLD_UPDATE_JSON);
        mockNativeModule.editorToggleMarkAtSelectionScalar.mockReturnValue(MOCK_BOLD_UPDATE_JSON);
        mockNativeModule.editorGetSelection.mockReturnValue(
            JSON.stringify({ type: 'text', anchor: 5, head: 5 })
        );
        mockNativeModule.editorGetSelectionState.mockReturnValue(MOCK_EMPTY_UPDATE_JSON);
        mockNativeModule.editorGetCurrentState.mockReturnValue(MOCK_EMPTY_UPDATE_JSON);
        mockNativeModule.editorSplitBlock.mockReturnValue(MOCK_EMPTY_UPDATE_JSON);
        mockNativeModule.editorInsertContentHtml.mockReturnValue(MOCK_INSERT_UPDATE_JSON);
        mockNativeModule.editorInsertContentJson.mockReturnValue(MOCK_INSERT_UPDATE_JSON);
        mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockReturnValue(
            MOCK_INSERT_UPDATE_JSON
        );
        mockNativeModule.editorReplaceHtml.mockReturnValue(MOCK_INSERT_UPDATE_JSON);
        mockNativeModule.editorReplaceJson.mockReturnValue(MOCK_INSERT_UPDATE_JSON);
        mockNativeModule.editorInsertTextScalar.mockReturnValue(MOCK_EMPTY_UPDATE_JSON);
        mockNativeModule.editorDeleteScalarRange.mockReturnValue(MOCK_EMPTY_UPDATE_JSON);
        mockNativeModule.editorReplaceTextScalar.mockReturnValue(MOCK_EMPTY_UPDATE_JSON);
        mockNativeModule.editorSplitBlockScalar.mockReturnValue(MOCK_EMPTY_UPDATE_JSON);
        mockNativeModule.editorDeleteAndSplitScalar.mockReturnValue(MOCK_EMPTY_UPDATE_JSON);
        mockNativeModule.editorDocToScalar.mockImplementation((_: number, pos: number) => pos);
        mockNativeModule.editorScalarToDoc.mockImplementation(
            (_: number, scalar: number) => scalar
        );
        mockNativeModule.editorToggleBlockquoteAtSelectionScalar.mockReturnValue(
            MOCK_EMPTY_UPDATE_JSON
        );
        mockNativeModule.editorToggleHeadingAtSelectionScalar.mockReturnValue(
            MOCK_EMPTY_UPDATE_JSON
        );
        mockNativeModule.editorWrapInList.mockReturnValue(MOCK_LIST_UPDATE_JSON);
        mockNativeModule.editorUnwrapFromList.mockReturnValue(MOCK_EMPTY_UPDATE_JSON);
        mockNativeModule.editorIndentListItem.mockReturnValue(MOCK_LIST_UPDATE_JSON);
        mockNativeModule.editorOutdentListItem.mockReturnValue(MOCK_LIST_UPDATE_JSON);
        mockNativeModule.editorInsertNode.mockReturnValue(MOCK_NODE_UPDATE_JSON);
        mockNativeModule.editorWrapInListAtSelectionScalar.mockReturnValue(MOCK_LIST_UPDATE_JSON);
        mockNativeModule.editorUnwrapFromListAtSelectionScalar.mockReturnValue(
            MOCK_EMPTY_UPDATE_JSON
        );
        mockNativeModule.editorIndentListItemAtSelectionScalar.mockReturnValue(
            MOCK_LIST_UPDATE_JSON
        );
        mockNativeModule.editorOutdentListItemAtSelectionScalar.mockReturnValue(
            MOCK_LIST_UPDATE_JSON
        );
        mockNativeModule.editorInsertNodeAtSelectionScalar.mockReturnValue(MOCK_NODE_UPDATE_JSON);
        mockNativeModule.editorUndo.mockReturnValue(MOCK_UNDO_UPDATE_JSON);
        mockNativeModule.editorRedo.mockReturnValue(MOCK_REDO_UPDATE_JSON);
        mockNativeModule.editorCanUndo.mockReturnValue(true);
        mockNativeModule.editorCanRedo.mockReturnValue(false);
    });

    // ── Rendering ───────────────────────────────────────────────

    describe('rendering', () => {
        it('renders without crashing', () => {
            const { getByTestId } = render(<NativeRichTextEditor />);
            expect(getByTestId('native-editor-view')).toBeTruthy();
        });

        it('creates bridge with config on mount', () => {
            render(<NativeRichTextEditor />);
            expect(mockNativeModule.editorCreate).toHaveBeenCalledTimes(1);
            expect(mockNativeModule.editorCreate).toHaveBeenCalledWith('{}');
        });

        it('creates bridge with maxLength config when provided', () => {
            render(<NativeRichTextEditor maxLength={200} />);
            expect(mockNativeModule.editorCreate).toHaveBeenCalledWith(
                JSON.stringify({ maxLength: 200, allowBase64Images: false })
            );
        });

        it('creates bridge with allowBase64Images config when provided', () => {
            render(<NativeRichTextEditor allowBase64Images />);
            expect(mockNativeModule.editorCreate).toHaveBeenCalledWith(
                JSON.stringify({ allowBase64Images: true })
            );
        });

        it('sets initial content via setHtml when initialContent is provided', () => {
            render(<NativeRichTextEditor initialContent='<p>hello</p>' />);
            expect(mockNativeModule.editorSetHtml).toHaveBeenCalledWith(1, '<p>hello</p>');
        });

        it('sets initialJSON via setJson when provided', () => {
            const doc = { type: 'doc', content: [] };
            render(<NativeRichTextEditor initialJSON={doc} />);
            expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(
                1,
                JSON.stringify(NORMALIZED_EMPTY_DOC)
            );
        });

        it('normalizes empty initialJSON using the provided schema', () => {
            render(
                <NativeRichTextEditor
                    initialJSON={{ type: 'doc', content: [] }}
                    schema={TITLE_FIRST_SCHEMA}
                />
            );
            expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(
                1,
                JSON.stringify(TITLE_EMPTY_DOC)
            );
        });

        it('does not call setHtml when no initialContent is provided', () => {
            render(<NativeRichTextEditor />);
            expect(mockNativeModule.editorSetHtml).not.toHaveBeenCalled();
        });

        it('passes editorId to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor />);
            const view = getByTestId('native-editor-view');
            expect(view.props.editorId).toBe(1);
        });

        it('passes editable prop to native view (default true)', () => {
            const { getByTestId } = render(<NativeRichTextEditor />);
            expect(getByTestId('native-editor-view').props.editable).toBe(true);
        });

        it('passes editable=false to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor editable={false} />);
            expect(getByTestId('native-editor-view').props.editable).toBe(false);
        });

        it('passes placeholder prop to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor placeholder='Type here...' />);
            expect(getByTestId('native-editor-view').props.placeholder).toBe('Type here...');
        });

        it('passes autoFocus prop to native view (default false)', () => {
            const { getByTestId } = render(<NativeRichTextEditor />);
            expect(getByTestId('native-editor-view').props.autoFocus).toBe(false);
        });

        it('passes autoFocus=true to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor autoFocus />);
            expect(getByTestId('native-editor-view').props.autoFocus).toBe(true);
        });

        it('passes keyboard input props to native view', () => {
            const { getByTestId } = render(
                <NativeRichTextEditor
                    autoCapitalize='none'
                    autoCorrect={false}
                    keyboardType='email-address'
                />
            );
            const viewProps = getByTestId('native-editor-view').props;

            expect(viewProps.autoCapitalize).toBe('none');
            expect(viewProps.autoCorrect).toBe(false);
            expect(viewProps.keyboardType).toBe('email-address');
        });

        it('passes toolbarPlacement to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor toolbarPlacement='inline' />);
            expect(getByTestId('native-editor-view').props.toolbarPlacement).toBe('inline');
        });

        it('passes heightBehavior to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor heightBehavior='autoGrow' />);
            expect(getByTestId('native-editor-view').props.heightBehavior).toBe('autoGrow');
        });

        it('passes allowImageResizing to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor allowImageResizing={false} />);
            expect(getByTestId('native-editor-view').props.allowImageResizing).toBe(false);
        });

        it('passes style to native view', () => {
            const customStyle = { height: 200, borderWidth: 1 };
            const { getByTestId } = render(<NativeRichTextEditor style={customStyle} />);
            expect(getByTestId('native-editor-view').props.style).toEqual(customStyle);
        });

        it('applies containerStyle to the outer wrapper view', () => {
            const containerStyle = { marginTop: 12, borderRadius: 8 };
            const { toJSON } = render(<NativeRichTextEditor containerStyle={containerStyle} />);
            expect(toJSON()).toMatchObject({
                props: {
                    style: [expect.any(Object), containerStyle],
                },
            });
        });

        it('mirrors containerStyle minHeight to the native view', () => {
            const { getByTestId } = render(
                <NativeRichTextEditor containerStyle={{ minHeight: 240 }} />
            );

            expect(getByTestId('native-editor-view').props.style).toEqual({
                minHeight: 240,
            });
        });

        it('serializes theme to native view', () => {
            const theme = {
                text: { fontSize: 18, color: '#112233' },
                list: { indent: 28, markerColor: '#445566' },
                horizontalRule: { color: '#778899', thickness: 2 },
                contentInsets: { top: 12, right: 16, bottom: 20, left: 16 },
            };
            const { getByTestId } = render(<NativeRichTextEditor theme={theme} />);
            expect(getByTestId('native-editor-view').props.themeJson).toBe(JSON.stringify(theme));
        });

        it('serializes toolbarItems to native view', () => {
            const toolbarItems = [
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
            ] as const;
            const { getByTestId } = render(<NativeRichTextEditor toolbarItems={toolbarItems} />);
            expect(getByTestId('native-editor-view').props.toolbarItemsJson).toBe(
                JSON.stringify(toolbarItems)
            );
        });

        it('serializes grouped native toolbar items with recursive link and image actions', () => {
            const toolbarItems = [
                {
                    type: 'group',
                    key: 'insert',
                    label: 'Insert',
                    icon: { type: 'glyph', text: '+' },
                    presentation: 'menu',
                    items: [
                        {
                            type: 'link',
                            label: 'Link',
                            icon: { type: 'default', id: 'link' },
                        },
                        {
                            type: 'image',
                            label: 'Image',
                            icon: { type: 'default', id: 'image' },
                        },
                    ],
                },
            ] as const;
            const { getByTestId } = render(
                <NativeRichTextEditor
                    toolbarItems={toolbarItems}
                    onRequestLink={jest.fn()}
                    onRequestImage={jest.fn()}
                />
            );

            expect(getByTestId('native-editor-view').props.toolbarItemsJson).toBe(
                JSON.stringify([
                    {
                        type: 'group',
                        key: 'insert',
                        label: 'Insert',
                        icon: { type: 'glyph', text: '+' },
                        presentation: 'menu',
                        items: [
                            {
                                type: 'action',
                                key: '__native-editor-link__',
                                label: 'Link',
                                icon: { type: 'default', id: 'link' },
                                isActive: false,
                                isDisabled: true,
                            },
                            {
                                type: 'action',
                                key: '__native-editor-image__',
                                label: 'Image',
                                icon: { type: 'default', id: 'image' },
                                isActive: false,
                                isDisabled: true,
                            },
                        ],
                    },
                ])
            );
        });

        it('serializes remote selections to native view', () => {
            const remoteSelections = [
                {
                    clientId: 2,
                    anchor: 4,
                    head: 9,
                    color: '#00AAFF',
                    name: 'Bob',
                    isFocused: true,
                },
            ] as const;
            const { getByTestId } = render(
                <NativeRichTextEditor remoteSelections={remoteSelections} />
            );
            expect(getByTestId('native-editor-view').props.remoteSelectionsJson).toBe(
                JSON.stringify(remoteSelections)
            );
        });

        it('serializes mentions addons and extends the schema passed to the bridge', () => {
            const addons = {
                mentions: {
                    trigger: '@',
                    theme: {
                        textColor: '#112233',
                        backgroundColor: '#ddeeff',
                        popoverBackgroundColor: '#ffffff',
                    },
                    suggestions: [
                        {
                            key: 'u1',
                            title: 'Alice',
                            subtitle: 'Design',
                            attrs: { id: 'u1', type: 'user' },
                        },
                    ],
                },
            } as const;

            const { getByTestId } = render(<NativeRichTextEditor addons={addons} />);
            const createArg = mockNativeModule.editorCreate.mock.calls[0]?.[0];
            const config = JSON.parse(createArg);
            const mentionNode = config.schema.nodes.find(
                (node: { name: string }) => node.name === 'mention'
            );

            expect(mentionNode).toEqual(
                expect.objectContaining({
                    name: 'mention',
                    content: '',
                    group: 'inline',
                    role: 'inline',
                    isVoid: true,
                })
            );
            expect(getByTestId('native-editor-view').props.addonsJson).toBe(
                JSON.stringify({
                    mentions: {
                        trigger: '@',
                        theme: addons.mentions.theme,
                        suggestions: [
                            {
                                key: 'u1',
                                title: 'Alice',
                                subtitle: 'Design',
                                label: 'Alice',
                                attrs: {
                                    label: 'Alice',
                                    mentionSuggestionChar: '@',
                                    id: 'u1',
                                    type: 'user',
                                },
                            },
                        ],
                    },
                })
            );
        });

        it('passes onToolbarAction handler to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor onToolbarAction={jest.fn()} />);
            expect(typeof getByTestId('native-editor-view').props.onToolbarAction).toBe('function');
        });

        it('passes onAddonEvent handler to native view', () => {
            const { getByTestId } = render(
                <NativeRichTextEditor addons={{ mentions: { suggestions: [] } }} />
            );
            expect(typeof getByTestId('native-editor-view').props.onAddonEvent).toBe('function');
        });

        it('rebinds the native view to a new editor instance when mentions are enabled after mount', () => {
            const { getByTestId, rerender } = render(<NativeRichTextEditor />);

            expect(getByTestId('native-editor-view').props.editorId).toBe(1);

            rerender(
                <NativeRichTextEditor
                    addons={{
                        mentions: {
                            suggestions: [{ key: 'u1', title: 'Alice' }],
                        },
                    }}
                />
            );

            expect(mockNativeModule.editorCreate).toHaveBeenCalledTimes(2);
            expect(getByTestId('native-editor-view').props.editorId).toBe(2);
            expect(getByTestId('native-editor-view').props.addonsJson).toBe(
                JSON.stringify({
                    mentions: {
                        trigger: '@',
                        suggestions: [
                            {
                                key: 'u1',
                                title: 'Alice',
                                label: 'Alice',
                                attrs: {
                                    label: 'Alice',
                                    mentionSuggestionChar: '@',
                                },
                            },
                        ],
                    },
                })
            );
        });

        it('preserves uncontrolled content when mentions are enabled after mount', () => {
            const liveDocument = {
                type: 'doc',
                content: [
                    {
                        type: 'paragraph',
                        content: [{ type: 'text', text: 'Live content' }],
                    },
                ],
            };
            const liveDocumentJson = JSON.stringify(liveDocument);
            const { getByTestId, rerender } = render(
                <NativeRichTextEditor initialContent='<p>Initial content</p>' />
            );

            expect(getByTestId('native-editor-view').props.editorId).toBe(1);
            mockNativeModule.editorGetJson.mockReturnValue(liveDocumentJson);
            mockNativeModule.editorSetHtml.mockClear();
            mockNativeModule.editorSetJson.mockClear();

            rerender(
                <NativeRichTextEditor
                    initialContent='<p>Initial content</p>'
                    addons={{
                        mentions: {
                            suggestions: [{ key: 'u1', title: 'Alice' }],
                        },
                    }}
                />
            );

            expect(getByTestId('native-editor-view').props.editorId).toBe(2);
            expect(mockNativeModule.editorGetJson).toHaveBeenCalledWith(1);
            expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(2, liveDocumentJson);
            expect(mockNativeModule.editorSetHtml).not.toHaveBeenCalled();
        });

        it('keeps the native view command ref after mentions recreate the editor', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            const { getByTestId, rerender } = render(<NativeRichTextEditor ref={ref} />);

            rerender(
                <NativeRichTextEditor
                    ref={ref}
                    addons={{
                        mentions: {
                            suggestions: [{ key: 'u1', title: 'Alice' }],
                        },
                    }}
                />
            );
            mockApplyEditorUpdate.mockClear();

            act(() => {
                ref.current!.toggleMark('bold');
            });

            expect(getByTestId('native-editor-view').props.editorId).toBe(2);
            expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledWith(
                2,
                0,
                0,
                'bold'
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_BOLD_UPDATE_JSON);
        });
    });

    // ── Ref Methods ─────────────────────────────────────────────

    describe('ref methods', () => {
        it('toggleMark(bold) calls bridge.toggleMark and applyEditorUpdate', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            mockNativeModule.editorGetHtml
                .mockReturnValueOnce('<p>plain</p>')
                .mockReturnValueOnce('<p><strong>plain</strong></p>');
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.toggleMark('bold');
            });

            expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0,
                'bold'
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_BOLD_UPDATE_JSON);
        });

        it('scopes Android pending native updates to the current editor id', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const ref = createRef<NativeRichTextEditorRef>();

            try {
                const { getByTestId } = render(<NativeRichTextEditor ref={ref} />);
                mockNativeModule.editorGetSelection
                    .mockReturnValueOnce(JSON.stringify({ type: 'text', anchor: 0, head: 0 }))
                    .mockReturnValue(JSON.stringify({ type: 'text', anchor: 0, head: 5 }));

                act(() => {
                    ref.current!.toggleMark('bold');
                });

                const viewProps = getByTestId('native-editor-view').props;
                expect(viewProps.editorUpdateJson).toBe(MOCK_BOLD_UPDATE_JSON);
                expect(viewProps.editorUpdateEditorId).toBe(1);
                expect(viewProps.editorUpdateRevision).toBe(1);
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('refreshes live Android selection before scalar ref commands', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const ref = createRef<NativeRichTextEditorRef>();

            try {
                render(<NativeRichTextEditor ref={ref} />);
                mockNativeModule.editorGetSelection.mockReturnValueOnce(
                    JSON.stringify({ type: 'text', anchor: 4, head: 4 })
                );

                act(() => {
                    ref.current!.toggleMark('bold');
                });

                expect(mockNativeModule.editorGetSelection).toHaveBeenCalledWith(1);
                expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledWith(
                    1,
                    4,
                    4,
                    'bold'
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('preserves backward live Android selection before scalar ref commands', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const ref = createRef<NativeRichTextEditorRef>();

            try {
                render(<NativeRichTextEditor ref={ref} />);
                mockNativeModule.editorGetSelection.mockReturnValueOnce(
                    JSON.stringify({ type: 'text', anchor: 11, head: 6 })
                );

                act(() => {
                    ref.current!.toggleMark('bold');
                });

                expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledWith(
                    1,
                    11,
                    6,
                    'bold'
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('drops Android ref mutations while the previous native update is unsettled', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const ref = createRef<NativeRichTextEditorRef>();

            try {
                const { getByTestId } = render(<NativeRichTextEditor ref={ref} />);
                mockNativeModule.editorGetSelection
                    .mockReturnValueOnce(JSON.stringify({ type: 'text', anchor: 0, head: 0 }))
                    .mockReturnValue(JSON.stringify({ type: 'text', anchor: 0, head: 5 }));

                act(() => {
                    ref.current!.toggleMark('bold');
                });
                act(() => {
                    ref.current!.toggleMark('italic');
                });

                expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledTimes(1);
                expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenLastCalledWith(
                    1,
                    0,
                    0,
                    'bold'
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorReady({
                        nativeEvent: { editorId: 1 },
                    });
                });
                act(() => {
                    ref.current!.toggleMark('italic');
                });
                expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledTimes(1);

                act(() => {
                    getByTestId('native-editor-view').props.onEditorReady({
                        nativeEvent: { editorId: 1, editorUpdateRevision: 99 },
                    });
                });
                act(() => {
                    ref.current!.toggleMark('italic');
                });
                expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledTimes(1);

                act(() => {
                    getByTestId('native-editor-view').props.onEditorReady({
                        nativeEvent: { editorId: 1, editorUpdateRevision: 1 },
                    });
                });
                expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();
                act(() => {
                    ref.current!.toggleMark('italic');
                });

                expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledTimes(2);
                expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenLastCalledWith(
                    1,
                    0,
                    5,
                    'italic'
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('retries controlled updates after Android command preflight detaches then becomes ready', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });

            try {
                mockNativeModule.editorGetHtml.mockReturnValue('<p>old</p>');
                const { getByTestId, rerender } = render(
                    <NativeRichTextEditor value='<p>old</p>' />
                );
                mockNativeModule.editorPrepareForCommand.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(JSON.stringify({ ready: false, blockedReason: 'detached' }))
                    .mockReturnValue(JSON.stringify({ ready: true }));

                act(() => {
                    rerender(<NativeRichTextEditor value='<p>new</p>' />);
                });

                expect(mockNativeModule.editorPrepareForCommand).toHaveBeenCalledTimes(1);
                expect(mockNativeModule.editorReplaceHtml).not.toHaveBeenCalledWith(
                    1,
                    '<p>new</p>'
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorReady({
                        nativeEvent: { editorId: 1 },
                    });
                });

                expect(mockNativeModule.editorReplaceHtml).toHaveBeenCalledWith(1, '<p>new</p>');
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('toggleMark at a collapsed cursor skips native reapply when the document version is unchanged', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            mockNativeModule.editorToggleMarkAtSelectionScalar.mockReturnValue(
                MOCK_COLLAPSED_BOLD_UPDATE_JSON
            );
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.toggleMark('bold');
            });

            expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0,
                'bold'
            );
            expect(mockApplyEditorUpdate).not.toHaveBeenCalled();
        });

        it('toggleList(bulletList) calls bridge.toggleList and applyEditorUpdate', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.toggleList('bulletList');
            });

            expect(mockNativeModule.editorWrapInListAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0,
                'bulletList'
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_LIST_UPDATE_JSON);
        });

        it('setLink(href) calls bridge.setMark and applyEditorUpdate', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            mockNativeModule.editorGetHtml
                .mockReturnValueOnce('<p>plain</p>')
                .mockReturnValueOnce('<p><a href="https://example.com">plain</a></p>');
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.setLink('https://example.com');
            });

            expect(mockNativeModule.editorSetMarkAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0,
                'link',
                JSON.stringify({ href: 'https://example.com' })
            );
        });

        it('unsetLink() calls bridge.unsetMark and applyEditorUpdate', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            mockNativeModule.editorGetHtml
                .mockReturnValueOnce('<p><a href="https://example.com">plain</a></p>')
                .mockReturnValueOnce('<p>plain</p>');
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.unsetLink();
            });

            expect(mockNativeModule.editorUnsetMarkAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0,
                'link'
            );
        });

        it('toggleList(orderedList) converts from bulletList in one native call', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            mockNativeModule.editorGetCurrentState
                .mockReturnValueOnce(MOCK_EMPTY_UPDATE_JSON)
                .mockReturnValueOnce(
                    JSON.stringify({
                        renderElements: [],
                        selection: { type: 'text', anchor: 0, head: 0 },
                        activeState: { marks: {}, nodes: { bulletList: true } },
                        historyState: { canUndo: true, canRedo: false },
                    })
                );
            mockNativeModule.editorUnwrapFromListAtSelectionScalar.mockReturnValueOnce(
                MOCK_EMPTY_UPDATE_JSON
            );
            mockNativeModule.editorWrapInListAtSelectionScalar.mockReturnValueOnce(
                MOCK_ORDERED_LIST_UPDATE_JSON
            );

            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.toggleList('orderedList');
            });

            expect(mockNativeModule.editorWrapInListAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0,
                'orderedList'
            );
            expect(mockNativeModule.editorUnwrapFromListAtSelectionScalar).not.toHaveBeenCalled();
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_ORDERED_LIST_UPDATE_JSON);
        });

        it('insertNode(horizontalRule) calls bridge.insertNode and applyEditorUpdate', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.insertNode('horizontalRule');
            });

            expect(mockNativeModule.editorInsertNodeAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0,
                'horizontalRule'
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_NODE_UPDATE_JSON);
        });

        it('toggleBlockquote calls bridge.toggleBlockquote and applyEditorUpdate', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            mockNativeModule.editorToggleBlockquoteAtSelectionScalar.mockReturnValueOnce(
                MOCK_LIST_UPDATE_JSON
            );
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.toggleBlockquote();
            });

            expect(mockNativeModule.editorToggleBlockquoteAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_LIST_UPDATE_JSON);
        });

        it('toggleHeading calls bridge.toggleHeading and applyEditorUpdate', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.toggleHeading(2);
            });

            expect(mockNativeModule.editorToggleHeadingAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0,
                2
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_EMPTY_UPDATE_JSON);
        });

        it('forwards native toolbar action events to onToolbarAction', () => {
            const onToolbarAction = jest.fn();
            const { getByTestId } = render(
                <NativeRichTextEditor onToolbarAction={onToolbarAction} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onToolbarAction({
                    nativeEvent: { key: 'insertMention' },
                });
            });

            expect(onToolbarAction).toHaveBeenCalledWith('insertMention');
        });

        it('syncs Android toolbar action preflight update before invoking callback', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onToolbarAction = jest.fn();
            const onSelectionChange = jest.fn();
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                selection: { type: 'text', anchor: 3, head: 3 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 3,
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor
                        onToolbarAction={onToolbarAction}
                        onSelectionChange={onSelectionChange}
                    />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: MOCK_BOLD_UPDATE_JSON },
                    });
                });
                onSelectionChange.mockClear();

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: 'insertMention',
                            documentVersion: 3,
                            updateJson: correctedUpdateJson,
                        },
                    });
                });

                expect(onSelectionChange).toHaveBeenCalledWith({
                    type: 'text',
                    anchor: 3,
                    head: 3,
                });
                expect(onToolbarAction).toHaveBeenCalledWith('insertMention');
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('ignores stale Android toolbar actions from older document versions', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onToolbarAction = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onToolbarAction={onToolbarAction} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: MOCK_BOLD_UPDATE_JSON },
                    });
                });

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: 'insertMention',
                            documentVersion: 1,
                        },
                    });
                });

                expect(onToolbarAction).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('ignores versionless Android toolbar actions after a document version is known', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onToolbarAction = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onToolbarAction={onToolbarAction} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: MOCK_BOLD_UPDATE_JSON },
                    });
                });

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: 'insertMention',
                        },
                    });
                });

                expect(onToolbarAction).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('accepts versioned Android toolbar actions without a preflight update', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onToolbarAction = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onToolbarAction={onToolbarAction} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: MOCK_BOLD_UPDATE_JSON },
                    });
                });

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: 'insertMention',
                            documentVersion: 2,
                        },
                    });
                });

                expect(onToolbarAction).toHaveBeenCalledWith('insertMention');
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('routes native link toolbar actions through onRequestLink', () => {
            const onRequestLink = jest.fn();
            const { getByTestId } = render(<NativeRichTextEditor onRequestLink={onRequestLink} />);

            mockNativeModule.editorSetSelection.mockClear();

            act(() => {
                getByTestId('native-editor-view').props.onToolbarAction({
                    nativeEvent: { key: '__native-editor-link__' },
                });
            });

            expect(onRequestLink).toHaveBeenCalledTimes(1);
            const context = onRequestLink.mock.calls[0][0];
            expect(context.isActive).toBe(false);
            act(() => {
                context.setLink('https://example.com');
            });
            expect(mockNativeModule.editorSetSelection).not.toHaveBeenCalled();
            expect(mockNativeModule.editorSetMarkAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0,
                'link',
                JSON.stringify({ href: 'https://example.com' })
            );
        });

        it('uses Android preflight link state for native link toolbar context', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onRequestLink = jest.fn();
            const preflightUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'linked', marks: ['link'] }],
                selection: { type: 'text', anchor: 0, head: 6 },
                activeState: {
                    marks: { link: true },
                    markAttrs: { link: { href: 'https://preflight.example' } },
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: ['link'],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onRequestLink={onRequestLink} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-link__',
                            documentVersion: 2,
                            updateJson: preflightUpdateJson,
                        },
                    });
                });

                expect(onRequestLink).toHaveBeenCalledTimes(1);
                expect(onRequestLink.mock.calls[0][0].isActive).toBe(true);
                expect(onRequestLink.mock.calls[0][0].href).toBe('https://preflight.example');
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('routes native image toolbar actions through onRequestImage', () => {
            const onRequestImage = jest.fn();
            const { getByTestId } = render(
                <NativeRichTextEditor onRequestImage={onRequestImage} />
            );

            mockNativeModule.editorSetSelection.mockClear();

            act(() => {
                getByTestId('native-editor-view').props.onToolbarAction({
                    nativeEvent: { key: '__native-editor-image__' },
                });
            });

            expect(onRequestImage).toHaveBeenCalledTimes(1);
            const context = onRequestImage.mock.calls[0][0];
            expect(context.allowBase64).toBe(false);
            act(() => {
                context.insertImage('https://example.com/cat.png', { alt: 'Cat' });
            });
            expect(mockNativeModule.editorSetSelection).not.toHaveBeenCalled();
            expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0,
                JSON.stringify({
                    type: 'doc',
                    content: [
                        {
                            type: 'image',
                            attrs: {
                                src: 'https://example.com/cat.png',
                                alt: 'Cat',
                            },
                        },
                    ],
                })
            );
        });

        it('drops link request callbacks when selection changes at the same document version', () => {
            const onRequestLink = jest.fn();
            const requestStateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'hello', marks: [] }],
                selection: { type: 'text', anchor: 0, head: 0 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: ['link'],
                    insertableNodes: [],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 1,
            });
            const movedSelectionStateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'hello', marks: [] }],
                selection: { type: 'text', anchor: 2, head: 2 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: ['link'],
                    insertableNodes: [],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 1,
            });
            const { getByTestId } = render(<NativeRichTextEditor onRequestLink={onRequestLink} />);

            act(() => {
                getByTestId('native-editor-view').props.onToolbarAction({
                    nativeEvent: {
                        editorId: 1,
                        key: '__native-editor-link__',
                        documentVersion: 1,
                        updateJson: requestStateJson,
                    },
                });
            });

            const context = onRequestLink.mock.calls[0][0];
            mockNativeModule.editorSetMarkAtSelectionScalar.mockClear();
            mockNativeModule.editorUnsetMarkAtSelectionScalar.mockClear();

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: {
                        editorId: 1,
                        anchor: 2,
                        head: 2,
                        documentVersion: 1,
                        stateJson: movedSelectionStateJson,
                    },
                });
                context.setLink('https://example.com');
                context.unsetLink();
            });

            expect(mockNativeModule.editorSetMarkAtSelectionScalar).not.toHaveBeenCalled();
            expect(mockNativeModule.editorUnsetMarkAtSelectionScalar).not.toHaveBeenCalled();
        });

        it('drops image request callbacks when selection changes at the same document version', () => {
            const onRequestImage = jest.fn();
            const requestStateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'hello', marks: [] }],
                selection: { type: 'text', anchor: 0, head: 0 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: ['image'],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 1,
            });
            const movedSelectionStateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'hello', marks: [] }],
                selection: { type: 'text', anchor: 2, head: 2 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: ['image'],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 1,
            });
            const { getByTestId } = render(
                <NativeRichTextEditor onRequestImage={onRequestImage} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onToolbarAction({
                    nativeEvent: {
                        editorId: 1,
                        key: '__native-editor-image__',
                        documentVersion: 1,
                        updateJson: requestStateJson,
                    },
                });
            });

            const context = onRequestImage.mock.calls[0][0];
            mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: {
                        editorId: 1,
                        anchor: 2,
                        head: 2,
                        documentVersion: 1,
                        stateJson: movedSelectionStateJson,
                    },
                });
                context.insertImage('https://example.com/cat.png');
            });

            expect(
                mockNativeModule.editorInsertContentJsonAtSelectionScalar
            ).not.toHaveBeenCalled();
        });

        it('drops stale Android link request callbacks after document advances', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onRequestLink = jest.fn();
            const advancedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'changed', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onRequestLink={onRequestLink} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-link__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestLink.mock.calls[0][0];

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: advancedUpdateJson },
                    });
                    context.setLink('https://example.com');
                    context.unsetLink();
                });

                expect(mockNativeModule.editorSetMarkAtSelectionScalar).not.toHaveBeenCalled();
                expect(mockNativeModule.editorUnsetMarkAtSelectionScalar).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('drops stale Android image request callbacks after document advances', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onRequestImage = jest.fn();
            const advancedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'changed', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onRequestImage={onRequestImage} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-image__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestImage.mock.calls[0][0];

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: advancedUpdateJson },
                    });
                    context.insertImage('https://example.com/cat.png');
                });

                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('drops stale Android link request callbacks after same-version editor recreation', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onRequestLink = jest.fn();

            try {
                const { getByTestId, rerender } = render(
                    <NativeRichTextEditor onRequestLink={onRequestLink} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-link__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestLink.mock.calls[0][0];

                rerender(
                    <NativeRichTextEditor
                        maxLength={100}
                        onRequestLink={onRequestLink}
                    />
                );
                expect(getByTestId('native-editor-view').props.editorId).toBe(2);

                mockNativeModule.editorSetMarkAtSelectionScalar.mockClear();
                mockNativeModule.editorUnsetMarkAtSelectionScalar.mockClear();
                act(() => {
                    context.setLink('https://example.com');
                    context.unsetLink();
                });

                expect(mockNativeModule.editorSetMarkAtSelectionScalar).not.toHaveBeenCalled();
                expect(mockNativeModule.editorUnsetMarkAtSelectionScalar).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('drops stale Android image request callbacks after same-version editor recreation', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onRequestImage = jest.fn();

            try {
                const { getByTestId, rerender } = render(
                    <NativeRichTextEditor onRequestImage={onRequestImage} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-image__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestImage.mock.calls[0][0];

                rerender(
                    <NativeRichTextEditor
                        maxLength={100}
                        onRequestImage={onRequestImage}
                    />
                );
                expect(getByTestId('native-editor-view').props.editorId).toBe(2);

                mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();
                act(() => {
                    context.insertImage('https://example.com/cat.png');
                });

                expect(
                    mockNativeModule.editorInsertContentJsonAtSelectionScalar
                ).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('cancels Android link callback when command preflight advances the document', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onRequestLink = jest.fn();
            const preflightUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'corrected', marks: [] }],
                selection: { type: 'text', anchor: 9, head: 9 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: ['link'],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onRequestLink={onRequestLink} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-link__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestLink.mock.calls[0][0];
                mockNativeModule.editorSetMarkAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                    JSON.stringify({ ready: true, updateJSON: preflightUpdateJson })
                );

                act(() => {
                    context.setLink('https://example.com');
                });

                expect(mockNativeModule.editorSetMarkAtSelectionScalar).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('cancels Android image callback when command preflight advances the document', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onRequestImage = jest.fn();
            const preflightUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'corrected', marks: [] }],
                selection: { type: 'text', anchor: 9, head: 9 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: ['image'],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onRequestImage={onRequestImage} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-image__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestImage.mock.calls[0][0];
                mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                    JSON.stringify({ ready: true, updateJSON: preflightUpdateJson })
                );

                act(() => {
                    context.insertImage('https://example.com/cat.png');
                });

                expect(
                    mockNativeModule.editorInsertContentJsonAtSelectionScalar
                ).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('retries Android link callback after composition-blocked command preflight', () => {
            jest.useFakeTimers();
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onRequestLink = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onRequestLink={onRequestLink} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-link__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestLink.mock.calls[0][0];
                mockNativeModule.editorSetMarkAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({ ready: false, blockedReason: 'composition' })
                    )
                    .mockReturnValue('{"ready":true}');

                act(() => {
                    context.setLink('https://example.com');
                });

                expect(mockNativeModule.editorSetMarkAtSelectionScalar).not.toHaveBeenCalled();

                act(() => {
                    jest.advanceTimersByTime(50);
                });

                expect(mockNativeModule.editorSetMarkAtSelectionScalar).toHaveBeenCalledWith(
                    1,
                    0,
                    0,
                    'link',
                    JSON.stringify({ href: 'https://example.com' })
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
                jest.useRealTimers();
            }
        });

        it('retries iOS link callback after composition-blocked command preflight', () => {
            jest.useFakeTimers();
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'ios',
            });
            const onRequestLink = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onRequestLink={onRequestLink} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-link__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestLink.mock.calls[0][0];
                mockNativeModule.editorSetMarkAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({ ready: false, blockedReason: 'composition' })
                    )
                    .mockReturnValue('{"ready":true}');

                act(() => {
                    context.setLink('https://example.com');
                });

                expect(mockNativeModule.editorSetMarkAtSelectionScalar).not.toHaveBeenCalled();

                act(() => {
                    jest.advanceTimersByTime(50);
                });

                expect(mockNativeModule.editorSetMarkAtSelectionScalar).toHaveBeenCalledWith(
                    1,
                    0,
                    0,
                    'link',
                    JSON.stringify({ href: 'https://example.com' })
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
                jest.useRealTimers();
            }
        });

        it('cancels iOS link retry when the document advances before retry', () => {
            jest.useFakeTimers();
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'ios',
            });
            const onRequestLink = jest.fn();
            const advancedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'changed', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: ['link'],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onRequestLink={onRequestLink} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-link__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestLink.mock.calls[0][0];
                mockNativeModule.editorSetMarkAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({ ready: false, blockedReason: 'composition' })
                    )
                    .mockReturnValue('{"ready":true}');

                act(() => {
                    context.setLink('https://example.com');
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: advancedUpdateJson },
                    });
                    jest.advanceTimersByTime(50);
                });

                expect(mockNativeModule.editorSetMarkAtSelectionScalar).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
                jest.useRealTimers();
            }
        });

        it('cancels iOS image retry when selection changes before retry', () => {
            jest.useFakeTimers();
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'ios',
            });
            const onRequestImage = jest.fn();
            const selectionStateJson = JSON.stringify({
                renderElements: [],
                selection: { type: 'text', anchor: 2, head: 2 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: ['image'],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 1,
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onRequestImage={onRequestImage} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-image__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestImage.mock.calls[0][0];
                mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({ ready: false, blockedReason: 'composition' })
                    )
                    .mockReturnValue('{"ready":true}');

                act(() => {
                    context.insertImage('https://example.com/cat.png');
                    getByTestId('native-editor-view').props.onSelectionChange({
                        nativeEvent: {
                            editorId: 1,
                            anchor: 2,
                            head: 2,
                            documentVersion: 1,
                            stateJson: selectionStateJson,
                        },
                    });
                    jest.advanceTimersByTime(50);
                });

                expect(
                    mockNativeModule.editorInsertContentJsonAtSelectionScalar
                ).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
                jest.useRealTimers();
            }
        });

        it('cancels Android link retry when the document advances before retry', () => {
            jest.useFakeTimers();
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onRequestLink = jest.fn();
            const advancedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'changed', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: ['link'],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onRequestLink={onRequestLink} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-link__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestLink.mock.calls[0][0];
                mockNativeModule.editorSetMarkAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({ ready: false, blockedReason: 'composition' })
                    )
                    .mockReturnValue('{"ready":true}');

                act(() => {
                    context.setLink('https://example.com');
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: advancedUpdateJson },
                    });
                    jest.advanceTimersByTime(50);
                });

                expect(mockNativeModule.editorSetMarkAtSelectionScalar).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
                jest.useRealTimers();
            }
        });

        it('retries Android image callback after pending-update command preflight', () => {
            jest.useFakeTimers();
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onRequestImage = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onRequestImage={onRequestImage} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: {
                            editorId: 1,
                            key: '__native-editor-image__',
                            documentVersion: 1,
                        },
                    });
                });

                const context = onRequestImage.mock.calls[0][0];
                mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({ ready: false, blockedReason: 'pendingUpdate' })
                    )
                    .mockReturnValue('{"ready":true}');

                act(() => {
                    context.insertImage('https://example.com/cat.png');
                });

                expect(
                    mockNativeModule.editorInsertContentJsonAtSelectionScalar
                ).not.toHaveBeenCalled();

                act(() => {
                    jest.advanceTimersByTime(50);
                });

                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                    1,
                    0,
                    0,
                    JSON.stringify({
                        type: 'doc',
                        content: [
                            {
                                type: 'image',
                                attrs: {
                                    src: 'https://example.com/cat.png',
                                },
                            },
                        ],
                    })
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
                jest.useRealTimers();
            }
        });

        it('ignores versionless Android command preflight updates after document version is known', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const ref = createRef<NativeRichTextEditorRef>();
            const onActiveStateChange = jest.fn();
            const versionedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'current', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            const versionlessUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'stale', marks: ['italic'] }],
                selection: { type: 'text', anchor: 5, head: 5 },
                activeState: {
                    marks: { italic: true },
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor ref={ref} onActiveStateChange={onActiveStateChange} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: versionedUpdateJson },
                    });
                });
                onActiveStateChange.mockClear();
                mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                    JSON.stringify({ ready: true, updateJSON: versionlessUpdateJson })
                );

                expect(ref.current!.getContent()).toBe('<p>test content</p>');
                expect(onActiveStateChange).not.toHaveBeenCalled();

                mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                    JSON.stringify({ ready: true, updateJSON: versionlessUpdateJson })
                );
                act(() => {
                    ref.current!.toggleMark('bold');
                });

                expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalled();
                expect(onActiveStateChange).toHaveBeenCalledWith(
                    expect.objectContaining({ marks: { bold: true } })
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('indentListItem calls bridge.indentListItem and applyEditorUpdate', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.indentListItem();
            });

            expect(mockNativeModule.editorIndentListItemAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_LIST_UPDATE_JSON);
        });

        it('outdentListItem calls bridge.outdentListItem and applyEditorUpdate', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.outdentListItem();
            });

            expect(mockNativeModule.editorOutdentListItemAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_LIST_UPDATE_JSON);
        });

        it('insertText(hello) replaces the current selection atomically', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.insertText('hello');
            });

            expect(mockNativeModule.editorReplaceSelectionText).toHaveBeenCalledWith(1, 'hello');
            expect(mockNativeModule.editorGetSelection).not.toHaveBeenCalled();
            expect(mockNativeModule.editorInsertText).not.toHaveBeenCalled();
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_INSERT_UPDATE_JSON);
        });

        it('allows imperative mutations when editable is false', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} editable={false} />);

            act(() => {
                ref.current!.insertText('hello');
            });

            expect(mockNativeModule.editorReplaceSelectionText).toHaveBeenCalledWith(1, 'hello');
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_INSERT_UPDATE_JSON);
        });

        it('insertContentHtml calls bridge.insertContentHtml', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.insertContentHtml('<p>hi</p>');
            });

            expect(mockNativeModule.editorInsertContentHtml).toHaveBeenCalledWith(1, '<p>hi</p>');
            expect(mockApplyEditorUpdate).toHaveBeenCalled();
        });

        it('insertContentJson calls bridge.insertContentJson', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);
            const doc = { type: 'doc', content: [] };

            act(() => {
                ref.current!.insertContentJson(doc);
            });

            expect(mockNativeModule.editorInsertContentJson).toHaveBeenCalledWith(
                1,
                JSON.stringify(doc)
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalled();
        });

        it('insertImage inserts an image fragment JSON', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.insertImage('https://example.com/cat.png', {
                    title: 'Cat',
                    width: 320,
                    height: 180,
                });
            });

            expect(mockNativeModule.editorInsertContentJson).toHaveBeenCalledWith(
                1,
                JSON.stringify({
                    type: 'doc',
                    content: [
                        {
                            type: 'image',
                            attrs: {
                                src: 'https://example.com/cat.png',
                                title: 'Cat',
                                width: 320,
                                height: 180,
                            },
                        },
                    ],
                })
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_INSERT_UPDATE_JSON);
        });

        it('insertImage blocks base64 image sources unless allowBase64Images is enabled', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.insertImage('data:image/png;base64,AAAA');
            });

            expect(mockNativeModule.editorInsertContentJson).not.toHaveBeenCalled();
        });

        it('insertImage allows base64 image sources when enabled', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} allowBase64Images />);

            act(() => {
                ref.current!.insertImage('data:image/png;base64,AAAA');
            });

            expect(mockNativeModule.editorInsertContentJson).toHaveBeenCalledWith(
                1,
                JSON.stringify({
                    type: 'doc',
                    content: [
                        {
                            type: 'image',
                            attrs: {
                                src: 'data:image/png;base64,AAAA',
                            },
                        },
                    ],
                })
            );
        });

        it('setContent calls bridge.replaceHtml', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.setContent('<p>new</p>');
            });

            expect(mockNativeModule.editorReplaceHtml).toHaveBeenCalledWith(1, '<p>new</p>');
            expect(mockApplyEditorUpdate).toHaveBeenCalled();
        });

        it('setContentJson calls bridge.replaceJson', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);
            const doc = { type: 'doc', content: [] };

            act(() => {
                ref.current!.setContentJson(doc);
            });

            expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledWith(
                1,
                JSON.stringify(NORMALIZED_EMPTY_DOC)
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalled();
        });

        it('clearContent resets the bridge with normalized empty content', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const ref = createRef<NativeRichTextEditorRef>();

            try {
                render(<NativeRichTextEditor ref={ref} />);

                act(() => {
                    ref.current!.clearContent();
                });

                expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(
                    1,
                    JSON.stringify(NORMALIZED_EMPTY_DOC)
                );
                expect(mockNativeModule.editorReplaceJson).not.toHaveBeenCalled();
                expect(mockApplyEditorResetUpdate).toHaveBeenCalledWith(MOCK_EMPTY_UPDATE_JSON);
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('retries Android setContentJson after in flight native update is acknowledged', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const ref = createRef<NativeRichTextEditorRef>();
            const firstDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'first' }] }],
            };
            const firstUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'first', marks: [] }],
                selection: { type: 'text', anchor: 5, head: 5 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            const clearUpdateJson = JSON.stringify({
                renderElements: [],
                selection: { type: 'text', anchor: 0, head: 0 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 3,
            });

            try {
                const { getByTestId } = render(<NativeRichTextEditor ref={ref} />);
                mockNativeModule.editorReplaceJson
                    .mockReturnValueOnce(firstUpdateJson)
                    .mockReturnValueOnce(clearUpdateJson);

                act(() => {
                    ref.current!.setContentJson(firstDoc);
                });
                const queuedRevision = getByTestId('native-editor-view').props.editorUpdateRevision;

                act(() => {
                    ref.current!.setContentJson({ type: 'doc', content: [] });
                });

                expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledTimes(1);

                act(() => {
                    getByTestId('native-editor-view').props.onEditorReady({
                        nativeEvent: { editorId: 1, editorUpdateRevision: queuedRevision },
                    });
                });

                expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledTimes(2);
                expect(mockNativeModule.editorReplaceJson).toHaveBeenLastCalledWith(
                    1,
                    JSON.stringify(NORMALIZED_EMPTY_DOC)
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('clearContent supersedes an in flight Android native update', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const ref = createRef<NativeRichTextEditorRef>();
            const firstDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'first' }] }],
            };
            const firstUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'first', marks: [] }],
                selection: { type: 'text', anchor: 5, head: 5 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            const resetUpdateJson = JSON.stringify({
                renderElements: [],
                selection: { type: 'text', anchor: 0, head: 0 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 3,
            });

            try {
                const { getByTestId } = render(<NativeRichTextEditor ref={ref} />);
                mockNativeModule.editorReplaceJson.mockReturnValueOnce(firstUpdateJson);
                mockNativeModule.editorGetCurrentState.mockReturnValueOnce(resetUpdateJson);

                act(() => {
                    ref.current!.setContentJson(firstDoc);
                });
                const queuedRevision = getByTestId('native-editor-view').props.editorUpdateRevision;

                act(() => {
                    ref.current!.clearContent();
                });

                expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledTimes(1);
                expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(
                    1,
                    JSON.stringify(NORMALIZED_EMPTY_DOC)
                );
                expect(mockApplyEditorResetUpdate).toHaveBeenCalledWith(resetUpdateJson);
                expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();
                expect(getByTestId('native-editor-view').props.editorResetUpdateJson).toBe(
                    resetUpdateJson
                );
                const resetRevision =
                    getByTestId('native-editor-view').props.editorResetUpdateRevision;

                act(() => {
                    getByTestId('native-editor-view').props.onEditorReady({
                        nativeEvent: { editorId: 1, editorUpdateRevision: queuedRevision },
                    });
                });

                expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledTimes(1);
                expect(mockNativeModule.editorSetJson).toHaveBeenCalledTimes(1);
                expect(getByTestId('native-editor-view').props.editorResetUpdateJson).toBe(
                    resetUpdateJson
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorReady({
                        nativeEvent: { editorId: 1, editorUpdateRevision: resetRevision },
                    });
                });

                expect(
                    getByTestId('native-editor-view').props.editorResetUpdateJson
                ).toBeUndefined();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('clears Android reset acknowledgement even when another update is in flight', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const ref = createRef<NativeRichTextEditorRef>();
            const nextDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'next' }] }],
            };
            const resetUpdateJson = JSON.stringify({
                renderElements: [],
                selection: { type: 'text', anchor: 0, head: 0 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 3,
            });
            const nextUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'next', marks: [] }],
                selection: { type: 'text', anchor: 4, head: 4 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 4,
            });

            try {
                const { getByTestId } = render(<NativeRichTextEditor ref={ref} />);
                mockNativeModule.editorGetCurrentState.mockReturnValueOnce(resetUpdateJson);
                mockNativeModule.editorReplaceJson.mockReturnValueOnce(nextUpdateJson);

                act(() => {
                    ref.current!.clearContent();
                });
                const resetRevision =
                    getByTestId('native-editor-view').props.editorResetUpdateRevision;

                act(() => {
                    ref.current!.setContentJson(nextDoc);
                });

                expect(getByTestId('native-editor-view').props.editorResetUpdateJson).toBe(
                    resetUpdateJson
                );
                expect(getByTestId('native-editor-view').props.editorUpdateJson).toBe(
                    nextUpdateJson
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorReady({
                        nativeEvent: { editorId: 1, editorUpdateRevision: resetRevision },
                    });
                });

                expect(
                    getByTestId('native-editor-view').props.editorResetUpdateJson
                ).toBeUndefined();
                expect(getByTestId('native-editor-view').props.editorUpdateJson).toBe(
                    nextUpdateJson
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('queues Android reset again after clear then native typing update then clear', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const ref = createRef<NativeRichTextEditorRef>();
            const resetUpdateJson = JSON.stringify({
                renderElements: [],
                selection: { type: 'text', anchor: 0, head: 0 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 3,
            });
            const secondResetUpdateJson = JSON.stringify({
                renderElements: [],
                selection: { type: 'text', anchor: 0, head: 0 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 5,
            });
            const typedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'second', marks: [] }],
                selection: { type: 'text', anchor: 6, head: 6 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 4,
            });

            try {
                mockNativeModule.editorGetCurrentState.mockImplementation(() => {
                    const resetWrites = mockNativeModule.editorSetJson.mock.calls.length;
                    if (resetWrites === 0) return MOCK_EMPTY_UPDATE_JSON;
                    if (resetWrites === 1) return resetUpdateJson;
                    return secondResetUpdateJson;
                });
                const { getByTestId } = render(<NativeRichTextEditor ref={ref} />);

                act(() => {
                    ref.current!.clearContent();
                });
                const firstResetRevision =
                    getByTestId('native-editor-view').props.editorResetUpdateRevision;

                act(() => {
                    getByTestId('native-editor-view').props.onEditorReady({
                        nativeEvent: { editorId: 1, editorUpdateRevision: firstResetRevision },
                    });
                });

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: typedUpdateJson },
                    });
                });

                act(() => {
                    ref.current!.clearContent();
                });

                expect(mockNativeModule.editorSetJson).toHaveBeenCalledTimes(2);
                expect(getByTestId('native-editor-view').props.editorResetUpdateJson).toBe(
                    secondResetUpdateJson
                );
                expect(
                    getByTestId('native-editor-view').props.editorResetUpdateRevision
                ).toBeGreaterThan(firstResetRevision);
                expect(mockApplyEditorResetUpdate).toHaveBeenCalledWith(resetUpdateJson);
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('getContent returns bridge.getHtml()', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            const content = ref.current!.getContent();

            expect(mockNativeModule.editorPrepareForCommand).toHaveBeenCalledWith(1);
            expect(mockNativeModule.editorGetHtml).toHaveBeenCalled();
            expect(content).toBe('<p>test content</p>');
        });

        it('getContent flushes native preflight corrections before reading', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                selection: { type: 'text', anchor: 3, head: 3 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            render(<NativeRichTextEditor ref={ref} />);

            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                JSON.stringify({ ready: true, updateJSON: correctedUpdateJson })
            );
            mockNativeModule.editorGetHtml.mockReturnValueOnce('<p>the</p>');

            let content = '';
            act(() => {
                content = ref.current!.getContent();
            });

            expect(content).toBe('<p>the</p>');
            expect(mockNativeModule.editorGetHtml).toHaveBeenCalled();
        });

        it('getContent does not read native content when preflight is blocked', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            expect(ref.current!.getContent()).toBe('<p>test content</p>');
            mockNativeModule.editorGetHtml.mockClear();
            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce('{"ready":false}');

            let content = '';
            act(() => {
                content = ref.current!.getContent();
            });

            expect(content).toBe('<p>test content</p>');
            expect(mockNativeModule.editorGetHtml).not.toHaveBeenCalled();
        });

        it('getContentJson returns bridge.getJson()', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            const json = ref.current!.getContentJson();

            expect(mockNativeModule.editorGetJson).toHaveBeenCalled();
            expect(json).toEqual({
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello' }] }],
            });
        });

        it('getContentJson flushes native preflight corrections before reading', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            const correctedDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'the' }] }],
            };
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                selection: { type: 'text', anchor: 3, head: 3 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            render(<NativeRichTextEditor ref={ref} />);

            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                JSON.stringify({ ready: true, updateJSON: correctedUpdateJson })
            );
            mockNativeModule.editorGetJson.mockReturnValueOnce(JSON.stringify(correctedDoc));

            let json = {};
            act(() => {
                json = ref.current!.getContentJson();
            });

            expect(json).toEqual(correctedDoc);
            expect(mockNativeModule.editorGetJson).toHaveBeenCalled();
        });

        it('getContentJson does not read native content when preflight is blocked', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            const expectedJson = ref.current!.getContentJson();
            mockNativeModule.editorGetJson.mockClear();
            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce('{"ready":false}');

            let json = {};
            act(() => {
                json = ref.current!.getContentJson();
            });

            expect(json).toEqual(expectedJson);
            expect(mockNativeModule.editorGetJson).not.toHaveBeenCalled();
        });

        it('getTextContent strips HTML tags', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            const text = ref.current!.getTextContent();

            expect(text).toBe('test content');
        });

        it('getTextContent flushes native preflight corrections before reading', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                selection: { type: 'text', anchor: 3, head: 3 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            render(<NativeRichTextEditor ref={ref} />);

            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                JSON.stringify({ ready: true, updateJSON: correctedUpdateJson })
            );
            mockNativeModule.editorGetHtml.mockReturnValueOnce('<p>the</p>');

            let text = '';
            act(() => {
                text = ref.current!.getTextContent();
            });

            expect(text).toBe('the');
            expect(mockNativeModule.editorGetHtml).toHaveBeenCalled();
        });

        it('getTextContent does not read native content when preflight is blocked', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            expect(ref.current!.getTextContent()).toBe('test content');
            mockNativeModule.editorGetHtml.mockClear();
            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce('{"ready":false}');

            let text = '';
            act(() => {
                text = ref.current!.getTextContent();
            });

            expect(text).toBe('test content');
            expect(mockNativeModule.editorGetHtml).not.toHaveBeenCalled();
        });

        it('getCaretRect returns the native caret rectangle', async () => {
            const ref = createRef<NativeRichTextEditorRef>();
            mockNativeGetCaretRect.mockResolvedValue(
                JSON.stringify({
                    x: 8,
                    y: 32,
                    width: 2,
                    height: 20,
                    editorWidth: 320,
                    editorHeight: 64,
                })
            );
            render(<NativeRichTextEditor ref={ref} />);

            await expect(ref.current!.getCaretRect()).resolves.toEqual({
                x: 8,
                y: 32,
                width: 2,
                height: 20,
                editorWidth: 320,
                editorHeight: 64,
            });
            expect(mockNativeGetCaretRect.mock.contexts[0]).toEqual(
                expect.objectContaining({
                    getCaretRect: mockNativeGetCaretRect,
                    applyEditorUpdate: mockApplyEditorUpdate,
                })
            );
        });

        it('getCaretRect returns null when native cannot measure the caret', async () => {
            const ref = createRef<NativeRichTextEditorRef>();
            mockNativeGetCaretRect.mockResolvedValue(null);
            render(<NativeRichTextEditor ref={ref} />);

            await expect(ref.current!.getCaretRect()).resolves.toBeNull();
        });

        it('undo calls bridge.undo and applyEditorUpdate', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.undo();
            });

            expect(mockNativeModule.editorUndo).toHaveBeenCalledWith(1);
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_UNDO_UPDATE_JSON);
        });

        it('redo calls bridge.redo and applyEditorUpdate', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            act(() => {
                ref.current!.redo();
            });

            expect(mockNativeModule.editorRedo).toHaveBeenCalledWith(1);
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_REDO_UPDATE_JSON);
        });

        it('canUndo returns bridge.canUndo()', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            expect(ref.current!.canUndo()).toBe(true);
            expect(mockNativeModule.editorCanUndo).toHaveBeenCalledWith(1);
        });

        it('canRedo returns bridge.canRedo()', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            expect(ref.current!.canRedo()).toBe(false);
            expect(mockNativeModule.editorCanRedo).toHaveBeenCalledWith(1);
        });

        it('getBridge does NOT exist on ref', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            expect((ref.current as unknown as Record<string, unknown>).getBridge).toBeUndefined();
        });
    });

    // ── Controlled Mode ─────────────────────────────────────────

    describe('controlled mode', () => {
        it('uses value prop for initial setHtml instead of initialContent', () => {
            render(
                <NativeRichTextEditor initialContent='<p>initial</p>' value='<p>controlled</p>' />
            );

            // value takes precedence — setHtml called with controlled value
            expect(mockNativeModule.editorSetHtml).toHaveBeenCalledWith(1, '<p>controlled</p>');
        });

        it('calls replaceHtml (not setHtml) when value prop changes', () => {
            mockNativeModule.editorGetHtml.mockReturnValueOnce('<p>old</p>');

            const { rerender } = render(<NativeRichTextEditor value='<p>old</p>' />);

            mockNativeModule.editorReplaceHtml.mockClear();
            mockApplyEditorUpdate.mockClear();
            mockNativeModule.editorGetHtml.mockReturnValueOnce('<p>old</p>');

            rerender(<NativeRichTextEditor value='<p>new</p>' />);

            expect(mockNativeModule.editorReplaceHtml).toHaveBeenCalledWith(1, '<p>new</p>');
        });

        it('suppresses content callbacks for controlled updates', () => {
            const onContentChange = jest.fn();
            mockNativeModule.editorGetHtml.mockReturnValueOnce('<p>old</p>');

            const { rerender } = render(
                <NativeRichTextEditor value='<p>old</p>' onContentChange={onContentChange} />
            );

            onContentChange.mockClear();
            mockNativeModule.editorGetHtml.mockReturnValueOnce('<p>old</p>');

            rerender(<NativeRichTextEditor value='<p>new</p>' onContentChange={onContentChange} />);

            // Content callbacks should be suppressed for controlled value changes
            expect(onContentChange).not.toHaveBeenCalled();
        });

        it('does not call replaceHtml when value is unchanged', () => {
            const { rerender } = render(<NativeRichTextEditor value='<p>same</p>' />);

            mockNativeModule.editorReplaceHtml.mockClear();
            mockNativeModule.editorGetHtml.mockReturnValue('<p>same</p>');

            rerender(<NativeRichTextEditor value='<p>same</p>' />);

            expect(mockNativeModule.editorReplaceHtml).not.toHaveBeenCalled();
        });

        it('syncs native preflight corrections before comparing controlled HTML', () => {
            const onContentChange = jest.fn();
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                selection: { type: 'text', anchor: 3, head: 3 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            mockNativeModule.editorGetHtml.mockReturnValue('<p>teh</p>');
            const { rerender } = render(
                <NativeRichTextEditor value='<p>teh</p>' onContentChange={onContentChange} />
            );

            mockNativeModule.editorReplaceHtml.mockClear();
            onContentChange.mockClear();
            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                JSON.stringify({ ready: true, updateJSON: correctedUpdateJson })
            );
            mockNativeModule.editorGetHtml.mockReturnValue('<p>the</p>');

            rerender(<NativeRichTextEditor value='<p>the</p>' onContentChange={onContentChange} />);

            expect(mockNativeModule.editorReplaceHtml).not.toHaveBeenCalled();
            expect(onContentChange).toHaveBeenCalledWith('<p>the</p>');
        });

        it('continues controlled HTML replacement when a new prop differs from preflight correction', () => {
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                selection: { type: 'text', anchor: 3, head: 3 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            mockNativeModule.editorGetHtml.mockReturnValue('<p>teh</p>');
            const { rerender } = render(<NativeRichTextEditor value='<p>teh</p>' />);

            mockNativeModule.editorReplaceHtml.mockClear();
            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                JSON.stringify({ ready: true, updateJSON: correctedUpdateJson })
            );
            mockNativeModule.editorGetHtml.mockReturnValue('<p>the</p>');

            rerender(<NativeRichTextEditor value='<p>target</p>' />);

            expect(mockNativeModule.editorReplaceHtml).toHaveBeenCalledWith(1, '<p>target</p>');
        });

        it('does not reapply a stale controlled HTML value after a native correction advances the document', () => {
            jest.useFakeTimers();
            try {
                const ref = createRef<NativeRichTextEditorRef>();
                const correctedUpdateJson = JSON.stringify({
                    renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                    selection: { type: 'text', anchor: 3, head: 3 },
                    activeState: {
                        marks: {},
                        markAttrs: {},
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: [],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: true, canRedo: false },
                    documentVersion: 2,
                });

                mockNativeModule.editorGetHtml.mockReturnValue('<p>teh</p>');
                render(<NativeRichTextEditor ref={ref} value='<p>teh</p>' />);

                mockNativeModule.editorReplaceHtml.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({
                            ready: false,
                            blockedReason: 'composition',
                            updateJSON: correctedUpdateJson,
                        })
                    )
                    .mockReturnValue('{"ready":true}');
                mockNativeModule.editorGetHtml.mockReturnValue('<p>the</p>');

                act(() => {
                    ref.current!.toggleMark('bold');
                });
                act(() => {
                    jest.advanceTimersByTime(50);
                });

                expect(mockNativeModule.editorReplaceHtml).not.toHaveBeenCalledWith(
                    1,
                    '<p>teh</p>'
                );
            } finally {
                jest.useRealTimers();
            }
        });

        it('delays composition-blocked controlled HTML retries instead of spinning immediately', () => {
            jest.useFakeTimers();
            try {
                mockNativeModule.editorGetHtml.mockReturnValue('<p>old</p>');
                const { rerender } = render(<NativeRichTextEditor value='<p>old</p>' />);

                mockNativeModule.editorPrepareForCommand.mockClear();
                mockNativeModule.editorReplaceHtml.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({ ready: false, blockedReason: 'composition' })
                    )
                    .mockReturnValue('{"ready":true}');

                rerender(<NativeRichTextEditor value='<p>new</p>' />);

                expect(mockNativeModule.editorPrepareForCommand).toHaveBeenCalledTimes(1);
                expect(mockNativeModule.editorReplaceHtml).not.toHaveBeenCalled();

                act(() => {
                    jest.advanceTimersByTime(49);
                });

                expect(mockNativeModule.editorPrepareForCommand).toHaveBeenCalledTimes(1);

                act(() => {
                    jest.advanceTimersByTime(1);
                });

                expect(mockNativeModule.editorPrepareForCommand.mock.calls.length).toBeGreaterThanOrEqual(2);
                expect(mockNativeModule.editorReplaceHtml).toHaveBeenCalledWith(1, '<p>new</p>');
            } finally {
                jest.useRealTimers();
            }
        });

        it('does not retry controlled HTML updates after destroyed command preflight', () => {
            jest.useFakeTimers();
            try {
                mockNativeModule.editorGetHtml.mockReturnValue('<p>old</p>');
                const { rerender } = render(<NativeRichTextEditor value='<p>old</p>' />);

                mockNativeModule.editorPrepareForCommand.mockClear();
                mockNativeModule.editorReplaceHtml.mockClear();
                mockNativeModule.editorPrepareForCommand.mockReturnValue(
                    JSON.stringify({ ready: false, blockedReason: 'destroyed' })
                );

                rerender(<NativeRichTextEditor value='<p>new</p>' />);

                expect(mockNativeModule.editorPrepareForCommand).toHaveBeenCalledTimes(1);
                expect(mockNativeModule.editorReplaceHtml).not.toHaveBeenCalled();

                act(() => {
                    jest.advanceTimersByTime(200);
                });

                expect(mockNativeModule.editorPrepareForCommand).toHaveBeenCalledTimes(1);
                expect(mockNativeModule.editorReplaceHtml).not.toHaveBeenCalled();
            } finally {
                jest.useRealTimers();
            }
        });

        it('syncs blocked preflight correction before retrying controlled HTML with latest prop', () => {
            jest.useFakeTimers();
            try {
                const onContentChange = jest.fn();
                const correctedUpdateJson = JSON.stringify({
                    renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                    selection: { type: 'text', anchor: 3, head: 3 },
                    activeState: {
                        marks: {},
                        markAttrs: {},
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: [],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: true, canRedo: false },
                    documentVersion: 2,
                });

                mockNativeModule.editorGetHtml.mockReturnValue('<p>teh</p>');
                const { rerender } = render(
                    <NativeRichTextEditor value='<p>teh</p>' onContentChange={onContentChange} />
                );

                mockNativeModule.editorReplaceHtml.mockClear();
                onContentChange.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({
                            ready: false,
                            blockedReason: 'composition',
                            updateJSON: correctedUpdateJson,
                        })
                    )
                    .mockReturnValue('{"ready":true}');
                mockNativeModule.editorGetHtml.mockReturnValue('<p>the</p>');

                rerender(
                    <NativeRichTextEditor
                        value='<p>target</p>'
                        onContentChange={onContentChange}
                    />
                );

                expect(onContentChange).toHaveBeenCalledWith('<p>the</p>');
                expect(mockNativeModule.editorReplaceHtml).not.toHaveBeenCalled();

                act(() => {
                    jest.advanceTimersByTime(50);
                });

                expect(mockNativeModule.editorReplaceHtml).toHaveBeenCalledWith(1, '<p>target</p>');
            } finally {
                jest.useRealTimers();
            }
        });

        it('syncs blocked preflight correction before retrying controlled JSON with latest prop', () => {
            jest.useFakeTimers();
            try {
                const correctedDoc = {
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'the' }] }],
                };
                const targetDoc = {
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'target' }] }],
                };
                const correctedUpdateJson = JSON.stringify({
                    renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                    selection: { type: 'text', anchor: 3, head: 3 },
                    activeState: {
                        marks: {},
                        markAttrs: {},
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: [],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: true, canRedo: false },
                    documentVersion: 2,
                });

                mockNativeModule.editorGetJson.mockReturnValue(
                    JSON.stringify({
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'teh' }] }],
                    })
                );
                const { rerender } = render(
                    <NativeRichTextEditor
                        valueJSON={{
                            type: 'doc',
                            content: [
                                { type: 'paragraph', content: [{ type: 'text', text: 'teh' }] },
                            ],
                        }}
                    />
                );

                mockNativeModule.editorReplaceJson.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({
                            ready: false,
                            blockedReason: 'composition',
                            updateJSON: correctedUpdateJson,
                        })
                    )
                    .mockReturnValue('{"ready":true}');
                mockNativeModule.editorGetJson.mockReturnValue(JSON.stringify(correctedDoc));

                rerender(<NativeRichTextEditor valueJSON={targetDoc} />);

                expect(mockNativeModule.editorReplaceJson).not.toHaveBeenCalled();

                act(() => {
                    jest.advanceTimersByTime(50);
                });

                expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledWith(
                    1,
                    JSON.stringify(targetDoc)
                );
            } finally {
                jest.useRealTimers();
            }
        });

        it('clears queued Android controlled HTML update when corrected prop already matches native', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });

            const queuedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'target', marks: [] }],
                selection: { type: 'text', anchor: 6, head: 6 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 3,
            });
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                selection: { type: 'text', anchor: 3, head: 3 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 4,
            });

            try {
                mockNativeModule.editorGetHtml.mockReturnValue('<p>old</p>');
                const { getByTestId, rerender } = render(
                    <NativeRichTextEditor value='<p>old</p>' />
                );

                mockNativeModule.editorReplaceHtml.mockReturnValueOnce(queuedUpdateJson);
                rerender(<NativeRichTextEditor value='<p>target</p>' />);

                const queuedRevision = getByTestId('native-editor-view').props.editorUpdateRevision;
                expect(getByTestId('native-editor-view').props.editorUpdateJson).toBe(
                    queuedUpdateJson
                );

                mockNativeModule.editorGetHtml.mockReturnValue('<p>the</p>');
                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: correctedUpdateJson },
                    });
                });

                rerender(<NativeRichTextEditor value='<p>the</p>' />);

                expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();
                expect(getByTestId('native-editor-view').props.editorUpdateEditorId).toBe(1);
                expect(getByTestId('native-editor-view').props.editorUpdateRevision).toBeGreaterThan(
                    queuedRevision
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('retries latest Android controlled HTML after in flight native update is acknowledged', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const queuedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'queued', marks: [] }],
                selection: { type: 'text', anchor: 6, head: 6 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 3,
            });
            const latestUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'latest', marks: [] }],
                selection: { type: 'text', anchor: 6, head: 6 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 4,
            });

            try {
                mockNativeModule.editorGetHtml.mockReturnValue('<p>old</p>');
                const { getByTestId, rerender } = render(
                    <NativeRichTextEditor value='<p>old</p>' />
                );

                mockNativeModule.editorReplaceHtml
                    .mockReturnValueOnce(queuedUpdateJson)
                    .mockReturnValueOnce(latestUpdateJson);

                rerender(<NativeRichTextEditor value='<p>queued</p>' />);
                const queuedRevision = getByTestId('native-editor-view').props.editorUpdateRevision;

                rerender(<NativeRichTextEditor value='<p>latest</p>' />);

                expect(mockNativeModule.editorReplaceHtml).toHaveBeenCalledTimes(1);
                expect(mockNativeModule.editorReplaceHtml).toHaveBeenCalledWith(
                    1,
                    '<p>queued</p>'
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorReady({
                        nativeEvent: { editorId: 1, editorUpdateRevision: queuedRevision },
                    });
                });

                expect(mockNativeModule.editorReplaceHtml).toHaveBeenCalledTimes(2);
                expect(mockNativeModule.editorReplaceHtml).toHaveBeenLastCalledWith(
                    1,
                    '<p>latest</p>'
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('retries latest Android controlled JSON after in flight native update is acknowledged', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const oldDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'old' }] }],
            };
            const queuedDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'queued' }] }],
            };
            const latestDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'latest' }] }],
            };
            const queuedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'queued', marks: [] }],
                selection: { type: 'text', anchor: 6, head: 6 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 3,
            });
            const latestUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'latest', marks: [] }],
                selection: { type: 'text', anchor: 6, head: 6 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 4,
            });

            try {
                mockNativeModule.editorGetJson.mockReturnValue(JSON.stringify(oldDoc));
                const { getByTestId, rerender } = render(
                    <NativeRichTextEditor valueJSON={oldDoc} />
                );

                mockNativeModule.editorReplaceJson
                    .mockReturnValueOnce(queuedUpdateJson)
                    .mockReturnValueOnce(latestUpdateJson);

                rerender(<NativeRichTextEditor valueJSON={queuedDoc} />);
                const queuedRevision = getByTestId('native-editor-view').props.editorUpdateRevision;

                rerender(<NativeRichTextEditor valueJSON={latestDoc} />);

                expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledTimes(1);
                expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledWith(
                    1,
                    JSON.stringify(queuedDoc)
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorReady({
                        nativeEvent: { editorId: 1, editorUpdateRevision: queuedRevision },
                    });
                });

                expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledTimes(2);
                expect(mockNativeModule.editorReplaceJson).toHaveBeenLastCalledWith(
                    1,
                    JSON.stringify(latestDoc)
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('applies reset-mode Android controlled JSON immediately while native update is in flight', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const oldDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'old' }] }],
            };
            const remoteDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'remote' }] }],
            };
            const localUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'local', marks: [] }],
                selection: { type: 'text', anchor: 5, head: 5 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            const remoteResetUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'remote', marks: [] }],
                selection: { type: 'text', anchor: 6, head: 6 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 3,
            });
            const onContentChangeJSON = jest.fn();
            const ref = createRef<NativeRichTextEditorRef>();

            try {
                mockNativeModule.editorGetJson.mockReturnValue(JSON.stringify(oldDoc));
                mockNativeModule.editorReplaceJson.mockReturnValueOnce(localUpdateJson);
                mockNativeModule.editorGetCurrentState.mockReturnValueOnce(MOCK_EMPTY_UPDATE_JSON);
                const { getByTestId, rerender } = render(
                    <NativeRichTextEditor
                        ref={ref}
                        valueJSON={oldDoc}
                        valueJSONUpdateMode='reset'
                        onContentChangeJSON={onContentChangeJSON}
                    />
                );

                act(() => {
                    ref.current!.setContentJson({
                        type: 'doc',
                        content: [
                            {
                                type: 'paragraph',
                                content: [{ type: 'text', text: 'local' }],
                            },
                        ],
                    });
                });

                expect(getByTestId('native-editor-view').props.editorUpdateJson).toBe(
                    localUpdateJson
                );

                onContentChangeJSON.mockClear();
                mockNativeModule.editorSetJson.mockClear();
                mockNativeModule.editorReplaceJson.mockClear();
                mockNativeModule.editorGetCurrentState.mockReturnValueOnce(remoteResetUpdateJson);

                rerender(
                    <NativeRichTextEditor
                        ref={ref}
                        valueJSON={remoteDoc}
                        valueJSONUpdateMode='reset'
                        onContentChangeJSON={onContentChangeJSON}
                    />
                );

                expect(mockNativeModule.editorReplaceJson).not.toHaveBeenCalled();
                expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(
                    1,
                    JSON.stringify(remoteDoc)
                );
                expect(mockApplyEditorResetUpdate).toHaveBeenCalledWith(remoteResetUpdateJson);
                expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();
                expect(getByTestId('native-editor-view').props.editorResetUpdateJson).toBe(
                    remoteResetUpdateJson
                );
                expect(onContentChangeJSON).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('preserves live selection for opt-in reset-mode controlled JSON updates', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const oldDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello' }] }],
            };
            const remoteDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello!' }] }],
            };
            const remoteResetUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'hello!', marks: [] }],
                selection: { type: 'text', anchor: 1, head: 1, anchorScalar: 0, headScalar: 0 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 3,
            });
            const restoredSelectionStateJson = JSON.stringify({
                selection: { type: 'text', anchor: 5, head: 5, anchorScalar: 5, headScalar: 5 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: ['bold'],
                    insertableNodes: [],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 3,
            });
            const onSelectionChange = jest.fn();

            try {
                mockNativeModule.editorGetJson.mockReturnValue(JSON.stringify(oldDoc));
                mockNativeModule.editorGetCurrentState.mockReturnValueOnce(MOCK_EMPTY_UPDATE_JSON);
                const { getByTestId, rerender } = render(
                    <NativeRichTextEditor
                        valueJSON={oldDoc}
                        valueJSONUpdateMode='reset'
                        preserveSelectionOnValueJSONReset
                        onSelectionChange={onSelectionChange}
                    />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onSelectionChange({
                        nativeEvent: {
                            editorId: 1,
                            anchor: 5,
                            head: 5,
                            documentVersion: 1,
                            stateJson: JSON.stringify({
                                selection: {
                                    type: 'text',
                                    anchor: 5,
                                    head: 5,
                                    anchorScalar: 5,
                                    headScalar: 5,
                                },
                                activeState: {
                                    marks: {},
                                    markAttrs: {},
                                    nodes: { paragraph: true },
                                    commands: {},
                                    allowedMarks: [],
                                    insertableNodes: [],
                                },
                                historyState: { canUndo: false, canRedo: false },
                                documentVersion: 1,
                            }),
                        },
                    });
                });

                mockApplyEditorResetUpdate.mockClear();
                mockNativeModule.editorSetJson.mockClear();
                mockNativeModule.editorSetSelection.mockClear();
                mockNativeModule.editorGetSelectionState.mockReturnValueOnce(
                    restoredSelectionStateJson
                );
                mockNativeModule.editorGetCurrentState.mockReturnValueOnce(remoteResetUpdateJson);

                rerender(
                    <NativeRichTextEditor
                        valueJSON={remoteDoc}
                        valueJSONUpdateMode='reset'
                        preserveSelectionOnValueJSONReset
                        onSelectionChange={onSelectionChange}
                    />
                );

                expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(
                    1,
                    JSON.stringify(remoteDoc)
                );
                expect(mockNativeModule.editorSetSelection).toHaveBeenCalledWith(1, 5, 5);
                expect(mockNativeModule.editorGetSelectionState).toHaveBeenCalled();
                expect(mockApplyEditorResetUpdate).toHaveBeenCalledTimes(1);
                expect(JSON.parse(mockApplyEditorResetUpdate.mock.calls[0][0]).selection).toEqual({
                    type: 'text',
                    anchor: 5,
                    head: 5,
                    anchorScalar: 5,
                    headScalar: 5,
                });
                expect(onSelectionChange).toHaveBeenLastCalledWith({
                    type: 'text',
                    anchor: 5,
                    head: 5,
                    anchorScalar: 5,
                    headScalar: 5,
                });
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('uses explicit reset-mode selection before falling back to live selection', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const oldDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello' }] }],
            };
            const remoteDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello!' }] }],
            };
            const remoteResetUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'hello!', marks: [] }],
                selection: { type: 'text', anchor: 1, head: 1 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 3,
            });
            const explicitSelectionStateJson = JSON.stringify({
                selection: { type: 'text', anchor: 7, head: 7, anchorScalar: 7, headScalar: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: false, canRedo: false },
                documentVersion: 3,
            });

            try {
                mockNativeModule.editorGetJson.mockReturnValue(JSON.stringify(oldDoc));
                mockNativeModule.editorGetCurrentState.mockReturnValueOnce(MOCK_EMPTY_UPDATE_JSON);
                const { getByTestId, rerender } = render(
                    <NativeRichTextEditor
                        valueJSON={oldDoc}
                        valueJSONUpdateMode='reset'
                        preserveSelectionOnValueJSONReset
                    />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onSelectionChange({
                        nativeEvent: {
                            editorId: 1,
                            anchor: 5,
                            head: 5,
                            documentVersion: 1,
                        },
                    });
                });

                mockApplyEditorResetUpdate.mockClear();
                mockNativeModule.editorSetSelection.mockClear();
                mockNativeModule.editorGetSelectionState.mockReturnValueOnce(
                    explicitSelectionStateJson
                );
                mockNativeModule.editorGetCurrentState.mockReturnValueOnce(remoteResetUpdateJson);

                rerender(
                    <NativeRichTextEditor
                        valueJSON={remoteDoc}
                        valueJSONUpdateMode='reset'
                        preserveSelectionOnValueJSONReset
                        selectionOnValueJSONReset={{ type: 'text', anchor: 7, head: 7 }}
                    />
                );

                expect(mockNativeModule.editorSetSelection).toHaveBeenCalledWith(1, 7, 7);
                expect(JSON.parse(mockApplyEditorResetUpdate.mock.calls[0][0]).selection).toEqual({
                    type: 'text',
                    anchor: 7,
                    head: 7,
                    anchorScalar: 7,
                    headScalar: 7,
                });
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('skips autolink rewriting for controlled value updates', () => {
            const linkedDoc = {
                type: 'doc',
                content: [
                    {
                        type: 'paragraph',
                        content: [{ type: 'text', text: 'Visit https://example.com ' }],
                    },
                ],
            };

            const { rerender } = render(
                <NativeRichTextEditor value='<p>old</p>' autoDetectLinks />
            );

            mockNativeModule.editorGetJson.mockClear();
            mockNativeModule.editorSetMarkAtSelectionScalar.mockClear();
            mockNativeModule.editorGetHtml.mockReturnValueOnce('<p>old</p>');
            mockNativeModule.editorGetJson.mockReturnValueOnce(JSON.stringify(linkedDoc));
            mockNativeModule.editorReplaceHtml.mockReturnValueOnce(
                MOCK_AUTO_LINK_SOURCE_UPDATE_JSON
            );

            rerender(
                <NativeRichTextEditor value='<p>Visit https://example.com</p>' autoDetectLinks />
            );

            expect(mockNativeModule.editorReplaceHtml).toHaveBeenCalledWith(
                1,
                '<p>Visit https://example.com</p>'
            );
            expect(mockNativeModule.editorGetJson).not.toHaveBeenCalled();
            expect(mockNativeModule.editorSetMarkAtSelectionScalar).not.toHaveBeenCalled();
        });

        it('calls setJson when valueJSON prop is provided', () => {
            const doc = { type: 'doc', content: [] };
            render(<NativeRichTextEditor valueJSON={doc} />);

            expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(
                1,
                JSON.stringify(NORMALIZED_EMPTY_DOC)
            );
        });

        it('normalizes empty valueJSON using the provided schema', () => {
            render(
                <NativeRichTextEditor
                    valueJSON={{ type: 'doc', content: [] }}
                    schema={TITLE_FIRST_SCHEMA}
                />
            );

            expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(
                1,
                JSON.stringify(TITLE_EMPTY_DOC)
            );
        });

        it('does not call replaceJson when valueJSON is unchanged', () => {
            const doc = { type: 'doc', content: [{ type: 'paragraph' }] };
            // Mock getJson to return the same doc
            mockNativeModule.editorGetJson.mockReturnValue(JSON.stringify(doc));

            const { rerender } = render(<NativeRichTextEditor valueJSON={doc} />);

            mockNativeModule.editorReplaceJson.mockClear();

            // Re-render with a new object reference but identical content
            rerender(
                <NativeRichTextEditor
                    valueJSON={{ type: 'doc', content: [{ type: 'paragraph' }] }}
                />
            );

            expect(mockNativeModule.editorReplaceJson).not.toHaveBeenCalled();
        });

        it('syncs native preflight corrections before comparing controlled JSON', () => {
            const correctedDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'the' }] }],
            };
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                selection: { type: 'text', anchor: 3, head: 3 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            mockNativeModule.editorGetJson.mockReturnValue(
                JSON.stringify({
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'teh' }] }],
                })
            );
            const { rerender } = render(
                <NativeRichTextEditor
                    valueJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'teh' }] }],
                    }}
                />
            );

            mockNativeModule.editorReplaceJson.mockClear();
            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                JSON.stringify({ ready: true, updateJSON: correctedUpdateJson })
            );
            mockNativeModule.editorGetJson.mockReturnValue(JSON.stringify(correctedDoc));

            rerender(<NativeRichTextEditor valueJSON={correctedDoc} />);

            expect(mockNativeModule.editorReplaceJson).not.toHaveBeenCalled();
        });

        it('continues controlled JSON replacement when a new prop differs from preflight correction', () => {
            const correctedDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'the' }] }],
            };
            const targetDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'target' }] }],
            };
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'the', marks: [] }],
                selection: { type: 'text', anchor: 3, head: 3 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            mockNativeModule.editorGetJson.mockReturnValue(
                JSON.stringify({
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'teh' }] }],
                })
            );
            const { rerender } = render(
                <NativeRichTextEditor
                    valueJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'teh' }] }],
                    }}
                />
            );

            mockNativeModule.editorReplaceJson.mockClear();
            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                JSON.stringify({ ready: true, updateJSON: correctedUpdateJson })
            );
            mockNativeModule.editorGetJson.mockReturnValue(JSON.stringify(correctedDoc));

            rerender(<NativeRichTextEditor valueJSON={targetDoc} />);

            expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledWith(
                1,
                JSON.stringify(targetDoc)
            );
        });

        it('uses the replacement selection when valueJSON changes externally', () => {
            const initialDoc = { type: 'doc', content: [{ type: 'paragraph' }] };
            const nextDoc = {
                type: 'doc',
                content: [
                    {
                        type: 'paragraph',
                        content: [{ type: 'text', text: 'remote change' }],
                    },
                ],
            };

            mockNativeModule.editorGetJson
                .mockReturnValueOnce(JSON.stringify(initialDoc))
                .mockReturnValueOnce(JSON.stringify(initialDoc))
                .mockReturnValueOnce(JSON.stringify(nextDoc));
            mockNativeModule.editorGetCurrentState
                .mockReturnValueOnce(MOCK_EMPTY_UPDATE_JSON)
                .mockReturnValueOnce(
                    JSON.stringify({
                        renderElements: [],
                        selection: { type: 'text', anchor: 5, head: 5 },
                        activeState: {
                            marks: {},
                            nodes: { paragraph: true },
                            commands: {},
                            allowedMarks: [],
                            insertableNodes: [],
                        },
                        historyState: { canUndo: false, canRedo: false },
                    })
                );
            mockNativeModule.editorReplaceJson.mockReturnValue(
                JSON.stringify({
                    renderElements: [],
                    selection: { type: 'text', anchor: 0, head: 0 },
                    activeState: {
                        marks: {},
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: [],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: true, canRedo: false },
                })
            );

            const onSelectionChange = jest.fn();
            const { rerender, getByTestId } = render(
                <NativeRichTextEditor
                    valueJSON={initialDoc}
                    onSelectionChange={onSelectionChange}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: {
                        anchor: 5,
                        head: 5,
                        stateJson: JSON.stringify({
                            selection: { type: 'text', anchor: 5, head: 5 },
                            activeState: {
                                marks: {},
                                nodes: { paragraph: true },
                                commands: {},
                                allowedMarks: [],
                                insertableNodes: [],
                            },
                            historyState: { canUndo: false, canRedo: false },
                        }),
                    },
                });
            });

            onSelectionChange.mockClear();
            mockApplyEditorUpdate.mockClear();
            mockNativeModule.editorSetSelection.mockClear();

            rerender(
                <NativeRichTextEditor valueJSON={nextDoc} onSelectionChange={onSelectionChange} />
            );

            expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledWith(
                1,
                JSON.stringify(nextDoc)
            );
            expect(mockNativeModule.editorSetSelection).not.toHaveBeenCalled();
            expect(mockApplyEditorUpdate).toHaveBeenCalledTimes(1);
            expect(JSON.parse(mockApplyEditorUpdate.mock.calls[0][0]).selection).toEqual({
                type: 'text',
                anchor: 0,
                head: 0,
            });
            expect(onSelectionChange).toHaveBeenCalledWith({
                type: 'text',
                anchor: 0,
                head: 0,
            });
        });
    });

    // ── Callbacks ───────────────────────────────────────────────

    describe('callbacks', () => {
        it('onActiveStateChange fires with ActiveState from update', () => {
            const onActiveStateChange = jest.fn();
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} onActiveStateChange={onActiveStateChange} />);

            act(() => {
                ref.current!.toggleMark('bold');
            });

            expect(onActiveStateChange).toHaveBeenCalledWith({
                marks: { bold: true },
                markAttrs: {},
                nodes: { paragraph: true },
                commands: {},
                allowedMarks: [],
                insertableNodes: [],
            });
        });

        it('onHistoryStateChange fires with HistoryState from update', () => {
            const onHistoryStateChange = jest.fn();
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} onHistoryStateChange={onHistoryStateChange} />);

            act(() => {
                ref.current!.toggleMark('bold');
            });

            expect(onHistoryStateChange).toHaveBeenCalledWith({
                canUndo: true,
                canRedo: false,
            });
        });

        it('onContentChangeJSON fires with JSON from bridge', () => {
            const onContentChangeJSON = jest.fn();
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} onContentChangeJSON={onContentChangeJSON} />);

            act(() => {
                ref.current!.toggleMark('bold');
            });

            expect(onContentChangeJSON).toHaveBeenCalledWith(JSON.parse(MOCK_DOCUMENT_JSON_STR));
        });

        it('emits selection changes before content JSON changes for local edits', () => {
            const onSelectionChange = jest.fn();
            const onContentChangeJSON = jest.fn();
            const ref = createRef<NativeRichTextEditorRef>();
            render(
                <NativeRichTextEditor
                    ref={ref}
                    onSelectionChange={onSelectionChange}
                    onContentChangeJSON={onContentChangeJSON}
                />
            );

            act(() => {
                ref.current!.insertText('!');
            });

            expect(onSelectionChange).toHaveBeenCalledWith({ type: 'text', anchor: 5, head: 5 });
            expect(onContentChangeJSON).toHaveBeenCalledWith(JSON.parse(MOCK_DOCUMENT_JSON_STR));
            expect(onSelectionChange.mock.invocationCallOrder[0]).toBeLessThan(
                onContentChangeJSON.mock.invocationCallOrder[0]
            );
        });

        it('onContentChange fires with HTML from bridge', () => {
            const onContentChange = jest.fn();
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} onContentChange={onContentChange} />);

            act(() => {
                ref.current!.setContent('<p>new</p>');
            });

            expect(onContentChange).toHaveBeenCalledWith('<p>test content</p>');
        });

        it('uses the combined content snapshot when both content callbacks are subscribed', () => {
            const onContentChange = jest.fn();
            const onContentChangeJSON = jest.fn();
            const ref = createRef<NativeRichTextEditorRef>();
            render(
                <NativeRichTextEditor
                    ref={ref}
                    onContentChange={onContentChange}
                    onContentChangeJSON={onContentChangeJSON}
                />
            );

            act(() => {
                ref.current!.toggleMark('bold');
            });

            expect(mockNativeModule.editorGetContentSnapshot).toHaveBeenCalledTimes(1);
            expect(mockNativeModule.editorGetHtml).not.toHaveBeenCalled();
            expect(mockNativeModule.editorGetJson).not.toHaveBeenCalled();
            expect(onContentChange).toHaveBeenCalledWith('<p>test content</p>');
            expect(onContentChangeJSON).toHaveBeenCalledWith(JSON.parse(MOCK_DOCUMENT_JSON_STR));
        });

        it('passes onEditorUpdate handler to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor onContentChange={jest.fn()} />);
            const view = getByTestId('native-editor-view');
            expect(typeof view.props.onEditorUpdate).toBe('function');
        });

        it('passes onSelectionChange handler to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor onSelectionChange={jest.fn()} />);
            const view = getByTestId('native-editor-view');
            expect(typeof view.props.onSelectionChange).toBe('function');
        });

        it('ignores stale native update events from a previous editor id', () => {
            const onSelectionChange = jest.fn();
            const onActiveStateChange = jest.fn();
            const { getByTestId } = render(
                <NativeRichTextEditor
                    onSelectionChange={onSelectionChange}
                    onActiveStateChange={onActiveStateChange}
                />
            );
            onSelectionChange.mockClear();
            onActiveStateChange.mockClear();

            act(() => {
                getByTestId('native-editor-view').props.onEditorUpdate({
                    nativeEvent: {
                        editorId: 999,
                        updateJson: MOCK_BOLD_UPDATE_JSON,
                    },
                });
            });

            expect(onSelectionChange).not.toHaveBeenCalled();
            expect(onActiveStateChange).not.toHaveBeenCalled();
        });

        it('ignores zero-id native events on Android', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onFocus = jest.fn();
            const onActiveStateChange = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor
                        onFocus={onFocus}
                        onActiveStateChange={onActiveStateChange}
                    />
                );
                onActiveStateChange.mockClear();

                act(() => {
                    getByTestId('native-editor-view').props.onFocusChange({
                        nativeEvent: {
                            editorId: 0,
                            isFocused: true,
                        },
                    });
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: {
                            editorId: 0,
                            updateJson: MOCK_BOLD_UPDATE_JSON,
                        },
                    });
                });

                expect(onFocus).not.toHaveBeenCalled();
                expect(onActiveStateChange).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('ignores id-less native events on Android', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onToolbarAction = jest.fn();
            const onQueryChange = jest.fn();

            try {
                const { getByTestId, queryByTestId } = render(
                    <NativeRichTextEditor
                        heightBehavior='autoGrow'
                        onToolbarAction={onToolbarAction}
                        addons={{ mentions: { suggestions: [], onQueryChange } }}
                    />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onToolbarAction({
                        nativeEvent: { key: 'insertMention' },
                    });
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 3, head: 7 },
                                isActive: true,
                            }),
                        },
                    });
                    getByTestId('native-editor-view').props.onContentHeightChange({
                        nativeEvent: { contentHeight: 240 },
                    });
                });

                expect(onToolbarAction).not.toHaveBeenCalled();
                expect(onQueryChange).not.toHaveBeenCalled();
                expect(queryByTestId('native-editor-inline-mention-suggestions')).toBeNull();
                expect(getByTestId('native-editor-view').props.style).not.toEqual(
                    expect.arrayContaining([{ height: 240 }])
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('ignores stale native selection events from a previous editor id', () => {
            const onSelectionChange = jest.fn();
            const { getByTestId } = render(
                <NativeRichTextEditor onSelectionChange={onSelectionChange} />
            );
            onSelectionChange.mockClear();
            mockNativeModule.editorGetSelectionState.mockClear();

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: {
                        editorId: 999,
                        anchor: 5,
                        head: 5,
                    },
                });
            });

            expect(mockNativeModule.editorGetSelectionState).not.toHaveBeenCalled();
            expect(onSelectionChange).not.toHaveBeenCalled();
        });

        it('ignores stale native selection state without applying anchor and head', () => {
            const onSelectionChange = jest.fn();
            const { getByTestId } = render(
                <NativeRichTextEditor onSelectionChange={onSelectionChange} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onEditorUpdate({
                    nativeEvent: { updateJson: MOCK_BOLD_UPDATE_JSON },
                });
            });
            onSelectionChange.mockClear();

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: {
                        anchor: 0,
                        head: 0,
                        stateJson: JSON.stringify({
                            renderElements: [],
                            selection: { type: 'text', anchor: 0, head: 0 },
                            activeState: {
                                marks: {},
                                markAttrs: {},
                                nodes: {},
                                commands: {},
                                allowedMarks: [],
                                insertableNodes: [],
                            },
                            historyState: { canUndo: false, canRedo: false },
                            documentVersion: 1,
                        }),
                    },
                });
            });

            expect(onSelectionChange).not.toHaveBeenCalled();
        });

        it('ignores versionless Android editor updates after a document version is known', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onSelectionChange = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onSelectionChange={onSelectionChange} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: MOCK_BOLD_UPDATE_JSON },
                    });
                });
                onSelectionChange.mockClear();

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: {
                            editorId: 1,
                            updateJson: JSON.stringify({
                                renderElements: [],
                                selection: { type: 'text', anchor: 9, head: 9 },
                                activeState: {
                                    marks: {},
                                    markAttrs: {},
                                    nodes: {},
                                    commands: {},
                                    allowedMarks: [],
                                    insertableNodes: [],
                                },
                                historyState: { canUndo: false, canRedo: false },
                            }),
                        },
                    });
                });

                expect(onSelectionChange).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('ignores stale Android selection fallback state without applying anchor and head', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onSelectionChange = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor onSelectionChange={onSelectionChange} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: MOCK_BOLD_UPDATE_JSON },
                    });
                });
                onSelectionChange.mockClear();
                mockNativeModule.editorSetSelection.mockClear();
                mockNativeModule.editorGetSelectionState.mockReturnValueOnce(
                    JSON.stringify({
                        renderElements: [],
                        selection: { type: 'text', anchor: 9, head: 9 },
                        activeState: {
                            marks: {},
                            markAttrs: {},
                            nodes: {},
                            commands: {},
                            allowedMarks: [],
                            insertableNodes: [],
                        },
                        historyState: { canUndo: false, canRedo: false },
                        documentVersion: 1,
                    })
                );

                act(() => {
                    getByTestId('native-editor-view').props.onSelectionChange({
                        nativeEvent: {
                            editorId: 1,
                            anchor: 9,
                            head: 9,
                        },
                    });
                });

                expect(mockNativeModule.editorSetSelection).not.toHaveBeenCalled();
                expect(onSelectionChange).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('ignores versionless Android selection stateJson after a document version is known', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onSelectionChange = jest.fn();
            const ref = createRef<NativeRichTextEditorRef>();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor ref={ref} onSelectionChange={onSelectionChange} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: MOCK_BOLD_UPDATE_JSON },
                    });
                });
                onSelectionChange.mockClear();
                mockNativeModule.editorSetSelection.mockClear();
                mockNativeModule.editorGetSelection.mockReturnValueOnce(
                    JSON.stringify({ type: 'text', anchor: 0, head: 5 })
                );

                act(() => {
                    getByTestId('native-editor-view').props.onSelectionChange({
                        nativeEvent: {
                            editorId: 1,
                            documentVersion: 2,
                            anchor: 9,
                            head: 9,
                            stateJson: JSON.stringify({
                                renderElements: [],
                                selection: { type: 'text', anchor: 9, head: 9 },
                                activeState: {
                                    marks: {},
                                    markAttrs: {},
                                    nodes: {},
                                    commands: {},
                                    allowedMarks: [],
                                    insertableNodes: [],
                                },
                                historyState: { canUndo: false, canRedo: false },
                            }),
                        },
                    });
                });

                expect(mockNativeModule.editorSetSelection).not.toHaveBeenCalled();
                expect(onSelectionChange).not.toHaveBeenCalled();

                act(() => {
                    ref.current!.toggleMark('bold');
                });

                expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledWith(
                    1,
                    0,
                    5,
                    'bold'
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('refreshes activeState on native selection changes', () => {
            const onActiveStateChange = jest.fn();
            const onHistoryStateChange = jest.fn();
            const onSelectionChange = jest.fn();
            mockNativeModule.editorGetCurrentState.mockReturnValueOnce(
                JSON.stringify({
                    renderElements: [],
                    selection: { type: 'text', anchor: 0, head: 0 },
                    activeState: {
                        marks: {},
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: ['bold'],
                        insertableNodes: ['horizontalRule'],
                    },
                    historyState: { canUndo: false, canRedo: false },
                })
            );

            const { getByTestId } = render(
                <NativeRichTextEditor
                    onActiveStateChange={onActiveStateChange}
                    onHistoryStateChange={onHistoryStateChange}
                    onSelectionChange={onSelectionChange}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: {
                        anchor: 5,
                        head: 5,
                        stateJson: JSON.stringify({
                            selection: { type: 'text', anchor: 5, head: 5 },
                            activeState: {
                                marks: {},
                                nodes: { bulletList: true, listItem: true },
                                commands: { indentList: false, outdentList: true },
                                allowedMarks: ['bold'],
                                insertableNodes: [],
                            },
                            historyState: { canUndo: false, canRedo: false },
                        }),
                    },
                });
            });

            expect(mockNativeModule.editorGetCurrentState).toHaveBeenCalledTimes(1);
            expect(mockNativeModule.editorGetSelectionState).not.toHaveBeenCalled();
            expect(onActiveStateChange).toHaveBeenCalledWith({
                marks: {},
                markAttrs: {},
                nodes: { bulletList: true, listItem: true },
                commands: { indentList: false, outdentList: true },
                allowedMarks: ['bold'],
                insertableNodes: [],
            });
            expect(onHistoryStateChange).toHaveBeenCalledWith({
                canUndo: false,
                canRedo: false,
            });
            expect(onSelectionChange).toHaveBeenCalledWith({
                type: 'text',
                anchor: 5,
                head: 5,
            });
        });

        it('preserves trusted native selection state for subsequent bridge commands', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            const { getByTestId } = render(<NativeRichTextEditor ref={ref} />);

            mockNativeModule.editorDocToScalar.mockClear();
            mockNativeModule.editorToggleMarkAtSelectionScalar.mockClear();

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: {
                        anchor: 1,
                        head: 2,
                        stateJson: JSON.stringify({
                            renderElements: [],
                            selection: { type: 'node', pos: 7 },
                            activeState: {
                                marks: {},
                                markAttrs: {},
                                nodes: { image: true },
                                commands: {},
                                allowedMarks: [],
                                insertableNodes: [],
                            },
                            historyState: { canUndo: false, canRedo: false },
                            documentVersion: 2,
                        }),
                    },
                });
            });
            act(() => {
                ref.current!.toggleMark('bold');
            });

            expect(mockNativeModule.editorDocToScalar).toHaveBeenCalledWith(1, 7);
            expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledWith(
                1,
                7,
                7,
                'bold'
            );
        });

        it('falls back to editorGetSelectionState when the native selection event omits stateJson', () => {
            const onActiveStateChange = jest.fn();
            mockNativeModule.editorGetSelectionState.mockReturnValueOnce(
                JSON.stringify({
                    selection: { type: 'text', anchor: 9, head: 9 },
                    activeState: {
                        marks: { italic: true },
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: ['italic'],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: true, canRedo: false },
                })
            );

            const { getByTestId } = render(
                <NativeRichTextEditor onActiveStateChange={onActiveStateChange} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: { anchor: 9, head: 9 },
                });
            });

            expect(mockNativeModule.editorGetSelectionState).toHaveBeenCalledTimes(1);
            expect(onActiveStateChange).toHaveBeenCalledWith({
                marks: { italic: true },
                markAttrs: {},
                nodes: { paragraph: true },
                commands: {},
                allowedMarks: ['italic'],
                insertableNodes: [],
            });
        });

        it('normalizes a full rendered mention selection to all using the visible mention label length', () => {
            const onSelectionChange = jest.fn();
            mockNativeModule.editorGetCurrentState.mockReturnValueOnce(
                JSON.stringify({
                    renderElements: [
                        { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                        {
                            type: 'opaqueInlineAtom',
                            nodeType: 'mention',
                            label: '@Alice',
                            docPos: 1,
                        },
                        { type: 'blockEnd' },
                    ],
                    selection: { type: 'text', anchor: 0, head: 0 },
                    activeState: {
                        marks: {},
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: ['bold'],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: false, canRedo: false },
                })
            );

            const { getByTestId } = render(
                <NativeRichTextEditor onSelectionChange={onSelectionChange} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: { anchor: 0, head: 6 },
                });
            });

            expect(onSelectionChange).toHaveBeenCalledWith({ type: 'all' });
        });

        it('normalizes full rendered text selection with emoji using scalar length', () => {
            const onSelectionChange = jest.fn();
            mockNativeModule.editorGetCurrentState.mockReturnValueOnce(
                JSON.stringify({
                    renderElements: [
                        { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                        { type: 'textRun', text: 'A😀B', marks: [] },
                        { type: 'blockEnd' },
                    ],
                    selection: { type: 'text', anchor: 0, head: 0 },
                    activeState: {
                        marks: {},
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: ['bold'],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: false, canRedo: false },
                })
            );

            const { getByTestId } = render(
                <NativeRichTextEditor onSelectionChange={onSelectionChange} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: { anchor: 0, head: 3 },
                });
            });

            expect(onSelectionChange).toHaveBeenCalledWith({ type: 'all' });
        });

        it('normalizes backward full rendered text selection with emoji using scalar length', () => {
            const onSelectionChange = jest.fn();
            mockNativeModule.editorGetCurrentState.mockReturnValueOnce(
                JSON.stringify({
                    renderElements: [
                        { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                        { type: 'textRun', text: 'A😀B', marks: [] },
                        { type: 'blockEnd' },
                    ],
                    selection: { type: 'text', anchor: 0, head: 0 },
                    activeState: {
                        marks: {},
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: ['bold'],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: false, canRedo: false },
                })
            );

            const { getByTestId } = render(
                <NativeRichTextEditor onSelectionChange={onSelectionChange} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: { anchor: 3, head: 0 },
                });
            });

            expect(onSelectionChange).toHaveBeenCalledWith({ type: 'all' });
        });

        it('normalizes full rendered mention selection with emoji label using scalar length', () => {
            const onSelectionChange = jest.fn();
            mockNativeModule.editorGetCurrentState.mockReturnValueOnce(
                JSON.stringify({
                    renderElements: [
                        { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                        {
                            type: 'opaqueInlineAtom',
                            nodeType: 'mention',
                            label: '@A😀',
                            docPos: 1,
                        },
                        { type: 'blockEnd' },
                    ],
                    selection: { type: 'text', anchor: 0, head: 0 },
                    activeState: {
                        marks: {},
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: ['bold'],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: false, canRedo: false },
                })
            );

            const { getByTestId } = render(
                <NativeRichTextEditor onSelectionChange={onSelectionChange} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: { anchor: 0, head: 3 },
                });
            });

            expect(onSelectionChange).toHaveBeenCalledWith({ type: 'all' });
        });

        it('normalizes backward full rendered mention selection with emoji label using scalar length', () => {
            const onSelectionChange = jest.fn();
            mockNativeModule.editorGetCurrentState.mockReturnValueOnce(
                JSON.stringify({
                    renderElements: [
                        { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                        {
                            type: 'opaqueInlineAtom',
                            nodeType: 'mention',
                            label: '@A😀',
                            docPos: 1,
                        },
                        { type: 'blockEnd' },
                    ],
                    selection: { type: 'text', anchor: 0, head: 0 },
                    activeState: {
                        marks: {},
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: ['bold'],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: false, canRedo: false },
                })
            );

            const { getByTestId } = render(
                <NativeRichTextEditor onSelectionChange={onSelectionChange} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: { anchor: 3, head: 0 },
                });
            });

            expect(onSelectionChange).toHaveBeenCalledWith({ type: 'all' });
        });

        it('passes onFocusChange handler to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor onFocus={jest.fn()} />);
            const view = getByTestId('native-editor-view');
            expect(typeof view.props.onFocusChange).toBe('function');
        });

        it('renders the JS toolbar inline when toolbarPlacement is inline', () => {
            const { getByTestId } = render(<NativeRichTextEditor toolbarPlacement='inline' />);

            expect(getByTestId('native-editor-js-toolbar')).toBeTruthy();
        });

        it('passes standalone toolbar frames to native while focused', () => {
            const { getByTestId } = render(<NativeRichTextEditor showToolbar={false} />);

            act(() => {
                _setEditorToolbarFrameForTests(1, { x: 12, y: 24, width: 320, height: 48 });
            });
            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });

            expect(JSON.parse(getByTestId('native-editor-view').props.toolbarFrameJson)).toEqual({
                x: 12,
                y: 24,
                width: 320,
                height: 48,
            });
        });

        it('clears standalone toolbar frames after blur to avoid stale native hit-testing', () => {
            const { getByTestId } = render(<NativeRichTextEditor showToolbar={false} />);

            act(() => {
                _setEditorToolbarFrameForTests(1, { x: 12, y: 24, width: 320, height: 48 });
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });

            expect(getByTestId('native-editor-view').props.toolbarFrameJson).toBeDefined();

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: false },
                });
            });

            expect(getByTestId('native-editor-view').props.toolbarFrameJson).toBeUndefined();
        });

        it('honors native blur events during standalone toolbar interaction', () => {
            const onBlur = jest.fn();
            const { getByTestId } = render(
                <NativeRichTextEditor showToolbar={false} onBlur={onBlur} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });
            mockNativeFocus.mockClear();

            act(() => {
                _beginEditorToolbarInteractionForTests();
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: false },
                });
            });

            expect(onBlur).toHaveBeenCalledTimes(1);
            expect(mockNativeFocus).not.toHaveBeenCalled();
        });

        it('serializes multiple standalone toolbar frames for native outside-tap filtering', () => {
            const { getByTestId } = render(<NativeRichTextEditor showToolbar={false} />);

            act(() => {
                _setEditorToolbarFrameForTests(1, { x: 12, y: 24, width: 320, height: 48 });
                _setEditorToolbarFrameForTests(2, { x: 0, y: 480, width: 390, height: 56 });
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });

            expect(JSON.parse(getByTestId('native-editor-view').props.toolbarFrameJson)).toEqual({
                frames: [
                    { x: 12, y: 24, width: 320, height: 48 },
                    { x: 0, y: 480, width: 390, height: 56 },
                ],
            });
        });

        it('does not serialize toolbar frames owned by another editor', () => {
            const { getAllByTestId } = render(
                <View>
                    <NativeRichTextEditor showToolbar={false} />
                    <NativeRichTextEditor showToolbar={false} />
                </View>
            );
            const [firstView, secondView] = getAllByTestId('native-editor-view');

            act(() => {
                _setEditorToolbarFrameForTests(1, { x: 12, y: 24, width: 320, height: 48 }, 2);
                _setEditorToolbarFrameForTests(2, { x: 0, y: 480, width: 390, height: 56 }, 1);
                firstView.props.onFocusChange({
                    nativeEvent: { isFocused: true, editorId: 1 },
                });
            });

            expect(JSON.parse(firstView.props.toolbarFrameJson)).toEqual({
                x: 0,
                y: 480,
                width: 390,
                height: 56,
            });

            act(() => {
                firstView.props.onFocusChange({
                    nativeEvent: { isFocused: false, editorId: 1 },
                });
                secondView.props.onFocusChange({
                    nativeEvent: { isFocused: true, editorId: 2 },
                });
            });

            expect(firstView.props.toolbarFrameJson).toBeUndefined();
            expect(JSON.parse(secondView.props.toolbarFrameJson)).toEqual({
                x: 12,
                y: 24,
                width: 320,
                height: 48,
            });
        });

        it('applies theme.toolbar.marginTop to the inline JS toolbar chrome', () => {
            const { getByTestId } = render(
                <NativeRichTextEditor
                    toolbarPlacement='inline'
                    theme={{ toolbar: { marginTop: 24 } }}
                />
            );

            expect(getByTestId('native-editor-js-toolbar').props.style).toEqual(
                expect.arrayContaining([expect.objectContaining({ marginTop: 24 })])
            );
        });

        it('applies theme.toolbar.showTopBorder to the inline JS toolbar content', () => {
            const { toJSON } = render(
                <NativeRichTextEditor
                    toolbarPlacement='inline'
                    theme={{
                        toolbar: {
                            showTopBorder: true,
                            borderColor: '#123456',
                            borderWidth: 2,
                        },
                    }}
                />
            );

            const tree = toJSON();
            const inlineToolbarContentStyle = StyleSheet.flatten(
                tree?.children?.[1]?.children?.[0]?.props.style
            );

            expect(tree).not.toBeNull();
            expect(inlineToolbarContentStyle.borderTopWidth).toBe(2);
            expect(inlineToolbarContentStyle.borderTopColor).toBe('#123456');
        });

        it('applies theme.toolbar.showTopBorder to inline mention suggestions', () => {
            const { getByTestId } = render(
                <NativeRichTextEditor
                    toolbarPlacement='inline'
                    theme={{
                        toolbar: {
                            showTopBorder: true,
                            borderColor: '#123456',
                            borderWidth: 2,
                        },
                    }}
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                },
                            ],
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });
            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                        }),
                    },
                });
            });

            const style = StyleSheet.flatten(
                getByTestId('native-editor-inline-mention-suggestions').props.style
            );

            expect(style.borderTopWidth).toBe(2);
            expect(style.borderTopColor).toBe('#123456');
        });

        it('grows the native view height from native content-height events in autoGrow mode', () => {
            const { getByTestId } = render(
                <NativeRichTextEditor heightBehavior='autoGrow' style={{ minHeight: 120 }} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onContentHeightChange({
                    nativeEvent: { contentHeight: 240 },
                });
            });

            expect(getByTestId('native-editor-view').props.style).toEqual([
                { minHeight: 120 },
                { height: 240 },
            ]);
        });

        it('mirrors containerStyle minHeight into autoGrow native height styles', () => {
            const { getByTestId } = render(
                <NativeRichTextEditor
                    heightBehavior='autoGrow'
                    containerStyle={{ minHeight: 120 }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onContentHeightChange({
                    nativeEvent: { contentHeight: 240 },
                });
            });

            expect(getByTestId('native-editor-view').props.style).toEqual([
                { minHeight: 120 },
                { height: 240 },
            ]);
        });

        it('uses height from native content-height events on Android', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const pixelRatioSpy = jest.spyOn(PixelRatio, 'get').mockReturnValue(2.625);

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor heightBehavior='autoGrow' style={{ minHeight: 120 }} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onContentHeightChange({
                        nativeEvent: { editorId: 1, contentHeight: 240 },
                    });
                });

                expect(getByTestId('native-editor-view').props.style).toEqual([
                    { minHeight: 120 },
                    { height: Math.ceil(240 / 2.625) },
                ]);
            } finally {
                pixelRatioSpy.mockRestore();
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('updates autoGrow height across sequential native content-height events', () => {
            const { getByTestId } = render(
                <NativeRichTextEditor heightBehavior='autoGrow' style={{ minHeight: 120 }} />
            );

            act(() => {
                getByTestId('native-editor-view').props.onContentHeightChange({
                    nativeEvent: { contentHeight: 180 },
                });
            });

            expect(getByTestId('native-editor-view').props.style).toEqual([
                { minHeight: 120 },
                { height: 180 },
            ]);

            act(() => {
                getByTestId('native-editor-view').props.onContentHeightChange({
                    nativeEvent: { contentHeight: 320 },
                });
            });

            expect(getByTestId('native-editor-view').props.style).toEqual([
                { minHeight: 120 },
                { height: 320 },
            ]);

            act(() => {
                getByTestId('native-editor-view').props.onContentHeightChange({
                    nativeEvent: { contentHeight: 150 },
                });
            });

            expect(getByTestId('native-editor-view').props.style).toEqual([
                { minHeight: 120 },
                { height: 150 },
            ]);
        });

        it('normalizes native update payloads before firing callbacks', () => {
            const onActiveStateChange = jest.fn();
            const onHistoryStateChange = jest.fn();
            const { getByTestId } = render(
                <NativeRichTextEditor
                    onActiveStateChange={onActiveStateChange}
                    onHistoryStateChange={onHistoryStateChange}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onEditorUpdate({
                    nativeEvent: {
                        updateJson: JSON.stringify({
                            renderElements: [],
                            selection: { type: 'text', anchor: 1, head: 1 },
                            activeState: {
                                marks: { bold: true },
                                nodes: { paragraph: true },
                            },
                            historyState: { canUndo: true, canRedo: false },
                        }),
                    },
                });
            });

            expect(onActiveStateChange).toHaveBeenCalledWith({
                marks: { bold: true },
                markAttrs: {},
                nodes: { paragraph: true },
                commands: {},
                allowedMarks: [],
                insertableNodes: [],
            });
            expect(onHistoryStateChange).toHaveBeenCalledWith({
                canUndo: true,
                canRedo: false,
            });
        });

        it('does not auto-link native updates unless the feature is enabled', () => {
            const { getByTestId } = render(<NativeRichTextEditor />);

            act(() => {
                getByTestId('native-editor-view').props.onEditorUpdate({
                    nativeEvent: { updateJson: MOCK_AUTO_LINK_SOURCE_UPDATE_JSON },
                });
            });

            expect(mockNativeModule.editorGetJson).not.toHaveBeenCalled();
            expect(mockNativeModule.editorDocToScalar).not.toHaveBeenCalled();
            expect(mockNativeModule.editorSetMarkAtSelectionScalar).not.toHaveBeenCalled();
            expect(mockNativeModule.editorSetSelection).not.toHaveBeenCalled();
            expect(mockApplyEditorUpdate).not.toHaveBeenCalled();
        });

        it('auto-links a detected URL from native updates when enabled', () => {
            const linkedDoc = {
                type: 'doc',
                content: [
                    {
                        type: 'paragraph',
                        content: [{ type: 'text', text: 'Visit https://example.com ' }],
                    },
                ],
            };
            mockNativeModule.editorGetJson.mockReturnValueOnce(JSON.stringify(linkedDoc));
            mockNativeModule.editorSetMarkAtSelectionScalar.mockReturnValueOnce(
                MOCK_AUTO_LINKED_UPDATE_JSON
            );
            mockNativeModule.editorGetSelectionState.mockReturnValueOnce(
                MOCK_AUTO_LINKED_UPDATE_JSON
            );

            const { getByTestId } = render(<NativeRichTextEditor autoDetectLinks />);

            act(() => {
                getByTestId('native-editor-view').props.onEditorUpdate({
                    nativeEvent: { updateJson: MOCK_AUTO_LINK_SOURCE_UPDATE_JSON },
                });
            });

            expect(mockNativeModule.editorGetJson).toHaveBeenCalledWith(1);
            expect(mockNativeModule.editorDocToScalar).toHaveBeenNthCalledWith(1, 1, 7);
            expect(mockNativeModule.editorDocToScalar).toHaveBeenNthCalledWith(2, 1, 26);
            expect(mockNativeModule.editorSetMarkAtSelectionScalar).toHaveBeenCalledWith(
                1,
                7,
                26,
                'link',
                JSON.stringify({ href: 'https://example.com' })
            );
            expect(mockNativeModule.editorSetSelection).toHaveBeenCalledWith(1, 27, 27);
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(MOCK_AUTO_LINKED_UPDATE_JSON);
        });

        it('retries autolink when explicit mark command is canceled by a native preflight', () => {
            const linkedDoc = {
                type: 'doc',
                content: [
                    {
                        type: 'paragraph',
                        content: [{ type: 'text', text: 'Visit https://example.com ' }],
                    },
                ],
            };
            const preflightUpdateJson = JSON.stringify({
                renderElements: [],
                selection: { type: 'text', anchor: 27, head: 27 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: ['link'],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 6,
            });
            const linkedUpdateJson = JSON.stringify({
                renderElements: [],
                selection: { type: 'text', anchor: 27, head: 27 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: ['link'],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 7,
            });
            mockNativeModule.editorGetJson
                .mockReturnValueOnce(JSON.stringify(linkedDoc))
                .mockReturnValueOnce(JSON.stringify(linkedDoc));
            mockNativeModule.editorPrepareForCommand
                .mockReturnValueOnce(
                    JSON.stringify({ ready: true, updateJSON: preflightUpdateJson })
                )
                .mockReturnValueOnce('{"ready":true}');
            mockNativeModule.editorSetMarkAtSelectionScalar.mockReturnValueOnce(linkedUpdateJson);
            mockNativeModule.editorGetSelectionState.mockReturnValueOnce(linkedUpdateJson);

            const { getByTestId } = render(<NativeRichTextEditor autoDetectLinks />);

            act(() => {
                getByTestId('native-editor-view').props.onEditorUpdate({
                    nativeEvent: { updateJson: MOCK_AUTO_LINK_SOURCE_UPDATE_JSON },
                });
            });

            expect(mockNativeModule.editorGetJson).toHaveBeenCalledTimes(2);
            expect(mockNativeModule.editorSetMarkAtSelectionScalar).toHaveBeenCalledTimes(1);
            expect(mockNativeModule.editorSetMarkAtSelectionScalar).toHaveBeenCalledWith(
                1,
                7,
                26,
                'link',
                JSON.stringify({ href: 'https://example.com' })
            );
            expect(mockApplyEditorUpdate).toHaveBeenCalledWith(linkedUpdateJson);
        });

        it('forwards mention addon query and select events to the configured callbacks', () => {
            const onQueryChange = jest.fn();
            const onSelect = jest.fn();
            const { getByTestId } = render(
                <NativeRichTextEditor
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                            ],
                            onQueryChange,
                            onSelect,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                        }),
                    },
                });
            });
            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsSelect',
                            trigger: '@',
                            suggestionKey: 'u1',
                            attrs: {
                                id: 'u1',
                                kind: 'user',
                                label: '@Alice',
                                mentionSuggestionChar: '@',
                            },
                        }),
                    },
                });
            });

            expect(onQueryChange).toHaveBeenCalledWith({
                query: 'ali',
                trigger: '@',
                range: { anchor: 3, head: 7 },
                isActive: true,
            });
            expect(onSelect).toHaveBeenCalledWith({
                trigger: '@',
                suggestion: {
                    key: 'u1',
                    title: 'Alice',
                    label: '@Alice',
                    attrs: { id: 'u1', kind: 'user' },
                },
                attrs: {
                    id: 'u1',
                    kind: 'user',
                    label: '@Alice',
                    mentionSuggestionChar: '@',
                },
            });
        });

        it('resolves custom attrs before inserting a selected mention when configured', () => {
            const resolveSelectionAttrs = jest.fn(() => ({
                source: 'directory',
                entityType: 'user',
            }));
            const onSelect = jest.fn();
            const { getByTestId } = render(
                <NativeRichTextEditor
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                            ],
                            resolveSelectionAttrs,
                            onSelect,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                            documentVersion: 1,
                        }),
                    },
                });
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsSelectRequest',
                            trigger: '@',
                            suggestionKey: 'u1',
                            range: { anchor: 3, head: 7 },
                            attrs: {
                                id: 'u1',
                                kind: 'user',
                                label: '@Alice',
                                mentionSuggestionChar: '@',
                            },
                            documentVersion: 1,
                        }),
                    },
                });
            });

            expect(resolveSelectionAttrs).toHaveBeenCalledWith({
                trigger: '@',
                suggestion: {
                    key: 'u1',
                    title: 'Alice',
                    label: '@Alice',
                    attrs: { id: 'u1', kind: 'user' },
                },
                range: { anchor: 3, head: 7 },
                attrs: {
                    id: 'u1',
                    kind: 'user',
                    label: '@Alice',
                    mentionSuggestionChar: '@',
                },
                documentVersion: 1,
            });
            expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                1,
                3,
                7,
                JSON.stringify({
                    type: 'doc',
                    content: [
                        {
                            type: 'mention',
                            attrs: {
                                id: 'u1',
                                kind: 'user',
                                label: '@Alice',
                                mentionSuggestionChar: '@',
                                source: 'directory',
                                entityType: 'user',
                            },
                        },
                    ],
                })
            );
            expect(onSelect).toHaveBeenCalledWith({
                trigger: '@',
                suggestion: {
                    key: 'u1',
                    title: 'Alice',
                    label: '@Alice',
                    attrs: { id: 'u1', kind: 'user' },
                },
                attrs: {
                    id: 'u1',
                    kind: 'user',
                    label: '@Alice',
                    mentionSuggestionChar: '@',
                    source: 'directory',
                    entityType: 'user',
                },
                documentVersion: 1,
            });
        });

        it('inserts versionless iOS native mention select requests with a versionless live query', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'ios',
            });
            const resolveSelectionAttrs = jest.fn(() => ({ source: 'directory' }));
            const onSelect = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor
                        addons={{
                            mentions: {
                                suggestions: [
                                    {
                                        key: 'u1',
                                        title: 'Alice',
                                        label: '@Alice',
                                        attrs: { id: 'u1', kind: 'user' },
                                    },
                                ],
                                resolveSelectionAttrs,
                                onSelect,
                            },
                        }}
                    />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 3, head: 7 },
                                isActive: true,
                            }),
                        },
                    });
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            eventJson: JSON.stringify({
                                type: 'mentionsSelectRequest',
                                trigger: '@',
                                suggestionKey: 'u1',
                                range: { anchor: 3, head: 7 },
                                attrs: {
                                    id: 'u1',
                                    kind: 'user',
                                    label: '@Alice',
                                    mentionSuggestionChar: '@',
                                },
                            }),
                        },
                    });
                });

                expect(resolveSelectionAttrs).toHaveBeenCalledWith(
                    expect.not.objectContaining({ documentVersion: expect.any(Number) })
                );
                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                    1,
                    3,
                    7,
                    expect.any(String)
                );
                expect(onSelect).toHaveBeenCalledWith(
                    expect.not.objectContaining({ documentVersion: expect.any(Number) })
                );
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('inserts native mention select requests after preflight sync clears the live query', () => {
            const resolveSelectionAttrs = jest.fn(() => ({ source: 'directory' }));
            const onSelect = jest.fn();
            const preflightUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'an @ali', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            const { getByTestId } = render(
                <NativeRichTextEditor
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                            ],
                            resolveSelectionAttrs,
                            onSelect,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                            documentVersion: 1,
                        }),
                    },
                });
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsSelectRequest',
                            trigger: '@',
                            suggestionKey: 'u1',
                            range: { anchor: 4, head: 7 },
                            attrs: {
                                id: 'u1',
                                kind: 'user',
                                label: '@Alice',
                                mentionSuggestionChar: '@',
                            },
                            updateJson: preflightUpdateJson,
                        }),
                    },
                });
            });

            expect(resolveSelectionAttrs).toHaveBeenCalledWith(
                expect.objectContaining({
                    range: { anchor: 4, head: 7 },
                    documentVersion: 2,
                })
            );
            expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                1,
                4,
                7,
                expect.any(String)
            );
            expect(onSelect).toHaveBeenCalledWith(
                expect.objectContaining({
                    documentVersion: 2,
                    attrs: expect.objectContaining({ source: 'directory' }),
                })
            );
        });

        it('retries native mention select requests after preflight sync clears the live query', () => {
            jest.useFakeTimers();
            const resolveSelectionAttrs = jest.fn(() => ({ source: 'directory' }));
            const onSelect = jest.fn();
            const preflightUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'an @ali', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor
                        addons={{
                            mentions: {
                                suggestions: [
                                    {
                                        key: 'u1',
                                        title: 'Alice',
                                        label: '@Alice',
                                        attrs: { id: 'u1', kind: 'user' },
                                    },
                                ],
                                resolveSelectionAttrs,
                                onSelect,
                            },
                        }}
                    />
                );

                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({ ready: false, blockedReason: 'composition' })
                    )
                    .mockReturnValue('{"ready":true}');

                act(() => {
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 3, head: 7 },
                                isActive: true,
                                documentVersion: 1,
                            }),
                        },
                    });
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            eventJson: JSON.stringify({
                                type: 'mentionsSelectRequest',
                                trigger: '@',
                                suggestionKey: 'u1',
                                range: { anchor: 4, head: 7 },
                                attrs: {
                                    id: 'u1',
                                    kind: 'user',
                                    label: '@Alice',
                                    mentionSuggestionChar: '@',
                                },
                                updateJson: preflightUpdateJson,
                            }),
                        },
                    });
                });

                expect(resolveSelectionAttrs).not.toHaveBeenCalled();
                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).not.toHaveBeenCalled();
                expect(onSelect).not.toHaveBeenCalled();

                act(() => {
                    jest.advanceTimersByTime(50);
                });

                expect(resolveSelectionAttrs).toHaveBeenCalledWith(
                    expect.objectContaining({ documentVersion: 2 })
                );
                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                    1,
                    4,
                    7,
                    expect.any(String)
                );
                expect(onSelect).toHaveBeenCalledWith(
                    expect.objectContaining({ documentVersion: 2 })
                );
            } finally {
                jest.useRealTimers();
            }
        });

        it('retries Android native mention select requests without resolving attrs until unblocked', () => {
            jest.useFakeTimers();
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const resolveSelectionAttrs = jest.fn(() => ({
                source: 'directory',
            }));
            const onSelect = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor
                        addons={{
                            mentions: {
                                suggestions: [
                                    {
                                        key: 'u1',
                                        title: 'Alice',
                                        label: '@Alice',
                                        attrs: { id: 'u1', kind: 'user' },
                                    },
                                ],
                                resolveSelectionAttrs,
                                onSelect,
                            },
                        }}
                    />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 3, head: 7 },
                                isActive: true,
                                documentVersion: 1,
                            }),
                        },
                    });
                });

                mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({ ready: false, blockedReason: 'composition' })
                    )
                    .mockReturnValue('{"ready":true}');

                act(() => {
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsSelectRequest',
                                trigger: '@',
                                suggestionKey: 'u1',
                                range: { anchor: 3, head: 7 },
                                attrs: {
                                    id: 'u1',
                                    kind: 'user',
                                    label: '@Alice',
                                    mentionSuggestionChar: '@',
                                },
                                documentVersion: 1,
                            }),
                        },
                    });
                });

                expect(resolveSelectionAttrs).not.toHaveBeenCalled();
                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).not.toHaveBeenCalled();
                expect(onSelect).not.toHaveBeenCalled();

                act(() => {
                    jest.advanceTimersByTime(50);
                });

                expect(resolveSelectionAttrs).toHaveBeenCalledTimes(1);
                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                    1,
                    3,
                    7,
                    expect.any(String)
                );
                expect(onSelect).toHaveBeenCalledTimes(1);
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
                jest.useRealTimers();
            }
        });

        it('ignores stale native mention select requests', () => {
            const resolveSelectionAttrs = jest.fn(() => ({ source: 'directory' }));
            const onSelect = jest.fn();
            const currentUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'an @ali', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            const { getByTestId } = render(
                <NativeRichTextEditor
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                            ],
                            resolveSelectionAttrs,
                            onSelect,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onEditorUpdate({
                    nativeEvent: { updateJson: currentUpdateJson },
                });
            });
            mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();

            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsSelectRequest',
                            trigger: '@',
                            suggestionKey: 'u1',
                            range: { anchor: 3, head: 7 },
                            attrs: {
                                id: 'u1',
                                kind: 'user',
                                label: '@Alice',
                                mentionSuggestionChar: '@',
                            },
                            documentVersion: 1,
                        }),
                    },
                });
            });

            expect(resolveSelectionAttrs).not.toHaveBeenCalled();
            expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).not.toHaveBeenCalled();
            expect(onSelect).not.toHaveBeenCalled();
        });

        it('drops native mention select requests when the query changes at the same document version', () => {
            const resolveSelectionAttrs = jest.fn(() => ({ source: 'directory' }));
            const onSelect = jest.fn();
            const { getByTestId } = render(
                <NativeRichTextEditor
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                            ],
                            resolveSelectionAttrs,
                            onSelect,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        editorId: 1,
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                            documentVersion: 1,
                        }),
                    },
                });
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        editorId: 1,
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'alicia',
                            trigger: '@',
                            range: { anchor: 3, head: 10 },
                            isActive: true,
                            documentVersion: 1,
                        }),
                    },
                });
            });

            mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();

            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        editorId: 1,
                        eventJson: JSON.stringify({
                            type: 'mentionsSelectRequest',
                            trigger: '@',
                            suggestionKey: 'u1',
                            range: { anchor: 3, head: 7 },
                            attrs: {
                                id: 'u1',
                                kind: 'user',
                                label: '@Alice',
                                mentionSuggestionChar: '@',
                            },
                            documentVersion: 1,
                        }),
                    },
                });
            });

            expect(resolveSelectionAttrs).not.toHaveBeenCalled();
            expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).not.toHaveBeenCalled();
            expect(onSelect).not.toHaveBeenCalled();
        });

        it('ignores stale Android mention query and select events with matching editor id', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const resolveSelectionAttrs = jest.fn(() => ({ source: 'directory' }));
            const onQueryChange = jest.fn();
            const onSelect = jest.fn();
            const currentUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'an @ali', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            try {
                const { getByTestId, queryByTestId } = render(
                    <NativeRichTextEditor
                        toolbarPlacement='inline'
                        addons={{
                            mentions: {
                                suggestions: [
                                    {
                                        key: 'u1',
                                        title: 'Alice',
                                        label: '@Alice',
                                        attrs: { id: 'u1', kind: 'user' },
                                    },
                                ],
                                resolveSelectionAttrs,
                                onQueryChange,
                                onSelect,
                            },
                        }}
                    />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onFocusChange({
                        nativeEvent: { editorId: 1, isFocused: true },
                    });
                    getByTestId('native-editor-view').props.onEditorUpdate({
                        nativeEvent: { editorId: 1, updateJson: currentUpdateJson },
                    });
                });
                mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();

                act(() => {
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 3, head: 7 },
                                isActive: true,
                                documentVersion: 1,
                            }),
                        },
                    });
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsSelectRequest',
                                trigger: '@',
                                suggestionKey: 'u1',
                                range: { anchor: 3, head: 7 },
                                attrs: {
                                    id: 'u1',
                                    kind: 'user',
                                    label: '@Alice',
                                    mentionSuggestionChar: '@',
                                },
                                documentVersion: 1,
                            }),
                        },
                    });
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsSelect',
                                trigger: '@',
                                suggestionKey: 'u1',
                                attrs: {
                                    id: 'u1',
                                    kind: 'user',
                                    label: '@Alice',
                                    mentionSuggestionChar: '@',
                                },
                                documentVersion: 1,
                            }),
                        },
                    });
                });

                expect(resolveSelectionAttrs).not.toHaveBeenCalled();
                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).not.toHaveBeenCalled();
                expect(onQueryChange).not.toHaveBeenCalled();
                expect(onSelect).not.toHaveBeenCalled();
                expect(queryByTestId('native-editor-inline-mention-suggestions')).toBeNull();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('ignores versionless Android mention events after a document version is known', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onQueryChange = jest.fn();
            const onSelect = jest.fn();
            const resolveSelectionAttrs = jest.fn(() => ({ source: 'directory' }));

            try {
                const { getByTestId, queryByTestId } = render(
                    <NativeRichTextEditor
                        toolbarPlacement='inline'
                        addons={{
                            mentions: {
                                suggestions: [
                                    {
                                        key: 'u1',
                                        title: 'Alice',
                                        label: '@Alice',
                                        attrs: { id: 'u1', kind: 'user' },
                                    },
                                ],
                                onQueryChange,
                                onSelect,
                                resolveSelectionAttrs,
                            },
                        }}
                    />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onFocusChange({
                        nativeEvent: { editorId: 1, isFocused: true },
                    });
                });
                act(() => {
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 3, head: 7 },
                                isActive: true,
                            }),
                        },
                    });
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsSelectRequest',
                                trigger: '@',
                                suggestionKey: 'u1',
                                range: { anchor: 3, head: 7 },
                                attrs: {
                                    id: 'u1',
                                    kind: 'user',
                                    label: '@Alice',
                                    mentionSuggestionChar: '@',
                                },
                            }),
                        },
                    });
                });

                expect(onQueryChange).not.toHaveBeenCalled();
                expect(resolveSelectionAttrs).not.toHaveBeenCalled();
                expect(onSelect).not.toHaveBeenCalled();
                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).not.toHaveBeenCalled();
                expect(queryByTestId('native-editor-inline-mention-suggestions')).toBeNull();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('resolves mention themes before inserting a selected mention when configured', () => {
            const resolveSelectionAttrs = jest.fn(() => ({
                entityType: 'user',
            }));
            const resolveTheme = jest.fn(({ attrs }) =>
                attrs.entityType === 'user'
                    ? { textColor: '#445566', backgroundColor: '#eef6ff' }
                    : null
            );
            const onSelect = jest.fn();
            const { getByTestId } = render(
                <NativeRichTextEditor
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                            ],
                            resolveSelectionAttrs,
                            resolveTheme,
                            onSelect,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                            documentVersion: 1,
                        }),
                    },
                });
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsSelectRequest',
                            trigger: '@',
                            suggestionKey: 'u1',
                            range: { anchor: 3, head: 7 },
                            attrs: {
                                id: 'u1',
                                kind: 'user',
                                label: '@Alice',
                                mentionSuggestionChar: '@',
                            },
                            documentVersion: 1,
                        }),
                    },
                });
            });

            const finalAttrs = {
                id: 'u1',
                kind: 'user',
                label: '@Alice',
                mentionSuggestionChar: '@',
                entityType: 'user',
                mentionTheme: { textColor: '#445566', backgroundColor: '#eef6ff' },
            };
            const suggestion = {
                key: 'u1',
                title: 'Alice',
                label: '@Alice',
                attrs: { id: 'u1', kind: 'user' },
            };

            expect(resolveTheme).toHaveBeenCalledWith({
                trigger: '@',
                suggestion,
                range: { anchor: 3, head: 7 },
                attrs: {
                    id: 'u1',
                    kind: 'user',
                    label: '@Alice',
                    mentionSuggestionChar: '@',
                    entityType: 'user',
                },
                documentVersion: 1,
            });
            expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                1,
                3,
                7,
                JSON.stringify({
                    type: 'doc',
                    content: [
                        {
                            type: 'mention',
                            attrs: finalAttrs,
                        },
                    ],
                })
            );
            expect(onSelect).toHaveBeenCalledWith({
                trigger: '@',
                suggestion,
                attrs: finalAttrs,
                documentVersion: 1,
            });
        });

        it('renders inline mention suggestions in the JS toolbar and inserts the selected mention', () => {
            const onSelect = jest.fn();
            const { getByTestId, getByText, queryByTestId } = render(
                <NativeRichTextEditor
                    toolbarPlacement='inline'
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: 'Alice',
                                    subtitle: 'Design',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                                {
                                    key: 'u2',
                                    title: 'Bob',
                                    label: 'Bob',
                                    attrs: { id: 'u2', kind: 'user' },
                                },
                            ],
                            onSelect,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });
            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                        }),
                    },
                });
            });

            expect(getByTestId('native-editor-inline-mention-suggestions')).toBeTruthy();
            expect(getByTestId('native-editor-inline-mention-suggestion-u1')).toBeTruthy();
            expect(getByText('@Alice')).toBeTruthy();
            expect(queryByTestId('native-editor-inline-mention-suggestion-u2')).toBeNull();

            fireEvent.press(getByTestId('native-editor-inline-mention-suggestion-u1'));

            expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                1,
                3,
                7,
                JSON.stringify({
                    type: 'doc',
                    content: [
                        {
                            type: 'mention',
                            attrs: {
                                id: 'u1',
                                kind: 'user',
                                label: 'Alice',
                                mentionSuggestionChar: '@',
                            },
                        },
                    ],
                })
            );
            expect(onSelect).toHaveBeenCalledWith({
                trigger: '@',
                suggestion: {
                    key: 'u1',
                    title: 'Alice',
                    label: 'Alice',
                    subtitle: 'Design',
                    attrs: { id: 'u1', kind: 'user' },
                },
                attrs: {
                    id: 'u1',
                    kind: 'user',
                    label: 'Alice',
                    mentionSuggestionChar: '@',
                },
            });
        });

        it('passes documentVersion through inline mention attrs and selection callbacks', () => {
            const onSelect = jest.fn();
            const resolveSelectionAttrs = jest.fn(() => ({ source: 'inline' }));
            const { getByTestId } = render(
                <NativeRichTextEditor
                    toolbarPlacement='inline'
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                            ],
                            resolveSelectionAttrs,
                            onSelect,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });
            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                            documentVersion: 2,
                        }),
                    },
                });
            });

            fireEvent.press(getByTestId('native-editor-inline-mention-suggestion-u1'));

            expect(resolveSelectionAttrs).toHaveBeenCalledWith(
                expect.objectContaining({
                    documentVersion: 2,
                    range: { anchor: 3, head: 7 },
                })
            );
            expect(onSelect).toHaveBeenCalledWith(
                expect.objectContaining({
                    documentVersion: 2,
                    attrs: expect.objectContaining({ source: 'inline' }),
                })
            );
        });

        it('cancels inline mention insertion when preflight commits an autocorrect first', () => {
            const onSelect = jest.fn();
            const resolveSelectionAttrs = jest.fn(() => ({ source: 'inline' }));
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'an @ali', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            const { getByTestId, queryByTestId } = render(
                <NativeRichTextEditor
                    toolbarPlacement='inline'
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                            ],
                            resolveSelectionAttrs,
                            onSelect,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });
            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                        }),
                    },
                });
            });

            mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();
            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                JSON.stringify({ ready: true, updateJSON: correctedUpdateJson })
            );
            fireEvent.press(getByTestId('native-editor-inline-mention-suggestion-u1'));

            expect(
                mockNativeModule.editorInsertContentJsonAtSelectionScalar
            ).not.toHaveBeenCalled();
            expect(resolveSelectionAttrs).not.toHaveBeenCalled();
            expect(onSelect).not.toHaveBeenCalled();
            expect(queryByTestId('native-editor-inline-mention-suggestions')).toBeNull();
        });

        it('retains and retries Android inline mention insertion while composition is blocked', () => {
            jest.useFakeTimers();
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onSelect = jest.fn();

            try {
                const { getByTestId, queryByTestId } = render(
                    <NativeRichTextEditor
                        toolbarPlacement='inline'
                        addons={{
                            mentions: {
                                suggestions: [
                                    {
                                        key: 'u1',
                                        title: 'Alice',
                                        label: '@Alice',
                                        attrs: { id: 'u1', kind: 'user' },
                                    },
                                ],
                                onSelect,
                            },
                        }}
                    />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onFocusChange({
                        nativeEvent: { editorId: 1, isFocused: true },
                    });
                });
                act(() => {
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 3, head: 7 },
                                isActive: true,
                                documentVersion: 1,
                            }),
                        },
                    });
                });

                mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({ ready: false, blockedReason: 'composition' })
                    )
                    .mockReturnValue('{"ready":true}');

                fireEvent.press(getByTestId('native-editor-inline-mention-suggestion-u1'));

                expect(
                    mockNativeModule.editorInsertContentJsonAtSelectionScalar
                ).not.toHaveBeenCalled();
                expect(getByTestId('native-editor-inline-mention-suggestions')).toBeTruthy();

                act(() => {
                    jest.advanceTimersByTime(50);
                });

                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                    1,
                    3,
                    7,
                    expect.any(String)
                );
                expect(onSelect).toHaveBeenCalledTimes(1);
                expect(queryByTestId('native-editor-inline-mention-suggestions')).toBeNull();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
                jest.useRealTimers();
            }
        });

        it('cancels iOS inline mention retry when the query changes before retry', () => {
            jest.useFakeTimers();
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'ios',
            });
            const onSelect = jest.fn();

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor
                        toolbarPlacement='inline'
                        addons={{
                            mentions: {
                                suggestions: [
                                    {
                                        key: 'u1',
                                        title: 'Alice',
                                        label: '@Alice',
                                        attrs: { id: 'u1', kind: 'user' },
                                    },
                                ],
                                onSelect,
                            },
                        }}
                    />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onFocusChange({
                        nativeEvent: { editorId: 1, isFocused: true },
                    });
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 3, head: 7 },
                                isActive: true,
                                documentVersion: 1,
                            }),
                        },
                    });
                });

                mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand
                    .mockReturnValueOnce(
                        JSON.stringify({ ready: false, blockedReason: 'composition' })
                    )
                    .mockReturnValue('{"ready":true}');

                fireEvent.press(getByTestId('native-editor-inline-mention-suggestion-u1'));

                act(() => {
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'alex',
                                trigger: '@',
                                range: { anchor: 3, head: 8 },
                                isActive: true,
                                documentVersion: 1,
                            }),
                        },
                    });
                    jest.advanceTimersByTime(50);
                });

                expect(
                    mockNativeModule.editorInsertContentJsonAtSelectionScalar
                ).not.toHaveBeenCalled();
                expect(onSelect).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
                jest.useRealTimers();
            }
        });

        it('clears Android inline mention query state after same-version editor recreation', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onSelect = jest.fn();
            const addons = {
                mentions: {
                    suggestions: [
                        {
                            key: 'u1',
                            title: 'Alice',
                            label: '@Alice',
                            attrs: { id: 'u1', kind: 'user' },
                        },
                    ],
                    onSelect,
                },
            };

            try {
                const { getByTestId, queryByTestId, rerender } = render(
                    <NativeRichTextEditor toolbarPlacement='inline' addons={addons} />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onFocusChange({
                        nativeEvent: { editorId: 1, isFocused: true },
                    });
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 3, head: 7 },
                                isActive: true,
                                documentVersion: 1,
                            }),
                        },
                    });
                });

                expect(getByTestId('native-editor-inline-mention-suggestion-u1')).toBeTruthy();

                rerender(
                    <NativeRichTextEditor
                        toolbarPlacement='inline'
                        addons={addons}
                        maxLength={100}
                    />
                );

                expect(getByTestId('native-editor-view').props.editorId).toBe(2);
                expect(queryByTestId('native-editor-inline-mention-suggestions')).toBeNull();
                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).not.toHaveBeenCalled();
                expect(onSelect).not.toHaveBeenCalled();
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('uses a recomputed inline mention range after preflight autocorrect', () => {
            const onSelect = jest.fn();
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'an @ali', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            const { getByTestId } = render(
                <NativeRichTextEditor
                    toolbarPlacement='inline'
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                            ],
                            onSelect,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });
            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                        }),
                    },
                });
            });

            mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();
            mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                JSON.stringify({ ready: true, updateJSON: correctedUpdateJson })
            );
            fireEvent.press(getByTestId('native-editor-inline-mention-suggestion-u1'));

            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 4, head: 7 },
                            isActive: true,
                        }),
                    },
                });
            });
            fireEvent.press(getByTestId('native-editor-inline-mention-suggestion-u1'));

            expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                1,
                4,
                7,
                expect.any(String)
            );
            expect(onSelect).toHaveBeenCalledTimes(1);
        });

        it('uses fresh Android mention versions after preflight autocorrect', () => {
            const originalPlatform = Platform.OS;
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: 'android',
            });
            const onSelect = jest.fn();
            const correctedUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'an @ali', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });

            try {
                const { getByTestId } = render(
                    <NativeRichTextEditor
                        toolbarPlacement='inline'
                        addons={{
                            mentions: {
                                suggestions: [
                                    {
                                        key: 'u1',
                                        title: 'Alice',
                                        label: '@Alice',
                                        attrs: { id: 'u1', kind: 'user' },
                                    },
                                ],
                                onSelect,
                            },
                        }}
                    />
                );

                act(() => {
                    getByTestId('native-editor-view').props.onFocusChange({
                        nativeEvent: { editorId: 1, isFocused: true },
                    });
                });
                act(() => {
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 3, head: 7 },
                                isActive: true,
                                documentVersion: 1,
                            }),
                        },
                    });
                });

                mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();
                mockNativeModule.editorPrepareForCommand.mockReturnValueOnce(
                    JSON.stringify({ ready: true, updateJSON: correctedUpdateJson })
                );
                fireEvent.press(getByTestId('native-editor-inline-mention-suggestion-u1'));

                expect(
                    mockNativeModule.editorInsertContentJsonAtSelectionScalar
                ).not.toHaveBeenCalled();

                act(() => {
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 5, head: 7 },
                                isActive: true,
                            }),
                        },
                    });
                });
                expect(() =>
                    getByTestId('native-editor-inline-mention-suggestion-u1')
                ).toThrow();

                act(() => {
                    getByTestId('native-editor-view').props.onAddonEvent({
                        nativeEvent: {
                            editorId: 1,
                            eventJson: JSON.stringify({
                                type: 'mentionsQueryChange',
                                query: 'ali',
                                trigger: '@',
                                range: { anchor: 4, head: 7 },
                                isActive: true,
                                documentVersion: 2,
                            }),
                        },
                    });
                });
                fireEvent.press(getByTestId('native-editor-inline-mention-suggestion-u1'));

                expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                    1,
                    4,
                    7,
                    expect.any(String)
                );
                expect(onSelect).toHaveBeenCalledTimes(1);
            } finally {
                Object.defineProperty(Platform, 'OS', {
                    configurable: true,
                    value: originalPlatform,
                });
            }
        });

        it('ignores stale inline mention queries from older document versions', () => {
            const onSelect = jest.fn();
            const onQueryChange = jest.fn();
            const currentUpdateJson = JSON.stringify({
                renderElements: [{ type: 'textRun', text: 'an @ali', marks: [] }],
                selection: { type: 'text', anchor: 7, head: 7 },
                activeState: {
                    marks: {},
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: [],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
                documentVersion: 2,
            });
            const { getByTestId, queryByTestId } = render(
                <NativeRichTextEditor
                    toolbarPlacement='inline'
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                            ],
                            onQueryChange,
                            onSelect,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });
            act(() => {
                getByTestId('native-editor-view').props.onEditorUpdate({
                    nativeEvent: { updateJson: currentUpdateJson },
                });
            });
            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                            documentVersion: 1,
                        }),
                    },
                });
            });

            mockNativeModule.editorInsertContentJsonAtSelectionScalar.mockClear();

            expect(
                mockNativeModule.editorInsertContentJsonAtSelectionScalar
            ).not.toHaveBeenCalled();
            expect(onQueryChange).not.toHaveBeenCalled();
            expect(onSelect).not.toHaveBeenCalled();
            expect(queryByTestId('native-editor-inline-mention-suggestions')).toBeNull();
        });

        it('clears inline mention queries when the document advances past their version', () => {
            const onQueryChange = jest.fn();
            const { getByTestId, queryByTestId } = render(
                <NativeRichTextEditor
                    toolbarPlacement='inline'
                    addons={{
                        mentions: {
                            suggestions: [
                                {
                                    key: 'u1',
                                    title: 'Alice',
                                    label: '@Alice',
                                    attrs: { id: 'u1', kind: 'user' },
                                },
                            ],
                            onQueryChange,
                        },
                    }}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });
            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                            documentVersion: 1,
                        }),
                    },
                });
            });

            expect(getByTestId('native-editor-inline-mention-suggestions')).toBeTruthy();

            act(() => {
                getByTestId('native-editor-view').props.onEditorUpdate({
                    nativeEvent: { updateJson: MOCK_BOLD_UPDATE_JSON },
                });
            });

            expect(queryByTestId('native-editor-inline-mention-suggestions')).toBeNull();
        });

        it('renders mention suggestions in a standalone EditorToolbar and inserts the selected mention', () => {
            const onSelect = jest.fn();
            const resolveTheme = jest.fn(({ attrs }) =>
                attrs.kind === 'channel'
                    ? {
                          textColor: '#0055AA',
                          backgroundColor: '#E8F2FF',
                          borderRadius: 18,
                          fontWeight: '700' as const,
                      }
                    : undefined
            );
            const { getByTestId, getByText, queryByTestId } = render(
                <View>
                    <NativeRichTextEditor
                        showToolbar={false}
                        addons={{
                            mentions: {
                                theme: {
                                    backgroundColor: '#FAFAFA',
                                    popoverBackgroundColor: '#111111',
                                    popoverBorderColor: '#222222',
                                    popoverBorderWidth: 2,
                                    popoverBorderRadius: 16,
                                    optionTextColor: '#333333',
                                    optionSecondaryTextColor: '#777777',
                                    optionHighlightedBackgroundColor: '#DDEEFF',
                                    optionHighlightedTextColor: '#004488',
                                },
                                suggestions: [
                                    {
                                        key: 'u1',
                                        title: 'Alice',
                                        label: 'Alice',
                                        subtitle: 'Design',
                                        attrs: { id: 'u1', kind: 'user' },
                                    },
                                    {
                                        key: 'u2',
                                        title: 'Bob',
                                        label: 'Bob',
                                        attrs: { id: 'u2', kind: 'user' },
                                    },
                                ],
                                resolveSelectionAttrs: ({ suggestion, attrs }) => ({
                                    ...attrs,
                                    kind: suggestion.key === 'u1' ? 'channel' : 'user',
                                }),
                                resolveTheme,
                                onSelect,
                            },
                        }}
                    />
                    <EditorToolbar
                        activeState={{
                            marks: {},
                            markAttrs: {},
                            nodes: {},
                            commands: {},
                            allowedMarks: [],
                            insertableNodes: [],
                        }}
                        historyState={{ canUndo: false, canRedo: false }}
                        onToggleBold={jest.fn()}
                        onToggleItalic={jest.fn()}
                        onToggleUnderline={jest.fn()}
                        onToggleStrike={jest.fn()}
                        onUndo={jest.fn()}
                        onRedo={jest.fn()}
                    />
                </View>
            );

            act(() => {
                _setEditorToolbarFrameForTests(1, { x: 0, y: 400, width: 390, height: 48 });
            });
            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true },
                });
            });
            act(() => {
                getByTestId('native-editor-view').props.onAddonEvent({
                    nativeEvent: {
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'ali',
                            trigger: '@',
                            range: { anchor: 3, head: 7 },
                            isActive: true,
                        }),
                    },
                });
            });

            expect(getByTestId('editor-toolbar-mention-suggestions')).toBeTruthy();
            expect(getByTestId('editor-toolbar-mention-suggestion-u1')).toBeTruthy();
            expect(queryByTestId('editor-toolbar-mention-suggestion-u2')).toBeNull();

            const suggestionListStyle = StyleSheet.flatten(
                getByTestId('editor-toolbar-mention-suggestions').props.style
            );
            const suggestionChipStyle = StyleSheet.flatten(
                getByTestId('editor-toolbar-mention-suggestion-u1').props.style
            );
            const suggestionTitleStyle = StyleSheet.flatten(getByText('@Alice').props.style);

            expect(suggestionListStyle.backgroundColor).toBe('#111111');
            expect(suggestionListStyle.borderColor).toBe('#222222');
            expect(suggestionListStyle.borderWidth).toBe(2);
            expect(suggestionListStyle.borderRadius).toBe(16);
            expect(suggestionChipStyle.backgroundColor).toBe('#E8F2FF');
            expect(suggestionChipStyle.borderRadius).toBe(18);
            expect(suggestionTitleStyle.color).toBe('#0055AA');
            expect(suggestionTitleStyle.fontWeight).toBe('700');
            expect(resolveTheme).toHaveBeenCalledWith(
                expect.objectContaining({
                    suggestion: expect.objectContaining({ key: 'u1' }),
                    attrs: expect.objectContaining({ kind: 'channel' }),
                })
            );

            fireEvent.press(getByTestId('editor-toolbar-mention-suggestion-u1'));

            expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                1,
                3,
                7,
                JSON.stringify({
                    type: 'doc',
                    content: [
                        {
                            type: 'mention',
                            attrs: {
                                id: 'u1',
                                kind: 'channel',
                                label: 'Alice',
                                mentionSuggestionChar: '@',
                                mentionTheme: {
                                    textColor: '#0055AA',
                                    backgroundColor: '#E8F2FF',
                                    borderRadius: 18,
                                    fontWeight: '700',
                                },
                            },
                        },
                    ],
                })
            );
            expect(onSelect).toHaveBeenCalledWith({
                trigger: '@',
                suggestion: {
                    key: 'u1',
                    title: 'Alice',
                    label: 'Alice',
                    subtitle: 'Design',
                    attrs: { id: 'u1', kind: 'user' },
                },
                attrs: {
                    id: 'u1',
                    kind: 'channel',
                    label: 'Alice',
                    mentionSuggestionChar: '@',
                    mentionTheme: {
                        textColor: '#0055AA',
                        backgroundColor: '#E8F2FF',
                        borderRadius: 18,
                        fontWeight: '700',
                    },
                },
            });
        });
    });

    // ── Cleanup ─────────────────────────────────────────────────

    describe('cleanup', () => {
        it('destroys bridge on unmount', () => {
            const { unmount } = render(<NativeRichTextEditor />);

            unmount();

            expect(mockNativeModule.editorDestroy).toHaveBeenCalledWith(1);
        });

        it('ref methods are safe no-ops after unmount (getContent returns empty)', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            const { unmount } = render(<NativeRichTextEditor ref={ref} />);

            // Capture ref before unmount, since React clears it
            const capturedRef = ref.current!;
            unmount();

            // Should not throw — guard against destroyed bridge
            expect(capturedRef.getContent()).toBe('');
            expect(capturedRef.getTextContent()).toBe('');
            expect(capturedRef.getContentJson()).toEqual({});
            expect(capturedRef.canUndo()).toBe(false);
            expect(capturedRef.canRedo()).toBe(false);
        });

        it('ref mutation methods are no-ops after unmount (no crash)', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            const { unmount } = render(<NativeRichTextEditor ref={ref} />);

            const capturedRef = ref.current!;
            unmount();

            // Clear mocks to verify no further native calls
            jest.clearAllMocks();

            // These should be silent no-ops
            capturedRef.toggleMark('bold');
            capturedRef.toggleList('bulletList');
            capturedRef.insertNode('horizontalRule');
            capturedRef.insertText('hello');
            capturedRef.undo();
            capturedRef.redo();
            capturedRef.setContent('<p>x</p>');
            capturedRef.setContentJson({ type: 'doc' });
            capturedRef.clearContent();
            capturedRef.insertContentHtml('<p>x</p>');
            capturedRef.insertContentJson({ type: 'doc' });

            // No native module calls after unmount
            expect(mockNativeModule.editorToggleMarkAtSelectionScalar).not.toHaveBeenCalled();
            expect(mockNativeModule.editorWrapInListAtSelectionScalar).not.toHaveBeenCalled();
            expect(mockNativeModule.editorInsertNodeAtSelectionScalar).not.toHaveBeenCalled();
            expect(mockNativeModule.editorInsertText).not.toHaveBeenCalled();
            expect(mockNativeModule.editorReplaceSelectionText).not.toHaveBeenCalled();
            expect(mockNativeModule.editorReplaceHtml).not.toHaveBeenCalled();
            expect(mockNativeModule.editorReplaceJson).not.toHaveBeenCalled();
            expect(mockNativeModule.editorSetJson).not.toHaveBeenCalled();
            expect(mockNativeModule.editorUndo).not.toHaveBeenCalled();
            expect(mockNativeModule.editorRedo).not.toHaveBeenCalled();
        });
    });
});
