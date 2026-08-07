import { normalizeNativeEditorV2U64 } from './NativeEditorV2Decimal';

/** The boundary failure codes this package knows about at build time. */
export const NATIVE_EDITOR_BOUNDARY_ERROR_CODES = [
    'INVALID_RESOURCE_LIMIT',
    'CONFIG_INVALID',
    'INPUT_LIMIT_EXCEEDED',
    'CONFIG_PARSE_FAILED',
    'DOCUMENT_PARSE_FAILED',
    'DOCUMENT_INVALID',
    'DOCUMENT_LIMIT_EXCEEDED',
    'POSITION_LIMIT_EXCEEDED',
    'SCHEMA_INVALID',
    'REQUIRED_ATTRIBUTE_MISSING',
    'UNKNOWN_MARK',
    'MAX_LENGTH_EXCEEDED',
    'MUTATION_REJECTED',
    'COLLABORATION_DECODE_FAILED',
    'COLLABORATION_APPLY_FAILED',
    'SESSION_NOT_FOUND',
    'IMAGE_POLICY_INVALID',
    'IMAGE_REQUEST_TIMEOUT',
] as const;

export type KnownNativeEditorBoundaryErrorCode =
    (typeof NATIVE_EDITOR_BOUNDARY_ERROR_CODES)[number];

/** Known codes remain suggested while future native codes can cross the boundary losslessly. */
export type NativeEditorBoundaryErrorCode = KnownNativeEditorBoundaryErrorCode | (string & {});

/** The subsystems a typed error can come from. Each maps to its own error class. */
export const NATIVE_EDITOR_ERROR_DOMAINS = [
    'boundary',
    'document',
    'operation',
    'lifecycle',
    'snapshot',
    'transport',
] as const;

/**
 * Which subsystem raised a failure:
 *
 * - `boundary` — the value never reached the engine (bad config, oversized input).
 * - `document` — the document or schema was rejected.
 * - `operation` — a mutation could not be applied (stale revision, invalid position).
 * - `lifecycle` — the session was destroyed or is being destroyed.
 * - `snapshot` — a room snapshot could not be exported or restored.
 * - `transport` — collaboration transport failure.
 */
export type NativeEditorErrorDomain = (typeof NATIVE_EDITOR_ERROR_DOMAINS)[number];

/** Codes carried by `operation`-domain failures. */
export const NATIVE_EDITOR_OPERATION_ERROR_CODES = [
    'ENGINE_NOT_READY',
    'REVISION_MISMATCH',
    'POSITION_INVALID',
    'TRANSACTION_INVALID',
    'OPERATION_INVALID',
    'OPERATION_LIMIT_EXCEEDED',
    'OPERATION_RESOURCE_EXHAUSTED',
    'DOCUMENT_INVALID',
    'DOCUMENT_LIMIT_EXCEEDED',
    'ENGINE_INVARIANT_FAILED',
] as const;

/**
 * A failure record from the native boundary, normalized for JavaScript. It is
 * carried on every {@link NativeEditorV2ErrorBase}; the numeric fields stay
 * decimal strings because they span the full Rust `u64` range.
 */
export interface NativeEditorV2Error {
    domain: NativeEditorErrorDomain;
    code: NativeEditorBoundaryErrorCode;
    message: string;
    /** Identity of the request that failed. */
    requestId: string | null;
    /** Index of the failing operation within a multi-operation transaction. */
    operationIndex: string | null;
    /** The bound that was exceeded, for limit failures. */
    limit: string | null;
    /** The value that exceeded it. */
    actual: string | null;
    /** Code-specific context, e.g. `expectedRevision`/`actualRevision` for `REVISION_MISMATCH`. */
    details: Record<string, unknown> | null;
}

/**
 * A value rejected before it reached the engine — an out-of-range resource
 * limit, an invalid image policy, an oversized input. Thrown synchronously by
 * the validating helpers in this package.
 */
export class NativeEditorBoundaryError extends Error {
    constructor(
        readonly code: NativeEditorBoundaryErrorCode,
        message: string,
        readonly limit?: number,
        readonly actual?: number,
        readonly details?: Record<string, unknown>
    ) {
        super(message);
        this.name = 'NativeEditorBoundaryError';
    }
}

