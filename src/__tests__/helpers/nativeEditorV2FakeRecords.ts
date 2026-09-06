import type { DocumentJSON } from '../../NativeEditorBridge';
import { normalizeNativeEditorV2U64 } from '../../NativeEditorV2Decimal';

export const V2_FAKE_STEP1_FRAME = new Uint8Array([0, 0, 1]);

export const V2_FAKE_STEP2_FRAME = new Uint8Array([0, 0, 2]);

export const V2_FAKE_STEP2_INVALID_FRAGMENT_FRAME = new Uint8Array([0, 0, 5]);

export const V2_FAKE_UPDATE_FRAME = new Uint8Array([0, 1, 1]);

export const V2_FAKE_AWARENESS_FRAME = new Uint8Array([0, 2, 1]);

export const V2_FAKE_MALFORMED_FRAME = new Uint8Array([0xff]);

export const V2_FAKE_INCOMPATIBLE_FRAME = new Uint8Array([0xfe]);

export const V2_FAKE_U64_MAX = 18_446_744_073_709_551_615n;

export const V2_FAKE_U32_MAX = 0xffff_ffff;

export const V2_FAKE_MAX_ADMITTED_REMOTE_AWARENESS_CLOCK = V2_FAKE_U32_MAX - 1;

export const V2_FAKE_AWARENESS_RENEWAL_INTERVAL_MILLIS = 15_000n;

export const V2_FAKE_AWARENESS_EXPIRY_MILLIS = 30_000n;

export const V2_FAKE_DEFAULT_MAX_AWARENESS_PEER_BYTES = 64 * 1024;

/** The single native notification the collaboration transport delivers. */
export const V2_FAKE_TRANSPORT_EVENT_NAME = 'onCollaborationTransportEvent';

export const V2_FAKE_MALFORMED_AWARENESS_MESSAGE =
    'awareness update cannot decode: fake entry requires canonical u64 clientId and exact u32 clock';

export const EMPTY_DOC: DocumentJSON = { type: 'doc', content: [{ type: 'paragraph' }] };

/**
 * Mirrors the core's `document_is_empty`: a lone, contentless block of the
 * schema's preferred text block. The fake has to agree with the core here —
 * a fake that omits a field the core emits is a fake that cannot catch the
 * frozen render-update shape drifting.
 */
export function fakeDocumentIsEmpty(doc: DocumentJSON): boolean {
    const content = doc.content;
    if (content == null || content.length === 0) return true;
    if (content.length > 1) return false;
    const block = content[0];
    if (block == null) return true;
    const blockContent = block.content;
    if (blockContent != null && blockContent.length > 0) return false;
    if (typeof block.text === 'string' && block.text.length > 0) return false;
    return block.type === 'paragraph';
}

export type FakeTransportState =
    | 'Detached'
    | 'Disconnected'
    | 'Connecting'
    | 'Handshaking'
    | 'Synchronized'
    | 'Incompatible'
    | 'Destroyed';

export type FakeDocumentState = 'LocalReady' | 'AwaitRemote' | 'RoomReady';

/** The Rust-tagged JSON wire value accepted by the fake native entry. */
export interface FakeNativeEditorLocalAwarenessWireSelection {
    type: 'text';
    anchor: number;
    head: number;
}

/**
 * The fake models the Rust wire contract, not the opaque caller intent that
 * TypeScript validates before native invocation.
 */
export interface FakeNativeEditorLocalAwarenessWireIntent {
    state: Record<string, unknown>;
    focused: boolean;
    /** Absent retains the engine-owned cursor; `null` clears it. */
    selection?: FakeNativeEditorLocalAwarenessWireSelection | null;
}

export interface FakeErrorRecord {
    domain: string;
    code: string;
    message: string;
    requestId: string | null;
    operationIndex: string | null;
    limit: string | null;
    actual: string | null;
    details: Record<string, unknown> | null;
}

export function okRecord(value: unknown): Record<string, unknown> {
    return { value, error: null };
}

export function errorRecord(domain: string, code: string, message: string): FakeErrorRecord {
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

export function errRecord(error: FakeErrorRecord): Record<string, unknown> {
    return { value: null, error };
}

export function transportError(
    code: string,
    message: string,
    details: Record<string, unknown> | null = null
): Record<string, unknown> {
    const error = errorRecord('transport', code, message);
    error.details = details;
    return errRecord(error);
}

export function lifecycleError(code: string, message: string): Record<string, unknown> {
    return errRecord(errorRecord('lifecycle', code, message));
}

export function operationError(
    code: string,
    message: string,
    details: Record<string, unknown> | null = null
): Record<string, unknown> {
    const error = errorRecord('operation', code, message);
    error.details = details;
    return errRecord(error);
}

export function snapshotError(code: string, message: string): Record<string, unknown> {
    return errRecord(errorRecord('snapshot', code, message));
}

export function boundaryError(code: string, message: string): Record<string, unknown> {
    return errRecord(errorRecord('boundary', code, message));
}

export function awarenessPeerBytesLimitError(
    limit: number,
    actual: number
): Record<string, unknown> {
    const error = errorRecord(
        'boundary',
        'INPUT_LIMIT_EXCEEDED',
        `input exceeds limit ${limit}: ${actual}`
    );
    error.limit = String(limit);
    error.actual = String(actual);
    error.details = { field: 'maxAwarenessPeerBytes' };
    return errRecord(error);
}

/** Match the production v2 boundary: u64s are canonical decimal strings only. */
export function canonicalV2U64(value: unknown): string | null {
    return normalizeNativeEditorV2U64(value);
}

/** Match platform/JS exact-u32 admission before a native integer conversion. */
export function exactV2U32(value: unknown): number | null {
    return typeof value === 'number' &&
        Number.isFinite(value) &&
        Number.isInteger(value) &&
        value >= 0 &&
        value <= 0xffff_ffff
        ? value
        : null;
}

export function parseV2RequestEnvelope(
    requestJson: string,
    requiresBaseRevision: boolean
): Record<string, unknown> | Record<string, unknown> {
    let request: unknown;
    try {
        request = JSON.parse(requestJson);
    } catch {
        return {
            __v2RequestError: boundaryError('CONFIG_INVALID', 'malformed v2 request envelope'),
        };
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

export function requestEnvelopeError(
    parsed: Record<string, unknown>
): Record<string, unknown> | null {
    const error = parsed.__v2RequestError;
    return error != null && typeof error === 'object' ? (error as Record<string, unknown>) : null;
}

/**
 * Mirrors the Rust `PositionEnvelope`: `{ offset, kind, affinity? }` in scalar
 * currency. Bare integers are rejected exactly as serde rejects them.
 */
export function fakePositionEnvelopeScalar(value: unknown): number | null {
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return null;
    const envelope = value as Record<string, unknown>;
    if (envelope.kind !== 'scalar') return null;
    if (
        envelope.affinity !== undefined &&
        envelope.affinity !== 'before' &&
        envelope.affinity !== 'after'
    ) {
        return null;
    }
    return exactV2U32(envelope.offset);
}
