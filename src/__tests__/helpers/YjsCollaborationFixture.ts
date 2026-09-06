// The collaboration data plane lives natively: Swift and Kotlin own the
// socket, and Rust owns lifecycle state, generations, y-sync framing, the
// outbox, awareness clocks, peer expiry, retry eligibility, and close
// classification. Those contracts are covered by the Rust and platform
// suites — never re-asserted here through a JavaScript simulation.
//
// TypeScript owns exactly what this file tests: declaring transport intent
// on one authentic document handle, rendering the state/peers/errors the
// native transport reports, publishing local awareness intent, and tearing
// the binding down. It drives the faithful fake native v2 runtime in
// ./helpers/nativeEditorV2Fake.

import { readFileSync } from 'fs';

import { join } from 'path';

import {
    createFakeNativeEditorV2Runtime,
    fakeDocForText,
    V2_FAKE_AWARENESS_FRAME,
    V2_FAKE_STEP2_FRAME,
    V2_FAKE_UPDATE_FRAME,
    type FakeNativeEditorV2Runtime,
} from './nativeEditorV2Fake';

export const mockNativeModule: Record<string, jest.Mock> = {};

jest.mock('expo-modules-core', () => {
    const React = require('react');
    const { View } = require('react-native');
    const MockNativeView = React.forwardRef(
        (props: Record<string, unknown>, ref: React.Ref<unknown>) => {
            React.useImperativeHandle(ref, () => ({}));
            return React.createElement(View, { testID: 'native-editor-view', ...props });
        }
    );
    MockNativeView.displayName = 'MockNativeView';
    return {
        requireNativeModule: () => mockNativeModule,
        requireNativeViewManager: () => MockNativeView,
    };
});

import { act, renderHook } from '@testing-library/react-native';

import {
    createYjsCollaborationController,
    useYjsCollaboration,
    type YjsCollaborationOptions,
    type YjsCollaborationState,
} from '../../YjsCollaboration';

import {
    createNativeEditorDocumentHandle,
    type NativeEditorDocumentHandle,
    type NativeEditorCreateConfig,
    _resetNativeModuleCache,
    type DocumentJSON,
    type NativeEditorLocalAwarenessIntent,
    type NativeEditorPeerInfo,
} from '../../NativeEditorBridge';

import * as PublicApi from '../../index';

export const TRANSPORT_URL = 'wss://example.test/collaboration';

export const SERVER_DOC = fakeDocForText('server');

export const SECOND_SERVER_DOC = fakeDocForText('server update');

export const SNAPSHOT_DOC = fakeDocForText('snapshot');

export const ALICE = { userId: '1', name: 'Alice', color: '#f00' };

export function remotePeer(
    overrides: Partial<NativeEditorPeerInfo> = {}
): NativeEditorPeerInfo {
    return {
        clientId: '42',
        clock: 3,
        isLocal: false,
        state: {
            state: { user: { userId: '2', name: 'Bob', color: '#00f' } },
            focused: true,
        },
        cursor: { anchor: 4, head: 9 },
        ...overrides,
    };
}

export function localAwarenessIntent(
    state: Record<string, unknown> = { user: ALICE },
    focused = false
): NativeEditorLocalAwarenessIntent {
    return { state, focused };
}

export let runtime: FakeNativeEditorV2Runtime;

export function snapshotState(doc: DocumentJSON, revision = 7): Uint8Array {
    return new TextEncoder().encode(JSON.stringify({ doc, revision }));
}

export function createRoomHandle(
    options: {
        documentId?: string;
        withSnapshot?: boolean;
        limits?: NativeEditorCreateConfig['limits'];
    } = {}
): NativeEditorDocumentHandle {
    const documentId = options.documentId ?? 'doc-1';
    return createNativeEditorDocumentHandle({
        initialization: {
            type: 'room',
            documentId,
            lineageId: 'lineage-1',
            ...(options.withSnapshot
                ? {
                      snapshot: {
                          metadata: {
                              formatVersion: 1,
                              documentId,
                              lineageId: 'lineage-1',
                              fragmentName: 'prosemirror',
                              schemaFingerprint: 'fakefingerprint',
                          },
                          encodedState: snapshotState(SNAPSHOT_DOC),
                      },
                  }
                : {}),
        },
        ...(options.limits === undefined ? {} : { limits: options.limits }),
    });
}

export function createLocalHandle(doc?: DocumentJSON): NativeEditorDocumentHandle {
    return createNativeEditorDocumentHandle({
        initialization: doc ? { type: 'localJson', json: doc } : { type: 'localEmpty' },
    });
}

export interface ControllerSetup {
    controller: ReturnType<typeof createYjsCollaborationController>;
    handle: NativeEditorDocumentHandle;
    states: YjsCollaborationState[];
    errors: Error[];
    peersLog: NativeEditorPeerInfo[][];
}

export function setupController(
    overrides: Partial<YjsCollaborationOptions> & { handle?: NativeEditorDocumentHandle } = {}
): ControllerSetup {
    const states: YjsCollaborationState[] = [];
    const errors: Error[] = [];
    const peersLog: NativeEditorPeerInfo[][] = [];
    const handle = overrides.handle ?? createRoomHandle();
    const controller = createYjsCollaborationController({
        documentId: 'doc-1',
        handle,
        transport: { url: TRANSPORT_URL, connect: false },
        onStateChange: (state) => states.push({ ...state }),
        onError: (error) => errors.push(error),
        onPeersChange: (peers) => peersLog.push(peers),
        ...overrides,
    } as YjsCollaborationOptions);
    return { controller, handle, states, errors, peersLog };
}

/** The transport intent JSON TypeScript last handed to the native module. */
export function configuredTransport(callIndex = -1): unknown {
    const calls = runtime.module.editorV2CollaborationConfigureTransport.mock.calls;
    const call = calls.at(callIndex);
    if (call == null) throw new Error('no transport configuration was issued');
    return JSON.parse(call[1] as string);
}

/** Drive the native transport all the way to `Synchronized`. */
export function synchronize(handle: NativeEditorDocumentHandle): void {
    runtime.transportOpen(handle.editorId);
    runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
}

export function latestStatus(setup: ControllerSetup): string {
    return setup.controller.state.status;
}

export function awarenessPayload(callIndex = -1): unknown {
    const calls = runtime.module.editorV2CollaborationSetAwareness.mock.calls;
    const call = calls.at(callIndex);
    if (call == null) throw new Error('no awareness intent was published');
    return JSON.parse(call[1] as string);
}

beforeEach(() => {
    _resetNativeModuleCache();
    runtime = createFakeNativeEditorV2Runtime();
    for (const key of Object.keys(mockNativeModule)) {
        delete mockNativeModule[key];
    }
    for (const [key, impl] of Object.entries(runtime.module)) {
        mockNativeModule[key] = impl;
    }
});
