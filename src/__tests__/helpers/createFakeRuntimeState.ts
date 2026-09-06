import type { DocumentJSON, NativeEditorPeerInfo } from '../../NativeEditorBridge';
import {
    type FakeSession,
    type PendingRemote,
    type FakeProtocolAdapterResolution,
    awarenessFrame,
    documentFrame,
    protocolReplyFrame,
} from './nativeEditorV2FakeTypes';
import {
    V2_FAKE_TRANSPORT_EVENT_NAME,
    lifecycleError,
    canonicalV2U64,
    boundaryError,
    transportError,
    operationError,
    type FakeTransportState,
    type FakeErrorRecord,
    errorRecord,
    V2_FAKE_U32_MAX,
    V2_FAKE_U64_MAX,
    V2_FAKE_AWARENESS_RENEWAL_INTERVAL_MILLIS,
    V2_FAKE_AWARENESS_EXPIRY_MILLIS,
    V2_FAKE_MAX_ADMITTED_REMOTE_AWARENESS_CLOCK,
    exactV2U32,
    V2_FAKE_MALFORMED_AWARENESS_MESSAGE,
    okRecord,
    EMPTY_DOC,
} from './nativeEditorV2FakeRecords';
import { projectFakeLocalAwareness, installFakeDocument } from './nativeEditorV2FakeAwareness';
import { cloneDoc } from './nativeEditorV2FakeDocument';

