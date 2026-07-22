import { requireNativeModule } from 'expo-modules-core';
import type { EditorMentionTheme } from './EditorTheme';
import {
    validateEditorCreateLimits,
    type EditorCollaborationLimits,
    type EditorEditingLimits,
    type EditorResourceLimits,
} from './ResourceLimits';
import type { SchemaDefinition } from './schemas';
import {
    NativeEditorBoundaryError,
    NativeEditorV2BoundaryError,
    NativeEditorV2ErrorBase,
    NativeEditorV2NonRetryableError,
    nativeEditorV2ErrorToException,
    normalizeNativeEditorV2Error,
    type NativeEditorV2Error,
} from './NativeEditorBoundaryError';

// ─── Shared types ───────────────────────────────────────────────
// Neutral document/render state types shared by the v2 document
// handle, the React components, and the schema/addon helpers.

export interface Selection {
    type: 'text' | 'node' | 'all';
    anchor?: number;
    head?: number;
    pos?: number;
}

export interface ListContext {
    ordered: boolean;
    index: number;
    total: number;
    start: number;
    isFirst: boolean;
    isLast: boolean;
    kind?: string | null;
    checked?: boolean | null;
}

export interface RenderMarkWithAttrs {
    type: string;
    [key: string]: unknown;
}

export type RenderMark = string | RenderMarkWithAttrs;

export interface RenderElement {
    type:
        | 'textRun'
        | 'blockStart'
        | 'blockEnd'
        | 'voidInline'
        | 'voidBlock'
        | 'opaqueInlineAtom'
        | 'opaqueBlockAtom';
    text?: string;
    marks?: RenderMark[];
    nodeType?: string;
    depth?: number;
    docPos?: number;
    label?: string;
    attrs?: Record<string, unknown>;
    mentionTheme?: EditorMentionTheme;
    listContext?: ListContext;
}

interface RenderBlocksPatch {
    startIndex: number;
    deleteCount: number;
    renderBlocks: RenderElement[][];
}

export interface ActiveState {
    marks: Record<string, boolean>;
    markAttrs: Record<string, Record<string, unknown>>;
    nodes: Record<string, boolean>;
    commands: Record<string, boolean>;
    allowedMarks: string[];
    insertableNodes: string[];
}

export interface HistoryState {
    canUndo: boolean;
    canRedo: boolean;
}

export interface EditorUpdate {
    renderElements: RenderElement[];
    renderBlocks?: RenderElement[][];
    renderPatch?: RenderBlocksPatch;
    selection: Selection;
    activeState: ActiveState;
    historyState: HistoryState;
    documentVersion?: number;
}

export interface ContentSnapshot {
    html: string;
    json: DocumentJSON;
}

export interface DocumentJSON {
    [key: string]: unknown;
}

export interface CollaborationPeer {
    clientId: number;
    isLocal: boolean;
    state: Record<string, unknown> | null;
}

export type CommandBlockedReason =
    | 'composition'
    | 'detached'
    | 'pendingUpdate'
    | 'destroyed'
    | 'unknown';

export interface CommandBlockedInfo {
    blocked: boolean;
    reason: CommandBlockedReason | null;
}

let _nativeModule: NativeEditorV2Module | null = null;

function getNativeModule(): NativeEditorV2Module {
    if (!_nativeModule) {
        _nativeModule = requireNativeModule<NativeEditorV2Module>('NativeEditor');
    }
    return _nativeModule;
}

/** @internal Reset the cached native module reference. For testing only. */
export function _resetNativeModuleCache(): void {
    _nativeModule = null;
}

// ─── FFI v2 surface ─────────────────────────────────────────────
// Production v2 document handle and typed result normalization.
// This is the only construction path: the native module exposes the
// editor_v2_* UniFFI ABI, and everything below consumes the frozen v2
// result records ({ value, error } with exactly one side set),
// normalizes them at the JavaScript boundary (decimal-string
// revisions/identifiers, direct binary values, unsafe-integer rejection), and
// raises typed imperative errors per domain with a distinct non-retryable
// class for ENGINE_INVARIANT_FAILED and lifecycle-destroyed states.

const ERR_V2_NATIVE_RESPONSE = 'NativeEditorBridge: invalid v2 result record from native module';
const ERR_V2_DESTROYED = 'NativeEditorBridge: v2 editor handle has been destroyed';
const V2_ENVELOPE_VERSION = 1;
const CANONICAL_V2_DECIMAL_ID = /^(0|[1-9]\d*)$/;

/**
 * The v2 surface of the NativeEditor native module — the complete
 * production ABI. Every call resolves the method lazily and fails clearly
 * when the v2 surface is absent. Decimal-string identifiers keep full u64
 * fidelity across the JavaScript boundary, and binaries travel as direct
 * Uint8Array values (never JSON number arrays).
 */
export interface NativeEditorV2Module {
    editorV2Create(configJson: string, snapshotState: Uint8Array | null): unknown;
    editorV2Destroy(editorId: string): unknown;
    editorV2GetState(editorId: string): unknown;
    editorV2GetDocumentJson(editorId: string): unknown;
    editorV2GetDocumentHtml(editorId: string): unknown;
    editorV2GetContentSnapshot(editorId: string): unknown;
    editorV2ReplaceDocument(editorId: string, requestJson: string): unknown;
    editorV2ApplyInput(editorId: string, requestJson: string): unknown;
    editorV2ApplyCommand(editorId: string, requestJson: string): unknown;
    editorV2ApplyLocalApi(editorId: string, requestJson: string): unknown;
    editorV2SetSelection(editorId: string, requestJson: string): unknown;
    editorV2Undo(editorId: string, requestJson: string): unknown;
    editorV2Redo(editorId: string, requestJson: string): unknown;
    editorV2RenderUpdate(
        editorId: string,
        mirrorScalarAnchor: number | null,
        mirrorScalarHead: number | null
    ): unknown;
    editorV2CollaborationBeginConnect(editorId: string): unknown;
    editorV2CollaborationSocketOpen(editorId: string, generation: string): unknown;
    editorV2CollaborationReceive(editorId: string, generation: string, message: Uint8Array): unknown;
    editorV2CollaborationSocketClose(
        editorId: string,
        generation: string,
        code: number | null,
        reason: string | null
    ): unknown;
    editorV2CollaborationTakeOutbound(editorId: string, generation: string): unknown;
    editorV2CollaborationSetAwareness(editorId: string, awarenessJson: string): unknown;
    editorV2CollaborationPeers(editorId: string): unknown;
    editorV2SnapshotExport(editorId: string): unknown;
    editorV2SnapshotRestore(editorId: string, metadataJson: string, encodedState: Uint8Array): unknown;
}

