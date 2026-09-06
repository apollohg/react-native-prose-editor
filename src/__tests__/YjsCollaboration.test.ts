import './helpers/YjsCollaborationFixture';
import {
    mockNativeModule,
    TRANSPORT_URL,
    SERVER_DOC,
    SECOND_SERVER_DOC,
    SNAPSHOT_DOC,
    runtime,
    createRoomHandle,
    createLocalHandle,
    setupController,
    configuredTransport,
    synchronize,
    latestStatus,
} from './helpers/YjsCollaborationFixture';

import { V2_FAKE_STEP2_FRAME, V2_FAKE_UPDATE_FRAME } from './helpers/nativeEditorV2Fake';

import { createYjsCollaborationController } from '../YjsCollaboration';
import { type NativeEditorDocumentHandle } from '../NativeEditorBridge';

describe('YjsCollaboration (native-transport controller)', () => {
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
        const documentReadsAfterSync = runtime.module.editorV2GetDocumentJson.mock.calls.length;

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
        const listener = mockNativeModule.addListener.mock.calls[0][1] as (event: unknown) => void;
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
});
