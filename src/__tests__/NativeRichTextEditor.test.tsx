// Shared v2 document session, the only construction mode. Native module and
// view manager are both mocked.

const mockNativeFocus = jest.fn();
const mockNativeBlur = jest.fn();
const mockNativeGetCaretRect = jest.fn();
const mockNativeModule: Record<string, jest.Mock> = {};

jest.mock('expo-modules-core', () => {
    const React = require('react');
    const { View } = require('react-native');

    const MockNativeView = React.forwardRef(
        (props: Record<string, unknown>, ref: React.Ref<unknown>) => {
            React.useImperativeHandle(ref, () => ({
                focus: mockNativeFocus,
                blur: mockNativeBlur,
                getCaretRect: mockNativeGetCaretRect,
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

jest.mock('../schemas', () => {
    const actual = jest.requireActual('../schemas');
    const mockResolveDocumentDescriptor = jest.fn(actual.resolveDocumentDescriptor);
    return {
        ...actual,
        resolveDocumentDescriptor: mockResolveDocumentDescriptor,
    };
});

import React, { createRef } from 'react';
import { StyleSheet, View } from 'react-native';
import { render, act, fireEvent } from '@testing-library/react-native';

import {
    NativeRichTextEditor,
    type NativeRichTextEditorProps,
    type NativeRichTextEditorRef,
} from '../NativeRichTextEditor';
import {
    createNativeEditorDocumentHandle,
    type NativeEditorDocumentHandle,
    _resetNativeModuleCache,
    type DocumentJSON,
} from '../NativeEditorBridge';
import {
    NativeEditorV2BoundaryError,
    NativeEditorV2NonRetryableError,
    NativeEditorV2OperationError,
} from '../NativeEditorBoundaryError';
import * as EditorUpdateRevision from '../EditorUpdateRevision';
import { _resetEditorToolbarFrameRegistryForTests } from '../EditorToolbar';
import { createYjsCollaborationController, useYjsCollaboration } from '../YjsCollaboration';
import {
    createFakeNativeEditorV2Runtime,
    fakeDocForText,
    V2_FAKE_STEP1_FRAME,
    V2_FAKE_STEP2_FRAME,
    V2_FAKE_UPDATE_FRAME,
    type FakeNativeEditorV2Runtime,
} from './helpers/nativeEditorV2Fake';
import { withMentionsSchema } from '../addons';
import { tiptapSchema, type SchemaDefinition } from '../schemas';

const mockResolveDocumentDescriptor = require('../schemas').resolveDocumentDescriptor as jest.Mock;

const HANDLE_OWNED_ARTICLE_SCHEMA: SchemaDefinition = {
    nodes: [
        { name: 'article', content: '(title | image)+', role: 'doc' },
        { name: 'title', content: 'inline*', group: 'block', role: 'textBlock' },
        { name: 'image', content: '', group: 'block', role: 'block', attrs: { src: {} } },
        { name: 'text', content: '', group: 'inline', role: 'text' },
    ],
    marks: [],
};

/** The room URL every collaboration-bound view test configures. */
const V2_TRANSPORT_URL = 'wss://example.test/collaboration';

describe('NativeRichTextEditor (v2 document mode)', () => {
    let v2Runtime: FakeNativeEditorV2Runtime;

    const V2_INITIAL_DOC = fakeDocForText('hello');
    const V2_DOC_B = fakeDocForText('local b');
    const V2_DOC_C = fakeDocForText('local c');
    const V2_SERVER_DOC = fakeDocForText('server');
    const V2_SERVER_UPDATE_DOC = fakeDocForText('server update');

    function createV2RoomHandle(options: { withSnapshot?: boolean } = {}) {
        return createNativeEditorDocumentHandle({
            initialization: {
                type: 'room',
                documentId: 'doc-1',
                lineageId: 'lineage-1',
                ...(options.withSnapshot
                    ? {
                          snapshot: {
                              metadata: {
                                  formatVersion: 1,
                                  documentId: 'doc-1',
                                  lineageId: 'lineage-1',
                                  fragmentName: 'prosemirror',
                                  schemaFingerprint: 'fakefingerprint',
                              },
                              encodedState: new TextEncoder().encode(
                                  JSON.stringify({ doc: V2_INITIAL_DOC, revision: 7 })
                              ),
                          },
                      }
                    : {}),
            },
        });
    }

    function createV2LocalHandle(doc?: DocumentJSON) {
        return createNativeEditorDocumentHandle({
            initialization: doc ? { type: 'localJson', json: doc } : { type: 'localEmpty' },
        });
    }

    /**
     * Bind one controller to a handle. The socket lives natively, so the
     * test drives it through the fake runtime's transport controls rather
     * than through any JavaScript-owned WebSocket.
     */
    function setupV2Controller(handle: NativeEditorDocumentHandle) {
        const controller = createYjsCollaborationController({
            documentId: 'doc-1',
            handle,
            transport: { url: V2_TRANSPORT_URL, connect: false },
        });
        return { controller };
    }

    beforeEach(() => {
        jest.useFakeTimers();
        _resetNativeModuleCache();
        v2Runtime = createFakeNativeEditorV2Runtime();
        for (const key of Object.keys(mockNativeModule)) {
            delete mockNativeModule[key];
        }
        Object.assign(mockNativeModule, v2Runtime.module);
        mockNativeFocus.mockClear();
        mockNativeBlur.mockClear();
        mockNativeGetCaretRect.mockReset();
        mockResolveDocumentDescriptor.mockClear();
    });

    afterEach(() => {
        act(() => {
            _resetEditorToolbarFrameRegistryForTests();
        });
        jest.useRealTimers();
    });

    it('accepts only handle-bound view props and never creates a session on mount', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);

        // The @ts-expect-error assertions are the public hard-cutover
        // contract. Each field belongs to NativeEditorV2CreateConfig, never
        // to the mounted view.
        const removedComponentProps: readonly NativeRichTextEditorProps[] = [
            {
                documentHandle: handle,
                // @ts-expect-error schema belongs to handle creation
                schema: undefined,
            },
            {
                documentHandle: handle,
                // @ts-expect-error resource limits belong to handle creation
                resourceLimits: undefined,
            },
            {
                documentHandle: handle,
                // @ts-expect-error base64 policy belongs to handle creation
                allowBase64Images: false,
            },
            {
                documentHandle: handle,
                // @ts-expect-error maximum length belongs to handle creation
                maxLength: 1,
            },
            {
                documentHandle: handle,
                // @ts-expect-error engine read-only belongs to handle creation
                readOnly: true,
            },
            {
                documentHandle: handle,
                // @ts-expect-error input filtering belongs to handle creation
                inputFilter: '[a-z]',
            },
            {
                documentHandle: handle,
                // @ts-expect-error fragment selection belongs to handle creation
                fragmentName: 'prosemirror',
            },
            {
                documentHandle: handle,
                // @ts-expect-error initial HTML belongs to handle creation
                initialContent: '<p>legacy</p>',
            },
            {
                documentHandle: handle,
                // @ts-expect-error initial JSON belongs to handle creation
                initialJSON: V2_INITIAL_DOC,
            },
        ];
        expect(removedComponentProps).toHaveLength(9);

        mockNativeModule.editorV2Create.mockClear();
        mockResolveDocumentDescriptor.mockClear();
        const { getByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} editable={false} />
        );

        expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        expect(mockResolveDocumentDescriptor).not.toHaveBeenCalled();
        expect(getByTestId('native-editor-view').props.editorId).toBe(handle.editorId);
        handle.destroy();
    });

    it('uses custom-root metadata owned by the handle for clear and image fragments', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: HANDLE_OWNED_ARTICLE_SCHEMA,
            initialization: { type: 'localEmpty' },
        });
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);

        act(() => ref.current!.clearContent());
        const clearRequest = JSON.parse(
            mockNativeModule.editorV2ReplaceDocument.mock.calls[
                mockNativeModule.editorV2ReplaceDocument.mock.calls.length - 1
            ][1] as string
        ) as Record<string, unknown>;
        expect(clearRequest.setJson).toEqual({
            type: 'article',
            content: [{ type: 'title' }],
        });

        act(() => ref.current!.insertImage('https://example.test/image.png'));
        const imageRequest = JSON.parse(
            mockNativeModule.editorV2ApplyCommand.mock.calls[
                mockNativeModule.editorV2ApplyCommand.mock.calls.length - 1
            ][1] as string
        ) as { command: Record<string, unknown> };
        expect(imageRequest.command).toEqual({
            type: 'insertContentJson',
            json: {
                type: 'article',
                content: [{ type: 'image', attrs: { src: 'https://example.test/image.png' } }],
            },
        });

        handle.destroy();
    });

    it('normalizes an empty controlled custom-root document from its handle descriptor', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: HANDLE_OWNED_ARTICLE_SCHEMA,
            initialization: { type: 'localEmpty' },
        });
        const { rerender } = render(<NativeRichTextEditor documentHandle={handle} />);
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        rerender(
            <NativeRichTextEditor
                documentHandle={handle}
                valueJSON={{ type: 'article', content: [] }}
            />
        );

        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(1);
        const request = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        expect(request.setJson).toEqual({
            type: 'article',
            content: [{ type: 'title' }],
        });

        handle.destroy();
    });

    it('retains immutable custom-root defaults after the caller mutates its schema', () => {
        const mutableCardDefault = {
            appearance: { tone: 'blue' },
            labels: ['initial'],
        };
        const schema: SchemaDefinition = {
            nodes: [
                { name: 'article', content: 'card', role: 'doc' },
                {
                    name: 'card',
                    content: '',
                    group: 'block',
                    role: 'block',
                    attrs: { config: { default: mutableCardDefault } },
                },
                { name: 'text', content: '', group: 'inline', role: 'text' },
            ],
            marks: [],
        };
        const handle = createNativeEditorDocumentHandle({
            schema,
            initialization: { type: 'localEmpty' },
        });
        mutableCardDefault.appearance.tone = 'mutated';
        mutableCardDefault.labels.push('later');

        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={{ type: 'article', content: [] }}
            />
        );

        const expectedEmptyArticle = {
            type: 'article',
            content: [
                {
                    type: 'card',
                    attrs: {
                        config: { appearance: { tone: 'blue' }, labels: ['initial'] },
                    },
                },
            ],
        };
        const controlledRequest = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        expect(controlledRequest.setJson).toEqual(expectedEmptyArticle);

        act(() => ref.current!.clearContent());
        const clearRequest = JSON.parse(
            mockNativeModule.editorV2ReplaceDocument.mock.calls[
                mockNativeModule.editorV2ReplaceDocument.mock.calls.length - 1
            ][1] as string
        ) as Record<string, unknown>;
        expect(clearRequest.setJson).toEqual(expectedEmptyArticle);

        handle.destroy();
    });

    it('drives the retained document API through the v2 handle', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onContentChange = jest.fn();
        const onContentChangeJSON = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onContentChange={onContentChange}
                onContentChangeJSON={onContentChangeJSON}
            />
        );

        expect(getByTestId('native-editor-view')).toBeTruthy();

        expect(ref.current!.getContentJson()).toEqual(V2_INITIAL_DOC);
        expect(ref.current!.getContent()).toBe('<p>hello</p>');
        expect(ref.current!.getIsEmpty()).toBe(false);
        expect(ref.current!.canUndo()).toBe(false);

        act(() => {
            ref.current!.setContent('<p>world</p>');
        });
        expect(mockNativeModule.editorV2ReplaceDocument).toHaveBeenCalledTimes(1);
        const replaceRequest = JSON.parse(
            mockNativeModule.editorV2ReplaceDocument.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        expect(replaceRequest.setHtml).toBe('<p>world</p>');
        expect(replaceRequest.history).toBe('undoableBoundary');
        expect(ref.current!.getContent()).toBe('<p>world</p>');
        expect(onContentChange).toHaveBeenCalledWith('<p>world</p>');
        expect(onContentChangeJSON).toHaveBeenCalledWith(fakeDocForText('world'));

        // Undo state is the engine's, not a TypeScript copy.
        expect(ref.current!.canUndo()).toBe(true);
        act(() => {
            ref.current!.undo();
        });
        expect(ref.current!.getContentJson()).toEqual(V2_INITIAL_DOC);
        expect(ref.current!.canUndo()).toBe(false);

        act(() => {
            ref.current!.setContentJson(V2_DOC_B);
        });
        expect(ref.current!.getContentJson()).toEqual(V2_DOC_B);

        act(() => {
            ref.current!.clearContent();
        });
        expect(ref.current!.getIsEmpty()).toBe(true);
        const clearRequest = JSON.parse(
            mockNativeModule.editorV2ReplaceDocument.mock.calls[
                mockNativeModule.editorV2ReplaceDocument.mock.calls.length - 1
            ][1] as string
        ) as Record<string, unknown>;
        expect(clearRequest.history).toBe('resetAndClear');
        expect(ref.current!.canUndo()).toBe(false);
        expect(ref.current!.getContentJson()).toEqual({
            type: 'doc',
            content: [{ type: 'paragraph' }],
        });

        handle.destroy();
    });

    it('maps controlled valueJSONUpdateMode="replace" to an undoable engine boundary', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='replace'
            />
        );
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_DOC_B}
                valueJSONUpdateMode='replace'
            />
        );

        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(1);
        const request = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        expect(request.history).toBe('undoableBoundary');
        expect(request.setJson).toEqual(V2_DOC_B);
        expect(request.baseDocumentRevision).toBe('1');
        // Verified through engine undo state, not document content.
        expect(ref.current!.canUndo()).toBe(true);
        // Release the controlled prop first — a controlled value always
        // re-drives the document, so undo is observable uncontrolled.
        rerender(
            <NativeRichTextEditor ref={ref} documentHandle={handle} valueJSONUpdateMode='replace' />
        );
        act(() => {
            ref.current!.undo();
        });
        expect(ref.current!.getContentJson()).toEqual(V2_INITIAL_DOC);
        handle.destroy();
    });

    it('maps controlled valueJSONUpdateMode="reset" to a non-undoable history clear', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='replace'
            />
        );
        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_DOC_B}
                valueJSONUpdateMode='replace'
            />
        );
        expect(ref.current!.canUndo()).toBe(true);
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_DOC_C}
                valueJSONUpdateMode='reset'
            />
        );

        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(1);
        const request = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        expect(request.history).toBe('resetAndClear');
        expect(request.setJson).toEqual(V2_DOC_C);
        // The reset cleared the engine undo stack that the replace filled.
        expect(ref.current!.canUndo()).toBe(false);
        expect(ref.current!.getContentJson()).toEqual(V2_DOC_C);
        handle.destroy();
    });

    it('refreshes from the engine after REVISION_MISMATCH and re-applies against the fresh revision', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='replace'
            />
        );
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        // The engine advances behind the component's back (rev 1 -> 2).
        handle.bridge.replaceDocument({ setJson: V2_DOC_B, history: 'undoableBoundary' });

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_DOC_C}
                valueJSONUpdateMode='replace'
            />
        );

        // First attempt used the stale cached base revision and was
        // rejected; the component refreshed and re-applied against the
        // engine's actual revision.
        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(2);
        const staleRequest = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        const freshRequest = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[1][1] as string
        ) as Record<string, unknown>;
        expect(String(staleRequest.baseDocumentRevision)).toBe('1');
        expect(String(freshRequest.baseDocumentRevision)).toBe('2');
        expect(freshRequest.setJson).toEqual(V2_DOC_C);
        expect(ref.current!.getContentJson()).toEqual(V2_DOC_C);
        handle.destroy();
    });

    it('renders nothing for a room editor without snapshot until an accepted server Step 2, and emits no client state', () => {
        const handle = createV2RoomHandle();
        const { controller } = setupV2Controller(handle);
        const ref = createRef<NativeRichTextEditorRef>();
        const { queryByTestId, rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
            />
        );

        // Loading: the engine is AwaitRemote, so nothing renders.
        expect(queryByTestId('native-editor-view')).toBeNull();

        act(() => {
            controller.connect();
        });
        act(() => {
            v2Runtime.transportOpen(handle.editorId);
        });
        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
            />
        );
        // Handshaking is not synchronized: still no client-side document.
        expect(queryByTestId('native-editor-view')).toBeNull();
        expect(controller.state.status).toBe('handshaking');

        act(() => {
            v2Runtime.pushRemoteDoc(handle.editorId, V2_SERVER_DOC);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
        });
        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
            />
        );

        // The first render comes only from the accepted server Step 2.
        expect(queryByTestId('native-editor-view')).not.toBeNull();
        expect(controller.state.status).toBe('synchronized');
        expect(ref.current!.getContentJson()).toEqual(V2_SERVER_DOC);

        // Server-owned room init: the editor emitted no client state at
        // any point (no local mutations, no seeding).
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ReplaceDocument).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();
        handle.destroy();
    });

    it('re-renders remote commits from the shared engine with no TypeScript document push (no callback reset loop)', () => {
        const handle = createV2RoomHandle({ withSnapshot: true });
        const { controller } = setupV2Controller(handle);
        const ref = createRef<NativeRichTextEditorRef>();
        const onContentChangeJSON = jest.fn();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
                onContentChangeJSON={onContentChangeJSON}
            />
        );
        expect(ref.current!.getContentJson()).toEqual(V2_INITIAL_DOC);

        act(() => {
            controller.connect();
        });
        act(() => {
            v2Runtime.transportOpen(handle.editorId);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
        });
        expect(controller.state.status).toBe('synchronized');
        mockNativeModule.editorV2ApplyLocalApi.mockClear();
        mockNativeModule.editorV2ReplaceDocument.mockClear();
        mockNativeModule.editorV2ApplyInput.mockClear();

        act(() => {
            v2Runtime.pushRemoteDoc(handle.editorId, V2_SERVER_UPDATE_DOC);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_UPDATE_FRAME);
        });
        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
                onContentChangeJSON={onContentChangeJSON}
            />
        );

        // Document updates flow through the returned Rust state only.
        expect(ref.current!.getContentJson()).toEqual(V2_SERVER_UPDATE_DOC);
        expect(controller.state.documentJson).toEqual(V2_SERVER_UPDATE_DOC);
        expect(onContentChangeJSON).toHaveBeenCalledWith(V2_SERVER_UPDATE_DOC);
        // No full-document reset (or any local mutation) was pushed back
        // into the engine from TypeScript.
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ReplaceDocument).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();
        handle.destroy();
    });

    it('still serializes remoteSelections for the native view in v2 mode', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const remoteSelections = [
            {
                clientId: '42',
                anchor: 4,
                head: 9,
                color: '#00f',
                name: 'Bob',
                isFocused: true,
            },
        ];
        const { getByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} remoteSelections={remoteSelections} />
        );
        expect(getByTestId('native-editor-view').props.remoteSelectionsJson).toBe(
            JSON.stringify(remoteSelections)
        );
        handle.destroy();
    });

    it('forwards focus and blur ref calls to the native view', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);

        act(() => {
            ref.current!.focus();
        });
        expect(mockNativeFocus).toHaveBeenCalledTimes(1);
        act(() => {
            ref.current!.blur();
        });
        expect(mockNativeBlur).toHaveBeenCalledTimes(1);
        handle.destroy();
    });

    it('parses getCaretRect JSON from the native view', async () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);

        mockNativeGetCaretRect.mockReturnValue(
            JSON.stringify({ x: 1, y: 2, width: 3, height: 4, editorWidth: 100, editorHeight: 50 })
        );
        let rect: unknown;
        await act(async () => {
            rect = await ref.current!.getCaretRect();
        });
        expect(rect).toEqual({
            x: 1,
            y: 2,
            width: 3,
            height: 4,
            editorWidth: 100,
            editorHeight: 50,
        });

        mockNativeGetCaretRect.mockReturnValue(null);
        await act(async () => {
            rect = await ref.current!.getCaretRect();
        });
        expect(rect).toBeNull();

        mockNativeGetCaretRect.mockReturnValue('{"x":1}');
        await act(async () => {
            rect = await ref.current!.getCaretRect();
        });
        expect(rect).toBeNull();
        handle.destroy();
    });

    it('passes placeholder, accessibility props, and theme through to the native view', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                placeholder='Write something…'
                accessibilityLabel='Body'
                accessibilityHint='Message body editor'
                theme={{ paragraph: { textSize: 17 } }}
            />
        );
        const view = getByTestId('native-editor-view');
        expect(view.props.placeholder).toBe('Write something…');
        expect(view.props.accessibilityLabel).toBe('Body');
        expect(view.props.accessibilityHint).toBe('Message body editor');
        expect(JSON.parse(view.props.themeJson as string)).toEqual({
            paragraph: { textSize: 17 },
        });
        handle.destroy();
    });

    function renderUpdateValue(editorId: string, anchor?: number, head?: number): string {
        const raw = v2Runtime.module.editorV2RenderUpdate(
            editorId,
            anchor ?? null,
            head ?? null
        ) as { value: string };
        return raw.value;
    }

    it('binds the native view to the session editor id and is editable by default', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId } = render(<NativeRichTextEditor documentHandle={handle} />);
        const view = getByTestId('native-editor-view');
        expect(view.props.editorId).toBe(handle.editorId);
        expect(view.props.editable).toBe(true);
        expect(view.props.showToolbar).toBe(true);
        expect(view.props.autoFocus).toBe(false);
        expect(typeof view.props.onEditorUpdate).toBe('function');
        expect(typeof view.props.onEditorError).toBe('function');
        // The native side marked the session live for view binding at create.
        expect(v2Runtime.liveEditorIds()).toContain(handle.editorId);
        handle.destroy();
    });

    it.each(['iOS', 'Android'])(
        'routes each valid %s autonomous native error once through the bound handle',
        (platform) => {
            const handle = createV2LocalHandle(V2_INITIAL_DOC);
            const primaryListener = jest.fn();
            const secondaryListener = jest.fn();
            handle.addErrorListener(primaryListener);
            handle.addErrorListener(secondaryListener);
            const { getByTestId } = render(<NativeRichTextEditor documentHandle={handle} />);
            const view = getByTestId('native-editor-view');
            const error = {
                domain: 'operation',
                code: 'POSITION_INVALID',
                message: 'native selection is invalid',
                requestId: '7',
            };
            const emit = () =>
                view.props.onEditorError({
                    // iOS constructs the map identity-first while Android
                    // supplies the error field first.
                    nativeEvent:
                        platform === 'iOS'
                            ? { editorId: handle.editorId, error }
                            : { error, editorId: handle.editorId },
                });

            act(() => emit());
            act(() => emit());

            // Equal native failures are separate emissions, never value-deduped.
            expect(primaryListener).toHaveBeenCalledTimes(2);
            expect(secondaryListener).toHaveBeenCalledTimes(2);
            expect(primaryListener).toHaveBeenNthCalledWith(
                1,
                expect.objectContaining({ code: 'POSITION_INVALID', requestId: '7' })
            );
            handle.destroy();
        }
    );

    it('rejects invalid error identities and turns one malformed current error into FFI_RESULT_INVALID', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const received: unknown[] = [];
        handle.addErrorListener((error) => received.push(error));
        const { getByTestId } = render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);
        const view = getByTestId('native-editor-view');
        const validError = {
            domain: 'operation',
            code: 'POSITION_INVALID',
            message: 'native selection is invalid',
        };

        for (const editorId of [undefined, 1, '01', '18446744073709551616', '999']) {
            act(() => {
                view.props.onEditorError({ nativeEvent: { editorId, error: validError } });
            });
        }
        expect(received).toHaveLength(0);

        act(() => {
            view.props.onEditorError({
                nativeEvent: { editorId: handle.editorId, error: { code: 42 } },
            });
        });
        expect(received).toHaveLength(1);
        expect(received[0]).toBeInstanceOf(NativeEditorV2NonRetryableError);
        expect((received[0] as NativeEditorV2NonRetryableError).code).toBe('FFI_RESULT_INVALID');

        // The boundary event is non-terminal: a normal interaction remains usable.
        act(() => ref.current!.toggleMark('bold'));
        expect(mockNativeModule.editorV2ApplyCommand).toHaveBeenCalledTimes(1);
        handle.destroy();
    });

    it('drops late autonomous errors after unsubscribe, rebind, unmount, and destroy', () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const receivedA: unknown[] = [];
        const receivedB: unknown[] = [];
        const unsubscribeA = handleA.addErrorListener((error) => receivedA.push(error));
        handleB.addErrorListener((error) => receivedB.push(error));
        const { getByTestId, rerender, unmount } = render(
            <NativeRichTextEditor documentHandle={handleA} />
        );
        const firstABinding = getByTestId('native-editor-view').props.onEditorError;
        const errorA = {
            nativeEvent: {
                editorId: handleA.editorId,
                error: { domain: 'operation', code: 'POSITION_INVALID', message: 'A error' },
            },
        };

        act(() => firstABinding(errorA));
        expect(receivedA).toHaveLength(1);
        unsubscribeA();
        act(() => firstABinding(errorA));
        expect(receivedA).toHaveLength(1);

        rerender(<NativeRichTextEditor documentHandle={handleB} />);
        const bBinding = getByTestId('native-editor-view').props.onEditorError;
        rerender(<NativeRichTextEditor documentHandle={handleA} />);
        const reboundABinding = getByTestId('native-editor-view').props.onEditorError;
        handleA.addErrorListener((error) => receivedA.push(error));

        act(() => firstABinding(errorA));
        act(() =>
            bBinding({
                nativeEvent: {
                    editorId: handleB.editorId,
                    error: { domain: 'operation', code: 'POSITION_INVALID', message: 'B error' },
                },
            })
        );
        expect(receivedA).toHaveLength(1);
        expect(receivedB).toHaveLength(0);

        act(() => reboundABinding(errorA));
        expect(receivedA).toHaveLength(2);

        unmount();
        act(() => reboundABinding(errorA));
        expect(receivedA).toHaveLength(2);

        handleA.destroy();
        act(() => reboundABinding(errorA));
        expect(receivedA).toHaveLength(2);
        handleB.destroy();
    });

    it('respects editable={false} on the native view', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} editable={false} />
        );
        expect(getByTestId('native-editor-view').props.editable).toBe(false);
        handle.destroy();
    });

    it('routes a native typing commit through the engine exactly once and never echoes it back', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onContentChange = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onContentChange={onContentChange}
            />
        );
        mockNativeModule.editorV2ApplyInput.mockClear();
        mockNativeModule.editorV2ApplyCommand.mockClear();
        mockNativeModule.editorV2ApplyLocalApi.mockClear();
        mockNativeModule.editorV2ReplaceDocument.mockClear();

        // While the IME is composing there is no native event and no engine
        // traffic at all — transient composing text never crosses JS.
        expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();

        // The native adapter commits the final text as ONE typed transaction,
        // then the view emits the resulting update.
        const commitRequest = JSON.stringify({
            version: 1,
            requestId: '1',
            baseDocumentRevision: handle.bridge.getState().documentRevision,
            text: '!',
        });
        let commitOutcome: { value: string } | undefined;
        act(() => {
            commitOutcome = v2Runtime.module.editorV2ApplyInput(handle.editorId, commitRequest) as {
                value: string;
            };
        });
        expect(JSON.parse(commitOutcome!.value)).toMatchObject({ type: 'transaction' });

        act(() => {
            getByTestId('native-editor-view').props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: renderUpdateValue(handle.editorId),
                    documentRevision: handle.bridge.getState().documentRevision,
                },
            });
        });

        // The component issued no mutation of its own: exactly one typed
        // transaction reached the engine (the adapter's).
        expect(mockNativeModule.editorV2ApplyInput).toHaveBeenCalledTimes(1);
        expect(mockNativeModule.editorV2ApplyCommand).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ReplaceDocument).not.toHaveBeenCalled();
        // The content callback fired exactly once for the one commit.
        expect(onContentChange).toHaveBeenCalledTimes(1);
        expect(onContentChange).toHaveBeenCalledWith('<p>hello!</p>');
        // No echo: the view already applied the adapter's update natively.
        expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();
        expect(ref.current!.getContent()).toBe('<p>hello!</p>');
        handle.destroy();
    });

    it.each(['iOS', 'Android'])(
        'accepts only an authentic canonical %s native commit payload and suppresses its exact echo',
        (platform) => {
            const handle = createV2LocalHandle(V2_INITIAL_DOC);
            const onContentChange = jest.fn();
            const { getByTestId, rerender } = render(
                <NativeRichTextEditor documentHandle={handle} onContentChange={onContentChange} />
            );
            const view = getByTestId('native-editor-view');
            const commit = (text: string) => {
                v2Runtime.module.editorV2ApplyInput(
                    handle.editorId,
                    JSON.stringify({
                        version: 1,
                        requestId: text === '!' ? '1' : '2',
                        baseDocumentRevision: handle.bridge.getState().documentRevision,
                        text,
                    })
                );
                const documentRevision = handle.bridge.getState().documentRevision;
                return {
                    editorId: handle.editorId,
                    documentRevision,
                    updateJson: renderUpdateValue(handle.editorId),
                };
            };
            const first = commit('!');
            const emit = (payload: Record<string, unknown>) =>
                view.props.onEditorUpdate({
                    // Keep both native property orderings covered: iOS emits
                    // identity first while Android builds its map update-first.
                    nativeEvent:
                        platform === 'iOS'
                            ? payload
                            : {
                                  updateJson: payload.updateJson,
                                  documentRevision: payload.documentRevision,
                                  editorId: payload.editorId,
                              },
                });

            const mismatchedSnapshot = JSON.parse(first.updateJson) as Record<string, unknown>;
            mismatchedSnapshot.documentVersion = '0';
            const rejectedPayloads: Record<string, unknown>[] = [
                { ...first, editorId: '999' },
                { ...first, documentRevision: '01' },
                { ...first, documentRevision: '18446744073709551616' },
                { ...first, updateJson: '{}' },
                { ...first, updateJson: JSON.stringify(mismatchedSnapshot) },
            ];
            for (const rejected of rejectedPayloads) {
                act(() => emit(rejected));
            }
            expect(onContentChange).not.toHaveBeenCalled();

            act(() => emit(first));
            expect(onContentChange).toHaveBeenCalledTimes(1);

            // Duplicate native delivery must not refresh, notify, or replace
            // the one-shot echo token.
            act(() => emit(first));
            expect(onContentChange).toHaveBeenCalledTimes(1);

            // The identical revision signal is the sole suppressed echo.
            rerender(
                <NativeRichTextEditor
                    documentHandle={handle}
                    documentRevision={first.documentRevision}
                    onContentChange={onContentChange}
                />
            );
            expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();

            // A different/newer external revision clears that token and is
            // pushed, rather than being hidden behind native revision N.
            handle.bridge.replaceDocument({ setJson: V2_DOC_B, history: 'undoableBoundary' });
            const externalRevision = handle.bridge.getState().documentRevision;
            rerender(
                <NativeRichTextEditor
                    documentHandle={handle}
                    documentRevision={externalRevision}
                    onContentChange={onContentChange}
                />
            );
            expect(
                JSON.parse(getByTestId('native-editor-view').props.editorUpdateJson as string)
                    .documentVersion
            ).toBe(externalRevision);

            // A stale native commit after N+1 is deterministically rejected:
            // it produces no further content notification of its own.
            const notificationsBeforeStaleCommit = onContentChange.mock.calls.length;
            act(() => emit(first));
            expect(onContentChange).toHaveBeenCalledTimes(notificationsBeforeStaleCommit);
            handle.destroy();
        }
    );

    it('forwards native selection changes with the engine selection and fires focus/blur edges', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const onSelectionChange = jest.fn();
        const onFocus = jest.fn();
        const onBlur = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                onSelectionChange={onSelectionChange}
                onFocus={onFocus}
                onBlur={onBlur}
            />
        );

        act(() => {
            getByTestId('native-editor-view').props.onSelectionChange({
                nativeEvent: {
                    anchor: 1,
                    head: 3,
                    editorId: handle.editorId,
                    stateJson: renderUpdateValue(handle.editorId, 1, 3),
                },
            });
        });
        expect(onSelectionChange).toHaveBeenCalledTimes(1);
        // The native mirror offsets are scalar coordinates; the render
        // payload resolves them to the engine's document coordinates.
        expect(onSelectionChange).toHaveBeenCalledWith({ type: 'text', anchor: 2, head: 4 });

        // Without a state payload the raw scalar range is forwarded.
        act(() => {
            getByTestId('native-editor-view').props.onSelectionChange({
                nativeEvent: { anchor: 2, head: 4, editorId: handle.editorId },
            });
        });
        expect(onSelectionChange).toHaveBeenLastCalledWith({ type: 'text', anchor: 2, head: 4 });

        act(() => {
            getByTestId('native-editor-view').props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });
        expect(onFocus).toHaveBeenCalledTimes(1);
        expect(onBlur).not.toHaveBeenCalled();
        act(() => {
            getByTestId('native-editor-view').props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });
        expect(onFocus).toHaveBeenCalledTimes(1);
        act(() => {
            getByTestId('native-editor-view').props.onFocusChange({
                nativeEvent: { isFocused: false, editorId: handle.editorId },
            });
        });
        expect(onBlur).toHaveBeenCalledTimes(1);
        handle.destroy();
    });

    it('discards a stale auto-grow height before returning from fixed mode', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor documentHandle={handle} heightBehavior='autoGrow' />
        );

        act(() => {
            getByTestId('native-editor-view').props.onContentHeightChange({
                nativeEvent: { contentHeight: 420, editorId: handle.editorId },
            });
        });
        expect(getByTestId('native-editor-view').props.style).toEqual(
            expect.objectContaining({ height: 420 })
        );

        rerender(<NativeRichTextEditor documentHandle={handle} heightBehavior='fixed' />);
        rerender(<NativeRichTextEditor documentHandle={handle} heightBehavior='autoGrow' />);

        expect(getByTestId('native-editor-view').props.style).not.toEqual(
            expect.objectContaining({ height: 420 })
        );
        handle.destroy();
    });

    it('ignores missing and late editor-scoped interaction events after rebinding to B', () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const onSelectionChange = jest.fn();
        const onFocus = jest.fn();
        const onBlur = jest.fn();
        const onToolbarAction = jest.fn();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor
                documentHandle={handleA}
                heightBehavior='autoGrow'
                onSelectionChange={onSelectionChange}
                onFocus={onFocus}
                onBlur={onBlur}
                onToolbarAction={onToolbarAction}
            />
        );

        rerender(
            <NativeRichTextEditor
                documentHandle={handleB}
                heightBehavior='autoGrow'
                onSelectionChange={onSelectionChange}
                onFocus={onFocus}
                onBlur={onBlur}
                onToolbarAction={onToolbarAction}
            />
        );
        const view = getByTestId('native-editor-view');

        act(() => {
            view.props.onSelectionChange({ nativeEvent: { anchor: 9, head: 9 } });
            view.props.onFocusChange({ nativeEvent: { isFocused: true } });
            view.props.onContentHeightChange({ nativeEvent: { contentHeight: 240 } });
            view.props.onToolbarAction({ nativeEvent: { key: 'late-action' } });
            view.props.onSelectionChange({
                nativeEvent: { anchor: 9, head: 9, editorId: handleA.editorId },
            });
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handleA.editorId },
            });
            view.props.onFocusChange({
                nativeEvent: { isFocused: false, editorId: handleA.editorId },
            });
            view.props.onContentHeightChange({
                nativeEvent: { contentHeight: 240, editorId: handleA.editorId },
            });
            view.props.onToolbarAction({
                nativeEvent: { key: 'late-action', editorId: handleA.editorId },
            });
        });

        expect(onSelectionChange).not.toHaveBeenCalled();
        expect(onFocus).not.toHaveBeenCalled();
        expect(onBlur).not.toHaveBeenCalled();
        expect(onToolbarAction).not.toHaveBeenCalled();
        expect(view.props.style).not.toEqual(expect.objectContaining({ height: 240 }));

        act(() => {
            view.props.onSelectionChange({
                nativeEvent: { anchor: 2, head: 2, editorId: handleB.editorId },
            });
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handleB.editorId },
            });
            view.props.onContentHeightChange({
                nativeEvent: { contentHeight: 240, editorId: handleB.editorId },
            });
            view.props.onToolbarAction({
                nativeEvent: { key: 'B-action', editorId: handleB.editorId },
            });
        });

        expect(onSelectionChange).toHaveBeenCalledWith({ type: 'text', anchor: 2, head: 2 });
        expect(onFocus).toHaveBeenCalledTimes(1);
        expect(onToolbarAction).toHaveBeenCalledWith('B-action');
        expect(getByTestId('native-editor-view').props.style).toEqual(
            expect.objectContaining({ height: 240 })
        );
        handleA.destroy();
        handleB.destroy();
    });

    it('keeps editor-bound awareness hooks inert after localAwareness is removed', () => {
        const handle = createV2RoomHandle({ withSnapshot: true });
        let localAwareness: { userId: string; name: string; color: string } | undefined = {
            userId: '1',
            name: 'Alice',
            color: '#f00',
        };
        let collaboration: ReturnType<typeof useYjsCollaboration> | null = null;

        function CollaborationBoundEditor() {
            collaboration = useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                localAwareness,
                transport: { url: V2_TRANSPORT_URL, connect: true },
            });
            return <NativeRichTextEditor {...collaboration.editorBindings} />;
        }

        const { getByTestId, rerender } = render(<CollaborationBoundEditor />);
        act(() => {
            v2Runtime.transportOpen(handle.editorId);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
        });
        localAwareness = undefined;
        rerender(<CollaborationBoundEditor />);

        expect(v2Runtime.session(handle.editorId).desiredAwareness).toBeNull();
        const awarenessCallsAfterClear =
            mockNativeModule.editorV2CollaborationSetAwareness.mock.calls.length;
        act(() => {
            getByTestId('native-editor-view').props.onSelectionChange({
                nativeEvent: { anchor: 1, head: 3, editorId: handle.editorId },
            });
            getByTestId('native-editor-view').props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });

        expect(mockNativeModule.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(
            awarenessCallsAfterClear
        );
        expect(v2Runtime.session(handle.editorId).desiredAwareness).toBeNull();
        expect(collaboration).not.toBeNull();
        handle.destroy();
    });

    it('drives the toolbar from collapsed stored state without leaking it into explicit mirrors', () => {
        const handle = createV2LocalHandle({
            type: 'doc',
            content: [
                {
                    type: 'paragraph',
                    content: [{ type: 'text', text: 'bold', marks: [{ type: 'bold' }] }],
                },
            ],
        });
        const ref = createRef<NativeRichTextEditorRef>();
        const onActiveStateChange = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onActiveStateChange={onActiveStateChange}
            />
        );

        act(() => {
            ref.current!.toggleMark('bold');
        });
        let view = getByTestId('native-editor-view');
        expect(typeof view.props.editorUpdateJson).toBe('string');
        let pushed = JSON.parse(view.props.editorUpdateJson as string) as {
            activeState: { marks: Record<string, boolean> };
        };
        expect(pushed.activeState.marks.bold).toBe(false);
        expect(view.props.editorUpdateRevision).toBeGreaterThan(0);
        expect(view.props.editorUpdateEditorId).toBe(handle.editorId);
        expect(onActiveStateChange).toHaveBeenCalled();
        expect(
            (onActiveStateChange.mock.calls.at(-1)![0] as { marks: Record<string, boolean> }).marks
                .bold
        ).toBe(false);

        const mirror = handle.bridge.renderUpdate({ anchor: 0, head: 0 });
        expect(mirror.activeState).toMatchObject({
            marks: { bold: true },
            markAttrs: {},
            nodes: {},
        });

        act(() => {
            ref.current!.toggleMark('bold');
        });
        view = getByTestId('native-editor-view');
        pushed = JSON.parse(view.props.editorUpdateJson as string);
        expect(pushed.activeState.marks.bold).toBe(true);

        act(() => {
            ref.current!.setLink('https://example.test/stored');
            ref.current!.toggleHeading(2);
        });
        pushed = JSON.parse(getByTestId('native-editor-view').props.editorUpdateJson as string);
        expect(pushed.activeState).toMatchObject({
            marks: { bold: true, link: true },
            markAttrs: { link: { href: 'https://example.test/stored' } },
            nodes: { 'heading:2': true },
        });
        expect(handle.bridge.renderUpdate({ anchor: 0, head: 0 }).activeState).toMatchObject({
            marks: { bold: true },
            markAttrs: {},
            nodes: {},
        });
        handle.destroy();
    });

    it('drops a pending pushed update when the same editor component rebinds to another handle', () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const ref = createRef<NativeRichTextEditorRef>();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor ref={ref} documentHandle={handleA} />
        );

        act(() => {
            ref.current!.toggleMark('bold');
        });
        expect(getByTestId('native-editor-view').props.editorUpdateEditorId).toBe(handleA.editorId);

        rerender(<NativeRichTextEditor ref={ref} documentHandle={handleB} />);

        const view = getByTestId('native-editor-view');
        expect(view.props.editorId).toBe(handleB.editorId);
        expect(view.props.editorUpdateJson).toBeUndefined();
        expect(view.props.editorUpdateEditorId).toBeUndefined();
        handleA.destroy();
        handleB.destroy();
    });

    it('clears A interaction state before B publishes its authoritative snapshot', () => {
        const handleA = createV2LocalHandle(fakeDocForText('alpha'));
        const handleB = createV2LocalHandle({
            type: 'doc',
            content: [
                {
                    type: 'paragraph',
                    content: [{ type: 'text', text: 'beta', marks: [{ type: 'italic' }] }],
                },
            ],
        });
        const ref = createRef<NativeRichTextEditorRef>();
        const onFocus = jest.fn();
        const onRequestLink = jest.fn();
        const toolbarItems: NonNullable<NativeRichTextEditorProps['toolbarItems']> = [
            { type: 'mark', mark: 'bold', label: 'Bold', icon: 'bold' },
            { type: 'mark', mark: 'italic', label: 'Italic', icon: 'italic' },
        ];
        const { getByLabelText, getByTestId, rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handleA}
                toolbarItems={toolbarItems}
                toolbarPlacement='inline'
                onFocus={onFocus}
                onRequestLink={onRequestLink}
            />
        );

        act(() => {
            ref.current!.toggleMark('bold');
            getByTestId('native-editor-view').props.onSelectionChange({
                nativeEvent: {
                    anchor: 2,
                    head: 2,
                    editorId: handleA.editorId,
                },
            });
            getByTestId('native-editor-view').props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handleA.editorId },
            });
        });
        expect(getByLabelText('Bold').props.accessibilityState.selected).toBe(true);
        expect(onFocus).toHaveBeenCalledTimes(1);

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handleB}
                toolbarItems={toolbarItems}
                toolbarPlacement='inline'
                onFocus={onFocus}
                onRequestLink={onRequestLink}
            />
        );

        const view = getByTestId('native-editor-view');
        expect(getByLabelText('Bold').props.accessibilityState.selected).toBe(false);
        act(() => {
            view.props.onToolbarAction({
                nativeEvent: { key: '__native-editor-link__', editorId: handleB.editorId },
            });
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handleB.editorId },
            });
        });
        expect(onRequestLink.mock.calls.at(-1)![0].selection).toEqual({
            type: 'text',
            anchor: 0,
            head: 0,
        });
        expect(onFocus).toHaveBeenCalledTimes(2);

        act(() => {
            view.props.onEditorUpdate({
                nativeEvent: {
                    editorId: handleB.editorId,
                    updateJson: renderUpdateValue(handleB.editorId, 2, 2),
                    documentRevision: handleB.bridge.getState().documentRevision,
                },
            });
        });

        expect(getByLabelText('Bold').props.accessibilityState.selected).toBe(false);
        expect(getByLabelText('Italic').props.accessibilityState.selected).toBe(true);
        act(() => ref.current!.toggleMark('bold'));
        const request = JSON.parse(
            mockNativeModule.editorV2ApplyCommand.mock.calls.at(-1)![1] as string
        ) as { baseDocumentRevision: string };
        expect(request.baseDocumentRevision).toBe(handleB.bridge.getState().documentRevision);
        handleA.destroy();
        handleB.destroy();
    });

    it('treats B revisions as new observations after rebinding from A', () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const ref = createRef<NativeRichTextEditorRef>();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor ref={ref} documentHandle={handleA} />
        );

        const nativeCommit = JSON.stringify({
            version: 1,
            requestId: '1',
            baseDocumentRevision: handleA.bridge.getState().documentRevision,
            text: '!',
        });
        act(() => {
            v2Runtime.module.editorV2ApplyInput(handleA.editorId, nativeCommit);
            getByTestId('native-editor-view').props.onEditorUpdate({
                nativeEvent: {
                    editorId: handleA.editorId,
                    updateJson: renderUpdateValue(handleA.editorId),
                    documentRevision: handleA.bridge.getState().documentRevision,
                },
            });
        });
        const nativeRevision = handleA.bridge.getState().documentRevision;

        rerender(<NativeRichTextEditor ref={ref} documentHandle={handleB} />);

        // B's initial revision is pulled natively, rather than being treated
        // as an external update under A's observation state.
        expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();

        act(() => {
            handleB.bridge.replaceDocument({ setJson: V2_DOC_C, history: 'undoableBoundary' });
        });
        const externalRevision = handleB.bridge.getState().documentRevision;
        expect(externalRevision).toBe(nativeRevision);

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handleB}
                documentRevision={externalRevision}
            />
        );

        const view = getByTestId('native-editor-view');
        expect(view.props.editorUpdateEditorId).toBe(handleB.editorId);
        expect(JSON.parse(view.props.editorUpdateJson as string)).toMatchObject({
            documentVersion: externalRevision,
        });
        handleA.destroy();
        handleB.destroy();
    });

    it('discards a re-entrant A render entirely after the same editor component rebinds to B', () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const ref = createRef<NativeRichTextEditorRef>();
        const onActiveStateChange = jest.fn();
        const onRequestLink = jest.fn();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handleA}
                onActiveStateChange={onActiveStateChange}
                onRequestLink={onRequestLink}
            />
        );
        act(() => {});
        const renderAUpdate = handleA.bridge.renderUpdate.bind(handleA.bridge);
        const renderUpdate = jest.spyOn(handleA.bridge, 'renderUpdate').mockImplementation(() => {
            act(() => {
                rerender(
                    <NativeRichTextEditor
                        ref={ref}
                        documentHandle={handleB}
                        onActiveStateChange={onActiveStateChange}
                        onRequestLink={onRequestLink}
                    />
                );
            });
            return renderAUpdate();
        });

        onActiveStateChange.mockClear();
        ref.current!.toggleMark('bold');

        const view = getByTestId('native-editor-view');
        expect(view.props.editorId).toBe(handleB.editorId);
        expect(view.props.editorUpdateJson).toBeUndefined();
        expect(view.props.editorUpdateEditorId).toBeUndefined();
        expect(renderUpdate).toHaveBeenCalledTimes(1);
        expect(onActiveStateChange).not.toHaveBeenCalled();

        act(() => {
            view.props.onToolbarAction({
                nativeEvent: { key: '__native-editor-link__', editorId: handleB.editorId },
            });
        });
        expect(onRequestLink).toHaveBeenCalledWith(
            expect.objectContaining({ selection: { type: 'text', anchor: 0, head: 0 } })
        );
        renderUpdate.mockRestore();
        handleA.destroy();
        handleB.destroy();
    });

    it('applies a JS-driven render snapshot locally without parsing its native handoff JSON', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onActiveStateChange = jest.fn();
        const snapshot = Object.freeze({
            renderBlocks: Object.freeze([]),
            renderPatch: null,
            selection: Object.freeze({ type: 'text' as const, anchor: 1, head: 1 }),
            activeState: Object.freeze({
                marks: Object.freeze({ bold: true }),
                markAttrs: Object.freeze({}),
                nodes: Object.freeze({}),
                commands: Object.freeze({}),
                allowedMarks: Object.freeze([]),
                insertableNodes: Object.freeze([]),
            }),
            historyState: Object.freeze({ canUndo: true, canRedo: false }),
            documentVersion: '42',
            stateRevision: '7',
            scalarLength: 5,
            documentIsEmpty: false,
        }) as ReturnType<typeof handle.bridge.renderUpdate>;
        const snapshotJson = JSON.stringify(snapshot);
        const renderUpdate = jest.spyOn(handle.bridge, 'renderUpdate').mockReturnValue(snapshot);
        const parse = jest.spyOn(JSON, 'parse');
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onActiveStateChange={onActiveStateChange}
            />
        );

        act(() => {
            ref.current!.toggleMark('bold');
        });

        expect(getByTestId('native-editor-view').props.editorUpdateJson).toBe(snapshotJson);
        expect(onActiveStateChange.mock.calls.at(-1)![0]).toBe(snapshot.activeState);
        expect(parse).not.toHaveBeenCalledWith(snapshotJson);
        parse.mockRestore();
        renderUpdate.mockRestore();
        handle.destroy();
    });

    it('maps astral scalars, block and atom extents, and selected active state through the fake snapshot', () => {
        const handle = createV2LocalHandle({
            type: 'doc',
            content: [
                {
                    type: 'paragraph',
                    content: [
                        {
                            type: 'text',
                            text: 'A😀',
                            marks: [{ type: 'bold' }],
                        },
                    ],
                },
                {
                    type: 'paragraph',
                    content: [{ type: 'text', text: 'I', marks: [{ type: 'italic' }] }],
                },
                { type: 'image', attrs: { src: 'https://example.test/image.png' } },
                {
                    type: 'paragraph',
                    content: [{ type: 'text', text: 'Z' }],
                },
            ],
        });

        handle.bridge.setSelection({
            baseDocumentRevision: '1',
            selection: {
                type: 'text',
                anchor: { offset: 3, kind: 'scalar' },
                head: { offset: 3, kind: 'scalar' },
            },
        });
        const authoritative = handle.bridge.renderUpdate();
        expect(authoritative.scalarLength).toBe(8);
        expect(authoritative.selection).toEqual({
            type: 'text',
            anchor: 5,
            head: 5,
            anchorScalar: 3,
            headScalar: 3,
        });
        expect(authoritative.activeState.marks).toEqual({ italic: true });

        const selections = [0, 2, 3, 4, 5, 6, 7, 8, 99].map(
            (scalar) => handle.bridge.renderUpdate({ anchor: scalar, head: scalar }).selection
        );
        expect(selections).toEqual([
            { type: 'text', anchor: 1, head: 1, anchorScalar: 0, headScalar: 0 },
            { type: 'text', anchor: 3, head: 3, anchorScalar: 2, headScalar: 2 },
            { type: 'text', anchor: 5, head: 5, anchorScalar: 3, headScalar: 3 },
            { type: 'text', anchor: 6, head: 6, anchorScalar: 4, headScalar: 4 },
            { type: 'text', anchor: 7, head: 7, anchorScalar: 5, headScalar: 5 },
            { type: 'text', anchor: 8, head: 8, anchorScalar: 6, headScalar: 6 },
            { type: 'text', anchor: 9, head: 9, anchorScalar: 7, headScalar: 7 },
            { type: 'text', anchor: 10, head: 10, anchorScalar: 8, headScalar: 8 },
            { type: 'text', anchor: 10, head: 10, anchorScalar: 8, headScalar: 8 },
        ]);

        const astral = handle.bridge.renderUpdate({ anchor: 1, head: 1 });
        expect(astral.selection).toEqual({
            type: 'text',
            anchor: 2,
            head: 2,
            anchorScalar: 1,
            headScalar: 1,
        });
        expect(astral.activeState.marks).toEqual({ bold: true });

        const atom = handle.bridge.renderUpdate({ anchor: 5, head: 5 });
        expect(atom.selection).toEqual({
            type: 'text',
            anchor: 7,
            head: 7,
            anchorScalar: 5,
            headScalar: 5,
        });
        expect(atom.activeState.marks).toEqual({});
        handle.destroy();
    });

    it('maps empty placeholders plus supported inline and block atoms through the fake snapshot', () => {
        const handle = createV2LocalHandle({
            type: 'doc',
            content: [
                { type: 'paragraph' },
                {
                    type: 'paragraph',
                    content: [
                        { type: 'hardBreak' },
                        {
                            type: 'mention',
                            atom: true,
                            attrs: { label: 'Ada', mentionSuggestionChar: '@' },
                        },
                    ],
                },
                { type: 'callout', atom: true, attrs: { label: 'X' } },
            ],
        });

        expect(handle.bridge.renderUpdate().scalarLength).toBe(11);
        expect(
            [0, 1, 2, 3, 6, 7, 8, 9, 11, 99].map(
                (scalar) => handle.bridge.renderUpdate({ anchor: scalar, head: scalar }).selection
            )
        ).toEqual([
            { type: 'text', anchor: 1, head: 1, anchorScalar: 1, headScalar: 1 },
            { type: 'text', anchor: 1, head: 1, anchorScalar: 1, headScalar: 1 },
            { type: 'text', anchor: 3, head: 3, anchorScalar: 2, headScalar: 2 },
            { type: 'text', anchor: 4, head: 4, anchorScalar: 3, headScalar: 3 },
            { type: 'text', anchor: 4, head: 4, anchorScalar: 3, headScalar: 3 },
            { type: 'text', anchor: 5, head: 5, anchorScalar: 7, headScalar: 7 },
            { type: 'text', anchor: 6, head: 6, anchorScalar: 8, headScalar: 8 },
            { type: 'text', anchor: 6, head: 6, anchorScalar: 8, headScalar: 8 },
            { type: 'text', anchor: 7, head: 7, anchorScalar: 11, headScalar: 11 },
            { type: 'text', anchor: 7, head: 7, anchorScalar: 11, headScalar: 11 },
        ]);
        handle.destroy();
    });

    it('suppresses the view update after the editor update revision is exhausted', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onActiveStateChange = jest.fn();
        const errors: unknown[] = [];
        const originalAllocateEditorUpdateRevision =
            EditorUpdateRevision.allocateEditorUpdateRevision;
        const allocateEditorUpdateRevision = jest
            .spyOn(EditorUpdateRevision, 'allocateEditorUpdateRevision')
            .mockImplementation((currentRevision) =>
                currentRevision === 0
                    ? { revision: 0xffff_ffff }
                    : originalAllocateEditorUpdateRevision(currentRevision)
            );
        const renderUpdate = jest.spyOn(handle.bridge, 'renderUpdate');
        handle.addErrorListener((error) => errors.push(error));
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onActiveStateChange={onActiveStateChange}
            />
        );

        act(() => {
            ref.current!.toggleMark('bold');
        });
        const view = getByTestId('native-editor-view');
        const pushedUpdateJson = view.props.editorUpdateJson;
        const pushedUpdateRevision = view.props.editorUpdateRevision;
        expect(pushedUpdateRevision).toBe(0xffff_ffff);

        renderUpdate.mockClear();
        onActiveStateChange.mockClear();
        act(() => {
            ref.current!.toggleMark('bold');
        });

        expect(renderUpdate).not.toHaveBeenCalled();
        expect(getByTestId('native-editor-view').props.editorUpdateJson).toBe(pushedUpdateJson);
        expect(getByTestId('native-editor-view').props.editorUpdateRevision).toBe(
            pushedUpdateRevision
        );
        expect(onActiveStateChange).not.toHaveBeenCalled();
        expect(errors.at(-1)).toBeInstanceOf(NativeEditorV2BoundaryError);
        expect((errors.at(-1) as NativeEditorV2BoundaryError).code).toBe('CONFIG_INVALID');
        allocateEditorUpdateRevision.mockRestore();
        handle.destroy();
    });

    it('renders the inline JS toolbar only in inline placement', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { queryByTestId, rerender } = render(
            <NativeRichTextEditor documentHandle={handle} toolbarPlacement='keyboard' />
        );
        expect(queryByTestId('native-editor-js-toolbar')).toBeNull();
        rerender(<NativeRichTextEditor documentHandle={handle} toolbarPlacement='inline' />);
        expect(queryByTestId('native-editor-js-toolbar')).not.toBeNull();
        handle.destroy();
    });

    it('forwards focused toolbar frames to native without racing a refocus after blur', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const onBlur = jest.fn();
        const viewPrototype = (
            View as unknown as {
                prototype: {
                    measureInWindow: (
                        callback: (x: number, y: number, width: number, height: number) => void
                    ) => void;
                };
            }
        ).prototype;
        const measureInWindow = jest
            .spyOn(viewPrototype, 'measureInWindow')
            .mockImplementation(function (callback) {
                const testID = (this as unknown as { props?: { testID?: string } }).props?.testID;
                if (testID === 'editor-toolbar-root') {
                    callback(12, 24, 320, 48);
                }
            });

        try {
            const { getByTestId } = render(
                <NativeRichTextEditor
                    documentHandle={handle}
                    toolbarPlacement='inline'
                    onBlur={onBlur}
                />
            );
            const nativeView = getByTestId('native-editor-view');

            act(() => {
                nativeView.props.onFocusChange({
                    nativeEvent: { isFocused: true, editorId: handle.editorId },
                });
                jest.runOnlyPendingTimers();
            });

            expect(JSON.parse(getByTestId('native-editor-view').props.toolbarFrameJson)).toEqual({
                x: 12,
                y: 24,
                width: 320,
                height: 48,
            });
            expect(mockNativeFocus).not.toHaveBeenCalled();

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: false, editorId: handle.editorId },
                });
            });

            expect(getByTestId('native-editor-view').props.toolbarFrameJson).toBeUndefined();
            expect(onBlur).toHaveBeenCalledTimes(1);
            expect(mockNativeFocus).not.toHaveBeenCalled();
        } finally {
            measureInWindow.mockRestore();
            handle.destroy();
        }
    });

    it('routes every typing/command ref method through the v2 bridge with the frozen payload', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);

        const lastCommand = (): Record<string, unknown> => {
            const calls = mockNativeModule.editorV2ApplyCommand.mock.calls;
            const request = JSON.parse(calls[calls.length - 1][1] as string) as Record<
                string,
                unknown
            >;
            return request.command as Record<string, unknown>;
        };
        const commandCount = () => mockNativeModule.editorV2ApplyCommand.mock.calls.length;

        act(() => ref.current!.toggleMark('bold'));
        expect(lastCommand()).toEqual({ type: 'toggleMark', markType: 'bold' });

        act(() => ref.current!.setLink('https://example.com'));
        expect(lastCommand()).toEqual({
            type: 'setMark',
            markType: 'link',
            attrs: { href: 'https://example.com' },
        });

        act(() => ref.current!.unsetLink());
        expect(lastCommand()).toEqual({ type: 'unsetMark', markType: 'link' });

        act(() => ref.current!.toggleBlockquote());
        expect(lastCommand()).toEqual({ type: 'toggleBlockquote' });

        act(() => ref.current!.toggleHeading(2));
        expect(lastCommand()).toEqual({ type: 'toggleHeading', level: 2 });

        act(() => ref.current!.toggleList('bulletList'));
        expect(lastCommand()).toEqual({
            type: 'wrapInList',
            listType: 'bulletList',
            itemType: 'listItem',
        });
        // The engine now reports the list active: toggling unwraps.
        act(() => ref.current!.toggleList('bulletList'));
        expect(lastCommand()).toEqual({ type: 'unwrapFromList' });

        act(() => ref.current!.indentListItem());
        expect(lastCommand()).toEqual({ type: 'indentListItem' });

        act(() => ref.current!.outdentListItem());
        expect(lastCommand()).toEqual({ type: 'outdentListItem' });

        act(() => ref.current!.insertNode('horizontalRule'));
        expect(lastCommand()).toEqual({ type: 'insertNode', nodeType: 'horizontalRule' });

        act(() => ref.current!.insertImage('https://example.com/a.png'));
        expect(lastCommand()).toEqual({
            type: 'insertContentJson',
            json: {
                type: 'doc',
                content: [{ type: 'image', attrs: { src: 'https://example.com/a.png' } }],
            },
        });

        act(() => ref.current!.insertContentHtml('<p>hi</p>'));
        expect(lastCommand()).toEqual({ type: 'insertContentHtml', html: '<p>hi</p>' });

        const fragment = { type: 'doc', content: [{ type: 'paragraph' }] };
        act(() => ref.current!.insertContentJson(fragment));
        expect(lastCommand()).toEqual({ type: 'insertContentJson', json: fragment });

        // Every command carried the rendered engine revision as its base.
        const bases = mockNativeModule.editorV2ApplyCommand.mock.calls.map((call) =>
            String((JSON.parse(call[1] as string) as Record<string, unknown>).baseDocumentRevision)
        );
        expect(bases.every((base) => base !== 'undefined' && base !== 'null')).toBe(true);

        const commandsBefore = commandCount();
        act(() => ref.current!.insertText('abc'));
        expect(commandCount()).toBe(commandsBefore);
        const inputCalls = mockNativeModule.editorV2ApplyInput.mock.calls;
        const inputRequest = JSON.parse(inputCalls[inputCalls.length - 1][1] as string) as Record<
            string,
            unknown
        >;
        expect(inputRequest.text).toBe('abc');
        handle.destroy();
    });

    it('propagates structured error envelopes from ref commands', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);
        v2Runtime.injectNextApplyCommandError(handle.editorId, {
            domain: 'operation',
            code: 'POSITION_INVALID',
            message: 'selection is outside the document',
            requestId: null,
            operationIndex: null,
            limit: null,
            actual: null,
            details: null,
        });
        let thrown: unknown;
        act(() => {
            try {
                ref.current!.toggleMark('bold');
            } catch (error) {
                thrown = error;
            }
        });
        expect(thrown).toBeInstanceOf(NativeEditorV2OperationError);
        expect((thrown as NativeEditorV2OperationError).code).toBe('POSITION_INVALID');
        handle.destroy();
    });

    it('refreshes from the engine on REVISION_MISMATCH and never retries the command', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);
        mockNativeModule.editorV2ApplyInput.mockClear();

        // The engine advances behind the component's back (rev 1 -> 2).
        handle.bridge.replaceDocument({ setJson: V2_DOC_B, history: 'undoableBoundary' });

        act(() => {
            ref.current!.insertText('abc');
        });
        // Stale base rejected; the component refreshed and did not retry.
        expect(mockNativeModule.editorV2ApplyInput).toHaveBeenCalledTimes(1);
        expect(ref.current!.getContentJson()).toEqual(V2_DOC_B);
        handle.destroy();
    });

    it('rejects controlled valueJSON replace while the collaboration transport is connected', () => {
        const handle = createV2RoomHandle({ withSnapshot: true });
        const { controller } = setupV2Controller(handle);
        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='replace'
            />
        );
        act(() => {
            controller.connect();
        });
        act(() => {
            v2Runtime.transportOpen(handle.editorId);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
        });
        expect(controller.state.status).toBe('synchronized');

        let thrown: unknown;
        try {
            rerender(
                <NativeRichTextEditor
                    ref={ref}
                    documentHandle={handle}
                    documentRevision={controller.state.documentRevision}
                    valueJSON={V2_DOC_B}
                    valueJSONUpdateMode='replace'
                />
            );
        } catch (error) {
            thrown = error;
        }
        expect((thrown as { code?: string })?.code).toBe('WHOLE_DOCUMENT_REPLACEMENT_CONNECTED');
        handle.destroy();
    });

    it('rejects controlled valueJSON reset while the collaboration transport is connected', () => {
        const handle = createV2RoomHandle({ withSnapshot: true });
        const { controller } = setupV2Controller(handle);
        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='reset'
            />
        );
        act(() => {
            controller.connect();
        });
        act(() => {
            v2Runtime.transportOpen(handle.editorId);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
        });
        expect(controller.state.status).toBe('synchronized');

        let thrown: unknown;
        try {
            rerender(
                <NativeRichTextEditor
                    ref={ref}
                    documentHandle={handle}
                    documentRevision={controller.state.documentRevision}
                    valueJSON={V2_DOC_C}
                    valueJSONUpdateMode='reset'
                />
            );
        } catch (error) {
            thrown = error;
        }
        expect((thrown as { code?: string })?.code).toBe('WHOLE_DOCUMENT_REPLACEMENT_CONNECTED');
        handle.destroy();
    });

    it('read-only mode blocks every mutation ref method but keeps selection and controlled content', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onSelectionChange = jest.fn();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                editable={false}
                onSelectionChange={onSelectionChange}
            />
        );

        const mutations: Array<() => void> = [
            () => ref.current!.toggleMark('bold'),
            () => ref.current!.setLink('https://example.com'),
            () => ref.current!.unsetLink(),
            () => ref.current!.toggleBlockquote(),
            () => ref.current!.toggleHeading(1),
            () => ref.current!.toggleList('bulletList'),
            () => ref.current!.indentListItem(),
            () => ref.current!.outdentListItem(),
            () => ref.current!.insertNode('horizontalRule'),
            () => ref.current!.insertImage('https://example.com/a.png'),
            () => ref.current!.insertText('x'),
            () => ref.current!.insertContentHtml('<p>x</p>'),
            () => ref.current!.insertContentJson({ type: 'doc', content: [] }),
        ];
        for (const mutation of mutations) {
            let thrown: unknown;
            act(() => {
                try {
                    mutation();
                } catch (error) {
                    thrown = error;
                }
            });
            expect(thrown).toBeInstanceOf(NativeEditorV2OperationError);
            expect((thrown as NativeEditorV2OperationError).code).toBe('MUTATION_REJECTED');
        }
        expect(mockNativeModule.editorV2ApplyCommand).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ReplaceDocument).not.toHaveBeenCalled();

        // Selection still flows.
        act(() => {
            getByTestId('native-editor-view').props.onSelectionChange({
                nativeEvent: { anchor: 0, head: 2, editorId: handle.editorId },
            });
        });
        expect(onSelectionChange).toHaveBeenCalledWith({ type: 'text', anchor: 0, head: 2 });

        // Controlled API content still passes under read-only (native parity).
        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                editable={false}
                valueJSON={V2_DOC_B}
            />
        );
        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(1);
        handle.destroy();
    });

    it('never destroys the shared handle on unmount and drops everything after destroy', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onContentChange = jest.fn();
        const { unmount, getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onContentChange={onContentChange}
            />
        );
        const view = getByTestId('native-editor-view');

        // Destroying is the consumer's call. While mounted, every surface
        // degrades to the structured destroyed contract.
        handle.destroy();
        expect(mockNativeModule.editorV2Destroy).toHaveBeenCalledTimes(1);
        expect(ref.current!.getContent()).toBe('');
        let thrown: unknown;
        try {
            ref.current!.toggleMark('bold');
        } catch (error) {
            thrown = error;
        }
        expect(thrown).toBeInstanceOf(NativeEditorV2NonRetryableError);
        expect((thrown as NativeEditorV2NonRetryableError).code).toBe('ENGINE_DESTROYED');

        // Events arriving for the destroyed session are dropped.
        onContentChange.mockClear();
        act(() => {
            view.props.onEditorUpdate({
                nativeEvent: { editorId: handle.editorId, updateJson: '{}' },
            });
        });
        expect(onContentChange).not.toHaveBeenCalled();

        // Unmount never destroys the shared handle a second time.
        unmount();
        expect(mockNativeModule.editorV2Destroy).toHaveBeenCalledTimes(1);
    });

    it('routes toolbar link/image actions through the request props and custom keys to onToolbarAction', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onRequestLink = jest.fn();
        const onRequestImage = jest.fn();
        const onToolbarAction = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onRequestLink={onRequestLink}
                onRequestImage={onRequestImage}
                onToolbarAction={onToolbarAction}
            />
        );

        act(() => {
            getByTestId('native-editor-view').props.onToolbarAction({
                nativeEvent: { key: '__native-editor-link__', editorId: handle.editorId },
            });
        });
        expect(onRequestLink).toHaveBeenCalledTimes(1);
        const linkContext = onRequestLink.mock.calls[0][0] as {
            isActive: boolean;
            setLink: (href: string) => void;
            unsetLink: () => void;
        };
        expect(linkContext.isActive).toBe(false);
        act(() => {
            linkContext.setLink('https://example.com');
        });
        let calls = mockNativeModule.editorV2ApplyCommand.mock.calls;
        expect(
            (JSON.parse(calls[calls.length - 1][1] as string) as Record<string, unknown>).command
        ).toEqual({
            type: 'setMark',
            markType: 'link',
            attrs: { href: 'https://example.com' },
        });

        act(() => {
            getByTestId('native-editor-view').props.onToolbarAction({
                nativeEvent: { key: '__native-editor-image__', editorId: handle.editorId },
            });
        });
        expect(onRequestImage).toHaveBeenCalledTimes(1);
        const imageContext = onRequestImage.mock.calls[0][0] as {
            insertImage: (src: string) => void;
        };
        act(() => {
            imageContext.insertImage('https://example.com/b.png');
        });
        calls = mockNativeModule.editorV2ApplyCommand.mock.calls;
        expect(
            (JSON.parse(calls[calls.length - 1][1] as string) as Record<string, unknown>).command
        ).toEqual({
            type: 'insertContentJson',
            json: {
                type: 'doc',
                content: [{ type: 'image', attrs: { src: 'https://example.com/b.png' } }],
            },
        });

        act(() => {
            getByTestId('native-editor-view').props.onToolbarAction({
                nativeEvent: { key: 'action:custom:0', editorId: handle.editorId },
            });
        });
        expect(onToolbarAction).toHaveBeenCalledWith('action:custom:0');
        handle.destroy();
    });

    it('resolves mention styling with active mark attrs and inserts the resolved mention', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: withMentionsSchema(tiptapSchema),
            initialization: {
                type: 'localJson',
                json: {
                    type: 'doc',
                    content: [
                        {
                            type: 'paragraph',
                            content: [
                                {
                                    type: 'text',
                                    text: '@al',
                                    marks: [
                                        {
                                            type: 'link',
                                            attrs: { href: 'https://example.test/alice' },
                                        },
                                    ],
                                },
                            ],
                        },
                    ],
                },
            },
        });
        const resolveSelectionAttrs = jest.fn(() => ({ kind: 'user' }));
        const resolveTheme = jest.fn(() => ({ node: { textColor: '#445566' } }));
        const onSelect = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                addons={{
                    mentions: {
                        suggestions: [
                            {
                                key: 'alice',
                                title: 'Alice',
                                attrs: { id: 'user-alice' },
                            },
                        ],
                        resolveSelectionAttrs,
                        resolveTheme,
                        onSelect,
                    },
                }}
            />
        );
        const documentVersion = handle.bridge.getState().documentRevision;

        act(() => {
            getByTestId('native-editor-view').props.onAddonEvent({
                nativeEvent: {
                    editorId: handle.editorId,
                    eventJson: JSON.stringify({
                        type: 'mentionsSelectRequest',
                        trigger: '@',
                        suggestionKey: 'alice',
                        attrs: {
                            id: 'user-alice',
                            label: 'Alice',
                            mentionSuggestionChar: '@',
                        },
                        range: { anchor: 0, head: 3 },
                        documentVersion,
                    }),
                },
            });
        });

        expect(resolveSelectionAttrs).toHaveBeenCalledWith(
            expect.objectContaining({
                attrs: {
                    id: 'user-alice',
                    label: 'Alice',
                    mentionSuggestionChar: '@',
                },
                markAttrs: {
                    link: { href: 'https://example.test/alice' },
                },
            })
        );
        expect(resolveTheme).toHaveBeenCalledWith(
            expect.objectContaining({
                attrs: {
                    id: 'user-alice',
                    label: 'Alice',
                    mentionSuggestionChar: '@',
                    kind: 'user',
                },
                markAttrs: {
                    link: { href: 'https://example.test/alice' },
                },
            })
        );
        const applyCommandCalls = mockNativeModule.editorV2ApplyCommand.mock.calls;
        expect(
            (
                JSON.parse(applyCommandCalls[applyCommandCalls.length - 1][1] as string) as Record<
                    string,
                    unknown
                >
            ).command
        ).toEqual({
            type: 'insertContentJson',
            json: {
                type: 'doc',
                content: [
                    {
                        type: 'mention',
                        attrs: {
                            id: 'user-alice',
                            label: 'Alice',
                            mentionSuggestionChar: '@',
                            kind: 'user',
                            mentionTheme: { node: { textColor: '#445566' } },
                        },
                    },
                    { type: 'text', text: ' ' },
                ],
            },
        });
        expect(onSelect).toHaveBeenCalledWith(
            expect.objectContaining({
                attrs: expect.objectContaining({
                    id: 'user-alice',
                    kind: 'user',
                    mentionTheme: { node: { textColor: '#445566' } },
                }),
            })
        );
        handle.destroy();
    });

    it('feeds the inline toolbar mention suggestions while a query is active', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: withMentionsSchema(tiptapSchema),
            initialization: {
                type: 'localJson',
                json: {
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: '@al' }] }],
                },
            },
        });
        const onSelect = jest.fn();
        const { getByTestId, queryByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                toolbarPlacement='inline'
                addons={{
                    mentions: {
                        suggestions: [
                            { key: 'alice', title: 'Alice', attrs: { id: 'user-alice' } },
                        ],
                        onSelect,
                    },
                }}
            />
        );
        const view = getByTestId('native-editor-view');

        act(() => {
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });

        expect(queryByTestId('editor-toolbar-mention-suggestions')).toBeNull();

        const emitQueryChange = (isActive: boolean) => {
            act(() => {
                view.props.onAddonEvent({
                    nativeEvent: {
                        editorId: handle.editorId,
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'al',
                            trigger: '@',
                            range: { anchor: 0, head: 3 },
                            isActive,
                            documentVersion: handle.bridge.getState().documentRevision,
                        }),
                    },
                });
            });
        };

        emitQueryChange(true);

        expect(queryByTestId('editor-toolbar-mention-suggestions')).not.toBeNull();

        act(() => {
            fireEvent.press(getByTestId('editor-toolbar-mention-suggestion-alice'));
        });

        // Native adapter parity: a range selection uses Before affinity, in
        // scalar currency. Omitting affinity defaults to After, which Yrs
        // cannot represent for a range.
        const setSelectionCalls = mockNativeModule.editorV2SetSelection.mock.calls;
        expect(
            (
                JSON.parse(setSelectionCalls[setSelectionCalls.length - 1][1] as string) as Record<
                    string,
                    unknown
                >
            ).selection
        ).toEqual({
            type: 'text',
            anchor: { offset: 0, kind: 'scalar', affinity: 'before' },
            head: { offset: 3, kind: 'scalar', affinity: 'before' },
        });

        const applyCommandCalls = mockNativeModule.editorV2ApplyCommand.mock.calls;
        expect(
            (
                JSON.parse(applyCommandCalls[applyCommandCalls.length - 1][1] as string) as Record<
                    string,
                    unknown
                >
            ).command
        ).toEqual({
            type: 'insertContentJson',
            json: {
                type: 'doc',
                content: [
                    {
                        type: 'mention',
                        attrs: {
                            id: 'user-alice',
                            label: 'Alice',
                            mentionSuggestionChar: '@',
                        },
                    },
                    { type: 'text', text: ' ' },
                ],
            },
        });
        expect(onSelect).toHaveBeenCalledWith(
            expect.objectContaining({
                trigger: '@',
                suggestion: expect.objectContaining({ key: 'alice' }),
            })
        );
        expect(queryByTestId('editor-toolbar-mention-suggestions')).toBeNull();

        emitQueryChange(true);
        expect(queryByTestId('editor-toolbar-mention-suggestions')).not.toBeNull();
        emitQueryChange(false);
        expect(queryByTestId('editor-toolbar-mention-suggestions')).toBeNull();

        handle.destroy();
    });

    it('never writes an unrenderable mention theme into the document', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: withMentionsSchema(tiptapSchema),
            initialization: {
                type: 'localJson',
                json: {
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: '@al' }] }],
                },
            },
        });
        const consoleError = jest.spyOn(console, 'error').mockImplementation(() => {});
        const { getByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                toolbarPlacement='inline'
                addons={{
                    mentions: {
                        suggestions: [
                            { key: 'alice', title: 'Alice', attrs: { id: 'user-alice' } },
                        ],
                        // The pre-1.0 flat shape, no longer part of EditorMentionTheme.
                        resolveTheme: () => ({ textColor: '#CC0000' }) as never,
                    },
                }}
            />
        );
        const view = getByTestId('native-editor-view');

        act(() => {
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });
        act(() => {
            view.props.onAddonEvent({
                nativeEvent: {
                    editorId: handle.editorId,
                    eventJson: JSON.stringify({
                        type: 'mentionsQueryChange',
                        query: 'al',
                        trigger: '@',
                        range: { anchor: 0, head: 3 },
                        isActive: true,
                        documentVersion: handle.bridge.getState().documentRevision,
                    }),
                },
            });
        });
        act(() => {
            fireEvent.press(getByTestId('editor-toolbar-mention-suggestion-alice'));
        });

        // The document must stay renderable: a rejected theme is dropped, not
        // persisted into content that every later renderUpdate revalidates.
        expect(() => handle.bridge.renderUpdate()).not.toThrow();
        const inserted = mockNativeModule.editorV2ApplyCommand.mock.calls
            .map((call) => JSON.parse(call[1] as string) as Record<string, unknown>)
            .filter(
                (request) =>
                    (request.command as Record<string, unknown>)?.type === 'insertContentJson'
            );
        expect(inserted.length).toBeGreaterThan(0);
        const attrs = (
            (
                (inserted[inserted.length - 1].command as Record<string, unknown>).json as Record<
                    string,
                    Record<string, unknown>[]
                >
            ).content[0] as Record<string, Record<string, unknown>>
        ).attrs;
        expect(attrs.mentionTheme).toBeUndefined();
        expect(consoleError).toHaveBeenCalledWith(
            expect.stringContaining('mentions.resolveTheme'),
            expect.anything()
        );

        consoleError.mockRestore();
        handle.destroy();
    });

    it('falls back to Before affinity when a collapsed mention caret is unrepresentable', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: withMentionsSchema(tiptapSchema),
            initialization: {
                type: 'localJson',
                json: {
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: '@' }] }],
                },
            },
        });
        const onSelect = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                toolbarPlacement='inline'
                addons={{
                    mentions: {
                        suggestions: [
                            { key: 'alice', title: 'Alice', attrs: { id: 'user-alice' } },
                        ],
                        onSelect,
                    },
                }}
            />
        );
        const view = getByTestId('native-editor-view');

        act(() => {
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });
        act(() => {
            view.props.onAddonEvent({
                nativeEvent: {
                    editorId: handle.editorId,
                    eventJson: JSON.stringify({
                        type: 'mentionsQueryChange',
                        query: '',
                        trigger: '@',
                        range: { anchor: 1, head: 1 },
                        isActive: true,
                        documentVersion: handle.bridge.getState().documentRevision,
                    }),
                },
            });
        });

        // The engine rejects After stickiness at this boundary position.
        mockNativeModule.editorV2SetSelection.mockImplementationOnce(() => ({
            value: null,
            error: {
                domain: 'operation',
                code: 'POSITION_INVALID',
                message: 'selection cannot be represented with the requested Yrs affinity',
                requestId: null,
                operationIndex: null,
                limit: null,
                actual: null,
                details: null,
            },
        }));

        act(() => {
            fireEvent.press(getByTestId('editor-toolbar-mention-suggestion-alice'));
        });

        const calls = mockNativeModule.editorV2SetSelection.mock.calls;
        const affinities = calls
            .slice(-2)
            .map(
                (call) =>
                    (
                        (JSON.parse(call[1] as string) as Record<string, unknown>)
                            .selection as Record<string, Record<string, unknown>>
                    ).anchor.affinity
            );
        expect(affinities).toEqual(['after', 'before']);
        expect(onSelect).toHaveBeenCalledWith(
            expect.objectContaining({ suggestion: expect.objectContaining({ key: 'alice' }) })
        );

        handle.destroy();
    });

    it('applies per-suggestion resolveTheme styling to inline toolbar suggestions', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: withMentionsSchema(tiptapSchema),
            initialization: {
                type: 'localJson',
                json: {
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: '@a' }] }],
                },
            },
        });
        const { getByTestId, getByText } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                toolbarPlacement='inline'
                addons={{
                    mentions: {
                        suggestions: [
                            { key: 'channel', title: 'General' },
                            { key: 'alice', title: 'Alice' },
                        ],
                        resolveSelectionAttrs: ({ suggestion, attrs }) => ({
                            ...attrs,
                            type: suggestion.key === 'channel' ? 'channel' : 'user',
                        }),
                        resolveTheme: ({ attrs }) =>
                            attrs.type === 'channel'
                                ? {
                                      node: { textColor: '#00FF00', backgroundColor: '#00FF00' },
                                      suggestions: {
                                          option: {
                                              textColor: '#CC0000',
                                              backgroundColor: '#FFEEEE',
                                          },
                                      },
                                  }
                                : {
                                      suggestions: {
                                          option: {
                                              textColor: '#0000CC',
                                              backgroundColor: '#EEEEFF',
                                          },
                                      },
                                  },
                    },
                }}
            />
        );
        const view = getByTestId('native-editor-view');

        act(() => {
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });
        act(() => {
            view.props.onAddonEvent({
                nativeEvent: {
                    editorId: handle.editorId,
                    eventJson: JSON.stringify({
                        type: 'mentionsQueryChange',
                        query: 'a',
                        trigger: '@',
                        range: { anchor: 0, head: 2 },
                        isActive: true,
                        documentVersion: handle.bridge.getState().documentRevision,
                    }),
                },
            });
        });

        const flattenStyle = (element: { props: { style?: unknown } }) =>
            StyleSheet.flatten(element.props.style) as Record<string, unknown>;

        expect(
            flattenStyle(getByTestId('editor-toolbar-mention-suggestion-channel')).backgroundColor
        ).toBe('#FFEEEE');
        expect(
            flattenStyle(getByTestId('editor-toolbar-mention-suggestion-alice')).backgroundColor
        ).toBe('#EEEEFF');
        expect(flattenStyle(getByText('@General')).color).toBe('#CC0000');
        expect(flattenStyle(getByText('@Alice')).color).toBe('#0000CC');

        handle.destroy();
    });
});
