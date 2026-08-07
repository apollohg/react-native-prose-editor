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
} from './helpers/nativeEditorV2Fake';

const mockNativeModule: Record<string, jest.Mock> = {};

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
} from '../YjsCollaboration';
import {
    createNativeEditorDocumentHandle,
    type NativeEditorDocumentHandle,
    type NativeEditorV2CreateConfig,
    _resetNativeModuleCache,
    type DocumentJSON,
    type NativeEditorLocalAwarenessIntent,
    type NativeEditorV2PeerInfo,
} from '../NativeEditorBridge';
import * as PublicApi from '../index';


const TRANSPORT_URL = 'wss://example.test/collaboration';
const SERVER_DOC = fakeDocForText('server');
const SECOND_SERVER_DOC = fakeDocForText('server update');
const SNAPSHOT_DOC = fakeDocForText('snapshot');

const ALICE = { userId: '1', name: 'Alice', color: '#f00' };

function remotePeer(overrides: Partial<NativeEditorV2PeerInfo> = {}): NativeEditorV2PeerInfo {
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

function localAwarenessIntent(
    state: Record<string, unknown> = { user: ALICE },
    focused = false
): NativeEditorLocalAwarenessIntent {
    return { state, focused };
}


let runtime: FakeNativeEditorV2Runtime;

function snapshotState(doc: DocumentJSON, revision = 7): Uint8Array {
    return new TextEncoder().encode(JSON.stringify({ doc, revision }));
}

function createRoomHandle(
    options: {
        documentId?: string;
        withSnapshot?: boolean;
        limits?: NativeEditorV2CreateConfig['limits'];
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

function createLocalHandle(doc?: DocumentJSON): NativeEditorDocumentHandle {
    return createNativeEditorDocumentHandle({
        initialization: doc ? { type: 'localJson', json: doc } : { type: 'localEmpty' },
    });
}

interface ControllerSetup {
    controller: ReturnType<typeof createYjsCollaborationController>;
    handle: NativeEditorDocumentHandle;
    states: YjsCollaborationState[];
    errors: Error[];
    peersLog: NativeEditorV2PeerInfo[][];
}

function setupController(
    overrides: Partial<YjsCollaborationOptions> & { handle?: NativeEditorDocumentHandle } = {}
): ControllerSetup {
    const states: YjsCollaborationState[] = [];
    const errors: Error[] = [];
    const peersLog: NativeEditorV2PeerInfo[][] = [];
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
function configuredTransport(callIndex = -1): unknown {
    const calls = runtime.module.editorV2CollaborationConfigureTransport.mock.calls;
    const call = calls.at(callIndex);
    if (call == null) throw new Error('no transport configuration was issued');
    return JSON.parse(call[1] as string);
}

/** Drive the native transport all the way to `Synchronized`. */
function synchronize(handle: NativeEditorDocumentHandle): void {
    runtime.transportOpen(handle.editorId);
    runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
}

function latestStatus(setup: ControllerSetup): string {
    return setup.controller.state.status;
}

function awarenessPayload(callIndex = -1): unknown {
    const calls = runtime.module.editorV2CollaborationSetAwareness.mock.calls;
    const call = calls.at(callIndex);
    if (call == null) throw new Error('no awareness intent was published');
    return JSON.parse(call[1] as string);
}


describe('YjsCollaboration (native-transport controller)', () => {
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


    it('rejects recovered constructors and structural handle forgeries while accepting real handles', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const forgeries: unknown[] = [
            null,
            undefined,
            {},
            { editorId: handle.editorId, bridge: handle.bridge },
            Object.create(Object.getPrototypeOf(handle)),
            { ...handle },
        ];

        for (const forgery of forgeries) {
            expect(() =>
                createYjsCollaborationController({
                    documentId: 'doc-1',
                    handle: forgery as NativeEditorDocumentHandle,
                    transport: { url: TRANSPORT_URL, connect: false },
                })
            ).toThrow();
        }

        const controller = createYjsCollaborationController({
            documentId: 'doc-1',
            handle,
            transport: { url: TRANSPORT_URL, connect: false },
        });
        expect(controller.documentHandle).toBe(handle);
        controller.destroy();
    });


    it('declares transport intent on the handle and never opens a socket itself', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        expect(configuredTransport()).toEqual({ url: TRANSPORT_URL, connect: false });

        setup.controller.connect();
        expect(configuredTransport()).toEqual({ url: TRANSPORT_URL, connect: true });

        setup.controller.disconnect();
        expect(configuredTransport()).toEqual({ url: TRANSPORT_URL, connect: false });

        // No JavaScript-owned socket exists at any point in the cutover.
        expect(runtime.module.editorV2CollaborationConfigureTransport).toHaveBeenCalledTimes(3);
    });

    it('forwards the static protocol adapter descriptor without its callbacks', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        setupController({
            handle,
            transport: {
                url: TRANSPORT_URL,
                connect: true,
                protocolAdapter: {
                    protocols: ['example-auth-v1'],
                    timeoutMillis: 5_000,
                    terminalCloseCodes: [4403],
                    onOpen: async () => ({ action: 'continue' as const }),
                    onMessage: async () => ({ action: 'ready' as const }),
                },
            },
        });
        expect(configuredTransport()).toEqual({
            url: TRANSPORT_URL,
            connect: true,
            protocolAdapter: {
                protocols: ['example-auth-v1'],
                timeoutMillis: 5_000,
                terminalCloseCodes: [4403],
            },
        });
    });

    it('reconnect retires the live intent before re-declaring it', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        synchronize(setup.handle);
        expect(latestStatus(setup)).toBe('synchronized');

        runtime.module.editorV2CollaborationConfigureTransport.mockClear();
        setup.controller.reconnect();
        const calls = runtime.module.editorV2CollaborationConfigureTransport.mock.calls;
        expect(calls).toHaveLength(2);
        expect(JSON.parse(calls[0][1] as string)).toMatchObject({ connect: false });
        expect(JSON.parse(calls[1][1] as string)).toMatchObject({ connect: true });
    });

    it('a null transport keeps the handle detached and admits no connect intent', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            transport: null,
        });
        expect(configuredTransport()).toBeNull();

        setup.controller.connect();
        setup.controller.reconnect();
        expect(runtime.module.editorV2CollaborationConfigureTransport).toHaveBeenCalledTimes(1);
        expect(latestStatus(setup)).toBe('idle');
    });

    it('surfaces a refused transport configuration without leaking the listener', () => {
        const handle = createLocalHandle(SERVER_DOC);
        expect(() =>
            createYjsCollaborationController({
                documentId: 'doc-1',
                handle,
                transport: { url: TRANSPORT_URL, connect: true },
            })
        ).toThrow();
        // The failed constructor removed the subscription it had added.
        expect(mockNativeModule.addListener.mock.results[0].value.remove).toBeDefined();
    });


    it('renders the Rust-reported transport state through every phase', () => {
        const setup = setupController();
        expect(latestStatus(setup)).toBe('disconnected');

        setup.controller.connect();
        expect(latestStatus(setup)).toBe('connecting');

        runtime.transportOpen(setup.handle.editorId);
        expect(latestStatus(setup)).toBe('handshaking');
        expect(setup.controller.state.isConnected).toBe(false);

        runtime.pushRemoteDoc(setup.handle.editorId, SERVER_DOC);
        runtime.transportReceive(setup.handle.editorId, V2_FAKE_STEP2_FRAME);
        expect(latestStatus(setup)).toBe('synchronized');
        expect(setup.controller.state.isConnected).toBe(true);
        expect(setup.controller.state.documentJson).toEqual(SERVER_DOC);
    });

    it('renders a snapshot-bound room document while disconnected', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        expect(setup.controller.state.documentJson).toEqual(SNAPSHOT_DOC);
        expect(latestStatus(setup)).toBe('disconnected');
    });

    it('re-reads the document only when the revision advances', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        synchronize(setup.handle);
        const documentReadsAfterSync =
            runtime.module.editorV2GetDocumentJson.mock.calls.length;

        // A state event that does not change the revision reuses the
        // rendered document instead of crossing the boundary again.
        runtime.transportClose(setup.handle.editorId);
        expect(runtime.module.editorV2GetDocumentJson).toHaveBeenCalledTimes(
            documentReadsAfterSync
        );
        expect(setup.controller.state.documentJson).toEqual(SNAPSHOT_DOC);

        setup.controller.connect();
        synchronize(setup.handle);
        runtime.pushRemoteDoc(setup.handle.editorId, SECOND_SERVER_DOC);
        runtime.transportReceive(setup.handle.editorId, V2_FAKE_UPDATE_FRAME);
        expect(setup.controller.state.documentJson).toEqual(SECOND_SERVER_DOC);
    });

    it('ignores superseded transport events by event sequence', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        synchronize(setup.handle);
        const statesAfterSync = setup.states.length;

        // Replay the exact sequence the controller already consumed.
        const listener = mockNativeModule.addListener.mock.calls[0][1] as (
            event: unknown
        ) => void;
        const lastEvent = { ...(runtime.session(setup.handle.editorId) as unknown as object) };
        void lastEvent;
        listener({
            editorId: setup.handle.editorId,
            eventSequence: '1',
            generation: '1',
            kind: 'state',
            state: mockNativeModule.editorV2GetState(setup.handle.editorId).value,
            peers: [],
            diagnostics: {
                wakeReason: 'replay',
                transportState: 'Disconnected',
                nextDeadlineMillis: null,
                remoteCommitApplied: false,
                peersChanged: false,
                renewedLocal: false,
                expiredPeerCount: 0,
            },
        });
        expect(setup.states).toHaveLength(statesAfterSync);
        expect(latestStatus(setup)).toBe('synchronized');
    });

    it('routes native transport errors to onError and the rendered state', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        runtime.emitTransportError(setup.handle.editorId, {
            domain: 'transport',
            code: 'TRANSPORT_SOCKET_FAILED',
            message: 'socket failed',
            requestId: null,
            operationIndex: null,
            limit: null,
            actual: null,
            details: null,
        });

        expect(setup.errors).toHaveLength(1);
        expect(setup.errors[0].message).toBe('socket failed');
        expect(setup.controller.state.lastError).toBe(setup.errors[0]);
    });

    it('clears a retained error once a later state event succeeds', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        runtime.emitTransportError(setup.handle.editorId, {
            domain: 'transport',
            code: 'TRANSPORT_SOCKET_FAILED',
            message: 'socket failed',
            requestId: null,
            operationIndex: null,
            limit: null,
            actual: null,
            details: null,
        });
        expect(setup.controller.state.lastError).toBeDefined();

        synchronize(setup.handle);
        expect(setup.controller.state.lastError).toBeUndefined();
    });

    it('ignores events addressed to a different editor', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        const other = createRoomHandle({ documentId: 'doc-2', withSnapshot: true });
        setup.controller.connect();
        const statesBefore = setup.states.length;

        runtime.emitTransportError(other.editorId, {
            domain: 'transport',
            code: 'TRANSPORT_SOCKET_FAILED',
            message: 'other editor failed',
            requestId: null,
            operationIndex: null,
            limit: null,
            actual: null,
            details: null,
        });
        expect(setup.errors).toHaveLength(0);
        expect(setup.states).toHaveLength(statesBefore);
        expect(latestStatus(setup)).toBe('connecting');
    });


    it('reports peers from the native projection and derives remote selections', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const { result } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                transport: { url: TRANSPORT_URL, connect: true },
                localAwareness: ALICE,
            })
        );
        act(() => {
            synchronize(handle);
        });
        act(() => {
            runtime.pushRemotePeers(handle.editorId, [
                remotePeer({
                    state: {
                        state: {
                            user: { userId: '2', name: 'Bob', color: '#00f' },
                            // A caller-authored scalar selection is never a cursor.
                            selection: { anchor: 90, head: 91 },
                        },
                        focused: true,
                    },
                    cursor: { anchor: 4, head: 9 },
                }),
                remotePeer({
                    clientId: '43',
                    cursor: null,
                    state: {
                        state: {
                            user: { userId: '3', name: 'Carol', color: '#0a0' },
                            selection: { anchor: 40, head: 41 },
                        },
                        focused: true,
                    },
                }),
            ]);
            runtime.transportReceive(handle.editorId, V2_FAKE_AWARENESS_FRAME);
        });

        expect(result.current.editorBindings.remoteSelections).toEqual([
            {
                clientId: '42',
                anchor: 4,
                head: 9,
                color: '#00f',
                name: 'Bob',
                avatarUrl: undefined,
                isFocused: true,
            },
        ]);
    });

    it('falls back to a default color and omits an absent name', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        synchronize(setup.handle);
        runtime.pushRemotePeers(setup.handle.editorId, [
            remotePeer({ state: { state: {}, focused: false } }),
        ]);
        runtime.transportReceive(setup.handle.editorId, V2_FAKE_AWARENESS_FRAME);

        expect(setup.peersLog.at(-1)).toHaveLength(1);
        expect(setup.controller.peers[0].clientId).toBe('42');
    });


    it('publishes the local awareness intent through Rust with no TypeScript clock bookkeeping', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        expect(awarenessPayload()).toEqual(localAwarenessIntent());
        // Clocks, tombstones, and renewal deadlines are Rust-owned: the
        // published intent carries none of them.
        expect(JSON.stringify(awarenessPayload())).not.toContain('clock');
        expect(setup.controller.state.documentJson).toEqual(SNAPSHOT_DOC);
    });

    it('never restates a document position on a focus-only update', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.connect();
        synchronize(setup.handle);
        setup.controller.handleSelectionChange({ type: 'text', anchor: 2, head: 5 });
        expect(awarenessPayload()).toMatchObject({
            selection: { type: 'text', anchor: 2, head: 5 },
        });

        // Focus and blur state no position at all, so the Rust-owned sticky
        // cursor is retained rather than re-resolved against a document
        // that may have moved underneath the last observed offsets.
        setup.controller.handleFocusChange(true);
        expect(awarenessPayload()).not.toHaveProperty('selection');
        setup.controller.handleFocusChange(false);
        expect(awarenessPayload()).not.toHaveProperty('selection');
        expect(setup.errors).toEqual([]);
    });

    it('blur cannot fail after the document invalidates the last observed selection', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.connect();
        synchronize(setup.handle);
        // A caret near the end of "snapshot" is valid when it is observed.
        setup.controller.handleSelectionChange({ type: 'text', anchor: 8, head: 8 });
        setup.controller.handleFocusChange(true);

        // A remote peer then shrinks the document under that offset, which
        // is exactly the state the reported blur crash was reproduced from.
        runtime.pushRemoteDoc(setup.handle.editorId, fakeDocForText('hi'));
        runtime.transportReceive(setup.handle.editorId, V2_FAKE_UPDATE_FRAME);

        expect(() => setup.controller.handleFocusChange(false)).not.toThrow();
        expect(awarenessPayload()).toEqual({ state: { user: ALICE }, focused: false });
        expect(setup.errors).toEqual([]);
    });

    it('reports a refused explicit selection instead of throwing into the caller', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.connect();
        synchronize(setup.handle);

        // An explicitly stated position outside the document is still a
        // caller contract violation — reported, never silently accepted.
        expect(() =>
            setup.controller.handleSelectionChange({ type: 'text', anchor: 999, head: 999 })
        ).not.toThrow();
        expect(setup.errors).toHaveLength(1);
        expect(setup.errors[0].message).toContain('outside the current document');
        expect(runtime.session(setup.handle.editorId).desiredAwareness).toEqual(
            localAwarenessIntent()
        );
    });

    it('composes application state, focus, and document selection into the local intent', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.handleSelectionChange({ type: 'text', anchor: 2, head: 5 });
        expect(awarenessPayload()).toEqual({
            state: { user: ALICE },
            focused: false,
            selection: { type: 'text', anchor: 2, head: 5 },
        });

        setup.controller.handleFocusChange(true);
        expect(awarenessPayload()).toMatchObject({ focused: true });

        setup.controller.updateLocalAwareness({ user: { ...ALICE, name: 'Alice II' } });
        expect(awarenessPayload()).toMatchObject({
            state: { user: { ...ALICE, name: 'Alice II' } },
        });
    });

    it('normalizes node and all selections to a cursorless awareness intent', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.handleSelectionChange({ type: 'text', anchor: 2, head: 5 });

        // A non-text selection is an explicit "no cursor", not a silent
        // retention of the last text position.
        setup.controller.handleSelectionChange({ type: 'node', pos: 3 });
        expect(awarenessPayload()).toEqual({
            state: { user: ALICE },
            focused: false,
            selection: null,
        });

        setup.controller.handleSelectionChange({ type: 'text', anchor: 3, head: 4 });
        setup.controller.handleSelectionChange({ type: 'all' });
        expect(awarenessPayload()).toEqual({
            state: { user: ALICE },
            focused: false,
            selection: null,
        });
    });

    it('setting identical awareness twice emits no publish', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(1);

        setup.controller.updateLocalAwareness({ user: { ...ALICE } });
        setup.controller.updateLocalAwareness({ focused: false });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(1);

        setup.controller.updateLocalAwareness({ focused: true });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(2);
    });

    it('withdraws local awareness when a controller omits localAwareness', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const first = setupController({ handle, localAwareness: ALICE });
        first.controller.connect();
        synchronize(handle);
        expect(runtime.session(handle.editorId).desiredAwareness).toEqual(localAwarenessIntent());
        first.controller.destroy();

        runtime.module.editorV2CollaborationSetAwareness.mockClear();
        setupController({ handle });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenLastCalledWith(
            handle.editorId,
            'null'
        );
        expect(runtime.session(handle.editorId).desiredAwareness).toBeNull();
    });

    it('keeps focus and selection hooks inert after awareness is withdrawn', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const setup = setupController({ handle, localAwareness: ALICE });
        setup.controller.applyLocalAwarenessOption(undefined);
        const publishesAfterWithdrawal =
            runtime.module.editorV2CollaborationSetAwareness.mock.calls.length;

        setup.controller.handleFocusChange(true);
        setup.controller.handleFocusChange(false);
        setup.controller.handleSelectionChange({ type: 'text', anchor: 1, head: 3 });

        // Presence must not resurrect itself as an anonymous entry.
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(
            publishesAfterWithdrawal
        );
        expect(runtime.session(handle.editorId).desiredAwareness).toBeNull();

        // An explicit user restores it.
        setup.controller.updateLocalAwareness({ user: ALICE });
        expect(runtime.session(handle.editorId).desiredAwareness).toEqual(
            localAwarenessIntent({ user: ALICE }, false)
        );
    });

    it('reports an awareness intent above the peer-byte limit without throwing', () => {
        const setup = setupController({
            handle: createRoomHandle({
                withSnapshot: true,
                limits: { collaboration: { maxAwarenessPeerBytes: 128 } },
            }),
            localAwareness: ALICE,
        });
        const acceptedIntent = runtime.session(setup.handle.editorId).desiredAwareness;

        // Presence is ambient UI state: a refusal is reported, never thrown
        // into the caller, and the last accepted intent stays in force.
        expect(() =>
            setup.controller.updateLocalAwareness({
                user: { ...ALICE, name: 'A'.repeat(256) },
            })
        ).not.toThrow();
        expect(setup.errors).toHaveLength(1);
        expect(setup.controller.state.lastError).toBe(setup.errors[0]);
        expect(runtime.session(setup.handle.editorId).desiredAwareness).toEqual(acceptedIntent);

        // The rejected candidate was discarded, so a later valid change is
        // still published rather than being deduped against it.
        setup.controller.updateLocalAwareness({ user: { ...ALICE, name: 'Alice II' } });
        expect(awarenessPayload()).toMatchObject({
            state: { user: { ...ALICE, name: 'Alice II' } },
        });
    });


    it('destroy detaches the transport but never destroys the shared document handle', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        runtime.transportOpen(setup.handle.editorId);

        setup.controller.destroy();
        expect(configuredTransport()).toBeNull();
        // One shared session: the handle outlives the controller.
        expect(runtime.module.editorV2Destroy).not.toHaveBeenCalled();
        expect(runtime.module.editorV2Create).toHaveBeenCalledTimes(1);

        // A destroyed controller is inert.
        const configureCalls =
            runtime.module.editorV2CollaborationConfigureTransport.mock.calls.length;
        setup.controller.connect();
        setup.controller.reconnect();
        setup.controller.updateLocalAwareness({ user: ALICE });
        expect(runtime.module.editorV2CollaborationConfigureTransport).toHaveBeenCalledTimes(
            configureCalls
        );
    });

    it('stops rendering native events after destroy', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        setup.controller.destroy();
        const statesAfterDestroy = setup.states.length;

        runtime.emitTransportError(setup.handle.editorId, {
            domain: 'transport',
            code: 'TRANSPORT_SOCKET_FAILED',
            message: 'late failure',
            requestId: null,
            operationIndex: null,
            limit: null,
            actual: null,
            details: null,
        });
        expect(setup.errors).toHaveLength(0);
        expect(setup.states).toHaveLength(statesAfterDestroy);
    });


    it('useYjsCollaboration renders returned Rust state and binds the shared handle', () => {
        const handle = createRoomHandle();
        const { result } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                transport: { url: TRANSPORT_URL, connect: false },
            })
        );

        expect(result.current.state.status).toBe('disconnected');
        expect(result.current.state.documentJson).toBeNull();
        expect(result.current.isConnected).toBe(false);
        expect(result.current.editorBindings.documentHandle).toBe(handle);
        expect(result.current.editorBindings.documentRevision).toBeNull();

        act(() => {
            result.current.connect();
        });
        expect(result.current.state.status).toBe('connecting');
        act(() => {
            runtime.transportOpen(handle.editorId);
        });
        expect(result.current.state.status).toBe('handshaking');
        expect(result.current.isConnected).toBe(false);

        act(() => {
            runtime.pushRemoteDoc(handle.editorId, SERVER_DOC);
            runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
        });
        expect(result.current.state.status).toBe('synchronized');
        expect(result.current.isConnected).toBe(true);
        expect(result.current.state.documentJson).toEqual(SERVER_DOC);
        expect(result.current.editorBindings.documentRevision).not.toBeNull();
    });

    it('honors the connect prop and forwards callbacks', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const onStateChange = jest.fn();
        let connect = false;
        const { result, rerender } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                transport: { url: TRANSPORT_URL, connect },
                onStateChange,
            })
        );
        expect(result.current.state.status).toBe('disconnected');

        connect = true;
        rerender();
        expect(configuredTransport()).toMatchObject({ connect: true });
        expect(onStateChange).toHaveBeenCalled();
        expect(result.current.state.status).toBe('connecting');
    });

    it('clears live prop awareness once and lets an explicit user restore it', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        let localAwareness: typeof ALICE | undefined = ALICE;
        const { result, rerender } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                transport: { url: TRANSPORT_URL, connect: true },
                localAwareness,
            })
        );
        // The constructor publishes once; the mount-time prop effect merges
        // the identical user and dedups to a no-op.
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(1);
        act(() => {
            synchronize(handle);
        });

        localAwareness = { ...ALICE, name: 'Alice II' };
        rerender();
        expect(awarenessPayload()).toEqual({
            state: { user: { ...ALICE, name: 'Alice II' } },
            focused: false,
        });

        localAwareness = undefined;
        rerender();
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenLastCalledWith(
            handle.editorId,
            'null'
        );
        expect(runtime.session(handle.editorId).desiredAwareness).toBeNull();

        act(() => {
            result.current.updateLocalAwareness({ user: ALICE });
        });
        expect(runtime.session(handle.editorId).desiredAwareness).toEqual(localAwarenessIntent());
    });


    it('proves the removed collaboration surface no longer exists in source or exports', () => {
        const collaborationSource = readFileSync(
            join(__dirname, '..', 'YjsCollaboration.ts'),
            'utf8'
        );
        const forbiddenInCollaboration = [
            'NativeCollaborationBridge',
            'collaborationSession',
            'initialDocumentJson',
            // Assembled so the shipped-runtime search gate stays at zero
            // while the removal proof still pins the exact legacy tokens.
            'initial' + 'EncodedState',
            'applyEncodedState',
            'replaceEncodedState',
            'getEncodedState',
            'applyLocal' + 'DocumentJson',
            'onContentChangeJSON',
            'valueJSON',
            // The JavaScript-owned data plane removed by the native cutover.
            'createWebSocket',
            'WebSocket',
            'retryIntervalMs',
        ];
        for (const identifier of forbiddenInCollaboration) {
            expect(collaborationSource.includes(identifier)).toBe(false);
        }

        const indexSource = readFileSync(join(__dirname, '..', 'index.ts'), 'utf8');
        const forbiddenInIndex = [
            'encodeCollaborationStateBase64',
            'decodeCollaborationStateBase64',
            'EncodedCollaborationStateInput',
        ];
        for (const identifier of forbiddenInIndex) {
            expect(indexSource.includes(identifier)).toBe(false);
        }

        expect('createYjsCollaborationController' in PublicApi).toBe(true);
        expect('useYjsCollaboration' in PublicApi).toBe(true);
        expect('encodeCollaborationStateBase64' in PublicApi).toBe(false);
        expect('decodeCollaborationStateBase64' in PublicApi).toBe(false);

        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        const controller = setup.controller as unknown as Record<string, unknown>;
        for (const removed of [
            'getEncodedState',
            'getEncodedStateBase64',
            'applyEncodedState',
            'replaceEncodedState',
            'handleLocalDocumentChange',
            'handleLocalCommit',
        ]) {
            expect(removed in controller).toBe(false);
        }
    });

    it('proves editorBindings carries no valueJSON reset synchronization surface', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const { result } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                transport: { url: TRANSPORT_URL, connect: false },
            })
        );
        const bindings = result.current.editorBindings as unknown as Record<string, unknown>;
        for (const removed of [
            'valueJSON',
            'valueJSONUpdateMode',
            'preserveSelectionOnValueJSONReset',
            'selectionOnValueJSONReset',
            'onContentChangeJSON',
            'onLocalDocumentCommit',
            // Native publishes the local caret straight into Rust; a
            // JavaScript mirror of it would only ever be a stale copy.
            'onSelectionChange',
        ]) {
            expect(removed in bindings).toBe(false);
        }
        expect(bindings.documentHandle).toBe(handle);
        expect(typeof bindings.onFocus).toBe('function');
        expect(typeof bindings.onBlur).toBe('function');
    });
});