function invokeNativeEditorV2<K extends keyof NativeEditorV2Module>(
    name: K,
    ...args: Parameters<NativeEditorV2Module[K]>
): unknown {
    const nativeModule = getNativeModule() as unknown as Record<string, unknown>;
    const method = nativeModule[name as string];
    if (typeof method !== 'function') {
        throw new Error(
            `NativeEditorBridge: native module does not expose the v2 entry ${String(name)}`
        );
    }
    return (method as (...fnArgs: unknown[]) => unknown).apply(nativeModule, args);
}

// ─── Result record normalization ────────────────────────────────

/** The discriminated envelope every v2 result record normalizes into. */
export type NativeEditorV2Result<T> =
    | { ok: true; value: T }
    | { ok: false; error: NativeEditorV2Error };

function isPlainRecord(value: unknown): value is Record<string, unknown> {
    return value != null && typeof value === 'object' && !Array.isArray(value);
}

/**
 * Validate a raw v2 result record: exactly one of value/error, every error
 * field type, every domain/code string, and the caller-supplied value
 * normalizer. Returns null for any contract violation; the imperative layer
 * turns that into the non-retryable FFI_RESULT_INVALID error.
 */
export function normalizeNativeEditorV2Result<T>(
    raw: unknown,
    normalizeValue: (value: unknown) => T | null
): NativeEditorV2Result<T> | null {
    if (!isPlainRecord(raw)) return null;
    const hasValue = raw.value !== null && raw.value !== undefined;
    const hasError = raw.error !== null && raw.error !== undefined;
    if (hasValue === hasError) return null;
    if (hasError) {
        const error = normalizeNativeEditorV2Error({ error: raw.error });
        return error == null ? null : { ok: false, error };
    }
    const value = normalizeValue(raw.value);
    return value == null ? null : { ok: true, value };
}

/** Parse a JSON-string result value; anything else is a contract violation. */
export function parseNativeEditorV2JsonValue(value: unknown): unknown | null {
    if (typeof value !== 'string' || value === '') return null;
    try {
        return JSON.parse(value) as unknown;
    } catch {
        return null;
    }
}

/**
 * Validate a direct binary result value. JSON number arrays are rejected on
 * purpose: the frozen v2 contract moves bytes as bytes.
 */
export function normalizeNativeEditorV2Bytes(value: unknown): Uint8Array | null {
    if (value instanceof Uint8Array) return value;
    if (ArrayBuffer.isView(value)) {
        return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
    }
    if (value instanceof ArrayBuffer) return new Uint8Array(value);
    return null;
}

/** FfiUnitResult successes are exactly `true`; anything else is invalid. */
export function normalizeNativeEditorV2Unit(value: unknown): true | null {
    return value === true ? true : null;
}

/**
 * Normalize a request/revision identifier to its canonical decimal string.
 * Decimal strings of any size pass verbatim (never Number()'d); numbers must
 * be non-negative safe integers. Everything else is rejected.
 */
export function normalizeNativeEditorV2DecimalId(value: unknown): string | null {
    if (typeof value === 'string') {
        return CANONICAL_V2_DECIMAL_ID.test(value) ? value : null;
    }
    if (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0) {
        return String(value);
    }
    return null;
}

function normalizeRevisionField(record: Record<string, unknown>, field: string): string | null {
    return normalizeNativeEditorV2DecimalId(record[field]);
}

function unsignedSafeInteger(value: unknown): number | null {
    return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function optionalBoolean(value: unknown): boolean | null {
    return typeof value === 'boolean' ? value : null;
}

// ─── Typed value shapes ─────────────────────────────────────────

const V2_DOCUMENT_STATES = ['LocalReady', 'AwaitRemote', 'RoomReady'] as const;
export type NativeEditorV2DocumentState = (typeof V2_DOCUMENT_STATES)[number];

const V2_TRANSPORT_STATES = [
    'Detached',
    'Disconnected',
    'Connecting',
    'Handshaking',
    'Synchronized',
    'Incompatible',
    'Destroying',
    'Destroyed',
] as const;
export type NativeEditorV2TransportState = (typeof V2_TRANSPORT_STATES)[number];

const V2_RENDER_STATES = ['Loading', 'Ready'] as const;
export type NativeEditorV2RenderState = (typeof V2_RENDER_STATES)[number];

function whitelisted<T extends string>(value: unknown, allowed: readonly T[]): T | null {
    return typeof value === 'string' && (allowed as readonly string[]).includes(value)
        ? (value as T)
        : null;
}

export interface NativeEditorV2EditorState {
    documentState: NativeEditorV2DocumentState;
    transportState: NativeEditorV2TransportState;
    renderState: NativeEditorV2RenderState;
    documentRevision: string;
    stateRevision: string;
    canUndo: boolean;
    canRedo: boolean;
}

export function normalizeNativeEditorV2StateValue(value: unknown): NativeEditorV2EditorState | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    const documentState = whitelisted(parsed.documentState, V2_DOCUMENT_STATES);
    const transportState = whitelisted(parsed.transportState, V2_TRANSPORT_STATES);
    const renderState = whitelisted(parsed.renderState, V2_RENDER_STATES);
    const documentRevision = normalizeRevisionField(parsed, 'documentRevision');
    const stateRevision = normalizeRevisionField(parsed, 'stateRevision');
    const canUndo = optionalBoolean(parsed.canUndo);
    const canRedo = optionalBoolean(parsed.canRedo);
    if (
        documentState == null ||
        transportState == null ||
        renderState == null ||
        documentRevision == null ||
        stateRevision == null ||
        canUndo == null ||
        canRedo == null
    ) {
        return null;
    }
    return {
        documentState,
        transportState,
        renderState,
        documentRevision,
        stateRevision,
        canUndo,
        canRedo,
    };
}

export type NativeEditorV2MutationOutcome =
    | {
          type: 'transaction';
          changed: boolean;
          documentRevision: string;
          stateRevision: string;
          canUndo: boolean;
          canRedo: boolean;
      }
    | { type: 'notApplicable' }
    | { type: 'replacement'; changed: boolean; documentRevision: string };

