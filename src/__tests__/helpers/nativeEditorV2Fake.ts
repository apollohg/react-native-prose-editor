// ─── Fake native v2 runtime ──────────────────────────────────
// A faithful, stateful fake of the 22 `editorV2*` production entries. It
// reproduces the Rust-side semantics the TypeScript controller and the v2
// document bindings rely on:
// - room sessions auto-attach the collaboration runtime; local sessions
//   refuse `beginConnect` with transport/TRANSPORT_NOT_ROOM_BOUND;
// - authoritative transport lifecycle: begin_connect only from Disconnected,
//   open -> Handshaking, accepted Step 2 -> Synchronized, close code 1008 ->
//   Incompatible (anything else retryable), stale generations rejected with
//   TRANSPORT_STALE_GENERATION;
// - document readiness: room without snapshot starts AwaitRemote/Loading and
//   rejects local edits with ENGINE_NOT_READY until an accepted Step 2;
// - whole-document replacement is rejected while Connecting/Handshaking/
//   Synchronized (WHOLE_DOCUMENT_REPLACEMENT_CONNECTED);
// - offline local edits queue document frames; take_outbound drains protocol
//   replies before document frames, one frame per call, empty queue -> empty
//   bytes;
// - snapshot restore is the detach/reattach path out of Incompatible;
// - a real undo stack so replace/reset history policy is verifiable through
//   engine undo state instead of document content.
//
// The fake owns awareness clocks (the TypeScript side must never see a clock
// field): publishing desired awareness while a generation is live enqueues an
// awareness frame carrying a Rust-side monotonically increasing clock.

import type { DocumentJSON, NativeEditorV2PeerInfo } from '../NativeEditorBridge';

export const V2_FAKE_STEP1_FRAME = new Uint8Array([0, 0, 1]);
export const V2_FAKE_STEP2_FRAME = new Uint8Array([0, 0, 2]);
export const V2_FAKE_STEP2_INVALID_FRAGMENT_FRAME = new Uint8Array([0, 0, 5]);
export const V2_FAKE_UPDATE_FRAME = new Uint8Array([0, 1, 1]);
export const V2_FAKE_AWARENESS_FRAME = new Uint8Array([0, 2, 1]);
export const V2_FAKE_MALFORMED_FRAME = new Uint8Array([0xff]);
export const V2_FAKE_INCOMPATIBLE_FRAME = new Uint8Array([0xfe]);

const EMPTY_DOC: DocumentJSON = { type: 'doc', content: [{ type: 'paragraph' }] };

type FakeTransportState =
    | 'Detached'
    | 'Disconnected'
    | 'Connecting'
    | 'Handshaking'
    | 'Synchronized'
    | 'Incompatible'
    | 'Destroyed';

type FakeDocumentState = 'LocalReady' | 'AwaitRemote' | 'RoomReady';

interface FakeErrorRecord {
    domain: string;
    code: string;
    message: string;
    requestId: string | null;
    operationIndex: string | null;
    limit: string | null;
    actual: string | null;
    details: Record<string, unknown> | null;
}

function okRecord(value: unknown): Record<string, unknown> {
    return { value, error: null };
}

function errorRecord(domain: string, code: string, message: string): FakeErrorRecord {
    return {
        domain,
        code,
        message,
        requestId: null,
        operationIndex: null,
        limit: null,
        actual: null,
        details: null,
    };
}

function errRecord(error: FakeErrorRecord): Record<string, unknown> {
    return { value: null, error };
}

function transportError(code: string, message: string): Record<string, unknown> {
    return errRecord(errorRecord('transport', code, message));
}

function lifecycleError(code: string, message: string): Record<string, unknown> {
    return errRecord(errorRecord('lifecycle', code, message));
}

function operationError(code: string, message: string): Record<string, unknown> {
    return errRecord(errorRecord('operation', code, message));
}

function snapshotError(code: string, message: string): Record<string, unknown> {
    return errRecord(errorRecord('snapshot', code, message));
}

function boundaryError(code: string, message: string): Record<string, unknown> {
    return errRecord(errorRecord('boundary', code, message));
}

