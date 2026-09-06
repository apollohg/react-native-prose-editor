import type { DocumentJSON } from '../../NativeEditorBridge';
import { createFakeRuntimeState } from './createFakeRuntimeState';
import {
    boundaryError,
    EMPTY_DOC,
    V2_FAKE_DEFAULT_MAX_AWARENESS_PEER_BYTES,
    okRecord,
    snapshotError,
} from './nativeEditorV2FakeRecords';
import { type FakeSession } from './nativeEditorV2FakeTypes';
import { cloneDoc, fakeDocForHtml } from './nativeEditorV2FakeDocument';
import { isFakeRecord, installFakeDocument } from './nativeEditorV2FakeAwareness';

export function createFakeLifecycleModule(
    context: Pick<
        ReturnType<typeof createFakeRuntimeState>,
        'counters' | 'sessions' | 'liveIds' | 'withSession'
    >
) {
    const { counters, sessions, liveIds, withSession } = context;

    const module0: Record<string, jest.Mock> = {
        editorV2Create: jest.fn((configJson: string, snapshotState: Uint8Array | null) => {
            let config: Record<string, unknown>;
            try {
                config = JSON.parse(configJson) as Record<string, unknown>;
            } catch {
                return boundaryError('CONFIG_INVALID', 'malformed create config');
            }
            const initialization = config.initialization as Record<string, unknown> | undefined;
            if (!initialization || typeof initialization.type !== 'string') {
                return boundaryError('CONFIG_INVALID', 'missing initialization');
            }
            counters.editorId += 1;
            const editorId = String(counters.editorId);
            const base: FakeSession = {
                editorId,
                roomBound: false,
                documentId: null,
                lineageId: null,
                documentState: 'LocalReady',
                transportState: 'Detached',
                renderState: 'Ready',
                documentRevision: 1,
                documentOrigin: 'import',
                stateRevision: 1,
                doc: cloneDoc(EMPTY_DOC),
                undoStack: [],
                redoStack: [],
                activeMarks: {},
                activeMarkAttrs: {},
                activeNodes: {},
                hasStoredMarks: false,
                hasStoredNodes: false,
                selection: { anchor: 1, head: 1 },
                liveGeneration: null,
                lastIssuedGeneration: 0n,
                protocolQueue: [],
                documentQueue: [],
                maxAwarenessPeerBytes: (() => {
                    const limits = config.limits;
                    const collaboration =
                        isFakeRecord(limits) && isFakeRecord(limits.collaboration)
                            ? limits.collaboration
                            : null;
                    return typeof collaboration?.maxAwarenessPeerBytes === 'number'
                        ? collaboration.maxAwarenessPeerBytes
                        : V2_FAKE_DEFAULT_MAX_AWARENESS_PEER_BYTES;
                })(),
                desiredAwareness: null,
                localAwarenessCursor: null,
                localClientId: String((counters.clientId += 1)),
                localClock: 0,
                localAwarenessLive: false,
                pendingLocalAwarenessTombstone: null,
                pendingLocalAwarenessTombstoneRetryMillis: null,
                remotePeers: [],
                remoteAwarenessClocks: new Map(),
                awarenessNowMillis: 0n,
                lastLocalAwarenessPublishMillis: null,
                remotePeerActivity: new Map(),
                destroyed: false,
                replySequence: 0,
                transportConfig: null,
            };
            if (initialization.type === 'localJson') {
                base.doc = cloneDoc((initialization.json as DocumentJSON) ?? EMPTY_DOC);
            } else if (initialization.type === 'localHtml') {
                base.doc = fakeDocForHtml(String(initialization.html ?? ''));
            } else if (initialization.type === 'room') {
                base.roomBound = true;
                base.documentId = String(initialization.documentId ?? '');
                base.lineageId = String(initialization.lineageId ?? '');
                base.transportState = 'Disconnected';
                if (initialization.snapshot != null && snapshotState != null) {
                    try {
                        const parsed = JSON.parse(new TextDecoder().decode(snapshotState)) as {
                            doc: DocumentJSON;
                            revision?: number;
                        };
                        base.doc = cloneDoc(parsed.doc);
                        base.documentRevision =
                            typeof parsed.revision === 'number' ? parsed.revision : 1;
                    } catch {
                        return boundaryError('CONFIG_INVALID', 'malformed snapshot state');
                    }
                    base.documentState = 'RoomReady';
                    base.renderState = 'Ready';
                } else {
                    base.documentState = 'AwaitRemote';
                    base.renderState = 'Loading';
                    base.doc = cloneDoc(EMPTY_DOC);
                }
            } else if (initialization.type !== 'localEmpty') {
                return boundaryError('CONFIG_INVALID', 'unknown initialization type');
            }
            sessions.set(editorId, base);
            // Mirrors the module marking the public id live for view binding.
            liveIds.add(editorId);
            return okRecord(JSON.stringify({ editorId }));
        }),

        editorV2Destroy: jest.fn((editorId: string) =>
            withSession(editorId, (session) => {
                session.destroyed = true;
                session.transportState = 'Destroyed';
                session.liveGeneration = null;
                session.localAwarenessLive = false;
                session.remotePeers = [];
                session.remoteAwarenessClocks.clear();
                session.remotePeerActivity.clear();
                liveIds.delete(editorId);
                return okRecord(true);
            })
        ),

        editorV2SnapshotExport: jest.fn((editorId: string) =>
            withSession(editorId, (session) =>
                okRecord({
                    metadataJson: JSON.stringify({
                        formatVersion: 1,
                        documentId: session.documentId ?? '',
                        lineageId: session.lineageId ?? '',
                        fragmentName: 'prosemirror',
                        schemaFingerprint: 'fakefingerprint',
                    }),
                    encodedState: new TextEncoder().encode(
                        JSON.stringify({ doc: session.doc, revision: session.documentRevision })
                    ),
                })
            )
        ),

        editorV2SnapshotRestore: jest.fn(
            (editorId: string, metadataJson: string, encodedState: Uint8Array) =>
                withSession(editorId, (session) => {
                    if (
                        session.transportState !== 'Detached' &&
                        session.transportState !== 'Disconnected'
                    ) {
                        return snapshotError(
                            'SNAPSHOT_RESTORE_CONNECTED',
                            'snapshot restore is only admitted while detached or disconnected'
                        );
                    }
                    if (session.documentQueue.length > 0) {
                        return snapshotError(
                            'SNAPSHOT_OUTBOX_NOT_EMPTY',
                            'unsent local document updates block snapshot restore'
                        );
                    }
                    const metadata = JSON.parse(metadataJson) as Record<string, unknown>;
                    if (
                        session.roomBound &&
                        session.documentId != null &&
                        metadata.documentId !== session.documentId
                    ) {
                        return snapshotError(
                            'SNAPSHOT_METADATA_MISMATCH',
                            'snapshot document id does not match the room'
                        );
                    }
                    const parsed = JSON.parse(new TextDecoder().decode(encodedState)) as {
                        doc: DocumentJSON;
                        revision?: number;
                    };
                    installFakeDocument(session, parsed.doc);
                    session.documentRevision =
                        typeof parsed.revision === 'number' ? parsed.revision : 1;
                    session.documentOrigin = 'restore';
                    session.documentState = 'RoomReady';
                    session.renderState = 'Ready';
                    session.transportState = 'Disconnected';
                    session.liveGeneration = null;
                    session.protocolQueue = [];
                    session.localAwarenessLive = false;
                    session.remotePeers = [];
                    session.remoteAwarenessClocks.clear();
                    session.remotePeerActivity.clear();
                    session.lastLocalAwarenessPublishMillis = null;
                    session.undoStack = [];
                    session.redoStack = [];
                    session.localClientId = String((counters.clientId += 1));
                    return okRecord(
                        JSON.stringify({
                            changed: true,
                            documentRevision: String(session.documentRevision),
                        })
                    );
                })
        ),
    };
    return { module0 };
}
