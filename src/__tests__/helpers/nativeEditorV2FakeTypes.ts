import type { DocumentJSON, NativeEditorV2PeerInfo } from '../../NativeEditorBridge';
import {
    type FakeDocumentState,
    type FakeTransportState,
    type FakeNativeEditorLocalAwarenessWireIntent,
    type FakeErrorRecord,
} from './nativeEditorV2FakeRecords';

/** Outbound document frames carry their revision so tests can assert order. */
export function documentFrame(revision: number): Uint8Array {
    return new Uint8Array([0x64, revision & 0xff]);
}

export function protocolReplyFrame(sequence: number): Uint8Array {
    return new Uint8Array([0x70, sequence & 0xff]);
}

export function awarenessFrame(clock: number): Uint8Array {
    return new Uint8Array([0x61, clock & 0xff]);
}

export interface FakeSession {
    editorId: string;
    roomBound: boolean;
    documentId: string | null;
    lineageId: string | null;
    documentState: FakeDocumentState;
    transportState: FakeTransportState;
    renderState: 'Loading' | 'Ready';
    documentRevision: number;
    documentOrigin:
        | 'nativeView'
        | 'jsApi'
        | 'remoteCollaboration'
        | 'history'
        | 'restore'
        | 'import';
    stateRevision: number;
    doc: DocumentJSON;
    undoStack: DocumentJSON[];
    redoStack: DocumentJSON[];
    activeMarks: Record<string, boolean>;
    activeMarkAttrs: Record<string, Record<string, unknown>>;
    activeNodes: Record<string, boolean>;
    hasStoredMarks: boolean;
    hasStoredNodes: boolean;
    selection: { anchor: number; head: number };
    liveGeneration: bigint | null;
    lastIssuedGeneration: bigint;
    protocolQueue: Uint8Array[];
    documentQueue: Uint8Array[];
    maxAwarenessPeerBytes: number;
    desiredAwareness: FakeNativeEditorLocalAwarenessWireIntent | null;
    localAwarenessCursor: { anchor: number; head: number } | null;
    localClientId: string;
    localClock: number;
    localAwarenessLive: boolean;
    pendingLocalAwarenessTombstone: Uint8Array | null;
    pendingLocalAwarenessTombstoneRetryMillis: bigint | null;
    remotePeers: NativeEditorV2PeerInfo[];
    remoteAwarenessClocks: Map<string, number>;
    awarenessNowMillis: bigint;
    lastLocalAwarenessPublishMillis: bigint | null;
    remotePeerActivity: Map<string, bigint>;
    destroyed: boolean;
    replySequence: number;
    /** Latest transport intent the TypeScript bridge configured, if any. */
    transportConfig: FakeTransportWireConfig | null;
}

/**
 * The transport intent TypeScript hands to the platform module. The native
 * side — never TypeScript — owns the socket that acts on it.
 */
export interface FakeTransportWireConfig {
    url: string;
    connect: boolean;
    protocolAdapter?: {
        protocols: string[];
        timeoutMillis?: number;
        terminalCloseCodes?: number[];
    };
}

/** One resolved protocol-adapter reply the bridge handed back to native. */
export interface FakeProtocolAdapterResolution {
    editorId: string;
    attemptId: string;
    eventId: string;
    responseJson: string;
}

export interface FakeV2SessionHandle {
    editorId: string;
}

export type FakeAwarenessBroadcastFailureCode =
    | 'TRANSPORT_REPLY_LIMIT_EXCEEDED'
    | 'TRANSPORT_RESOURCE_EXHAUSTED';

export interface FakeNativeEditorV2Runtime {
    /** The editorV2* entries, already jest.fn-wrapped for call assertions. */
    module: Record<string, jest.Mock>;
    /** Create a room session directly (mirrors what the TS bridge create sends). */
    sessions(): readonly FakeSession[];
    session(editorId: string): FakeSession;
    /** Ids the module marked live for view binding at create (view-binding surface). */
    liveEditorIds(): string[];
    /** The transport intent TypeScript last configured for this editor. */
    transportConfig(editorId: string): FakeTransportWireConfig | null;
    /** Native socket open: `Connecting` -> `Handshaking`, queueing Step 1. */
    transportOpen(editorId: string): void;
    /** Deliver one inbound frame the way the native socket would. */
    transportReceive(editorId: string, frame: Uint8Array): void;
    /** Native socket close; 1008 parks the transport `Incompatible`. */
    transportClose(editorId: string, code?: number | null): void;
    /** Deliver one native transport error notification. */
    emitTransportError(editorId: string, error: FakeErrorRecord): void;
    /** Deliver one protocol-adapter prelude notification. */
    emitProtocolAdapterEvent(
        editorId: string,
        event: {
            attemptId: string;
            eventId: string;
            phase: 'open' | 'message';
            negotiatedProtocol: string | null;
            frame?: { type: 'text' | 'binary'; data: string };
        }
    ): void;
    /** Adapter replies the bridge handed back to native, oldest first. */
    protocolAdapterResolutions(): readonly FakeProtocolAdapterResolution[];
    /** Queue the document the next accepted server Step 2 / update installs. */
    pushRemoteDoc(editorId: string, doc: DocumentJSON): void;
    /** Queue the clocked per-client delta the next inbound awareness frame applies. */
    pushRemotePeers(editorId: string, peers: NativeEditorV2PeerInfo[]): void;
    /** Seed the exact last-issued u64 generation for boundary tests. */
    seedLastIssuedGeneration(editorId: string, generation: string): void;
    /** Seed the exact Rust-owned local awareness u32 clock for boundary tests. */
    seedLocalAwarenessClock(editorId: string, clock: number): void;
    /** Retire the live generation natively without telling TypeScript. */
    retireLiveGeneration(editorId: string): void;
    /** One-shot error injected into the named entry (currently applyLocalApi). */
    injectNextApplyLocalApiError(editorId: string, error: FakeErrorRecord): void;
    /** One-shot error injected into applyCommand. */
    injectNextApplyCommandError(editorId: string, error: FakeErrorRecord): void;
    /** Fail the next local awareness outbox reservation after retaining native state. */
    injectNextAwarenessBroadcastFailure(
        editorId: string,
        code: FakeAwarenessBroadcastFailureCode
    ): void;
    /** Frames the session has queued, oldest first (protocol then document). */
    queuedFrames(editorId: string): Uint8Array[];
}

export interface PendingRemote {
    docs: DocumentJSON[];
    awarenessDeltas: NativeEditorV2PeerInfo[][];
    applyLocalApiErrors: FakeErrorRecord[];
    applyCommandErrors: FakeErrorRecord[];
    awarenessBroadcastErrors: FakeErrorRecord[];
}