/**
 * Turn a raw native error envelope into a {@link NativeEditorBoundaryError},
 * or null when the value is not one.
 */
export function parseNativeBoundaryError(value: unknown): NativeEditorBoundaryError | null {
    const envelope = value as { error?: Record<string, unknown> };
    const nativeError = envelope?.error;
    const code = nativeError?.code;
    if (typeof code !== 'string' || typeof nativeError?.message !== 'string') {
        return null;
    }
    return new NativeEditorBoundaryError(
        code,
        nativeError.message,
        typeof nativeError.limit === 'number' ? nativeError.limit : undefined,
        typeof nativeError.actual === 'number' ? nativeError.actual : undefined,
        nativeError.details != null && typeof nativeError.details === 'object'
            ? (nativeError.details as Record<string, unknown>)
            : undefined
    );
}

function nullableCanonicalDecimal(value: unknown): string | null | undefined {
    if (value == null) return null;
    return normalizeNativeEditorV2U64(value) ?? undefined;
}

function parseDetails(
    nativeError: Record<string, unknown>
): Record<string, unknown> | null | undefined {
    if (nativeError.details != null) {
        if (typeof nativeError.details !== 'object' || Array.isArray(nativeError.details)) {
            return undefined;
        }
        return nativeError.details as Record<string, unknown>;
    }
    if (nativeError.detailsJson == null) return null;
    if (typeof nativeError.detailsJson !== 'string') return undefined;
    try {
        const details: unknown = JSON.parse(nativeError.detailsJson);
        return details != null && typeof details === 'object' && !Array.isArray(details)
            ? (details as Record<string, unknown>)
            : undefined;
    } catch {
        return undefined;
    }
}

function isCanonicalDecimalDetail(value: unknown): value is string {
    return normalizeNativeEditorV2U64(value) != null;
}

/**
 * Error details are otherwise forward-compatible, but known v2 structures
 * carry u64 values and must not let JSON.parse silently admit rounded numbers.
 */
function hasValidKnownDetails(code: string, details: Record<string, unknown> | null): boolean {
    switch (code) {
        case 'REVISION_MISMATCH':
            return (
                details != null &&
                isCanonicalDecimalDetail(details.expectedRevision) &&
                isCanonicalDecimalDetail(details.actualRevision)
            );
        case 'TRANSPORT_STALE_GENERATION':
            return (
                details != null &&
                isCanonicalDecimalDetail(details.presentedGeneration) &&
                (details.liveGeneration === null ||
                    isCanonicalDecimalDetail(details.liveGeneration))
            );
        default:
            return true;
    }
}

/** Normalize a raw FFI v2 error record without changing the legacy error parser contract. */
export function normalizeNativeEditorV2Error(value: unknown): NativeEditorV2Error | null {
    const envelope = value as { error?: unknown };
    const nativeError = envelope?.error as Record<string, unknown> | undefined;
    if (nativeError == null || typeof nativeError !== 'object') return null;

    const { domain, code, message } = nativeError;
    if (
        typeof domain !== 'string' ||
        !NATIVE_EDITOR_ERROR_DOMAINS.includes(domain as NativeEditorErrorDomain) ||
        typeof code !== 'string' ||
        typeof message !== 'string'
    ) {
        return null;
    }

    const requestId = nullableCanonicalDecimal(nativeError.requestId);
    const operationIndex = nullableCanonicalDecimal(nativeError.operationIndex);
    const limit = nullableCanonicalDecimal(nativeError.limit);
    const actual = nullableCanonicalDecimal(nativeError.actual);
    const details = parseDetails(nativeError);
    if (
        requestId === undefined ||
        operationIndex === undefined ||
        limit === undefined ||
        actual === undefined ||
        details === undefined ||
        !hasValidKnownDetails(code, details)
    ) {
        return null;
    }

    return {
        domain: domain as NativeEditorErrorDomain,
        code,
        message,
        requestId,
        operationIndex,
        limit,
        actual,
        details,
    };
}

// FFI v2 typed imperative error classes (additive; the legacy parsing path
// above is unchanged). Recoverable engine errors throw the structured
// per-domain class; ENGINE_INVARIANT_FAILED and lifecycle-destroyed states
// throw the distinct non-retryable class.