export function normalizeNativeEditorV2MutationOutcomeValue(
    value: unknown
): NativeEditorV2MutationOutcome | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    if (parsed.type === 'notApplicable') {
        return { type: 'notApplicable' };
    }
    if (parsed.type === 'replacement') {
        const changed = optionalBoolean(parsed.changed);
        const documentRevision = normalizeRevisionField(parsed, 'documentRevision');
        if (changed == null || documentRevision == null) return null;
        return { type: 'replacement', changed, documentRevision };
    }
    if (parsed.type === 'transaction') {
        const changed = optionalBoolean(parsed.changed);
        const documentRevision = normalizeRevisionField(parsed, 'documentRevision');
        const stateRevision = normalizeRevisionField(parsed, 'stateRevision');
        const canUndo = optionalBoolean(parsed.canUndo);
        const canRedo = optionalBoolean(parsed.canRedo);
        if (
            changed == null ||
            documentRevision == null ||
            stateRevision == null ||
            canUndo == null ||
            canRedo == null
        ) {
            return null;
        }
        return {
            type: 'transaction',
            changed,
            documentRevision,
            stateRevision,
            canUndo,
            canRedo,
        };
    }
    return null;
}

export interface NativeEditorV2CommitInfo {
    changed: boolean;
    documentRevision: string;
}

export function normalizeNativeEditorV2CommitValue(value: unknown): NativeEditorV2CommitInfo | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    const changed = optionalBoolean(parsed.changed);
    const documentRevision = normalizeRevisionField(parsed, 'documentRevision');
    if (changed == null || documentRevision == null) return null;
    return { changed, documentRevision };
}

export function normalizeNativeEditorV2ChangedValue(value: unknown): boolean | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    return optionalBoolean(parsed.changed);
}

export function normalizeNativeEditorV2HtmlValue(value: unknown): string | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed) || typeof parsed.html !== 'string') return null;
    return parsed.html;
}

/**
 * Validate a render-update result value: the payload is itself a JSON
 * document (render blocks, active/history state, document version) and is
 * returned verbatim for the bound native view to apply.
 */
export function normalizeNativeEditorV2RenderUpdateValue(value: unknown): string | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    return isPlainRecord(parsed) ? (value as string) : null;
}

export function normalizeNativeEditorV2DocumentJsonValue(value: unknown): DocumentJSON | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    return isPlainRecord(parsed) ? (parsed as DocumentJSON) : null;
}

export function normalizeNativeEditorV2ContentSnapshotValue(
    value: unknown
): ContentSnapshot | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed) || typeof parsed.html !== 'string' || !isPlainRecord(parsed.json)) {
        return null;
    }
    return { html: parsed.html, json: parsed.json as DocumentJSON };
}

export interface NativeEditorV2SnapshotExport {
    metadataJson: string;
    encodedState: Uint8Array;
}

/** The snapshot export record arrives as direct fields (JSON + bytes), not a JSON string. */
export function normalizeNativeEditorV2SnapshotExportValue(
    value: unknown
): NativeEditorV2SnapshotExport | null {
    if (!isPlainRecord(value) || typeof value.metadataJson !== 'string') return null;
    const encodedState = normalizeNativeEditorV2Bytes(value.encodedState);
    if (encodedState == null) return null;
    return { metadataJson: value.metadataJson, encodedState };
}

export function normalizeNativeEditorV2CreateValue(value: unknown): { editorId: string } | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    const editorId = normalizeNativeEditorV2DecimalId(parsed.editorId);
    return editorId == null ? null : { editorId };
}

export function normalizeNativeEditorV2GenerationValue(value: unknown): string | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    return normalizeNativeEditorV2DecimalId(parsed.generation);
}

export function normalizeNativeEditorV2TransportStateValue(
    value: unknown
): NativeEditorV2TransportState | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    return whitelisted(parsed.transportState, V2_TRANSPORT_STATES);
}

export interface NativeEditorV2PeerInfo {
    clientId: string;
    clock: number;
    isLocal: boolean;
    state: Record<string, unknown> | null;
    cursor: { anchor: number; head: number } | null;
}

export function normalizeNativeEditorV2PeersValue(value: unknown): NativeEditorV2PeerInfo[] | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed) || !Array.isArray(parsed.peers)) return null;
    const peers: NativeEditorV2PeerInfo[] = [];
    for (const rawPeer of parsed.peers) {
        if (!isPlainRecord(rawPeer)) return null;
        const clientId = normalizeNativeEditorV2DecimalId(rawPeer.clientId);
        const clock = unsignedSafeInteger(rawPeer.clock);
        const isLocal = optionalBoolean(rawPeer.isLocal);
        if (clientId == null || clock == null || isLocal == null) return null;
        if (rawPeer.state !== null && !isPlainRecord(rawPeer.state)) return null;
        let cursor: NativeEditorV2PeerInfo['cursor'] = null;
        if (rawPeer.cursor !== null && rawPeer.cursor !== undefined) {
            if (!isPlainRecord(rawPeer.cursor)) return null;
            const anchor = unsignedSafeInteger(rawPeer.cursor.anchor);
            const head = unsignedSafeInteger(rawPeer.cursor.head);
            if (anchor == null || head == null) return null;
            cursor = { anchor, head };
        }
        peers.push({
            clientId,
            clock,
            isLocal,
            state: (rawPeer.state as Record<string, unknown> | null) ?? null,
            cursor,
        });
    }
    return peers;
}

export interface NativeEditorV2ReceiveClose {
    disposition: 'retryable' | 'incompatible';
    error: NativeEditorV2Error;
}

export interface NativeEditorV2ReceiveOutcome {
    framesDecoded: number;
    repliesEnqueued: number;
    replyBytesEnqueued: number;
    remoteCommitApplied: boolean;
    documentPromoted: boolean;
    transportState: NativeEditorV2TransportState;
    close: NativeEditorV2ReceiveClose | null;
}

export function normalizeNativeEditorV2ReceiveValue(
    value: unknown
): NativeEditorV2ReceiveOutcome | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    const framesDecoded = unsignedSafeInteger(parsed.framesDecoded);
    const repliesEnqueued = unsignedSafeInteger(parsed.repliesEnqueued);
    const replyBytesEnqueued = unsignedSafeInteger(parsed.replyBytesEnqueued);
    const remoteCommitApplied = optionalBoolean(parsed.remoteCommitApplied);
    const documentPromoted = optionalBoolean(parsed.documentPromoted);
    const transportState = whitelisted(parsed.transportState, V2_TRANSPORT_STATES);
    if (
        framesDecoded == null ||
        repliesEnqueued == null ||
        replyBytesEnqueued == null ||
        remoteCommitApplied == null ||
        documentPromoted == null ||
        transportState == null
    ) {
        return null;
    }
    let close: NativeEditorV2ReceiveClose | null = null;
    if (parsed.close !== null && parsed.close !== undefined) {
        if (!isPlainRecord(parsed.close)) return null;
        const disposition = whitelisted(parsed.close.disposition, ['retryable', 'incompatible']);
        const error = normalizeNativeEditorV2Error({ error: parsed.close.error });
        if (disposition == null || error == null) return null;
        close = { disposition, error };
    }
    return {
        framesDecoded,
        repliesEnqueued,
        replyBytesEnqueued,
        remoteCommitApplied,
        documentPromoted,
        transportState,
        close,
    };
}

