// ─── YjsCollaboration (Task 10 awareness scheduling) Tests ─────
// The legacy collaboration controller owned session creation, retry
// decisions, awareness clocks, valueJSON reset sync, and raw encoded-state
// APIs. All of that is removed. The current controller is a thin shell over
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
    createNativeEditorLocalAwarenessSelection,
    createNativeEditorDocumentHandle,
    type NativeEditorDocumentHandle,
    _resetNativeModuleCache,
    type DocumentJSON,
    type NativeEditorLocalAwarenessIntent,
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
const AWARENESS_CLOCK_EXHAUSTED_ERROR = {
    domain: 'transport',
    code: 'AWARENESS_CLOCK_EXHAUSTED',
    message: 'local awareness clock exhausted; a fresh editor identity is required',
    requestId: null,
    operationIndex: null,
    limit: null,
    actual: null,
    details: {
        requiresFreshEditorIdentity: true,
        retryable: false,
    },
};
const MALFORMED_FAKE_AWARENESS_MESSAGE =
    'awareness update cannot decode: fake entry requires canonical u64 clientId and exact u32 clock';

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

const MAX_HOST_TIMER_DELAY_MILLIS = 2_147_483_647;

/**
 * A manual monotonic clock and timer host. Advancing time fires every timer
 * already due at the final monotonic instant, deliberately exercising late
 * callbacks without borrowing Jest's wall clock.
 */
class FakeMonotonicClockTimer {
    private _nowMillis: bigint;
    private nextTimerId = 1;
    private readonly timers = new Map<
        number,
        { callback: () => void; deadlineMillis: bigint }
    >();
    readonly scheduledDelays: number[] = [];
    clearCalls = 0;

    constructor(nowMillis = 0n) {
        this._nowMillis = nowMillis;
    }

    nowMillis = (): bigint => this._nowMillis;

    setTimeout = (callback: () => void, delayMillis: number): number => {
        if (!Number.isSafeInteger(delayMillis) || delayMillis < 0) {
            throw new Error(`invalid fake timer delay ${delayMillis}`);
        }
        const timerId = this.nextTimerId++;
        this.scheduledDelays.push(delayMillis);
        this.timers.set(timerId, {
            callback,
            deadlineMillis: this._nowMillis + BigInt(delayMillis),
        });
        return timerId;
    };

    clearTimeout = (timerId: number): void => {
        this.clearCalls += 1;
        this.timers.delete(timerId);
    };

    get activeTimerCount(): number {
        return this.timers.size;
    }

    elapseWithoutFiringTimers(millis: bigint): void {
        if (millis < 0n) throw new Error('fake monotonic time cannot move backwards');
        this._nowMillis += millis;
    }

    advanceBy(millis: bigint): void {
        if (millis < 0n) throw new Error('fake monotonic time cannot move backwards');
        this._nowMillis += millis;
        for (;;) {
            const ready = [...this.timers.entries()]
                .filter(([, timer]) => timer.deadlineMillis <= this._nowMillis)
                .sort(([, left], [, right]) =>
                    left.deadlineMillis < right.deadlineMillis
                        ? -1
                        : left.deadlineMillis > right.deadlineMillis
                          ? 1
                          : 0
                )[0];
            if (ready == null) return;
            const [timerId, timer] = ready;
            this.timers.delete(timerId);
            timer.callback();
        }
    }
}

type TestControllerOptions = Partial<YjsCollaborationOptions> & {
    monotonicClock?: FakeMonotonicClockTimer;
    awarenessTimer?: FakeMonotonicClockTimer;
};

function rejectedAwarenessResult(message: string): Record<string, unknown> {
    return {
        value: null,
        error: {
            domain: 'boundary',
            code: 'AWARENESS_STATE_INVALID',
            message,
            requestId: null,
            operationIndex: null,
            limit: null,
            actual: null,
            details: null,
        },
    };
}

function localAwarenessIntent(
    state: Record<string, unknown> = { user: ALICE },
    focused = false
): NativeEditorLocalAwarenessIntent {
    return { state, focused };
}

// ─── Setup helpers ──────────────────────────────────────────────

let runtime: FakeNativeEditorV2Runtime;

function fakeAwarenessAudit(editorId: string) {
    const session = runtime.session(editorId);
    return {
        desiredAwareness: JSON.parse(JSON.stringify(session.desiredAwareness)) as unknown,
        localClock: session.localClock,
        localAwarenessLive: session.localAwarenessLive,
        queuedFrames: runtime.queuedFrames(editorId).map((frame) => Array.from(frame)),
        awarenessNowMillis: session.awarenessNowMillis,
        lastLocalAwarenessPublishMillis: session.lastLocalAwarenessPublishMillis,
        peers: handlePeers(editorId),
        remoteAwarenessClocks: [...session.remoteAwarenessClocks.entries()],
        remotePeerActivity: [...session.remotePeerActivity.entries()],
        transportState: session.transportState,
        liveGeneration: session.liveGeneration,
    };
}

function handlePeers(editorId: string): NativeEditorV2PeerInfo[] {
    return runtime.session(editorId).remotePeers.map((peer) => ({ ...peer }));
}