const CANONICAL_V2_U64 = /^(0|[1-9]\d*)$/;
const U64_MAX = 0xffff_ffff_ffff_ffffn;

/** Match the production v2 boundary: u64s are canonical decimal strings only. */
function canonicalV2U64(value: unknown): string | null {
    if (typeof value !== 'string' || !CANONICAL_V2_U64.test(value)) return null;
    try {
        return BigInt(value) <= U64_MAX ? value : null;
    } catch {
        return null;
    }
}

/** Match platform/JS exact-u32 admission before a native integer conversion. */
function exactV2U32(value: unknown): number | null {
    return typeof value === 'number' &&
        Number.isFinite(value) &&
        Number.isInteger(value) &&
        value >= 0 &&
        value <= 0xffff_ffff
        ? value
        : null;
}

function parseV2RequestEnvelope(
    requestJson: string,
    requiresBaseRevision: boolean
): Record<string, unknown> | Record<string, unknown> {
    let request: unknown;
    try {
        request = JSON.parse(requestJson);
    } catch {
        return { __v2RequestError: boundaryError('CONFIG_INVALID', 'malformed v2 request envelope') };
    }
    if (
        request == null ||
        typeof request !== 'object' ||
        Array.isArray(request) ||
        (request as Record<string, unknown>).version !== 1 ||
        canonicalV2U64((request as Record<string, unknown>).requestId) == null ||
        (requiresBaseRevision &&
            canonicalV2U64((request as Record<string, unknown>).baseDocumentRevision) == null)
    ) {
        return { __v2RequestError: boundaryError('CONFIG_INVALID', 'invalid v2 request envelope') };
    }
    return request as Record<string, unknown>;
}

function requestEnvelopeError(
    parsed: Record<string, unknown>
): Record<string, unknown> | null {
    const error = parsed.__v2RequestError;
    return error != null && typeof error === 'object' ? (error as Record<string, unknown>) : null;
}

/** Deterministic single-paragraph HTML used by the fake for html round-trips. */
export function fakeHtmlForDoc(doc: DocumentJSON): string {
    const content = Array.isArray(doc.content) ? doc.content : [];
    return content
        .map((block) => {
            const inline = Array.isArray(block?.content) ? block.content : [];
            const text = inline
                .map((node) => (typeof node?.text === 'string' ? node.text : ''))
                .join('');
            return `<p>${text}</p>`;
        })
        .join('');
}

export function fakeDocForHtml(html: string): DocumentJSON {
    const paragraphs: Record<string, unknown>[] = [];
    const pattern = /<p>([\s\S]*?)<\/p>/g;
    let match = pattern.exec(html);
    while (match) {
        const text = match[1].replace(/<[^>]+>/g, '');
        paragraphs.push(
            text.length > 0
                ? { type: 'paragraph', content: [{ type: 'text', text }] }
                : { type: 'paragraph' }
        );
        match = pattern.exec(html);
    }
    if (paragraphs.length === 0) {
        const text = html.replace(/<[^>]+>/g, '');
        paragraphs.push(
            text.length > 0
                ? { type: 'paragraph', content: [{ type: 'text', text }] }
                : { type: 'paragraph' }
        );
    }
    return { type: 'doc', content: paragraphs } as DocumentJSON;
}

export function fakeDocForText(text: string): DocumentJSON {
    return {
        type: 'doc',
        content: [{ type: 'paragraph', content: [{ type: 'text', text }] }],
    } as DocumentJSON;
}

function cloneDoc(doc: DocumentJSON): DocumentJSON {
    return JSON.parse(JSON.stringify(doc)) as DocumentJSON;
}

function appendText(doc: DocumentJSON, text: string): DocumentJSON {
    const next = cloneDoc(doc);
    const content = Array.isArray(next.content) ? next.content : [];
    if (content.length === 0) {
        content.push({ type: 'paragraph', content: [{ type: 'text', text }] });
        return next;
    }
    const last = content[content.length - 1] as Record<string, unknown>;
    const inline = Array.isArray(last.content) ? (last.content as Record<string, unknown>[]) : [];
    const lastText = inline.length > 0 ? inline[inline.length - 1] : null;
    if (lastText && typeof lastText.text === 'string') {
        lastText.text = `${lastText.text}${text}`;
    } else {
        inline.push({ type: 'text', text });
        last.content = inline;
    }
    return next;
}