// ─── Imperative throws ──────────────────────────────────────────

function invalidV2ResultError(): NativeEditorV2NonRetryableError {
    return new NativeEditorV2NonRetryableError({
        domain: 'boundary',
        code: 'FFI_RESULT_INVALID',
        message: ERR_V2_NATIVE_RESPONSE,
        requestId: null,
        operationIndex: null,
        limit: null,
        actual: null,
        details: null,
    });
}

function destroyedHandleError(): NativeEditorV2NonRetryableError {
    return new NativeEditorV2NonRetryableError({
        domain: 'lifecycle',
        code: 'ENGINE_DESTROYED',
        message: ERR_V2_DESTROYED,
        requestId: null,
        operationIndex: null,
        limit: null,
        actual: null,
        details: null,
    });
}

function invalidV2RequestError(message: string): NativeEditorV2BoundaryError {
    return new NativeEditorV2BoundaryError({
        domain: 'boundary',
        code: 'CONFIG_INVALID',
        message,
        requestId: null,
        operationIndex: null,
        limit: null,
        actual: null,
        details: null,
    });
}

/**
 * Unwrap a raw v2 result record on the imperative path: typed per-domain
 * throws for recoverable engine errors, the distinct non-retryable class for
 * ENGINE_INVARIANT_FAILED / lifecycle-destroyed states and for malformed
 * records (a bridge contract violation can never succeed on retry).
 */
export function unwrapNativeEditorV2Result<T>(
    raw: unknown,
    normalizeValue: (value: unknown) => T | null
): T {
    const result = normalizeNativeEditorV2Result(raw, normalizeValue);
    if (result == null) throw invalidV2ResultError();
    if (!result.ok) throw nativeEditorV2ErrorToException(result.error);
    return result.value;
}

function requireV2DecimalId(value: string | number, field: string): string {
    const normalized = normalizeNativeEditorV2DecimalId(value);
    if (normalized == null) {
        throw invalidV2RequestError(`NativeEditorBridge: invalid ${field} for v2 request`);
    }
    return normalized;
}

function requireV2Bytes(value: unknown, field: string): Uint8Array {
    const normalized = normalizeNativeEditorV2Bytes(value);
    if (normalized == null) {
        throw invalidV2RequestError(`NativeEditorBridge: invalid ${field} for v2 request`);
    }
    return normalized;
}

// ─── Document handle ────────────────────────────────────────────

export type NativeEditorV2HistoryMode = 'undoableBoundary' | 'resetAndClear';

export interface NativeEditorV2SnapshotMetadata {
    formatVersion: number;
    documentId: string;
    lineageId: string;
    fragmentName: string;
    schemaFingerprint: string;
}

export interface NativeEditorV2RoomSnapshot {
    metadata: NativeEditorV2SnapshotMetadata;
    encodedState: Uint8Array;
}

export type NativeEditorV2Initialization =
    | { type: 'localEmpty' }
    | { type: 'localJson'; json: DocumentJSON }
    | { type: 'localHtml'; html: string }
    | {
          type: 'room';
          documentId: string;
          lineageId: string;
          snapshot?: NativeEditorV2RoomSnapshot;
      };

export interface NativeEditorV2CreateConfig {
    initialization: NativeEditorV2Initialization;
    schema?: SchemaDefinition;
    fragmentName?: string;
    policy?: {
        maxLength?: number;
        readOnly?: boolean;
        inputFilter?: string;
        allowBase64Images?: boolean;
    };
    limits?: {
        resource?: EditorResourceLimits;
        editing?: EditorEditingLimits;
        collaboration?: EditorCollaborationLimits;
    };
}

export interface NativeEditorV2InputRequest {
    baseDocumentRevision: string | number;
    text: string;
}

export interface NativeEditorV2CommandRequest {
    baseDocumentRevision: string | number;
    command: Record<string, unknown>;
}

export interface NativeEditorV2LocalApiRequest {
    baseDocumentRevision: string | number;
    setJson?: DocumentJSON;
    setHtml?: string;
    history: NativeEditorV2HistoryMode;
}

export interface NativeEditorV2SelectionRequest {
    baseDocumentRevision: string | number;
    selection: Record<string, unknown>;
}

export interface NativeEditorV2ReplaceDocumentRequest {
    setJson?: DocumentJSON;
    setHtml?: string;
    history: NativeEditorV2HistoryMode;
}

const V2_CREATE_CONFIG_KEYS = new Set([
    'initialization',
    'schema',
    'fragmentName',
    'policy',
    'limits',
]);
const V2_CREATE_POLICY_KEYS = new Set([
    'maxLength',
    'readOnly',
    'inputFilter',
    'allowBase64Images',
]);
const V2_CREATE_LIMIT_KEYS = new Set(['resource', 'editing', 'collaboration']);
const V2_CREATE_RESOURCE_LIMIT_KEYS = new Set([
    'maxInputBytes',
    'maxDocumentNodes',
    'maxDocumentDepth',
    'maxSchemaNodes',
    'maxSchemaExpressionBytes',
    'maxCollaborationMessageBytes',
    'maxEncodedStateBytes',
]);
const V2_CREATE_EDITING_LIMIT_KEYS = new Set([
    'maxOperationsPerTransaction',
    'maxUndoGroups',
    'maxUndoRetainedUnits',
    'maxDerivedOutputBytes',
]);
const V2_CREATE_COLLABORATION_LIMIT_KEYS = new Set([
    'maxFramesPerMessage',
    'maxFrameBytes',
    'maxAggregateResponseBytes',
    'maxAwarenessPeers',
    'maxAwarenessPeerBytes',
    'maxAwarenessBytes',
    'maxPendingOutboxMessages',
    'maxPendingOutboxBytes',
    'maxPendingDependencyUpdateBytes',
    'maxPendingDependencyUpdateWork',
]);
const V2_CREATE_INITIALIZATION_KEYS: Readonly<Record<string, ReadonlySet<string>>> = {
    localEmpty: new Set(['type']),
    localJson: new Set(['type', 'json']),
    localHtml: new Set(['type', 'html']),
    room: new Set(['type', 'documentId', 'lineageId', 'snapshot']),
};
const V2_CREATE_ROOM_SNAPSHOT_KEYS = new Set(['metadata', 'encodedState']);
const V2_CREATE_SNAPSHOT_METADATA_KEYS = new Set([
    'formatVersion',
    'documentId',
    'lineageId',
    'fragmentName',
    'schemaFingerprint',
]);
const V2_CREATE_MAX_U32 = 0xffff_ffff;
const V2_CREATE_BOUNDARY_ERRORS = new WeakSet<object>();