function pendingAwarenessTombstone(editorId: string): number[] | null {
    const pending = (
        runtime.session(editorId) as unknown as {
            pendingLocalAwarenessTombstone?: Uint8Array | null;
        }
    ).pendingLocalAwarenessTombstone;
    return pending == null ? null : Array.from(pending);
}

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
    overrides: TestControllerOptions & { handle?: NativeEditorDocumentHandle } = {}
): ControllerSetup {
    const sockets: MockWebSocket[] = [];
    const states: YjsCollaborationState[] = [];
    const errors: Error[] = [];
    const peersLog: NativeEditorV2PeerInfo[][] = [];
    const handle = overrides.handle ?? createRoomHandle();
    const controller = createYjsCollaborationController(
        {
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
        } as YjsCollaborationOptions
    );
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

describe('YjsCollaboration (Task 10 awareness controller)', () => {
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
        expect(setup.errors).toHaveLength(2);
        expect(setup.errors[0]).toMatchObject({
            domain: 'boundary',
            code: 'CONFIG_INVALID',
        });
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
        expect(setup.errors).toHaveLength(2);
        const refusal = setup.errors.at(-1) as NativeEditorV2TransportError;
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

    it('issues u64 generations exactly and refuses exhaustion atomically', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const u64Max = '18446744073709551615';
        runtime.seedLastIssuedGeneration(handle.editorId, '18446744073709551614');

        expect(handle.bridge.collaborationBeginConnect()).toBe(u64Max);
        expect(runtime.session(handle.editorId).liveGeneration).toBe(18_446_744_073_709_551_615n);
        expect(runtime.session(handle.editorId).lastIssuedGeneration).toBe(
            18_446_744_073_709_551_615n
        );
        handle.bridge.collaborationSocketClose(u64Max, 1000, 'seeded max closed');

        for (let attempt = 0; attempt < 2; attempt += 1) {
            let exhaustion: unknown;
            try {
                handle.bridge.collaborationBeginConnect();
            } catch (error) {
                exhaustion = error;
            }
            expect(exhaustion).toBeInstanceOf(NativeEditorV2TransportError);
            expect(exhaustion).toMatchObject({
                domain: 'transport',
                code: 'TRANSPORT_GENERATION_EXHAUSTED',
                message: 'transport generation space is exhausted',
                details: {
                    action: 'beginConnect',
                    transportState: 'Disconnected',
                },
            });
            expect(handle.bridge.getState().transportState).toBe('Disconnected');
            expect(runtime.session(handle.editorId).liveGeneration).toBeNull();
            expect(runtime.session(handle.editorId).lastIssuedGeneration).toBe(
                18_446_744_073_709_551_615n
            );
        }
    });

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
        expect(runtime.session(setup.handle.editorId).liveGeneration).toBe(2n);
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
        expect(runtime.session(setup.handle.editorId).liveGeneration).toBe(2n);
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

    it('models the deterministic awareness renewal, expiry, flags, and next deadline', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        handle.bridge.collaborationSetAwareness(localAwarenessIntent());
        const generation = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(generation);
        handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);

        expect(handle.bridge.collaborationTick('14999')).toEqual({
            nextDeadlineMillis: '15000',
            renewedLocal: false,
            expiredPeers: [],
            outboundChanged: false,
            peersChanged: false,
        });
        expect(() => handle.bridge.collaborationTick('14998')).toThrow(
            expect.objectContaining({
                code: 'AWARENESS_TIME_REGRESSION',
                details: { nowMillis: '14998', lastNowMillis: '14999' },
            })
        );

        runtime.pushRemotePeers(handle.editorId, [
            remotePeer({ clientId: '10' }),
            remotePeer({ clientId: '2' }),
        ]);
        handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME);

        expect(handle.bridge.collaborationTick('15000')).toEqual({
            nextDeadlineMillis: '30000',
            renewedLocal: true,
            expiredPeers: [],
            outboundChanged: true,
            peersChanged: true,
        });
        expect(handle.bridge.collaborationTick('44998')).toEqual({
            nextDeadlineMillis: '44999',
            renewedLocal: true,
            expiredPeers: [],
            outboundChanged: true,
            peersChanged: true,
        });
        expect(handle.bridge.collaborationTick('44999')).toEqual({
            nextDeadlineMillis: '59998',
            renewedLocal: false,
            expiredPeers: ['2', '10'],
            outboundChanged: false,
            peersChanged: true,
        });
        expect(handle.bridge.collaborationPeers()).toEqual([
            expect.objectContaining({ isLocal: true, clock: 4 }),
        ]);
    });

    it('omits overflowing awareness deadlines while retaining representable candidates', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const generation = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(generation);
        handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);

        handle.bridge.collaborationTick('18446744073709521615');
        runtime.pushRemotePeers(handle.editorId, [remotePeer({ clientId: '7', clock: 1 })]);
        handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME);
        handle.bridge.collaborationSetAwareness(localAwarenessIntent());

        expect(handle.bridge.collaborationTick('18446744073709541615')).toEqual({
            nextDeadlineMillis: '18446744073709551615',
            renewedLocal: true,
            expiredPeers: [],
            outboundChanged: true,
            peersChanged: true,
        });
        expect(handle.bridge.collaborationTick('18446744073709551615')).toEqual({
            nextDeadlineMillis: null,
            renewedLocal: false,
            expiredPeers: ['7'],
            outboundChanged: false,
            peersChanged: true,
        });
    });

    it('renews local awareness at exactly 15 seconds, drains the current generation, and retains one timer', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        setup.controller.connect();
        openAndSynchronize(setup);

        expect(runtime.module.editorV2CollaborationTick).toHaveBeenLastCalledWith(
            setup.handle.editorId,
            '0'
        );
        expect(clock.activeTimerCount).toBe(1);
        expect(clock.scheduledDelays).toEqual([15_000]);

        clock.advanceBy(14_999n);
        expect(runtime.module.editorV2CollaborationTick).toHaveBeenCalledTimes(2);
        expect(sentFrames(setup.sockets[0]).at(-1)).toEqual([0x61, 2]);

        clock.advanceBy(1n);
        expect(runtime.module.editorV2CollaborationTick).toHaveBeenLastCalledWith(
            setup.handle.editorId,
            '15000'
        );
        expect(sentFrames(setup.sockets[0]).at(-1)).toEqual([0x61, 3]);
        expect(runtime.module.editorV2CollaborationTick).toHaveBeenCalledTimes(3);
        expect(clock.activeTimerCount).toBe(1);
        expect(clock.scheduledDelays).toEqual([15_000, 15_000]);
    });

    it('expires remote peers at exactly 30 seconds and cancels when Rust returns no deadline', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        setup.controller.connect();
        openAndSynchronize(setup);
        expect(clock.activeTimerCount).toBe(0);

        runtime.pushRemotePeers(setup.handle.editorId, [remotePeer({ clientId: '7' })]);
        setup.sockets[0].receive(V2_FAKE_AWARENESS_FRAME);
        expect(setup.controller.peers).toEqual([remotePeer({ clientId: '7' })]);
        expect(clock.activeTimerCount).toBe(1);
        expect(clock.scheduledDelays).toEqual([30_000]);

        clock.advanceBy(29_999n);
        expect(setup.controller.peers).toHaveLength(1);

        clock.advanceBy(1n);
        expect(runtime.module.editorV2CollaborationTick).toHaveBeenLastCalledWith(
            setup.handle.editorId,
            '30000'
        );
        expect(setup.controller.peers).toEqual([]);
        expect(clock.activeTimerCount).toBe(0);
    });

    it('cancels the previous awareness timer before replacing it and during teardown', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        setup.controller.connect();
        openAndSynchronize(setup);
        expect(clock.activeTimerCount).toBe(1);

        const clearsBeforeUpdate = clock.clearCalls;
        setup.controller.handleFocusChange(true);
        expect(clock.clearCalls).toBe(clearsBeforeUpdate + 1);
        expect(clock.activeTimerCount).toBe(1);

        const tickCallsBeforeDestroy = runtime.module.editorV2CollaborationTick.mock.calls.length;
        const clearsBeforeDestroy = clock.clearCalls;
        setup.controller.destroy();
        expect(clock.clearCalls).toBe(clearsBeforeDestroy + 1);
        expect(clock.activeTimerCount).toBe(0);

        clock.advanceBy(60_000n);
        expect(runtime.module.editorV2CollaborationTick).toHaveBeenCalledTimes(
            tickCallsBeforeDestroy
        );
    });

    it('uses bigint deadlines, clamps only the host delay, and ticks once at a late callback time', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        const tick = jest
            .spyOn(setup.handle.bridge, 'collaborationTick')
            .mockImplementation((nowMillis) => ({
                nextDeadlineMillis:
                    tick.mock.calls.length <= 2
                        ? '9007199254740993'
                        : (BigInt(nowMillis) + 5n).toString(),
                renewedLocal: false,
                expiredPeers: [],
                outboundChanged: false,
                peersChanged: false,
            }));

        setup.controller.connect();
        openAndSynchronize(setup);
        expect(tick).toHaveBeenCalledTimes(2);
        expect(clock.scheduledDelays).toEqual([MAX_HOST_TIMER_DELAY_MILLIS]);
        expect(clock.activeTimerCount).toBe(1);

        clock.advanceBy(BigInt(MAX_HOST_TIMER_DELAY_MILLIS) + 9n);
        expect(tick).toHaveBeenCalledTimes(3);
        expect(tick).toHaveBeenLastCalledWith(
            (BigInt(MAX_HOST_TIMER_DELAY_MILLIS) + 9n).toString()
        );
        expect(clock.scheduledDelays).toEqual([MAX_HOST_TIMER_DELAY_MILLIS, 5]);
        expect(clock.activeTimerCount).toBe(1);
    });

    it('stamps remote awareness at the current time after a long idle before scheduling its 30-second expiry', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        setup.controller.connect();
        openAndSynchronize(setup);
        runtime.module.editorV2CollaborationTick.mockClear();

        clock.elapseWithoutFiringTimers(60_000n);
        runtime.pushRemotePeers(setup.handle.editorId, [remotePeer({ clientId: '7' })]);
        setup.sockets[0].receive(V2_FAKE_AWARENESS_FRAME);

        expect(runtime.module.editorV2CollaborationTick).toHaveBeenCalledTimes(2);
        expect(runtime.session(setup.handle.editorId).remotePeerActivity.get('7')).toBe(60_000n);
        expect(setup.controller.peers).toEqual([remotePeer({ clientId: '7' })]);
        expect(clock.scheduledDelays.at(-1)).toBe(30_000);
        expect(clock.activeTimerCount).toBe(1);
    });

    it('stamps retained awareness republished by a handshake at current time without immediate renewal', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        setup.controller.connect();
        setup.sockets[0].open();
        runtime.module.editorV2CollaborationTick.mockClear();

        clock.elapseWithoutFiringTimers(60_000n);
        setup.sockets[0].receive(V2_FAKE_STEP2_FRAME);

        expect(runtime.module.editorV2CollaborationTick).toHaveBeenCalledTimes(2);
        expect(runtime.session(setup.handle.editorId)).toMatchObject({
            awarenessNowMillis: 60_000n,
            lastLocalAwarenessPublishMillis: 60_000n,
            localClock: 2,
        });
        expect(sentFrames(setup.sockets[0])).toEqual([
            Array.from(V2_FAKE_STEP1_FRAME),
            [0x61, 2],
        ]);
        expect(clock.scheduledDelays.at(-1)).toBe(15_000);
        expect(clock.activeTimerCount).toBe(1);
    });

    it('brackets local awareness publication after a long idle and schedules renewal 15 seconds later', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        setup.controller.connect();
        openAndSynchronize(setup);
        runtime.module.editorV2CollaborationTick.mockClear();

        clock.elapseWithoutFiringTimers(60_000n);
        setup.controller.updateLocalAwareness({ user: ALICE });

        expect(runtime.module.editorV2CollaborationTick).toHaveBeenCalledTimes(2);
        const [preTickOrder, postTickOrder] =
            runtime.module.editorV2CollaborationTick.mock.invocationCallOrder;
        const setOrder =
            runtime.module.editorV2CollaborationSetAwareness.mock.invocationCallOrder.at(-1);
        expect(preTickOrder).toBeLessThan(setOrder as number);
        expect(setOrder).toBeLessThan(postTickOrder as number);
        expect(runtime.session(setup.handle.editorId)).toMatchObject({
            awarenessNowMillis: 60_000n,
            lastLocalAwarenessPublishMillis: 60_000n,
            localClock: 1,
        });
        expect(sentFrames(setup.sockets[0])).toEqual([
            Array.from(V2_FAKE_STEP1_FRAME),
            [0x61, 1],
        ]);
        expect(clock.scheduledDelays.at(-1)).toBe(15_000);
        expect(clock.activeTimerCount).toBe(1);
    });

    it('retains a first live awareness candidate and retries a saturated broadcast without closing', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        setup.controller.connect();
        openAndSynchronize(setup);
        expect(clock.activeTimerCount).toBe(0);
        runtime.module.editorV2CollaborationSetAwareness.mockClear();

        runtime.session(setup.handle.editorId).protocolQueue.push(new Uint8Array([0x70, 0x7f]));
        runtime.injectNextAwarenessBroadcastFailure(
            setup.handle.editorId,
            'TRANSPORT_REPLY_LIMIT_EXCEEDED'
        );
        setup.controller.updateLocalAwareness({ user: ALICE });

        expect(runtime.session(setup.handle.editorId)).toMatchObject({
            desiredAwareness: localAwarenessIntent(),
            lastLocalAwarenessPublishMillis: null,
            transportState: 'Synchronized',
            liveGeneration: 1n,
        });
        expect(setup.errors.at(-1)).toEqual(
            expect.objectContaining({
                code: 'TRANSPORT_REPLY_LIMIT_EXCEEDED',
            })
        );
        expect(setup.sockets[0].close).not.toHaveBeenCalled();
        expect(sentFrames(setup.sockets[0]).at(-1)).toEqual([0x70, 0x7f]);
        expect(runtime.queuedFrames(setup.handle.editorId)).toEqual([]);
        expect(clock.activeTimerCount).toBe(1);
        const retryDelay = clock.scheduledDelays.at(-1) as number;
        expect(retryDelay).toBe(100);

        const retainedSetCalls = runtime.module.editorV2CollaborationSetAwareness.mock.calls.length;
        setup.controller.updateLocalAwareness({ user: { ...ALICE } });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(
            retainedSetCalls
        );

        clock.advanceBy(BigInt(retryDelay));

        expect(sentFrames(setup.sockets[0]).at(-1)).toEqual([0x61, 2]);
        expect(runtime.session(setup.handle.editorId).lastLocalAwarenessPublishMillis).toBe(
            BigInt(retryDelay)
        );
        expect(clock.scheduledDelays.at(-1)).toBe(15_000);
        expect(clock.activeTimerCount).toBe(1);
        expect(setup.sockets[0].close).not.toHaveBeenCalled();
    });

    it('retries a saturated renewal without tearing down its live generation or looping', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        setup.controller.connect();
        openAndSynchronize(setup);
        runtime.session(setup.handle.editorId).protocolQueue.push(new Uint8Array([0x70, 0x80]));
        runtime.injectNextAwarenessBroadcastFailure(
            setup.handle.editorId,
            'TRANSPORT_RESOURCE_EXHAUSTED'
        );

        clock.advanceBy(15_000n);

        expect(runtime.session(setup.handle.editorId)).toMatchObject({
            desiredAwareness: localAwarenessIntent(),
            lastLocalAwarenessPublishMillis: 0n,
            transportState: 'Synchronized',
            liveGeneration: 1n,
        });
        expect(setup.errors.at(-1)).toEqual(
            expect.objectContaining({
                code: 'TRANSPORT_RESOURCE_EXHAUSTED',
            })
        );
        expect(setup.sockets[0].close).not.toHaveBeenCalled();
        expect(sentFrames(setup.sockets[0]).at(-1)).toEqual([0x70, 0x80]);
        expect(runtime.queuedFrames(setup.handle.editorId)).toEqual([]);
        expect(clock.activeTimerCount).toBe(1);
        const retryDelay = clock.scheduledDelays.at(-1) as number;
        expect(retryDelay).toBe(100);

        clock.advanceBy(BigInt(retryDelay));

        expect(sentFrames(setup.sockets[0]).at(-1)).toEqual([0x61, 4]);
        expect(runtime.session(setup.handle.editorId).lastLocalAwarenessPublishMillis).toBe(
            15_000n + BigInt(retryDelay)
        );
        expect(clock.scheduledDelays.at(-1)).toBe(15_000);
        expect(clock.activeTimerCount).toBe(1);
        expect(setup.sockets[0].close).not.toHaveBeenCalled();
    });

    it.each([
        'TRANSPORT_REPLY_LIMIT_EXCEEDED',
        'TRANSPORT_RESOURCE_EXHAUSTED',
    ] as const)(
        'withdraws through %s, keeps hooks inert, and retries one tombstone on the one timer',
        (code) => {
            const clock = new FakeMonotonicClockTimer();
            const handle = createRoomHandle({
                documentId: `withdrawal-recovery-${code}`,
                withSnapshot: true,
            });
            const sockets: MockWebSocket[] = [];
            const errors: Error[] = [];
            let localAwareness: typeof ALICE | undefined = ALICE;
            const { result, rerender, unmount } = renderHook(() =>
                useYjsCollaboration({
                    documentId: `withdrawal-recovery-${code}`,
                    handle,
                    connect: true,
                    localAwareness,
                    monotonicClock: clock,
                    awarenessTimer: clock,
                    createWebSocket: () => {
                        const socket = new MockWebSocket();
                        sockets.push(socket);
                        return socket as unknown as WebSocket;
                    },
                    onError: (error) => errors.push(error),
                })
            );
            act(() => {
                sockets[0].open();
                sockets[0].receive(V2_FAKE_STEP2_FRAME);
            });
            const awarenessFramesBeforeClear = sentFrames(sockets[0]).filter(
                ([type]) => type === 0x61
            ).length;
            const clockBeforeClear = runtime.session(handle.editorId).localClock;
            runtime.session(handle.editorId).protocolQueue.push(new Uint8Array([0x70, 0x91]));
            runtime.injectNextAwarenessBroadcastFailure(handle.editorId, code);

            localAwareness = undefined;
            rerender();

            expect(errors.at(-1)).toMatchObject(
                code === 'TRANSPORT_REPLY_LIMIT_EXCEEDED'
                    ? {
                          domain: 'transport',
                          code,
                          message:
                              'maxPendingOutboxMessages exceeded while enqueueing an awareness broadcast',
                          limit: '1',
                          actual: '2',
                          details: {
                              action: 'awareness',
                              field: 'maxPendingOutboxMessages',
                              limit: 1,
                              actual: 2,
                          },
                      }
                    : {
                          domain: 'transport',
                          code,
                          message: 'awareness broadcast capacity could not be reserved',
                          limit: null,
                          actual: null,
                          details: null,
                      }
            );
            expect(runtime.session(handle.editorId)).toMatchObject({
                desiredAwareness: null,
                localAwarenessLive: false,
                localClock: clockBeforeClear + 1,
                lastLocalAwarenessPublishMillis: null,
                transportState: 'Synchronized',
                liveGeneration: 1n,
            });
            expect(pendingAwarenessTombstone(handle.editorId)).toEqual([
                0x61,
                (clockBeforeClear + 1) & 0xff,
            ]);
            expect(sentFrames(sockets[0]).at(-1)).toEqual([0x70, 0x91]);
            expect(sentFrames(sockets[0]).filter(([type]) => type === 0x61)).toHaveLength(
                awarenessFramesBeforeClear
            );
            expect(sockets[0].close).not.toHaveBeenCalled();
            expect(clock.scheduledDelays.at(-1)).toBe(100);
            expect(clock.activeTimerCount).toBe(1);

            const setCallsAfterClear =
                runtime.module.editorV2CollaborationSetAwareness.mock.calls.length;
            act(() => {
                result.current.editorBindings.onFocus();
                result.current.editorBindings.onSelectionChange({
                    type: 'text',
                    anchor: 1,
                    head: 3,
                });
                result.current.editorBindings.onBlur();
            });
            expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(
                setCallsAfterClear
            );

            handle.bridge.collaborationSetAwareness(null);
            expect(runtime.session(handle.editorId).localClock).toBe(clockBeforeClear + 1);
            expect(pendingAwarenessTombstone(handle.editorId)).toEqual([
                0x61,
                (clockBeforeClear + 1) & 0xff,
            ]);

            const tickCallsBeforeRetry = runtime.module.editorV2CollaborationTick.mock.calls.length;
            clock.advanceBy(100n);

            expect(runtime.module.editorV2CollaborationTick).toHaveBeenCalledTimes(
                tickCallsBeforeRetry + 1
            );
            expect(pendingAwarenessTombstone(handle.editorId)).toEqual([
                0x61,
                (clockBeforeClear + 1) & 0xff,
            ]);
            expect(sentFrames(sockets[0]).filter(([type]) => type === 0x61)).toHaveLength(
                awarenessFramesBeforeClear
            );
            expect(clock.scheduledDelays.at(-1)).toBe(14_900);
            expect(clock.activeTimerCount).toBe(1);

            clock.advanceBy(14_900n);
            expect(runtime.module.editorV2CollaborationTick).toHaveBeenCalledTimes(
                tickCallsBeforeRetry + 2
            );
            expect(pendingAwarenessTombstone(handle.editorId)).toBeNull();
            expect(sentFrames(sockets[0]).filter(([type]) => type === 0x61)).toHaveLength(
                awarenessFramesBeforeClear + 1
            );
            expect(sentFrames(sockets[0]).at(-1)).toEqual([
                0x61,
                (clockBeforeClear + 1) & 0xff,
            ]);
            expect(clock.activeTimerCount).toBe(0);

            act(() => {
                result.current.reconnect();
                sockets[1].open();
                sockets[1].receive(V2_FAKE_STEP2_FRAME);
            });
            expect(sentFrames(sockets[1]).filter(([type]) => type === 0x61)).toEqual([]);
            expect(runtime.session(handle.editorId).desiredAwareness).toBeNull();
            unmount();
            expect(clock.activeTimerCount).toBe(0);
            expect(sockets[1].close).toHaveBeenCalledTimes(1);
        }
    );

    it('rolls back the JS withdrawal after a non-recoverable native clear refusal', () => {
        const clock = new FakeMonotonicClockTimer();
        const handle = createRoomHandle({ withSnapshot: true });
        const sockets: MockWebSocket[] = [];
        const errors: Error[] = [];
        let localAwareness: typeof ALICE | undefined = ALICE;
        const { result, rerender } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'non-recoverable-withdrawal',
                handle,
                connect: true,
                localAwareness,
                monotonicClock: clock,
                awarenessTimer: clock,
                createWebSocket: () => {
                    const socket = new MockWebSocket();
                    sockets.push(socket);
                    return socket as unknown as WebSocket;
                },
                onError: (error) => errors.push(error),
            })
        );
        act(() => {
            sockets[0].open();
            sockets[0].receive(V2_FAKE_STEP2_FRAME);
        });
        runtime.seedLocalAwarenessClock(handle.editorId, 4_294_967_295);

        localAwareness = undefined;
        rerender();
        expect(errors.at(-1)).toMatchObject({ code: 'AWARENESS_CLOCK_EXHAUSTED' });
        expect(runtime.session(handle.editorId).desiredAwareness).toEqual(
            localAwarenessIntent()
        );
        expect(sockets[0].close).not.toHaveBeenCalled();

        const setCallsAfterRefusal =
            runtime.module.editorV2CollaborationSetAwareness.mock.calls.length;
        act(() => result.current.editorBindings.onFocus());
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(
            setCallsAfterRefusal + 1
        );
        expect(errors.at(-1)).toMatchObject({ code: 'AWARENESS_CLOCK_EXHAUSTED' });
        expect(sockets[0].close).not.toHaveBeenCalled();
    });

    it('makes detach and reattach idempotent without reopening an incompatible transport early', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const generation = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(generation);
        handle.bridge.collaborationReceive(generation, V2_FAKE_INCOMPATIBLE_FRAME);

        expect(() => handle.bridge.collaborationBeginConnect()).toThrow(
            expect.objectContaining({ code: 'TRANSPORT_INCOMPATIBLE' })
        );
        handle.bridge.collaborationDetach();
        handle.bridge.collaborationDetach();
        expect(handle.bridge.getState().transportState).toBe('Detached');
        handle.bridge.collaborationReattach();
        handle.bridge.collaborationReattach();
        expect(handle.bridge.getState().transportState).toBe('Disconnected');
        expect(handle.bridge.collaborationBeginConnect()).toBe('2');
    });

    it('mutates local awareness offline and handshaking, then republishes and tombstones it', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        handle.bridge.collaborationSetAwareness(localAwarenessIntent());
        expect(handle.bridge.collaborationPeers()).toEqual([
            expect.objectContaining({
                isLocal: true,
                clock: 1,
                state: { state: { user: ALICE }, focused: false },
            }),
        ]);
        expect(runtime.queuedFrames(handle.editorId)).toEqual([]);

        const firstGeneration = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(firstGeneration);
        const updatedLocal = { ...ALICE, name: 'Alice II' };
        handle.bridge.collaborationSetAwareness(localAwarenessIntent({ user: updatedLocal }));
        expect(handle.bridge.collaborationPeers()).toEqual([
            expect.objectContaining({
                isLocal: true,
                clock: 2,
                state: { state: { user: updatedLocal }, focused: false },
            }),
        ]);
        expect(runtime.queuedFrames(handle.editorId)).toEqual([]);

        handle.bridge.collaborationReceive(firstGeneration, V2_FAKE_STEP2_FRAME);
        expect(handle.bridge.collaborationPeers()).toEqual([
            expect.objectContaining({
                isLocal: true,
                clock: 3,
                state: { state: { user: updatedLocal }, focused: false },
            }),
        ]);
        expect(runtime.queuedFrames(handle.editorId).map((frame) => Array.from(frame))).toEqual([
            [0x61, 3],
        ]);

        handle.bridge.collaborationDetach();
        expect(runtime.session(handle.editorId).desiredAwareness).toEqual(
            localAwarenessIntent({ user: updatedLocal })
        );
        expect(handle.bridge.collaborationPeers()).toEqual([]);
        handle.bridge.collaborationReattach();
        expect(handle.bridge.collaborationPeers()).toEqual([]);

        const secondGeneration = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(secondGeneration);
        handle.bridge.collaborationReceive(secondGeneration, V2_FAKE_STEP2_FRAME);
        expect(handle.bridge.collaborationPeers()).toEqual([
            expect.objectContaining({
                isLocal: true,
                clock: 5,
                state: { state: { user: updatedLocal }, focused: false },
            }),
        ]);

        handle.bridge.collaborationSetAwareness(null);
        expect(runtime.session(handle.editorId).desiredAwareness).toBeNull();
        expect(runtime.session(handle.editorId).localClock).toBe(6);
        expect(handle.bridge.collaborationPeers()).toEqual([]);
    });

    it('rejects malformed desired awareness JSON atomically with the structured boundary error', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const generation = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(generation);
        handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);
        handle.bridge.collaborationTick('12000');
        handle.bridge.collaborationSetAwareness(localAwarenessIntent());

        const before = runtime.session(handle.editorId);
        const desiredBefore = JSON.parse(JSON.stringify(before.desiredAwareness)) as Record<
            string,
            unknown
        >;
        const clockBefore = before.localClock;
        const liveBefore = before.localAwarenessLive;
        const queuedBefore = runtime
            .queuedFrames(handle.editorId)
            .map((frame) => Array.from(frame));
        const nowBefore = before.awarenessNowMillis;
        const publishBefore = before.lastLocalAwarenessPublishMillis;

        expect(
            runtime.module.editorV2CollaborationSetAwareness(handle.editorId, '{not json')
        ).toEqual({
            value: null,
            error: {
                domain: 'boundary',
                code: 'AWARENESS_STATE_INVALID',
                message: expect.stringMatching(/^desired awareness state is not valid JSON:/),
                requestId: null,
                operationIndex: null,
                limit: null,
                actual: null,
                details: null,
            },
        });

        const after = runtime.session(handle.editorId);
        expect(after.desiredAwareness).toEqual(desiredBefore);
        expect(after.localClock).toBe(clockBefore);
        expect(after.localAwarenessLive).toBe(liveBefore);
        expect(runtime.queuedFrames(handle.editorId).map((frame) => Array.from(frame))).toEqual(
            queuedBefore
        );
        expect(after.awarenessNowMillis).toBe(nowBefore);
        expect(after.lastLocalAwarenessPublishMillis).toBe(publishBefore);
    });

    it('fake validates the production text-only intent and publishes nested engine-owned cursor state', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const intent: NativeEditorLocalAwarenessIntent = {
            state: { user: ALICE, selection: { anchor: 90, head: 91 } },
            focused: true,
            selection: createNativeEditorLocalAwarenessSelection(2, 5),
        };
        handle.bridge.collaborationSetAwareness(intent);
        const before = fakeAwarenessAudit(handle.editorId);

        const raw = runtime.module.editorV2CollaborationSetAwareness(
            handle.editorId,
            JSON.stringify({
                state: { user: ALICE, nested: { cursor: { forged: true } } },
                focused: false,
            })
        );
        expect(raw).toMatchObject({
            value: null,
            error: {
                domain: 'boundary',
                code: 'AWARENESS_STATE_INVALID',
                message: expect.stringContaining('reserved cursor key'),
            },
        });
        expect(fakeAwarenessAudit(handle.editorId)).toEqual(before);
        expect(handle.bridge.collaborationPeers()).toEqual([
            expect.objectContaining({
                state: {
                    state: { user: ALICE, selection: { anchor: 90, head: 91 } },
                    focused: true,
                    cursor: expect.any(Object),
                },
                cursor: { anchor: 2, head: 5 },
            }),
        ]);

        const taggedWireSelection = { type: 'text', anchor: 2, head: 5 };
        expect(
            runtime.module.editorV2CollaborationSetAwareness(
                handle.editorId,
                JSON.stringify({ state: { user: ALICE }, focused: true, selection: taggedWireSelection })
            )
        ).toEqual({ value: true, error: null });
        expect(runtime.session(handle.editorId).desiredAwareness).toEqual({
            state: { user: ALICE },
            focused: true,
            selection: taggedWireSelection,
        });

        for (const selection of [
            null,
            { anchor: 2, head: 5 },
            { type: 'node', pos: 2 },
            { type: 'all' },
            { type: 'text', anchor: 2, head: 5, pos: 2 },
        ]) {
            const invalidBefore = fakeAwarenessAudit(handle.editorId);
            const invalid = runtime.module.editorV2CollaborationSetAwareness(
                handle.editorId,
                JSON.stringify({ state: { user: ALICE }, focused: true, selection })
            );
            expect(invalid).toMatchObject({
                value: null,
                error: { domain: 'boundary', code: 'AWARENESS_STATE_INVALID' },
            });
            expect(fakeAwarenessAudit(handle.editorId)).toEqual(invalidBefore);
        }
    });

    it('fake resolves one engine-owned cursor across local and remote document edits', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        handle.bridge.collaborationSetAwareness({
            state: { user: ALICE },
            focused: true,
            selection: createNativeEditorLocalAwarenessSelection(9, 9),
        });
        expect(handle.bridge.collaborationPeers()[0].cursor).toEqual({ anchor: 9, head: 9 });

        handle.bridge.applyInput({
            baseDocumentRevision: '7',
            text: '++',
        });
        expect(handle.bridge.collaborationPeers()[0].cursor).toEqual({ anchor: 11, head: 11 });

        const generation = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(generation);
        handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);
        runtime.pushRemoteDoc(handle.editorId, fakeDocForText('XXsnapshot++'));
        handle.bridge.collaborationReceive(generation, V2_FAKE_UPDATE_FRAME);
        expect(handle.bridge.collaborationPeers()[0].cursor).toEqual({ anchor: 13, head: 13 });
    });

    it('reserves local clock headroom and rejects set-awareness atomically in every transport state', () => {
        for (const transportState of ['Disconnected', 'Handshaking', 'Synchronized'] as const) {
            const handle = createRoomHandle({
                documentId: `set-clock-${transportState}`,
                withSnapshot: true,
            });
            handle.bridge.collaborationSetAwareness(localAwarenessIntent());
            if (transportState !== 'Disconnected') {
                const generation = handle.bridge.collaborationBeginConnect();
                handle.bridge.collaborationSocketOpen(generation);
                if (transportState === 'Synchronized') {
                    handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);
                }
            }

            expect(() =>
                runtime.seedLocalAwarenessClock(handle.editorId, 4_294_967_295.5)
            ).toThrow('clock must be an exact u32');
            expect(() =>
                runtime.seedLocalAwarenessClock(handle.editorId, 4_294_967_296)
            ).toThrow('clock must be an exact u32');
            runtime.seedLocalAwarenessClock(handle.editorId, 4_294_967_293);
            handle.bridge.collaborationSetAwareness(
                localAwarenessIntent({ state: 'last publishable' })
            );
            expect(runtime.session(handle.editorId).localClock).toBe(4_294_967_294);

            const before = fakeAwarenessAudit(handle.editorId);
            expect(
                runtime.module.editorV2CollaborationSetAwareness(
                    handle.editorId,
                    JSON.stringify(localAwarenessIntent({ state: 'must reject' }))
                )
            ).toEqual({ value: null, error: AWARENESS_CLOCK_EXHAUSTED_ERROR });
            expect(fakeAwarenessAudit(handle.editorId)).toEqual(before);
        }
    });

    it('uses the final local clock for clear, then rejects an exhausted tombstone atomically', () => {
        const finalClockHandle = createRoomHandle({ withSnapshot: true });
        finalClockHandle.bridge.collaborationSetAwareness(localAwarenessIntent());
        runtime.seedLocalAwarenessClock(finalClockHandle.editorId, 4_294_967_294);
        finalClockHandle.bridge.collaborationSetAwareness(null);
        expect(runtime.session(finalClockHandle.editorId)).toMatchObject({
            desiredAwareness: null,
            localClock: 4_294_967_295,
            localAwarenessLive: false,
        });

        const exhaustedHandle = createRoomHandle({ withSnapshot: true });
        exhaustedHandle.bridge.collaborationSetAwareness(localAwarenessIntent());
        runtime.seedLocalAwarenessClock(exhaustedHandle.editorId, 4_294_967_295);
        const before = fakeAwarenessAudit(exhaustedHandle.editorId);
        expect(
            runtime.module.editorV2CollaborationSetAwareness(exhaustedHandle.editorId, 'null')
        ).toEqual({ value: null, error: AWARENESS_CLOCK_EXHAUSTED_ERROR });
        expect(fakeAwarenessAudit(exhaustedHandle.editorId)).toEqual(before);
    });

    it.each([
        'TRANSPORT_REPLY_LIMIT_EXCEEDED',
        'TRANSPORT_RESOURCE_EXHAUSTED',
    ] as const)(
        'closes handshake awareness reservation failure %s retryably without committing Step 2',
        (code) => {
            const handle = createRoomHandle({
                documentId: `handshake-reservation-${code}`,
            });
            handle.bridge.collaborationSetAwareness(localAwarenessIntent());
            const generation = handle.bridge.collaborationBeginConnect();
            handle.bridge.collaborationSocketOpen(generation);
            runtime.pushRemotePeers(handle.editorId, [
                remotePeer({ clientId: '42', clock: 1 }),
            ]);
            handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME);
            expect(handle.bridge.collaborationPeers()).toHaveLength(2);

            runtime.pushRemoteDoc(handle.editorId, SERVER_DOC);
            const queuedBeforeFailure = new Uint8Array([0x70, 0x44]);
            runtime.session(handle.editorId).protocolQueue.push(queuedBeforeFailure);
            runtime.injectNextAwarenessBroadcastFailure(handle.editorId, code);

            const outcome = handle.bridge.collaborationReceive(
                generation,
                V2_FAKE_STEP2_FRAME
            );

            expect(outcome).toMatchObject({
                repliesEnqueued: 0,
                replyBytesEnqueued: 0,
                remoteCommitApplied: false,
                documentPromoted: false,
                transportState: 'Disconnected',
                close: {
                    disposition: 'retryable',
                    error: {
                        domain: 'transport',
                        code,
                    },
                },
            });
            expect(outcome.close?.error).toMatchObject(
                code === 'TRANSPORT_REPLY_LIMIT_EXCEEDED'
                    ? {
                          message:
                              'maxPendingOutboxMessages exceeded while receiving a protocol message',
                          limit: '1',
                          actual: '2',
                          details: {
                              action: 'receiveMessage',
                              field: 'maxPendingOutboxMessages',
                              limit: 1,
                              actual: 2,
                          },
                      }
                    : {
                          message: 'protocol reply capacity could not be reserved',
                          details: {
                              action: 'receiveMessage',
                              reason: 'replyReservation',
                          },
                      }
            );
            expect(handle.bridge.getState()).toMatchObject({
                documentState: 'AwaitRemote',
                transportState: 'Disconnected',
            });
            expect(runtime.session(handle.editorId)).toMatchObject({
                desiredAwareness: localAwarenessIntent(),
                localClock: 3,
                localAwarenessLive: false,
                lastLocalAwarenessPublishMillis: null,
                liveGeneration: null,
            });
            expect(handle.bridge.collaborationPeers()).toEqual([]);
            expect(runtime.queuedFrames(handle.editorId)).toEqual([queuedBeforeFailure]);
            expect(() =>
                handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME)
            ).toThrow(expect.objectContaining({ code: 'TRANSPORT_STALE_GENERATION' }));
        }
    );

    it('closes incompatible when handshake awareness republish exhausts its clock', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        handle.bridge.collaborationSetAwareness(localAwarenessIntent());
        runtime.seedLocalAwarenessClock(handle.editorId, 4_294_967_294);
        const generation = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(generation);
        runtime.pushRemotePeers(handle.editorId, [remotePeer({ clientId: '42', clock: 1 })]);
        handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME);
        expect(handle.bridge.collaborationPeers()).toHaveLength(2);

        expect(handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME)).toEqual({
            framesDecoded: 1,
            repliesEnqueued: 0,
            replyBytesEnqueued: 0,
            remoteCommitApplied: false,
            documentPromoted: false,
            transportState: 'Incompatible',
            close: {
                disposition: 'incompatible',
                error: {
                    domain: 'transport',
                    code: 'AWARENESS_CLOCK_EXHAUSTED',
                    message: 'awareness frame handling failed',
                    requestId: null,
                    operationIndex: null,
                    limit: null,
                    actual: null,
                    details: {
                        action: 'receiveMessage',
                        cause: {
                            code: 'AWARENESS_CLOCK_EXHAUSTED',
                            message:
                                'local awareness clock exhausted; a fresh editor identity is required',
                            limit: null,
                            actual: null,
                            details: {
                                requiresFreshEditorIdentity: true,
                                retryable: false,
                            },
                        },
                    },
                },
            },
        });
        expect(runtime.session(handle.editorId)).toMatchObject({
            transportState: 'Incompatible',
            liveGeneration: null,
            desiredAwareness: localAwarenessIntent(),
            localClock: 4_294_967_295,
            localAwarenessLive: false,
            lastLocalAwarenessPublishMillis: null,
            remotePeers: [],
        });
        expect(runtime.session(handle.editorId).remotePeerActivity.size).toBe(0);
        expect(runtime.queuedFrames(handle.editorId)).toEqual([]);
    });

    it('rejects due renewal at exhausted headroom without publishing or changing its deadline', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        handle.bridge.collaborationSetAwareness(localAwarenessIntent());
        const generation = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(generation);
        handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);
        runtime.seedLocalAwarenessClock(handle.editorId, 4_294_967_294);
        const before = fakeAwarenessAudit(handle.editorId);

        expect(runtime.module.editorV2CollaborationTick(handle.editorId, '15000')).toEqual({
            value: null,
            error: AWARENESS_CLOCK_EXHAUSTED_ERROR,
        });
        expect(fakeAwarenessAudit(handle.editorId)).toEqual({
            ...before,
            awarenessNowMillis: 15_000n,
        });
    });

    it('sorts the combined local and remote awareness projection by numeric client id', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        handle.bridge.collaborationSetAwareness(localAwarenessIntent());
        const generation = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(generation);
        handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);
        const localClientId = runtime.session(handle.editorId).localClientId;

        runtime.pushRemotePeers(handle.editorId, [
            remotePeer({ clientId: '2000', clock: 1 }),
            remotePeer({ clientId: '2', clock: 1 }),
        ]);
        handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME);

        expect(handle.bridge.collaborationPeers().map((peer) => peer.clientId)).toEqual([
            '2',
            localClientId,
            '2000',
        ]);
    });

    it('merges clocked awareness deltas and only refreshes activity for admitted updates', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const generation = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(generation);
        handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);
        handle.bridge.collaborationTick('10000');

        const peer42Clock1 = remotePeer({ clientId: '42', clock: 1 });
        const peer43Clock5 = remotePeer({
            clientId: '43',
            clock: 5,
            cursor: null,
            state: { user: { userId: '3', name: 'Carol', color: '#0f0' } },
        });
        runtime.pushRemotePeers(handle.editorId, [peer43Clock5, peer42Clock1]);
        handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME);
        expect(handle.bridge.collaborationPeers()).toEqual([peer42Clock1, peer43Clock5]);

        handle.bridge.collaborationTick('20000');
        const peer42Clock2 = remotePeer({
            clientId: '42',
            clock: 2,
            state: { user: { userId: '2', name: 'Bob II', color: '#00f' } },
        });
        runtime.pushRemotePeers(handle.editorId, [peer42Clock2]);
        handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME);
        expect(handle.bridge.collaborationPeers()).toEqual([peer42Clock2, peer43Clock5]);

        handle.bridge.collaborationTick('25000');
        runtime.pushRemotePeers(handle.editorId, [
            remotePeer({
                clientId: '43',
                clock: 5,
                cursor: null,
                state: { user: { userId: '3', name: 'equal ignored', color: '#0f0' } },
            }),
        ]);
        handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME);
        runtime.pushRemotePeers(handle.editorId, [
            remotePeer({
                clientId: '43',
                clock: 4,
                cursor: null,
                state: { user: { userId: '3', name: 'stale ignored', color: '#0f0' } },
            }),
        ]);
        handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME);
        expect(handle.bridge.collaborationPeers()).toEqual([peer42Clock2, peer43Clock5]);

        handle.bridge.collaborationTick('30000');
        const peer42Clock3 = remotePeer({
            clientId: '42',
            clock: 3,
            state: { user: { userId: '2', name: 'Bob III', color: '#00f' } },
        });
        runtime.pushRemotePeers(handle.editorId, [peer42Clock3]);
        handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME);
        expect(handle.bridge.collaborationTick('39999')).toEqual({
            nextDeadlineMillis: '40000',
            renewedLocal: false,
            expiredPeers: [],
            outboundChanged: false,
            peersChanged: false,
        });
        expect(handle.bridge.collaborationTick('40000')).toEqual({
            nextDeadlineMillis: '60000',
            renewedLocal: false,
            expiredPeers: ['43'],
            outboundChanged: false,
            peersChanged: true,
        });
        expect(handle.bridge.collaborationPeers()).toEqual([peer42Clock3]);
        expect(handle.bridge.collaborationTick('59999')).toEqual({
            nextDeadlineMillis: '60000',
            renewedLocal: false,
            expiredPeers: [],
            outboundChanged: false,
            peersChanged: false,
        });

        runtime.pushRemotePeers(handle.editorId, [
            remotePeer({ clientId: '42', clock: 4, state: null, cursor: null }),
        ]);
        handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME);
        expect(handle.bridge.collaborationPeers()).toEqual([]);
        expect(handle.bridge.collaborationTick('60000')).toEqual({
            nextDeadlineMillis: null,
            renewedLocal: false,
            expiredPeers: [],
            outboundChanged: false,
            peersChanged: false,
        });
    });

    it('admits max-minus-one remote clocks and rejects a mixed terminal-clock delta atomically', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const generation = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(generation);
        handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);
        handle.bridge.collaborationTick('10000');

        const edgePeer = remotePeer({ clientId: '41', clock: 4_294_967_294 });
        runtime.pushRemotePeers(handle.editorId, [edgePeer]);
        expect(
            handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME).close
        ).toBeNull();
        expect(handle.bridge.collaborationPeers()).toEqual([edgePeer]);

        runtime.pushRemotePeers(handle.editorId, [
            remotePeer({ clientId: '42', clock: 1 }),
            remotePeer({ clientId: '43', clock: 4_294_967_295 }),
        ]);
        expect(handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME)).toEqual({
            framesDecoded: 1,
            repliesEnqueued: 0,
            replyBytesEnqueued: 0,
            remoteCommitApplied: false,
            documentPromoted: false,
            transportState: 'Incompatible',
            close: {
                disposition: 'incompatible',
                error: {
                    domain: 'transport',
                    code: 'TRANSPORT_AWARENESS_LIMIT_EXCEEDED',
                    message: 'awareness frame handling failed',
                    requestId: null,
                    operationIndex: null,
                    limit: null,
                    actual: null,
                    details: {
                        action: 'receiveMessage',
                        cause: {
                            code: 'INPUT_LIMIT_EXCEEDED',
                            message: 'input exceeds limit 4294967294: 4294967295',
                            limit: 4_294_967_294,
                            actual: 4_294_967_295,
                            details: { field: 'awarenessClock' },
                        },
                    },
                },
            },
        });
        const session = runtime.session(handle.editorId);
        expect(session.transportState).toBe('Incompatible');
        expect(session.liveGeneration).toBeNull();
        expect(session.remotePeers).toEqual([]);
        expect(session.remoteAwarenessClocks.size).toBe(0);
        expect(session.remotePeerActivity.size).toBe(0);
        expect(handle.bridge.collaborationPeers()).toEqual([]);
    });

    it('closes retryably and atomically for malformed fake-wire client ids and clocks', () => {
        const malformedEntries: Array<{
            label: string;
            peer: NativeEditorV2PeerInfo;
        }> = [
            {
                label: 'noncanonical client id',
                peer: remotePeer({ clientId: '01', clock: 1 }),
            },
            {
                label: 'negative clock',
                peer: remotePeer({ clientId: '51', clock: -1 }),
            },
            {
                label: 'fractional clock',
                peer: remotePeer({ clientId: '51', clock: 1.5 }),
            },
            {
                label: 'clock above u32',
                peer: remotePeer({ clientId: '51', clock: 4_294_967_296 }),
            },
        ];

        for (const { label, peer } of malformedEntries) {
            const handle = createRoomHandle({ documentId: `malformed-${label}`, withSnapshot: true });
            const generation = handle.bridge.collaborationBeginConnect();
            handle.bridge.collaborationSocketOpen(generation);
            handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);
            runtime.pushRemotePeers(handle.editorId, [
                remotePeer({ clientId: '40', clock: 1 }),
                peer,
            ]);

            expect(
                handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME)
            ).toEqual({
                framesDecoded: 1,
                repliesEnqueued: 0,
                replyBytesEnqueued: 0,
                remoteCommitApplied: false,
                documentPromoted: false,
                transportState: 'Disconnected',
                close: {
                    disposition: 'retryable',
                    error: {
                        domain: 'transport',
                        code: 'TRANSPORT_PROTOCOL_INVALID',
                        message: 'awareness frame handling failed',
                        requestId: null,
                        operationIndex: null,
                        limit: null,
                        actual: null,
                        details: {
                            action: 'receiveMessage',
                            cause: {
                                code: 'COLLABORATION_DECODE_FAILED',
                                message: MALFORMED_FAKE_AWARENESS_MESSAGE,
                                limit: null,
                                actual: null,
                                details: null,
                            },
                        },
                    },
                },
            });
            expect(runtime.session(handle.editorId).remotePeers).toEqual([]);
            expect(runtime.session(handle.editorId).remoteAwarenessClocks.size).toBe(0);
            expect(runtime.session(handle.editorId).remotePeerActivity.size).toBe(0);
        }
    });

    it('sorts validated awareness entries numerically before choosing an admission error', () => {
        for (const reverse of [false, true]) {
            const handle = createRoomHandle({
                documentId: `sorted-admission-${reverse}`,
                withSnapshot: true,
            });
            handle.bridge.collaborationSetAwareness(localAwarenessIntent());
            const generation = handle.bridge.collaborationBeginConnect();
            handle.bridge.collaborationSocketOpen(generation);
            handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);
            const session = runtime.session(handle.editorId);
            expect(session.localClock).toBe(2);

            const localViolation = remotePeer({
                clientId: session.localClientId,
                clock: 3,
                isLocal: false,
                state: null,
                cursor: null,
            });
            const lowerClientTerminalViolation = remotePeer({
                clientId: '2',
                clock: 4_294_967_295,
            });
            runtime.pushRemotePeers(
                handle.editorId,
                reverse
                    ? [lowerClientTerminalViolation, localViolation]
                    : [localViolation, lowerClientTerminalViolation]
            );

            expect(
                handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME)
            ).toMatchObject({
                transportState: 'Incompatible',
                close: {
                    disposition: 'incompatible',
                    error: {
                        code: 'TRANSPORT_AWARENESS_LIMIT_EXCEEDED',
                        details: {
                            action: 'receiveMessage',
                            cause: {
                                code: 'INPUT_LIMIT_EXCEEDED',
                                message: 'input exceeds limit 4294967294: 4294967295',
                                limit: 4_294_967_294,
                                actual: 4_294_967_295,
                                details: { field: 'awarenessClock' },
                            },
                        },
                    },
                },
            });
            expect(runtime.session(handle.editorId).remotePeers).toEqual([]);
            expect(runtime.session(handle.editorId).remoteAwarenessClocks.size).toBe(0);
            expect(runtime.session(handle.editorId).remotePeerActivity.size).toBe(0);
        }
    });

    it('ignores same and older local-client echoes without refreshing remote activity', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        handle.bridge.collaborationSetAwareness(localAwarenessIntent());
        const generation = handle.bridge.collaborationBeginConnect();
        handle.bridge.collaborationSocketOpen(generation);
        handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);
        const session = runtime.session(handle.editorId);
        const localClientId = session.localClientId;
        const localBefore = handle.bridge
            .collaborationPeers()
            .find((peer) => peer.clientId === localClientId);

        for (const clock of [session.localClock, session.localClock - 1]) {
            runtime.pushRemotePeers(handle.editorId, [
                remotePeer({
                    clientId: localClientId,
                    clock,
                    isLocal: false,
                    state: clock === session.localClock ? null : { remote: 'must be ignored' },
                    cursor: null,
                }),
            ]);
            expect(
                handle.bridge.collaborationReceive(generation, V2_FAKE_AWARENESS_FRAME).close
            ).toBeNull();
        }

        expect(
            handle.bridge.collaborationPeers().find((peer) => peer.clientId === localClientId)
        ).toEqual(localBefore);
        expect(session.remoteAwarenessClocks.has(localClientId)).toBe(false);
        expect(session.remotePeerActivity.has(localClientId)).toBe(false);
    });

    it('rejects greater and terminal local-client clocks before applying a mixed delta', () => {
        for (const incomingClock of [3, 4_294_967_295]) {
            const handle = createRoomHandle({
                documentId: `local-echo-${incomingClock}`,
                withSnapshot: true,
            });
            handle.bridge.collaborationSetAwareness(localAwarenessIntent());
            const generation = handle.bridge.collaborationBeginConnect();
            handle.bridge.collaborationSocketOpen(generation);
            handle.bridge.collaborationReceive(generation, V2_FAKE_STEP2_FRAME);
            const session = runtime.session(handle.editorId);
            expect(session.localClock).toBe(2);

            runtime.pushRemotePeers(handle.editorId, [
                remotePeer({ clientId: '42', clock: 1 }),
                remotePeer({
                    clientId: session.localClientId,
                    clock: incomingClock,
                    isLocal: false,
                    state: null,
                    cursor: null,
                }),
            ]);
            const outcome = handle.bridge.collaborationReceive(
                generation,
                V2_FAKE_AWARENESS_FRAME
            );
            expect(outcome).toMatchObject({
                transportState: 'Incompatible',
                close: {
                    disposition: 'incompatible',
                    error: {
                        domain: 'transport',
                        code: 'TRANSPORT_AWARENESS_LIMIT_EXCEEDED',
                        message: 'awareness frame handling failed',
                        details: {
                            action: 'receiveMessage',
                            cause: {
                                code: 'INPUT_LIMIT_EXCEEDED',
                                message: `input exceeds limit 2: ${incomingClock}`,
                                limit: 2,
                                actual: incomingClock,
                                details: { field: 'awarenessClock' },
                            },
                        },
                    },
                },
            });
            expect(runtime.session(handle.editorId)).toMatchObject({
                transportState: 'Incompatible',
                liveGeneration: null,
                desiredAwareness: localAwarenessIntent(),
                localClock: 3,
                localAwarenessLive: false,
                remotePeers: [],
            });
            expect(runtime.session(handle.editorId).remoteAwarenessClocks.size).toBe(0);
            expect(runtime.session(handle.editorId).remotePeerActivity.size).toBe(0);
        }
    });

    // ── Awareness / peers ───────────────────────────────────────

    it('refreshes sticky local peers after every local commit offline and drains document frames only while live', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        setup.controller.handleSelectionChange({ type: 'text', anchor: 9, head: 9 });
        setup.handle.bridge.applyInput({ baseDocumentRevision: '7', text: '++' });

        const peerReadsBeforeOfflineCommit =
            runtime.module.editorV2CollaborationPeers.mock.calls.length;
        const outboundDrainsBeforeOfflineCommit =
            runtime.module.editorV2CollaborationTakeOutbound.mock.calls.length;
        setup.controller.handleLocalCommit();
        expect(runtime.module.editorV2CollaborationPeers).toHaveBeenCalledTimes(
            peerReadsBeforeOfflineCommit + 1
        );
        expect(runtime.module.editorV2CollaborationTakeOutbound).toHaveBeenCalledTimes(
            outboundDrainsBeforeOfflineCommit
        );
        expect(setup.controller.state.documentRevision).toBe('8');

        setup.controller.connect();
        openAndSynchronize(setup);
        expect(setup.controller.peers[0]).toEqual(
            expect.objectContaining({ isLocal: true, cursor: { anchor: 11, head: 11 } })
        );

        setup.handle.bridge.applyInput({ baseDocumentRevision: '8', text: '!' });
        const outboundDrainsBeforeLiveCommit =
            runtime.module.editorV2CollaborationTakeOutbound.mock.calls.length;
        setup.controller.handleLocalCommit();
        expect(setup.controller.peers[0]).toEqual(
            expect.objectContaining({ isLocal: true, cursor: { anchor: 12, head: 12 } })
        );
        expect(runtime.module.editorV2CollaborationTakeOutbound.mock.calls.length).toBeGreaterThan(
            outboundDrainsBeforeLiveCommit
        );
        expect(runtime.session(setup.handle.editorId).documentRevision).toBe(9);
    });

    it('refreshes resolved remote peer cursors after a remote document commit without writing the document', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        setup.controller.connect();
        openAndSynchronize(setup);

        runtime.pushRemotePeers(setup.handle.editorId, [
            remotePeer({ clientId: '2', cursor: { anchor: 9, head: 9 } }),
        ]);
        setup.sockets[0].receive(V2_FAKE_AWARENESS_FRAME);
        expect(setup.controller.peers[0]).toEqual(
            expect.objectContaining({ clientId: '2', cursor: { anchor: 9, head: 9 } })
        );

        const localInputWritesBeforeRemoteCommit =
            runtime.module.editorV2ApplyInput.mock.calls.length;
        runtime.pushRemoteDoc(setup.handle.editorId, fakeDocForText('XXsnapshot'));
        setup.sockets[0].receive(V2_FAKE_UPDATE_FRAME);
        expect(setup.controller.peers[0]).toEqual(
            expect.objectContaining({ clientId: '2', cursor: { anchor: 11, head: 11 } })
        );
        expect(runtime.module.editorV2ApplyInput).toHaveBeenCalledTimes(
            localInputWritesBeforeRemoteCommit
        );
        expect(runtime.session(setup.handle.editorId).documentRevision).toBe(8);
    });

    it('refreshes peers after a snapshot-restore commit and reattach without extra document writes', () => {
        const clock = new FakeMonotonicClockTimer();
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        setup.controller.handleSelectionChange({ type: 'text', anchor: 9, head: 9 });
        setup.controller.connect();
        openAndSynchronize(setup);
        setup.controller.disconnect();

        setup.handle.bridge.snapshotRestore(
            {
                formatVersion: 1,
                documentId: 'doc-1',
                lineageId: 'lineage-1',
                fragmentName: 'prosemirror',
                schemaFingerprint: 'fakefingerprint',
            },
            snapshotState(fakeDocForText('go'), 20)
        );
        const peerReadsBeforeSnapshotCommit =
            runtime.module.editorV2CollaborationPeers.mock.calls.length;
        const localInputWritesBeforeSnapshotCommit =
            runtime.module.editorV2ApplyInput.mock.calls.length;
        setup.controller.handleLocalCommit();
        expect(runtime.module.editorV2CollaborationPeers).toHaveBeenCalledTimes(
            peerReadsBeforeSnapshotCommit + 1
        );
        expect(runtime.module.editorV2ApplyInput).toHaveBeenCalledTimes(
            localInputWritesBeforeSnapshotCommit
        );
        expect(runtime.session(setup.handle.editorId).documentRevision).toBe(20);

        setup.controller.reconnect();
        const reattachOrder = runtime.module.editorV2CollaborationReattach.mock.invocationCallOrder.at(-1);
        const beginConnectOrder =
            runtime.module.editorV2CollaborationBeginConnect.mock.invocationCallOrder.at(-1);
        expect(reattachOrder).toBeDefined();
        expect(beginConnectOrder).toBeDefined();
        expect(
            runtime.module.editorV2CollaborationPeers.mock.invocationCallOrder.some(
                (order) => order > (reattachOrder as number) && order < (beginConnectOrder as number)
            )
        ).toBe(true);

        openAndSynchronize(setup, 1);
        expect(setup.controller.peers[0]).toEqual(
            expect.objectContaining({ isLocal: true, cursor: { anchor: 3, head: 3 } })
        );
    });

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
        expect(JSON.parse(initialPayload)).toEqual({
            state: { user: ALICE },
            focused: false,
        });

        setup.controller.connect();
        setup.sockets[0].open();
        setup.sockets[0].receive(V2_FAKE_STEP2_FRAME);
        // The disconnected set owns clock 1; synchronization re-publishes
        // the retained desired state with clock 2.
        expect(sentFrames(setup.sockets[0])).toEqual([
            Array.from(V2_FAKE_STEP1_FRAME),
            [0x61, 2],
        ]);

        // The local peer projection comes from Rust (with its clock).
        expect(setup.controller.peers).toEqual([
            {
                clientId: expect.any(String),
                clock: 2,
                isLocal: true,
                state: { state: { user: ALICE }, focused: false },
                cursor: null,
            },
        ]);

        // A remote awareness update renders through the returned Rust state.
        runtime.pushRemotePeers(setup.handle.editorId, [remotePeer()]);
        setup.sockets[0].receive(V2_FAKE_AWARENESS_FRAME);
        expect(setup.controller.peers).toHaveLength(2);
        expect(setup.controller.peers[0]).toEqual(remotePeer());
        expect(setup.peersLog.length).toBeGreaterThanOrEqual(1);

        // Updating local awareness mutates clock 3 and publishes that entry.
        setup.controller.updateLocalAwareness({ user: { ...ALICE, name: 'Alice II' } });
        const updatePayload = runtime.module.editorV2CollaborationSetAwareness.mock
            .calls[1][1] as string;
        expect(updatePayload).not.toContain('"clock"');
        expect(sentFrames(setup.sockets[0]).slice(2)).toEqual([[0x61, 3]]);
    });

    it('composes application state, focus, and document selection into the local intent', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.handleSelectionChange({ type: 'text', anchor: 2, head: 5 });
        const selectionPayload = JSON.parse(
            runtime.module.editorV2CollaborationSetAwareness.mock.calls[1][1] as string
        );
        expect(selectionPayload).toEqual({
            state: { user: ALICE },
            focused: true,
            selection: { type: 'text', anchor: 2, head: 5 },
        });

        setup.controller.handleFocusChange(false);
        const focusPayload = JSON.parse(
            runtime.module.editorV2CollaborationSetAwareness.mock.calls[2][1] as string
        );
        expect(focusPayload).toEqual({
            state: { user: ALICE },
            focused: false,
            selection: { type: 'text', anchor: 2, head: 5 },
        });
    });

    it('commits focus, user, and selection candidates only after native acceptance', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        const nativeSet = runtime.module.editorV2CollaborationSetAwareness;

        nativeSet.mockImplementationOnce(() => rejectedAwarenessResult('focus rejected'));
        setup.controller.handleFocusChange(true);
        setup.controller.updateLocalAwareness({ user: { ...ALICE, name: 'Alice II' } });
        expect(JSON.parse(nativeSet.mock.calls.at(-1)?.[1] as string)).toEqual({
            state: { user: { ...ALICE, name: 'Alice II' } },
            focused: false,
        });

        nativeSet.mockImplementationOnce(() => rejectedAwarenessResult('user rejected'));
        setup.controller.updateLocalAwareness({ user: { ...ALICE, name: 'Rejected' } });
        setup.controller.handleSelectionChange({ type: 'text', anchor: 2, head: 5 });
        expect(JSON.parse(nativeSet.mock.calls.at(-1)?.[1] as string)).toEqual({
            state: { user: { ...ALICE, name: 'Alice II' } },
            focused: true,
            selection: { type: 'text', anchor: 2, head: 5 },
        });

        setup.controller.handleSelectionChange({ type: 'text', anchor: 999, head: 999 });
        setup.controller.handleFocusChange(false);
        expect(JSON.parse(nativeSet.mock.calls.at(-1)?.[1] as string)).toEqual({
            state: { user: { ...ALICE, name: 'Alice II' } },
            focused: false,
            selection: { type: 'text', anchor: 2, head: 5 },
        });
        expect(setup.errors.map((error) => error.message)).toEqual([
            'focus rejected',
            'user rejected',
            expect.stringContaining('outside'),
        ]);
    });

    it('normalizes node and all selections to a cursorless awareness intent', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.handleSelectionChange({ type: 'text', anchor: 2, head: 5 });

        setup.controller.handleSelectionChange({ type: 'node', pos: 3 });
        expect(
            JSON.parse(
                runtime.module.editorV2CollaborationSetAwareness.mock.calls.at(-1)?.[1] as string
            )
        ).toEqual({ state: { user: ALICE }, focused: true });

        setup.controller.handleSelectionChange({ type: 'text', anchor: 3, head: 4 });
        setup.controller.handleSelectionChange({ type: 'all' });
        expect(
            JSON.parse(
                runtime.module.editorV2CollaborationSetAwareness.mock.calls.at(-1)?.[1] as string
            )
        ).toEqual({ state: { user: ALICE }, focused: true });
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
        expect(payload).toEqual({ state: { user: ALICE }, focused: true });
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

    it('clears retained native awareness when a fresh controller omits localAwareness', () => {
        const clock = new FakeMonotonicClockTimer();
        const handle = createRoomHandle({ withSnapshot: true });
        const first = setupController({
            handle,
            localAwareness: ALICE,
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        first.controller.connect();
        openAndSynchronize(first);
        first.controller.destroy();
        expect(runtime.session(handle.editorId)).toMatchObject({
            desiredAwareness: localAwarenessIntent(),
            localAwarenessLive: false,
        });
        const clockAfterDestroy = runtime.session(handle.editorId).localClock;
        runtime.module.editorV2CollaborationSetAwareness.mockClear();

        const second = setupController({
            handle,
            monotonicClock: clock,
            awarenessTimer: clock,
        });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(1);
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenLastCalledWith(
            handle.editorId,
            'null'
        );
        expect(runtime.session(handle.editorId)).toMatchObject({
            desiredAwareness: null,
            localAwarenessLive: false,
            localClock: clockAfterDestroy,
        });

        handle.bridge.collaborationSetAwareness(null);
        expect(runtime.session(handle.editorId).localClock).toBe(clockAfterDestroy);
        expect(runtime.queuedFrames(handle.editorId)).toEqual([]);
        expect(pendingAwarenessTombstone(handle.editorId)).toEqual([
            0x61,
            clockAfterDestroy & 0xff,
        ]);

        second.controller.reconnect();
        expect(second.sockets).toHaveLength(1);
        second.sockets[0].open();
        second.sockets[0].receive(V2_FAKE_STEP2_FRAME);
        expect(sentFrames(second.sockets[0])).toEqual([Array.from(V2_FAKE_STEP1_FRAME)]);
        expect(runtime.session(handle.editorId).desiredAwareness).toBeNull();
        expect(clock.activeTimerCount).toBe(1);
        expect(clock.scheduledDelays.at(-1)).toBe(15_000);

        clock.advanceBy(15_000n);
        expect(sentFrames(second.sockets[0])).toEqual([
            Array.from(V2_FAKE_STEP1_FRAME),
            [0x61, clockAfterDestroy & 0xff],
        ]);
        expect(pendingAwarenessTombstone(handle.editorId)).toBeNull();
        expect(clock.activeTimerCount).toBe(0);
        second.controller.destroy();
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

    it('renders only Rust-resolved peer cursors and ignores scalar state selections', () => {
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
                remotePeer({
                    state: {
                        state: {
                            user: { userId: '2', name: 'Bob', color: '#00f' },
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

    it('clears live prop awareness once, keeps hooks inert, and lets an explicit user restore it', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const sockets: MockWebSocket[] = [];
        let localAwareness: typeof ALICE | undefined = ALICE;
        const { result, rerender } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                connect: true,
                localAwareness,
                createWebSocket: () => {
                    const socket = new MockWebSocket();
                    sockets.push(socket);
                    return socket as unknown as WebSocket;
                },
            })
        );
        // The constructor publishes once; the mount-time prop effect merges
        // the identical user and dedups to a no-op.
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(1);
        act(() => {
            sockets[0].open();
            sockets[0].receive(V2_FAKE_STEP2_FRAME);
        });

        localAwareness = { ...ALICE, name: 'Alice II' };
        rerender();
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(2);
        const payload = JSON.parse(
            runtime.module.editorV2CollaborationSetAwareness.mock.calls[1][1] as string
        );
        expect(payload).toEqual({
            state: { user: { ...ALICE, name: 'Alice II' } },
            focused: false,
        });

        const awarenessFramesBeforeClear = sentFrames(sockets[0]).filter(
            ([type]) => type === 0x61
        ).length;
        localAwareness = undefined;
        rerender();
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(3);
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenLastCalledWith(
            handle.editorId,
            'null'
        );
        expect(runtime.session(handle.editorId)).toMatchObject({
            desiredAwareness: null,
            localAwarenessLive: false,
        });
        expect(sentFrames(sockets[0]).filter(([type]) => type === 0x61)).toHaveLength(
            awarenessFramesBeforeClear + 1
        );

        act(() => {
            result.current.editorBindings.onFocus();
            result.current.editorBindings.onSelectionChange({ type: 'text', anchor: 1, head: 3 });
            result.current.editorBindings.onBlur();
        });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(3);

        act(() => {
            result.current.updateLocalAwareness({ user: ALICE });
            result.current.editorBindings.onSelectionChange({ type: 'text', anchor: 1, head: 3 });
            result.current.editorBindings.onBlur();
        });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(6);
        expect(
            JSON.parse(
                runtime.module.editorV2CollaborationSetAwareness.mock.calls.at(-1)?.[1] as string
            )
        ).toEqual({
            state: { user: ALICE },
            focused: false,
            selection: { type: 'text', anchor: 1, head: 3 },
        });

        localAwareness = ALICE;
        rerender();
        localAwareness = undefined;
        rerender();
        act(() => {
            result.current.reconnect();
            sockets[1].open();
            sockets[1].receive(V2_FAKE_STEP2_FRAME);
        });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(7);
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenLastCalledWith(
            handle.editorId,
            'null'
        );
        expect(sentFrames(sockets[1]).filter(([type]) => type === 0x61)).toEqual([]);
        expect(runtime.session(handle.editorId).desiredAwareness).toBeNull();
    });

    it('clears disconnected prop awareness so reconnect cannot republish it', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const sockets: MockWebSocket[] = [];
        let localAwareness: typeof ALICE | undefined = ALICE;
        const { result, rerender } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                connect: false,
                localAwareness,
                createWebSocket: () => {
                    const socket = new MockWebSocket();
                    sockets.push(socket);
                    return socket as unknown as WebSocket;
                },
            })
        );

        localAwareness = undefined;
        rerender();
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(2);
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenLastCalledWith(
            handle.editorId,
            'null'
        );
        expect(runtime.session(handle.editorId)).toMatchObject({
            desiredAwareness: null,
            localAwarenessLive: false,
        });

        act(() => {
            result.current.connect();
            sockets[0].open();
            sockets[0].receive(V2_FAKE_STEP2_FRAME);
        });
        expect(sentFrames(sockets[0]).filter(([type]) => type === 0x61)).toEqual([]);
        expect(runtime.session(handle.editorId).desiredAwareness).toBeNull();
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
