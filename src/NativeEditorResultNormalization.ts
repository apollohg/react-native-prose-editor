import {
    NativeEditorV2BoundaryError,
    NativeEditorV2NonRetryableError,
    nativeEditorV2ErrorToException,
    normalizeNativeEditorV2Error,
} from './NativeEditorBoundaryError';
import { normalizeNativeEditorV2U64 } from './NativeEditorV2Decimal';
import {
    type NativeEditorV2Result,
    ERR_V2_NATIVE_RESPONSE,
    ERR_V2_DESTROYED,
} from './NativeEditorNativeModule';
import { type RenderElement, type RenderMark } from './NativeEditorTypes';
import { type NativeEditorV2PeerInfo } from './NativeEditorCollaborationTransport';

export function isPlainRecord(value: unknown): value is Record<string, unknown> {
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
 * V2 never accepts numeric compatibility values: JavaScript numbers cannot
 * represent the complete Rust u64 domain.
 */
export function normalizeNativeEditorV2DecimalId(value: unknown): string | null {
    return normalizeNativeEditorV2U64(value);
}

export function normalizeRevisionField(
    record: Record<string, unknown>,
    field: string
): string | null {
    return normalizeNativeEditorV2DecimalId(record[field]);
}

export function nativeEditorV2U32(value: unknown): number | null {
    return typeof value === 'number' &&
        Number.isFinite(value) &&
        Number.isInteger(value) &&
        value >= 0 &&
        value <= 0xffff_ffff
        ? value
        : null;
}

/** Require one exact JavaScript/platform u32 value at the v2 boundary. */
export function requireNativeEditorV2U32(value: unknown, field: string): number {
    const normalized = nativeEditorV2U32(value);
    if (normalized == null) {
        throw invalidV2RequestError(`NativeEditorBridge: invalid u32 ${field}`);
    }
    return normalized;
}

export function optionalBoolean(value: unknown): boolean | null {
    return typeof value === 'boolean' ? value : null;
}

export const V2_DOCUMENT_STATES = ['LocalReady', 'AwaitRemote', 'RoomReady'] as const;

/**
 * Readiness of the engine document. `AwaitRemote` is a room document still
 * waiting for the server's copy: content getters return empty values and the
 * view renders nothing until it promotes to `RoomReady`.
 */
export type NativeEditorV2DocumentState = (typeof V2_DOCUMENT_STATES)[number];

export const V2_TRANSPORT_STATES = [
    'Detached',
    'Disconnected',
    'Connecting',
    'Handshaking',
    'Synchronized',
    'Incompatible',
    'Destroying',
    'Destroyed',
] as const;

/** Raw transport lifecycle. `YjsTransportStatus` is the friendlier projection of it. */
export type NativeEditorV2TransportState = (typeof V2_TRANSPORT_STATES)[number];

export const V2_RENDER_STATES = ['Loading', 'Ready'] as const;

/** Whether the engine has a render snapshot to draw. */
export type NativeEditorV2RenderState = (typeof V2_RENDER_STATES)[number];

export const V2_DOCUMENT_ORIGINS = [
    'nativeView',
    'jsApi',
    'remoteCollaboration',
    'history',
    'restore',
    'import',
] as const;

export type NativeEditorV2DocumentOrigin = (typeof V2_DOCUMENT_ORIGINS)[number];

export function whitelisted<T extends string>(value: unknown, allowed: readonly T[]): T | null {
    return typeof value === 'string' && (allowed as readonly string[]).includes(value)
        ? (value as T)
        : null;
}

/** The engine's own account of a session: readiness, revisions, and history. */
export interface NativeEditorV2EditorState {
    documentState: NativeEditorV2DocumentState;
    transportState: NativeEditorV2TransportState;
    renderState: NativeEditorV2RenderState;
    /** Decimal-string revision advancing on every document change. */
    documentRevision: string;
    /** Trusted origin of the transaction that produced documentRevision. */
    documentOrigin: NativeEditorV2DocumentOrigin;
    /** Decimal-string revision advancing on every state change, document or not. */
    stateRevision: string;
    canUndo: boolean;
    canRedo: boolean;
}

export function normalizeNativeEditorV2StateValue(
    value: unknown
): NativeEditorV2EditorState | null {
    const parsed = typeof value === 'string' ? parseNativeEditorV2JsonValue(value) : value;
    if (!isPlainRecord(parsed)) return null;
    const documentState = whitelisted(parsed.documentState, V2_DOCUMENT_STATES);
    const transportState = whitelisted(parsed.transportState, V2_TRANSPORT_STATES);
    const renderState = whitelisted(parsed.renderState, V2_RENDER_STATES);
    const documentRevision = normalizeRevisionField(parsed, 'documentRevision');
    const documentOrigin = whitelisted(parsed.documentOrigin, V2_DOCUMENT_ORIGINS);
    const stateRevision = normalizeRevisionField(parsed, 'stateRevision');
    const canUndo = optionalBoolean(parsed.canUndo);
    const canRedo = optionalBoolean(parsed.canRedo);
    if (
        documentState == null ||
        transportState == null ||
        renderState == null ||
        documentRevision == null ||
        documentOrigin == null ||
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
        documentOrigin,
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

export function normalizeNativeEditorV2CommitValue(
    value: unknown
): NativeEditorV2CommitInfo | null {
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

export const RENDER_ELEMENT_TYPES = new Set<RenderElement['type']>([
    'textRun',
    'blockStart',
    'blockEnd',
    'voidInline',
    'voidBlock',
    'opaqueInlineAtom',
    'opaqueBlockAtom',
]);

export function hasExactOwnKeys(
    record: Record<string, unknown>,
    expected: readonly string[]
): boolean {
    const actual = Object.keys(record).sort();
    const sortedExpected = [...expected].sort();
    return (
        actual.length === sortedExpected.length &&
        actual.every((key, index) => key === sortedExpected[index])
    );
}

export function hasOnlyOwnKeys(
    record: Record<string, unknown>,
    allowed: readonly string[]
): boolean {
    const allowedKeys = new Set(allowed);
    return Object.keys(record).every((key) => allowedKeys.has(key));
}

export function booleanRecord(value: unknown): value is Record<string, boolean> {
    return (
        isPlainRecord(value) && Object.values(value).every((entry) => typeof entry === 'boolean')
    );
}

export function stringArray(value: unknown): value is string[] {
    return Array.isArray(value) && value.every((entry) => typeof entry === 'string');
}

export function validJsonValue(value: unknown): boolean {
    if (value === null) return true;
    if (typeof value === 'string' || typeof value === 'boolean') return true;
    if (typeof value === 'number') return Number.isFinite(value);
    if (Array.isArray(value)) return value.every(validJsonValue);
    return isPlainRecord(value) && Object.values(value).every(validJsonValue);
}

export function validRenderMark(value: unknown): value is RenderMark {
    return (
        typeof value === 'string' ||
        (isPlainRecord(value) &&
            typeof value.type === 'string' &&
            Object.values(value).every(validJsonValue))
    );
}

export function normalizeNativeEditorV2PeersValue(value: unknown): NativeEditorV2PeerInfo[] | null {
    const parsed = typeof value === 'string' ? parseNativeEditorV2JsonValue(value) : value;
    if (!isPlainRecord(parsed) || !Array.isArray(parsed.peers)) return null;
    const peers: NativeEditorV2PeerInfo[] = [];
    for (const rawPeer of parsed.peers) {
        if (!isPlainRecord(rawPeer)) return null;
        const clientId = normalizeNativeEditorV2DecimalId(rawPeer.clientId);
        const clock = nativeEditorV2U32(rawPeer.clock);
        const isLocal = optionalBoolean(rawPeer.isLocal);
        if (clientId == null || clock == null || isLocal == null) return null;
        if (rawPeer.state !== null && !isPlainRecord(rawPeer.state)) return null;
        let cursor: NativeEditorV2PeerInfo['cursor'] = null;
        if (rawPeer.cursor !== null && rawPeer.cursor !== undefined) {
            if (!isPlainRecord(rawPeer.cursor)) return null;
            const anchor = nativeEditorV2U32(rawPeer.cursor.anchor);
            const head = nativeEditorV2U32(rawPeer.cursor.head);
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

export const REJECTED_V2_RECORD_PREVIEW_CHARS = 4_000;

/** Dev-only preview of a rejected boundary record, bounded so a large document never floods the log. */
export function describeRejectedV2Record(raw: unknown): string {
    let serialized: string;
    try {
        serialized = JSON.stringify(raw);
    } catch {
        return `<unserializable ${typeof raw}>`;
    }
    if (serialized === undefined) return `<${typeof raw}>`;
    return serialized.length > REJECTED_V2_RECORD_PREVIEW_CHARS
        ? `${serialized.slice(0, REJECTED_V2_RECORD_PREVIEW_CHARS)}… (${serialized.length} chars)`
        : serialized;
}

export function invalidV2ResultError(): NativeEditorV2NonRetryableError {
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

export function destroyedHandleError(): NativeEditorV2NonRetryableError {
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

export function invalidV2RequestError(message: string): NativeEditorV2BoundaryError {
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
    if (result == null) {
        // The thrown error cannot carry the payload, so name the rejected
        // record here — otherwise the failure is unattributable in the app.
        if (__DEV__) {
            console.error(
                'NativeEditorBridge: native module returned a record this boundary rejected',
                describeRejectedV2Record(raw)
            );
        }
        throw invalidV2ResultError();
    }
    if (!result.ok) throw nativeEditorV2ErrorToException(result.error);
    return result.value;
}

export function requireV2DecimalId(value: string, field: string): string {
    const normalized = normalizeNativeEditorV2DecimalId(value);
    if (normalized == null) {
        throw invalidV2RequestError(`NativeEditorBridge: invalid ${field} for v2 request`);
    }
    return normalized;
}

export function requireV2Bytes(value: unknown, field: string): Uint8Array {
    const normalized = normalizeNativeEditorV2Bytes(value);
    if (normalized == null) {
        throw invalidV2RequestError(`NativeEditorBridge: invalid ${field} for v2 request`);
    }
    return normalized;
}