function trustedV2CreateBoundaryError(error: NativeEditorV2BoundaryError): NativeEditorV2BoundaryError {
    V2_CREATE_BOUNDARY_ERRORS.add(error);
    return error;
}

function invalidV2CreateRequestError(message: string): NativeEditorV2BoundaryError {
    return trustedV2CreateBoundaryError(invalidV2RequestError(message));
}

function validateV2CreateLimits(limits: NativeEditorV2CreateConfig['limits']): void {
    try {
        validateEditorCreateLimits(limits);
    } catch (error) {
        if (!(error instanceof NativeEditorBoundaryError)) throw error;
        throw trustedV2CreateBoundaryError(
            new NativeEditorV2BoundaryError({
                domain: 'boundary',
                code: error.code,
                message: error.message,
                requestId: null,
                operationIndex: null,
                limit: error.limit ?? null,
                actual: error.actual ?? null,
                details: error.details ?? null,
            })
        );
    }
}

function emptyV2CreateRecord(): Record<string, unknown> {
    return Object.create(null) as Record<string, unknown>;
}

function isV2CreateRecord(value: unknown): value is Record<string, unknown> {
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return false;
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
}

function hasOwnV2CreateKey(value: Record<string, unknown>, key: string): boolean {
    return Object.prototype.hasOwnProperty.call(value, key);
}

function ownV2CreateValue(value: Record<string, unknown>, key: string): unknown {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (descriptor === undefined) return undefined;
    if (!('value' in descriptor)) {
        throw invalidV2CreateRequestError(
            `NativeEditorBridge: accessor ${key} is not allowed for v2 create`
        );
    }
    return descriptor.value;
}

function requireKnownV2CreateKeys(
    value: unknown,
    allowed: ReadonlySet<string>,
    label: string
): asserts value is Record<string, unknown> {
    if (
        !isV2CreateRecord(value) ||
        Reflect.ownKeys(value).some((key) => typeof key !== 'string' || !allowed.has(key))
    ) {
        throw invalidV2CreateRequestError(`NativeEditorBridge: invalid ${label} for v2 create`);
    }
}

function normalizeV2CreateRecord(
    value: Record<string, unknown>,
    allowed: ReadonlySet<string>,
    label: string
): Record<string, unknown> {
    requireKnownV2CreateKeys(value, allowed, label);
    const normalized = emptyV2CreateRecord();
    for (const key of allowed) {
        if (!hasOwnV2CreateKey(value, key)) continue;
        const fieldValue = ownV2CreateValue(value, key);
        if (fieldValue === null) {
            throw invalidV2CreateRequestError(`NativeEditorBridge: invalid ${label} for v2 create`);
        }
        if (fieldValue !== undefined) normalized[key] = fieldValue;
    }
    return normalized;
}

function invalidV2JsonValue(label: string): never {
    throw invalidV2CreateRequestError(`NativeEditorBridge: invalid ${label} for v2 create`);
}

function normalizeV2JsonValue(
    value: unknown,
    label: string,
    ancestors: WeakSet<object> = new WeakSet<object>()
): unknown {
    if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
    if (typeof value === 'number') {
        if (!Number.isFinite(value)) invalidV2JsonValue(label);
        return value;
    }
    if (typeof value !== 'object') invalidV2JsonValue(label);

    if (ancestors.has(value)) invalidV2JsonValue(label);
    ancestors.add(value);
    try {
        if (Array.isArray(value)) {
            const prototype = Object.getPrototypeOf(value);
            if (prototype !== Array.prototype && prototype !== null) invalidV2JsonValue(label);
            const lengthDescriptor = Object.getOwnPropertyDescriptor(value, 'length');
            if (
                lengthDescriptor === undefined ||
                !('value' in lengthDescriptor) ||
                typeof lengthDescriptor.value !== 'number'
            ) {
                invalidV2JsonValue(label);
            }
            const length = lengthDescriptor.value;
            const normalized = new Array<unknown>(length);
            let elementCount = 0;
            for (const key of Reflect.ownKeys(value)) {
                if (key === 'length') continue;
                if (typeof key !== 'string' || !/^(0|[1-9]\d*)$/.test(key)) {
                    invalidV2JsonValue(label);
                }
                const index = Number(key);
                if (!Number.isSafeInteger(index) || index < 0 || index >= length) {
                    invalidV2JsonValue(label);
                }
                const descriptor = Object.getOwnPropertyDescriptor(value, key);
                if (
                    descriptor === undefined ||
                    !('value' in descriptor) ||
                    descriptor.enumerable !== true
                ) {
                    invalidV2JsonValue(label);
                }
                normalized[index] = normalizeV2JsonValue(descriptor.value, label, ancestors);
                elementCount += 1;
            }
            if (elementCount !== length) invalidV2JsonValue(label);
            Object.setPrototypeOf(normalized, null);
            return normalized;
        }

        if (!isV2CreateRecord(value)) invalidV2JsonValue(label);
        const normalized = emptyV2CreateRecord();
        for (const key of Reflect.ownKeys(value)) {
            if (typeof key !== 'string') invalidV2JsonValue(label);
            const descriptor = Object.getOwnPropertyDescriptor(value, key);
            if (
                descriptor === undefined ||
                !('value' in descriptor) ||
                descriptor.enumerable !== true
            ) {
                invalidV2JsonValue(label);
            }
            normalized[key] = normalizeV2JsonValue(descriptor.value, label, ancestors);
        }
        return normalized;
    } finally {
        ancestors.delete(value);
    }
}

function normalizeV2CreatePolicy(value: Record<string, unknown>): Record<string, unknown> {
    const policy = normalizeV2CreateRecord(value, V2_CREATE_POLICY_KEYS, 'policy');
    const maxLength = ownV2CreateValue(policy, 'maxLength');
    if (
        maxLength !== undefined &&
        (typeof maxLength !== 'number' ||
            !Number.isSafeInteger(maxLength) ||
            maxLength < 0 ||
            maxLength > V2_CREATE_MAX_U32)
    ) {
        invalidV2JsonValue('policy.maxLength');
    }
    for (const key of ['readOnly', 'allowBase64Images']) {
        const value = ownV2CreateValue(policy, key);
        if (value !== undefined && typeof value !== 'boolean') {
            invalidV2JsonValue(`policy.${key}`);
        }
    }
    const inputFilter = ownV2CreateValue(policy, 'inputFilter');
    if (inputFilter !== undefined && typeof inputFilter !== 'string') {
        invalidV2JsonValue('policy.inputFilter');
    }
    return policy;
}

