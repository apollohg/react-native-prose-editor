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
// - explicit detach/reattach is the path out of Incompatible;
// - a real undo stack so replace/reset history policy is verifiable through
//   engine undo state instead of document content.
//
// The fake owns awareness clocks (the TypeScript side must never see a clock
// field): publishing desired awareness while a generation is live enqueues an
// awareness frame carrying a Rust-side monotonically increasing clock.

import type { DocumentJSON, NativeEditorV2PeerInfo } from '../NativeEditorBridge';
import { normalizeNativeEditorV2U64 } from '../../NativeEditorV2Decimal';

export const V2_FAKE_STEP1_FRAME = new Uint8Array([0, 0, 1]);
export const V2_FAKE_STEP2_FRAME = new Uint8Array([0, 0, 2]);
export const V2_FAKE_STEP2_INVALID_FRAGMENT_FRAME = new Uint8Array([0, 0, 5]);
export const V2_FAKE_UPDATE_FRAME = new Uint8Array([0, 1, 1]);
export const V2_FAKE_AWARENESS_FRAME = new Uint8Array([0, 2, 1]);
export const V2_FAKE_MALFORMED_FRAME = new Uint8Array([0xff]);
export const V2_FAKE_INCOMPATIBLE_FRAME = new Uint8Array([0xfe]);

const V2_FAKE_U64_MAX = 18_446_744_073_709_551_615n;
const V2_FAKE_MAX_ADMITTED_REMOTE_AWARENESS_CLOCK = 0xffff_fffe;
const V2_FAKE_AWARENESS_RENEWAL_INTERVAL_MILLIS = 15_000n;
const V2_FAKE_AWARENESS_EXPIRY_MILLIS = 30_000n;

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

function transportError(
    code: string,
    message: string,
    details: Record<string, unknown> | null = null
): Record<string, unknown> {
    const error = errorRecord('transport', code, message);
    error.details = details;
    return errRecord(error);
}

function lifecycleError(code: string, message: string): Record<string, unknown> {
    return errRecord(errorRecord('lifecycle', code, message));
}

function operationError(
    code: string,
    message: string,
    details: Record<string, unknown> | null = null
): Record<string, unknown> {
    const error = errorRecord('operation', code, message);
    error.details = details;
    return errRecord(error);
}

function snapshotError(code: string, message: string): Record<string, unknown> {
    return errRecord(errorRecord('snapshot', code, message));
}

function boundaryError(code: string, message: string): Record<string, unknown> {
    return errRecord(errorRecord('boundary', code, message));
}