export function createFakeRuntimeState() {
    const sessions = new Map<string, FakeSession>();

    const pending = new Map<string, PendingRemote>();

    const liveIds = new Set<string>();

    const transportListeners = new Map<string, ((event: unknown) => void)[]>();

    const protocolAdapterResolutions: FakeProtocolAdapterResolution[] = [];

    const counters = { editorId: 0, clientId: 1000, transportEventSequence: 0n };

    function listenersFor(eventName: string): ((event: unknown) => void)[] {
        let entry = transportListeners.get(eventName);
        if (!entry) {
            entry = [];
            transportListeners.set(eventName, entry);
        }
        return entry;
    }

    /**
     * Deliver one native transport notification. The sequence is
     * runtime-global and strictly increasing, exactly as the platform
     * modules mint it, so superseded events stay observable to consumers.
     */
    function emitTransportEvent(event: Record<string, unknown>): void {
        counters.transportEventSequence += 1n;
        const delivered = {
            ...event,
            eventSequence: String(counters.transportEventSequence),
        };
        for (const listener of [...listenersFor(V2_FAKE_TRANSPORT_EVENT_NAME)]) {
            listener(delivered);
        }
    }

    /** The projected peer set the native side ships with every state event. */
    function projectedPeers(session: FakeSession): NativeEditorPeerInfo[] {
        const peers: NativeEditorPeerInfo[] = [];
        if (session.localAwarenessLive && session.desiredAwareness != null) {
            const local = projectFakeLocalAwareness(
                session.desiredAwareness,
                session.localAwarenessCursor
            );
            peers.push({
                clientId: session.localClientId,
                clock: session.localClock,
                isLocal: true,
                state: local.state,
                cursor: local.cursor,
            });
        }
        peers.push(...session.remotePeers);
        peers.sort((left, right) => {
            const leftId = BigInt(left.clientId);
            const rightId = BigInt(right.clientId);
            return leftId < rightId ? -1 : leftId > rightId ? 1 : 0;
        });
        return peers;
    }

    /** Publish the session's current authority state as a transport event. */
    function emitTransportState(session: FakeSession, wakeReason: string): void {
        emitTransportEvent({
            editorId: session.editorId,
            generation: session.liveGeneration === null ? null : String(session.liveGeneration),
            kind: 'state',
            state: stateJson(session),
            peers: projectedPeers(session),
            diagnostics: {
                wakeReason,
                transportState: session.transportState,
                nextDeadlineMillis: null,
                remoteCommitApplied: false,
                peersChanged: false,
                renewedLocal: false,
                expiredPeerCount: 0,
            },
        });
    }

    function pendingFor(editorId: string): PendingRemote {
        let entry = pending.get(editorId);
        if (!entry) {
            entry = {
                docs: [],
                awarenessDeltas: [],
                applyLocalApiErrors: [],
                applyCommandErrors: [],
                awarenessBroadcastErrors: [],
            };
            pending.set(editorId, entry);
        }
        return entry;
    }

    function getSession(editorId: string): FakeSession | null {
        return sessions.get(editorId) ?? null;
    }

    function requireSession(editorId: string): FakeSession {
        const session = getSession(editorId);
        if (!session) throw new Error(`unknown fake session ${editorId}`);
        return session;
    }

    function withSession(
        editorId: string,
        run: (session: FakeSession) => Record<string, unknown>
    ): Record<string, unknown> {
        const session = getSession(editorId);
        if (!session || session.destroyed) {
            return lifecycleError('ENGINE_DESTROYED', 'editor session is not registered');
        }
        return run(session);
    }

    function requireLiveGeneration(
        session: FakeSession,
        generation: string,
        action: string
    ): Record<string, unknown> | null {
        const presentedGeneration = canonicalV2U64(generation);
        if (presentedGeneration == null) {
            return boundaryError('CONFIG_INVALID', 'generation must be canonical decimal u64 text');
        }
        if (session.liveGeneration == null || generation !== String(session.liveGeneration)) {
            return transportError(
                'TRANSPORT_STALE_GENERATION',
                `${action} rejected: stale transport generation`,
                {
                    presentedGeneration,
                    liveGeneration:
                        session.liveGeneration == null ? null : String(session.liveGeneration),
                }
            );
        }
        return null;
    }

    function revisionMismatchError(
        session: FakeSession,
        expectedRevision: string
    ): Record<string, unknown> {
        return operationError(
            'REVISION_MISMATCH',
            'base document revision does not match the engine revision',
            {
                expectedRevision,
                actualRevision: String(session.documentRevision),
            }
        );
    }

    function retireGeneration(session: FakeSession, next: FakeTransportState): void {
        session.liveGeneration = null;
        session.transportState = next;
        clearTransportAwareness(session);
    }

    function awarenessClockExhaustedError(): FakeErrorRecord {
        const error = errorRecord(
            'transport',
            'AWARENESS_CLOCK_EXHAUSTED',
            'local awareness clock exhausted; a fresh editor identity is required'
        );
        error.details = {
            requiresFreshEditorIdentity: true,
            retryable: false,
        };
        return error;
    }

    function advanceLocalAwarenessClock(
        session: FakeSession,
        transition: 'publish' | 'tombstone'
    ): FakeErrorRecord | null {
        const nextClock = session.localClock + 1;
        if (
            nextClock > V2_FAKE_U32_MAX ||
            (transition === 'publish' && nextClock === V2_FAKE_U32_MAX)
        ) {
            return awarenessClockExhaustedError();
        }
        session.localClock = nextClock;
        return null;
    }

    function clearTransportAwareness(session: FakeSession): FakeErrorRecord | null {
        if (session.localAwarenessLive) {
            const clockError = advanceLocalAwarenessClock(session, 'tombstone');
            if (clockError) return clockError;
            session.localAwarenessLive = false;
        }
        session.remotePeers = [];
        session.remoteAwarenessClocks.clear();
        session.remotePeerActivity.clear();
        return null;
    }

    function checkedAddV2U64(left: bigint, right: bigint): bigint | null {
        return left > V2_FAKE_U64_MAX - right ? null : left + right;
    }

    function setLocalAwarenessState(session: FakeSession): FakeErrorRecord | null {
        const clockError = advanceLocalAwarenessClock(session, 'publish');
        if (clockError) return clockError;
        session.localAwarenessLive = true;
        return null;
    }

    function enqueueLocalAwareness(session: FakeSession): FakeErrorRecord | null {
        const injected = pendingFor(session.editorId).awarenessBroadcastErrors.shift();
        if (injected) return injected;
        session.protocolQueue.push(awarenessFrame(session.localClock));
        session.lastLocalAwarenessPublishMillis = session.awarenessNowMillis;
        return null;
    }

    function publishLocalAwareness(session: FakeSession): FakeErrorRecord | null {
        const clockError = setLocalAwarenessState(session);
        if (clockError) return clockError;
        return enqueueLocalAwareness(session);
    }

    function withdrawLocalAwareness(session: FakeSession): FakeErrorRecord | null {
        if (session.localAwarenessLive) {
            const clockError = advanceLocalAwarenessClock(session, 'tombstone');
            if (clockError) return clockError;
            session.localAwarenessLive = false;
        }
        // A transport close may already have clocked the local tombstone.
        // Explicit withdrawal retains that exact frame for the next live
        // generation instead of advancing the clock again.
        session.pendingLocalAwarenessTombstone = awarenessFrame(session.localClock);
        session.pendingLocalAwarenessTombstoneRetryMillis = checkedAddV2U64(
            session.awarenessNowMillis,
            V2_FAKE_AWARENESS_RENEWAL_INTERVAL_MILLIS
        );
        return null;
    }

    function enqueuePendingLocalAwarenessTombstone(session: FakeSession): FakeErrorRecord | null {
        const tombstone = session.pendingLocalAwarenessTombstone;
        if (tombstone == null) return null;
        const injected = pendingFor(session.editorId).awarenessBroadcastErrors.shift();
        if (injected) {
            session.pendingLocalAwarenessTombstoneRetryMillis = checkedAddV2U64(
                session.awarenessNowMillis,
                V2_FAKE_AWARENESS_RENEWAL_INTERVAL_MILLIS
            );
            return injected;
        }
        session.protocolQueue.push(tombstone);
        session.pendingLocalAwarenessTombstone = null;
        session.pendingLocalAwarenessTombstoneRetryMillis = null;
        return null;
    }

    function nextAwarenessDeadline(session: FakeSession): bigint | null {
        const localRenewal =
            session.transportState === 'Synchronized' && session.desiredAwareness != null
                ? session.lastLocalAwarenessPublishMillis == null
                    ? session.awarenessNowMillis
                    : checkedAddV2U64(
                          session.lastLocalAwarenessPublishMillis,
                          V2_FAKE_AWARENESS_RENEWAL_INTERVAL_MILLIS
                      )
                : null;
        const tombstoneRetry =
            session.transportState === 'Synchronized' &&
            session.pendingLocalAwarenessTombstone != null
                ? session.pendingLocalAwarenessTombstoneRetryMillis
                : null;
        let remoteExpiry: bigint | null = null;
        for (const seenAt of session.remotePeerActivity.values()) {
            const deadline = checkedAddV2U64(seenAt, V2_FAKE_AWARENESS_EXPIRY_MILLIS);
            if (deadline == null) continue;
            if (remoteExpiry == null || deadline < remoteExpiry) remoteExpiry = deadline;
        }
        const deadlines = [localRenewal, tombstoneRetry, remoteExpiry].filter(
            (deadline): deadline is bigint => deadline != null
        );
        return deadlines.reduce<bigint | null>(
            (earliest, deadline) => (earliest == null || deadline < earliest ? deadline : earliest),
            null
        );
    }

    function applyRemoteAwarenessDelta(
        session: FakeSession,
        entries: NativeEditorPeerInfo[]
    ): void {
        for (const peer of entries) {
            const clientId = canonicalV2U64(peer.clientId);
            if (clientId == null || clientId === session.localClientId) continue;
            const currentClock = session.remoteAwarenessClocks.get(clientId);
            const currentPeerIndex = session.remotePeers.findIndex(
                (candidate) => candidate.clientId === clientId
            );
            const isTombstone = peer.state == null;
            const removesEqualClockLivePeer =
                isTombstone && currentPeerIndex >= 0 && currentClock === peer.clock;
            if (currentClock != null && peer.clock <= currentClock && !removesEqualClockLivePeer) {
                continue;
            }
            if (currentClock == null && isTombstone) continue;

            session.remoteAwarenessClocks.set(clientId, peer.clock);
            if (isTombstone) {
                if (currentPeerIndex >= 0) session.remotePeers.splice(currentPeerIndex, 1);
                session.remotePeerActivity.delete(clientId);
                continue;
            }

            const admittedPeer = { ...peer, clientId, isLocal: false };
            if (currentPeerIndex >= 0) {
                session.remotePeers[currentPeerIndex] = admittedPeer;
            } else {
                session.remotePeers.push(admittedPeer);
            }
            session.remotePeerActivity.set(clientId, session.awarenessNowMillis);
        }
        session.remotePeers.sort((left, right) => {
            const leftId = BigInt(left.clientId);
            const rightId = BigInt(right.clientId);
            return leftId < rightId ? -1 : leftId > rightId ? 1 : 0;
        });
    }

    function remoteAwarenessClockLimitError(
        session: FakeSession,
        entries: NativeEditorPeerInfo[]
    ): FakeErrorRecord | null {
        for (const peer of entries) {
            const clientId = canonicalV2U64(peer.clientId);
            if (clientId == null) continue;
            const clockLimit =
                clientId === session.localClientId
                    ? session.localClock
                    : V2_FAKE_MAX_ADMITTED_REMOTE_AWARENESS_CLOCK;
            if (peer.clock <= clockLimit) continue;

            const error = errorRecord(
                'transport',
                'TRANSPORT_AWARENESS_LIMIT_EXCEEDED',
                'awareness frame handling failed'
            );
            error.details = {
                action: 'receiveMessage',
                cause: {
                    code: 'INPUT_LIMIT_EXCEEDED',
                    message: `input exceeds limit ${clockLimit}: ${peer.clock}`,
                    limit: clockLimit,
                    actual: peer.clock,
                    details: { field: 'awarenessClock' },
                },
            };
            return error;
        }
        return null;
    }

    function validateAndSortAwarenessDelta(
        entries: NativeEditorPeerInfo[]
    ): NativeEditorPeerInfo[] | null {
        const validated: NativeEditorPeerInfo[] = [];
        for (const peer of entries) {
            const clientId = canonicalV2U64(peer.clientId);
            const clock = exactV2U32(peer.clock);
            if (clientId == null || clock == null) return null;
            validated.push({ ...peer, clientId, clock });
        }
        validated.sort((left, right) => {
            const leftId = BigInt(left.clientId);
            const rightId = BigInt(right.clientId);
            return leftId < rightId ? -1 : leftId > rightId ? 1 : 0;
        });
        return validated;
    }

    function malformedAwarenessReceiveError(): FakeErrorRecord {
        const error = errorRecord(
            'transport',
            'TRANSPORT_PROTOCOL_INVALID',
            'awareness frame handling failed'
        );
        error.details = {
            action: 'receiveMessage',
            cause: {
                code: 'COLLABORATION_DECODE_FAILED',
                message: V2_FAKE_MALFORMED_AWARENESS_MESSAGE,
                limit: null,
                actual: null,
                details: null,
            },
        };
        return error;
    }

    function awarenessReceiveError(cause: FakeErrorRecord): FakeErrorRecord {
        const error = errorRecord('transport', cause.code, 'awareness frame handling failed');
        error.details = {
            action: 'receiveMessage',
            cause: {
                code: cause.code,
                message: cause.message,
                limit: cause.limit,
                actual: cause.actual,
                details: cause.details,
            },
        };
        return error;
    }

    function isAwarenessReservationFailure(cause: FakeErrorRecord): boolean {
        return (
            cause.code === 'TRANSPORT_REPLY_LIMIT_EXCEEDED' ||
            cause.code === 'TRANSPORT_RESOURCE_EXHAUSTED'
        );
    }

    function handshakeReservationReceiveError(cause: FakeErrorRecord): FakeErrorRecord {
        if (cause.code === 'TRANSPORT_REPLY_LIMIT_EXCEEDED') {
            const field =
                typeof cause.details?.field === 'string'
                    ? cause.details.field
                    : 'maxPendingOutboxMessages';
            const error = errorRecord(
                'transport',
                cause.code,
                `${field} exceeded while receiving a protocol message`
            );
            error.limit = cause.limit;
            error.actual = cause.actual;
            error.details = {
                action: 'receiveMessage',
                field,
                limit: Number(cause.limit),
                actual: Number(cause.actual),
            };
            return error;
        }
        const error = errorRecord(
            'transport',
            cause.code,
            'protocol reply capacity could not be reserved'
        );
        error.details = {
            action: 'receiveMessage',
            reason: 'replyReservation',
        };
        return error;
    }

    function queueDocumentUpdate(session: FakeSession): void {
        if (!session.roomBound) return;
        session.documentQueue.push(documentFrame(session.documentRevision));
    }

    function applyReplacement(session: FakeSession, nextDoc: DocumentJSON, history: string): void {
        if (history === 'undoableBoundary') {
            session.undoStack.push(cloneDoc(session.doc));
            session.redoStack = [];
        } else {
            session.undoStack = [];
            session.redoStack = [];
        }
        installFakeDocument(session, nextDoc);
        session.documentRevision += 1;
        session.documentOrigin = 'import';
        queueDocumentUpdate(session);
    }

    function admitReplacement(session: FakeSession): Record<string, unknown> | null {
        if (session.documentState === 'AwaitRemote') {
            return operationError(
                'ENGINE_NOT_READY',
                'room document is awaiting the remote initial state'
            );
        }
        if (
            session.roomBound &&
            (session.transportState === 'Connecting' ||
                session.transportState === 'Handshaking' ||
                session.transportState === 'Synchronized')
        ) {
            return lifecycleError(
                'WHOLE_DOCUMENT_REPLACEMENT_CONNECTED',
                'whole-document replacement is rejected while a transport is live'
            );
        }
        return null;
    }

    function stateJson(session: FakeSession): string {
        return JSON.stringify({
            documentState: session.documentState,
            transportState: session.transportState,
            renderState: session.renderState,
            documentRevision: String(session.documentRevision),
            documentOrigin: session.documentOrigin,
            stateRevision: String(session.stateRevision),
            canUndo: session.undoStack.length > 0,
            canRedo: session.redoStack.length > 0,
        });
    }

    function handleReceive(session: FakeSession, message: Uint8Array): Record<string, unknown> {
        const tag = message.length > 1 ? message[1] : message[0];
        const outcome = (fields: {
            framesDecoded?: number;
            repliesEnqueued?: number;
            replyBytesEnqueued?: number;
            remoteCommitApplied?: boolean;
            documentPromoted?: boolean;
            close?: { disposition: 'retryable' | 'incompatible'; error: FakeErrorRecord } | null;
        }): Record<string, unknown> =>
            okRecord(
                JSON.stringify({
                    framesDecoded: fields.framesDecoded ?? 1,
                    repliesEnqueued: fields.repliesEnqueued ?? 0,
                    replyBytesEnqueued: fields.replyBytesEnqueued ?? 0,
                    remoteCommitApplied: fields.remoteCommitApplied ?? false,
                    documentPromoted: fields.documentPromoted ?? false,
                    transportState: session.transportState,
                    close: fields.close ?? null,
                })
            );

        // Sync Step 1: enqueue the Step 2 reply (protocol bucket).
        if (tag === 0x00 && message[2] === 1 && message[0] === 0) {
            session.replySequence += 1;
            const reply = protocolReplyFrame(session.replySequence);
            session.protocolQueue.push(reply);
            return outcome({ repliesEnqueued: 1, replyBytesEnqueued: reply.length });
        }
        // Sync Step 2: the only synchronization gate.
        if (tag === 0x00 && message[2] === 2 && message[0] === 0) {
            if (session.transportState !== 'Handshaking') {
                return outcome({});
            }
            if (session.desiredAwareness != null) {
                const clockError = publishLocalAwareness(session);
                if (clockError) {
                    if (isAwarenessReservationFailure(clockError)) {
                        const error = handshakeReservationReceiveError(clockError);
                        retireGeneration(session, 'Disconnected');
                        return outcome({ close: { disposition: 'retryable', error } });
                    }
                    const error = awarenessReceiveError(clockError);
                    retireGeneration(session, 'Incompatible');
                    return outcome({ close: { disposition: 'incompatible', error } });
                }
            }
            let documentPromoted = false;
            if (session.documentState === 'AwaitRemote') {
                const remote = pendingFor(session.editorId);
                installFakeDocument(session, remote.docs.shift() ?? EMPTY_DOC);
                session.documentState = 'RoomReady';
                session.renderState = 'Ready';
                session.documentRevision += 1;
                session.documentOrigin = 'remoteCollaboration';
                documentPromoted = true;
            }
            session.transportState = 'Synchronized';
            return outcome({ documentPromoted });
        }
        // Step 2 without a valid configured fragment: unchanged doc, Incompatible.
        if (tag === 0x00 && message[2] === 5 && message[0] === 0) {
            const error = errorRecord(
                'document',
                'DOCUMENT_INVALID',
                'step 2 did not install a valid configured fragment'
            );
            retireGeneration(session, 'Incompatible');
            return outcome({ close: { disposition: 'incompatible', error } });
        }
        // Remote document update (requires Synchronized; never synchronizes).
        if (tag === 0x01) {
            if (session.transportState !== 'Synchronized') {
                const error = errorRecord(
                    'transport',
                    'TRANSPORT_PROTOCOL_INVALID',
                    'update frame received before synchronization'
                );
                retireGeneration(session, 'Disconnected');
                return outcome({ close: { disposition: 'retryable', error } });
            }
            const remote = pendingFor(session.editorId);
            const nextDoc = remote.docs.shift();
            if (!nextDoc) {
                const error = errorRecord(
                    'transport',
                    'TRANSPORT_PROTOCOL_INVALID',
                    'update frame without a queued remote document'
                );
                retireGeneration(session, 'Disconnected');
                return outcome({ close: { disposition: 'retryable', error } });
            }
            installFakeDocument(session, nextDoc);
            session.documentRevision += 1;
            session.documentOrigin = 'remoteCollaboration';
            return outcome({ remoteCommitApplied: true });
        }
        // Remote awareness state.
        if (tag === 0x02) {
            const remote = pendingFor(session.editorId);
            const entries = validateAndSortAwarenessDelta(remote.awarenessDeltas.shift() ?? []);
            if (entries == null) {
                const error = malformedAwarenessReceiveError();
                retireGeneration(session, 'Disconnected');
                return outcome({ close: { disposition: 'retryable', error } });
            }
            const clockLimitError = remoteAwarenessClockLimitError(session, entries);
            if (clockLimitError) {
                retireGeneration(session, 'Incompatible');
                return outcome({
                    close: { disposition: 'incompatible', error: clockLimitError },
                });
            }
            applyRemoteAwarenessDelta(session, entries);
            return outcome({});
        }
        if (tag === 0xff) {
            const error = errorRecord(
                'transport',
                'TRANSPORT_PROTOCOL_INVALID',
                'malformed protocol frame'
            );
            retireGeneration(session, 'Disconnected');
            return outcome({ close: { disposition: 'retryable', error } });
        }
        if (tag === 0xfe) {
            const error = errorRecord(
                'document',
                'DOCUMENT_INVALID',
                'permanently inadmissible remote document state'
            );
            retireGeneration(session, 'Incompatible');
            return outcome({ close: { disposition: 'incompatible', error } });
        }
        const error = errorRecord(
            'transport',
            'TRANSPORT_PROTOCOL_INVALID',
            'unknown protocol frame'
        );
        retireGeneration(session, 'Disconnected');
        return outcome({ close: { disposition: 'retryable', error } });
    }
    return {
        counters,
        sessions,
        liveIds,
        withSession,
        stateJson,
        admitReplacement,
        applyReplacement,
        revisionMismatchError,
        queueDocumentUpdate,
        pendingFor,
        withdrawLocalAwareness,
        enqueuePendingLocalAwarenessTombstone,
        setLocalAwarenessState,
        enqueueLocalAwareness,
        clearTransportAwareness,
        emitTransportState,
        protocolAdapterResolutions,
        listenersFor,
        getSession,
        requireSession,
        handleReceive,
        retireGeneration,
        emitTransportEvent,
    };
}