/** Outbound document frames carry their revision so tests can assert order. */
function documentFrame(revision: number): Uint8Array {
    return new Uint8Array([0x64, revision & 0xff]);
}

function protocolReplyFrame(sequence: number): Uint8Array {
    return new Uint8Array([0x70, sequence & 0xff]);
}

function awarenessFrame(clock: number): Uint8Array {
    return new Uint8Array([0x61, clock & 0xff]);
}

interface FakeSession {
    editorId: string;
    roomBound: boolean;
    documentId: string | null;
    lineageId: string | null;
    documentState: FakeDocumentState;
    transportState: FakeTransportState;
    renderState: 'Loading' | 'Ready';
    documentRevision: number;
    stateRevision: number;
    doc: DocumentJSON;
    undoStack: DocumentJSON[];
    redoStack: DocumentJSON[];
    activeMarks: Record<string, boolean>;
    activeMarkAttrs: Record<string, Record<string, unknown>>;
    activeNodes: Record<string, boolean>;
    liveGeneration: number | null;
    lastIssuedGeneration: number;
    protocolQueue: Uint8Array[];
    documentQueue: Uint8Array[];
    desiredAwareness: Record<string, unknown> | null;
    localClientId: string;
    localClock: number;
    remotePeers: NativeEditorV2PeerInfo[];
    destroyed: boolean;
    replySequence: number;
}

export interface FakeV2SessionHandle {
    editorId: string;
}

export interface FakeNativeEditorV2Runtime {
    /** The editorV2* entries, already jest.fn-wrapped for call assertions. */
    module: Record<string, jest.Mock>;
    /** Create a room session directly (mirrors what the TS bridge create sends). */
    sessions(): readonly FakeSession[];
    session(editorId: string): FakeSession;
    /** Ids the module marked live for view binding at create (view-binding surface). */
    liveEditorIds(): string[];
    /** Queue the document the next accepted server Step 2 / update installs. */
    pushRemoteDoc(editorId: string, doc: DocumentJSON): void;
    /** Queue the peer set the next inbound awareness frame installs. */
    pushRemotePeers(editorId: string, peers: NativeEditorV2PeerInfo[]): void;
    /** Retire the live generation natively without telling TypeScript. */
    retireLiveGeneration(editorId: string): void;
    /** One-shot error injected into the named entry (currently applyLocalApi). */
    injectNextApplyLocalApiError(editorId: string, error: FakeErrorRecord): void;
    /** One-shot error injected into applyCommand. */
    injectNextApplyCommandError(editorId: string, error: FakeErrorRecord): void;
    /** Frames the session has queued, oldest first (protocol then document). */
    queuedFrames(editorId: string): Uint8Array[];
}

interface PendingRemote {
    docs: DocumentJSON[];
    peerSets: NativeEditorV2PeerInfo[][];
    applyLocalApiErrors: FakeErrorRecord[];
    applyCommandErrors: FakeErrorRecord[];
}

