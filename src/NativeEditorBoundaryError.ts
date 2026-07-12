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

export type NativeEditorBoundaryErrorCode =
    (typeof NATIVE_EDITOR_BOUNDARY_ERROR_CODES)[number];

const NATIVE_EDITOR_BOUNDARY_ERROR_CODE_SET: ReadonlySet<string> = new Set(
    NATIVE_EDITOR_BOUNDARY_ERROR_CODES
);

function isNativeEditorBoundaryErrorCode(
    value: unknown
): value is NativeEditorBoundaryErrorCode {
    return typeof value === 'string' && NATIVE_EDITOR_BOUNDARY_ERROR_CODE_SET.has(value);
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
    if (
        !isNativeEditorBoundaryErrorCode(code) ||
        typeof nativeError?.message !== 'string'
    ) {
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
