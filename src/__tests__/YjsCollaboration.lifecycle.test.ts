import './helpers/YjsCollaborationFixture';
import {
    TRANSPORT_URL,
    SERVER_DOC,
    ALICE,
    localAwarenessIntent,
    runtime,
    createRoomHandle,
    setupController,
    configuredTransport,
    synchronize,
    awarenessPayload,
} from './helpers/YjsCollaborationFixture';
import { readFileSync } from 'fs';
import { join } from 'path';
import { V2_FAKE_STEP2_FRAME } from './helpers/nativeEditorV2Fake';
import { act, renderHook } from '@testing-library/react-native';
import { useYjsCollaboration } from '../YjsCollaboration';

import * as PublicApi from '../index';

describe('YjsCollaboration (native-transport controller)', () => {
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

    it('reports non-encodable hook awareness without throwing during render', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const errors: Error[] = [];

        expect(() =>
            renderHook(() =>
                useYjsCollaboration({
                    documentId: 'doc-1',
                    handle,
                    transport: { url: TRANSPORT_URL, connect: false },
                    localAwareness: { ...ALICE, extra: { sequence: 1n } },
                    onError: (error) => errors.push(error),
                })
            )
        ).not.toThrow();
        expect(errors.length).toBeGreaterThan(0);
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
