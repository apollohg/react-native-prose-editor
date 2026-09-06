import { createFakeLifecycleModule } from './createFakeLifecycleModule';
import { createFakeEditingModule } from './createFakeEditingModule';
import { createFakeCollaborationModule } from './createFakeCollaborationModule';
import { createFakeRuntimeState } from './createFakeRuntimeState';
import { type FakeNativeEditorV2Runtime } from './nativeEditorV2FakeTypes';
import {
    V2_FAKE_STEP1_FRAME,
    canonicalV2U64,
    exactV2U32,
    errorRecord,
} from './nativeEditorV2FakeRecords';
import { cloneDoc } from './nativeEditorV2FakeDocument';

export function createFakeRuntimeControls(
    context: Pick<ReturnType<typeof createFakeLifecycleModule>, 'module0'> &
        Pick<ReturnType<typeof createFakeEditingModule>, 'module1'> &
        Pick<ReturnType<typeof createFakeCollaborationModule>, 'module2'> &
        Pick<
            ReturnType<typeof createFakeRuntimeState>,
            | 'sessions'
            | 'getSession'
            | 'liveIds'
            | 'requireSession'
            | 'emitTransportState'
            | 'handleReceive'
            | 'retireGeneration'
            | 'emitTransportEvent'
            | 'protocolAdapterResolutions'
            | 'pendingFor'
        >
): FakeNativeEditorV2Runtime {
    const {
        module0,
        module1,
        module2,
        sessions,
        getSession,
        liveIds,
        requireSession,
        emitTransportState,
        handleReceive,
        retireGeneration,
        emitTransportEvent,
        protocolAdapterResolutions,
        pendingFor,
    } = context;

    const module = { ...module0, ...module1, ...module2 };

    return {
        module,
        sessions: () => [...sessions.values()],
        session: (editorId: string) => {
            const session = getSession(editorId);
            if (!session) throw new Error(`unknown fake session ${editorId}`);
            return session;
        },
        liveEditorIds: () => [...liveIds],
        transportConfig: (editorId) => getSession(editorId)?.transportConfig ?? null,
        transportOpen: (editorId) => {
            const session = requireSession(editorId);
            if (session.transportState !== 'Connecting') {
                throw new Error(
                    `socket open requires Connecting (found ${session.transportState})`
                );
            }
            session.transportState = 'Handshaking';
            // The native side answers an opened socket with Sync Step 1.
            session.protocolQueue.push(new Uint8Array(V2_FAKE_STEP1_FRAME));
            emitTransportState(session, 'socketOpen');
        },
        transportReceive: (editorId, frame) => {
            const session = requireSession(editorId);
            handleReceive(session, frame);
            emitTransportState(session, 'receive');
        },
        transportClose: (editorId, code = null) => {
            const session = requireSession(editorId);
            retireGeneration(session, code === 1008 ? 'Incompatible' : 'Disconnected');
            emitTransportState(session, 'socketClose');
        },
        emitTransportError: (editorId, error) => {
            const session = requireSession(editorId);
            emitTransportEvent({
                editorId: session.editorId,
                generation: session.liveGeneration === null ? null : String(session.liveGeneration),
                kind: 'error',
                error,
            });
        },
        emitProtocolAdapterEvent: (editorId, event) => {
            const session = requireSession(editorId);
            emitTransportEvent({
                editorId: session.editorId,
                generation:
                    session.liveGeneration === null
                        ? String(session.lastIssuedGeneration)
                        : String(session.liveGeneration),
                kind: 'protocolAdapter',
                ...event,
            });
        },
        protocolAdapterResolutions: () => [...protocolAdapterResolutions],
        pushRemoteDoc: (editorId, doc) => {
            pendingFor(editorId).docs.push(cloneDoc(doc));
        },
        pushRemotePeers: (editorId, peers) => {
            pendingFor(editorId).awarenessDeltas.push(peers);
        },
        seedLastIssuedGeneration: (editorId, generation) => {
            const canonicalGeneration = canonicalV2U64(generation);
            if (canonicalGeneration == null) {
                throw new Error('generation must be canonical decimal u64 text');
            }
            const session = getSession(editorId);
            if (!session) throw new Error(`unknown fake session ${editorId}`);
            session.lastIssuedGeneration = BigInt(canonicalGeneration);
        },
        seedLocalAwarenessClock: (editorId, clock) => {
            const exactClock = exactV2U32(clock);
            if (exactClock == null) throw new Error('clock must be an exact u32');
            const session = getSession(editorId);
            if (!session) throw new Error(`unknown fake session ${editorId}`);
            session.localClock = exactClock;
        },
        retireLiveGeneration: (editorId) => {
            const session = getSession(editorId);
            if (session) session.liveGeneration = null;
        },
        injectNextApplyLocalApiError: (editorId, error) => {
            pendingFor(editorId).applyLocalApiErrors.push(error);
        },
        injectNextApplyCommandError: (editorId, error) => {
            pendingFor(editorId).applyCommandErrors.push(error);
        },
        injectNextAwarenessBroadcastFailure: (editorId, code) => {
            const error = errorRecord(
                'transport',
                code,
                code === 'TRANSPORT_REPLY_LIMIT_EXCEEDED'
                    ? 'maxPendingOutboxMessages exceeded while enqueueing an awareness broadcast'
                    : 'awareness broadcast capacity could not be reserved'
            );
            if (code === 'TRANSPORT_REPLY_LIMIT_EXCEEDED') {
                error.limit = '1';
                error.actual = '2';
                error.details = {
                    action: 'awareness',
                    field: 'maxPendingOutboxMessages',
                    limit: 1,
                    actual: 2,
                };
            }
            pendingFor(editorId).awarenessBroadcastErrors.push(error);
        },
        queuedFrames: (editorId) => {
            const session = getSession(editorId);
            if (!session) return [];
            return [...session.protocolQueue, ...session.documentQueue];
        },
    };
}