export function createFakeNativeEditorV2Runtime(): FakeNativeEditorV2Runtime {
    const sessions = new Map<string, FakeSession>();
    const pending = new Map<string, PendingRemote>();
    const liveIds = new Set<string>();
    let editorIdCounter = 0;
    let clientIdCounter = 1000;

    function pendingFor(editorId: string): PendingRemote {
        let entry = pending.get(editorId);
        if (!entry) {
            entry = { docs: [], peerSets: [], applyLocalApiErrors: [], applyCommandErrors: [] };
            pending.set(editorId, entry);
        }
        return entry;
    }

    function getSession(editorId: string): FakeSession | null {
        return sessions.get(editorId) ?? null;
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
        if (
            session.liveGeneration == null ||
            canonicalV2U64(generation) == null ||
            generation !== String(session.liveGeneration)
        ) {
            return transportError(
                'TRANSPORT_STALE_GENERATION',
                `${action} rejected: stale transport generation`
            );
        }
        return null;
    }

    function retireGeneration(session: FakeSession, next: FakeTransportState): void {
        session.liveGeneration = null;
        session.transportState = next;
        session.remotePeers = [];
    }

    function queueDocumentUpdate(session: FakeSession): void {
        if (!session.roomBound) return;
        session.documentQueue.push(documentFrame(session.documentRevision));
    }

    function applyReplacement(
        session: FakeSession,
        nextDoc: DocumentJSON,
        history: string
    ): void {
        if (history === 'undoableBoundary') {
            session.undoStack.push(cloneDoc(session.doc));
            session.redoStack = [];
        } else {
            session.undoStack = [];
            session.redoStack = [];
        }
        session.doc = cloneDoc(nextDoc);
        session.documentRevision += 1;
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
            let documentPromoted = false;
            if (session.documentState === 'AwaitRemote') {
                const remote = pendingFor(session.editorId);
                session.doc = cloneDoc(remote.docs.shift() ?? EMPTY_DOC);
                session.documentState = 'RoomReady';
                session.renderState = 'Ready';
                session.documentRevision += 1;
                documentPromoted = true;
            }
            session.transportState = 'Synchronized';
            if (session.desiredAwareness != null) {
                session.localClock += 1;
                session.protocolQueue.push(awarenessFrame(session.localClock));
            }
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
            session.doc = cloneDoc(nextDoc);
            session.documentRevision += 1;
            return outcome({ remoteCommitApplied: true });
        }
        // Remote awareness state.
        if (tag === 0x02) {
            const remote = pendingFor(session.editorId);
            session.remotePeers = remote.peerSets.shift() ?? [];
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

    const module: Record<string, jest.Mock> = {
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
            editorIdCounter += 1;
            const editorId = String(editorIdCounter);
            const base: FakeSession = {
                editorId,
                roomBound: false,
                documentId: null,
                lineageId: null,
                documentState: 'LocalReady',
                transportState: 'Detached',
                renderState: 'Ready',
                documentRevision: 1,
                stateRevision: 1,
                doc: cloneDoc(EMPTY_DOC),
                undoStack: [],
                redoStack: [],
                activeMarks: {},
                activeMarkAttrs: {},
                activeNodes: {},
                liveGeneration: null,
                lastIssuedGeneration: 0,
                protocolQueue: [],
                documentQueue: [],
                desiredAwareness: null,
                localClientId: String((clientIdCounter += 1)),
                localClock: 0,
                remotePeers: [],
                destroyed: false,
                replySequence: 0,
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
                        const parsed = JSON.parse(
                            new TextDecoder().decode(snapshotState)
                        ) as { doc: DocumentJSON; revision?: number };
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
                liveIds.delete(editorId);
                return okRecord(true);
            })
        ),
        editorV2GetState: jest.fn((editorId: string) =>
            withSession(editorId, (session) => okRecord(stateJson(session)))
        ),
        editorV2GetDocumentJson: jest.fn((editorId: string) =>
            withSession(editorId, (session) => okRecord(JSON.stringify(session.doc)))
        ),
        editorV2GetDocumentHtml: jest.fn((editorId: string) =>
            withSession(editorId, (session) =>
                okRecord(JSON.stringify({ html: fakeHtmlForDoc(session.doc) }))
            )
        ),
        editorV2GetContentSnapshot: jest.fn((editorId: string) =>
            withSession(editorId, (session) =>
                okRecord(
                    JSON.stringify({ html: fakeHtmlForDoc(session.doc), json: session.doc })
                )
            )
        ),
        editorV2ReplaceDocument: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const request = parseV2RequestEnvelope(requestJson, false);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                const rejected = admitReplacement(session);
                if (rejected) return rejected;
                const nextDoc =
                    request.setJson != null
                        ? (request.setJson as DocumentJSON)
                        : fakeDocForHtml(String(request.setHtml ?? ''));
                applyReplacement(session, nextDoc, String(request.history));
                return okRecord(
                    JSON.stringify({
                        changed: true,
                        documentRevision: String(session.documentRevision),
                    })
                );
            })
        ),
        editorV2ApplyInput: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const request = parseV2RequestEnvelope(requestJson, true);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                if (session.documentState === 'AwaitRemote') {
                    return operationError(
                        'ENGINE_NOT_READY',
                        'room document is awaiting the remote initial state'
                    );
                }
                if (request.baseDocumentRevision !== String(session.documentRevision)) {
                    return operationError(
                        'REVISION_MISMATCH',
                        'base document revision does not match the engine revision'
                    );
                }
                session.undoStack.push(cloneDoc(session.doc));
                session.redoStack = [];
                session.doc = appendText(session.doc, String(request.text ?? ''));
                session.documentRevision += 1;
                session.stateRevision += 1;
                queueDocumentUpdate(session);
                return okRecord(
                    JSON.stringify({
                        type: 'transaction',
                        changed: true,
                        documentRevision: String(session.documentRevision),
                        stateRevision: String(session.stateRevision),
                        canUndo: session.undoStack.length > 0,
                        canRedo: session.redoStack.length > 0,
                    })
                );
            })
        ),
        editorV2ApplyCommand: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const injected = pendingFor(session.editorId).applyCommandErrors.shift();
                if (injected) return errRecord(injected);
                const request = parseV2RequestEnvelope(requestJson, true);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                if (session.documentState === 'AwaitRemote') {
                    return operationError(
                        'ENGINE_NOT_READY',
                        'room document is awaiting the remote initial state'
                    );
                }
                if (request.baseDocumentRevision !== String(session.documentRevision)) {
                    return operationError(
                        'REVISION_MISMATCH',
                        'base document revision does not match the engine revision'
                    );
                }
                const command = (request.command ?? {}) as Record<string, unknown>;
                const type = String(command.type ?? '');
                const stateOnlyOutcome = () => {
                    session.stateRevision += 1;
                    return okRecord(
                        JSON.stringify({
                            type: 'transaction',
                            changed: false,
                            documentRevision: String(session.documentRevision),
                            stateRevision: String(session.stateRevision),
                            canUndo: session.undoStack.length > 0,
                            canRedo: session.redoStack.length > 0,
                        })
                    );
                };
                const docChangeOutcome = (apply: () => void) => {
                    session.undoStack.push(cloneDoc(session.doc));
                    session.redoStack = [];
                    apply();
                    session.documentRevision += 1;
                    session.stateRevision += 1;
                    queueDocumentUpdate(session);
                    return okRecord(
                        JSON.stringify({
                            type: 'transaction',
                            changed: true,
                            documentRevision: String(session.documentRevision),
                            stateRevision: String(session.stateRevision),
                            canUndo: session.undoStack.length > 0,
                            canRedo: session.redoStack.length > 0,
                        })
                    );
                };
                const appendBlocks = (blocks: unknown[]) => {
                    const next = cloneDoc(session.doc);
                    const content = Array.isArray(next.content) ? next.content : [];
                    content.push(...(blocks as Record<string, unknown>[]));
                    next.content = content;
                    session.doc = next;
                };
                switch (type) {
                    case 'toggleMark': {
                        const markType = String(command.markType ?? '');
                        if (session.activeMarks[markType]) {
                            session.activeMarks[markType] = false;
                            delete session.activeMarkAttrs[markType];
                        } else {
                            session.activeMarks[markType] = true;
                        }
                        return stateOnlyOutcome();
                    }
                    case 'setMark': {
                        const markType = String(command.markType ?? '');
                        session.activeMarks[markType] = true;
                        session.activeMarkAttrs[markType] =
                            (command.attrs as Record<string, unknown>) ?? {};
                        return stateOnlyOutcome();
                    }
                    case 'unsetMark': {
                        const markType = String(command.markType ?? '');
                        session.activeMarks[markType] = false;
                        delete session.activeMarkAttrs[markType];
                        return stateOnlyOutcome();
                    }
                    case 'toggleHeading': {
                        const level = String(command.level ?? '');
                        const key = `heading:${level}`;
                        if (session.activeNodes[key]) {
                            delete session.activeNodes[key];
                        } else {
                            session.activeNodes[key] = true;
                        }
                        return stateOnlyOutcome();
                    }
                    case 'toggleBlockquote': {
                        if (session.activeNodes.blockquote) {
                            delete session.activeNodes.blockquote;
                        } else {
                            session.activeNodes.blockquote = true;
                        }
                        return stateOnlyOutcome();
                    }
                    case 'wrapInList': {
                        session.activeNodes[String(command.listType ?? '')] = true;
                        return stateOnlyOutcome();
                    }
                    case 'unwrapFromList': {
                        session.activeNodes = {};
                        return stateOnlyOutcome();
                    }
                    case 'indentListItem':
                    case 'outdentListItem':
                        return stateOnlyOutcome();
                    case 'insertNode':
                        return docChangeOutcome(() =>
                            appendBlocks([
                                {
                                    type: 'paragraph',
                                    content: [
                                        { type: 'text', text: `[${String(command.nodeType ?? '')}]` },
                                    ],
                                },
                            ])
                        );
                    case 'insertContentHtml':
                        return docChangeOutcome(() => {
                            const fragment = fakeDocForHtml(String(command.html ?? ''));
                            appendBlocks(
                                Array.isArray(fragment.content) ? fragment.content : []
                            );
                        });
                    case 'insertContentJson': {
                        const fragment = (command.json ?? {}) as DocumentJSON;
                        return docChangeOutcome(() =>
                            appendBlocks(Array.isArray(fragment.content) ? fragment.content : [])
                        );
                    }
                    case 'replaceSelectionText':
                        return docChangeOutcome(() => {
                            session.doc = appendText(session.doc, String(command.text ?? ''));
                        });
                    default:
                        return okRecord(JSON.stringify({ type: 'notApplicable' }));
                }
            })
        ),
        editorV2RenderUpdate: jest.fn(
            (editorId: string, mirrorAnchor: unknown, mirrorHead: unknown) =>
                withSession(editorId, (session) => {
                    if (
                        (mirrorAnchor == null) !== (mirrorHead == null) ||
                        (mirrorAnchor != null && exactV2U32(mirrorAnchor) == null) ||
                        (mirrorHead != null && exactV2U32(mirrorHead) == null)
                    ) {
                        return boundaryError('CONFIG_INVALID', 'invalid render mirror scalar offsets');
                    }
                    if (session.documentState === 'AwaitRemote') {
                        return operationError(
                            'ENGINE_NOT_READY',
                            'room document is awaiting the remote initial state'
                        );
                    }
                    const blocks = (Array.isArray(session.doc.content) ? session.doc.content : []).map(
                        (block) => {
                            const inline = Array.isArray(block?.content) ? block.content : [];
                            const text = inline
                                .map((node) => (typeof node?.text === 'string' ? node.text : ''))
                                .join('');
                            const nodeType = String(block?.type ?? 'paragraph');
                            return [
                                { type: 'blockStart', nodeType, depth: 0 },
                                ...(text.length > 0
                                    ? [{ type: 'textRun', text, marks: [] as string[] }]
                                    : []),
                                { type: 'blockEnd', nodeType },
                            ];
                        }
                    );
                    const scalarLength = fakeHtmlForDoc(session.doc).replace(/<[^>]+>/g, '').length;
                    const update: Record<string, unknown> = {
                        renderBlocks: blocks,
                        renderPatch: null,
                        activeState: {
                            marks: { ...session.activeMarks },
                            markAttrs: { ...session.activeMarkAttrs },
                            nodes: { ...session.activeNodes },
                            commands: {},
                            allowedMarks: ['bold', 'italic', 'underline', 'strike', 'link'],
                            insertableNodes: ['image', 'horizontalRule', 'hardBreak'],
                        },
                        historyState: {
                            canUndo: session.undoStack.length > 0,
                            canRedo: session.redoStack.length > 0,
                        },
                        documentVersion: String(session.documentRevision),
                        scalarLength,
                    };
                    if (mirrorAnchor != null && mirrorHead != null) {
                        const anchor = exactV2U32(mirrorAnchor)!;
                        const head = exactV2U32(mirrorHead)!;
                        update.selection = {
                            type: 'text',
                            anchor,
                            head,
                            anchorScalar: anchor,
                            headScalar: head,
                        };
                    }
                    return okRecord(JSON.stringify(update));
                })
        ),
        editorV2ApplyLocalApi: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const injected = pendingFor(session.editorId).applyLocalApiErrors.shift();
                if (injected) return errRecord(injected);
                const request = parseV2RequestEnvelope(requestJson, true);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                if (request.baseDocumentRevision !== String(session.documentRevision)) {
                    return operationError(
                        'REVISION_MISMATCH',
                        'base document revision does not match the engine revision'
                    );
                }
                const rejected = admitReplacement(session);
                if (rejected) return rejected;
                const nextDoc =
                    request.setJson != null
                        ? (request.setJson as DocumentJSON)
                        : fakeDocForHtml(String(request.setHtml ?? ''));
                applyReplacement(session, nextDoc, String(request.history));
                return okRecord(
                    JSON.stringify({
                        type: 'replacement',
                        changed: true,
                        documentRevision: String(session.documentRevision),
                    })
                );
            })
        ),
        editorV2SetSelection: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const request = parseV2RequestEnvelope(requestJson, true);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                if (request.baseDocumentRevision !== String(session.documentRevision)) {
                    return operationError(
                        'REVISION_MISMATCH',
                        'base document revision does not match the engine revision'
                    );
                }
                return okRecord(JSON.stringify({ type: 'notApplicable' }));
            })
        ),
        editorV2Undo: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const request = parseV2RequestEnvelope(requestJson, false);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                const previous = session.undoStack.pop();
                if (!previous) return okRecord(JSON.stringify({ changed: false }));
                session.redoStack.push(cloneDoc(session.doc));
                session.doc = previous;
                session.documentRevision += 1;
                queueDocumentUpdate(session);
                return okRecord(JSON.stringify({ changed: true }));
            })
        ),
        editorV2Redo: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const request = parseV2RequestEnvelope(requestJson, false);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                const next = session.redoStack.pop();
                if (!next) return okRecord(JSON.stringify({ changed: false }));
                session.undoStack.push(cloneDoc(session.doc));
                session.doc = next;
                session.documentRevision += 1;
                queueDocumentUpdate(session);
                return okRecord(JSON.stringify({ changed: true }));
            })
        ),
        editorV2CollaborationBeginConnect: jest.fn((editorId: string) =>
            withSession(editorId, (session) => {
                if (!session.roomBound) {
                    // Mirrors not_room_bound() in collaboration_runtime/state.rs:
                    // ErrorDomain::Transport + TRANSPORT_NOT_ROOM_BOUND.
                    return transportError(
                        'TRANSPORT_NOT_ROOM_BOUND',
                        'local-only sessions have no room binding to connect to'
                    );
                }
                if (session.transportState === 'Incompatible') {
                    return transportError(
                        'TRANSPORT_INCOMPATIBLE',
                        'transport is parked incompatible until detach/reattach'
                    );
                }
                if (session.transportState !== 'Disconnected') {
                    return transportError(
                        'TRANSPORT_INVALID_TRANSITION',
                        `begin_connect is only admitted from Disconnected (found ${session.transportState})`
                    );
                }
                session.lastIssuedGeneration += 1;
                session.liveGeneration = session.lastIssuedGeneration;
                session.transportState = 'Connecting';
                return okRecord(JSON.stringify({ generation: String(session.liveGeneration) }));
            })
        ),
        editorV2CollaborationSocketOpen: jest.fn((editorId: string, generation: string) =>
            withSession(editorId, (session) => {
                const stale = requireLiveGeneration(session, generation, 'socketOpen');
                if (stale) return stale;
                if (session.transportState !== 'Connecting') {
                    return transportError(
                        'TRANSPORT_INVALID_TRANSITION',
                        'socket_open is only admitted from Connecting'
                    );
                }
                session.transportState = 'Handshaking';
                return okRecord(new Uint8Array(V2_FAKE_STEP1_FRAME));
            })
        ),
        editorV2CollaborationReceive: jest.fn(
            (editorId: string, generation: string, message: Uint8Array) =>
                withSession(editorId, (session) => {
                    const stale = requireLiveGeneration(session, generation, 'receive');
                    if (stale) return stale;
                    if (
                        session.transportState !== 'Handshaking' &&
                        session.transportState !== 'Synchronized'
                    ) {
                        return transportError(
                            'TRANSPORT_INVALID_TRANSITION',
                            'receive is only admitted from Handshaking/Synchronized'
                        );
                    }
                    return handleReceive(session, message);
                })
        ),
        editorV2CollaborationSocketClose: jest.fn(
            (editorId: string, generation: string, code: unknown, _reason: string | null) =>
                withSession(editorId, (session) => {
                    const stale = requireLiveGeneration(session, generation, 'socketClose');
                    if (stale) return stale;
                    if (code != null && exactV2U32(code) == null) {
                        return boundaryError('CONFIG_INVALID', 'invalid collaboration close code');
                    }
                    const next: FakeTransportState =
                        code === 1008 ? 'Incompatible' : 'Disconnected';
                    retireGeneration(session, next);
                    return okRecord(JSON.stringify({ transportState: session.transportState }));
                })
        ),
        editorV2CollaborationTakeOutbound: jest.fn((editorId: string, generation: string) =>
            withSession(editorId, (session) => {
                const stale = requireLiveGeneration(session, generation, 'takeOutbound');
                if (stale) return stale;
                if (
                    session.transportState !== 'Handshaking' &&
                    session.transportState !== 'Synchronized'
                ) {
                    return transportError(
                        'TRANSPORT_INVALID_TRANSITION',
                        'take_outbound is only admitted from Handshaking/Synchronized'
                    );
                }
                const frame = session.protocolQueue.shift() ?? session.documentQueue.shift();
                return okRecord(frame ? new Uint8Array(frame) : new Uint8Array());
            })
        ),
        editorV2CollaborationSetAwareness: jest.fn((editorId: string, awarenessJson: string) =>
            withSession(editorId, (session) => {
                if (!session.roomBound) {
                    return boundaryError(
                        'CONFIG_INVALID',
                        'local sessions have no attached collaboration runtime'
                    );
                }
                if (awarenessJson.trim() === 'null') {
                    session.desiredAwareness = null;
                } else {
                    session.desiredAwareness = JSON.parse(awarenessJson) as Record<
                        string,
                        unknown
                    >;
                }
                if (
                    session.liveGeneration != null &&
                    (session.transportState === 'Handshaking' ||
                        session.transportState === 'Synchronized')
                ) {
                    session.localClock += 1;
                    session.protocolQueue.push(awarenessFrame(session.localClock));
                }
                return okRecord(true);
            })
        ),
        editorV2CollaborationPeers: jest.fn((editorId: string) =>
            withSession(editorId, (session) => {
                const peers: NativeEditorV2PeerInfo[] = [];
                if (session.desiredAwareness != null) {
                    peers.push({
                        clientId: session.localClientId,
                        clock: session.localClock,
                        isLocal: true,
                        state: session.desiredAwareness,
                        cursor: null,
                    });
                }
                peers.push(...session.remotePeers);
                return okRecord(JSON.stringify({ peers }));
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
                    session.doc = cloneDoc(parsed.doc);
                    session.documentRevision =
                        typeof parsed.revision === 'number' ? parsed.revision : 1;
                    session.documentState = 'RoomReady';
                    session.renderState = 'Ready';
                    session.transportState = 'Disconnected';
                    session.liveGeneration = null;
                    session.protocolQueue = [];
                    session.remotePeers = [];
                    session.undoStack = [];
                    session.redoStack = [];
                    session.localClientId = String((clientIdCounter += 1));
                    return okRecord(
                        JSON.stringify({
                            changed: true,
                            documentRevision: String(session.documentRevision),
                        })
                    );
                })
        ),
    };

    return {
        module,
        sessions: () => [...sessions.values()],
        session: (editorId: string) => {
            const session = getSession(editorId);
            if (!session) throw new Error(`unknown fake session ${editorId}`);
            return session;
        },
        liveEditorIds: () => [...liveIds],
        pushRemoteDoc: (editorId, doc) => {
            pendingFor(editorId).docs.push(cloneDoc(doc));
        },
        pushRemotePeers: (editorId, peers) => {
            pendingFor(editorId).peerSets.push(peers);
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
        queuedFrames: (editorId) => {
            const session = getSession(editorId);
            if (!session) return [];
            return [...session.protocolQueue, ...session.documentQueue];
        },
    };
}
