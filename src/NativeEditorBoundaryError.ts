export type NativeEditorBoundaryErrorCode =
    | 'INVALID_RESOURCE_LIMIT'
    | 'INPUT_LIMIT_EXCEEDED'
    | 'CONFIG_PARSE_FAILED'
    | 'DOCUMENT_PARSE_FAILED'
    | 'DOCUMENT_INVALID'
    | 'DOCUMENT_LIMIT_EXCEEDED'
    | 'POSITION_LIMIT_EXCEEDED'
    | 'SCHEMA_INVALID'
    | 'REQUIRED_ATTRIBUTE_MISSING'
    | 'UNKNOWN_MARK'
    | 'MAX_LENGTH_EXCEEDED'
    | 'MUTATION_REJECTED'
    | 'COLLABORATION_DECODE_FAILED'
    | 'IMAGE_POLICY_INVALID'
    | 'IMAGE_REQUEST_TIMEOUT';

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
    if (
        typeof envelope?.error?.code !== 'string' ||
        typeof envelope.error.message !== 'string'
    ) {
        return null;
    }
    return new NativeEditorBoundaryError(
        envelope.error.code as NativeEditorBoundaryErrorCode,
        envelope.error.message,
        typeof envelope.error.limit === 'number' ? envelope.error.limit : undefined,
        typeof envelope.error.actual === 'number' ? envelope.error.actual : undefined,
        envelope.error.details != null && typeof envelope.error.details === 'object'
            ? (envelope.error.details as Record<string, unknown>)
            : undefined
    );
}
