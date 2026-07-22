// ─── NativeRichTextEditor Tests (v2 document mode) ─────────────
// Tests for the React component wrapper around the native view in its only
// construction mode: a shared v2 document session (NativeEditorDocumentHandle).
// Both the native module and native view manager are mocked.
//
// Tests cover:
// - The retained document API through the v2 handle (set/get/clear/undo)
// - Controlled valueJSON history policy (replace vs reset)
// - REVISION_MISMATCH refresh and re-apply
// - Room editors: loading until an accepted server Step 2, no client state
// - Remote commits re-rendered from the engine (no TS-side document push)
// - Offline edits queued in the engine and drained on reconnect
// - Native view wiring (props passthrough, focus/blur, getCaretRect)
// ────────────────────────────────────────────────────────────────

// ─── Mock Setup (must be before imports) ────────────────────────

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

// ─── Imports (after mock setup) ─────────────────────────────────

import React, { createRef } from 'react';
import { render, act } from '@testing-library/react-native';

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
import { createYjsCollaborationController } from '../YjsCollaboration';
import {
    createFakeNativeEditorV2Runtime,
    fakeDocForText,
    V2_FAKE_STEP1_FRAME,
    V2_FAKE_STEP2_FRAME,
    V2_FAKE_UPDATE_FRAME,
    type FakeNativeEditorV2Runtime,
} from './helpers/nativeEditorV2Fake';
import type { SchemaDefinition } from '../schemas';

const mockResolveDocumentDescriptor = require('../schemas')
    .resolveDocumentDescriptor as jest.Mock;

const HANDLE_OWNED_ARTICLE_SCHEMA: SchemaDefinition = {
    nodes: [
        { name: 'article', content: '(title | image)+', role: 'doc' },
        { name: 'title', content: 'inline*', group: 'block', role: 'textBlock' },
        { name: 'image', content: '', group: 'block', role: 'block', attrs: { src: {} } },
        { name: 'text', content: '', group: 'inline', role: 'text' },
    ],
    marks: [],
};

// ─── Tests ──────────────────────────────────────────────────────

class V2MockWebSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;

    readyState = V2MockWebSocket.CONNECTING;
    binaryType?: string;
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: unknown }) => void) | null = null;
    onerror: (() => void) | null = null;
    onclose: ((event?: { code?: number; reason?: string }) => void) | null = null;
    send = jest.fn();
    close = jest.fn(() => {
        this.readyState = V2MockWebSocket.CLOSED;
        this.onclose?.({ code: 1000, reason: '' });
    });

    open(): void {
        this.readyState = V2MockWebSocket.OPEN;
        this.onopen?.();
    }

    receive(bytes: Uint8Array): void {
        this.onmessage?.({ data: bytes.slice().buffer });
    }

    serverClose(code: number, reason = ''): void {
        this.readyState = V2MockWebSocket.CLOSED;
        this.onclose?.({ code, reason });
    }
}

