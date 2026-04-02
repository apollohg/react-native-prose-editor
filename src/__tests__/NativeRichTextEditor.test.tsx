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
    activeState: { marks: {}, markAttrs: {}, nodes: {}, commands: {} },
    historyState: { canUndo: false, canRedo: false },
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
});

const MOCK_DOCUMENT_JSON_STR = JSON.stringify({
    type: 'doc',
    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello' }] }],
});

// ─── Mock Setup (must be before imports) ────────────────────────

let mockEditorIdCounter = 0;
const mockApplyEditorUpdate = jest.fn();
const mockNativeFocus = jest.fn();
const mockNativeBlur = jest.fn();

const mockNativeModule = {
    editorCreate: jest.fn((_configJson: string) => ++mockEditorIdCounter),
    editorDestroy: jest.fn(),
    editorSetHtml: jest.fn(() => '[]'),
    editorGetHtml: jest.fn(() => '<p>test content</p>'),
    editorSetJson: jest.fn(() => '[]'),
    editorGetJson: jest.fn(() => MOCK_DOCUMENT_JSON_STR),
    editorInsertText: jest.fn(() => MOCK_INSERT_UPDATE_JSON),
    editorReplaceSelectionText: jest.fn(() => MOCK_INSERT_UPDATE_JSON),
    editorDeleteRange: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorToggleMark: jest.fn(() => MOCK_BOLD_UPDATE_JSON),
    editorSetMark: jest.fn(() => MOCK_BOLD_UPDATE_JSON),
    editorUnsetMark: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorSetSelection: jest.fn(),
    editorGetSelection: jest.fn(() => JSON.stringify({ type: 'text', anchor: 5, head: 5 })),
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
    editorWrapInListAtSelectionScalar: jest.fn(() => MOCK_LIST_UPDATE_JSON),
    editorUnwrapFromListAtSelectionScalar: jest.fn(() => MOCK_EMPTY_UPDATE_JSON),
    editorIndentListItemAtSelectionScalar: jest.fn(() => MOCK_LIST_UPDATE_JSON),
    editorOutdentListItemAtSelectionScalar: jest.fn(() => MOCK_LIST_UPDATE_JSON),
    editorInsertNodeAtSelectionScalar: jest.fn(() => MOCK_NODE_UPDATE_JSON),
    editorDocToScalar: jest.fn((_: number, pos: number) => pos),
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
                applyEditorUpdate: mockApplyEditorUpdate,
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
import { render, act } from '@testing-library/react-native';
import { PixelRatio, Platform } from 'react-native';

import { NativeRichTextEditor, type NativeRichTextEditorRef } from '../NativeRichTextEditor';
import { _resetNativeModuleCache } from '../NativeEditorBridge';

// ─── Tests ──────────────────────────────────────────────────────

describe('NativeRichTextEditor', () => {
    beforeEach(() => {
        _resetNativeModuleCache();
        mockEditorIdCounter = 0;
        mockApplyEditorUpdate.mockClear();
        mockNativeFocus.mockClear();
        mockNativeBlur.mockClear();

        // Reset all mocks to defaults (mockClear only clears call history,
        // mockReset also clears return values and implementations)
        for (const key of Object.keys(mockNativeModule)) {
            (mockNativeModule as Record<string, jest.Mock>)[key].mockReset();
        }

        // Re-establish default return values
        mockNativeModule.editorCreate.mockImplementation(
            (_configJson: string) => ++mockEditorIdCounter
        );
        mockNativeModule.editorSetHtml.mockReturnValue('[]');
        mockNativeModule.editorGetHtml.mockReturnValue('<p>test content</p>');
        mockNativeModule.editorSetJson.mockReturnValue('[]');
        mockNativeModule.editorGetJson.mockReturnValue(MOCK_DOCUMENT_JSON_STR);
        mockNativeModule.editorInsertText.mockReturnValue(MOCK_INSERT_UPDATE_JSON);
        mockNativeModule.editorReplaceSelectionText.mockReturnValue(MOCK_INSERT_UPDATE_JSON);
        mockNativeModule.editorDeleteRange.mockReturnValue(MOCK_EMPTY_UPDATE_JSON);
        mockNativeModule.editorToggleMark.mockReturnValue(MOCK_BOLD_UPDATE_JSON);
        mockNativeModule.editorToggleMarkAtSelectionScalar.mockReturnValue(MOCK_BOLD_UPDATE_JSON);
        mockNativeModule.editorGetSelection.mockReturnValue(
            JSON.stringify({ type: 'text', anchor: 5, head: 5 })
        );
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
                JSON.stringify({ maxLength: 200 })
            );
        });

        it('sets initial content via setHtml when initialContent is provided', () => {
            render(<NativeRichTextEditor initialContent='<p>hello</p>' />);
            expect(mockNativeModule.editorSetHtml).toHaveBeenCalledWith(1, '<p>hello</p>');
        });

        it('sets initialJSON via setJson when provided', () => {
            const doc = { type: 'doc', content: [] };
            render(<NativeRichTextEditor initialJSON={doc} />);
            expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(1, JSON.stringify(doc));
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

        it('passes toolbarPlacement to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor toolbarPlacement='inline' />);
            expect(getByTestId('native-editor-view').props.toolbarPlacement).toBe('inline');
        });

        it('passes heightBehavior to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor heightBehavior='autoGrow' />);
            expect(getByTestId('native-editor-view').props.heightBehavior).toBe('autoGrow');
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
                                label: '@Alice',
                                attrs: {
                                    label: '@Alice',
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
                                label: '@Alice',
                                attrs: {
                                    label: '@Alice',
                                },
                            },
                        ],
                    },
                })
            );
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

        it('toggleMark at a collapsed cursor skips native reapply when HTML is unchanged', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            mockNativeModule.editorToggleMarkAtSelectionScalar.mockReturnValue(
                MOCK_COLLAPSED_BOLD_UPDATE_JSON
            );
            mockNativeModule.editorGetHtml
                .mockReturnValueOnce('<p>abc</p>')
                .mockReturnValueOnce('<p>abc</p>');
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
            expect(mockNativeModule.editorSetSelection).toHaveBeenCalledWith(1, 0, 0);
            expect(mockNativeModule.editorSetMarkAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0,
                'link',
                JSON.stringify({ href: 'https://example.com' })
            );
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

            expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledWith(1, JSON.stringify(doc));
            expect(mockApplyEditorUpdate).toHaveBeenCalled();
        });

        it('getContent returns bridge.getHtml()', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            const content = ref.current!.getContent();

            expect(mockNativeModule.editorGetHtml).toHaveBeenCalled();
            expect(content).toBe('<p>test content</p>');
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

        it('getTextContent strips HTML tags', () => {
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} />);

            const text = ref.current!.getTextContent();

            expect(text).toBe('test content');
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

        it('calls setJson when valueJSON prop is provided', () => {
            const doc = { type: 'doc', content: [] };
            render(<NativeRichTextEditor valueJSON={doc} />);

            expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(1, JSON.stringify(doc));
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

        it('preserves the live selection when valueJSON changes externally', () => {
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
                    nativeEvent: { anchor: 5, head: 5 },
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
            expect(mockNativeModule.editorSetSelection).toHaveBeenCalledWith(1, 5, 5);
            expect(mockApplyEditorUpdate).toHaveBeenCalledTimes(1);
            expect(JSON.parse(mockApplyEditorUpdate.mock.calls[0][0]).selection).toEqual({
                type: 'text',
                anchor: 5,
                head: 5,
            });
            expect(onSelectionChange).toHaveBeenCalledWith({
                type: 'text',
                anchor: 5,
                head: 5,
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

        it('onContentChangeJSON fires with JSON from bridge', () => {
            const onContentChangeJSON = jest.fn();
            const ref = createRef<NativeRichTextEditorRef>();
            render(<NativeRichTextEditor ref={ref} onContentChangeJSON={onContentChangeJSON} />);

            act(() => {
                ref.current!.toggleMark('bold');
            });

            expect(onContentChangeJSON).toHaveBeenCalledWith(JSON.parse(MOCK_DOCUMENT_JSON_STR));
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

        it('refreshes activeState on native selection changes', () => {
            const onActiveStateChange = jest.fn();
            const onSelectionChange = jest.fn();
            mockNativeModule.editorGetCurrentState
                .mockReturnValueOnce(
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
                )
                .mockReturnValueOnce(
                    JSON.stringify({
                        renderElements: [],
                        selection: { type: 'text', anchor: 5, head: 5 },
                        activeState: {
                            marks: {},
                            nodes: { bulletList: true, listItem: true },
                            commands: { indentList: false, outdentList: true },
                            allowedMarks: ['bold'],
                            insertableNodes: [],
                        },
                        historyState: { canUndo: false, canRedo: false },
                    })
                );

            const { getByTestId } = render(
                <NativeRichTextEditor
                    onActiveStateChange={onActiveStateChange}
                    onSelectionChange={onSelectionChange}
                />
            );

            act(() => {
                getByTestId('native-editor-view').props.onSelectionChange({
                    nativeEvent: { anchor: 5, head: 5 },
                });
            });

            expect(mockNativeModule.editorGetCurrentState).toHaveBeenCalledTimes(2);
            expect(onActiveStateChange).toHaveBeenCalledWith({
                marks: {},
                markAttrs: {},
                nodes: { bulletList: true, listItem: true },
                commands: { indentList: false, outdentList: true },
                allowedMarks: ['bold'],
                insertableNodes: [],
            });
            expect(onSelectionChange).toHaveBeenCalledWith({
                type: 'text',
                anchor: 5,
                head: 5,
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

        it('passes onFocusChange handler to native view', () => {
            const { getByTestId } = render(<NativeRichTextEditor onFocus={jest.fn()} />);
            const view = getByTestId('native-editor-view');
            expect(typeof view.props.onFocusChange).toBe('function');
        });

        it('renders the JS toolbar inline when toolbarPlacement is inline', () => {
            const { getByTestId } = render(<NativeRichTextEditor toolbarPlacement='inline' />);

            expect(getByTestId('native-editor-js-toolbar')).toBeTruthy();
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
                        nativeEvent: { contentHeight: 240 },
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
            const { getByTestId } = render(
                <NativeRichTextEditor onActiveStateChange={onActiveStateChange} />
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
                            attrs: { id: 'u1', kind: 'user', label: '@Alice' },
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
                attrs: { id: 'u1', kind: 'user', label: '@Alice' },
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
            capturedRef.insertContentHtml('<p>x</p>');
            capturedRef.insertContentJson({ type: 'doc' });

            // No native module calls after unmount
            expect(mockNativeModule.editorToggleMarkAtSelectionScalar).not.toHaveBeenCalled();
            expect(mockNativeModule.editorWrapInListAtSelectionScalar).not.toHaveBeenCalled();
            expect(mockNativeModule.editorInsertNodeAtSelectionScalar).not.toHaveBeenCalled();
            expect(mockNativeModule.editorInsertText).not.toHaveBeenCalled();
            expect(mockNativeModule.editorReplaceSelectionText).not.toHaveBeenCalled();
            expect(mockNativeModule.editorUndo).not.toHaveBeenCalled();
            expect(mockNativeModule.editorRedo).not.toHaveBeenCalled();
        });
    });
});
