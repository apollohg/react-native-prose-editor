// Shared v2 document session, the only construction mode. Native module and
// view manager are both mocked.

export const mockNativeFocus = jest.fn();

export const mockNativeBlur = jest.fn();

export const mockNativeGetCaretRect = jest.fn();

export const mockNativeBeginExternalComposition = jest.fn();

export const mockNativeUpdateExternalComposition = jest.fn();

export const mockNativeCommitExternalComposition = jest.fn();

export const mockNativeCancelExternalComposition = jest.fn();

export const mockNativeViewRender = jest.fn();

export let mockExternalCompositionSupported = true;

export const mockNativeModule: Record<string, jest.Mock> = {};

jest.mock('expo-modules-core', () => {
    const React = require('react');
    const { View } = require('react-native');

    const MockNativeView = React.forwardRef(
        (props: Record<string, unknown>, ref: React.Ref<unknown>) => {
            mockNativeViewRender();
            React.useImperativeHandle(
                ref,
                () => ({
                    focus: mockNativeFocus,
                    blur: mockNativeBlur,
                    getCaretRect: mockNativeGetCaretRect,
                    beginExternalTextComposition: mockNativeBeginExternalComposition,
                    updateExternalTextComposition: mockNativeUpdateExternalComposition,
                    commitExternalTextComposition: mockNativeCommitExternalComposition,
                    ...(mockExternalCompositionSupported
                        ? { cancelExternalTextComposition: mockNativeCancelExternalComposition }
                        : {}),
                }),
                []
            );
            return React.createElement(View, { testID: 'native-editor-view', ...props });
        }
    );
    MockNativeView.displayName = 'MockNativeView';

    return {
        requireNativeModule: () => mockNativeModule,
        requireNativeViewManager: () => MockNativeView,
    };
});

jest.mock('../../schemas', () => {
    const actual = jest.requireActual('../../schemas');
    const mockResolveDocumentDescriptor = jest.fn(actual.resolveDocumentDescriptor);
    return {
        ...actual,
        resolveDocumentDescriptor: mockResolveDocumentDescriptor,
    };
});

import React, { createRef, StrictMode } from 'react';

import { Platform, StyleSheet, View } from 'react-native';

import { render, act, fireEvent } from '@testing-library/react-native';

import {
    NativeRichTextEditor,
    type NativeRichTextEditorProps,
    type NativeRichTextEditorRef,
} from '../../NativeRichTextEditor';

import {
    createNativeEditorDocumentHandle,
    type NativeEditorDocumentHandle,
    _resetNativeModuleCache,
    type DocumentJSON,
    type RenderBlocksPatch,
    type RenderElement,
} from '../../NativeEditorBridge';

import {
    NativeEditorV2BoundaryError,
    NativeEditorV2LifecycleError,
    NativeEditorV2NonRetryableError,
    NativeEditorV2OperationError,
} from '../../NativeEditorBoundaryError';

import * as EditorUpdateRevision from '../../EditorUpdateRevision';

import { _resetEditorToolbarFrameRegistryForTests } from '../../EditorToolbar';

import { createYjsCollaborationController, useYjsCollaboration } from '../../YjsCollaboration';

import {
    createFakeNativeEditorV2Runtime,
    fakeDocForText,
    V2_FAKE_STEP1_FRAME,
    V2_FAKE_STEP2_FRAME,
    V2_FAKE_UPDATE_FRAME,
    type FakeNativeEditorV2Runtime,
} from './nativeEditorV2Fake';

import { withMentionsSchema } from '../../addons';

import { defineAtomNode, withAtomsSchema, type AtomComponentProps } from '../../atoms';

import { AtomUpdateAttrsError, DEFAULT_ATOM_CHIP_HEIGHT } from '../../atomInstances';

import { tiptapCompatibleSchema, type SchemaDefinition } from '../../schemas';

export const mockResolveDocumentDescriptor = require('../../schemas')
    .resolveDocumentDescriptor as jest.Mock;

export function deferred<T>() {
    let resolve!: (value: T) => void;
    let reject!: (error: unknown) => void;
    const promise = new Promise<T>((resolvePromise, rejectPromise) => {
        resolve = resolvePromise;
        reject = rejectPromise;
    });
    return { promise, resolve, reject };
}

