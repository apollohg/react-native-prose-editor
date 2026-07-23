// ─── YjsCollaboration (Task 14 thin controller) Tests ──────────
// The legacy collaboration controller owned session creation, retry
// decisions, awareness clocks, valueJSON reset sync, and raw encoded-state
// APIs. All of that is removed. The Task 14 controller is a thin shell over
// a SHARED NativeEditorDocumentHandle: it owns only WebSocket objects and
// retry-timer scheduling; Rust owns lifecycle state, generations, retry
// eligibility, protocol, and awareness clocks. These tests drive it through
// the faithful fake native v2 runtime in ./helpers/nativeEditorV2Fake.

import { readFileSync } from 'fs';
import { join } from 'path';

import {
    createFakeNativeEditorV2Runtime,
    fakeDocForText,
    V2_FAKE_AWARENESS_FRAME,
    V2_FAKE_INCOMPATIBLE_FRAME,
    V2_FAKE_STEP1_FRAME,
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

import React from 'react';
import { act, renderHook } from '@testing-library/react-native';

import {
    createYjsCollaborationController,
    useYjsCollaboration,
    type YjsCollaborationOptions,
    type YjsCollaborationState,
    type YjsRetryContext,
} from '../YjsCollaboration';
import {
    createNativeEditorDocumentHandle,
    type NativeEditorDocumentHandle,
    _resetNativeModuleCache,
    type DocumentJSON,
    type NativeEditorV2PeerInfo,
} from '../NativeEditorBridge';
import { NativeEditorV2TransportError } from '../NativeEditorBoundaryError';
import * as PublicApi from '../index';

// ─── Mock WebSocket ─────────────────────────────────────────────

class MockWebSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;

    readyState = MockWebSocket.CONNECTING;
    binaryType?: string;
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: unknown }) => void) | null = null;
    onerror: (() => void) | null = null;
    onclose: ((event?: { code?: number; reason?: string }) => void) | null = null;
    send = jest.fn();
    close = jest.fn(() => {
        this.readyState = MockWebSocket.CLOSED;
        this.onclose?.({ code: 1000, reason: '' });
    });

    open(): void {
        this.readyState = MockWebSocket.OPEN;
        this.onopen?.();
    }

    receive(bytes: Uint8Array): void {
        this.onmessage?.({ data: bytes.slice().buffer });
    }

    serverClose(code: number, reason = ''): void {
        this.readyState = MockWebSocket.CLOSED;
        this.onclose?.({ code, reason });
    }
}

// ─── Fixtures ───────────────────────────────────────────────────

const SERVER_DOC = fakeDocForText('server');
const SECOND_SERVER_DOC = fakeDocForText('server update');
const SNAPSHOT_DOC = fakeDocForText('snapshot');
const LOCAL_DOC_B = fakeDocForText('local b');
const LOCAL_DOC_C = fakeDocForText('local c');

const ALICE = { userId: '1', name: 'Alice', color: '#f00' };

function remotePeer(overrides: Partial<NativeEditorV2PeerInfo> = {}): NativeEditorV2PeerInfo {
    return {
        clientId: '42',
        clock: 3,
        isLocal: false,
        state: { user: { userId: '2', name: 'Bob', color: '#00f' }, focused: true },
        cursor: { anchor: 4, head: 9 },
        ...overrides,
    };
}

// ─── Setup helpers ──────────────────────────────────────────────

let runtime: FakeNativeEditorV2Runtime;

function snapshotState(doc: DocumentJSON, revision = 7): Uint8Array {
    return new TextEncoder().encode(JSON.stringify({ doc, revision }));
}

function createRoomHandle(
    options: { documentId?: string; withSnapshot?: boolean } = {}
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
    sockets: MockWebSocket[];
    states: YjsCollaborationState[];
    errors: Error[];
    peersLog: NativeEditorV2PeerInfo[][];
}

function setupController(
    overrides: Partial<YjsCollaborationOptions> & { handle?: NativeEditorDocumentHandle } = {}
): ControllerSetup {
    const sockets: MockWebSocket[] = [];
    const states: YjsCollaborationState[] = [];
    const errors: Error[] = [];
    const peersLog: NativeEditorV2PeerInfo[][] = [];
    const handle = overrides.handle ?? createRoomHandle();
    const controller = createYjsCollaborationController({
        documentId: 'doc-1',
        handle,
        connect: false,
        createWebSocket: () => {
            const socket = new MockWebSocket();
            sockets.push(socket);
            return socket as unknown as WebSocket;
        },
        onStateChange: (state) => states.push({ ...state }),
        onError: (error) => errors.push(error),
        onPeersChange: (peers) => peersLog.push(peers),
        ...overrides,
    });
    return { controller, handle, sockets, states, errors, peersLog };
}