describe('NativeRichTextEditor (v2 document mode)', () => {
    const OriginalWebSocket = global.WebSocket;
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

    function setupV2Controller(handle: NativeEditorDocumentHandle) {
        const sockets: V2MockWebSocket[] = [];
        const controller = createYjsCollaborationController({
            documentId: 'doc-1',
            handle,
            connect: false,
            createWebSocket: () => {
                const socket = new V2MockWebSocket();
                sockets.push(socket);
                return socket as unknown as WebSocket;
            },
        });
        return { controller, sockets };
    }

    function v2SentFrames(socket: V2MockWebSocket): number[][] {
        return socket.send.mock.calls.map((call) =>
            Array.from(new Uint8Array(call[0] as ArrayBuffer))
        );
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
        global.WebSocket = V2MockWebSocket as unknown as typeof WebSocket;
    });

    afterEach(() => {
        jest.useRealTimers();
        global.WebSocket = OriginalWebSocket;
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
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSONUpdateMode='replace'
            />
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
        const { controller, sockets } = setupV2Controller(handle);
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
            sockets[0].open();
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
            sockets[0].receive(V2_FAKE_STEP2_FRAME);
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
        const { controller, sockets } = setupV2Controller(handle);
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
            sockets[0].open();
            sockets[0].receive(V2_FAKE_STEP2_FRAME);
        });
        expect(controller.state.status).toBe('synchronized');
        mockNativeModule.editorV2ApplyLocalApi.mockClear();
        mockNativeModule.editorV2ReplaceDocument.mockClear();
        mockNativeModule.editorV2ApplyInput.mockClear();

        act(() => {
            v2Runtime.pushRemoteDoc(handle.editorId, V2_SERVER_UPDATE_DOC);
            sockets[0].receive(V2_FAKE_UPDATE_FRAME);
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

    it('queues offline local edits in the engine and drains them in order after reconnect', () => {
        const handle = createV2RoomHandle({ withSnapshot: true });
        const { controller, sockets } = setupV2Controller(handle);
        const ref = createRef<NativeRichTextEditorRef>();
        render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
                onLocalDocumentCommit={() => controller.handleLocalCommit()}
            />
        );

        act(() => {
            controller.connect();
        });
        act(() => {
            sockets[0].open();
            sockets[0].receive(V2_FAKE_STEP2_FRAME);
        });
        expect(controller.state.status).toBe('synchronized');

        act(() => {
            sockets[0].serverClose(1006);
        });
        expect(controller.state.status).toBe('disconnected');

        // Offline local edits through the editor queue in the engine.
        act(() => {
            ref.current!.setContentJson(V2_DOC_B);
        });
        act(() => {
            ref.current!.setContentJson(V2_DOC_C);
        });
        expect(v2Runtime.queuedFrames(handle.editorId)).toHaveLength(2);

        // The retry timer reconnects; on open the take-outbound loop
        // delivers Step 1, then the queued document frames in order.
        act(() => {
            jest.advanceTimersByTime(500);
        });
        expect(sockets).toHaveLength(2);
        act(() => {
            sockets[1].open();
        });
        expect(v2SentFrames(sockets[1])).toEqual([
            Array.from(V2_FAKE_STEP1_FRAME),
            [0x64, 8],
            [0x64, 9],
        ]);

        // Back online, an engine mutation flushes through the commit ping.
        act(() => {
            sockets[1].receive(V2_FAKE_STEP2_FRAME);
        });
        expect(controller.state.status).toBe('synchronized');
        act(() => {
            ref.current!.undo();
        });
        expect(v2SentFrames(sockets[1])).toEqual([
            Array.from(V2_FAKE_STEP1_FRAME),
            [0x64, 8],
            [0x64, 9],
            [0x64, 10],
        ]);
        expect(ref.current!.getContentJson()).toEqual(V2_DOC_B);
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
            <NativeRichTextEditor
                documentHandle={handle}
                remoteSelections={remoteSelections}
            />
        );
        expect(getByTestId('native-editor-view').props.remoteSelectionsJson).toBe(
            JSON.stringify(remoteSelections)
        );
        handle.destroy();
    });

    // ── Native view wiring (ported from the legacy suite) ──────

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
        expect(rect).toEqual({ x: 1, y: 2, width: 3, height: 4, editorWidth: 100, editorHeight: 50 });

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

    // ── Interactive binding (Task 18B) ────────────────────────

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
        // The native side marked the session live for view binding at create.
        expect(v2Runtime.liveEditorIds()).toContain(handle.editorId);
        handle.destroy();
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
        const onLocalDocumentCommit = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onContentChange={onContentChange}
                onLocalDocumentCommit={onLocalDocumentCommit}
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
            commitOutcome = v2Runtime.module.editorV2ApplyInput(
                handle.editorId,
                commitRequest
            ) as { value: string };
        });
        expect(JSON.parse(commitOutcome!.value)).toMatchObject({ type: 'transaction' });

        act(() => {
            getByTestId('native-editor-view').props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: renderUpdateValue(handle.editorId),
                    documentVersion: handle.bridge.getState().documentRevision,
                },
            });
        });

        // The component issued no mutation of its own: exactly one typed
        // transaction reached the engine (the adapter's).
        expect(mockNativeModule.editorV2ApplyInput).toHaveBeenCalledTimes(1);
        expect(mockNativeModule.editorV2ApplyCommand).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ReplaceDocument).not.toHaveBeenCalled();
        // Content callbacks and the collaboration flush ping fired once.
        expect(onContentChange).toHaveBeenCalledTimes(1);
        expect(onContentChange).toHaveBeenCalledWith('<p>hello!</p>');
        expect(onLocalDocumentCommit).toHaveBeenCalledTimes(1);
        // No echo: the view already applied the adapter's update natively.
        expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();
        expect(ref.current!.getContent()).toBe('<p>hello!</p>');
        handle.destroy();
    });

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
        expect(onSelectionChange).toHaveBeenCalledWith({ type: 'text', anchor: 1, head: 3 });

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

    it('drives the toolbar enabled state from the engine active state after a command', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
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
        expect(pushed.activeState.marks.bold).toBe(true);
        expect(view.props.editorUpdateRevision).toBeGreaterThan(0);
        expect(view.props.editorUpdateEditorId).toBe(handle.editorId);
        expect(onActiveStateChange).toHaveBeenCalled();
        expect(
            (onActiveStateChange.mock.calls.at(-1)![0] as { marks: Record<string, boolean> })
                .marks.bold
        ).toBe(true);

        act(() => {
            ref.current!.toggleMark('bold');
        });
        view = getByTestId('native-editor-view');
        pushed = JSON.parse(view.props.editorUpdateJson as string);
        expect(pushed.activeState.marks.bold).toBe(false);
        handle.destroy();
    });

    it('suppresses the view update after the editor update revision is exhausted', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onActiveStateChange = jest.fn();
        const errors: unknown[] = [];
        const allocateEditorUpdateRevision = jest
            .spyOn(EditorUpdateRevision, 'allocateEditorUpdateRevision')
            .mockImplementation((currentRevision) =>
                currentRevision === 0
                    ? { revision: 0xffff_ffff }
                    : jest
                          .requireActual<typeof import('../EditorUpdateRevision')>(
                              '../EditorUpdateRevision'
                          )
                          .allocateEditorUpdateRevision(currentRevision)
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
            String(
                (JSON.parse(call[1] as string) as Record<string, unknown>).baseDocumentRevision
            )
        );
        expect(bases.every((base) => base !== 'undefined' && base !== 'null')).toBe(true);

        const commandsBefore = commandCount();
        act(() => ref.current!.insertText('abc'));
        expect(commandCount()).toBe(commandsBefore);
        const inputCalls = mockNativeModule.editorV2ApplyInput.mock.calls;
        const inputRequest = JSON.parse(
            inputCalls[inputCalls.length - 1][1] as string
        ) as Record<string, unknown>;
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
        const { controller, sockets } = setupV2Controller(handle);
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
            sockets[0].open();
            sockets[0].receive(V2_FAKE_STEP2_FRAME);
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
        const { controller, sockets } = setupV2Controller(handle);
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
            sockets[0].open();
            sockets[0].receive(V2_FAKE_STEP2_FRAME);
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
});