/** Codes that mark a failure as permanently non-retryable for this handle. */
export const NATIVE_EDITOR_V2_NON_RETRYABLE_CODES = [
    'ENGINE_INVARIANT_FAILED',
    'ENGINE_DESTROYING',
    'ENGINE_DESTROYED',
] as const;

export function isNativeEditorV2NonRetryableCode(code: string): boolean {
    return (NATIVE_EDITOR_V2_NON_RETRYABLE_CODES as readonly string[]).includes(code);
}

/** Base class for every typed error raised from a normalized FFI v2 error record. */
export class NativeEditorV2ErrorBase extends Error {
    readonly domain: NativeEditorErrorDomain;
    readonly code: NativeEditorBoundaryErrorCode;
    readonly requestId: string | null;
    readonly operationIndex: string | null;
    readonly limit: string | null;
    readonly actual: string | null;
    readonly details: Record<string, unknown> | null;

    constructor(
        readonly error: NativeEditorV2Error,
        name: string
    ) {
        super(error.message);
        this.name = name;
        this.domain = error.domain;
        this.code = error.code;
        this.requestId = error.requestId;
        this.operationIndex = error.operationIndex;
        this.limit = error.limit;
        this.actual = error.actual;
        this.details = error.details;
    }
}

/** A value the engine refused to admit. */
export class NativeEditorV2BoundaryError extends NativeEditorV2ErrorBase {
    constructor(error: NativeEditorV2Error) {
        super(error, 'NativeEditorV2BoundaryError');
    }
}

/** The document or schema was rejected. */
export class NativeEditorV2DocumentError extends NativeEditorV2ErrorBase {
    constructor(error: NativeEditorV2Error) {
        super(error, 'NativeEditorV2DocumentError');
    }
}

/**
 * A mutation could not be applied. `REVISION_MISMATCH` is the routine one:
 * the document moved on, so re-read it and retry against the fresh revision.
 */
export class NativeEditorV2OperationError extends NativeEditorV2ErrorBase {
    constructor(error: NativeEditorV2Error) {
        super(error, 'NativeEditorV2OperationError');
    }
}

/** The session is not in a state that permits this call. */
export class NativeEditorV2LifecycleError extends NativeEditorV2ErrorBase {
    constructor(error: NativeEditorV2Error) {
        super(error, 'NativeEditorV2LifecycleError');
    }
}

/** A room snapshot could not be exported or restored. */
export class NativeEditorV2SnapshotError extends NativeEditorV2ErrorBase {
    constructor(error: NativeEditorV2Error) {
        super(error, 'NativeEditorV2SnapshotError');
    }
}

/** The collaboration transport failed. */
export class NativeEditorV2TransportError extends NativeEditorV2ErrorBase {
    constructor(error: NativeEditorV2Error) {
        super(error, 'NativeEditorV2TransportError');
    }
}

/**
 * Distinct class for failures that can never succeed on retry for this
 * handle: ENGINE_INVARIANT_FAILED and the lifecycle-destroyed states
 * (ENGINE_DESTROYING / ENGINE_DESTROYED).
 */
export class NativeEditorV2NonRetryableError extends NativeEditorV2ErrorBase {
    constructor(error: NativeEditorV2Error) {
        super(error, 'NativeEditorV2NonRetryableError');
    }
}

const NATIVE_EDITOR_V2_DOMAIN_ERROR_CLASSES: Record<
    NativeEditorErrorDomain,
    new (error: NativeEditorV2Error) => NativeEditorV2ErrorBase
> = {
    boundary: NativeEditorV2BoundaryError,
    document: NativeEditorV2DocumentError,
    operation: NativeEditorV2OperationError,
    lifecycle: NativeEditorV2LifecycleError,
    snapshot: NativeEditorV2SnapshotError,
    transport: NativeEditorV2TransportError,
};

/** Map a normalized FFI v2 error record to its typed imperative exception. */
export function nativeEditorV2ErrorToException(
    error: NativeEditorV2Error
): NativeEditorV2ErrorBase {
    if (isNativeEditorV2NonRetryableCode(error.code)) {
        return new NativeEditorV2NonRetryableError(error);
    }
    const ErrorClass = NATIVE_EDITOR_V2_DOMAIN_ERROR_CLASSES[error.domain];
    return new ErrorClass(error);
}
