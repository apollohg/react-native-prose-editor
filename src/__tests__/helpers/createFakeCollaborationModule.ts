import { createFakeRuntimeState } from './createFakeRuntimeState';
import {
    boundaryError,
    awarenessPeerBytesLimitError,
    okRecord,
    errRecord,
    transportError,
    V2_FAKE_U64_MAX,
} from './nativeEditorV2FakeRecords';
import {
    parseFakeAwarenessIntent,
    fakeCursorForIntent,
    parseFakeTransportConfig,
} from './nativeEditorV2FakeAwareness';
import { fakeScalarDocumentMap } from './nativeEditorV2FakeDocument';

export function createFakeCollaborationModule(
    context: Pick<
        ReturnType<typeof createFakeRuntimeState>,
        | 'withSession'
        | 'withdrawLocalAwareness'
        | 'enqueuePendingLocalAwarenessTombstone'
        | 'setLocalAwarenessState'
        | 'enqueueLocalAwareness'
        | 'clearTransportAwareness'
        | 'emitTransportState'
        | 'protocolAdapterResolutions'
        | 'listenersFor'
    >
) {
    const {
        withSession,
        withdrawLocalAwareness,
        enqueuePendingLocalAwarenessTombstone,
        setLocalAwarenessState,
        enqueueLocalAwareness,
        clearTransportAwareness,
        emitTransportState,
        protocolAdapterResolutions,
        listenersFor,
    } = context;

    const module2: Record<string, jest.Mock> = {
        editorV2CollaborationSetAwareness: jest.fn((editorId: string, awarenessJson: string) =>
            withSession(editorId, (session) => {
                if (!session.roomBound) {
                    return boundaryError(
                        'CONFIG_INVALID',
                        'local sessions have no attached collaboration runtime'
                    );
                }
                const awarenessBytes = new TextEncoder().encode(awarenessJson).byteLength;
                if (awarenessBytes > session.maxAwarenessPeerBytes) {
                    return awarenessPeerBytesLimitError(
                        session.maxAwarenessPeerBytes,
                        awarenessBytes
                    );
                }
                if (awarenessJson.trim() === 'null') {
                    if (session.desiredAwareness == null) return okRecord(true);
                    const clockError = withdrawLocalAwareness(session);
                    if (clockError) return errRecord(clockError);
                    session.desiredAwareness = null;
                    session.localAwarenessCursor = null;
                    session.lastLocalAwarenessPublishMillis = null;
                    if (session.transportState === 'Synchronized') {
                        const reservationError = enqueuePendingLocalAwarenessTombstone(session);
                        if (reservationError) return errRecord(reservationError);
                    }
                } else {
                    const desiredAwareness = parseFakeAwarenessIntent(awarenessJson);
                    if ('domain' in desiredAwareness) return errRecord(desiredAwareness);
                    const cursor = fakeCursorForIntent(
                        desiredAwareness.selection,
                        session.localAwarenessCursor
                    );
                    // Only a caller-stated position is validated: a retained
                    // sticky cursor is already engine-owned.
                    if (cursor != null && desiredAwareness.selection != null) {
                        const positionMap = fakeScalarDocumentMap(session.doc);
                        if (
                            positionMap.clampDocumentOffset(cursor.anchor) !== cursor.anchor ||
                            positionMap.clampDocumentOffset(cursor.head) !== cursor.head
                        ) {
                            return boundaryError(
                                'AWARENESS_STATE_INVALID',
                                'local awareness selection is outside the current document'
                            );
                        }
                    }
                    const clockError = setLocalAwarenessState(session);
                    if (clockError) return errRecord(clockError);
                    session.pendingLocalAwarenessTombstone = null;
                    session.pendingLocalAwarenessTombstoneRetryMillis = null;
                    session.desiredAwareness = desiredAwareness;
                    session.localAwarenessCursor = cursor;
                }
                if (awarenessJson.trim() !== 'null' && session.transportState === 'Synchronized') {
                    const reservationError = enqueueLocalAwareness(session);
                    if (reservationError) return errRecord(reservationError);
                }
                return okRecord(true);
            })
        ),

        editorV2CollaborationConfigureTransport: jest.fn((editorId: string, configJson: string) =>
            withSession(editorId, (session) => {
                if (!session.roomBound) {
                    return transportError(
                        'TRANSPORT_NOT_ROOM_BOUND',
                        'local-only sessions have no room binding to configure'
                    );
                }
                const config = parseFakeTransportConfig(configJson);
                if (config != null && 'domain' in config) return errRecord(config);
                session.transportConfig = config;
                if (config === null) {
                    session.liveGeneration = null;
                    session.transportState = 'Detached';
                    clearTransportAwareness(session);
                    emitTransportState(session, 'configureDetach');
                    return okRecord(true);
                }
                if (!config.connect) {
                    if (session.transportState !== 'Disconnected') {
                        session.liveGeneration = null;
                        session.transportState = 'Disconnected';
                        clearTransportAwareness(session);
                    }
                    emitTransportState(session, 'configureDisconnect');
                    return okRecord(true);
                }
                // Connect intent: the native transport mints the generation
                // and starts the attempt without any TypeScript involvement.
                if (session.transportState === 'Incompatible') {
                    emitTransportState(session, 'configureParked');
                    return okRecord(true);
                }
                if (
                    session.transportState === 'Detached' ||
                    session.transportState === 'Disconnected'
                ) {
                    if (session.lastIssuedGeneration === V2_FAKE_U64_MAX) {
                        return transportError(
                            'TRANSPORT_GENERATION_EXHAUSTED',
                            'transport generation space is exhausted',
                            { action: 'configureTransport', transportState: session.transportState }
                        );
                    }
                    session.lastIssuedGeneration += 1n;
                    session.liveGeneration = session.lastIssuedGeneration;
                    session.transportState = 'Connecting';
                }
                emitTransportState(session, 'configureConnect');
                return okRecord(true);
            })
        ),

        editorV2CollaborationResolveProtocolAdapter: jest.fn(
            (editorId: string, attemptId: string, eventId: string, responseJson: string) =>
                withSession(editorId, () => {
                    protocolAdapterResolutions.push({
                        editorId,
                        attemptId,
                        eventId,
                        responseJson,
                    });
                    return okRecord(true);
                })
        ),

        addListener: jest.fn((eventName: string, listener: (event: unknown) => void) => {
            const listeners = listenersFor(eventName);
            listeners.push(listener);
            return {
                remove: () => {
                    const index = listeners.indexOf(listener);
                    if (index >= 0) listeners.splice(index, 1);
                },
            };
        }),
    };
    return { module2 };
}