function normalizeV2SnapshotMetadata(value: unknown): Record<string, unknown> {
    const metadata = normalizeV2CreateRecord(
        value as Record<string, unknown>,
        V2_CREATE_SNAPSHOT_METADATA_KEYS,
        'snapshot metadata'
    );
    const formatVersion = ownV2CreateValue(metadata, 'formatVersion');
    if (
        typeof formatVersion !== 'number' ||
        !Number.isSafeInteger(formatVersion) ||
        formatVersion < 0 ||
        formatVersion > V2_CREATE_MAX_U32
    ) {
        invalidV2JsonValue('snapshot metadata.formatVersion');
    }
    for (const key of ['documentId', 'lineageId', 'fragmentName', 'schemaFingerprint']) {
        if (typeof ownV2CreateValue(metadata, key) !== 'string') {
            invalidV2JsonValue(`snapshot metadata.${key}`);
        }
    }
    return metadata;
}

function buildV2CreateRequestUnchecked(config: NativeEditorV2CreateConfig): {
    configJson: string;
    snapshotState: Uint8Array | null;
} {
    if (!isV2CreateRecord(config)) {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid v2 create config');
    }
    requireKnownV2CreateKeys(config, V2_CREATE_CONFIG_KEYS, 'config');
    const initializationValue = ownV2CreateValue(config, 'initialization');
    if (!isV2CreateRecord(initializationValue)) {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid v2 create config');
    }

    const policyValue = ownV2CreateValue(config, 'policy');
    const policy =
        policyValue === undefined
            ? undefined
            : normalizeV2CreatePolicy(policyValue as Record<string, unknown>);

    const limitsValue = ownV2CreateValue(config, 'limits');
    let limits: Record<string, unknown> | undefined;
    if (limitsValue !== undefined) {
        requireKnownV2CreateKeys(limitsValue, V2_CREATE_LIMIT_KEYS, 'limits');
        limits = emptyV2CreateRecord();
        for (const [group, keys] of [
            ['resource', V2_CREATE_RESOURCE_LIMIT_KEYS],
            ['editing', V2_CREATE_EDITING_LIMIT_KEYS],
            ['collaboration', V2_CREATE_COLLABORATION_LIMIT_KEYS],
        ] as const) {
            const overrides = ownV2CreateValue(limitsValue, group);
            if (overrides !== undefined) {
                limits[group] = normalizeV2CreateRecord(
                    overrides as Record<string, unknown>,
                    keys,
                    `${group} limits`
                );
            }
        }
    }
    validateV2CreateLimits(limits as NativeEditorV2CreateConfig['limits']);

    const envelope = emptyV2CreateRecord();
    const schema = ownV2CreateValue(config, 'schema');
    if (schema === null) {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid schema for v2 create');
    }
    if (schema !== undefined) {
        if (!isV2CreateRecord(schema)) {
            throw invalidV2CreateRequestError('NativeEditorBridge: invalid schema for v2 create');
        }
        envelope.schema = normalizeV2JsonValue(schema, 'schema');
    }
    const fragmentName = ownV2CreateValue(config, 'fragmentName');
    if (fragmentName !== undefined && typeof fragmentName !== 'string') {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid fragmentName for v2 create');
    }
    if (fragmentName !== undefined) envelope.fragmentName = fragmentName;

    let snapshotState: Uint8Array | null = null;
    const initialization = initializationValue;
    const initializationType = ownV2CreateValue(initialization, 'type');
    const initializationKeys =
        typeof initializationType === 'string'
            ? V2_CREATE_INITIALIZATION_KEYS[initializationType]
            : undefined;
    if (initializationKeys === undefined) {
        throw invalidV2CreateRequestError('NativeEditorBridge: unknown v2 initialization type');
    }
    requireKnownV2CreateKeys(initialization, initializationKeys, 'initialization');
    switch (initializationType) {
        case 'localEmpty': {
            const localEmpty = emptyV2CreateRecord();
            localEmpty.type = 'localEmpty';
            envelope.initialization = localEmpty;
            break;
        }
        case 'localJson': {
            const json = ownV2CreateValue(initialization, 'json');
            if (!isV2CreateRecord(json)) {
                throw invalidV2CreateRequestError(
                    'NativeEditorBridge: invalid localJson initialization for v2 create'
                );
            }
            const localJson = emptyV2CreateRecord();
            localJson.type = 'localJson';
            localJson.json = normalizeV2JsonValue(json, 'localJson initialization');
            envelope.initialization = localJson;
            break;
        }
        case 'localHtml': {
            const html = ownV2CreateValue(initialization, 'html');
            if (typeof html !== 'string') {
                throw invalidV2CreateRequestError(
                    'NativeEditorBridge: invalid localHtml initialization for v2 create'
                );
            }
            const localHtml = emptyV2CreateRecord();
            localHtml.type = 'localHtml';
            localHtml.html = html;
            envelope.initialization = localHtml;
            break;
        }
        case 'room': {
            const documentId = ownV2CreateValue(initialization, 'documentId');
            const lineageId = ownV2CreateValue(initialization, 'lineageId');
            if (typeof documentId !== 'string' || typeof lineageId !== 'string') {
                throw invalidV2CreateRequestError(
                    'NativeEditorBridge: invalid room initialization for v2 create'
                );
            }
            const room = emptyV2CreateRecord();
            room.type = 'room';
            room.documentId = documentId;
            room.lineageId = lineageId;
            const snapshot = ownV2CreateValue(initialization, 'snapshot');
            if (snapshot !== undefined) {
                requireKnownV2CreateKeys(snapshot, V2_CREATE_ROOM_SNAPSHOT_KEYS, 'room snapshot');
                const metadataValue = ownV2CreateValue(snapshot, 'metadata');
                const metadata = normalizeV2SnapshotMetadata(metadataValue);
                room.snapshot = metadata;
                snapshotState = requireV2Bytes(
                    ownV2CreateValue(snapshot, 'encodedState'),
                    'snapshot encodedState'
                );
            }
            envelope.initialization = room;
            break;
        }
        default:
            throw invalidV2CreateRequestError('NativeEditorBridge: unknown v2 initialization type');
    }
    if (policy !== undefined) envelope.policy = policy;
    if (limits !== undefined) envelope.limits = limits;
    const configJson = JSON.stringify(envelope);
    if (configJson === undefined) {
        throw invalidV2CreateRequestError('NativeEditorBridge: v2 create config is not serializable');
    }
    return { configJson, snapshotState };
}