/** Match the production v2 boundary: u64s are canonical decimal strings only. */
function canonicalV2U64(value: unknown): string | null {
    return normalizeNativeEditorV2U64(value);
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

type FakeDocumentNode = Record<string, unknown>;

interface FakeInlineSpan {
    scalarStart: number;
    scalarEnd: number;
    documentStart: number;
    documentEnd: number;
    kind: 'text' | 'atom';
    marks: FakeDocumentNode[];
}

interface FakePositionBlock {
    scalarStart: number;
    scalarLength: number;
    documentStart: number;
    documentEnd: number;
    isVoid: boolean;
    isPlaceholder: boolean;
    inlineSpans: FakeInlineSpan[];
    ancestors: FakeDocumentNode[];
}

interface FakeScalarDocumentMap {
    scalarLength: number;
    clampDocumentOffset(offset: number): number;
    scalarToDocument(offset: number): number;
    documentToScalar(offset: number): number;
    activeStateAt(offset: number): {
        marks: Record<string, boolean>;
        markAttrs: Record<string, Record<string, unknown>>;
        nodes: Record<string, boolean>;
    };
}

/** The native snapshot measures text in Unicode scalar values, never UTF-16 code units. */
function unicodeScalarLength(text: string): number {
    return Array.from(text).length;
}

function isFakeVoidNode(node: FakeDocumentNode): boolean {
    return (
        node.atom === true ||
        node.type === 'hardBreak' ||
        node.type === 'hard_break' ||
        node.type === 'mention' ||
        node.type === 'image' ||
        node.type === 'horizontalRule' ||
        node.type === 'horizontal_rule'
    );
}

function isFakeBlockVoidNode(node: FakeDocumentNode): boolean {
    return (
        node.atom === true ||
        node.type === 'image' ||
        node.type === 'horizontalRule' ||
        node.type === 'horizontal_rule'
    );
}

function fakeAtomLabel(node: FakeDocumentNode): string {
    const type = typeof node.type === 'string' ? node.type : '';
    const attrs =
        node.attrs != null && typeof node.attrs === 'object' && !Array.isArray(node.attrs)
            ? (node.attrs as Record<string, unknown>)
            : {};
    let label = typeof attrs.label === 'string' && attrs.label.length > 0 ? attrs.label : type;
    const trigger = typeof attrs.mentionSuggestionChar === 'string' ? attrs.mentionSuggestionChar : '';
    if (type === 'mention' && trigger.length > 0 && !label.startsWith(trigger)) {
        label = `${trigger}${label}`;
    }
    return type === 'mention' ? label : `[${label}]`;
}

function fakeInlineAtomScalarLength(node: FakeDocumentNode): number {
    return node.type === 'hardBreak' || node.type === 'hard_break'
        ? 1
        : unicodeScalarLength(fakeAtomLabel(node));
}

function fakeBlockAtomScalarLength(node: FakeDocumentNode): number {
    return node.type === 'image' ||
        node.type === 'horizontalRule' ||
        node.type === 'horizontal_rule'
        ? 1
        : unicodeScalarLength(fakeAtomLabel(node));
}

function fakeDocumentNodeSize(node: FakeDocumentNode): number {
    if (typeof node.text === 'string') return unicodeScalarLength(node.text);
    if (isFakeVoidNode(node)) return 1;
    const content = Array.isArray(node.content) ? node.content : [];
    return (
        2 +
        content.reduce(
            (total, child) =>
                child != null && typeof child === 'object' && !Array.isArray(child)
                    ? total + fakeDocumentNodeSize(child as FakeDocumentNode)
                    : total,
            0
        )
    );
}

/**
 * Model the production PositionMap for the fake's supported top-level block schema.
 * Text blocks expose an empty-block placeholder; block boundaries contribute one
 * rendered separator scalar; and opaque atoms keep their one-token document extent
 * while exposing their rendered scalar width.
 */
function fakeScalarDocumentMap(doc: DocumentJSON): FakeScalarDocumentMap {
    const blocks: FakePositionBlock[] = [];
    const content = Array.isArray(doc.content) ? doc.content : [];
    let documentLength = 0;

    for (const rawBlock of content) {
        if (rawBlock == null || typeof rawBlock !== 'object' || Array.isArray(rawBlock)) continue;
        const block = rawBlock as FakeDocumentNode;
        if (isFakeBlockVoidNode(block)) {
            blocks.push({
                scalarStart: 0,
                scalarLength: fakeBlockAtomScalarLength(block),
                documentStart: documentLength,
                documentEnd: documentLength,
                isVoid: true,
                isPlaceholder: false,
                inlineSpans: [],
                ancestors: [block],
            });
            documentLength += fakeDocumentNodeSize(block);
            continue;
        }

        const inline = Array.isArray(block.content) ? block.content : [];
        const documentStart = documentLength + 1;
        let inlineDocumentOffset = documentStart;
        let inlineScalarOffset = 0;
        const inlineSpans: FakeInlineSpan[] = [];
        for (const rawInline of inline) {
            if (
                rawInline == null ||
                typeof rawInline !== 'object' ||
                Array.isArray(rawInline)
            ) {
                continue;
            }
            const inlineNode = rawInline as FakeDocumentNode;
            if (typeof inlineNode.text === 'string') {
                const length = unicodeScalarLength(inlineNode.text);
                inlineSpans.push({
                    scalarStart: inlineScalarOffset,
                    scalarEnd: inlineScalarOffset + length,
                    documentStart: inlineDocumentOffset,
                    documentEnd: inlineDocumentOffset + length,
                    kind: 'text',
                    marks: Array.isArray(inlineNode.marks)
                        ? inlineNode.marks.filter(
                              (mark): mark is FakeDocumentNode =>
                                  mark != null && typeof mark === 'object' && !Array.isArray(mark)
                          )
                        : [],
                });
                inlineScalarOffset += length;
                inlineDocumentOffset += length;
            } else if (isFakeVoidNode(inlineNode)) {
                const length = fakeInlineAtomScalarLength(inlineNode);
                inlineSpans.push({
                    scalarStart: inlineScalarOffset,
                    scalarEnd: inlineScalarOffset + length,
                    documentStart: inlineDocumentOffset,
                    documentEnd: inlineDocumentOffset + 1,
                    kind: 'atom',
                    marks: [],
                });
                inlineScalarOffset += length;
                inlineDocumentOffset += 1;
            } else {
                inlineDocumentOffset += fakeDocumentNodeSize(inlineNode);
            }
        }
        const isPlaceholder = inline.length === 0;
        blocks.push({
            scalarStart: 0,
            scalarLength: isPlaceholder ? 1 : inlineScalarOffset,
            documentStart,
            documentEnd: inlineDocumentOffset,
            isVoid: false,
            isPlaceholder,
            inlineSpans,
            ancestors: [block],
        });
        documentLength += fakeDocumentNodeSize(block);
    }

    let scalarLength = 0;
    for (const [index, block] of blocks.entries()) {
        block.scalarStart = scalarLength;
        scalarLength += block.scalarLength + (index + 1 < blocks.length ? 1 : 0);
    }

    const clampScalar = (offset: number) => Math.min(Math.max(offset, 0), scalarLength);
    const clampDocumentOffset = (offset: number) =>
        Math.min(Math.max(offset, 0), documentLength);
    const blockForDocumentOffset = (offset: number): FakePositionBlock | undefined => {
        let previous: FakePositionBlock | undefined;
        for (const block of blocks) {
            if (block.isVoid) {
                if (offset === block.documentStart) return block;
                if (offset < block.documentStart) {
                    if (!previous) return block;
                    return offset - previous.documentEnd <= block.documentStart - offset
                        ? previous
                        : block;
                }
                previous = block;
                continue;
            }
            if (offset >= block.documentStart && offset <= block.documentEnd) return block;
            if (offset < block.documentStart) {
                if (!previous) return block;
                return offset - previous.documentEnd <= block.documentStart - offset
                    ? previous
                    : block;
            }
            previous = block;
        }
        return previous;
    };
    const scalarToDocument = (offset: number) => {
        const scalar = clampScalar(offset);
        const block = [...blocks]
            .reverse()
            .find((candidate) => candidate.scalarStart <= scalar);
        if (!block) return 0;
        const intraScalar = scalar - block.scalarStart;
        if (block.isVoid) {
            return intraScalar >= block.scalarLength
                ? block.documentStart + 1
                : block.documentStart;
        }
        if (block.isPlaceholder) return block.documentStart;
        const span = block.inlineSpans.find(
            (candidate) =>
                intraScalar >= candidate.scalarStart && intraScalar < candidate.scalarEnd
        );
        if (!span) return block.documentEnd;
        return span.kind === 'text'
            ? span.documentStart + (intraScalar - span.scalarStart)
            : span.documentStart;
    };
    const documentToScalar = (offset: number) => {
        const position = clampDocumentOffset(offset);
        const block = blockForDocumentOffset(position);
        if (!block) return scalarLength;
        if (block.isVoid) {
            return block.scalarStart +
                (position <= block.documentStart ? 0 : block.scalarLength);
        }
        if (block.isPlaceholder) {
            return block.scalarStart +
                (position < block.documentStart ? 0 : block.scalarLength);
        }
        if (position < block.documentStart) return block.scalarStart;
        if (position > block.documentEnd) return block.scalarStart + block.scalarLength;
        for (const span of block.inlineSpans) {
            if (position < span.documentStart) return block.scalarStart + span.scalarStart;
            if (position < span.documentEnd) {
                return block.scalarStart +
                    span.scalarStart +
                    (span.kind === 'text' ? position - span.documentStart : 0);
            }
        }
        return block.scalarStart + block.scalarLength;
    };
    const activeStateAt = (offset: number) => {
        const position = clampDocumentOffset(offset);
        const block = blockForDocumentOffset(position);
        const span =
            block?.inlineSpans.find(
                (candidate) =>
                    position >= candidate.documentStart && position < candidate.documentEnd
            ) ??
            [...(block?.inlineSpans ?? [])]
                .reverse()
                .find((candidate) => position === candidate.documentEnd);
        const marks: Record<string, boolean> = {};
        const markAttrs: Record<string, Record<string, unknown>> = {};
        const nodes: Record<string, boolean> = {};
        for (const mark of span?.marks ?? []) {
            const type = typeof mark.type === 'string' ? mark.type : '';
            if (!type) continue;
            marks[type] = true;
            if (mark.attrs != null && typeof mark.attrs === 'object' && !Array.isArray(mark.attrs)) {
                markAttrs[type] = { ...(mark.attrs as Record<string, unknown>) };
            }
        }
        for (const node of block?.ancestors ?? []) {
            const type = typeof node.type === 'string' ? node.type : '';
            if (type === 'heading') {
                const level = (node.attrs as Record<string, unknown> | undefined)?.level;
                if (level != null) nodes[`heading:${String(level)}`] = true;
            } else if (type === 'blockquote' || type === 'bulletList' || type === 'orderedList') {
                nodes[type] = true;
            }
        }
        return { marks, markAttrs, nodes };
    };

    return { scalarLength, clampDocumentOffset, scalarToDocument, documentToScalar, activeStateAt };
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
    hasStoredMarks: boolean;
    hasStoredNodes: boolean;
    selection: { anchor: number; head: number };
    liveGeneration: bigint | null;
    lastIssuedGeneration: bigint;
    protocolQueue: Uint8Array[];
    documentQueue: Uint8Array[];
    desiredAwareness: Record<string, unknown> | null;
    localClientId: string;
    localClock: number;
    localAwarenessLive: boolean;
    remotePeers: NativeEditorV2PeerInfo[];
    remoteAwarenessClocks: Map<string, number>;
    awarenessNowMillis: bigint;
    lastLocalAwarenessPublishMillis: bigint | null;
    remotePeerActivity: Map<string, bigint>;
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
    /** Queue the clocked per-client delta the next inbound awareness frame applies. */
    pushRemotePeers(editorId: string, peers: NativeEditorV2PeerInfo[]): void;
    /** Seed the exact last-issued u64 generation for boundary tests. */
    seedLastIssuedGeneration(editorId: string, generation: string): void;
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
    awarenessDeltas: NativeEditorV2PeerInfo[][];
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
            entry = {
                docs: [],
                awarenessDeltas: [],
                applyLocalApiErrors: [],
                applyCommandErrors: [],
            };
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

    function clearTransportAwareness(session: FakeSession): void {
        if (session.localAwarenessLive) {
            session.localClock += 1;
            session.localAwarenessLive = false;
        }
        session.remotePeers = [];
        session.remoteAwarenessClocks.clear();
        session.remotePeerActivity.clear();
    }

    function checkedAddV2U64(left: bigint, right: bigint): bigint | null {
        return left > V2_FAKE_U64_MAX - right ? null : left + right;
    }

    function setLocalAwarenessState(session: FakeSession): void {
        session.localClock += 1;
        session.localAwarenessLive = true;
    }

    function enqueueLocalAwareness(session: FakeSession): void {
        session.protocolQueue.push(awarenessFrame(session.localClock));
        session.lastLocalAwarenessPublishMillis = session.awarenessNowMillis;
    }

    function publishLocalAwareness(session: FakeSession): void {
        setLocalAwarenessState(session);
        enqueueLocalAwareness(session);
    }

    function clearLocalAwareness(session: FakeSession): void {
        if (!session.localAwarenessLive) return;
        session.localClock += 1;
        session.localAwarenessLive = false;
        if (session.transportState === 'Synchronized') {
            session.protocolQueue.push(awarenessFrame(session.localClock));
        }
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
        let remoteExpiry: bigint | null = null;
        for (const seenAt of session.remotePeerActivity.values()) {
            const deadline = checkedAddV2U64(seenAt, V2_FAKE_AWARENESS_EXPIRY_MILLIS);
            if (deadline == null) continue;
            if (remoteExpiry == null || deadline < remoteExpiry) remoteExpiry = deadline;
        }
        if (localRenewal == null) return remoteExpiry;
        if (remoteExpiry == null) return localRenewal;
        return localRenewal < remoteExpiry ? localRenewal : remoteExpiry;
    }

    function applyRemoteAwarenessDelta(
        session: FakeSession,
        entries: NativeEditorV2PeerInfo[]
    ): void {
        for (const peer of entries) {
            const clientId = canonicalV2U64(peer.clientId);
            if (peer.isLocal || clientId == null || clientId === session.localClientId) continue;
            const currentClock = session.remoteAwarenessClocks.get(clientId);
            const currentPeerIndex = session.remotePeers.findIndex(
                (candidate) => candidate.clientId === clientId
            );
            const isTombstone = peer.state == null;
            const removesEqualClockLivePeer =
                isTombstone && currentPeerIndex >= 0 && currentClock === peer.clock;
            if (
                currentClock != null &&
                peer.clock <= currentClock &&
                !removesEqualClockLivePeer
            ) {
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
        entries: NativeEditorV2PeerInfo[]
    ): FakeErrorRecord | null {
        for (const peer of entries) {
            const clientId = canonicalV2U64(peer.clientId);
            if (peer.isLocal || clientId == null || clientId === session.localClientId) continue;
            if (peer.clock <= V2_FAKE_MAX_ADMITTED_REMOTE_AWARENESS_CLOCK) continue;

            const error = errorRecord(
                'transport',
                'TRANSPORT_AWARENESS_LIMIT_EXCEEDED',
                'awareness frame handling failed'
            );
            error.details = {
                action: 'receiveMessage',
                cause: {
                    code: 'INPUT_LIMIT_EXCEEDED',
                    message: `input exceeds limit ${V2_FAKE_MAX_ADMITTED_REMOTE_AWARENESS_CLOCK}: ${peer.clock}`,
                    limit: V2_FAKE_MAX_ADMITTED_REMOTE_AWARENESS_CLOCK,
                    actual: peer.clock,
                    details: { field: 'awarenessClock' },
                },
            };
            return error;
        }
        return null;
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
                publishLocalAwareness(session);
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
            const entries = remote.awarenessDeltas.shift() ?? [];
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
                hasStoredMarks: false,
                hasStoredNodes: false,
                selection: { anchor: 1, head: 1 },
                liveGeneration: null,
                lastIssuedGeneration: 0n,
                protocolQueue: [],
                documentQueue: [],
                desiredAwareness: null,
                localClientId: String((clientIdCounter += 1)),
                localClock: 0,
                localAwarenessLive: false,
                remotePeers: [],
                remoteAwarenessClocks: new Map(),
                awarenessNowMillis: 0n,
                lastLocalAwarenessPublishMillis: null,
                remotePeerActivity: new Map(),
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
                session.localAwarenessLive = false;
                session.remotePeers = [];
                session.remoteAwarenessClocks.clear();
                session.remotePeerActivity.clear();
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
                    return revisionMismatchError(session, request.baseDocumentRevision);
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
                    return revisionMismatchError(session, request.baseDocumentRevision);
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
                const documentActiveState = () =>
                    fakeScalarDocumentMap(session.doc).activeStateAt(session.selection.head);
                const storedMarks = () => {
                    const documentState = documentActiveState();
                    return {
                        marks: {
                            ...(session.hasStoredMarks ? session.activeMarks : documentState.marks),
                        },
                        markAttrs: {
                            ...(session.hasStoredMarks
                                ? session.activeMarkAttrs
                                : documentState.markAttrs),
                        },
                    };
                };
                const storedNodes = () => ({
                    ...(session.hasStoredNodes
                        ? session.activeNodes
                        : documentActiveState().nodes),
                });
                switch (type) {
                    case 'toggleMark': {
                        const markType = String(command.markType ?? '');
                        const next = storedMarks();
                        if (next.marks[markType]) {
                            next.marks[markType] = false;
                            session.activeMarks = next.marks;
                            session.activeMarkAttrs = next.markAttrs;
                            delete session.activeMarkAttrs[markType];
                        } else {
                            next.marks[markType] = true;
                            session.activeMarks = next.marks;
                            session.activeMarkAttrs = next.markAttrs;
                        }
                        session.hasStoredMarks = true;
                        return stateOnlyOutcome();
                    }
                    case 'setMark': {
                        const markType = String(command.markType ?? '');
                        const next = storedMarks();
                        next.marks[markType] = true;
                        next.markAttrs[markType] =
                            (command.attrs as Record<string, unknown>) ?? {};
                        session.activeMarks = next.marks;
                        session.activeMarkAttrs = next.markAttrs;
                        session.hasStoredMarks = true;
                        return stateOnlyOutcome();
                    }
                    case 'unsetMark': {
                        const markType = String(command.markType ?? '');
                        const next = storedMarks();
                        next.marks[markType] = false;
                        session.activeMarks = next.marks;
                        session.activeMarkAttrs = next.markAttrs;
                        delete session.activeMarkAttrs[markType];
                        session.hasStoredMarks = true;
                        return stateOnlyOutcome();
                    }
                    case 'toggleHeading': {
                        const level = String(command.level ?? '');
                        const key = `heading:${level}`;
                        const next = storedNodes();
                        next[key] = !next[key];
                        session.activeNodes = next;
                        session.hasStoredNodes = true;
                        return stateOnlyOutcome();
                    }
                    case 'toggleBlockquote': {
                        const next = storedNodes();
                        next.blockquote = !next.blockquote;
                        session.activeNodes = next;
                        session.hasStoredNodes = true;
                        return stateOnlyOutcome();
                    }
                    case 'wrapInList': {
                        const next = storedNodes();
                        next[String(command.listType ?? '')] = true;
                        session.activeNodes = next;
                        session.hasStoredNodes = true;
                        return stateOnlyOutcome();
                    }
                    case 'unwrapFromList': {
                        session.activeNodes = Object.fromEntries(
                            Object.keys(storedNodes()).map((key) => [key, false])
                        );
                        session.hasStoredNodes = true;
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
                                { type: 'blockEnd' },
                            ];
                        }
                    );
                    const scalarMap = fakeScalarDocumentMap(session.doc);
                    const selection =
                        mirrorAnchor != null && mirrorHead != null
                            ? {
                                  anchor: scalarMap.scalarToDocument(exactV2U32(mirrorAnchor)!),
                                  head: scalarMap.scalarToDocument(exactV2U32(mirrorHead)!),
                              }
                            : session.selection;
                    const documentActiveState = scalarMap.activeStateAt(selection.head);
                    const usesStoredState =
                        mirrorAnchor == null &&
                        mirrorHead == null &&
                        selection.anchor === selection.head;
                    const marks = { ...documentActiveState.marks };
                    const markAttrs = { ...documentActiveState.markAttrs };
                    const nodes = { ...documentActiveState.nodes };
                    if (usesStoredState && session.hasStoredMarks) {
                        for (const [markType, active] of Object.entries(session.activeMarks)) {
                            marks[markType] = active;
                            if (!active) delete markAttrs[markType];
                        }
                        for (const [markType, attrs] of Object.entries(session.activeMarkAttrs)) {
                            if (marks[markType] && Object.keys(attrs).length > 0) {
                                markAttrs[markType] = { ...attrs };
                            }
                        }
                    }
                    if (usesStoredState && session.hasStoredNodes) {
                        Object.assign(nodes, session.activeNodes);
                    }
                    const update: Record<string, unknown> = {
                        renderBlocks: blocks,
                        renderPatch: null,
                        activeState: {
                            marks,
                            markAttrs,
                            nodes,
                            commands: {},
                            allowedMarks: ['bold', 'italic', 'underline', 'strike', 'link'],
                            insertableNodes: ['image', 'horizontalRule', 'hardBreak'],
                        },
                        historyState: {
                            canUndo: session.undoStack.length > 0,
                            canRedo: session.redoStack.length > 0,
                        },
                        documentVersion: String(session.documentRevision),
                        stateRevision: String(session.stateRevision),
                        scalarLength: scalarMap.scalarLength,
                    };
                    if (mirrorAnchor != null && mirrorHead != null) {
                        update.selection = {
                            type: 'text',
                            anchor: selection.anchor,
                            head: selection.head,
                            anchorScalar: scalarMap.documentToScalar(selection.anchor),
                            headScalar: scalarMap.documentToScalar(selection.head),
                        };
                    } else {
                        update.selection = {
                            type: 'text',
                            anchor: selection.anchor,
                            head: selection.head,
                            anchorScalar: scalarMap.documentToScalar(selection.anchor),
                            headScalar: scalarMap.documentToScalar(selection.head),
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
                    return revisionMismatchError(session, request.baseDocumentRevision);
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
                    return revisionMismatchError(session, request.baseDocumentRevision);
                }
                const selection = request.selection as Record<string, unknown> | undefined;
                const anchor = exactV2U32(selection?.anchor);
                const head = exactV2U32(selection?.head);
                if (anchor != null && head != null) {
                    const scalarMap = fakeScalarDocumentMap(session.doc);
                    session.selection = {
                        anchor: scalarMap.clampDocumentOffset(anchor),
                        head: scalarMap.clampDocumentOffset(head),
                    };
                    session.stateRevision += 1;
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
                if (session.lastIssuedGeneration === V2_FAKE_U64_MAX) {
                    return transportError(
                        'TRANSPORT_GENERATION_EXHAUSTED',
                        'transport generation space is exhausted',
                        {
                            action: 'beginConnect',
                            transportState: session.transportState,
                        }
                    );
                }
                const nextGeneration = session.lastIssuedGeneration + 1n;
                session.lastIssuedGeneration = nextGeneration;
                session.liveGeneration = nextGeneration;
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
                    if (session.desiredAwareness == null) return okRecord(true);
                    clearLocalAwareness(session);
                    session.desiredAwareness = null;
                    session.lastLocalAwarenessPublishMillis = null;
                } else {
                    let desiredAwareness: Record<string, unknown>;
                    try {
                        desiredAwareness = JSON.parse(awarenessJson) as Record<string, unknown>;
                    } catch (error) {
                        const message = error instanceof Error ? error.message : String(error);
                        return boundaryError(
                            'AWARENESS_STATE_INVALID',
                            `desired awareness state is not valid JSON: ${message}`
                        );
                    }
                    session.desiredAwareness = desiredAwareness;
                    setLocalAwarenessState(session);
                }
                if (awarenessJson.trim() !== 'null' && session.transportState === 'Synchronized') {
                    enqueueLocalAwareness(session);
                }
                return okRecord(true);
            })
        ),
        editorV2CollaborationPeers: jest.fn((editorId: string) =>
            withSession(editorId, (session) => {
                const peers: NativeEditorV2PeerInfo[] = [];
                if (session.localAwarenessLive && session.desiredAwareness != null) {
                    peers.push({
                        clientId: session.localClientId,
                        clock: session.localClock,
                        isLocal: true,
                        state: session.desiredAwareness,
                        cursor: null,
                    });
                }
                peers.push(...session.remotePeers);
                peers.sort((left, right) => {
                    const leftId = BigInt(left.clientId);
                    const rightId = BigInt(right.clientId);
                    return leftId < rightId ? -1 : leftId > rightId ? 1 : 0;
                });
                return okRecord(JSON.stringify({ peers }));
            })
        ),
        editorV2CollaborationTick: jest.fn((editorId: string, nowMillis: string) =>
            withSession(editorId, (session) => {
                if (!session.roomBound) {
                    return boundaryError(
                        'CONFIG_INVALID',
                        'local sessions have no attached collaboration runtime'
                    );
                }
                const canonicalNowMillis = canonicalV2U64(nowMillis);
                if (canonicalNowMillis == null) {
                    return boundaryError('CONFIG_INVALID', 'nowMillis must be canonical decimal u64 text');
                }
                const acceptedNowMillis = BigInt(canonicalNowMillis);
                if (acceptedNowMillis < session.awarenessNowMillis) {
                    return transportError(
                        'AWARENESS_TIME_REGRESSION',
                        'awareness tick nowMillis must not decrease',
                        {
                            nowMillis: canonicalNowMillis,
                            lastNowMillis: String(session.awarenessNowMillis),
                        }
                    );
                }
                session.awarenessNowMillis = acceptedNowMillis;
                const expiredPeers = [...session.remotePeerActivity.entries()]
                    .filter(
                        ([, seenAt]) =>
                            session.awarenessNowMillis >= seenAt &&
                            session.awarenessNowMillis - seenAt >=
                                V2_FAKE_AWARENESS_EXPIRY_MILLIS
                    )
                    .map(([clientId]) => clientId)
                    .sort((left, right) => (BigInt(left) < BigInt(right) ? -1 : 1));
                for (const clientId of expiredPeers) {
                    const clock = session.remoteAwarenessClocks.get(clientId);
                    if (clock != null) session.remoteAwarenessClocks.set(clientId, clock + 1);
                    session.remotePeerActivity.delete(clientId);
                }
                if (expiredPeers.length > 0) {
                    const expired = new Set(expiredPeers);
                    session.remotePeers = session.remotePeers.filter(
                        (peer) => !expired.has(canonicalV2U64(peer.clientId) ?? peer.clientId)
                    );
                }
                let renewedLocal = false;
                if (
                    session.transportState === 'Synchronized' &&
                    session.desiredAwareness != null &&
                    (session.lastLocalAwarenessPublishMillis == null ||
                        (session.awarenessNowMillis >= session.lastLocalAwarenessPublishMillis &&
                            session.awarenessNowMillis - session.lastLocalAwarenessPublishMillis >=
                                V2_FAKE_AWARENESS_RENEWAL_INTERVAL_MILLIS))
                ) {
                    publishLocalAwareness(session);
                    renewedLocal = true;
                }
                const nextDeadline = nextAwarenessDeadline(session);
                return okRecord(
                    JSON.stringify({
                        nextDeadlineMillis: nextDeadline == null ? null : String(nextDeadline),
                        renewedLocal,
                        expiredPeers,
                        outboundChanged: renewedLocal,
                        peersChanged: renewedLocal || expiredPeers.length > 0,
                    })
                );
            })
        ),
        editorV2CollaborationDetach: jest.fn((editorId: string) =>
            withSession(editorId, (session) => {
                if (session.transportState === 'Detached') {
                    clearTransportAwareness(session);
                    return okRecord(true);
                }
                session.liveGeneration = null;
                session.transportState = 'Detached';
                clearTransportAwareness(session);
                return okRecord(true);
            })
        ),
        editorV2CollaborationReattach: jest.fn((editorId: string) =>
            withSession(editorId, (session) => {
                if (!session.roomBound) {
                    return transportError(
                        'TRANSPORT_NOT_ROOM_BOUND',
                        'local-only sessions have no room binding to reattach'
                    );
                }
                if (session.transportState !== 'Detached') {
                    if (session.transportState === 'Disconnected') {
                        clearTransportAwareness(session);
                        return okRecord(true);
                    }
                    return transportError(
                        'TRANSPORT_INVALID_TRANSITION',
                        `reattach is only admitted from Detached (found ${session.transportState})`
                    );
                }
                session.transportState = 'Disconnected';
                clearTransportAwareness(session);
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
                    session.doc = cloneDoc(parsed.doc);
                    session.documentRevision =
                        typeof parsed.revision === 'number' ? parsed.revision : 1;
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
