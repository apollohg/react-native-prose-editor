// ─── NativeEditorBridge Tests ──────────────────────────────────
// Unit tests for the TypeScript bridge module. The native module
// is mocked since it requires a native runtime (iOS/Android).
//
// Tests cover:
// - Bridge creation and destruction lifecycle
// - All editing operations returning parsed types
// - Content loading (HTML, JSON) and retrieval
// - Selection management
// - Undo/redo with null returns
// - Error handling for invalid JSON responses
// - Dispose pattern (methods throw after destroy)
// ────────────────────────────────────────────────────────────────

// ─── Mock Data ──────────────────────────────────────────────────

const MOCK_RENDER_ELEMENTS_JSON = JSON.stringify([
    { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
    { type: 'textRun', text: 'hello', marks: [] },
    { type: 'blockEnd' },
]);

const MOCK_UPDATE_JSON = JSON.stringify({
    renderElements: [
        { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
        { type: 'textRun', text: 'hello world', marks: [] },
        { type: 'blockEnd' },
    ],
    selection: { type: 'text', anchor: 11, head: 11 },
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

const MOCK_TOGGLE_BOLD_UPDATE_JSON = JSON.stringify({
    renderElements: [
        { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
        { type: 'textRun', text: 'bold text', marks: ['bold'] },
        { type: 'blockEnd' },
    ],
    selection: { type: 'text', anchor: 0, head: 9 },
    activeState: {
        marks: { bold: true },
        markAttrs: {},
        nodes: { paragraph: true },
        commands: {},
        allowedMarks: [],
        insertableNodes: [],
    },
    historyState: { canUndo: true, canRedo: false },
    documentVersion: 3,
});

const MOCK_ORDERED_LIST_UPDATE_JSON = JSON.stringify({
    renderElements: [
        { type: 'blockStart', nodeType: 'orderedList', depth: 0 },
        { type: 'blockStart', nodeType: 'listItem', depth: 1 },
        { type: 'blockStart', nodeType: 'paragraph', depth: 2 },
        { type: 'textRun', text: 'hello', marks: [] },
        { type: 'blockEnd' },
        { type: 'blockEnd' },
        { type: 'blockEnd' },
    ],
    selection: { type: 'text', anchor: 3, head: 3 },
    activeState: {
        marks: {},
        markAttrs: {},
        nodes: { orderedList: true, listItem: true, paragraph: true },
        commands: { indentList: true, outdentList: false },
        allowedMarks: [],
        insertableNodes: [],
    },
    historyState: { canUndo: true, canRedo: false },
    documentVersion: 4,
});

const MOCK_DOCUMENT_JSON = JSON.stringify({
    type: 'doc',
    content: [
        {
            type: 'paragraph',
            content: [{ type: 'text', text: 'hello world' }],
        },
    ],
});

const MOCK_COLLABORATION_RESULT_JSON = JSON.stringify({
    messages: [[1, 2, 3]],
    documentChanged: true,
    documentJson: {
        type: 'doc',
        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'synced' }] }],
    },
    peersChanged: true,
    peers: [{ clientId: 1, isLocal: true, state: { user: { name: 'Alice' } } }],
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

const MOCK_ENCODED_COLLABORATION_STATE_JSON = JSON.stringify([1, 2, 3, 4]);
const MOCK_COLLABORATION_SCHEMA = {
    nodes: [
        { name: 'doc', content: 'block+', role: 'doc' },
        { name: 'paragraph', content: 'inline*', group: 'block', role: 'textBlock' },
        {
            name: 'calloutDivider',
            content: '',
            group: 'block',
            role: 'block',
            isVoid: true,
        },
        { name: 'text', content: '', group: 'inline', role: 'text' },
    ],
    marks: [],
};

// ─── Mock Native Module ─────────────────────────────────────────
// jest.mock is hoisted above imports. We define mockNativeModule as
// a mutable object populated in beforeEach so it's always available
// when requireNativeModule is called at module scope.

let mockEditorIdCounter = 0;

// This object is shared between the mock factory and the test code.
// The factory returns it from requireNativeModule at import time,
// and we populate the methods in beforeEach.
const mockNativeModule: Record<string, jest.Mock> = {};

function resetMockNativeModule() {
    mockEditorIdCounter = 0;
    mockNativeModule.editorCreate = jest.fn((_configJson: string) => ++mockEditorIdCounter);
    mockNativeModule.editorDestroy = jest.fn();
    mockNativeModule.collaborationSessionCreate = jest.fn(
        (_configJson: string) => ++mockEditorIdCounter
    );
    mockNativeModule.collaborationSessionDestroy = jest.fn();
    mockNativeModule.collaborationSessionGetDocumentJson = jest.fn(() => MOCK_DOCUMENT_JSON);
    mockNativeModule.collaborationSessionGetEncodedState = jest.fn(
        () => MOCK_ENCODED_COLLABORATION_STATE_JSON
    );
    mockNativeModule.collaborationSessionGetPeersJson = jest.fn(
        () => '[{"clientId":1,"isLocal":true,"state":{"user":{"name":"Alice"}}}]'
    );
    mockNativeModule.collaborationSessionStart = jest.fn(() => MOCK_COLLABORATION_RESULT_JSON);
    mockNativeModule.collaborationSessionApplyLocalDocumentJson = jest.fn(
        () => MOCK_COLLABORATION_RESULT_JSON
    );
    mockNativeModule.collaborationSessionApplyEncodedState = jest.fn(
        () => MOCK_COLLABORATION_RESULT_JSON
    );
    mockNativeModule.collaborationSessionReplaceEncodedState = jest.fn(
        () => MOCK_COLLABORATION_RESULT_JSON
    );
    mockNativeModule.collaborationSessionHandleMessage = jest.fn(
        () => MOCK_COLLABORATION_RESULT_JSON
    );
    mockNativeModule.collaborationSessionSetLocalAwareness = jest.fn(
        () => MOCK_COLLABORATION_RESULT_JSON
    );
    mockNativeModule.collaborationSessionClearLocalAwareness = jest.fn(
        () => MOCK_COLLABORATION_RESULT_JSON
    );
    mockNativeModule.editorSetHtml = jest.fn(() => MOCK_RENDER_ELEMENTS_JSON);
    mockNativeModule.editorGetHtml = jest.fn(() => '<p>hello world</p>');
    mockNativeModule.editorSetJson = jest.fn(() => MOCK_RENDER_ELEMENTS_JSON);
    mockNativeModule.editorGetJson = jest.fn(() => MOCK_DOCUMENT_JSON);
    mockNativeModule.editorGetContentSnapshot = jest.fn(() =>
        JSON.stringify({
            html: '<p>hello world</p>',
            json: JSON.parse(MOCK_DOCUMENT_JSON),
        })
    );
    mockNativeModule.editorInsertText = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorReplaceSelectionText = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorDeleteRange = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorToggleMark = jest.fn(() => MOCK_TOGGLE_BOLD_UPDATE_JSON);
    mockNativeModule.editorSetMark = jest.fn(() => MOCK_TOGGLE_BOLD_UPDATE_JSON);
    mockNativeModule.editorUnsetMark = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorToggleBlockquote = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorToggleHeading = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorSetSelection = jest.fn();
    mockNativeModule.editorGetSelection = jest.fn(() =>
        JSON.stringify({ type: 'text', anchor: 0, head: 0 })
    );
    mockNativeModule.editorGetSelectionState = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorGetCurrentState = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorSplitBlock = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorInsertContentHtml = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorInsertContentJson = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorInsertContentJsonAtSelectionScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorReplaceHtml = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorReplaceJson = jest.fn(() => MOCK_UPDATE_JSON);
    // Scalar-position APIs
    mockNativeModule.editorInsertTextScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorDeleteScalarRange = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorReplaceTextScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorSplitBlockScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorDeleteAndSplitScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorSetSelectionScalar = jest.fn();
    mockNativeModule.editorToggleMarkAtSelectionScalar = jest.fn(
        () => MOCK_TOGGLE_BOLD_UPDATE_JSON
    );
    mockNativeModule.editorSetMarkAtSelectionScalar = jest.fn(() => MOCK_TOGGLE_BOLD_UPDATE_JSON);
    mockNativeModule.editorUnsetMarkAtSelectionScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorToggleBlockquoteAtSelectionScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorToggleHeadingAtSelectionScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorWrapInListAtSelectionScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorUnwrapFromListAtSelectionScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorIndentListItemAtSelectionScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorOutdentListItemAtSelectionScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorInsertNodeAtSelectionScalar = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorDocToScalar = jest.fn((_: number, pos: number) => pos);
    // List / node APIs
    mockNativeModule.editorWrapInList = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorUnwrapFromList = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorIndentListItem = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorOutdentListItem = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorInsertNode = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorUndo = jest.fn(() => MOCK_UPDATE_JSON);
    mockNativeModule.editorRedo = jest.fn(() => '');
    mockNativeModule.editorCanUndo = jest.fn(() => true);
    mockNativeModule.editorCanRedo = jest.fn(() => false);
}

// Initialize immediately so the module-scope requireNativeModule call works.
resetMockNativeModule();

jest.mock('expo-modules-core', () => ({
    requireNativeModule: () => mockNativeModule,
}));

// ─── Imports ────────────────────────────────────────────────────

import {
    NativeCollaborationBridge,
    NativeEditorBridge,
    _resetNativeModuleCache,
} from '../NativeEditorBridge';

// ─── Tests ──────────────────────────────────────────────────────

describe('NativeEditorBridge', () => {
    beforeEach(() => {
        _resetNativeModuleCache();
        resetMockNativeModule();
    });

    // ── Creation & Destruction ──────────────────────────────────

    describe('create', () => {
        it('creates a bridge with empty config (no options)', () => {
            const bridge = NativeEditorBridge.create();

            expect(mockNativeModule.editorCreate).toHaveBeenCalledTimes(1);
            expect(mockNativeModule.editorCreate).toHaveBeenCalledWith('{}');
            expect(bridge.editorId).toBe(1);
            expect(bridge.isDestroyed).toBe(false);

            bridge.destroy();
        });

        it('creates a bridge with maxLength in config', () => {
            const bridge = NativeEditorBridge.create({ maxLength: 500 });

            expect(mockNativeModule.editorCreate).toHaveBeenCalledWith(
                JSON.stringify({ maxLength: 500 })
            );
            expect(bridge.editorId).toBe(1);

            bridge.destroy();
        });

        it('creates a bridge with schemaJson in config', () => {
            const schemaJson = JSON.stringify({ nodes: { doc: {} } });
            const bridge = NativeEditorBridge.create({ schemaJson });

            expect(mockNativeModule.editorCreate).toHaveBeenCalledWith(
                JSON.stringify({ schema: { nodes: { doc: {} } } })
            );
            expect(bridge.editorId).toBe(1);

            bridge.destroy();
        });

        it('creates a bridge with both maxLength and schemaJson', () => {
            const schemaJson = JSON.stringify({ nodes: { doc: {} } });
            const bridge = NativeEditorBridge.create({ maxLength: 200, schemaJson });

            expect(mockNativeModule.editorCreate).toHaveBeenCalledWith(
                JSON.stringify({ maxLength: 200, schema: { nodes: { doc: {} } } })
            );
            expect(bridge.editorId).toBe(1);

            bridge.destroy();
        });

        it('creates a bridge with allowBase64Images in config', () => {
            const bridge = NativeEditorBridge.create({ allowBase64Images: true });

            expect(mockNativeModule.editorCreate).toHaveBeenCalledWith(
                JSON.stringify({ allowBase64Images: true })
            );
            expect(bridge.editorId).toBe(1);

            bridge.destroy();
        });

        it('falls back to default when schemaJson is invalid JSON', () => {
            const bridge = NativeEditorBridge.create({ schemaJson: 'not valid json' });

            // Should still create with empty config (schema parse failed)
            expect(mockNativeModule.editorCreate).toHaveBeenCalledWith('{}');
            expect(bridge.editorId).toBe(1);

            bridge.destroy();
        });

        it('assigns unique editor IDs to each instance', () => {
            const bridge1 = NativeEditorBridge.create();
            const bridge2 = NativeEditorBridge.create();

            expect(bridge1.editorId).toBe(1);
            expect(bridge2.editorId).toBe(2);

            bridge1.destroy();
            bridge2.destroy();
        });
    });

    describe('destroy', () => {
        it('calls native editorDestroy with the correct ID', () => {
            const bridge = NativeEditorBridge.create();
            const editorId = bridge.editorId;

            bridge.destroy();

            expect(mockNativeModule.editorDestroy).toHaveBeenCalledWith(editorId);
            expect(bridge.isDestroyed).toBe(true);
        });

        it('is idempotent — calling destroy twice does not call native twice', () => {
            const bridge = NativeEditorBridge.create();

            bridge.destroy();
            bridge.destroy();

            expect(mockNativeModule.editorDestroy).toHaveBeenCalledTimes(1);
        });
    });

    describe('NativeCollaborationBridge', () => {
        it('creates a collaboration session and starts sync', () => {
            const bridge = NativeCollaborationBridge.create({
                clientId: 7,
                fragmentName: 'prosemirror',
                initialEncodedState: Uint8Array.from([4, 5, 6]),
            });

            expect(mockNativeModule.collaborationSessionCreate).toHaveBeenCalledWith(
                JSON.stringify({ clientId: 7, fragmentName: 'prosemirror' })
            );
            expect(mockNativeModule.collaborationSessionReplaceEncodedState).toHaveBeenCalledWith(
                1,
                JSON.stringify([4, 5, 6])
            );

            expect(bridge.getDocumentJson()).toEqual(JSON.parse(MOCK_DOCUMENT_JSON));
            expect(bridge.getEncodedState()).toEqual(Uint8Array.from([1, 2, 3, 4]));
            expect(bridge.getEncodedStateBase64()).toBe('AQIDBA==');
            expect(bridge.getPeers()).toEqual([
                {
                    clientId: 1,
                    isLocal: true,
                    state: { user: { name: 'Alice' } },
                },
            ]);
            expect(bridge.start()).toEqual(JSON.parse(MOCK_COLLABORATION_RESULT_JSON));

            bridge.destroy();
            expect(mockNativeModule.collaborationSessionDestroy).toHaveBeenCalledWith(1);
        });

        it('forwards schema metadata when creating a collaboration session', () => {
            const bridge = NativeCollaborationBridge.create({
                fragmentName: 'default',
                schema: MOCK_COLLABORATION_SCHEMA,
            });

            expect(mockNativeModule.collaborationSessionCreate).toHaveBeenCalledWith(
                JSON.stringify({
                    fragmentName: 'default',
                    schema: MOCK_COLLABORATION_SCHEMA,
                })
            );

            bridge.destroy();
        });

        it('destroys the session if initial encoded state restoration fails', () => {
            mockNativeModule.collaborationSessionReplaceEncodedState.mockReturnValueOnce(
                JSON.stringify({ error: 'invalid encoded state' })
            );

            expect(() =>
                NativeCollaborationBridge.create({
                    initialEncodedState: Uint8Array.from([4, 5, 6]),
                })
            ).toThrow('NativeEditorBridge: invalid encoded state');
            expect(mockNativeModule.collaborationSessionDestroy).toHaveBeenCalledWith(1);
        });

        it('routes local document and awareness updates through the native module', () => {
            const bridge = NativeCollaborationBridge.create();
            const nextDoc = {
                type: 'doc',
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'next' }] }],
            };

            bridge.applyLocalDocumentJson(nextDoc);
            bridge.applyEncodedState('BgUE');
            bridge.replaceEncodedState('AwIB');
            bridge.handleMessage([9, 8, 7]);
            bridge.setLocalAwareness({ user: { name: 'Alice' }, focused: true });
            bridge.clearLocalAwareness();

            expect(
                mockNativeModule.collaborationSessionApplyLocalDocumentJson
            ).toHaveBeenCalledWith(1, JSON.stringify(nextDoc));
            expect(mockNativeModule.collaborationSessionApplyEncodedState).toHaveBeenCalledWith(
                1,
                JSON.stringify([6, 5, 4])
            );
            expect(mockNativeModule.collaborationSessionReplaceEncodedState).toHaveBeenCalledWith(
                1,
                JSON.stringify([3, 2, 1])
            );
            expect(mockNativeModule.collaborationSessionHandleMessage).toHaveBeenCalledWith(
                1,
                JSON.stringify([9, 8, 7])
            );
            expect(mockNativeModule.collaborationSessionSetLocalAwareness).toHaveBeenCalledWith(
                1,
                JSON.stringify({ user: { name: 'Alice' }, focused: true })
            );
            expect(mockNativeModule.collaborationSessionClearLocalAwareness).toHaveBeenCalledWith(
                1
            );
        });
    });

    // ── Content Loading ─────────────────────────────────────────

    describe('setHtml', () => {
        it('parses render elements from native response', () => {
            const bridge = NativeEditorBridge.create();

            const elements = bridge.setHtml('<p>hello</p>');

            expect(mockNativeModule.editorSetHtml).toHaveBeenCalledWith(
                bridge.editorId,
                '<p>hello</p>'
            );
            expect(elements).toHaveLength(3);
            expect(elements[0]).toEqual({
                type: 'blockStart',
                nodeType: 'paragraph',
                depth: 0,
            });
            expect(elements[1]).toEqual({
                type: 'textRun',
                text: 'hello',
                marks: [],
            });
            expect(elements[2]).toEqual({ type: 'blockEnd' });

            bridge.destroy();
        });

        it('returns empty array for empty response', () => {
            mockNativeModule.editorSetHtml.mockReturnValueOnce('[]');

            const bridge = NativeEditorBridge.create();
            const elements = bridge.setHtml('');

            expect(elements).toEqual([]);
            bridge.destroy();
        });

        it('preserves attrs on void render elements returned by native', () => {
            mockNativeModule.editorSetHtml.mockReturnValueOnce(
                JSON.stringify([
                    {
                        type: 'voidBlock',
                        nodeType: 'image',
                        docPos: 0,
                        attrs: { src: 'https://example.com/cat.png', alt: 'Cat' },
                    },
                ])
            );

            const bridge = NativeEditorBridge.create();
            const elements = bridge.setHtml('<img src="https://example.com/cat.png" alt="Cat">');

            expect(elements).toEqual([
                {
                    type: 'voidBlock',
                    nodeType: 'image',
                    docPos: 0,
                    attrs: { src: 'https://example.com/cat.png', alt: 'Cat' },
                },
            ]);

            bridge.destroy();
        });
    });

    describe('getHtml', () => {
        it('returns HTML string from native module', () => {
            const bridge = NativeEditorBridge.create();

            const html = bridge.getHtml();

            expect(mockNativeModule.editorGetHtml).toHaveBeenCalledWith(bridge.editorId);
            expect(html).toBe('<p>hello world</p>');

            bridge.destroy();
        });
    });

    describe('setJson', () => {
        it('serializes document JSON and parses render elements', () => {
            const bridge = NativeEditorBridge.create();
            const doc = { type: 'doc', content: [] };

            const elements = bridge.setJson(doc);

            expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(
                bridge.editorId,
                JSON.stringify(NORMALIZED_EMPTY_DOC)
            );
            expect(elements).toHaveLength(3);

            bridge.destroy();
        });

        it('normalizes empty root docs against the configured schema', () => {
            const bridge = NativeEditorBridge.create({
                schemaJson: JSON.stringify(TITLE_FIRST_SCHEMA),
            });

            bridge.setJsonString(JSON.stringify({ type: 'doc', content: [] }));

            expect(mockNativeModule.editorSetJson).toHaveBeenCalledWith(
                bridge.editorId,
                JSON.stringify(TITLE_EMPTY_DOC)
            );

            bridge.destroy();
        });
    });

    describe('getJson', () => {
        it('parses document JSON from native module', () => {
            const bridge = NativeEditorBridge.create();

            const doc = bridge.getJson();

            expect(mockNativeModule.editorGetJson).toHaveBeenCalledWith(bridge.editorId);
            expect(doc).toEqual({
                type: 'doc',
                content: [
                    {
                        type: 'paragraph',
                        content: [{ type: 'text', text: 'hello world' }],
                    },
                ],
            });

            bridge.destroy();
        });
    });

    describe('getContentSnapshot', () => {
        it('returns html and json in one native roundtrip', () => {
            const bridge = NativeEditorBridge.create();

            const snapshot = bridge.getContentSnapshot();

            expect(mockNativeModule.editorGetContentSnapshot).toHaveBeenCalledWith(bridge.editorId);
            expect(snapshot).toEqual({
                html: '<p>hello world</p>',
                json: {
                    type: 'doc',
                    content: [
                        {
                            type: 'paragraph',
                            content: [{ type: 'text', text: 'hello world' }],
                        },
                    ],
                },
            });

            bridge.destroy();
        });
    });

    // ── Editing Operations ──────────────────────────────────────

    describe('insertText', () => {
        it('returns parsed EditorUpdate with render elements and selection', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.insertText(0, 'hello world');

            expect(mockNativeModule.editorInsertText).toHaveBeenCalledWith(
                bridge.editorId,
                0,
                'hello world'
            );
            expect(update).not.toBeNull();
            expect(update!.renderElements).toHaveLength(3);
            expect(update!.selection).toEqual({
                type: 'text',
                anchor: 11,
                head: 11,
            });
            expect(update!.activeState).toEqual({
                marks: {},
                markAttrs: {},
                nodes: { paragraph: true },
                commands: {},
                allowedMarks: [],
                insertableNodes: [],
            });
            expect(update!.historyState).toEqual({
                canUndo: true,
                canRedo: false,
            });

            bridge.destroy();
        });

        it('reconstructs renderElements from renderBlocks when flattened elements are omitted', () => {
            const blocksOnlyUpdate = JSON.stringify({
                renderBlocks: [
                    [
                        { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                        { type: 'textRun', text: 'hello world', marks: [] },
                        { type: 'blockEnd' },
                    ],
                ],
                selection: { type: 'text', anchor: 11, head: 11 },
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
            mockNativeModule.editorInsertText.mockReturnValueOnce(blocksOnlyUpdate);

            const bridge = NativeEditorBridge.create();
            const update = bridge.insertText(0, 'hello world');

            expect(update).not.toBeNull();
            expect(update!.renderElements).toHaveLength(3);
            expect(update!.renderElements[1]).toEqual({
                type: 'textRun',
                text: 'hello world',
                marks: [],
            });
            expect(update!.renderBlocks).toHaveLength(1);

            bridge.destroy();
        });

        it('reconstructs patch-only updates from the cached render blocks', () => {
            const initialState = JSON.stringify({
                renderBlocks: [
                    [
                        { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                        { type: 'textRun', text: 'hello', marks: [] },
                        { type: 'blockEnd' },
                    ],
                ],
                selection: { type: 'text', anchor: 5, head: 5 },
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
            });
            const patchOnlyUpdate = JSON.stringify({
                renderPatch: {
                    startIndex: 0,
                    deleteCount: 1,
                    renderBlocks: [
                        [
                            { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                            { type: 'textRun', text: 'hello world', marks: [] },
                            { type: 'blockEnd' },
                        ],
                    ],
                },
                selection: { type: 'text', anchor: 11, head: 11 },
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
            mockNativeModule.editorGetCurrentState.mockReturnValueOnce(initialState);
            mockNativeModule.editorInsertText.mockReturnValueOnce(patchOnlyUpdate);

            const bridge = NativeEditorBridge.create();
            bridge.getCurrentState();
            const update = bridge.insertText(5, ' world');

            expect(update).not.toBeNull();
            expect(update!.renderBlocks).toHaveLength(1);
            expect(update!.renderElements).toHaveLength(3);
            expect(update!.renderElements[1]).toEqual({
                type: 'textRun',
                text: 'hello world',
                marks: [],
            });

            bridge.destroy();
        });
    });

    describe('deleteRange', () => {
        it('returns parsed EditorUpdate', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.deleteRange(0, 5);

            expect(mockNativeModule.editorDeleteRange).toHaveBeenCalledWith(bridge.editorId, 0, 5);
            expect(update).not.toBeNull();
            expect(update!.renderElements).toHaveLength(3);

            bridge.destroy();
        });
    });

    describe('replaceSelectionText', () => {
        it('returns parsed EditorUpdate for atomic selection replacement', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.replaceSelectionText('hello world');

            expect(mockNativeModule.editorReplaceSelectionText).toHaveBeenCalledWith(
                bridge.editorId,
                'hello world'
            );
            expect(update).not.toBeNull();
            expect(update!.selection).toEqual({
                type: 'text',
                anchor: 11,
                head: 11,
            });

            bridge.destroy();
        });
    });

    describe('insertContentJson', () => {
        it('returns parsed EditorUpdate for JSON fragment insertion', () => {
            const bridge = NativeEditorBridge.create();
            const doc = {
                type: 'doc',
                content: [{ type: 'mention', attrs: { id: 'u1', label: '@Alice' } }],
            };

            const update = bridge.insertContentJson(doc);

            expect(mockNativeModule.editorInsertContentJson).toHaveBeenCalledWith(
                bridge.editorId,
                JSON.stringify(doc)
            );
            expect(update).not.toBeNull();
            expect(update!.selection).toEqual({
                type: 'text',
                anchor: 11,
                head: 11,
            });

            bridge.destroy();
        });

        it('forwards explicit scalar selections for mention insertions', () => {
            const bridge = NativeEditorBridge.create();
            const doc = {
                type: 'doc',
                content: [{ type: 'mention', attrs: { id: 'u1', label: '@Alice' } }],
            };

            const update = bridge.insertContentJsonAtSelectionScalar(4, 7, doc);

            expect(mockNativeModule.editorInsertContentJsonAtSelectionScalar).toHaveBeenCalledWith(
                bridge.editorId,
                4,
                7,
                JSON.stringify(doc)
            );
            expect(update).not.toBeNull();
            expect(update!.selection).toEqual({
                type: 'text',
                anchor: 11,
                head: 11,
            });

            bridge.destroy();
        });
    });

    describe('replaceJson', () => {
        it('normalizes empty root docs before replacing content', () => {
            const bridge = NativeEditorBridge.create();

            bridge.replaceJson({ type: 'doc', content: [] });

            expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledWith(
                bridge.editorId,
                JSON.stringify(NORMALIZED_EMPTY_DOC)
            );

            bridge.destroy();
        });

        it('uses the configured schema when normalizing string replacements', () => {
            const bridge = NativeEditorBridge.create({
                schemaJson: JSON.stringify(TITLE_FIRST_SCHEMA),
            });

            bridge.replaceJsonString(JSON.stringify({ type: 'doc', content: [] }));

            expect(mockNativeModule.editorReplaceJson).toHaveBeenCalledWith(
                bridge.editorId,
                JSON.stringify(TITLE_EMPTY_DOC)
            );

            bridge.destroy();
        });
    });

    describe('toggleMark', () => {
        it('returns update with active marks', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.toggleMark('bold');

            expect(mockNativeModule.editorToggleMarkAtSelectionScalar).toHaveBeenCalledWith(
                bridge.editorId,
                0,
                0,
                'bold'
            );
            expect(update).not.toBeNull();
            expect(update!.activeState.marks).toHaveProperty('bold', true);
            expect(update!.renderElements[1]).toEqual({
                type: 'textRun',
                text: 'bold text',
                marks: ['bold'],
            });

            bridge.destroy();
        });

        it('sets an attributed link mark on the current selection', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.setMark('link', { href: 'https://example.com' });

            expect(mockNativeModule.editorSetMarkAtSelectionScalar).toHaveBeenCalledWith(
                bridge.editorId,
                0,
                0,
                'link',
                JSON.stringify({ href: 'https://example.com' })
            );
            expect(update).not.toBeNull();

            bridge.destroy();
        });

        it('removes an attributed link mark from the current selection', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.unsetMark('link');

            expect(mockNativeModule.editorUnsetMarkAtSelectionScalar).toHaveBeenCalledWith(
                bridge.editorId,
                0,
                0,
                'link'
            );
            expect(update).not.toBeNull();

            bridge.destroy();
        });
    });

    // ── Selection ───────────────────────────────────────────────

    describe('setSelection', () => {
        it('calls native setSelection with anchor and head', () => {
            const bridge = NativeEditorBridge.create();

            bridge.setSelection(3, 7);

            expect(mockNativeModule.editorSetSelection).toHaveBeenCalledWith(bridge.editorId, 3, 7);

            bridge.destroy();
        });
    });

    describe('getSelectionState', () => {
        it('returns parsed selection-related state without requiring render elements', () => {
            mockNativeModule.editorGetSelectionState.mockReturnValueOnce(
                JSON.stringify({
                    selection: { type: 'text', anchor: 4, head: 4 },
                    activeState: {
                        marks: { bold: true },
                        nodes: { paragraph: true },
                        commands: {},
                        allowedMarks: ['bold'],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: true, canRedo: false },
                })
            );
            const bridge = NativeEditorBridge.create();

            const update = bridge.getSelectionState();

            expect(mockNativeModule.editorGetSelectionState).toHaveBeenCalledWith(bridge.editorId);
            expect(update).toEqual({
                renderElements: [],
                selection: { type: 'text', anchor: 4, head: 4 },
                activeState: {
                    marks: { bold: true },
                    markAttrs: {},
                    nodes: { paragraph: true },
                    commands: {},
                    allowedMarks: ['bold'],
                    insertableNodes: [],
                },
                historyState: { canUndo: true, canRedo: false },
            });

            bridge.destroy();
        });
    });

    // ── List / Node Operations ──────────────────────────────────

    describe('toggleList', () => {
        it('returns parsed EditorUpdate', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.toggleList('bulletList');

            expect(mockNativeModule.editorWrapInListAtSelectionScalar).toHaveBeenCalledWith(
                bridge.editorId,
                11,
                11,
                'bulletList'
            );
            expect(update).not.toBeNull();
            expect(update!.renderElements).toHaveLength(3);

            bridge.destroy();
        });

        it('switches from bulletList to orderedList with a single wrap call', () => {
            mockNativeModule.editorGetCurrentState.mockReturnValueOnce(
                JSON.stringify({
                    renderElements: [],
                    selection: { type: 'text', anchor: 3, head: 3 },
                    activeState: {
                        marks: {},
                        nodes: { bulletList: true, listItem: true, paragraph: true },
                        commands: { indentList: true, outdentList: false },
                    },
                    historyState: { canUndo: true, canRedo: false },
                })
            );
            mockNativeModule.editorWrapInListAtSelectionScalar.mockReturnValueOnce(
                MOCK_ORDERED_LIST_UPDATE_JSON
            );

            const bridge = NativeEditorBridge.create();
            const update = bridge.toggleList('orderedList');

            expect(mockNativeModule.editorWrapInListAtSelectionScalar).toHaveBeenCalledWith(
                bridge.editorId,
                3,
                3,
                'orderedList'
            );
            expect(mockNativeModule.editorUnwrapFromListAtSelectionScalar).not.toHaveBeenCalled();
            expect(update).not.toBeNull();
            expect(update!.activeState.nodes.orderedList).toBe(true);

            bridge.destroy();
        });
    });

    describe('toggleBlockquote', () => {
        it('returns parsed EditorUpdate', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.toggleBlockquote();

            expect(mockNativeModule.editorToggleBlockquoteAtSelectionScalar).toHaveBeenCalledWith(
                bridge.editorId,
                0,
                0
            );
            expect(update).not.toBeNull();
            expect(update!.renderElements).toHaveLength(3);

            bridge.destroy();
        });
    });

    describe('toggleHeading', () => {
        it('returns parsed EditorUpdate', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.toggleHeading(3);

            expect(mockNativeModule.editorToggleHeadingAtSelectionScalar).toHaveBeenCalledWith(
                bridge.editorId,
                0,
                0,
                3
            );
            expect(update).not.toBeNull();

            bridge.destroy();
        });
    });

    describe('list depth commands', () => {
        it('indentListItem calls the native module and returns the update', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.indentListItem();

            expect(mockNativeModule.editorIndentListItemAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0
            );
            expect(update?.selection).toEqual({ type: 'text', anchor: 11, head: 11 });
        });

        it('outdentListItem calls the native module and returns the update', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.outdentListItem();

            expect(mockNativeModule.editorOutdentListItemAtSelectionScalar).toHaveBeenCalledWith(
                1,
                0,
                0
            );
            expect(update?.selection).toEqual({ type: 'text', anchor: 11, head: 11 });
        });
    });

    describe('unwrapFromList', () => {
        it('returns parsed EditorUpdate', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.unwrapFromList();

            expect(mockNativeModule.editorUnwrapFromListAtSelectionScalar).toHaveBeenCalledWith(
                bridge.editorId,
                0,
                0
            );
            expect(update).not.toBeNull();
            expect(update!.renderElements).toHaveLength(3);

            bridge.destroy();
        });
    });

    describe('insertNode', () => {
        it('returns parsed EditorUpdate', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.insertNode('horizontalRule');

            expect(mockNativeModule.editorInsertNodeAtSelectionScalar).toHaveBeenCalledWith(
                bridge.editorId,
                0,
                0,
                'horizontalRule'
            );
            expect(update).not.toBeNull();
            expect(update!.renderElements).toHaveLength(3);

            bridge.destroy();
        });
    });

    // ── History ─────────────────────────────────────────────────

    describe('undo', () => {
        it('returns EditorUpdate when undo is available', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.undo();

            expect(mockNativeModule.editorUndo).toHaveBeenCalledWith(bridge.editorId);
            expect(update).not.toBeNull();
            expect(update!.renderElements).toHaveLength(3);

            bridge.destroy();
        });

        it('returns null when nothing to undo (empty string response)', () => {
            mockNativeModule.editorUndo.mockReturnValueOnce('');

            const bridge = NativeEditorBridge.create();
            const update = bridge.undo();

            expect(update).toBeNull();

            bridge.destroy();
        });
    });

    describe('redo', () => {
        it('returns null when nothing to redo (empty string response)', () => {
            const bridge = NativeEditorBridge.create();

            const update = bridge.redo();

            expect(mockNativeModule.editorRedo).toHaveBeenCalledWith(bridge.editorId);
            expect(update).toBeNull();

            bridge.destroy();
        });

        it('returns EditorUpdate when redo is available', () => {
            mockNativeModule.editorRedo.mockReturnValueOnce(MOCK_UPDATE_JSON);

            const bridge = NativeEditorBridge.create();
            const update = bridge.redo();

            expect(update).not.toBeNull();
            expect(update!.historyState.canUndo).toBe(true);

            bridge.destroy();
        });
    });

    describe('canUndo / canRedo', () => {
        it('returns boolean from native module', () => {
            const bridge = NativeEditorBridge.create();

            expect(bridge.canUndo()).toBe(true);
            expect(bridge.canRedo()).toBe(false);

            expect(mockNativeModule.editorCanUndo).toHaveBeenCalledWith(bridge.editorId);
            expect(mockNativeModule.editorCanRedo).toHaveBeenCalledWith(bridge.editorId);

            bridge.destroy();
        });
    });

    // ── Error Handling ──────────────────────────────────────────

    describe('error handling', () => {
        it('throws on invalid JSON from setHtml', () => {
            mockNativeModule.editorSetHtml.mockReturnValueOnce('not valid json {');

            const bridge = NativeEditorBridge.create();

            expect(() => bridge.setHtml('<p>test</p>')).toThrow(
                'NativeEditorBridge: invalid JSON response from native module'
            );

            bridge.destroy();
        });

        it('throws on non-array JSON from setHtml', () => {
            mockNativeModule.editorSetHtml.mockReturnValueOnce('{"not": "an array"}');

            const bridge = NativeEditorBridge.create();

            expect(() => bridge.setHtml('<p>test</p>')).toThrow(
                'NativeEditorBridge: invalid JSON response from native module'
            );

            bridge.destroy();
        });

        it('throws native error responses from setHtml', () => {
            mockNativeModule.editorSetHtml.mockReturnValueOnce(
                JSON.stringify({ error: 'invalid html input' })
            );

            const bridge = NativeEditorBridge.create();

            expect(() => bridge.setHtml('<p>test</p>')).toThrow(
                'NativeEditorBridge: invalid html input'
            );

            bridge.destroy();
        });

        it('throws on error response from insertText', () => {
            mockNativeModule.editorInsertText.mockReturnValueOnce(
                JSON.stringify({ error: 'position out of bounds' })
            );

            const bridge = NativeEditorBridge.create();

            expect(() => bridge.insertText(9999, 'x')).toThrow(
                'NativeEditorBridge: position out of bounds'
            );

            bridge.destroy();
        });

        it('throws on invalid JSON from getJson', () => {
            mockNativeModule.editorGetJson.mockReturnValueOnce('broken json');

            const bridge = NativeEditorBridge.create();

            expect(() => bridge.getJson()).toThrow(
                'NativeEditorBridge: invalid JSON response from native module'
            );

            bridge.destroy();
        });

        it('throws on invalid JSON from insertText update', () => {
            mockNativeModule.editorInsertText.mockReturnValueOnce('{broken');

            const bridge = NativeEditorBridge.create();

            expect(() => bridge.insertText(0, 'x')).toThrow(
                'NativeEditorBridge: invalid JSON response from native module'
            );

            bridge.destroy();
        });
    });

    // ── Dispose Pattern ─────────────────────────────────────────

    describe('dispose pattern (methods throw after destroy)', () => {
        const ERROR_MSG = 'NativeEditorBridge: editor has been destroyed';

        let bridge: NativeEditorBridge;

        beforeEach(() => {
            bridge = NativeEditorBridge.create();
            bridge.destroy();
        });

        it('setHtml throws after destroy', () => {
            expect(() => bridge.setHtml('<p>x</p>')).toThrow(ERROR_MSG);
        });

        it('getHtml throws after destroy', () => {
            expect(() => bridge.getHtml()).toThrow(ERROR_MSG);
        });

        it('setJson throws after destroy', () => {
            expect(() => bridge.setJson({ type: 'doc' })).toThrow(ERROR_MSG);
        });

        it('getJson throws after destroy', () => {
            expect(() => bridge.getJson()).toThrow(ERROR_MSG);
        });

        it('insertText throws after destroy', () => {
            expect(() => bridge.insertText(0, 'x')).toThrow(ERROR_MSG);
        });

        it('deleteRange throws after destroy', () => {
            expect(() => bridge.deleteRange(0, 1)).toThrow(ERROR_MSG);
        });

        it('toggleMark throws after destroy', () => {
            expect(() => bridge.toggleMark('bold')).toThrow(ERROR_MSG);
        });

        it('setSelection throws after destroy', () => {
            expect(() => bridge.setSelection(0, 0)).toThrow(ERROR_MSG);
        });

        it('undo throws after destroy', () => {
            expect(() => bridge.undo()).toThrow(ERROR_MSG);
        });

        it('redo throws after destroy', () => {
            expect(() => bridge.redo()).toThrow(ERROR_MSG);
        });

        it('canUndo throws after destroy', () => {
            expect(() => bridge.canUndo()).toThrow(ERROR_MSG);
        });

        it('canRedo throws after destroy', () => {
            expect(() => bridge.canRedo()).toThrow(ERROR_MSG);
        });

        it('toggleList throws after destroy', () => {
            expect(() => bridge.toggleList('bulletList')).toThrow(ERROR_MSG);
        });

        it('toggleBlockquote throws after destroy', () => {
            expect(() => bridge.toggleBlockquote()).toThrow(ERROR_MSG);
        });

        it('unwrapFromList throws after destroy', () => {
            expect(() => bridge.unwrapFromList()).toThrow(ERROR_MSG);
        });

        it('insertNode throws after destroy', () => {
            expect(() => bridge.insertNode('horizontalRule')).toThrow(ERROR_MSG);
        });
    });

    // ── Render Element Variants ─────────────────────────────────

    describe('render element variants', () => {
        it('parses all render element types correctly', () => {
            const allTypes = JSON.stringify([
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: 'plain', marks: [] },
                { type: 'textRun', text: 'bold', marks: ['bold'] },
                {
                    type: 'blockStart',
                    nodeType: 'listItem',
                    depth: 1,
                    listContext: {
                        ordered: true,
                        index: 0,
                        total: 3,
                        start: 1,
                        isFirst: true,
                        isLast: false,
                    },
                },
                { type: 'voidInline', nodeType: 'image', docPos: 5 },
                { type: 'voidBlock', nodeType: 'horizontalRule', docPos: 10 },
                {
                    type: 'opaqueInlineAtom',
                    nodeType: 'mention',
                    label: '@Alice',
                    docPos: 15,
                },
                {
                    type: 'opaqueBlockAtom',
                    nodeType: 'codeBlock',
                    label: 'codeBlock',
                    docPos: 20,
                },
                { type: 'blockEnd' },
            ]);

            mockNativeModule.editorSetHtml.mockReturnValueOnce(allTypes);

            const bridge = NativeEditorBridge.create();
            const elements = bridge.setHtml('<div>complex</div>');

            expect(elements).toHaveLength(9);

            // blockStart with listContext
            expect(elements[3].type).toBe('blockStart');
            expect(elements[3].listContext).toEqual({
                ordered: true,
                index: 0,
                total: 3,
                start: 1,
                isFirst: true,
                isLast: false,
            });

            // voidInline
            expect(elements[4]).toEqual({
                type: 'voidInline',
                nodeType: 'image',
                docPos: 5,
            });

            // voidBlock
            expect(elements[5]).toEqual({
                type: 'voidBlock',
                nodeType: 'horizontalRule',
                docPos: 10,
            });

            // opaqueInlineAtom
            expect(elements[6]).toEqual({
                type: 'opaqueInlineAtom',
                nodeType: 'mention',
                label: '@Alice',
                docPos: 15,
            });

            // opaqueBlockAtom
            expect(elements[7]).toEqual({
                type: 'opaqueBlockAtom',
                nodeType: 'codeBlock',
                label: 'codeBlock',
                docPos: 20,
            });

            bridge.destroy();
        });
    });

    // ── Selection Variants ──────────────────────────────────────

    describe('selection variants in EditorUpdate', () => {
        it('parses node selection', () => {
            const nodeSelUpdate = JSON.stringify({
                renderElements: [],
                selection: { type: 'node', pos: 5 },
                activeState: { marks: {}, nodes: {}, commands: {} },
                historyState: { canUndo: false, canRedo: false },
            });
            mockNativeModule.editorInsertText.mockReturnValueOnce(nodeSelUpdate);

            const bridge = NativeEditorBridge.create();
            const update = bridge.insertText(0, 'x');

            expect(update!.selection).toEqual({ type: 'node', pos: 5 });

            bridge.destroy();
        });

        it('parses all selection', () => {
            const allSelUpdate = JSON.stringify({
                renderElements: [],
                selection: { type: 'all' },
                activeState: { marks: {}, nodes: {}, commands: {} },
                historyState: { canUndo: false, canRedo: false },
            });
            mockNativeModule.editorInsertText.mockReturnValueOnce(allSelUpdate);

            const bridge = NativeEditorBridge.create();
            const update = bridge.insertText(0, 'x');

            expect(update!.selection).toEqual({ type: 'all' });

            bridge.destroy();
        });
    });
});
