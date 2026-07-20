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

export const NATIVE_EDITOR_ERROR_DOMAINS = [
    'boundary',
    'document',
    'operation',
    'lifecycle',
    'snapshot',
    'transport',
] as const;

export type NativeEditorErrorDomain = (typeof NATIVE_EDITOR_ERROR_DOMAINS)[number];

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

export interface NativeEditorV2Error {
    domain: NativeEditorErrorDomain;
    code: NativeEditorBoundaryErrorCode;
    message: string;
    requestId: string | null;
    operationIndex: number | null;
    limit: number | null;
    actual: number | null;
    details: Record<string, unknown> | null;
}

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

const CANONICAL_DECIMAL_ID = /^(0|[1-9]\d*)$/;

function nullableUnsignedInteger(value: unknown): number | null | undefined {
    if (value == null) return null;
    if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) return undefined;
    return value;
}

function parseDetails(nativeError: Record<string, unknown>): Record<string, unknown> | null | undefined {
    if (nativeError.details != null) {
        if (
            typeof nativeError.details !== 'object' ||
            Array.isArray(nativeError.details)
        ) {
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

    const requestId = nativeError.requestId;
    if (
        requestId != null &&
        (typeof requestId !== 'string' || !CANONICAL_DECIMAL_ID.test(requestId))
    ) {
        return null;
    }
    const operationIndex = nullableUnsignedInteger(nativeError.operationIndex);
    const limit = nullableUnsignedInteger(nativeError.limit);
    const actual = nullableUnsignedInteger(nativeError.actual);
    const details = parseDetails(nativeError);
    if (operationIndex === undefined || limit === undefined || actual === undefined || details === undefined) {
        return null;
    }

    return {
        domain: domain as NativeEditorErrorDomain,
        code,
        message,
        requestId: requestId == null ? null : requestId,
        operationIndex,
        limit,
        actual,
        details,
    };
}