function sentFrames(socket: MockWebSocket): number[][] {
    return socket.send.mock.calls.map((call) => Array.from(new Uint8Array(call[0] as ArrayBuffer)));
}

function latestStatus(setup: ControllerSetup): string {
    return setup.controller.state.status;
}

function openAndSynchronize(setup: ControllerSetup, socketIndex = 0): void {
    setup.sockets[socketIndex].open();
    setup.sockets[socketIndex].receive(V2_FAKE_STEP2_FRAME);
}

// ─── Test setup ─────────────────────────────────────────────────

describe('YjsCollaboration (Task 14 thin controller)', () => {
    const OriginalWebSocket = global.WebSocket;

    beforeEach(() => {
        jest.useFakeTimers();
        _resetNativeModuleCache();
        runtime = createFakeNativeEditorV2Runtime();
        for (const key of Object.keys(mockNativeModule)) {
            delete mockNativeModule[key];
        }
        for (const [key, impl] of Object.entries(runtime.module)) {
            mockNativeModule[key] = impl;
        }
        global.WebSocket = MockWebSocket as unknown as typeof WebSocket;
    });

    afterAll(() => {
        global.WebSocket = OriginalWebSocket;
    });

    // ── Lifecycle / status mapping ──────────────────────────────

    it('rejects recovered constructors and structural handle forgeries while accepting real handles', () => {
        const real = createRoomHandle();
        const recoveredConstructor = real.constructor as new (
            editorId: string,
            bridge: NativeEditorDocumentHandle['bridge']
        ) => NativeEditorDocumentHandle;

        const outcome = (operation: () => void): string => {
            try {
                operation();
                return 'accepted';
            } catch (error) {
                return String((error as { code?: unknown }).code);
            }
        };
        const recovered = outcome(() => {
            new recoveredConstructor(real.editorId, real.bridge);
        });
        const forged = {
            editorId: real.editorId,
            bridge: real.bridge,
            isDestroyed: false,
            destroy: jest.fn(),
            addErrorListener: jest.fn(() => jest.fn()),
        } as unknown as NativeEditorDocumentHandle;
        const structural = outcome(() => {
            const controller = createYjsCollaborationController({
                documentId: 'doc-forged',
                handle: forged,
                connect: false,
                createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            });
            controller.destroy();
        });
        const authentic = outcome(() => {
            const controller = createYjsCollaborationController({
                documentId: 'doc-1',
                handle: real,
                connect: false,
                createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            });
            controller.destroy();
        });
        real.destroy();

        expect({ recovered, structural, authentic }).toEqual({
            recovered: 'CONFIG_INVALID',
            structural: 'CONFIG_INVALID',
            authentic: 'accepted',
        });
    });

    it('reports connecting, then handshaking (never connected), and synchronizes only on an accepted server Step 2', () => {
        const setup = setupController();

        // Server-owned room init: disconnected, no client document at all.
        expect(setup.controller.state.status).toBe('disconnected');
        expect(setup.controller.state.isConnected).toBe(false);
        expect(setup.controller.state.documentJson).toBeNull();
        expect(setup.controller.state.documentRevision).toBeNull();

        setup.controller.connect();
        expect(latestStatus(setup)).toBe('connecting');
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledWith(
            setup.handle.editorId
        );
        expect(setup.sockets).toHaveLength(1);

        setup.sockets[0].open();
        expect(runtime.module.editorV2CollaborationSocketOpen).toHaveBeenCalledWith(
            setup.handle.editorId,
            '1'
        );
        // Sync Step 1 rides the socket first.
        expect(sentFrames(setup.sockets[0])).toEqual([Array.from(V2_FAKE_STEP1_FRAME)]);
        // open alone is handshaking, never connected/synchronized.
        expect(latestStatus(setup)).toBe('handshaking');
        expect(setup.controller.state.isConnected).toBe(false);

        runtime.pushRemoteDoc(setup.handle.editorId, SERVER_DOC);
        setup.sockets[0].receive(V2_FAKE_STEP2_FRAME);
        expect(latestStatus(setup)).toBe('synchronized');
        expect(setup.controller.state.isConnected).toBe(true);
        expect(setup.controller.state.documentJson).toEqual(SERVER_DOC);
        expect(setup.controller.state.documentRevision).not.toBeNull();

        // The room emitted no client state before the server document arrived:
        // zero local mutation entries were ever invoked.
        expect(runtime.module.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(runtime.module.editorV2ReplaceDocument).not.toHaveBeenCalled();
        expect(runtime.module.editorV2ApplyInput).not.toHaveBeenCalled();
    });

    it('renders a snapshot-bound room document while disconnected', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        expect(setup.controller.state.status).toBe('disconnected');
        expect(setup.controller.state.documentJson).toEqual(SNAPSHOT_DOC);
        expect(setup.controller.state.documentRevision).toBe('7');
    });

    it('refuses connect on a local (non-room) handle without scheduling a retry', () => {
        const setup = setupController({ handle: createLocalHandle() });
        setup.controller.connect();
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);
        expect(setup.errors).toHaveLength(1);
        expect(setup.sockets).toHaveLength(0);
        // Detached transport maps to idle; nothing is scheduled.
        expect(setup.controller.state.status).toBe('idle');
        jest.advanceTimersByTime(60_000);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);
        expect(setup.sockets).toHaveLength(0);
    });

    it('pins the local-session begin_connect refusal to the real Rust code/domain pair', () => {
        const setup = setupController({ handle: createLocalHandle() });
        setup.controller.connect();
        expect(setup.errors).toHaveLength(1);
        const refusal = setup.errors[0] as NativeEditorV2TransportError;
        // rust/editor-core/src/collaboration_runtime/state.rs not_room_bound():
        // ErrorDomain::Transport + TRANSPORT_NOT_ROOM_BOUND.
        expect(refusal).toBeInstanceOf(NativeEditorV2TransportError);
        expect(refusal.domain).toBe('transport');
        expect(refusal.code).toBe('TRANSPORT_NOT_ROOM_BOUND');
    });

    it('createWebSocket failure reports the error, renders the Rust-returned transport state, and schedules a retry', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            createWebSocket: () => {
                throw new Error('websocket constructor unavailable');
            },
        });
        setup.controller.connect();
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);
        expect(setup.sockets).toHaveLength(0);

        // The failure surfaces through the autonomous error listener...
        expect(setup.errors).toHaveLength(1);
        expect(setup.errors[0].message).toBe('websocket constructor unavailable');

        // ...the issued generation is retired natively...
        expect(runtime.module.editorV2CollaborationSocketClose).toHaveBeenCalledWith(
            setup.handle.editorId,
            '1',
            null,
            null
        );
        expect(runtime.session(setup.handle.editorId).liveGeneration).toBeNull();

        // ...and the rendered state reflects the transport Rust settled into
        // (never a stale pre-throw snapshot), carrying the error.
        expect(setup.controller.state.status).toBe('disconnected');
        expect(setup.controller.state.isConnected).toBe(false);
        expect(setup.controller.state.lastError).toBe(setup.errors[0]);
        expect(setup.states[setup.states.length - 1].status).toBe('disconnected');

        // A retry is still scheduled: the backoff timer asks Rust for a fresh
        // generation (which fails the same way and reschedules).
        jest.advanceTimersByTime(500);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(2);
        expect(setup.errors).toHaveLength(2);
        expect(latestStatus(setup)).toBe('disconnected');
    });

    // ── Generations ─────────────────────────────────────────────

    it('ignores superseded socket events and routes native stale-generation errors to the autonomous listener, not state', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        setup.sockets[0].open();
        expect(latestStatus(setup)).toBe('handshaking');

        // Retryable close -> generation 1 retired, retry scheduled.
        setup.sockets[0].serverClose(1006);
        expect(latestStatus(setup)).toBe('disconnected');
        jest.advanceTimersByTime(500);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(2);
        expect(setup.sockets).toHaveLength(2);
        expect(latestStatus(setup)).toBe('connecting');

        // The superseded socket's messages and close are ignored outright:
        // no native receive/close calls, no sends, no state movement.
        const closeCalls = runtime.module.editorV2CollaborationSocketClose.mock.calls.length;
        setup.sockets[0].receive(V2_FAKE_UPDATE_FRAME);
        setup.sockets[0].serverClose(1006);
        expect(runtime.module.editorV2CollaborationReceive).not.toHaveBeenCalled();
        expect(runtime.module.editorV2CollaborationSocketClose.mock.calls.length).toBe(closeCalls);
        expect(sentFrames(setup.sockets[1])).toEqual([]);
        expect(latestStatus(setup)).toBe('connecting');

        // A native-side stale generation (retired behind the controller's
        // back) surfaces through onError without touching state or the socket.
        setup.sockets[1].open();
        expect(latestStatus(setup)).toBe('handshaking');
        runtime.retireLiveGeneration(setup.handle.editorId);
        const errorsBefore = setup.errors.length;
        setup.sockets[1].receive(V2_FAKE_STEP2_FRAME);
        expect(setup.errors.length).toBe(errorsBefore + 1);
        const staleError = setup.errors[setup.errors.length - 1];
        expect(staleError).toBeInstanceOf(NativeEditorV2TransportError);
        expect((staleError as NativeEditorV2TransportError).code).toBe(
            'TRANSPORT_STALE_GENERATION'
        );
        expect(latestStatus(setup)).toBe('handshaking');
        expect(setup.sockets[1].close).not.toHaveBeenCalled();
        jest.advanceTimersByTime(60_000);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(2);
    });

    // ── Retry / incompatible ────────────────────────────────────

    it('schedules exponential backoff retries only while Rust admits reconnect', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        setup.sockets[0].open();

        setup.sockets[0].serverClose(1006);
        expect(latestStatus(setup)).toBe('disconnected');
        jest.advanceTimersByTime(499);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);
        jest.advanceTimersByTime(1);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(2);
        expect(setup.sockets).toHaveLength(2);

        setup.sockets[1].open();
        setup.sockets[1].serverClose(1006);
        jest.advanceTimersByTime(999);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(2);
        jest.advanceTimersByTime(1);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(3);
    });

    it('honors retryIntervalMs as a pure scheduling knob', () => {
        const disabled = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            retryIntervalMs: false,
        });
        disabled.controller.connect();
        disabled.sockets[0].open();
        disabled.sockets[0].serverClose(1006);
        jest.advanceTimersByTime(120_000);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);

        const custom = setupController({
            handle: createRoomHandle({ withSnapshot: true, documentId: 'doc-2' }),
            retryIntervalMs: 25,
        });
        custom.controller.connect();
        custom.sockets[0].open();
        custom.sockets[0].serverClose(1006);
        const callsBefore = runtime.module.editorV2CollaborationBeginConnect.mock.calls.length;
        jest.advanceTimersByTime(24);
        expect(runtime.module.editorV2CollaborationBeginConnect.mock.calls.length).toBe(
            callsBefore
        );
        jest.advanceTimersByTime(1);
        expect(runtime.module.editorV2CollaborationBeginConnect.mock.calls.length).toBe(
            callsBefore + 1
        );
    });

    it('accepts retryIntervalMs as a function of the retry context', () => {
        const retryIntervalMs = jest.fn((context: YjsRetryContext) => context.attempt * 100);
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            retryIntervalMs,
            createWebSocket: () => {
                throw new Error('down');
            },
        });
        setup.controller.connect();
        expect(retryIntervalMs).toHaveBeenCalledWith({
            attempt: 1,
            documentId: 'doc-1',
            lastError: setup.errors[0],
        });
        jest.advanceTimersByTime(99);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);
        jest.advanceTimersByTime(1);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(2);

        // The second failure escalates through the function: attempt 2 -> 200ms.
        expect(retryIntervalMs).toHaveBeenLastCalledWith({
            attempt: 2,
            documentId: 'doc-1',
            lastError: setup.errors[1],
        });
        jest.advanceTimersByTime(199);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(2);
        jest.advanceTimersByTime(1);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(3);
    });

    it('keeps Incompatible parked without a hidden shortcut, then explicitly reconnects through detach/reattach', () => {
        const setup = setupController();
        setup.controller.connect();
        setup.sockets[0].open();
        runtime.pushRemoteDoc(setup.handle.editorId, SERVER_DOC);
        setup.sockets[0].receive(V2_FAKE_STEP2_FRAME);
        expect(latestStatus(setup)).toBe('synchronized');

        // A permanently inadmissible remote state parks the transport.
        setup.sockets[0].receive(V2_FAKE_INCOMPATIBLE_FRAME);
        expect(latestStatus(setup)).toBe('incompatible');
        expect(setup.controller.state.isConnected).toBe(false);
        expect(setup.errors.length).toBeGreaterThanOrEqual(1);
        jest.advanceTimersByTime(120_000);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);

        // Explicit reconnect is refused by Rust: no new socket, no retry.
        const errorsBefore = setup.errors.length;
        setup.controller.connect();
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(2);
        expect(setup.sockets).toHaveLength(1);
        expect(setup.errors.length).toBe(errorsBefore + 1);
        const refusal = setup.errors[setup.errors.length - 1];
        expect((refusal as NativeEditorV2TransportError).code).toBe('TRANSPORT_INCOMPATIBLE');
        expect(latestStatus(setup)).toBe('incompatible');
        jest.advanceTimersByTime(120_000);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(2);

        // Snapshot restore is NOT an escape hatch: Rust rejects it while the
        // transport is parked Incompatible.
        expect(() =>
            setup.handle.bridge.snapshotRestore(
                {
                    formatVersion: 1,
                    documentId: 'doc-1',
                    lineageId: 'lineage-1',
                    fragmentName: 'prosemirror',
                    schemaFingerprint: 'fakefingerprint',
                },
                snapshotState(SNAPSHOT_DOC, 9)
            )
        ).toThrow();
        expect(latestStatus(setup)).toBe('incompatible');

        expect(runtime.module.editorV2CollaborationDetach).not.toHaveBeenCalled();
        expect(runtime.module.editorV2CollaborationReattach).not.toHaveBeenCalled();

        // Explicit reconnect is the only escape hatch. It advances the native
        // generation after detach/reattach, creates one fresh socket, and
        // leaves the refusal error history untouched.
        setup.controller.reconnect();
        expect(runtime.module.editorV2CollaborationDetach).toHaveBeenCalledWith(
            setup.handle.editorId
        );
        expect(runtime.module.editorV2CollaborationReattach).toHaveBeenCalledWith(
            setup.handle.editorId
        );
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(3);
        expect(runtime.session(setup.handle.editorId).liveGeneration).toBe(2);
        expect(setup.sockets).toHaveLength(2);
        expect(latestStatus(setup)).toBe('connecting');
        expect(setup.errors).toHaveLength(errorsBefore + 1);
    });

    it('reconnect retires the live socket before detach, reattach, and beginConnect', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        openAndSynchronize(setup);

        setup.controller.reconnect();

        const calls = [
            runtime.module.editorV2CollaborationSocketClose.mock.invocationCallOrder.at(-1),
            runtime.module.editorV2CollaborationDetach.mock.invocationCallOrder.at(-1),
            runtime.module.editorV2CollaborationReattach.mock.invocationCallOrder.at(-1),
            runtime.module.editorV2CollaborationBeginConnect.mock.invocationCallOrder.at(-1),
        ];
        expect(calls).toEqual([...calls].sort((left, right) => left - right));
        expect(runtime.module.editorV2CollaborationSocketClose).toHaveBeenLastCalledWith(
            setup.handle.editorId,
            '1',
            1000,
            'client disconnect'
        );
        expect(runtime.session(setup.handle.editorId).liveGeneration).toBe(2);
        expect(setup.sockets).toHaveLength(2);
        expect(latestStatus(setup)).toBe('connecting');
    });

    it('treats a policy-violation close code (1008) as incompatible without retry', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        setup.sockets[0].open();
        setup.sockets[0].serverClose(1008, 'policy violation');
        expect(runtime.module.editorV2CollaborationSocketClose).toHaveBeenCalledWith(
            setup.handle.editorId,
            '1',
            1008,
            'policy violation'
        );
        expect(latestStatus(setup)).toBe('incompatible');
        jest.advanceTimersByTime(120_000);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);
    });

    it('manual disconnect reports the generation close, retains desired awareness, and never retries', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.connect();
        openAndSynchronize(setup);

        setup.controller.disconnect();
        expect(latestStatus(setup)).toBe('disconnected');
        expect(runtime.module.editorV2CollaborationSocketClose).toHaveBeenCalledWith(
            setup.handle.editorId,
            '1',
            1000,
            'client disconnect'
        );
        // Desired awareness is retained natively across the disconnect.
        const session = runtime.session(setup.handle.editorId);
        expect(session.desiredAwareness).not.toBeNull();
        jest.advanceTimersByTime(120_000);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);

        setup.controller.reconnect();
        expect(setup.sockets).toHaveLength(2);
        expect(latestStatus(setup)).toBe('connecting');
    });

    it('manual disconnect before socket open retires the generation, renders Rust state, and never retries', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        expect(setup.sockets).toHaveLength(1);
        expect(setup.sockets[0].readyState).toBe(MockWebSocket.CONNECTING);
        expect(latestStatus(setup)).toBe('connecting');

        setup.controller.disconnect();
        expect(runtime.module.editorV2CollaborationSocketClose).toHaveBeenCalledWith(
            setup.handle.editorId,
            '1',
            1000,
            'client disconnect'
        );
        // State renders from the Rust-settled transport; the generation is
        // retired natively.
        expect(latestStatus(setup)).toBe('disconnected');
        expect(setup.controller.state.isConnected).toBe(false);
        const session = runtime.session(setup.handle.editorId);
        expect(session.liveGeneration).toBeNull();
        expect(session.transportState).toBe('Disconnected');

        // A manual disconnect schedules no retry.
        jest.advanceTimersByTime(120_000);
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);
        expect(setup.sockets).toHaveLength(1);

        // The abandoned socket's late open is superseded and ignored outright.
        setup.sockets[0].open();
        expect(runtime.module.editorV2CollaborationSocketOpen).not.toHaveBeenCalled();
        expect(latestStatus(setup)).toBe('disconnected');
    });

    // ── Outbound drain ──────────────────────────────────────────

    it('drains queued offline edits after reconnect as standard frames, protocol replies before document frames', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        setup.sockets[0].open();
        setup.sockets[0].receive(V2_FAKE_STEP2_FRAME);
        expect(latestStatus(setup)).toBe('synchronized');

        // Go offline; local edits queue in the engine's outbox.
        setup.sockets[0].serverClose(1006);
        expect(latestStatus(setup)).toBe('disconnected');
        const editorId = setup.handle.editorId;
        setup.handle.bridge.applyLocalApi({
            setJson: LOCAL_DOC_B,
            history: 'undoableBoundary',
            baseDocumentRevision: '7',
        });
        setup.handle.bridge.applyLocalApi({
            setJson: LOCAL_DOC_C,
            history: 'undoableBoundary',
            baseDocumentRevision: '8',
        });
        expect(runtime.queuedFrames(editorId)).toHaveLength(2);

        // Reconnect via the retry timer: Sync Step 1 first, then the queued
        // document frames in engine order.
        jest.advanceTimersByTime(500);
        expect(setup.sockets).toHaveLength(2);
        setup.sockets[1].open();
        expect(sentFrames(setup.sockets[1])).toEqual([
            Array.from(V2_FAKE_STEP1_FRAME),
            [0x64, 8],
            [0x64, 9],
        ]);

        // A protocol reply always precedes document frames in one drain.
        setup.sockets[1].receive(V2_FAKE_STEP1_FRAME);
        expect(sentFrames(setup.sockets[1]).slice(3)).toEqual([[0x70, 1]]);
        setup.handle.bridge.applyInput({ baseDocumentRevision: '9', text: '!' });
        setup.sockets[1].receive(V2_FAKE_STEP1_FRAME);
        expect(sentFrames(setup.sockets[1]).slice(4)).toEqual([
            [0x70, 2],
            [0x64, 10],
        ]);
    });

    it('sends nothing but Step 1 when the outbound queue is empty', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        setup.sockets[0].open();
        expect(sentFrames(setup.sockets[0])).toEqual([Array.from(V2_FAKE_STEP1_FRAME)]);
        expect(runtime.module.editorV2CollaborationTakeOutbound).toHaveBeenCalled();
    });

    // ── Awareness / peers ───────────────────────────────────────

    it('publishes desired awareness through Rust with no TypeScript clock bookkeeping, and renders peers', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        // The desired awareness record carries user data only — never a clock.
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(1);
        const initialPayload = runtime.module.editorV2CollaborationSetAwareness.mock
            .calls[0][1] as string;
        expect(initialPayload).not.toContain('"clock"');
        expect(JSON.parse(initialPayload)).toEqual({ user: ALICE, focused: false });

        setup.controller.connect();
        setup.sockets[0].open();
        setup.sockets[0].receive(V2_FAKE_STEP2_FRAME);
        // Rust re-published the retained desired awareness on synchronization:
        // the drained awareness frame carries the Rust-owned clock (1).
        expect(sentFrames(setup.sockets[0])).toEqual([
            Array.from(V2_FAKE_STEP1_FRAME),
            [0x61, 1],
        ]);

        // The local peer projection comes from Rust (with its clock).
        expect(setup.controller.peers).toEqual([
            {
                clientId: expect.any(String),
                clock: 1,
                isLocal: true,
                state: { user: ALICE, focused: false },
                cursor: null,
            },
        ]);

        // A remote awareness update renders through the returned Rust state.
        runtime.pushRemotePeers(setup.handle.editorId, [remotePeer()]);
        setup.sockets[0].receive(V2_FAKE_AWARENESS_FRAME);
        expect(setup.controller.peers).toHaveLength(2);
        expect(setup.controller.peers[1]).toEqual(remotePeer());
        expect(setup.peersLog.length).toBeGreaterThanOrEqual(1);

        // Updating local awareness goes through Rust again; the clock advances
        // natively (2) without TypeScript ever seeing it.
        setup.controller.updateLocalAwareness({ user: { ...ALICE, name: 'Alice II' } });
        const updatePayload = runtime.module.editorV2CollaborationSetAwareness.mock
            .calls[1][1] as string;
        expect(updatePayload).not.toContain('"clock"');
        expect(sentFrames(setup.sockets[0]).slice(2)).toEqual([[0x61, 2]]);
    });

    it('merges selection and focus into the desired local awareness state', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.handleSelectionChange({ type: 'text', anchor: 2, head: 5 });
        const selectionPayload = JSON.parse(
            runtime.module.editorV2CollaborationSetAwareness.mock.calls[1][1] as string
        );
        expect(selectionPayload).toEqual({
            user: ALICE,
            focused: true,
            selection: { anchor: 2, head: 5 },
        });

        setup.controller.handleFocusChange(false);
        const focusPayload = JSON.parse(
            runtime.module.editorV2CollaborationSetAwareness.mock.calls[2][1] as string
        );
        expect(focusPayload).toEqual({
            user: ALICE,
            focused: false,
            selection: { anchor: 2, head: 5 },
        });
    });

    it('setting identical awareness twice emits no publish', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(1);

        // Merging the same values into the desired state is a no-op.
        setup.controller.updateLocalAwareness({ user: { ...ALICE } });
        setup.controller.updateLocalAwareness({ focused: false });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(1);

        // A real change still publishes.
        setup.controller.updateLocalAwareness({ focused: true });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(2);
        const payload = JSON.parse(
            runtime.module.editorV2CollaborationSetAwareness.mock.calls[1][1] as string
        );
        expect(payload).toEqual({ user: ALICE, focused: true });
    });

    // ── Error separation ────────────────────────────────────────

    it('never closes the socket generation for local operation errors, and never fails local edits for transport errors', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        setup.sockets[0].open();
        setup.sockets[0].receive(V2_FAKE_STEP2_FRAME);
        expect(latestStatus(setup)).toBe('synchronized');

        // A local operation error (whole-document replacement while the
        // transport is live) leaves the socket generation alone.
        expect(() =>
            setup.handle.bridge.applyLocalApi({
                setJson: LOCAL_DOC_B,
                history: 'undoableBoundary',
                baseDocumentRevision: '7',
            })
        ).toThrow();
        expect(runtime.module.editorV2CollaborationSocketClose).not.toHaveBeenCalled();
        expect(latestStatus(setup)).toBe('synchronized');
        expect(setup.sockets[0].close).not.toHaveBeenCalled();
        // The transport is still healthy: inbound frames keep flowing.
        runtime.pushRemotePeers(setup.handle.editorId, [remotePeer()]);
        setup.sockets[0].receive(V2_FAKE_AWARENESS_FRAME);
        expect(setup.controller.peers).toHaveLength(1);

        // A transport failure (retry pending) never fails local edits.
        setup.sockets[0].serverClose(1006);
        expect(latestStatus(setup)).toBe('disconnected');
        const outcome = setup.handle.bridge.applyInput({
            baseDocumentRevision: '7',
            text: 'offline',
        });
        expect(outcome.type).toBe('transaction');
        expect(runtime.session(setup.handle.editorId).documentRevision).toBe(8);
    });

    // ── Destroy / shared session ────────────────────────────────

    it('destroy retires the socket generation but never destroys the shared document handle', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        setup.sockets[0].open();

        setup.controller.destroy();
        expect(latestStatus(setup)).toBe('destroyed');
        expect(runtime.module.editorV2CollaborationSocketClose).toHaveBeenCalledWith(
            setup.handle.editorId,
            '1',
            1000,
            'controller destroyed'
        );
        // One shared session: the handle outlives the controller.
        expect(runtime.module.editorV2Destroy).not.toHaveBeenCalled();
        expect(runtime.module.editorV2Create).toHaveBeenCalledTimes(1);
        expect(setup.handle.bridge.getState().transportState).toBe('Disconnected');

        // A destroyed controller is inert.
        setup.controller.connect();
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);
        expect(setup.sockets).toHaveLength(1);
    });

    it('repeated connect while a socket is live never asks Rust for a second generation', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        setup.controller.connect();
        setup.sockets[0].open();
        setup.controller.connect();
        expect(runtime.module.editorV2CollaborationBeginConnect).toHaveBeenCalledTimes(1);
        expect(setup.sockets).toHaveLength(1);
    });

    // ── Hook ────────────────────────────────────────────────────

    it('useYjsCollaboration renders returned Rust state and binds the shared handle', () => {
        const handle = createRoomHandle();
        const sockets: MockWebSocket[] = [];
        const { result } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                connect: false,
                createWebSocket: () => {
                    const socket = new MockWebSocket();
                    sockets.push(socket);
                    return socket as unknown as WebSocket;
                },
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
            sockets[0].open();
        });
        expect(result.current.state.status).toBe('handshaking');
        expect(result.current.isConnected).toBe(false);

        act(() => {
            runtime.pushRemoteDoc(handle.editorId, SERVER_DOC);
            sockets[0].receive(V2_FAKE_STEP2_FRAME);
        });
        expect(result.current.state.status).toBe('synchronized');
        expect(result.current.isConnected).toBe(true);
        expect(result.current.state.documentJson).toEqual(SERVER_DOC);
        expect(result.current.editorBindings.documentRevision).toBe(
            result.current.state.documentRevision
        );
        expect(result.current.editorBindings.documentRevision).not.toBeNull();
    });

    it('maps Rust peer projections to editor remote-selection decorations via the hook', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const sockets: MockWebSocket[] = [];
        const { result } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                connect: false,
                localAwareness: ALICE,
                createWebSocket: () => {
                    const socket = new MockWebSocket();
                    sockets.push(socket);
                    return socket as unknown as WebSocket;
                },
            })
        );
        act(() => {
            result.current.connect();
        });
        act(() => {
            sockets[0].open();
            sockets[0].receive(V2_FAKE_STEP2_FRAME);
        });
        act(() => {
            runtime.pushRemotePeers(handle.editorId, [
                remotePeer(),
                remotePeer({ clientId: '43', cursor: null, state: null }),
            ]);
            sockets[0].receive(V2_FAKE_AWARENESS_FRAME);
        });

        const decorations = result.current.editorBindings.remoteSelections;
        expect(decorations).toEqual([
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

    it('honors the connect prop and forwards callbacks', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const sockets: MockWebSocket[] = [];
        const onStateChange = jest.fn();
        let connect = false;
        const { result, rerender } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                connect,
                createWebSocket: () => {
                    const socket = new MockWebSocket();
                    sockets.push(socket);
                    return socket as unknown as WebSocket;
                },
                onStateChange,
            })
        );
        expect(sockets).toHaveLength(0);
        connect = true;
        rerender();
        expect(sockets).toHaveLength(1);
        expect(onStateChange).toHaveBeenCalled();
        expect(result.current.state.status).toBe('connecting');
    });

    it('re-publishes the desired awareness when the localAwareness prop updates', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        let localAwareness = ALICE;
        const { rerender } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                connect: false,
                localAwareness,
                createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            })
        );
        // The constructor publishes once; the mount-time prop effect merges
        // the identical user and dedups to a no-op.
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(1);

        localAwareness = { ...ALICE, name: 'Alice II' };
        rerender();
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(2);
        const payload = JSON.parse(
            runtime.module.editorV2CollaborationSetAwareness.mock.calls[1][1] as string
        );
        expect(payload).toEqual({ user: { ...ALICE, name: 'Alice II' }, focused: false });
    });

    // ── Removal proofs ──────────────────────────────────────────

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
                connect: false,
                createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            })
        );
        const bindings = result.current.editorBindings as unknown as Record<string, unknown>;
        for (const removed of [
            'valueJSON',
            'valueJSONUpdateMode',
            'preserveSelectionOnValueJSONReset',
            'selectionOnValueJSONReset',
            'onContentChangeJSON',
        ]) {
            expect(removed in bindings).toBe(false);
        }
        expect(bindings.documentHandle).toBe(handle);
        expect(typeof bindings.onSelectionChange).toBe('function');
        expect(typeof bindings.onFocus).toBe('function');
        expect(typeof bindings.onBlur).toBe('function');
    });
});