function buildV2CreateRequest(config: NativeEditorV2CreateConfig): {
    configJson: string;
    snapshotState: Uint8Array | null;
} {
    try {
        return buildV2CreateRequestUnchecked(config);
    } catch (error) {
        if (typeof error === 'object' && error !== null && V2_CREATE_BOUNDARY_ERRORS.has(error)) {
            throw error;
        }
        throw invalidV2RequestError('NativeEditorBridge: invalid v2 create config');
    }
}

/**
 * Typed imperative v2 bridge bound to one decimal-string editor id. Every
 * entry normalizes the frozen result record, keeps revisions as decimal
 * strings, and throws typed errors; results that arrive for a destroyed
 * handle (including re-entrant destroy races) are classified non-retryable.
 */
export class NativeEditorV2Bridge {
    private readonly _editorId: string;
    private _destroyed = false;
    private _nextRequestId = 0;
    private readonly _errorListeners = new Set<(error: NativeEditorV2ErrorBase) => void>();

    /** @internal Created by createNativeEditorDocumentHandle. */
    constructor(editorId: string) {
        this._editorId = editorId;
    }

    get editorId(): string {
        return this._editorId;
    }

    get isDestroyed(): boolean {
        return this._destroyed;
    }

    private assertAlive(): void {
        if (this._destroyed) throw destroyedHandleError();
    }

    private callV2<T>(invoke: () => unknown, normalizeValue: (value: unknown) => T | null): T {
        this.assertAlive();
        const raw = invoke();
        // A re-entrant destroy racing the native call makes any result
        // arriving now a result for a destroyed handle: non-retryable.
        if (this._destroyed) throw destroyedHandleError();
        return unwrapNativeEditorV2Result(raw, normalizeValue);
    }

    private nextRequestId(): number {
        this._nextRequestId += 1;
        return this._nextRequestId;
    }

    /**
     * Serialize a request envelope. The base revision is spliced in as raw
     * canonical decimal digits so full u64 revisions survive without
     * Number()'ing them.
     */
    private buildEnvelopeJson(
        payload: Record<string, unknown>,
        baseDocumentRevision?: string | number
    ): string {
        const parts: string[] = [
            `"version":${V2_ENVELOPE_VERSION}`,
            `"requestId":${this.nextRequestId()}`,
        ];
        if (baseDocumentRevision !== undefined) {
            const digits = requireV2DecimalId(baseDocumentRevision, 'baseDocumentRevision');
            parts.push(`"baseDocumentRevision":${digits}`);
        }
        const payloadJson = JSON.stringify(payload);
        const inner = payloadJson.slice(1, payloadJson.length - 1);
        if (inner.length > 0) parts.push(inner);
        return `{${parts.join(',')}}`;
    }

    /** Destroy the session. Repeated destroy is safe. */
    destroy(): void {
        if (this._destroyed) return;
        // Terminal semantics (deliberate trade-off): the handle is marked
        // destroyed and the error listeners are cleared BEFORE the native
        // call, so a recoverable native destroy failure propagates but
        // subsequent destroy() calls will not retry the native call; the
        // session may leak natively in that edge. Callers should treat
        // destroy errors as terminal.
        this._destroyed = true;
        this._errorListeners.clear();
        try {
            unwrapNativeEditorV2Result(
                invokeNativeEditorV2('editorV2Destroy', this._editorId),
                normalizeNativeEditorV2Unit
            );
        } catch (error) {
            // An already-destroyed native session still satisfies the
            // caller's goal; every other failure is reported.
            if (
                error instanceof NativeEditorV2NonRetryableError &&
                (error.code === 'ENGINE_DESTROYED' || error.code === 'ENGINE_DESTROYING')
            ) {
                return;
            }
            throw error;
        }
    }

    /** Subscribe to autonomous native failures; returns the unsubscribe. */
    addErrorListener(listener: (error: NativeEditorV2ErrorBase) => void): () => void {
        this._errorListeners.add(listener);
        return () => {
            this._errorListeners.delete(listener);
        };
    }

    /**
     * @internal Route one autonomous native failure (input/accessibility) to
     * the error listeners exactly once. Accepts a bare error record or the
     * frozen envelope form; malformed payloads surface as a non-retryable
     * contract violation so the view stays usable.
     */
    _emitAutonomousError(raw: unknown): void {
        if (this._destroyed) return;
        const candidate = isPlainRecord(raw) && 'error' in raw ? raw : { error: raw };
        const normalized = normalizeNativeEditorV2Error(candidate);
        const exception =
            normalized == null ? invalidV2ResultError() : nativeEditorV2ErrorToException(normalized);
        for (const listener of this._errorListeners) {
            listener(exception);
        }
    }

    // ── State getters ───────────────────────────────────────────