export const HANDLE_OWNED_ARTICLE_SCHEMA: SchemaDefinition = {
    nodes: [
        { name: 'article', content: '(title | image)+', role: 'doc' },
        { name: 'title', content: 'inline*', group: 'block', role: 'textBlock' },
        { name: 'image', content: '', group: 'block', role: 'block', attrs: { src: {} } },
        { name: 'text', content: '', group: 'inline', role: 'text' },
    ],
    marks: [],
};

/** The room URL every collaboration-bound view test configures. */
export const V2_TRANSPORT_URL = 'wss://example.test/collaboration';

export let v2Runtime: FakeNativeEditorV2Runtime;

export const V2_INITIAL_DOC = fakeDocForText('hello');

export const V2_DOC_B = fakeDocForText('local b');

export const V2_DOC_C = fakeDocForText('local c');

export const V2_SERVER_DOC = fakeDocForText('server');

export const V2_SERVER_UPDATE_DOC = fakeDocForText('server update');

export function createV2RoomHandle(options: { withSnapshot?: boolean } = {}) {
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

export function createV2LocalHandle(doc?: DocumentJSON) {
    return createNativeEditorDocumentHandle({
        initialization: doc ? { type: 'localJson', json: doc } : { type: 'localEmpty' },
    });
}

/**
 * Bind one controller to a handle. The socket lives natively, so the
 * test drives it through the fake runtime's transport controls rather
 * than through any JavaScript-owned WebSocket.
 */
export function setupV2Controller(handle: NativeEditorDocumentHandle) {
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
    mockNativeViewRender.mockClear();
    mockExternalCompositionSupported = true;
    mockNativeBeginExternalComposition
        .mockReset()
        .mockImplementation(async (sessionId: string) =>
            JSON.stringify({ version: 1, type: 'active', sessionId })
        );
    mockNativeUpdateExternalComposition
        .mockReset()
        .mockImplementation(async (sessionId: string) =>
            JSON.stringify({ version: 1, type: 'active', sessionId })
        );
    mockNativeCommitExternalComposition
        .mockReset()
        .mockImplementation(async (sessionId: string, text: string) =>
            JSON.stringify({
                version: 1,
                type: 'ended',
                sessionId,
                outcome: 'committed',
                cause: 'consumer',
                text,
            })
        );
    mockNativeCancelExternalComposition
        .mockReset()
        .mockImplementation(async (sessionId: string, cause: string) =>
            JSON.stringify({
                version: 1,
                type: 'ended',
                sessionId,
                outcome: 'cancelled',
                cause,
                text: '',
            })
        );
    mockResolveDocumentDescriptor.mockClear();
});

afterEach(() => {
    act(() => {
        _resetEditorToolbarFrameRegistryForTests();
    });
    jest.useRealTimers();
});

export function renderUpdateValue(editorId: string, anchor?: number, head?: number): string {
    const raw = v2Runtime.module.editorV2RenderUpdate(editorId, anchor ?? null, head ?? null) as {
        value: string;
    };
    return raw.value;
}

export function counterAtomDefinition() {
    const component = jest.fn((props: AtomComponentProps) =>
        React.createElement(View, { testID: 'counter-atom', atomProps: props } as never)
    );
    return {
        component,
        definition: defineAtomNode({
            name: 'counterCard',
            attrs: { title: { default: '' } },
            html: {
                tag: 'div',
                staticAttrs: { 'data-native-atom': 'counter-card' },
                attrMap: { title: 'data-title' },
            },
            component,
            estimatedHeight: 120,
        }),
    };
}

export function atomBlock(
    nodeType = 'counterCard',
    docPos = 1,
    atomId?: string
): RenderElement[][] {
    return [
        [
            {
                type: 'voidBlock',
                nodeType,
                docPos,
                attrs: { title: 'a' },
                ...(atomId == null ? {} : { atomId }),
            },
        ],
    ];
}

export function installAtomRenderSource(
    source: () =>
        | { renderBlocks: RenderElement[][]; renderPatch: null }
        | { renderBlocks: null; renderPatch: RenderBlocksPatch }
) {
    const renderUpdate = mockNativeModule.editorV2RenderUpdate;
    const original = renderUpdate.getMockImplementation()!;
    renderUpdate.mockImplementation((editorId: string, anchor: unknown, head: unknown) => {
        const result = original(editorId, anchor, head) as {
            value: string;
            error: unknown;
        };
        if (result.error != null) return result;
        return {
            value: JSON.stringify({ ...JSON.parse(result.value), ...source() }),
            error: null,
        };
    });
}

export function disableExternalCompositionSupport() {
    mockExternalCompositionSupported = false;
}