    getState(): NativeEditorV2EditorState {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetState', this._editorId),
            normalizeNativeEditorV2StateValue
        );
    }

    getDocumentJson(): DocumentJSON {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetDocumentJson', this._editorId),
            normalizeNativeEditorV2DocumentJsonValue
        );
    }

    getDocumentHtml(): string {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetDocumentHtml', this._editorId),
            normalizeNativeEditorV2HtmlValue
        );
    }

    getContentSnapshot(): ContentSnapshot {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetContentSnapshot', this._editorId),
            normalizeNativeEditorV2ContentSnapshotValue
        );
    }

    /**
     * Fetch the engine's current render update (render blocks, active state,
     * history state, document version) as raw update JSON — the payload a
     * bound native view applies after a JS-driven engine change. A scalar
     * mirror selection resolves the engine selection into the update (doc
     * and scalar positions); without one the update carries no selection.
     */
    renderUpdate(mirrorScalarSelection?: { anchor: number; head: number }): string {
        this.assertAlive();
        const mirrorAnchor = mirrorScalarSelection?.anchor ?? null;
        const mirrorHead = mirrorScalarSelection?.head ?? null;
        if ((mirrorAnchor == null) !== (mirrorHead == null)) {
            throw invalidV2RequestError(
                'NativeEditorBridge: render update mirror requires both scalar anchor and head'
            );
        }
        return this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2RenderUpdate',
                    this._editorId,
                    mirrorAnchor,
                    mirrorHead
                ),
            normalizeNativeEditorV2RenderUpdateValue
        );
    }

    // ── Mutation entries ────────────────────────────────────────

    replaceDocument(request: NativeEditorV2ReplaceDocumentRequest): NativeEditorV2CommitInfo {
        this.assertAlive();
        const payload: Record<string, unknown> = {};
        if (request.setJson !== undefined) payload.setJson = request.setJson;
        if (request.setHtml !== undefined) payload.setHtml = request.setHtml;
        payload.history = request.history;
        const requestJson = this.buildEnvelopeJson(payload);
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ReplaceDocument', this._editorId, requestJson),
            normalizeNativeEditorV2CommitValue
        );
    }

    applyInput(request: NativeEditorV2InputRequest): NativeEditorV2MutationOutcome {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson(
            { text: request.text },
            request.baseDocumentRevision
        );
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ApplyInput', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    applyCommand(request: NativeEditorV2CommandRequest): NativeEditorV2MutationOutcome {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson(
            { command: request.command },
            request.baseDocumentRevision
        );
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ApplyCommand', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    applyLocalApi(request: NativeEditorV2LocalApiRequest): NativeEditorV2MutationOutcome {
        this.assertAlive();
        const payload: Record<string, unknown> = {};
        if (request.setJson !== undefined) payload.setJson = request.setJson;
        if (request.setHtml !== undefined) payload.setHtml = request.setHtml;
        payload.history = request.history;
        const requestJson = this.buildEnvelopeJson(payload, request.baseDocumentRevision);
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ApplyLocalApi', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    setSelection(request: NativeEditorV2SelectionRequest): NativeEditorV2MutationOutcome {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson(
            { selection: request.selection },
            request.baseDocumentRevision
        );
        return this.callV2(
            () => invokeNativeEditorV2('editorV2SetSelection', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    undo(): boolean {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson({});
        return this.callV2(
            () => invokeNativeEditorV2('editorV2Undo', this._editorId, requestJson),
            normalizeNativeEditorV2ChangedValue
        );
    }

    redo(): boolean {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson({});
        return this.callV2(
            () => invokeNativeEditorV2('editorV2Redo', this._editorId, requestJson),
            normalizeNativeEditorV2ChangedValue
        );
    }

    // ── Snapshots ───────────────────────────────────────────────

    snapshotExport(): NativeEditorV2SnapshotExport {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2SnapshotExport', this._editorId),
            normalizeNativeEditorV2SnapshotExportValue
        );
    }

    snapshotRestore(
        metadata: NativeEditorV2SnapshotMetadata,
        encodedState: Uint8Array
    ): NativeEditorV2CommitInfo {
        this.assertAlive();
        const bytes = requireV2Bytes(encodedState, 'snapshot encodedState');
        return this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2SnapshotRestore',
                    this._editorId,
                    JSON.stringify(metadata),
                    bytes
                ),
            normalizeNativeEditorV2CommitValue
        );
    }

    // ── Collaboration runtime ───────────────────────────────────

    collaborationBeginConnect(): string {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2CollaborationBeginConnect', this._editorId),
            normalizeNativeEditorV2GenerationValue
        );
    }

    collaborationSocketOpen(generation: string): Uint8Array {
        this.assertAlive();
        const acceptedGeneration = requireV2DecimalId(generation, 'generation');
        return this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2CollaborationSocketOpen',
                    this._editorId,
                    acceptedGeneration
                ),
            normalizeNativeEditorV2Bytes
        );
    }

    collaborationReceive(generation: string, message: Uint8Array): NativeEditorV2ReceiveOutcome {
        this.assertAlive();
        const acceptedGeneration = requireV2DecimalId(generation, 'generation');
        const bytes = requireV2Bytes(message, 'message');
        return this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2CollaborationReceive',
                    this._editorId,
                    acceptedGeneration,
                    bytes
                ),
            normalizeNativeEditorV2ReceiveValue
        );
    }

    collaborationSocketClose(
        generation: string,
        code?: number | null,
        reason?: string | null
    ): NativeEditorV2TransportState {
        this.assertAlive();
        const acceptedGeneration = requireV2DecimalId(generation, 'generation');
        const acceptedCode = code ?? null;
        if (
            acceptedCode !== null &&
            (!Number.isInteger(acceptedCode) || acceptedCode < 0 || acceptedCode > 0xffff_ffff)
        ) {
            throw invalidV2RequestError('NativeEditorBridge: invalid close code for v2 request');
        }
        return this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2CollaborationSocketClose',
                    this._editorId,
                    acceptedGeneration,
                    acceptedCode,
                    reason ?? null
                ),
            normalizeNativeEditorV2TransportStateValue
        );
    }

    collaborationTakeOutbound(generation: string): Uint8Array {
        this.assertAlive();
        const acceptedGeneration = requireV2DecimalId(generation, 'generation');
        return this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2CollaborationTakeOutbound',
                    this._editorId,
                    acceptedGeneration
                ),
            normalizeNativeEditorV2Bytes
        );
    }

    collaborationSetAwareness(state: Record<string, unknown> | null): void {
        this.assertAlive();
        const awarenessJson = state == null ? 'null' : JSON.stringify(state);
        this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2CollaborationSetAwareness',
                    this._editorId,
                    awarenessJson
                ),
            normalizeNativeEditorV2Unit
        );
    }

    collaborationPeers(): NativeEditorV2PeerInfo[] {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2CollaborationPeers', this._editorId),
            normalizeNativeEditorV2PeersValue
        );
    }
}

/**
 * The v2 document handle: a decimal-string editor id plus its typed bridge.
 * Created only through the v2 create entry; destroy and autonomous error
 * subscription mirror the bridge.
 */
let instantiateNativeEditorDocumentHandle!: (
    editorId: string,
    bridge: NativeEditorV2Bridge
) => NativeEditorDocumentHandle;

export class NativeEditorDocumentHandle {
    private constructor(
        public readonly editorId: string,
        public readonly bridge: NativeEditorV2Bridge
    ) {}

    private static readonly installFactory = (() => {
        instantiateNativeEditorDocumentHandle = (editorId, bridge) =>
            new NativeEditorDocumentHandle(editorId, bridge);
    })();

    get isDestroyed(): boolean {
        return this.bridge.isDestroyed;
    }

    destroy(): void {
        this.bridge.destroy();
    }

    addErrorListener(listener: (error: NativeEditorV2ErrorBase) => void): () => void {
        return this.bridge.addErrorListener(listener);
    }
}

export function createNativeEditorDocumentHandle(
    config: NativeEditorV2CreateConfig
): NativeEditorDocumentHandle {
    const { configJson, snapshotState } = buildV2CreateRequest(config);
    const value = unwrapNativeEditorV2Result(
        invokeNativeEditorV2('editorV2Create', configJson, snapshotState),
        normalizeNativeEditorV2CreateValue
    );
    return instantiateNativeEditorDocumentHandle(
        value.editorId,
        new NativeEditorV2Bridge(value.editorId)
    );
}
